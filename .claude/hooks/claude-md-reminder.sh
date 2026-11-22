#!/usr/bin/env bash
# Claude Code Stop hook: keep CLAUDE.md current.
#
# When files that CLAUDE.md documents changed on this branch (against
# origin/main, including uncommitted and untracked files) but CLAUDE.md did
# not, ask Claude once to review CLAUDE.md before finishing. It fires once per
# distinct set of changed files, so it never nags repeatedly about the same
# change, and never loops (it skips when the Stop hook is already active).
# Registered in .claude/settings.json; see "Keeping this file current" in
# CLAUDE.md for what each watched path maps to.

input=$(cat)
case "$input" in
  *'"stop_hook_active":true'* | *'"stop_hook_active": true'*) exit 0 ;;
esac

root=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
cd "$root" || exit 0

base=$(git merge-base HEAD origin/main 2>/dev/null || git rev-parse HEAD 2>/dev/null) || exit 0
changed=$(
  {
    git diff --name-only "$base"
    git diff --name-only --cached
    git ls-files --others --exclude-standard
  } 2>/dev/null | sort -u
)
[ -n "$changed" ] || exit 0

# CLAUDE.md already touched on this branch: nothing to remind about.
printf '%s\n' "$changed" | grep -qx 'CLAUDE.md' && exit 0

# Paths whose changes usually mean CLAUDE.md needs a look.
watched=$(printf '%s\n' "$changed" | grep -E '^(Cargo\.toml|rust-toolchain\.toml|clippy\.toml|\.github/(workflows|scripts)/.*|crates/[^/]+/Cargo\.toml|crates/accrete-cli/src/main\.rs|crates/accrete-viewer/src/main\.rs|crates/accrete-core/src/(lib|rng|hash|config|scenario)\.rs|crates/accrete-core/src/simulation/(mod|engine)\.rs|crates/accrete-core/src/agents/decision\.rs|crates/accrete-core/tests/common/.*|docs/DESIGN\.md)$')
[ -n "$watched" ] || exit 0

# Remind once per distinct set of watched changes.
stamp=$(git rev-parse --git-path claude-md-reminder.stamp 2>/dev/null) || exit 0
digest=$(printf '%s\n' "$watched" | git hash-object --stdin 2>/dev/null) || exit 0
[ -f "$stamp" ] && [ "$(cat "$stamp" 2>/dev/null)" = "$digest" ] && exit 0
printf '%s\n' "$digest" > "$stamp" 2>/dev/null

files=$(printf '%s\n' "$watched" | paste -sd ',' - | sed 's/,/, /g')
printf '{"decision":"block","reason":"CLAUDE.md check: these files changed on this branch but CLAUDE.md did not: %s. Review the \\"Keeping this file current\\" table in CLAUDE.md and update the affected sections in this same change. If nothing CLAUDE.md describes has changed, say so in one line and finish."}\n' "$files"
exit 0
