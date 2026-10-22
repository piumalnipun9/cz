//! Scoped, breadth-first directory discovery on a background thread.
//!
//! Guarantees:
//! * Every emitted path is a lexical descendant of one of the given roots
//!   (children are built with `parent.join(name)`; nothing is canonicalized).
//! * Symbolic links are not followed unless `follow_symlinks` is set; then a
//!   (device, inode) set prevents cycles.
//! * Unreadable or vanished directories are counted and skipped.
//! * Paths in `excluded_paths` (e.g. /proc) and mount points of excluded
//!   filesystem types (network, FUSE, virtual) are never entered unless they
//!   are themselves a root. Skipping them before `read_dir` also avoids
//!   triggering automounts or hanging on unreachable network shares.
//!
//! With `include_files` (used by `czi`) regular files are emitted too; only
//! directories are descended into.
//!
//! The scan is independent of the query: it collects directories once and the
//! UI re-matches them as the query changes, so typing never restarts I/O.
//! Breadth-first order means shallow (usually more relevant) directories
//! arrive first.

use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::config::SearchConfig;

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub include_hidden: bool,
    pub max_depth: usize,
    pub follow_symlinks: bool,
    pub max_entries: usize,
    pub include_files: bool,
    pub excluded_names: HashSet<OsString>,
    pub excluded_paths: HashSet<PathBuf>,
}

impl From<&SearchConfig> for ScanOptions {
    fn from(c: &SearchConfig) -> Self {
        let mut excluded_paths: HashSet<PathBuf> = c.exclusions.paths.iter().cloned().collect();
        if let Ok(table) = std::fs::read_to_string("/proc/self/mounts") {
            excluded_paths.extend(excluded_mounts(&table, &c.exclusions.filesystem_types));
        }
        Self {
            include_hidden: c.include_hidden,
            max_depth: c.max_depth,
            follow_symlinks: c.follow_symlinks,
            max_entries: c.max_scan_entries,
            include_files: false,
            excluded_names: c
                .exclusions
                .directories
                .iter()
                .map(OsString::from)
                .collect(),
            excluded_paths,
        }
    }
}

/// Mount points in a `/proc/self/mounts` table whose type matches `types`.
pub fn excluded_mounts(table: &str, types: &[String]) -> Vec<PathBuf> {
    let matches = |fstype: &str| {
        types.iter().any(|t| match t.strip_suffix('*') {
            Some(prefix) => fstype.starts_with(prefix),
            None => fstype == t,
        })
    };
    table
        .lines()
        .filter_map(|line| {
            let mut f = line.split(' ');
            let (_, mount, fstype) = (f.next()?, f.next()?, f.next()?);
            matches(fstype).then(|| PathBuf::from(unescape_mount(mount)))
        })
        .filter(|p| p != Path::new("/"))
        .collect()
}

/// Decodes the octal escapes (`\040` for space etc.) used in the mount table.
fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanSummary {
    pub dirs: usize,
    pub files: usize,
    pub errors: usize,
    pub truncated: bool,
    pub cancelled: bool,
}

pub enum ScanEvent {
    Batch(Vec<PathBuf>),
    Done(ScanSummary),
}

