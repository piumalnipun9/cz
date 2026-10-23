//! Shell integration.
//!
//! A child process cannot change its parent's working directory, so `cz` is a
//! shell function wrapping the binary:
//!
//! 1. `__cz_go` creates a private temp file (`mktemp`, mode 0600) and runs
//!    the binary with `--output-file <tmp>`. The TUI draws on stderr, so the
//!    terminal is untouched and stdout stays free for `--history` etc.
//! 2. On selection the binary writes the raw path bytes (no newline, no
//!    quoting) to the file and exits 0. Cancel exits non-zero and writes nothing.
//! 3. The function reads the file as *data* (`cat` plus a sentinel char so
//!    trailing newlines in a directory name survive command substitution),
//!    then runs `builtin cd -- "$dir"`. Nothing is ever `eval`ed, so names like
//!    `$(rm -rf ~)` or `a; b` are just directory names.
//!    A cancelled picker (status 130) returns 0, so prompts don't flag an error.
//! 4. Only if `cd` succeeds does it call `--record "$PWD"`, so cancelled or
//!    failed navigation is never learned.
//!
//! `czi` always opens the picker and also lists files; picking a file goes to
//! the directory containing it.
//!
//! `cz` alone and `cz -` are handled here with `builtin cd`, so they behave
//! exactly like `cd` and `cd -` (the shell tracks the previous directory).
//!
//! The only `eval` is the user's own `eval "$(cz --init zsh)"`, which evaluates
//! this fixed script, not search output.

use crate::cli::Shell;
use std::path::Path;

/// Quotes `s` for POSIX shells using single quotes.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

const TEMPLATE: &str = r#"# cz shell integration (__SHELL__). Load with:
#   eval "$(cz --init __SHELL__)"
__cz_bin=__BIN__

__cz_go() {
    local __cz_tmp __cz_dir __cz_status
    __cz_tmp=$(command mktemp "${TMPDIR:-/tmp}/cz.XXXXXXXX") || return 1
    command "$__cz_bin" --output-file "$__cz_tmp" "$@"
    __cz_status=$?
    if [ "$__cz_status" -eq 0 ] && [ -s "$__cz_tmp" ]; then
        __cz_dir=$(command cat -- "$__cz_tmp"; printf x)
        __cz_dir=${__cz_dir%x}
        command rm -f -- "$__cz_tmp"
        builtin cd -- "$__cz_dir" || return
        command "$__cz_bin" --record "$PWD" 2>/dev/null
        return 0
    fi
    command rm -f -- "$__cz_tmp"
    # 130 is a cancelled picker (Esc/Ctrl-C): not an error, so keep the prompt calm.
    [ "$__cz_status" -eq 130 ] && return 0
    return "$__cz_status"
}

cz() {
    # Like cd: no arguments goes home, `-` goes back.
    if [ "$#" -eq 0 ]; then
        builtin cd || return
        command "$__cz_bin" --record "$PWD" 2>/dev/null
        return 0
    fi
    if [ "$#" -eq 1 ] && [ "$1" = "-" ]; then
        builtin cd - || return
        command "$__cz_bin" --record "$PWD" 2>/dev/null
        return 0
    fi
    case "$1" in
        -h|--help|-V|--version|--history|--clear-history|--stats|--prune|--init|--record|--list)
            command "$__cz_bin" "$@"
            return
            ;;
    esac
    __cz_go "$@"
}

# czi: interactive picker over directories and files (like zoxide's `zi`).
czi() {
    __cz_go --pick "$@"
}
"#;

pub fn script(shell: Shell, bin: &Path) -> String {
    let name = match shell {
        Shell::Bash => "bash",
        Shell::Zsh => "zsh",
    };
    TEMPLATE
        .replace("__SHELL__", name)
        .replace("__BIN__", &sh_quote(&bin.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_is_safe() {
        assert_eq!(sh_quote("a'b $(x)"), r"'a'\''b $(x)'");
    }

    #[test]
    fn script_embeds_quoted_binary() {
        let s = script(Shell::Zsh, Path::new("/opt/my bin/cz"));
        assert!(s.contains("__cz_bin='/opt/my bin/cz'"));
        assert!(!s.contains("eval \"$__cz"));
        assert!(s.contains("czi() {"));
    }
}
