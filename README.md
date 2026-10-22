# cz — intelligent directory navigator

`cz <query>` merges two sources into one interactive, fuzzy-ranked list and
`cd`s your shell to the pick:

* **Global history:** every directory you have jumped to with cz, ranked by
  frequency and recency (frecency). It is searched regardless of scope.
* **Scoped discovery:** a live, background walk of *only* the roots you chose
  (the current directory by default).

cz is self-contained. It does not depend on zoxide, fzf, or fd at runtime.

## Install

**Requirements:** Linux on x86-64. The release binary is fully static, so it
runs on any distribution with no dependencies. zsh or bash is needed for the
shell integration.

### Quick install

```sh
curl -fsSL https://raw.githubusercontent.com/piumalnipun9/cz/main/install.sh | sh
```

This downloads the latest release, verifies its SHA-256 checksum and installs
`cz` to `~/.local/bin`. Options:

* `CZ_VERSION=v0.1.0` installs a specific version.
* `CZ_INSTALL_DIR=/some/dir` installs somewhere else.

For example:
`curl -fsSL https://raw.githubusercontent.com/piumalnipun9/cz/main/install.sh | CZ_INSTALL_DIR=~/bin sh`

### Manual download

```sh
VERSION=v0.1.0
curl -fLO https://github.com/piumalnipun9/cz/releases/download/$VERSION/cz-$VERSION-x86_64-unknown-linux-musl.tar.gz
curl -fLO https://github.com/piumalnipun9/cz/releases/download/$VERSION/cz-$VERSION-x86_64-unknown-linux-musl.tar.gz.sha256
sha256sum -c cz-$VERSION-x86_64-unknown-linux-musl.tar.gz.sha256
tar -xzf cz-$VERSION-x86_64-unknown-linux-musl.tar.gz
install -m 755 cz-$VERSION-x86_64-unknown-linux-musl/cz ~/.local/bin/cz
```

### From source

```sh
git clone https://github.com/piumalnipun9/cz && cd cz
cargo install --path .            # puts `cz` in ~/.cargo/bin
```

### Shell setup (required)

`cz` changes your shell's directory, so it needs a small shell function. Make
sure the install directory is on your `PATH`, then add one line to your rc
file and open a new terminal:

```sh
# zsh
echo 'eval "$(cz --init zsh)"' >> ~/.zshrc
# bash
echo 'eval "$(cz --init bash)"' >> ~/.bashrc
```

The function is called `cz` and shadows the binary. It calls the binary by the
absolute path baked in at `--init` time. If you move the binary, open a new
terminal (or re-run the `eval` line) to pick up the new location.

### Uninstall

Remove the `eval` line from your rc file, then:

```sh
rm ~/.local/bin/cz                        # or ~/.cargo/bin/cz
rm -rf ~/.local/share/cz ~/.config/cz     # history and config
```

## Usage

`cz` behaves like `cd` first and like `z` second:

| You type                     | What happens                                             |
|------------------------------|----------------------------------------------------------|
| `cz`                         | go home, like `cd`                                       |
| `cz -`                       | go back to the previous directory, like `cd -`           |
| `cz ~`, `cz ..`, `cz /etc`, `cz src` | if the argument is an existing directory, go there directly |
| `cz kicad`                   | search; jump directly if one match is clearly best, otherwise open the picker |
| `cz -i kicad`                | always open the picker                                   |

**When a search jumps directly.** cz waits up to `jump.wait_ms` (400 ms) for
the scan to finish. It then jumps if the best match has quality ≥ 0.6 and one
of these holds:

* it is the only match;
* it is an exact name match and the runner-up is not;
* it scores at least `jump.margin` (1.25) × the runner-up.

Otherwise the picker opens. Each pick is recorded, and history multiplies the
score, so a directory you picked once or twice recently pulls ahead of others
with the same name and starts jumping directly. As recency decays, close
matches go back to the picker. A slow scan (for example `-r`) always ends in
the picker, which keeps updating as the scan continues.

Scope flags:

| Command                           | Filesystem scope                     |
|-----------------------------------|--------------------------------------|
| `cz <query>`                      | current directory (default)          |
| `cz -H <query>` / `--home`        | your home directory                  |
| `cz -r <query>` / `--root`        | `/` (virtual/network mounts skipped) |
| `cz -p DIR <query>` / `--path`    | DIR                                  |
| `cz -p A -p B <query>`            | A and B                              |

