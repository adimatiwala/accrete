#!/usr/bin/env bash
# Self-test for ci-changes.sh: builds a throwaway git repository and checks
# the classification of each kind of change, including the fail-safe cases.
# Run locally with `bash .github/scripts/test-ci-changes.sh`; CI runs it
# before classifying a PR.
set -euo pipefail

script=$(cd "$(dirname "$0")" && pwd)/ci-changes.sh
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"
git init -q -b main
git config user.email ci@example.invalid
git config user.name ci
mkdir -p crates/accrete-core/src crates/accrete-cli/src crates/accrete-viz/src \
  crates/accrete-viewer/src tests/scenarios docs configs .github/workflows
for f in crates/accrete-core/src/lib.rs crates/accrete-cli/src/main.rs \
  crates/accrete-viz/src/lib.rs crates/accrete-viewer/src/main.rs \
  tests/scenarios/a.toml docs/DESIGN.md configs/default.toml README.md \
  crates/accrete-core/README.md .github/workflows/ci.yml Cargo.toml; do
  echo x >"$f"
done
git add -A && git commit -qm base

fails=0
check() { # name expected-flags
  local got
  got=$(bash "$script" "$base" HEAD | paste -sd ' ' -)
  if [ "$got" = "$2" ]; then
    echo "ok    $1"
  else
    echo "FAIL  $1: expected '$2', got '$got'"
    fails=$((fails + 1))
  fi
}
change() { # message, then shell commands to run
  git checkout -q main
  git checkout -q -B case
  eval "$2"
  git add -A
  git commit -qm "$1" --allow-empty
  base=main
}
ALL='rust=true core=true viewer=true'
NONE='rust=false core=false viewer=false'

change docs "echo y >> docs/DESIGN.md; echo y >> README.md";            check "docs only" "$NONE"
change cli "echo y >> crates/accrete-cli/src/main.rs";                  check "cli only" "rust=true core=false viewer=false"
change viewer "echo y >> crates/accrete-viewer/src/main.rs";            check "viewer only" "rust=true core=false viewer=true"
change viz "echo y >> crates/accrete-viz/src/lib.rs";                   check "viz only" "rust=true core=false viewer=true"
change core "echo y >> crates/accrete-core/src/lib.rs";                 check "core" "$ALL"
change cfg "echo y >> configs/default.toml";                            check "config" "$ALL"
change ci "echo y >> .github/workflows/ci.yml";                         check "CI file" "$ALL"
change readme "echo y >> crates/accrete-core/README.md";                check "crate README is code" "$ALL"
change unknown "echo y > new-top-level-file";                           check "unknown path" "$ALL"
change rename "git mv tests/scenarios/a.toml docs/a.md";                check "rename keeps the source path" "$ALL"
change mvcore "git mv crates/accrete-core/src/lib.rs crates/accrete-cli/src/lib.rs"; check "core->cli move" "$ALL"
change delete "git rm -q configs/default.toml";                         check "deletion" "$ALL"
change nonascii "echo y > 'docs/café notes.md'";                        check "non-ASCII doc with a space" "$NONE"
change empty ":";                                                       check "empty diff runs everything" "$ALL"
base="";               check "no base" "$ALL"
base=deadbeefdeadbeef; check "unknown base" "$ALL"

[ "$fails" -eq 0 ] || { echo "$fails case(s) failed"; exit 1; }
echo "all ci-changes.sh cases passed"
