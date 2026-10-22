#!/bin/sh
# cz installer: downloads a release binary, verifies its SHA-256 checksum and
# installs it. Usage:
#
#   curl -fsSL https://raw.githubusercontent.com/piumalnipun9/cz/main/install.sh | sh
#
# Environment:
#   CZ_VERSION      release tag to install (default: latest), e.g. v0.1.0
#   CZ_INSTALL_DIR  where to put the binary (default: ~/.local/bin)

set -eu

REPO="piumalnipun9/cz"
TARGET="x86_64-unknown-linux-musl"
INSTALL_DIR="${CZ_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${CZ_VERSION:-latest}"

err() {
    printf 'cz install: %s\n' "$1" >&2
    exit 1
}

[ "$(uname -s)" = "Linux" ] || err "only Linux is supported (found $(uname -s))"
case "$(uname -m)" in
    x86_64 | amd64) ;;
    *) err "only x86_64 is supported (found $(uname -m))" ;;
esac

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
    fetch_url() { curl -fsSLI -o /dev/null -w '%{url_effective}' "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO "$2" "$1"; }
    fetch_url() { wget -q --max-redirect=5 -S --spider "$1" 2>&1 | sed -n 's/^ *[Ll]ocation: //p' | tail -1 | tr -d '\r'; }
else
    err "curl or wget is required"
fi

if [ "$VERSION" = "latest" ]; then
    # The /releases/latest page redirects to /releases/tag/<version>.
    VERSION=$(fetch_url "https://github.com/$REPO/releases/latest" | sed 's|.*/tag/||')
    [ -n "$VERSION" ] || err "could not determine the latest release"
fi

ASSET="cz-$VERSION-$TARGET.tar.gz"
BASE="https://github.com/$REPO/releases/download/$VERSION"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

printf 'Downloading cz %s...\n' "$VERSION"
fetch "$BASE/$ASSET" "$TMP/$ASSET" || err "download failed: $BASE/$ASSET"
fetch "$BASE/$ASSET.sha256" "$TMP/$ASSET.sha256" || err "checksum download failed"

expected=$(cut -d' ' -f1 "$TMP/$ASSET.sha256")
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$TMP/$ASSET" | cut -d' ' -f1)
else
    actual=$(shasum -a 256 "$TMP/$ASSET" | cut -d' ' -f1)
fi
[ "$expected" = "$actual" ] || err "checksum mismatch (expected $expected, got $actual)"

tar -xzf "$TMP/$ASSET" -C "$TMP"
mkdir -p "$INSTALL_DIR"
install -m 755 "$TMP/cz-$VERSION-$TARGET/cz" "$INSTALL_DIR/cz"
printf 'Installed cz %s to %s/cz\n\n' "$VERSION" "$INSTALL_DIR"

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) printf 'Note: %s is not on your PATH. Add this to your shell rc file:\n  export PATH="%s:$PATH"\n\n' "$INSTALL_DIR" "$INSTALL_DIR" ;;
esac

cat <<'EOF'
Finish setup (cz can't change directories without this):

  1. Add the shell function to your rc file:
       zsh:   echo 'eval "$(cz --init zsh)"' >> ~/.zshrc
       bash:  echo 'eval "$(cz --init bash)"' >> ~/.bashrc

  2. Reload your shell. Already-open terminals won't see it otherwise:
       zsh:   source ~/.zshrc
       bash:  source ~/.bashrc
     (or open a new terminal)

  3. Check:  type cz   ->  should say "cz is a shell function"
EOF
