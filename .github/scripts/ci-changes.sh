#!/usr/bin/env bash
# Decide which CI work a change needs, from the files it touches.
#
# Usage: ci-changes.sh [BASE [HEAD]]
#   Compares BASE..HEAD (HEAD defaults to HEAD). Prints key=value lines for
#   $GITHUB_OUTPUT:
#
#   rust    run the Rust job at all (fmt, clippy, release tests, smoke run)
#   core    also run the slow debug test pass of accrete-core (golden hashes
#           must match in debug too); only changes that can alter simulation
#           results or core compilation need it
#   viewer  also build accrete-viewer (macroquad)
#
# Fail-safe by design: everything runs when there is no BASE, when BASE or
# HEAD cannot be resolved, when `git diff` fails, when the diff is empty, and
# for any path not recognised below. A new kind of file, or a broken
# comparison, can never silently skip CI. Renames are listed as a deletion
# plus an addition (`--no-renames`), so the source path of a moved file is
# classified too. Keep the categories in sync with CLAUDE.md ("Commands",
# path filtering).
set -euo pipefail

all() {
  printf 'rust=true\ncore=true\nviewer=true\n'
  exit 0
}

base=${1:-}
head=${2:-HEAD}
[ -n "$base" ] || all
git rev-parse -q --verify "$base^{commit}" >/dev/null 2>&1 || all
git rev-parse -q --verify "$head^{commit}" >/dev/null 2>&1 || all

# NUL-separated, unquoted paths; renames split into source and destination.
listing=$(mktemp)
trap 'rm -f "$listing"' EXIT
git diff --no-renames --name-only -z --end-of-options "$base" "$head" >"$listing" 2>/dev/null || all
[ -s "$listing" ] || all

rust=false core=false viewer=false
while IFS= read -r -d '' f; do
  [ -n "$f" ] || continue
  case "$f" in
    # Simulation core and everything it reads in tests: full run. Anything
    # under crates/ is code, including a crate's README (it may be compiled
    # into doc tests), so these come before the docs patterns.
    crates/accrete-core/* | configs/* | tests/*)
      rust=true core=true viewer=true ;;
    # Front ends: they cannot change simulation results.
    crates/accrete-cli/*)
      rust=true ;;
    crates/accrete-viz/* | crates/accrete-viewer/*)
      rust=true viewer=true ;;
    # Docs and editor/agent config: no build needed.
    *.md | docs/* | .claude/* | .github/pull_request_template.md | .gitignore | LICENSE*)
      ;;
    # Toolchain, dependencies, lints, line endings (golden files), CI itself.
    Cargo.toml | Cargo.lock | rust-toolchain.toml | clippy.toml | .gitattributes | .github/*)
      rust=true core=true viewer=true ;;
    # Unknown path (a new crate, a new top-level file): be safe.
    *)
      rust=true core=true viewer=true ;;
  esac
done <"$listing"

printf 'rust=%s\ncore=%s\nviewer=%s\n' "$rust" "$core" "$viewer"
