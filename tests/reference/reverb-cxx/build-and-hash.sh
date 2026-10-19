#!/usr/bin/env bash
# Rebuild the C++ reference for the FX1 reverb golden test and print the SHA-256 of each scenario's output.
#
#   build-and-hash.sh            build with g++ and clang++, render every scenario, check the two compilers agree,
#                                print "<name> <sha256>" lines, and diff them against the committed fixture
#   build-and-hash.sh --write    also overwrite the fixture (crates/enth-dsp/tests/fixtures/reverb-cxx-sha256.txt)
#
# The reference is upstream's header-only reverb, unmodified, at the commits pinned in docs/dsp/reverb.md, built at
# -O2 for baseline x86-64 (no FMA) with -ffp-contract=off and no fast-math: the condition for bit-exactness.
# Environment: WORKTREE (default: this repo), UPSTREAM (default ~/.cache/enth/reverb-upstream; cloned at the pinned commits if missing).
set -euo pipefail
unset DISPLAY WAYLAND_DISPLAY
here="$(cd "$(dirname "$0")" && pwd)"
worktree="${WORKTREE:-$(cd "$here/../../.." && pwd)}"
upstream="${UPSTREAM:-$HOME/.cache/enth/reverb-upstream}"
spec="$worktree/crates/enth-dsp/tests/fixtures/reverb-scenarios.txt"
fixture="$worktree/crates/enth-dsp/tests/fixtures/reverb-cxx-sha256.txt"
basics="$upstream/Signalsmith-Audio_basics/repo"
dsp="$upstream/Signalsmith-Audio_dsp/repo"
out="${OUT:-$(mktemp -d)}"
mkdir -p "$out"

want_basics=369e906e03760ec219d07b06fa06dc4fd099f377
want_dsp=ca390d28c41ea5108a528e2e81faa4088f2f8ac8
clone_at() { # url dir commit
  [ -d "$2/.git" ] && return 0
  mkdir -p "$2" && git -C "$2" init -q && git -C "$2" remote add origin "$1" && git -C "$2" fetch -q --depth 1 origin "$3" && git -C "$2" checkout -q FETCH_HEAD
}
clone_at https://github.com/Signalsmith-Audio/basics "$basics" "$want_basics"
clone_at https://github.com/Signalsmith-Audio/dsp "$dsp" "$want_dsp"
[ "$(git -C "$basics" rev-parse HEAD)" = "$want_basics" ] || { echo "basics is not at $want_basics" >&2; exit 1; }
[ "$(git -C "$dsp" rev-parse HEAD)" = "$want_dsp" ] || { echo "dsp is not at $want_dsp" >&2; exit 1; }
[ -z "$(git -C "$basics" status --porcelain)" ] && [ -z "$(git -C "$dsp" status --porcelain)" ] || { echo "upstream clones are modified" >&2; exit 1; }

flags=(-std=c++17 -O2 -ffp-contract=off -I "$dsp/include" -I "$basics/include")
g++ "${flags[@]}" -o "$out/h-gcc" "$here/harness.cpp"
clang++ "${flags[@]}" -o "$out/h-clang" "$here/harness.cpp"
g++ -std=c++17 -O2 -o "$out/tables" "$here/tables.cpp"
"$out/tables" > "$out/tables-libstdcxx.txt"

: > "$out/hashes.txt"
for name in $("$out/h-gcc" "$spec" --list); do
  "$out/h-gcc" "$spec" "$name" "$out/$name.gcc.f32"
  "$out/h-clang" "$spec" "$name" "$out/$name.clang.f32"
  a="$(sha256sum < "$out/$name.gcc.f32" | cut -d' ' -f1)"
  b="$(sha256sum < "$out/$name.clang.f32" | cut -d' ' -f1)"
  if [ "$a" != "$b" ]; then echo "MISMATCH g++ vs clang++ in $name: $a $b" >&2; exit 1; fi
  echo "$name $a" >> "$out/hashes.txt"
done
cat "$out/hashes.txt"
if [ "${1:-}" = "--write" ]; then cp "$out/hashes.txt" "$fixture"; echo "wrote $fixture"; fi
if diff -u "$fixture" "$out/hashes.txt"; then echo "fixture matches the C++ output"; else echo "FIXTURE DIFFERS" >&2; exit 1; fi