* Flags may appear before or after the query. Multiple words are matched
  independently, so `cz cnn front` works. Use `--` for a query that starts
  with `-`.
* **Scope flags combine.** `-H -p /srv` searches `~` and `/srv`. Roots nested
  in another root are dropped (`-r -H` is just `/`). No flag ever widens
  another flag's scope, and cz never widens the scope on its own when nothing
  matches.
* `-p` accepts `~`, relative paths, and paths with spaces (quote them as
  usual). `$VAR` is expanded only if the literal path does not exist and the
  variable is set, because the shell normally expands variables already.
  Invalid roots are an error (exit status 2) naming the path.
* `-h` is help; `-H` is home.

Other commands:

```
cz --history          # learned directories: score, visits, last visit
cz --stats            # database location, counts, top directories
cz --prune            # forget directories that no longer exist
cz --clear-history    # asks for confirmation on a TTY; -y skips it
cz --list <query>     # print ranked matches (no UI); handy for scripting
cz --first <query>    # take the best match without the UI
```

### Keys

| Key                              | Action                                  |
|----------------------------------|-----------------------------------------|
| typing                           | live fuzzy filter                       |
| ↑/↓, Ctrl-P/N, Ctrl-K/J          | move selection                          |
| PgUp/PgDn                        | move by a page                          |
| **Enter**                        | jump to the highlighted directory       |
| **Tab**                          | same as Enter: select and jump          |
| **Esc**                          | cancel; directory unchanged (status 1)  |
| **Ctrl-C**                       | quit; directory unchanged (status 130)  |
| Ctrl-U / Ctrl-W                  | clear query / delete previous word      |
| ←/→, Home/End, Ctrl-A/E          | move the cursor                         |

Each row shows a source badge: `hist` (history only), `fs` (found by the
scan), or `both`. The header shows the active roots, a scanning spinner with a
live directory count, and the number of matches. With `ui.show_preview` on and
a terminal at least 100 columns wide, a side pane lists the highlighted
directory's contents.

## Configuration

`$CZ_CONFIG`, else `$XDG_CONFIG_HOME/cz/config.toml`, else
`~/.config/cz/config.toml`. Every key is optional. Unknown keys are an error,
so typos don't go unnoticed.

```toml
[search]
include_hidden = true
max_depth = 15               # levels below each root
max_results = 100
follow_symlinks = false      # if true, (dev, inode) tracking prevents loops
max_scan_entries = 2000000   # memory guard for -r

[search.exclusions]
directories = [".git", "node_modules", "target", ".cache", ".venv", "venv", "__pycache__"]
paths = ["/proc", "/sys", "/dev", "/run"]
filesystem_types = ["proc", "sysfs", "autofs", "squashfs", "nfs", "nfs4", "cifs", "sshfs", "fuse.*"]  # (abridged)

[ranking]
match_weight = 1.0
frequency_weight = 1.0
recency_weight = 1.0
history_boost = 0.25
frequency_saturation = 100
recency_half_life_days = 7.0

[jump]
auto = true          # false = always open the picker for searches
margin = 1.25
min_quality = 0.6
wait_ms = 400

[ui]
show_scores = false
show_preview = true
```

Search roots are never set in the config. They come only from CLI flags.

History is stored in `$CZ_DATA_DIR/history.db`, else
`$XDG_DATA_HOME/cz/history.db`, else `~/.local/share/cz/history.db`. It is a
SQLite database in WAL mode, and paths are stored as raw bytes.

## How it works

### Architecture

```
src/
  main.rs              wiring: parse → load config/db → engine → TUI or --list
  cli/                 clap definitions, scope resolution and root dedup
  config/              serde/TOML config with defaults
  paths.rs             ~/$VAR expansion, lexical normalization, display
  database/            rusqlite: migrations (user_version), record/list/prune
  search/
    fuzzy.rs           nucleo-matcher → normalized match quality + highlights
    ranking.rs         frecency signals and the scoring formula
    merge.rs           CandidateSet: dedup by path, history/fs/both sources
    history.rs         loading and frecency ordering of history
    filesystem.rs      background BFS scanner (thread + channel, cancellable)
    mod.rs             Engine: incremental matching and top-K ranking
  tui/                 app state (pure, unit-tested), key map, rendering, loop
  shell/integration.rs the bash/zsh function
```

Data flow: history is loaded and inserted into the `Engine` first, so results
appear immediately. The scanner thread streams directory batches over a
channel. The UI loop drains them within a time budget of about 25 ms per
frame, matches only the new candidates, and re-ranks.

