#!/usr/bin/env bash
# Decide which CI work a change needs, from the files it touches.
#
# Usage: ci-changes.sh [BASE [HEAD]]
#   Compares BASE..HEAD (HEAD defaults to HEAD). With no BASE, or when BASE
#   cannot be resolved, everything runs. Prints key=value lines for
#   $GITHUB_OUTPUT:
#
#   rust    run the Rust job at all (fmt, clippy, release tests, smoke run)
#   core    also run the slow debug test pass of accrete-core (golden hashes
#           must match in debug too); only changes that can alter simulation
#           results or core compilation need it
#   viewer  also build accrete-viewer (macroquad)
#
# Any path not recognised below counts as affecting everything, so a new kind
# of file can never silently skip CI. Keep the categories in sync with
# CLAUDE.md ("Commands", CI paragraph).
set -euo pipefail

all() { printf 'rust=true\ncore=true\nviewer=true\n'; exit 0; }

base=${1:-}
head=${2:-HEAD}
[ -n "$base" ] || all
git rev-parse -q --verify "$base^{commit}" >/dev/null 2>&1 || all
git rev-parse -q --verify "$head^{commit}" >/dev/null 2>&1 || all

rust=false core=false viewer=false
while IFS= read -r f; do
  [ -n "$f" ] || continue
  case "$f" in
    # Docs and editor/agent config: no build needed.
    *.md | docs/* | .claude/* | .github/pull_request_template.md | .gitignore | LICENSE*)
      ;;
    # Simulation core and everything it reads in tests: full run.
    crates/accrete-core/* | configs/* | tests/*)
      rust=true core=true viewer=true ;;
    # Front ends: they cannot change simulation results.
    crates/accrete-cli/*)
      rust=true ;;
    crates/accrete-viz/*)
      rust=true viewer=true ;;
    crates/accrete-viewer/*)
      rust=true viewer=true ;;
    # Toolchain, dependencies, lints, line endings (golden files), CI itself.
    Cargo.toml | Cargo.lock | rust-toolchain.toml | clippy.toml | .gitattributes | .github/*)
      rust=true core=true viewer=true ;;
    # Unknown path (a new crate, a new top-level file): be safe.
    *)
      rust=true core=true viewer=true ;;
  esac
done < <(git diff --name-only "$base" "$head")

printf 'rust=%s\ncore=%s\nviewer=%s\n' "$rust" "$core" "$viewer"
