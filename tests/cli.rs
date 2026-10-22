//! End-to-end tests against the built binary, with isolated HOME/data/config.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Env {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    data: PathBuf,
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let data = tmp.path().join("data");
        fs::create_dir_all(&home).unwrap();
        Self {
            _tmp: tmp,
            home,
            data,
        }
    }

    fn mkdirs(&self, rel: &[&str]) {
        for r in rel {
            fs::create_dir_all(self.home.join(r)).unwrap();
        }
    }

    fn cmd(&self, cwd: &Path) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_cz"));
        c.current_dir(cwd)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", &self.home)
            .env("PWD", cwd)
            .env("CZ_DATA_DIR", &self.data)
            .env("CZ_CONFIG", self.home.join("nonexistent.toml"));
        c
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.cmd(cwd).args(args).output().unwrap()
    }

    fn lines(&self, cwd: &Path, args: &[&str]) -> Vec<String> {
        let o = self.run(cwd, args);
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(String::from)
            .collect()
    }
}

#[test]
fn default_scope_is_cwd_only() {
    let e = Env::new();
    e.mkdirs(&["Projects/CNN-frontend", "archive/CNN-frontend"]);
    let out = e.lines(&e.home.join("Projects"), &["--list", "CNN-frontend"]);
    assert_eq!(
        out,
        vec![e.home.join("Projects/CNN-frontend").to_string_lossy()]
    );
}

#[test]
fn home_flag_and_multiple_paths() {
    let e = Env::new();
    e.mkdirs(&[
        "Projects/CNN-frontend",
        "Uni/CNN-frontend",
        "Other/CNN-frontend",
        "work",
    ]);
    let work = e.home.join("work");
    assert_eq!(e.lines(&work, &["--list", "-H", "CNN-frontend"]).len(), 3);
    let out = e.lines(
        &work,
        &[
            "--list",
            "-p",
            "~/Projects",
            "--path",
            "../Uni",
            "CNN-frontend",
        ],
    );
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|l| !l.contains("Other")));
}

#[test]
fn no_implicit_broadening() {
    let e = Env::new();
    e.mkdirs(&["elsewhere/target-dir", "empty"]);
    let o = e.run(&e.home.join("empty"), &["--list", "target-dir"]);
    assert!(!o.status.success());
    assert!(o.stdout.is_empty());
}

