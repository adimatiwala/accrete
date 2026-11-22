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
  accrete-cli/     headless runner: run, branch, experiments, sweep, index, reproduce, render, trace, diag, world, config
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

# Snapshots and branching: save day 100, then compare a control and a drought to day 300
cargo run --release -p accrete-cli -- run --seed 1 --days 100 --save results/seed1_day100.snap.json
cargo run --release -p accrete-cli -- branch --load results/seed1_day100.snap.json --days 200 --drought 100:120:64:64:24:0.6 --label-b drought

# Experiments: override any config value, or run the Week 3 Day 5 ablation suite
cargo run --release -p accrete-cli -- run --seed 1 --set resources.food_regrowth=0.06 --label scarce
cargo run --release -p accrete-cli -- run --seed 1 --set infrastructure.household_weight=0 --set social.reciprocity_enabled=false --label selfish
cargo run --release -p accrete-cli -- experiments --seed 1 --days 100 -o results/emergence
cargo run --release -p accrete-cli -- experiments --seed 1,2,3 --days 300 --only scarcity40,scarcity20 -o results/scarcity

# Phase 2: ten seeds in parallel, with 95% intervals in emergence_aggregate.csv
cargo run --release -p accrete-cli -- experiments --seed 1-10 --jobs 8 --days 300 -o results/phase2
# Parameter sweep: a grid over food regrowth, 3 seeds per point, in parallel
cargo run --release -p accrete-cli -- sweep --param resources.food_regrowth=0.03,0.06,0.1,0.15 --seed 1-3 --days 150 --jobs 8
# Every run writes a manifest, upserts runs.sqlite, and refreshes runs_index.csv
cargo run --release -p accrete-cli -- reproduce results/phase2/control_seed1_manifest.json
cargo run --release -p accrete-cli -- index query -d results/phase2 --label control
cargo run --release -p accrete-cli -- index rebuild -d results/phase2   # recursive; also refreshes the CSV
# Mid-run state snapshots every 50 days (PNG maps still use --snap-every)
cargo run --release -p accrete-cli -- run --seed 1 --days 100 --save-every 50 -o results

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
| 1 | 100 → 513 | 3 + 3 | 0.95 → 0.52 | 1.6 → 4.2 tiles | 48 | 0 | 0 |
| 2 | 100 → 548 | 3 + 4 | 1.08 → 0.56 | 1.4 → 4.5 tiles | 54 | 0 | 0 |
| 3 | 100 → 458 | 1 + 2 | 1.08 → 0.45 | 1.4 → 3.1 tiles | 37 | 0 | 0 |

- **Clustering:** homes cluster along rivers, from a random pattern (R ≈ 1) to strongly clustered (R ≈ 0.45–0.56), and families bud off into hamlets that last 100+ days.
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
