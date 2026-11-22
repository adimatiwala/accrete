# Accrete

An agent-based, emergent civilization simulator: *a world that builds itself,
one decision at a time.* The specification is [`accrete.md`](accrete.md). Its
**Appendix A** is the full changelog of what has been implemented, with the
reasoning behind every design decision; [`docs/DESIGN.md`](docs/DESIGN.md) is a
short map of the code.

Rust, fully offline, and deterministic (designed to be bit-identical across
macOS and Windows, with golden hashes in `tests/golden/`).

```
crates/
  accrete-core/    pure simulation: world gen, agents, engine, social, weather, metrics
  accrete-viz/     overlay colours shared by the viewer and the CLI renderer
  accrete-cli/     headless runner: run, render, trace, diag, world, config
  accrete-viewer/  macroquad viewer: pan/zoom, overlays, inspector, playback
configs/default.toml   every tunable, mirroring Config::default()
tests/scenarios/       hand-built worlds for behavioural tests (Week 2 + Week 3)
tests/golden/          committed state hashes (world + ticks 0/100/1000, seed 1)
```

## Quick start

```sh
cargo build --release

# Interactive viewer: Space run/pause, +/- speed, Tab overlays (try "depletion"),
# click to inspect, H for help
cargo run --release -p accrete-viewer -- --seed 1 --play

# Headless 300-day run: metrics CSV, death log, final map, and a timelapse
# strip (terrain + depletion) in results/
cargo run --release -p accrete-cli -- run --seed 1 --days 300 --every 50 --timelapse 0,75,150,225,300

# Experiments: override any config value
cargo run --release -p accrete-cli -- run --seed 1 --set resources.food_regrowth=0.06 --label scarce
cargo run --release -p accrete-cli -- run --seed 1 --set infrastructure.household_weight=0 --set social.reciprocity_enabled=false --label selfish

# Why did agent 42 do that? Tick-by-tick options and utility terms
cargo run --release -p accrete-cli -- trace --seed 1 --day 120 --ticks 24 --agent 42 --options

# World only: statistics, validation and hash for several seeds
cargo run --release -p accrete-cli -- world --seed 1,2,3
```

Tests: `cargo test --release -p accrete-core`. This covers unit tests, the
world-generation acceptance checks, determinism and golden hashes,
conservation, population rules, droughts, and the 14 behavioural scenarios. To
re-bless the golden hashes deliberately, run `ACCRETE_BLESS=1 cargo test` and
explain the change in the commit message.

## What emerges (default config, 300 days)

| Seed | Population | Hamlets + forming | Clustering R (day 30 → 300) | Foraging radius | Shelters | Starvation deaths | Pathologies |
|---|---|---|---|---|---|---|---|
| 1 | 100 → 507 | 4 + 3 | 0.95 → 0.49 | 1.6 → 3.8 tiles | 41 | 0 | 0 |
| 2 | 100 → 583 | 4 + 5 | 1.08 → 0.53 | 1.4 → 4.5 tiles | 49 | 0 | 0 |
| 3 | 100 → 442 | 3 + 1 | 1.08 → 0.40 | 1.4 → 3.0 tiles | 34 | 0 | 0 |

- **Clustering:** homes cluster along rivers, from a random pattern (R ≈ 1) to strongly clustered (R ≈ 0.45), and families bud off into hamlets that last 100+ days.
- **Depletion:** settlements wear down the land around them, so foraging trips lengthen over time; the viewer's depletion overlay shows this clearly.
- **Shelters:** crowded families pool wood and stone to upgrade their homes.
- **Sharing:** knowledge of food spreads through meetings.

There is no rule anywhere in the code for settlements, roads, or where anyone lives.

**Honest negatives** (valid results, per the spec):
- Roads barely form under the spec's road-benefit rule.
- There is almost no division of labour, because everyone mainly gathers food.
- Barter happens, but only modestly.

Appendix A.9 of `accrete.md` discusses why, and what to try next.

Performance on one laptop core: about 945 ticks/s with 100 agents, and about 260–290
ticks/s with 500–600 agents (300 days in about 25–30 s).
