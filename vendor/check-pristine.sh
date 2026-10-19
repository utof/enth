#!/usr/bin/env bash
# Compare vendor/ against the pristine crates.io packages it was imported from.
#
#   vendor/check-pristine.sh            diffstat of vendor/ against the pristine sources (the patches, and nothing else)
#   vendor/check-pristine.sh --full     the whole unified diff
#   vendor/check-pristine.sh --at REV   check the vendor/ tree of a git revision instead of the working tree
#
# The pristine sources are the five `.crate` files named in vendor/README.md. The script finds them in cargo's registry
# cache (run `cargo fetch` first if they are missing), checks each sha256 against vendor/README.md, unpacks them into a
# temporary directory and diffs. Nothing is downloaded and nothing under vendor/ is changed.
#
# Files that are added on purpose (the ISC licence files, which the nice-plug packages lack) show up as "new file".
# Exit status: 0 when every checksum matches and the diff was produced, 1 on a checksum mismatch, 2 on anything else.
set -Eeuo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
mode=stat
rev=
while [ $# -gt 0 ]; do
  case $1 in
    --full) mode=full ;;
    --at) rev=${2:?--at needs a revision}; shift ;;
    *) echo "usage: $0 [--full] [--at REV]" >&2; exit 2 ;;
  esac
  shift
done

# directory under vendor/ | package | version | sha256 of the .crate (equal to the checksum in Cargo.lock)
crates=(
  "nice-plug/crates/nice-plug|nice-plug|0.4.2|3f47be74bed3e8aa17bb055d4ff89af1fb016aa0d2756cd9b6c029a3743676a5"
  "nice-plug/crates/nice-plug-core|nice-plug-core|0.4.2|ab8fe3f3e395c1140d7519a78069b59b2e3bf6da2b904d0c54ad7ab49a60ba09"
  "nice-plug/crates/nice-plug-egui|nice-plug-egui|0.6.0|54dd608bf12c94c7bdab22cd41122cd0e910454a2e6e59bd6a46ca0fb000c90e"
  "baseview|baseview|0.4.1|4179c1b5e41cec78a6b000eaa8317ddbc2556124dabc7344ff1df89d566cc517"
  "egui-baseview|egui-baseview|0.9.0|de6df8bd2f1c6b481ab6c029f741980d1a7b409053178fb9459053ae457828fe"
)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cargo_home=${CARGO_HOME:-$HOME/.cargo}

if [ -n "$rev" ]; then
  tree=$work/tree
  mkdir -p "$tree"
  git -C "$here" archive "$rev" vendor | tar -x -C "$tree"
  vendored=$tree/vendor
else
  vendored=$here
fi

status=0
for entry in "${crates[@]}"; do
  IFS='|' read -r dir package version sha <<<"$entry"
  file=$(ls "$cargo_home"/registry/cache/*/"$package-$version.crate" 2>/dev/null | head -n 1 || true)
  if [ -z "$file" ]; then
    echo "check-pristine: $package-$version.crate is not in cargo's registry cache (run cargo fetch)" >&2
    exit 2
  fi
  actual=$(sha256sum "$file" | cut -d' ' -f1)
  if [ "$actual" != "$sha" ]; then
    echo "check-pristine: $package-$version.crate has sha256 $actual, expected $sha" >&2
    status=1
    continue
  fi
  mkdir -p "$work/pristine/$dir"
  tar -xzf "$file" --strip-components=1 -C "$work/pristine/$dir"
done
[ "$status" -eq 0 ] || exit "$status"

# `git diff --no-index` compares the directories as they are (no .gitignore, no index), which is what is wanted: a file a
# nested .gitignore would hide (Cargo.lock in baseview) is still part of the import.
cd "$work"
for entry in "${crates[@]}"; do
  IFS='|' read -r dir _ _ _ <<<"$entry"
  echo "== $dir"
  if [ "$mode" = full ]; then
    git diff --no-index --no-color -- "pristine/$dir" "$vendored/$dir" || true
  else
    git diff --no-index --no-color --stat=120 -- "pristine/$dir" "$vendored/$dir" || true
  fi
done
