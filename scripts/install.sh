#!/bin/sh
set -eu

repo_root=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
cd "$repo_root"
dest="${A2AMX_INSTALL_DIR:-$HOME/.a2amx}"
mkdir -p "$dest/bin"
cargo build --release --locked

tmp="$dest/bin/.a2amx.$$"
trap 'rm -f "$tmp"' 0
trap 'exit 1' HUP INT TERM
cp target/release/a2amx "$tmp"
chmod 755 "$tmp"
mv -f "$tmp" "$dest/bin/a2amx"

version=$("$dest/bin/a2amx" --version)
printf 'installed %s to %s\n' "$version" "$dest/bin/a2amx"
printf '%s\n' 'a running daemon keeps its old binary; restart it to use this one (sessions end).'