/// Walks `roots` synchronously, calling `emit` for every directory found.
pub fn scan(
    roots: &[PathBuf],
    opts: &ScanOptions,
    cancel: &AtomicBool,
    mut emit: impl FnMut(PathBuf),
) -> ScanSummary {
    let mut sum = ScanSummary::default();
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    for r in roots {
        if opts.follow_symlinks
            && let Ok(m) = std::fs::metadata(r)
        {
            seen.insert((m.dev(), m.ino()));
        }
        queue.push_back((r.clone(), 0));
    }

    'outer: while let Some((dir, depth)) = queue.pop_front() {
        if depth >= opts.max_depth {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            sum.cancelled = true;
            break;
        }
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => {
                sum.errors += 1;
                continue;
            }
        };
        for entry in rd {
            let Ok(entry) = entry else {
                sum.errors += 1;
                continue;
            };
            let name = entry.file_name();
            if !opts.include_hidden && name.as_bytes().first() == Some(&b'.') {
                continue;
            }
            if opts.excluded_names.contains(&name) {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let is_dir = if ft.is_dir() {
                true
            } else if ft.is_symlink() && opts.follow_symlinks {
                entry.path().is_dir()
            } else {
                false
            };
            if !is_dir {
                if opts.include_files && (ft.is_file() || ft.is_symlink()) {
                    emit(dir.join(&name));
                    sum.files += 1;
                    if sum.dirs + sum.files >= opts.max_entries {
                        sum.truncated = true;
                        break 'outer;
                    }
                }
                continue;
            }
            let path = dir.join(&name);
            if opts.excluded_paths.contains(&path) {
                continue;
            }
            if opts.follow_symlinks {
                match std::fs::metadata(&path) {
                    Ok(m) if seen.insert((m.dev(), m.ino())) => {}
                    _ => continue,
                }
            }
            emit(path.clone());
            sum.dirs += 1;
            if sum.dirs + sum.files >= opts.max_entries {
                sum.truncated = true;
                break 'outer;
            }
            queue.push_back((path, depth + 1));
        }
    }
    sum
}

/// A running background scan. Dropping it cancels the walk.
pub struct ScanHandle {
    pub rx: Receiver<ScanEvent>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ScanHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.cancel();
        // Don't join: a read_dir on a slow mount could block exit.
        drop(self.thread.take());
    }
}

const BATCH_SIZE: usize = 512;
const BATCH_INTERVAL: Duration = Duration::from_millis(30);

