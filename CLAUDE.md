# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Accrete is a deterministic, agent-based civilization simulator in Rust. About 100 agents act only on what they have seen and remember. Households, hamlets, trails and trade are meant to *emerge*; nothing in the code scripts them. It runs fully offline and must give **bit-identical results on macOS (Apple Silicon) and Windows (x86-64)**.

Where to look:
- **`accrete.md`**: the specification (Sections 1–13). Its **Appendix A** is the implementation changelog: every place the spec was open or didn't work, what was chosen, and why. Read the relevant part before changing behaviour.
- **`docs/DESIGN.md`**: a module-by-module map of `accrete-core`, plus the tick pipeline.
- **`README.md`**: a quick start and the headline results table.

**Status:** Weeks 1–3 and most of Week 4 are built, including snapshots and branching. Route disruption, the random and omniscient baselines, and the behavioural-benchmark range files are **not built yet** (Appendix A, "Status against the plan" and A.10).

## Commands

The toolchain is pinned to Rust **1.86.0** in `rust-toolchain.toml`, with edition 2024. `rustup` installs it on first use.

```bash
cargo build --release                                   # whole workspace
cargo test --release -p accrete-core                    # main suite (~30 s); prefer --release
cargo test -p accrete-core                              # debug build; simulation tests take ~4 min
cargo test --release -p accrete-core --test scenarios   # one integration-test file (scenarios | simulation | worldgen)
cargo test --release -p accrete-core --test scenarios overcrowded_home   # one test, by name substring
cargo test --release -p accrete-core --lib failed_join  # unit tests inside src/ (e.g. simulation/engine.rs)
cargo fmt --all
cargo clippy -p accrete-core -p accrete-cli -p accrete-viz --all-targets -- -D warnings   # exactly what CI runs
```