#[test]
fn invalid_path_is_reported() {
    let e = Env::new();
    let o = e.run(&e.home, &["--list", "-p", "/no/such/dir", "x"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("/no/such/dir"));
}

#[test]
fn history_is_global_and_boosts_ranking() {
    let e = Env::new();
    e.mkdirs(&["a/proj", "b/proj", "elsewhere/proj", "cwd"]);
    let cwd = e.home.join("cwd");
    // History outside the scope still appears.
    for _ in 0..3 {
        assert!(
            e.run(&cwd, &["--record", "~/elsewhere/proj"])
                .status
                .success()
        );
    }
    let out = e.lines(&e.home, &["--list", "-p", "a", "-p", "b", "proj"]);
    assert_eq!(out[0], e.home.join("elsewhere/proj").to_string_lossy());
    assert_eq!(out.len(), 3);
    let hist = e.lines(&cwd, &["--history"]);
    assert_eq!(hist.len(), 1);
    assert!(hist[0].contains("~/elsewhere/proj") && hist[0].contains(" 3 "));
}

#[test]
fn record_rejects_missing_and_clear_works() {
    let e = Env::new();
    assert!(!e.run(&e.home, &["--record", "/no/such"]).status.success());
    e.run(&e.home, &["--record", "."]);
    assert_eq!(e.lines(&e.home, &["--history"]).len(), 1);
    assert!(e.run(&e.home, &["--clear-history"]).status.success());
    assert!(e.lines(&e.home, &["--history"]).is_empty());
}

#[test]
fn first_writes_raw_path_to_output_file() {
    let e = Env::new();
    e.mkdirs(&["x/we ird $(x) 'q'\ttab"]);
    let out = e.home.join("out");
    let o = e.run(
        &e.home,
        &["--first", "--output-file", out.to_str().unwrap(), "weird"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        fs::read(&out).unwrap(),
        e.home
            .join("x/we ird $(x) 'q'\ttab")
            .to_str()
            .unwrap()
            .as_bytes()
    );
}

#[test]
fn tui_without_terminal_fails_cleanly() {
    let e = Env::new();
    let o = e.run(&e.home, &["anything"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--list"));
}

/// Runs the generated shell function in bash and zsh against hostile names.
fn shell_roundtrip(shell: &str) {
    if Command::new(shell).arg("-c").arg("true").output().is_err() {
        eprintln!("skipping: {shell} not installed");
        return;
    }
    let e = Env::new();
    let names = [
        "sp ace",
        "$(touch PWNED)",
        "semi;colon",
        "quo'te\"d",
        "back\\slash",
        "-dash",
        "new\nline\n",
    ];
    for n in names {
        fs::create_dir_all(e.home.join("t").join(n)).unwrap();
    }
    for n in names {
        let target = e.home.join("t").join(n);
        let query = match n {
            "-dash" => "-- -dash".to_string(),
            "new\nline\n" => "newline".to_string(),
            _ => format!("'{}'", n.replace('\'', r"'\''")),
        };
        let out = Command::new(shell)
            .arg("-c")
            .arg(format!(
                "eval \"$('{bin}' --init {shell})\"\ncd -- '{t}' || exit 9\ncz --first {query} || exit 8\nprintf %s \"$PWD\"",
                bin = env!("CARGO_BIN_EXE_cz"),
                t = e.home.join("t").display(),
            ))
            .current_dir(&e.home)
            .env("HOME", &e.home)
            .env("CZ_DATA_DIR", &e.data)
            .env("CZ_CONFIG", e.home.join("none.toml"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{shell} {n:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            target.to_string_lossy(),
            "{shell} {n:?}"
        );
    }
    assert!(!e.home.join("t/PWNED").exists() && !e.home.join("PWNED").exists());
    // Every successful cd was recorded.
    assert_eq!(e.lines(&e.home, &["--history"]).len(), names.len());
}

#[test]
fn bash_integration() {
    shell_roundtrip("bash");
}

#[test]
fn zsh_integration() {
    shell_roundtrip("zsh");
}

#[test]
fn cancel_leaves_directory_and_history_unchanged() {
    let e = Env::new();
    e.mkdirs(&["t/x"]);
    // No match → binary fails → function must not cd or record.
    let out = Command::new("bash")
        .arg("-c")
        .arg(format!(
            "eval \"$('{}' --init bash)\"\ncd -- '{}'\ncz --first zzzz; s=$?\nprintf '%s|%s' \"$s\" \"$PWD\"",
            env!("CARGO_BIN_EXE_cz"),
            e.home.join("t").display()
        ))
        .env("HOME", &e.home)
        .env("CZ_DATA_DIR", &e.data)
        .env("CZ_CONFIG", e.home.join("none.toml"))
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("1|{}", e.home.join("t").display())
    );
    assert!(e.lines(&e.home, &["--history"]).is_empty());
}

/// Runs `body` in `shell` with the cz function loaded and an isolated env.
fn in_shell(shell: &str, e: &Env, cwd: &Path, body: &str) -> Output {
    Command::new(shell)
        .arg("-c")
        .arg(format!(
            "eval \"$('{}' --init {shell})\"\ncd -- '{}' || exit 9\n{body}",
            env!("CARGO_BIN_EXE_cz"),
            cwd.display()
        ))
        .env("HOME", &e.home)
        .env("CZ_DATA_DIR", &e.data)
        .env("CZ_CONFIG", e.home.join("none.toml"))
        .output()
        .unwrap()
}

fn pwd_after(shell: &str, e: &Env, cwd: &Path, cmd: &str) -> String {
    let o = in_shell(
        shell,
        e,
        cwd,
        &format!("{cmd} >/dev/null 2>&1; printf %s \"$PWD\""),
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn behaves_like_cd() {
    let e = Env::new();
    e.mkdirs(&["a/b", "other"]);
    let b = e.home.join("a/b");
    let s = |p: &Path| p.to_string_lossy().into_owned();
    for sh in ["bash", "zsh"] {
        assert_eq!(pwd_after(sh, &e, &b, "cz"), s(&e.home), "{sh}: cz");
        assert_eq!(
            pwd_after(sh, &e, &b, "cz .."),
            s(&e.home.join("a")),
            "{sh}: cz .."
        );
        assert_eq!(
            pwd_after(sh, &e, &b, "cz ~/other"),
            s(&e.home.join("other")),
            "{sh}: cz ~/other"
        );
        assert_eq!(pwd_after(sh, &e, &b, "cz / && cz -"), s(&b), "{sh}: cz -");
    }
}

#[test]
fn clear_match_jumps_without_picker_and_ambiguous_does_not() {
    let e = Env::new();
    e.mkdirs(&["p/kicad", "p/notes", "x/dup", "y/dup"]);
    // No terminal here: a jump must happen without ever opening the UI.
    assert_eq!(
        pwd_after("bash", &e, &e.home, "cz -H kicad"),
        e.home.join("p/kicad").to_string_lossy()
    );
    assert_eq!(
        pwd_after("zsh", &e, &e.home, "cz kicad"),
        e.home.join("p/kicad").to_string_lossy()
    );
    // Two equally good matches need the picker, which can't open here → stay put.
    assert_eq!(
        pwd_after("bash", &e, &e.home, "cz dup"),
        e.home.to_string_lossy()
    );
    // czi always wants the picker, even for an existing path.
    assert_eq!(
        pwd_after("bash", &e, &e.home, "czi p"),
        e.home.to_string_lossy()
    );
}