pub fn spawn(roots: Vec<PathBuf>, opts: ScanOptions) -> ScanHandle {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let thread = std::thread::Builder::new()
        .name("cz-scan".into())
        .spawn(move || {
            let mut batch = Vec::with_capacity(BATCH_SIZE);
            let mut last = Instant::now();
            let summary = scan(&roots, &opts, &flag, |p| {
                batch.push(p);
                if batch.len() >= BATCH_SIZE || last.elapsed() >= BATCH_INTERVAL {
                    if tx
                        .send(ScanEvent::Batch(std::mem::take(&mut batch)))
                        .is_err()
                    {
                        flag.store(true, Ordering::Relaxed);
                    }
                    last = Instant::now();
                }
            });
            if !batch.is_empty() {
                let _ = tx.send(ScanEvent::Batch(batch));
            }
            let _ = tx.send(ScanEvent::Done(summary));
        })
        .expect("spawning scan thread");
    ScanHandle {
        rx,
        cancel,
        thread: Some(thread),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn opts() -> ScanOptions {
        ScanOptions::from(&SearchConfig::default())
    }

    fn collect(roots: &[PathBuf], o: &ScanOptions) -> (Vec<PathBuf>, ScanSummary) {
        let mut v = Vec::new();
        let s = scan(roots, o, &AtomicBool::new(false), |p| v.push(p));
        v.sort();
        (v, s)
    }

    #[test]
    fn stays_inside_roots_and_skips_symlinks() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        let outside = t.path().join("outside/secret");
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(t.path().join("outside"), root.join("link")).unwrap();
        symlink(&root, root.join("a/loop")).unwrap();
        let (found, _) = collect(std::slice::from_ref(&root), &opts());
        assert_eq!(found, vec![root.join("a"), root.join("a/b")]);
        assert!(found.iter().all(|p| p.starts_with(&root)));
    }

    #[test]
    fn follow_symlinks_avoids_loops() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().to_path_buf();
        fs::create_dir_all(root.join("a")).unwrap();
        symlink(&root, root.join("a/loop")).unwrap();
        let o = ScanOptions {
            follow_symlinks: true,
            ..opts()
        };
        let (found, s) = collect(std::slice::from_ref(&root), &o);
        assert_eq!(found, vec![root.join("a")]);
        assert!(!s.truncated);
    }

    #[test]
    fn exclusions_hidden_and_depth() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().to_path_buf();
        for d in ["node_modules/x", ".hidden/y", "a/b/c/d"] {
            fs::create_dir_all(r.join(d)).unwrap();
        }
        let (found, _) = collect(std::slice::from_ref(&r), &opts());
        assert!(found.contains(&r.join(".hidden/y")));
        assert!(
            !found
                .iter()
                .any(|p| p.to_string_lossy().contains("node_modules"))
        );

        let o = ScanOptions {
            include_hidden: false,
            max_depth: 2,
            ..opts()
        };
        let (found, _) = collect(std::slice::from_ref(&r), &o);
        assert_eq!(found, vec![r.join("a"), r.join("a/b")]);
    }

    #[test]
    fn permission_errors_do_not_stop_scan() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().to_path_buf();
        fs::create_dir_all(r.join("locked/inner")).unwrap();
        fs::create_dir_all(r.join("open/inner")).unwrap();
        fs::set_permissions(r.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        let (found, s) = collect(std::slice::from_ref(&r), &opts());
        fs::set_permissions(r.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(found.contains(&r.join("open/inner")));
        assert!(found.contains(&r.join("locked")));
        // root bypasses permissions, so only assert the error count when unprivileged.
        if !found.contains(&r.join("locked/inner")) {
            assert_eq!(s.errors, 1);
        }
    }

    #[test]
    fn excluded_paths_and_entry_cap() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().to_path_buf();
        fs::create_dir_all(r.join("virt/x")).unwrap();
        fs::create_dir_all(r.join("ok")).unwrap();
        let mut o = opts();
        o.excluded_paths.insert(r.join("virt"));
        let (found, _) = collect(std::slice::from_ref(&r), &o);
        assert_eq!(found, vec![r.join("ok")]);
        // An excluded path given explicitly as a root is still searched.
        let (found, _) = collect(&[r.join("virt")], &o);
        assert_eq!(found, vec![r.join("virt/x")]);
        o.max_entries = 1;
        let (found, s) = collect(std::slice::from_ref(&r), &o);
        assert_eq!(found.len(), 1);
        assert!(s.truncated);
    }

    #[test]
    fn files_only_when_asked() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().to_path_buf();
        fs::create_dir_all(r.join("a")).unwrap();
        fs::write(r.join("a/notes.txt"), "").unwrap();
        let (found, _) = collect(std::slice::from_ref(&r), &opts());
        assert_eq!(found, vec![r.join("a")]);
        let o = ScanOptions {
            include_files: true,
            ..opts()
        };
        let (found, s) = collect(std::slice::from_ref(&r), &o);
        assert_eq!(found, vec![r.join("a"), r.join("a/notes.txt")]);
        assert_eq!((s.dirs, s.files), (1, 1));
    }

    #[test]
    fn mount_table_exclusions() {
        let table = "proc /proc proc rw 0 0\n\
                     /dev/sda1 / ext4 rw 0 0\n\
                     host:/x /mnt/my\\040share fuse.sshfs rw 0 0\n\
                     systemd-1 /remote/a autofs rw 0 0\n\
                     /dev/sdb1 /data ext4 rw 0 0\n";
        let types = crate::config::Exclusions::default().filesystem_types;
        let got = excluded_mounts(table, &types);
        assert_eq!(
            got,
            vec![
                PathBuf::from("/proc"),
                "/mnt/my share".into(),
                "/remote/a".into()
            ]
        );
    }

    #[test]
    fn background_scan_delivers_everything() {
        let t = tempfile::tempdir().unwrap();
        for i in 0..1500 {
            fs::create_dir(t.path().join(format!("d{i}"))).unwrap();
        }
        let h = spawn(vec![t.path().to_path_buf()], opts());
        let mut n = 0;
        loop {
            match h.rx.recv().unwrap() {
                ScanEvent::Batch(b) => n += b.len(),
                ScanEvent::Done(s) => {
                    assert_eq!(s.dirs, 1500);
                    break;
                }
            }
        }
        assert_eq!(n, 1500);
    }
}