CI (`.github/workflows/ci.yml`) runs on pull requests (including when a PR's base changes), on pushes to `main`, on merge-queue checks, and when started manually from the Actions tab (any branch). Feature branches are tested through their PR, and a newer run for a PR cancels its older one. Runs for `main` and manual runs are never cancelled. CI runs the same steps on `macos-latest` and `windows-latest`:
1. fmt check
2. the clippy command above
3. release tests for `accrete-core`, `accrete-cli` and `accrete-viz`
4. **debug** tests for `accrete-core`, which must match the same golden hashes
5. a viewer build
6. a headless smoke run: `cargo run --release -p accrete-cli -- run --seed 1 --days 30 --every 10 --check -o results`

**Path filtering.** A `changes` job runs `.github/scripts/ci-changes.sh BASE HEAD` to decide what is needed:
- **Docs only** (`*.md`, `docs/`, `.claude/`, the PR template, `.gitignore`): the Rust job is skipped.
- **`accrete-cli` only:** step 4 (debug tests) and step 5 (viewer build) are skipped.
- **`accrete-viz` or `accrete-viewer` only:** step 4 is skipped.
- **Anything else,** including core, `configs/`, `tests/`, the toolchain, dependencies, `.gitattributes`, CI files, and **any path the script doesn't recognise**: everything runs.

Pushes to `main` and manual runs always run everything. The `ci-result` job always reports, and treats a skipped Rust job as a pass, so it is the one check to require in branch protection. When you add a new crate or a new kind of file, give it a category in the script; otherwise it falls back to running everything.
- **The filter is fail-safe:** everything runs if the base can't be resolved, if `git diff` fails, if the diff is empty, or if the step's outputs are missing. Renames count as both their source and their destination.
- **Testing it:** try the script locally with `bash .github/scripts/ci-changes.sh origin/main HEAD`. Its self-test, `bash .github/scripts/test-ci-changes.sh`, runs in CI before every classification; add a case there whenever you change a category.

Running the simulation:

```bash
cargo run --release -p accrete-viewer -- --seed 1 --play          # interactive viewer (H = help, Tab = overlays)
cargo run --release -p accrete-cli -- run --seed 1,2,3 --days 300 --every 50 -o results
cargo run --release -p accrete-cli -- run --seed 1 --set infrastructure.household_weight=0 --label selfish
cargo run --release -p accrete-cli -- trace --seed 1 --day 120 --ticks 24 --agent 42 --options
cargo run --release -p accrete-cli -- world --seed 1,2,3          # world stats, validation, hash
cargo run --release -p accrete-cli -- run --seed 1 --days 100 --save results/seed1_day100.snap.json
cargo run --release -p accrete-cli -- run --load results/seed1_day100.snap.json --days 200
cargo run --release -p accrete-cli -- branch --load results/seed1_day100.snap.json --days 200 \
    --drought 100:120:64:64:24:0.6 --label-b drought      # control vs intervention, compared per day
```

- **`accrete` subcommands:** `world`, `run`, `branch`, `trace`, `diag`, `config` and `render`.
- **Snapshots:** `run --save` writes the final state as `*.snap.json` (gitignored); `run --load` resumes it, and `--set` or `--config` with `--load` turn the resumed run into an intervention. `branch` loads one snapshot twice, runs a control and an intervention (`--drought DAY:DAYS:X:Y:RADIUS:SEVERITY`, `--set`, `--config-b`), and writes a per-day `*_compare.csv`.
- **`--set section.key=value`** overrides any config value (repeatable).
- **`--check`** runs the conservation and bounds checks every tick. It's slow but catches ledger bugs.
- **Viewer flags:** `--seed`, `--day`, `--play`, `--select <agent>`, `--config <file>`, and `--screenshot <png>` for headless verification.
- **Seeds** can be decimal, `0x…` hex, or free text, which is hashed.
- **`results/`** is gitignored.

## Architecture

Four crates:
- **`accrete-core`:** the whole simulation. It has no graphics or OS-specific code, and its only dependencies are `rand_chacha`, `serde`, `serde_json` and `toml`.
- **`accrete-viz`:** overlay colours, shared so that both front ends draw identical pictures.
- **`accrete-cli`:** the headless `accrete` binary, built on `clap` and `png`.
- **`accrete-viewer`:** the macroquad viewer.

Both front ends depend on `core` and `viz`, and nothing depends on them.

**One tick = 1 hour, 24 ticks = 1 day.** `Simulation::step` in `simulation/engine.rs` is the pipeline:
1. observe
2. plan homes (staggered across agents)
3. propose: score options and plan routes
4. resolve contested actions
5. apply
6. deaths
7. meetings: sharing and barter, among survivors only
8. newborns join
9. the daily phase, on the last tick of each day

Cross-cutting ideas that take several files to see:

- **Belief versus truth.** Agents decide only from `AgentMemory` (`agents/memory.rs`, all fixed-size) and from `decision::Ctx`. `Ctx` exposes the real world, but decisions may read only:
  - the agent's own tile;
  - static terrain of tiles it knows;
  - its **own household** (residents, chests, crowding).

  Everything else comes from memory "as last seen". Routes are planned with `AgentBelief` over known tiles only. New perception goes through `engine::observe`. Never read live world state for distant or other households' facts.
- **Propose, then resolve.** During `propose`, the world is read-only and no agent sees another's choice, so processing order doesn't matter. Tie-breaking jitter is a stateless hash of `(agent, tick, option)` and draws no random numbers. Contested actions are settled in `resolve_*`:
  - gathering is split in proportion to each request;
  - household actions are handled in a `conflicts`-stream permutation;
  - births are decided in parent-ID order.

  Nothing may favour low IDs.
- **Decisions are explainable.** `decision::evaluate` returns candidates with named utility `term`s, which `accrete trace --options` and the viewer inspector show. Keep new options and terms named and recorded the same way.
- **Measurement only.** `simulation/{settlement,emergence,metrics,events}.rs` observe the run. Agents never read them, and that's a design rule, not an accident (see below).
- **Units.** Config values are **per day**. `TickRates::new` in `config.rs` is the *single* conversion to per-tick rates. Movement uses integer movement points: 10 per tick, with a terrain cost to enter each tile.
- **Scenarios.** `scenario.rs` loads the small hand-built TOML worlds in `tests/scenarios/`; the map legend and fields are in its module docs. Scenario resources have zero fertility, so nothing regrows unless the scenario says so. Behaviour tests in `tests/scenarios.rs` check outcomes within a window, not exact traces. Tests that need private access, such as calling `do_establish` directly, live in a `#[cfg(test)]` module next to the code, as in `simulation/engine.rs`.

## Rules that must not be broken

1. **Determinism, identical across platforms.** In `accrete-core`:
   - **No banned math.** No `std` transcendentals (`sin`, `exp`, `powf`, `powi`, …), no `mul_add`, no `DefaultHasher`. `clippy.toml` bans them, and CI fails on them. Only `+ - * /`, `sqrt`, comparisons, rounding and `as` casts are allowed.
   - **No unordered iteration** that affects state: use `Vec` in a fixed order or `BTreeMap`, never `HashMap` or `HashSet` traversal.
   - **No parallelism inside one simulation.** Parallel runs of separate seeds are fine.
   - **Floats:** sort with `total_cmp`, and add up in a fixed order.
   - **Randomness** comes only from the named `rng::Stream`s. New randomness uses an existing stream, or **appends** a new id. Never renumber streams: the id is part of the reproducibility contract. Scripted interventions must never draw from `weather`.
   - **No CPU-specific flags or fast-math** anywhere in the workspace.
2. **Everything that matters goes into the state hash.** A new field on `Agent`, `AgentMemory`, `World`, `Tile`, `Home` or `WeatherState` must also be written in that type's `stable_hash`, with a fixed order and floats written via their bits. `Config` is hashed through `Config::digest()`, which hashes its serialized JSON, so **changing any config default changes the golden hashes** even if behaviour doesn't change.
3. **Golden hashes** live in `tests/golden/`: the world hash plus the state at ticks 0, 100 and 1,000 for seed 1. They are shared by both OSes and by debug and release builds.
   - When a change is *meant* to alter results, re-record them with `ACCRETE_BLESS=1 cargo test --release -p accrete-core golden`.
   - Then confirm that `cargo test --release -p accrete-core` **and** `cargo test -p accrete-core` both pass, and explain the change in the commit message.
   - Never bless to make an unexplained failure go away.
   - If `world_seed1` changed, world generation changed. Check that this was intended.
4. **`configs/default.toml` must equal `Config::default()`.** The test `default_toml_matches_defaults` enforces this. Every config section uses `deny_unknown_fields`. After adding or changing a config field, regenerate the file with `cargo run -q --release -p accrete-cli -- config > configs/default.toml`.
5. **Define motives, never outcomes** (`accrete.md` §1). Every rule must be expressible as one agent's preference, perception or ability, using that agent's local knowledge. Never add logic that reads settlements, the road network, population-wide statistics or the metrics, or that triggers on a collective result such as "form a village at N agents". A behaviour that fails to emerge is a valid result: report it, don't force it.
6. **Conserve resources.** Every change to food, wood or stone is recorded in the `Ledger` as gathered, regrown, eaten, transferred, spoiled or used in construction. Inventories, chests and piles respect their capacity limits, and a dying agent's goods go to its home or tile. `check_invariants` / `--check` verify all of this.
7. **Every PR updates the changelog in `accrete.md` Appendix A.** This applies to all PRs, including tooling, CI and docs, not only simulation changes. Add a new `A.N` section, numbered in merge order, saying what changed, the decisions and why, and how it was verified. Also update the "Status against the plan" line when the status changes. Spec deviations and design decisions go there too.
   - When defaults or mechanics change results, re-run `accrete run --seed 1,2,3 --days 300` and update the results tables in README and Appendix A.9.

## Gotchas

- **Debug vs release:** prefer `--release` for tests. The debug simulation tests are slow (~4 min), but CI runs them because both builds must produce the same hashes.
- **Viewer lint:** clippy in CI skips `accrete-viewer`, since macroquad rendering may use any float math. That's safe because rendering never feeds back into the simulation. The CLI's image renderer has the only explicit clippy `allow` in the code.
- **Pinned dependency:** `fontdue` is pinned to 0.9.3 in `Cargo.lock` because 0.9.5 needs rustc 1.87. Edition 2024 with `rust-version = 1.86` keeps the resolver within the pinned toolchain.
- **Snapshots must stay complete.** `simulation/snapshot.rs` lists every saved field. A new piece of state that affects later ticks or the metrics must be added to `Snapshot` (and to `stable_hash` if it affects behaviour), and `SNAPSHOT_FORMAT` bumped when the layout changes.
  - **On load,** the state hash and a whole-body hash are both checked.
  - **Catching omissions:** the test `snapshot_at_500_resumes_to_the_golden_state_at_1000` catches anything left out.
  - **Layout changes:** the golden `snapshot_seed1_t100` (the file's bytes) fails on any layout change, so re-bless it together with the format bump.
- **`Simulation::set_config` returns `Result`.** It rejects `[world]` changes and a population cap below the current population.
- **Debug-only settings stay out of hashes.** `sim.check_invariants` and `sim.event_log_capacity` are excluded from `Config::digest`, and `Config::differs_in_results` ignores them. Keep any new debugging-only setting out of both.
- **Derived world data is not serialized.** The line-of-sight masks and the tile-to-homes index are rebuilt by `World::rebuild_derived`. Any new derived structure must be rebuilt there too, or it will be missing in scenario worlds and after loading a snapshot.
- **Memory pools:** the 64 resource-memory slots are split by type (food 40, wood 16, stone 8). One shared pool let abundant wood push out every food memory, and agents starved next to food.

## Keeping this file current

**This file is part of the code. Update it in the same change** whenever you alter something it describes:

| If you change… | Update |
|---|---|
| Build, test, lint or CI steps; the toolchain; workspace members or crate dependencies | **Commands**, **Architecture** (crates) |
| CI path categories in `.github/scripts/ci-changes.sh` (for example, a new crate) | **Commands** (path filtering) |
| `accrete` subcommands or flags, or viewer flags and keys | **Commands** |
| The tick pipeline order in `engine.rs`, or the `Ctx` rules for what decisions may read | **Architecture**, and `docs/DESIGN.md` too |
| `rng::Stream` ids, hashing, golden-file workflow, or banned methods in `clippy.toml` | **Rules** 1–3 |
| Config layout, units, `TickRates`, or how `default.toml` is generated | **Architecture** (units), **Rule** 4 |
| The scenario format or test layout | **Architecture** (scenarios), **Commands** |
| What is built or missing (`accrete.md` Appendix A status) | **What this is** (status), and the Appendix A changelog entry for the PR |
| A new non-obvious pitfall you hit | **Gotchas** |

Keep entries short and factual, and remove anything that is no longer true rather than adding caveats. A project Stop hook (`.claude/hooks/claude-md-reminder.sh`, registered in `.claude/settings.json`) reminds you once per set of changes when files covered by the table changed on the branch but this file didn't. If the reminder fires and nothing here is affected, say so and carry on.