The scan does not depend on the query: it collects directories once and the
query re-filters them in memory. Typing therefore never restarts filesystem
I/O, so no obsolete searches ever need cancelling. Dropping the scan handle
(on exit) cancels the walk. When a keystroke only appends to the query, only
the previous matches are re-scored. A burst of keystrokes, such as a paste,
causes a single re-rank.

### Matching

* A query without `/` matches the directory **basename**. A query with `/`
  matches the full displayed path (`~/Projects/CNN-frontend`). Otherwise `pro`
  would match every directory under `~/Projects`.
* Matching is case-insensitive. Words are ANDed and can match in any order.
* `quality = 0.7·fuzzy + 0.3·tier`, where:
  * `fuzzy` is nucleo's score divided by the score the query would get
    matching itself, clamped to 1.
  * `tier` is 1.0 for an exact basename, 0.6 for a prefix, 0.3 for a
    contiguous substring, and 0 for a scattered match.

### Ranking

```
score = quality × (match_weight + history_boost × (frequency_weight·F + recency_weight·R))
F = min(1, ln(1+visits) / ln(1+frequency_saturation))
R = 0.5 ^ (days_since_last_visit / recency_half_life_days)
```

History multiplies match quality instead of being added to it, so a weak match
cannot be rescued by heavy use. With the defaults, the largest possible
history multiplier is 1.5. A history entry therefore needs quality above about
0.67 to outrank an exact match that has never been visited. Among similar
matches, frequent and recent directories win.

Ties are broken in this order: history before filesystem, then shorter path,
then alphabetical. The tests in `search/ranking.rs` and `search/mod.rs` pin
these properties down.

Paths are normalized lexically, without resolving symlinks, so they match the
shell's logical `$PWD`. Each path is one candidate even when both sources
report it. History entries whose directory has disappeared are hidden from
results, and `--prune` deletes them.

### Shell integration (no `eval` of output)

A child process cannot change its parent shell's directory, so `cz` is a shell
function:

1. It creates a private temp file with `mktemp` and runs the binary with
   `--output-file <tmp>`. The TUI draws on stderr.
2. When you select a directory, the binary writes the raw path bytes to that
   file and exits 0. When you cancel, it writes nothing and exits non-zero.
3. The function reads the file as data. A sentinel character keeps trailing
   newlines intact. It then runs `builtin cd -- "$dir"`. The path is never
   evaluated, so a directory named `$(rm -rf ~)` is just a name.
4. Only after `cd` succeeds does it run `cz --record "$PWD"`. Cancelled or
   failed jumps are never learned.

### Scope safety

* Every discovered path is `parent.join(name)` starting from a validated root,
  so nothing escapes the roots.
* Symlinks are not followed by default.
* Unreadable or vanished directories are counted (shown as "N unreadable") and
  skipped.
* The scanner never enters:
  * `/proc`, `/sys`, `/dev`, `/run`;
  * any mount point whose filesystem type is excluded, read from
    `/proc/self/mounts`: virtual, network (NFS/CIFS/sshfs), FUSE, autofs, and
    snap squashfs.

  This avoids triggering automounts and hanging on unreachable network shares.
  A root you pass explicitly is still searched.

## Tests

`cargo test` runs:

* unit tests for every module (CLI parsing, root dedup, normalization, config,
  database, matcher, ranking properties, scanner scope/symlink/permission
  behavior, mount exclusions, and UI state);
* end-to-end tests of the binary;
* real bash **and** zsh sessions that `cz` into directories named
  `sp ace`, `$(touch PWNED)`, `semi;colon`, `quo'te"d`, `back\slash`, `-dash`,
  and one containing newlines. They check `$PWD`, check that nothing was
  executed, check that history was recorded, and check that a failed or
  cancelled search changes neither the directory nor the history.

## Not yet implemented (designed for)

These are planned: bookmarks, git-repository and project detection (an
`is_git` flag is already stored per directory), file navigation, multiple
selection, and search filters. The `Engine` and `CandidateSet` take new
sources the same way they take history and scan results.

Known limits:

* Matching runs on the UI thread. For `-r` on very large trees, a re-rank that
  is not a simple narrowing (for example, after a backspace) touches every
  candidate. It is fast in practice, but it is the first thing to move to a
  worker thread if needed.
* History never ages out.

## License

MIT © 2026 Piumal Nipun. See [LICENSE](LICENSE).
