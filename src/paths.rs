//! Path helpers: home lookup, `~`/`$VAR` expansion, lexical normalization and
//! display formatting. Normalization is purely lexical (no symlink resolution)
//! so it agrees with the shell's logical `$PWD`.

use std::env;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};

/// The current user's home directory (`$HOME`, falling back to the passwd entry).
pub fn home_dir() -> Option<PathBuf> {
    match env::var_os("HOME") {
        Some(h) if !h.is_empty() => Some(PathBuf::from(h)),
        #[allow(deprecated)]
        _ => env::home_dir(),
    }
}

/// Expands a leading `~` / `~/` and, when the literal path does not exist,
/// `$VAR` / `${VAR}` references to set environment variables.
///
/// Environment expansion is conservative because the shell normally expands
/// variables before cz sees them; a literal `$` that survived quoting is most
/// likely part of a real directory name, so we only expand when the literal
/// path is missing.
pub fn expand(raw: &OsStr) -> PathBuf {
    let bytes = raw.as_bytes();
    let tilde = if bytes == b"~" {
        home_dir()
    } else if let Some(rest) = bytes.strip_prefix(b"~/") {
        home_dir().map(|h| h.join(OsStr::from_bytes(rest)))
    } else {
        None
    };
    let path = tilde.unwrap_or_else(|| PathBuf::from(raw));
    if path.as_os_str().as_bytes().contains(&b'$') && !path.exists() {
        let expanded = expand_env(path.as_os_str().as_bytes());
        if let Some(e) = expanded {
            return PathBuf::from(OsString::from_vec(e));
        }
    }
    path
}

/// Expands `$NAME` and `${NAME}`; returns `None` if any referenced variable is unset.
fn expand_env(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'$' {
            out.push(s[i]);
            i += 1;
            continue;
        }
        let (name, next) = if s.get(i + 1) == Some(&b'{') {
            let end = s[i + 2..].iter().position(|&c| c == b'}')? + i + 2;
            (&s[i + 2..end], end + 1)
        } else {
            let len = s[i + 1..]
                .iter()
                .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
                .count();
            (&s[i + 1..i + 1 + len], i + 1 + len)
        };
        if name.is_empty() {
            out.push(b'$');
            i += 1;
            continue;
        }
        let val = env::var_os(OsStr::from_bytes(name))?;
        out.extend_from_slice(val.as_bytes());
        i = next;
    }
    Some(out)
}

/// Makes `path` absolute relative to `cwd` and removes `.`/`..` components lexically.
pub fn normalize(path: &Path, cwd: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let mut out = PathBuf::new();
    for comp in joined.components() {
        match comp {
            Component::RootDir => out.push("/"),
            Component::CurDir | Component::Prefix(_) => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(c) => out.push(c),
        }
    }
    if out.as_os_str().is_empty() {
        out.push("/");
    }
    out
}

/// True if `path` equals `root` or lies below it (component-wise, lexical).
pub fn is_within(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

/// Human-friendly rendering: the home prefix becomes `~`, invalid UTF-8 is
/// replaced and control characters are escaped (`\n`) so one path is always
/// one line. Only for display/matching; the real path is kept separately.
pub fn display(path: &Path, home: Option<&Path>) -> String {
    let s = display_raw(path, home);
    if s.chars().any(char::is_control) {
        s.chars()
            .flat_map(|c| {
                if c.is_control() {
                    c.escape_default().collect::<Vec<_>>()
                } else {
                    vec![c]
                }
            })
            .collect()
    } else {
        s
    }
}

fn display_raw(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && home != Path::new("/")
        && let Ok(rest) = path.strip_prefix(home)
    {
        return if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rest.to_string_lossy())
        };
    }
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_lexically() {
        let cwd = Path::new("/a/b");
        assert_eq!(
            normalize(Path::new("c/../d/./e"), cwd),
            PathBuf::from("/a/b/d/e")
        );
        assert_eq!(normalize(Path::new("/x/../.."), cwd), PathBuf::from("/"));
        assert_eq!(normalize(Path::new("/x//y/"), cwd), PathBuf::from("/x/y"));
    }

    #[test]
    fn within_is_component_wise() {
        assert!(is_within(Path::new("/a/bc/d"), Path::new("/a/bc")));
        assert!(!is_within(Path::new("/a/bcd"), Path::new("/a/bc")));
    }

    #[test]
    fn expands_env_only_when_set() {
        // SAFETY: tests in this module do not read CZ_TEST_VAR concurrently.
        unsafe { env::set_var("CZ_TEST_VAR", "/tmp/xyz") };
        assert_eq!(expand_env(b"${CZ_TEST_VAR}/a").unwrap(), b"/tmp/xyz/a");
        assert_eq!(expand_env(b"$CZ_TEST_VAR/a").unwrap(), b"/tmp/xyz/a");
        assert!(expand_env(b"$CZ_DEFINITELY_UNSET_VAR").is_none());
    }

    #[test]
    fn displays_home_as_tilde() {
        let home = Path::new("/home/u");
        assert_eq!(display(Path::new("/home/u/p q"), Some(home)), "~/p q");
        assert_eq!(display(Path::new("/home/uu"), Some(home)), "/home/uu");
        assert_eq!(display(Path::new("/a\nb"), None), "/a\\nb");
    }
}
