## What and why

<!-- What this changes, and the reason. Link the relevant accrete.md section. -->

## Checklist

- [ ] `cargo fmt --all`, CI clippy (`cargo clippy -p accrete-core -p accrete-cli -p accrete-viz --all-targets -- -D warnings`) and `cargo test --release -p accrete-core` pass
- [ ] Behaviour unchanged, **or** golden hashes re-blessed (`ACCRETE_BLESS=1`), debug tests pass too, and the reason is in the commit message
- [ ] Config changes: `configs/default.toml` regenerated (`cargo run -q --release -p accrete-cli -- config > configs/default.toml`)
- [ ] New rules are agent motives, never scripted outcomes (`accrete.md` §1)
- [ ] **Changelog:** a new `accrete.md` Appendix A section for this PR (what changed, why, how verified), plus any design decisions or spec deviations; results tables re-run if results moved
- [ ] `CLAUDE.md` updated if commands, architecture, rules or status changed (see its "Keeping this file current" table)
