# ACCRETE

**An emergent civilization simulator**  
*A world that builds itself, one decision at a time.*

**Status:** Implementation blueprint · **Target:** four-week laptop-scale MVP · **Platforms:** macOS (Apple Silicon) and Windows, fully offline · **Stack:** Rust + macroquad

## 1. Vision

Accrete is a deterministic, agent-based simulation in which individuals pursue local goals under resource, geography, and information constraints. Settlements, exchange networks, and transportation patterns are **observed outcomes**, not structures that the engine creates on command.

**Research question:** Can a functioning civilization emerge from hundreds of individually motivated agents without explicitly programming the formation of cities, roads, or economies?

The aim is an experimental simulation platform rather than a game or an economic forecasting model. Its outputs are not predictions of real human societies.

### Final MVP demonstration

1. Generate a new 128×128 world from a seed, including terrain, rivers, and resources.
2. Spawn 100 agents who independently explore, gather, eat, rest, and remember useful places.
3. Observe durable residential clusters, resource exchanges, and repeated travel paths as agent behaviors develop.
4. Inspect an agent to see its needs, known locations, action scores, current goal, and recent decisions.
5. Save the simulation on day 100, branch into a control and a drought scenario, and compare both on day 300.
6. Export metrics and rerun the same scenario reproducibly without launching the renderer.

**Scientific honesty:** The emergence of settlements, trade, or roads is a hypothesis to test, not a guaranteed feature. If a behavior does not emerge, investigate and report that outcome rather than injecting scripted formation events.

### Design principle: define motives, never outcomes

Every rule in Accrete describes what an **individual agent wants, knows, or can do**. No rule describes a **collective result**.

| ✅ Motive (allowed) | ❌ Outcome (not allowed) |
|---|---|
| "I value my household's time a little, weighted by my sociability." | "Build a road between the village and the forest." |
| "I prefer trading with agents who treated me well before." | "Form a market when 10 agents live nearby." |
| "Living in a crowded home makes resting less effective." | "Split a settlement when it reaches 30 agents." |
| "Paving this tile saves me and my household time." | "Connect every settlement to its nearest neighbour." |

**Test for any new rule:** could it be written as one agent's preference, perception, or ability, using only that agent's local knowledge? If it needs to know about a settlement, a road network, or the whole population, it is scripting an outcome and must not be added.

Agents are **self-interested in a broad sense**: they act for their own benefit, plus limited concern for their household and a memory of how others treated them. Research on agent-based models, from Schelling's segregation model to Sugarscape and Axelrod's cooperation experiments, shows that this is enough to produce trade, clustering, specialization, and cooperation without central coordination.

Pure selfishness is expected to fail at **public goods** such as roads, which cost one agent but help everyone. Accrete therefore includes three agent-level motives (household benefit, reciprocity, and tile-by-tile building) and treats them as experimental conditions. Turning them off, so agents become strictly selfish, is a valid experiment, and "no roads form under pure self-interest" is a valid result.

## 2. Scope and constraints

| Dimension | MVP | Stretch goal |
|---|---|---|
| World | 128×128 tiles | 256×256 with chunking |
| Initial agents | 100, with births and deaths (safety cap 1,000) | 500–1,000 initial |
| Resources | Food, wood, stone | Specialized goods and production chains |
| Agent intelligence | Interpretable utility scoring, local perception, memory | Learning, social knowledge exchange |
| World model | 2D terrain, biomes, rivers | Seasons, more sophisticated hydrology |
| Social systems | Barter, homes, emergent clusters | Institutions and specialization |
| Transportation | Traffic-worn trails, improved roads | Capacity, congestion, collective investment |
| Experiments | Snapshots, drought, route disruption | Parameter sweeps, uncertainty estimates |
| Execution | macOS (Apple Silicon) and Windows laptops, CPU, offline | Parallel batch execution; Linux |

**Explicitly excluded from the four-week MVP:** LLM agents, neural network training, 3D graphics, politics, warfare, detailed currencies, tax systems, multiplayer, and cloud services.

## 3. Architecture

Keep the simulation entirely independent of rendering:

```text
                 Configuration + seed
                          |
                          v
                 Simulation controller
                 fixed tick / event log
                          |
       +------------------+-------------------+
       |                  |                   |
       v                  v                   v
  World engine        Agent engine       Interaction engine
  terrain/resources   observe/decide     gather/trade/build
       |                  |                   |
       +------------------+-------------------+
                          |
                          v
                  Updated world state
                          |
                +---------+---------+
                |                   |
                v                   v
          macroquad viewer      headless CLI
          map/inspectors        batches/metrics
```

### Proposed repository structure

```text
accrete/
├── Cargo.toml                 # workspace manifest
├── rust-toolchain.toml        # pinned compiler for macOS + Windows
├── clippy.toml                # bans non-deterministic float methods
├── .gitattributes             # normalize text line endings
├── accrete.md                 # this specification
├── README.md                  # optional short project introduction
├── configs/
│   └── default.toml
├── crates/
│   ├── accrete-core/
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── world/{terrain,rivers,resources}.rs
│   │       ├── agents/{agent,memory,decision,actions}.rs
│   │       ├── economy/trade.rs
│   │       ├── infrastructure/roads.rs
│   │       └── simulation/{engine,events,snapshot,metrics}.rs
│   ├── accrete-viewer/
│   │   └── src/main.rs
│   └── accrete-cli/
│       └── src/main.rs
├── tests/
│   ├── golden/               # committed state hashes, shared by both OSes
│   ├── scenarios/            # tiny hand-built worlds for behaviour tests
│   └── benchmarks/           # accepted ranges for behavioural benchmarks
├── experiments/
└── results/                  # gitignored generated outputs
```

**Libraries:** `macroquad` (2D viewer), `libm` (deterministic math), `rand_chacha` (seeded RNG), `serde` with `postcard` or `bincode` (snapshots), `serde_json` with `float_roundtrip` (metrics and readable exports), `clap` (CLI), and optionally `toml` (configuration). Terrain noise is implemented in `accrete-core` rather than taken from the `noise` crate; see *Cross-platform determinism*. Commit `Cargo.lock` to pin dependency versions.

### Platform support

Accrete must build and run on both **macOS on Apple Silicon** (primary development machine: MacBook Pro M1 Pro) and **Windows 10/11 x86-64**, with no network access at runtime.

- **Toolchain:** stable Rust pinned with a `rust-toolchain.toml`, so both machines use the same compiler version. Targets: `aarch64-apple-darwin` and `x86_64-pc-windows-msvc`.
- **Offline:** the viewer and CLI never make network calls, and nothing depends on a cloud service, account, or telemetry. Dependencies are downloaded once with `cargo fetch`; after that, build with `cargo build --offline`. Optionally run `cargo vendor` to commit dependencies so a fresh machine can build with no internet.
- **No platform-specific code in `accrete-core`.** It must be pure Rust with no OS APIs, no system libraries, and no conditional compilation per platform. Platform differences belong only in the viewer and CLI, and macroquad already handles windowing on both OSes.
- **File paths:** use `std::path::PathBuf` and `Path::join`, never hard-coded `/` or `\` separators. Configs, snapshots, and results are stored relative to the working directory or to a path passed on the command line.
- **File formats:** snapshots and metrics use UTF-8 JSON, TOML, or CSV, written with `\n` line endings, so files created on one OS load on the other. A `.gitattributes` file normalizes line endings for text files in the repo.
- **Verification:** each weekly milestone is built and tested on both macOS and Windows before it is considered done. Because results must be bit-identical across OSes, a CI workflow running `cargo test --workspace` on GitHub Actions `macos-latest` (Apple Silicon) and `windows-latest` is strongly recommended. It catches platform drift on every push, even when no Windows machine is at hand. CI is only a development aid: Accrete itself must still build and run offline.

### Cross-platform determinism (decided: bit-identical)

The same seed, config, and Accrete version must produce **bit-identical simulation state** on macOS (Apple Silicon) and Windows (x86-64). That includes the state hash at every tick, snapshot contents, and exported metrics. A snapshot saved on either OS must resume on the other and continue identically.

**Approach:** keep `f32`/`f64`, but use only float operations that are guaranteed identical on both platforms. In Rust, `+ - * /`, `sqrt`, comparisons, `floor`/`ceil`/`round`/`trunc`, `abs`, `min`/`max`, and `as` conversions are exactly specified by IEEE 754. They give the same bits on ARM64 and x86-64, because Rust never uses x87 floating point on x86-64 and never fuses multiply-adds implicitly. Fixed-point arithmetic was considered and rejected, because it makes the code much harder to write and to read.

Rules for `accrete-core` (the viewer may use any float operations, since rendering never feeds back into the simulation):

- **No `std` transcendental functions.** `sin`, `cos`, `tan`, `exp`, `ln`, `log*`, `powf`, `powi`, `hypot`, `atan2`, `tanh`, `cbrt`, and similar call the platform's math library, which differs between macOS and Windows. Use the pure-Rust [`libm`](https://crates.io/crates/libm) crate instead, which gives identical results everywhere, or avoid these functions altogether.
- **No `mul_add`**, no fast-math, and no `-C target-cpu=native` or other CPU-specific flags in the workspace.
- **Own noise implementation.** Don't use the `noise` crate in the simulation, because its internals are not audited for these rules. Write a small seeded value or gradient noise in `accrete-core` using only the permitted operations.
- **Fixed order for all float reductions.** Sum, average, and accumulate in a defined iteration order, never through parallel reductions or unordered collections. Parallelism is allowed only between independent runs (for example, one seed per thread), never inside one simulation.
- **Random numbers:** use `rand_chacha::ChaCha8Rng` streams. Convert raw `u32`/`u64` output to floats and ranges with helpers written in `accrete-core`, rather than `rand`'s distributions, whose exact outputs may change between `rand` releases.
- **Explicit integer widths.** Use `u32`/`u64`/`i32` in simulation state rather than `usize` wherever a value is serialized or hashed.
- **Sorting floats:** use `f32::total_cmp`. Treat NaN as a bug: assert in debug builds that state values are finite.
- **Stable hashing:** compute the state hash with a fixed, documented algorithm (such as FNV-1a 64 written in `accrete-core`, or the `blake3` crate) over a canonical byte encoding: little-endian integers and floats as `to_bits()`, written in a fixed field order. Never use `std::collections::hash_map::DefaultHasher`, whose output is not guaranteed to stay the same between Rust versions.
- **Exact snapshot round-trips.** Floats must reload to the same bits. Use a binary format such as `postcard` or `bincode` for snapshots, or `serde_json` with its `float_roundtrip` feature enabled. Exported metrics in CSV and JSON are for humans and plotting, and are not used to resume a run.
- **Enforce with lints:** a `clippy.toml` `disallowed-methods` list bans the `std` transcendentals and `mul_add` in `accrete-core`, so a violation fails `cargo clippy`.

**Guarantee scope:** bit-identical results hold for the same Accrete commit and `Cargo.lock`. Updating dependencies or the pinned toolchain may change results; when that happens, regenerate the golden hashes deliberately and note it in the commit message.

### Engine rules

- Use fixed-duration logical ticks; never let rendering frame rate change simulation outcomes.
- Each tick: observe → update memories → propose actions → resolve conflicts → apply actions → regenerate resources → collect metrics.
- Agent decisions must use local observations and personal memory, not omniscient world state.
- Resolve contested resources and simultaneous moves deterministically, without systematically favoring low agent IDs.
- Keep simulation state serializable, including tick count, RNG states, configuration, and agent memory.
- Maintain explicit units (tiles, ticks, resource units) and document every conversion.
- Use reproducible ordering for iteration; avoid relying on unordered hash-map traversal in decision logic.
- Provide structured debug traces for why an agent selected an action.

## 4. Core simulation data model

These are conceptual Rust types, not a final API contract.

```rust
struct Tile {
    elevation: f32,
    moisture: f32,
    fertility: f32,
    biome: Biome,
    is_river: bool,
    river_distance: u8,      // tiles to nearest river tile, capped
    food: f32,
    food_capacity: f32,
    wood: f32,
    wood_capacity: f32,
    stone: f32,              // finite deposit, never regenerates
    traffic: f32,
    path_level: PathLevel,   // None, Trail, or Road
}

struct Home {
    position: Position,
    residents: Vec<AgentId>,         // kept sorted for deterministic iteration
    chests: Vec<Inventory>,          // up to 3, each 200 weight units
    shelter_level: u8,               // 0–2
    over_capacity_since: Option<u64>,
}

struct Agent {
    id: AgentId,
    position: Position,
    hunger: f32,
    energy: f32,
    health: f32,
    age_ticks: u64,          // ticks since birth (or since world start for initial agents)
    frailty: f32,            // permanent wear from prolonged hunger, exhaustion, or low health
    last_birth_tick: Option<u64>,
    parent: Option<AgentId>,
    inventory: Inventory,    // limited by carry weight
    movement_points: u32,
    memory: AgentMemory,     // fixed-size: known-tile bitset + bounded lists
    traits: Personality,     // curiosity, caution, sociability, diligence
    skills: Skills,          // aptitude + practice for food, wood, stone
    goal: Goal,
    home: Option<HomeId>,
}

struct Simulation {
    tick: u64,
    world: World,
    agents: Vec<Agent>,
    // Separate deterministic RNG streams and configuration.
}
```

Actions by week:
- **Week 2 (survival and households):** `Move`, `Explore`, `GatherFood`, `GatherWood`, `GatherStone`, `Eat`, `Rest`, `EstablishHome`, `BuildStorage`, `Deposit`, `Withdraw`, `Reproduce`.
- **Week 3 (social and infrastructure):** `Trade`, `ImproveTrail`, `ImproveShelter`.

All defaults below are starting values for tuning, set in the config file, not fixed constants.

### Population dynamics: aging, death, and birth

Population changes only through individual agent state. Nothing creates or removes agents to reach a target.

**Compressed lifespans.** Experiments run about 300 days, so lifespans are compressed enough that one or two generations turn over within a run. Defaults, all set in the config in days and converted to ticks in one place: maturity at **40 days**, elder age at **150 days**, so that most agents die of age around **200–250 days** depending on frailty.

**Health ceiling.** An agent's health can never recover above its ceiling:

```text
health_ceiling = clamp(1.0 − age_decline(age) − frailty, 0.0, 1.0)
age_decline(age) = 0                                         if age < elder_age
                 = (age − elder_age) × age_decline_per_day   otherwise
```

- `age_decline` is piecewise linear, so it needs only the operations allowed by the bit-identical rules.
- `frailty` increases by a configured amount for each tick the agent spends very hungry, exhausted, or at low health. It never decreases. Prolonged deprivation therefore shortens life, and an agent in a stable, well-fed home lives longer.
- Ordinary `health` still rises and falls with hunger, energy, and rest, but stays at or below `health_ceiling`.

**Death has exactly two causes, both deterministic, with no random death roll:**
1. **Deprivation:** `health` reaches 0 from unmet needs.
2. **Old age and wear:** `health_ceiling` reaches 0.

Each death is logged with its cause and the contributions of age and frailty, for example "ceiling reached 0: 70% age, 30% frailty from 41 days hungry", and the inspector shows this for dead agents.

**Reproduction (`Reproduce` action).** An agent may reproduce only if all of the following hold:
- its age is between maturity and elder age;
- it has a home and is at that home;
- its `health` is within a configured margin of its `health_ceiling`;
- it carries food above a configured surplus threshold, or its home's chests do;
- at least the reproduction cooldown (default **30 days**) has passed since its last birth;
- the population is below the safety cap;
- a roll from the dedicated `births` RNG stream succeeds.

When the agent is eligible, `Reproduce` is scored like any other utility option, so a hungry agent will not choose it over eating; the `births` roll happens only if the agent chooses it. A successful birth has these results:
- The newborn appears at the parent's home. It receives a configured amount of food from the parent, logged as a transfer, and the parent pays an energy cost.
- The newborn inherits part of the parent's memories (see *Memory*), keeping their freshness timestamps, so it never knows anything the parent didn't.
- Its personality traits are the parent's traits plus small variations drawn from the `births` stream.
- Until maturity, the agent is a **juvenile**: it gathers at a reduced rate (default 50%) and cannot reproduce.

**Initial agents** start as adults, with seeded starting ages spread between maturity and elder age, so they don't all age out at the same time.

**Deterministic births:** births are decided in the conflict-resolution phase. New agents receive IDs in ascending order of their parent's ID within a tick, and are added to the agent list only at the end of the tick, so they act from the next tick onward.

**Resource conservation:** when an agent dies, all of its inventory goes into its home's chests, or onto the tile where it died if it had no home or no chest space, logged as a transfer. It is removed from its home's residents, and a home with no residents left is marked vacant. Agents are never removed with resources still attached.

**Safety cap:** the population is capped at **1,000** agents (configurable). At the cap, `Reproduce` is unavailable and each blocked birth is logged with a warning. The cap never kills agents. If a run hits the cap, the run's results are flagged, because reaching it usually means the parameters are mistuned.

### Decision policy

Use utility-based decisions, with state-dependent weights:

\[
U(a,s) = \sum_i w_i(s) V_i(a,s) - C(a,s)
\]

Examples: hunger raises the value of eating and gathering food; low energy raises the value of resting; an unfamiliar nearby tile can raise the value of exploration; distance and terrain increase travel cost. Include a small, seeded personality variation to prevent identical agents from making identical decisions.

For debugging, record the chosen action, top competing actions, and the major utility terms. Avoid unbounded action oscillation by introducing action commitment or hysteresis if needed.

### Resource conservation

Every resource change should correspond to a defined event: regeneration, gathering, consumption, transfer, construction, or destruction (including spoilage). Write invariant checks so an agent cannot gather more than a tile contains, carry more than its carry limit, put more into storage than it holds, or trade more than its inventory holds.

### Time and movement

- **1 tick = 1 hour; 1 day = 24 ticks.** Day 100 is tick 2,400 and day 300 is tick 7,200. All rates in the config are written per day (or per hour where noted) and converted to per-tick values in one place.
- **1 tile ≈ 1 km × 1 km,** so the 128×128 map is a region of about 128 km × 128 km. Crossing a plains tile in one hour is a loaded foraging pace rather than a brisk walk (about 4 km/h); a finer tick is a stretch goal if travel realism matters. The sight radius of 6 tiles is therefore about 6 km.
- **Several homes can share a tile,** up to 8 per tile, since a 1 km² tile can hold a hamlet.
- **Movement uses integer movement points (MP),** so travel time follows terrain without float rounding. A moving agent gains 10 MP per tick, and entering a tile costs that tile's MP. Unused MP carries over to the next tick.

| Terrain | MP to enter | Time per tile |
|---|---|---|
| Plains | 10 | 1 tick |
| Forest | 20 | 2 ticks |
| Hills | 30 | 3 ticks |
| Mountains | 60 | 6 ticks |
| River tile (fording) | terrain cost + 20 | +2 ticks |
| Ocean | impassable | — |

Worn trails multiply the cost by 0.8 and improved roads by 0.5 (Week 3), rounded down, with a minimum of 5 MP. Integer MP keeps movement exactly reproducible.

### Resources and fertility

Resources are deliberately uneven across the map, so that different places are good for different things and agents have reasons to travel, settle, and trade.

**Fertility** (0–1) decides where food grows:

```text
fertility = clamp(biome_base + 0.3 × moisture + river_bonus(distance_to_river), 0, 1)
```

- `biome_base`: plains 0.3, forest 0.2, hills 0.1, mountains 0, ocean 0.
- `river_bonus` by distance in tiles from the nearest river tile: 1 → +0.35, 2 → +0.25, 3 → +0.15, 4 → +0.05, beyond 4 → 0. River tiles themselves grow no food.
- Distance to river is computed once at world generation with a breadth-first search, and stored on each tile.

**Food (renewable, wild crops).**
- A tile grows food only if its fertility is at least 0.15.
- A seeded **crop-patch noise field** makes food patchy rather than uniform: a patch factor of 1.5 for rich patches (noise above 0.65), 1.0 for average tiles, and 0.4 for poor ones (noise below 0.35).
- `food_capacity = 20 × fertility × patch_factor` units.
- Tiles start at 80% of capacity.
- Food regrows once per day: `food += 0.15 × fertility × (food_capacity − food)`. Fertile, depleted tiles near rivers recover in about a week, while poor tiles recover slowly. Food never exceeds capacity through growth.

**Wood (renewable, slow).** Forest tiles hold up to 40 units, and plains tiles with moisture above 0.5 hold up to 5 (scattered trees). Wood regrows once per day: `wood += 0.02 × (capacity − wood)`, so a clear-cut forest takes months to recover.

**Stone (finite).** A seeded deposit noise field places stone in clusters on hills and mountains: tiles with deposit noise above 0.6 get 50–200 units, drawn from the world-generation RNG stream. Stone never regenerates.

**Regeneration runs once per day,** at the first tick of each day, rather than every tick. This is cheaper and keeps the result exactly reproducible.

### Needs and eating

| Need | Default behaviour |
|---|---|
| Hunger (0 = full, 1 = starving) | Rises 0.33 per day. Eating 1 food unit lowers it by 0.165, so an adult needs about **2 food per day**; juveniles need about 1. |
| Energy (0 = exhausted, 1 = rested) | Falls 0.05 per active tick. Resting restores 0.1 per tick, or 0.125 at home. |
| Health (0 = dead) | When very hungry (hunger at least 0.7) or exhausted (energy 0), health falls 0.05 per day and frailty builds up. At hunger 1.0, health falls 0.15 per day. Otherwise health recovers toward the health ceiling. |

Agents eat **only from their own inventory.** To eat stored food, they first `Withdraw` it from storage at home.

### Inventory and storage

**Carry limit:** 30 weight units for adults and 15 for juveniles. Weights are food 1, wood 2, and stone 3, so an adult can carry 30 food (about 15 days of eating), 15 wood, 10 stone, or a mix.

**Spoilage:** carried food loses 2% per day, and stored food 0.5% per day. Spoilage is logged as destruction. It stops agents from hoarding forever, and makes storage and nearby food sources worth something.

**Storage chests** let agents keep more than they can carry:
- `BuildStorage` costs 10 wood and 6 ticks of work. It places a chest on the agent's home tile.
- A chest holds 200 weight units, and a home can have up to 3 chests.
- Every resident of a home can `Deposit` into and `Withdraw` from its chests. Each takes 1 tick. Non-residents have no access, so trade is their only way to get stored goods.
- Chests stay when residents die or leave. A vacant home keeps its chests and their contents, and whoever establishes a home there next can use them.
- If a dying agent's home has no chest with room, its inventory is dropped onto the tile as a pile that anyone can gather. Dropped food spoils at the carried rate.

### Observation and line of sight

- Each agent sees a **square** of radius 6 around itself (13×13 tiles).
- **Terrain height blocks sight.** A tile is visible if no tile between the agent and it, along a Bresenham line, rises above the straight sight line from the agent's eye (its tile elevation plus a small eye height) to the target tile.
- **Low cost:** terrain never changes, so each tile's visibility pattern is computed once at world generation and stored as a 169-bit mask, which fits in three `u64`s. That is about 384 KB for a 128×128 map, and takes well under a second to compute. Observing then means reading the mask, which is about 169 tile lookups per agent. The masks are rebuilt from the terrain when a snapshot loads, rather than saved.
- Agents observe again when they move, or every 6 ticks while staying put, so changes such as gathering and regrowth are noticed.

### Memory

Each agent's memory has a fixed size, so memory use and decision time stay bounded as the population grows:

| Memory | Size | Contents |
|---|---|---|
| Known tiles | Bitset, 16,384 bits (2 KB) | Tiles the agent has ever seen. Because terrain doesn't change, seeing a tile once tells the agent its terrain. |
| Resource memories | 64 entries | Position, resource type, amount seen, and the tick it was seen. When full, the entry with the lowest `amount × freshness` is replaced, with ties broken by position. |
| Changed-tile memories | 32 entries | Tiles where the agent saw a change to the normal terrain, such as a route blocked by a shock or a new road, so its route planning uses what it actually saw. |
| Social memories (Week 3) | 32 entries | Agents met and a reciprocity score for each, based on trade outcomes (see *Trade rule*). |
| Frequent tiles (Week 3) | 16 entries | Tiles the agent crossed most in the last 30 days, with crossing counts, used to judge whether paving a tile is worth it. Residents of the same home can see each other's lists. |

That is about 4 KB per agent, or about 4 MB at the 1,000-agent cap, so snapshots stay small and scanning memory during decisions is cheap. Newborns inherit the parent's best 50% of resource memories, and the known tiles within 16 tiles of home.

### Pathfinding and exploration

- Routes use **A\*** over tiles the agent knows and believes passable, with MP costs as the agent believes them to be. Unknown tiles are never part of a planned route, so agents can't use knowledge they don't have.
- **Exploration** happens through *frontier* tiles: known, passable tiles next to at least one unknown tile. `Explore` picks a frontier tile, weighing distance against the agent's curiosity, and routes there, which reveals new tiles. Agents also always learn about the tiles in their sight square, including when they step next to unknown ground.
- **Limits:** each A\* search expands at most 4,000 tiles. A route is reused until the agent arrives, finds the path blocked, or changes goal, and is planned again at most every 12 ticks unless blocked.

### Personality traits

Each agent has four traits in the range 0–1. Initial agents draw them from a seeded stream, and newborns get the parent's values ±0.1.

| Trait | What it changes |
|---|---|
| **Curiosity** | Exploring unknown land versus using known resources; how far away a frontier it will consider. |
| **Caution** | How much food it tries to keep in reserve, how much it prefers known routes, and how early it heads home or eats. |
| **Sociability** | How much it values living with and near others, how much crowding it tolerates, how willing it is to trade, and how much it counts benefits to its household. |
| **Diligence** | How long it works before resting, and how willing it is to spend time building (storage, trails, shelters). |

### Gathering skill

Separately from personality, each agent has a **skill** for each resource (food, wood, stone):
- `skill = aptitude + practice`.
- Aptitude is innate, between 0.7 and 1.3, drawn from a seeded stream. Newborns get the parent's aptitude ±0.1.
- Practice rises by 0.005 per tick spent gathering that resource, up to +0.3, and falls by 0.001 per day when unused.
- Base amount gathered per tick at skill 1.0: food 2 units, wood 1.5, stone 1. Juveniles gather at 50%.

Agents who keep gathering the same thing get better at it, so specialization can emerge on its own. Together with uneven resources, this is what gives trade its value.

### Homes, sharing, and crowding

- A **home** is a dwelling on a tile, with a list of residents; a tile holds up to 8 homes. `EstablishHome` founds a home on a passable land tile that isn't a river tile, or joins a known existing home that is below its comfortable capacity. Agents choose using what they know about nearby food, distance to a river, travel cost, how many people live there (weighted by sociability), and crowding.
- **Newborns** belong to their parent's home. As adults they may stay or move out. Nothing forces them out; crowding does.
- **Comfortable capacity** is 4 residents, plus 2 for each shelter level, up to 2 levels (Week 3).
- **Overcrowding cost:** when a home has more residents than its comfortable capacity for more than **7 consecutive days**:
  - resting there restores 15% less energy for each extra resident, up to 60% less;
  - every resident's frailty builds up slowly;
  - nobody in that home can reproduce.

  Because the penalty lowers the value of staying, crowded agents move out on their own, with sociable agents putting up with crowding longer.
- **Shelter improvement (Week 3):** `ImproveShelter` costs 20 wood, 10 stone, and 12 ticks of work. It adds 2 to the comfortable capacity and 10% to resting recovery.

### Trade rule (Week 3)

- Agents trade only with an agent on the same or a neighbouring tile.
- Each agent values each good by how much it needs it, comparing what it holds against its reserve target (set by caution) and its current needs.
- The agent proposing a trade offers a ratio based on its own values. The other agent accepts only if the trade leaves it better off by its own values, after counting the time the trade takes. A trade happens only when both sides gain, and only for goods each side actually holds.
- Unequal resources (food near rivers, wood in forests, stone in the hills) plus specialized skills create these gains.
- **Reciprocity:** after each trade, both agents update a reciprocity score for the other in their social memory. The score rises when the partner's share of the total gain was fair (close to half) and falls when the partner took most of it. Agents prefer to approach partners with high scores, need a slightly smaller gain to accept their offers, and need a larger gain to accept offers from partners with low scores. Scores slowly return to neutral over time. This lets cooperation build up through repeated self-interested trades, with no rule about groups or markets.

### Trails and roads

Paths develop at three levels, with different durability and effects:

| Level | How it appears | MP cost | Durability |
|---|---|---|---|
| **Raw traffic** | Every crossing adds 1 to the tile's traffic; traffic decays daily (`traffic_next = λ × traffic + crossings`, default λ = 0.95) | Normal | — |
| **Worn trail** | Forms automatically when traffic rises above a threshold (default 20) | × 0.8 | Fades back when traffic falls below the threshold for 30 days |
| **Improved road** | Built by an agent with `ImproveTrail`, one tile at a time, on an existing worn trail | × 0.5 | Loses 1 condition point per day without traffic; disappears at 0 (starts at 180) |

**Building is tile-by-tile.** `ImproveTrail` on one tile costs 2 wood, 2 stone, and 4 ticks of work. An agent considers it only for a worn-trail tile it is standing on, and builds when its perceived benefit over the next 30 days exceeds the cost:

```text
perceived_benefit = time_saved_per_crossing
                  × (own_expected_crossings
                     + household_weight × sociability × household_expected_crossings)
```

- `own_expected_crossings` and `household_expected_crossings` come from the frequent-tile lists of the agent and its home's residents, so the estimate uses only local, shared-household knowledge.
- `household_weight` defaults to 0.5. Setting it to 0 makes agents strictly selfish.
- Labour is converted to value at the agent's own current value of time, which is higher when it is hungry or tired, so starving agents don't build roads.
- Diligence raises an agent's willingness to spend time building.

Roads therefore appear on the heaviest-used tiles first and extend only where use justifies it, without any planner deciding where they go.

### Settlement detection (measurement only)

A **settlement** is a group of occupied homes that meets all of the following, measured once per day:

| Parameter | Meaning | Default | Why |
|---|---|---|---|
| **N** | Minimum occupied homes | **5** | A real hamlet is about 5 or more households. |
| **M** | Minimum total residents | **15** | About 3 people per home; stops a few near-empty homes counting. |
| **R** | Maximum distance from the group's centre | **3 tiles (≈ 3 km)** | Pre-modern villages and their fields span a few kilometres. |
| **P** | Minimum time the group must last | **60 days** | About a quarter of an agent's lifespan, and longer than a typical drought, so a temporary camp doesn't count. |

Homes are grouped by linking any two occupied homes within R tiles of each other. A group that meets N and M becomes a *candidate*, and becomes a settlement once it has stayed a candidate for P consecutive days. A settlement keeps its identity while at least half of its homes continue from one day to the next.

**Size classes** for reporting: **hamlet** (meets the thresholds above), **village** (at least 15 homes and 50 residents), and **town** (at least 50 homes and 150 residents). With a population of 100–1,000, a few hamlets and perhaps a village or two are a realistic outcome; a town would be a striking result.

Settlements are only measured; detecting one never changes agent behaviour. Results are also checked with N ∈ {3, 5, 8}, M ∈ {10, 15, 25}, R ∈ {2, 3, 5}, and P ∈ {30, 60, 90}, to show they don't depend on one choice of threshold.

### Random droughts

Droughts happen on their own, through ordinary world mechanics:
- **Frequency:** checked once per day using a dedicated `weather` RNG stream, with a chance of 1 in 150 per day. That is about one drought every 150 days, roughly once or twice per 300-day run and once or twice per agent lifetime. Lifespans are compressed, so this matches the real-world pattern of a significant regional drought a few times in a lifetime, without happening constantly.
- **Size:** small to medium, a radius of 6–16 tiles. That covers roughly 1–5% of the map, centred on a random land tile.
- **Length:** 10–40 days.
- **Severity:** food regrowth inside the region falls by 30–70%.
- **Limits:** at most one random drought at a time, and none for 60 days after one ends.

Every drought is logged as a world event with its centre, radius, start, end, and severity, and is shown on the map. Random droughts can be turned off in the config. When a run branches from a snapshot, both branches keep the same `weather` stream, so they get identical random droughts, and the scripted drought in the experiment is the only difference.

## 5. Week 1 — Procedural world generation

**Goal:** Create a repeatable, visually interesting environment with meaningful geographic constraints.

### Day 1: Workspace and world representation

- Create a Cargo workspace with `accrete-core`, `accrete-viewer`, and `accrete-cli`.
- Define grid coordinates, tiles, biomes, and simulation configuration.
- Implement seed parsing and deterministic RNG initialization.
- Add a minimal `generate_world(seed, config)` API.
- Verify `cargo build --workspace` and `cargo test --workspace`.

### Day 2: Terrain and biomes

- Generate elevation using seeded multi-octave noise, implemented in `accrete-core` using only deterministic float operations.
- Generate a separate moisture field.
- Classify ocean, plains, forest, hills, and mountains using thresholds.
- Ensure neighboring cells are reasonably continuous; prevent isolated artifacts where practical.
- Add a color-coded biome renderer.

### Day 3: Rivers and resources

- Build a simplified downhill river routing algorithm, with explicit treatment of local minima and map boundaries.
- Compute distance to the nearest river for every tile, and fertility from biome, moisture, and river proximity, as specified in *Resources and fertility*.
- Place food using fertility and the crop-patch noise field, wood in forests and moist plains, and clustered finite stone deposits in hills and mountains.
- Distinguish renewable resources (food, wood) from finite deposits (stone).
- Define passability and movement-point costs for each terrain type, including river fording.

### Day 4: Visualization

- Render a 128×128 tile grid using macroquad.
- Implement pan, zoom, and tile inspection.
- Add overlay modes: biome, elevation, moisture, fertility, river distance, food, wood, stone, and movement cost.
- Add seed input and regenerate control; show seed in the interface.

### Day 5: Starting positions and validation

- Place 100 initial agents in viable locations; do **not** prebuild cities.
- Test that spawn positions are traversable and have plausible nearby resources.
- Precompute the line-of-sight visibility mask for every tile, and test it against a brute-force check on a few tiles.
- Add a stable state hash for generated worlds, and commit a golden hash for the primary validation seed under `tests/golden/`. The same golden value must pass on macOS and Windows.
- Benchmark generation time and viewer responsiveness.

**Week 1 deliverable:** A navigable, seeded world map with meaningful terrain and resource distributions.

**Acceptance criteria:** Same seed/config gives the same state, bit-identical on macOS and Windows; different seeds differ; all tile properties are in valid ranges; rivers obey routing rules; resources obey biome constraints; generation runs headlessly.

## 6. Week 2 — Individual agents, households, and survival

**Goal:** Make 100 agents survive or fail through their own local decisions, and form households.

This is the densest week. If it runs over, Week 3 Day 5 (experiments) is the buffer.

### Day 1: Agent state, needs, and movement

- Implement needs (hunger, energy, health) with the defaults in *Needs and eating*.
- Implement weight-limited inventories, eating only from inventory, and food spoilage.
- Implement integer movement points and terrain costs.
- Add the fixed-timestep engine and pause/resume/step controls; ensure agents update consistently regardless of rendering FPS.

### Day 2: Observation and memory

- Use the precomputed line-of-sight masks for the 13×13 square observation.
- Implement fixed-size memory: the known-tile bitset, 64 resource memories with freshness-based replacement, and 32 changed-tile memories.
- Distinguish actual world state from an agent's belief about it, and prevent the decision system from reading undiscovered resources.

### Day 3: Decisions, traits, and routes

- Implement explore, move, gather, eat, and rest with utility scoring.
- Add the four personality traits and per-resource gathering aptitude.
- Implement A\* over known passable tiles, frontier-based exploration, route caching, and the search limits.
- Prevent rapid goal switching from creating movement loops.

### Day 4: Conflicts, regrowth, homes, and storage

- Resolve simultaneous claims on the same tile without duplicating resources.
- Add daily regrowth of food and wood.
- Allow agents to abandon depleted regions and search elsewhere.
- Implement `EstablishHome` (founding or joining a home) and shared residence.
- Implement storage chests with `BuildStorage`, `Deposit`, and `Withdraw`.

### Day 5: Population dynamics, inspection, and tests

- Implement aging, frailty, the health ceiling, and both causes of death, as specified in *Population dynamics*.
- Implement `Reproduce` with its eligibility conditions, juvenile period, memory inheritance, the `births` RNG stream, and the 1,000-agent safety cap.
- On death, transfer the agent's whole inventory to its home's chests or its tile, and update the home's residents.
- Clicking an agent reveals needs, inventory, traits, skills, home, remembered places, goal, action score breakdown, age, frailty, health ceiling, and reproduction eligibility; for dead agents, the cause of death.
- Record a rolling event log of actions and resource changes.
- Add the scenario-test runner and the Week 2 scenarios, and the four pathology detectors (see *Behavioural evaluation*).
- Test resource conservation, deterministic updates, death, and bounded agent properties.
- Compare a plentiful-food world against a scarce-food world, including births, deaths by cause, and lifespan.

**Week 2 deliverable:** A starting population of 100 locally informed agents who explore, gather, consume resources, migrate in response to scarcity, form shared homes with storage, age, reproduce, and die.

**Acceptance criteria:** Agents make valid independent decisions; agents never route through tiles they don't know; resource constraints are respected, including carry limits, storage capacity, spoilage, births, and deaths; inspection explains choices and every death; same seed/config reproduces the same state at a given tick; survival, births, and lifespans differ under controlled scarcity conditions; the population never exceeds the safety cap; all Week 2 scenario tests pass; pathology counts stay near zero on the primary seed.

## 7. Week 3 — Emergent settlements, trade, and infrastructure

**Goal:** Make social organization possible without scheduling settlement formation.

### Day 1: Crowding, moving out, and specialization

- Implement the overcrowding cost and moving out, as specified in *Homes, sharing, and crowding*.
- Add skill practice (learning by doing and slow decay), so specialization can emerge.
- Track how long agents remain associated with a home.
- Detect settlements and their size classes for analytics using N, M, R, and P, and ensure that detection **does not** itself grant benefits or trigger city-building behavior.

### Day 2: Exchange

- Value goods by each agent's own needs and reserve target.
- Let agents evaluate voluntary barter with agents on the same or neighbouring tiles, using the trade rule.
- Only execute exchanges that leave both parties better off and that each can afford from what it holds.
- Add social memories of agents met, with reciprocity scores that bias partner choice and acceptance.
- Log transactions, traded quantities, and distances.

### Day 3: Movement and trails

- Track the number of crossings per tile.
- Apply traffic decay: `traffic_next = lambda * traffic + crossings`.
- Turn tiles above a traffic threshold into worn trails (MP cost × 0.8), which fade back when traffic decays.
- Include trail costs in A\* routes, using the costs the agent has seen.
- Guard against pathological feedback where all traffic collapses onto one route for artificial reasons.

### Day 4: Improved infrastructure

- Add frequent-tile memories, shared between residents of the same home.
- Implement tile-by-tile `ImproveTrail` using the perceived-benefit rule in *Trails and roads*, including household weighting.
- Make constructed roads (MP cost × 0.5) more durable than naturally worn trails, with daily condition loss when unused.
- Implement `ImproveShelter` to raise a home's comfortable capacity.
- Show traffic heatmaps and improved-road overlays.
- Keep construction local and individually motivated; avoid central planning logic.

### Day 5: Emergence experiments

- Compare runs with and without barter.
- Compare runs with and without trail travel discounts.
- Compare strictly selfish agents (household weight 0, reciprocity off) against the default motives.
- Add the Week 3 scenarios for trade, reciprocity, and road building.
- Measure persistent clusters, average trip length, resource flows, and path network connectivity.
- Inspect failure modes: isolated agents, starvation loops, excessive clustering, and overpowered roads.

**Week 3 deliverable:** Agents can form durable residential clusters, exchange resources, and gradually create visible travel networks.

**Acceptance criteria:** Trade is voluntary and inventory-safe; trails reflect actual crossings; homes persist for meaningful periods; any detected settlement arises from agent state rather than a scripted city event; disabling systems changes outcomes in measurable ways.

## 8. Week 4 — Disruption, replay, and experiments

**Goal:** Turn the simulation into a reproducible laboratory for emergent behavior.

### Day 1: Snapshotting

- Serialize world, agents, inventories, memories, infrastructure, tick count, and RNG states.
- Load a snapshot and resume to the same subsequent result.
- Add a save format version and validation for incompatible files.
- Test save/load equivalence with a state hash.

### Day 2: Environmental shocks

- Implement drought as reduced food regrowth in a region and time window, used both by scripted experiment droughts and by random droughts.
- Implement random droughts on the `weather` RNG stream, with the frequency, size, length, severity, and cooldown in *Random droughts*, and a config switch to disable them.
- Implement route failure as temporarily impassable or expensive path tiles.
- Apply shocks through ordinary world mechanics, not direct agent behavior overrides.
- Record the timing and parameters of interventions.

### Day 3: Branching experiments

- Save a world at day 100.
- Branch into A: control; B: scripted drought from day 100 onward. Both branches keep identical random droughts from the shared `weather` stream.
- Continue both through day 300 from the same initial snapshot.
- Keep random streams and update ordering controlled to make comparisons interpretable.

### Day 4: Metrics and headless runner

- Export CSV/JSON metrics: population, births, deaths by cause, age distribution, mean lifespan, mean frailty, health, hunger, settlement count, settlement persistence, trade volume, average travel cost, inventory distribution, and recovery time (time for population and births to return to their pre-shock levels), plus pathology counts (oscillating, stuck, starved beside food, idle). Flag runs that hit the safety cap.
- Add random and omniscient baseline agents, and compare normal agents against both on the primary seed.
- Record behavioural benchmark ranges from the first validated runs, and run them as tests.
- Add CLI commands for seeded runs, snapshot loading, scenario selection, and result export.
- Validate the whole pipeline on the single primary seed first. The CLI must already accept a list of seeds, so scaling up later needs no code changes.

### Day 5: Final demo and hardening

- Build an interactive viewer with play/pause, tick step, speed control, seed selection, map overlays, and agent inspection.
- Add a scenario selector and control-versus-intervention plots or exports.
- Test replay, snapshot compatibility, performance, long-run stability, and that all scenario tests and behavioural benchmarks pass on both OSes.
- Record a short demo of a world developing, branching, and responding to drought.

**Week 4 deliverable:** A reproducible local experiment that compares the same emergent civilization under two different conditions.

**Acceptance criteria:** Snapshots resume accurately; interventions change world conditions rather than directly scripting agent decisions; metrics export correctly; repeated runs are reproducible; the viewer and headless runner produce consistent simulation results.

## 9. Experimental design

### Core hypothesis

Limited local information, resource scarcity, and repeated interaction can generate persistent communities and transportation patterns without global coordination.

### Suggested experiments

| Experiment | Control | Intervention | Measurements |
|---|---|---|---|
| Scarcity | Normal food regeneration | Lower regeneration | Survival, migration, cluster persistence |
| Trade | Barter enabled | Barter disabled | Trade volume, inventory inequality, survival |
| Trails | Movement wear reduces cost | No trail advantage | Network structure, average travel cost |
| Disaster | No scripted drought | Scripted regional drought at day 100 | Recovery time, population, relocation |
| Climate | Random droughts off | Random droughts on | Population stability, storage use, migration |
| Knowledge | Agents remember resources | Memory disabled/limited | Search effort, starvation, travel distance |
| Motives | Household weighting and reciprocity on | Strictly selfish (both off) | Road tiles built, trade volume, repeat trade partners, settlement count |

### Seed plan

1. **Phase 1, validation (MVP):** run everything on a single primary seed until the systems work: world generation, survival, households, trade, trails, snapshots, and branching. Golden hashes are recorded for this seed. A second seed is used only to check that different seeds produce different worlds.
2. **Phase 2, experiments (after the MVP):** scale up to multiple seeds per experiment, starting with 10, and compare averages and variability. Conclusions should never rest on a single attractive example.

In Phase 2, hold parameters fixed within a comparison, and record all intervention configurations. Report uncertainty and failed runs rather than selecting only visually appealing results.

### What counts as a settlement?

For measurement purposes only: a spatial cluster of at least *N* occupied homes and *M* residents within *R* tiles, persisting for *P* days. Defaults are N = 5, M = 15, R = 3, and P = 60, with hamlet, village, and town size classes; see *Settlement detection*. Expose N/M/R/P in experiment configuration and test sensitivity to those thresholds. Avoid defining settlement success so loosely that random crowding counts as civilization.

### What counts as a road?

Distinguish (1) raw traffic, (2) an emergent worn trail above a traffic threshold, and (3) a deliberately improved path with construction cost. These should have different durability and movement effects.

## 10. Validation and engineering quality

### Unit tests

- World generation is deterministic and tile values are valid.
- River segments obey routing and boundary constraints.
- Agents cannot spend, consume, gather, or trade unavailable resources.
- Health, hunger, and energy remain within defined bounds, and health never exceeds the health ceiling.
- Frailty never decreases; an agent with no frailty keeps a health ceiling of 1.0 until elder age.
- An agent dies exactly when health or the health ceiling reaches 0, and each death records a cause.
- A dying agent's whole inventory is transferred to its home or tile; total resources are unchanged by deaths and births.
- `Reproduce` is never executed when any eligibility condition fails, including for juveniles, agents past elder age, and at the population cap.
- Newborns inherit only memories the parent held, and get IDs in deterministic order.
- Pathfinding respects impassable tiles and route costs, never includes unknown tiles, and stays within the expansion limit.
- Line-of-sight masks match a brute-force visibility check.
- Fertility follows the river-distance bonus; food and wood regrowth never exceed capacity; stone never regenerates.
- Inventories never exceed the carry limit, and chests never exceed 200 weight units; spoilage is logged as destruction.
- Agents eat only from their own inventory.
- Memory never exceeds its fixed sizes, and replacement is deterministic.
- The overcrowding cost applies only after more than 7 consecutive days over capacity.
- Settlement detection produces stable results for known synthetic layouts.
- `ImproveTrail` happens only on worn-trail tiles, only when perceived benefit exceeds cost, and never when the household weight is 0 and the agent's own crossings are too few.
- Reciprocity scores stay within bounds and drift back to neutral; with reciprocity off, partner choice ignores them.
- No agent decision reads settlement detection results, the road network as a whole, or population-wide statistics.

### Integration tests

- Running the same seed/config for 1,000 ticks yields the same state hash.
- Save at tick 500, resume to tick 1,000, and compare against uninterrupted execution.
- Renderer on/off does not affect simulation state.
- Scenario branching starts from identical state.
- A drought modifies regeneration but never directly commands agents to migrate.
- Over a long run with random droughts on, drought frequency, size, length, and severity stay within their configured ranges, with at most one active at a time.
- A long run (for example 300 days) shows generational turnover: some initial agents die of old age and newborns reach maturity.
- A run with abundant food and low thresholds hits the 1,000-agent cap without exceeding it, and is flagged.

### Behavioural evaluation

The unit and integration tests check that the **mechanics** are correct. Behavioural evaluation checks that agents **act sensibly**. A run can conserve every resource and still have agents starving next to food or pacing back and forth. Accrete's agents are deterministic utility scorers, not language models, so this is ordinary automated testing rather than LLM-style evals.

There are four layers.

#### 1. Scenario tests (Week 2, extended in Week 3)

Each scenario is a tiny hand-built world, usually 10×10 tiles and loaded from a small file in `tests/scenarios/`, with one situation and an expected outcome within a tick limit. Scenarios bypass procedural generation, so they don't change when terrain generation is tuned.

Week 2 scenarios:

| Scenario | Expected outcome |
|---|---|
| Hungry agent, known food 3 tiles away | Walks there, gathers, and eats within 12 ticks |
| Hungry agent, no known food | Explores instead of idling |
| Exhausted agent | Rests before energy reaches 0 |
| Food hidden behind a hill | Agent is not drawn to it until it sees it |
| Unknown shortcut | Route uses only known tiles |
| Two agents, one food tile with little food | Resource is split without duplication; neither waits forever |
| Home over comfortable capacity for over 7 days | At least one adult moves out within 14 days |
| Agent eligible to reproduce, but hungry | Eats first |
| Full inventory, chest at home | Deposits instead of discarding or idling |

Week 3 scenarios:

| Scenario | Expected outcome |
|---|---|
| Two neighbours, one with surplus food and one with surplus wood | They trade |
| Two neighbours with identical surpluses | They don't trade |
| Partner with a low reciprocity score | Gets worse terms or is avoided |
| Busy worn trail used by the agent and its household | `ImproveTrail` is eventually chosen |
| Same trail, strictly selfish agent with few crossings | `ImproveTrail` is not chosen |

Scenarios check **outcomes within a window**, not exact tick-by-tick traces, so they keep passing while weights are retuned, as long as the behaviour stays sensible.

#### 2. Pathology detectors (Week 2)

These are counters that run during every simulation and appear in the metrics export and the viewer:

| Detector | Definition (defaults) |
|---|---|
| **Oscillating** | Goal switched more than 6 times in 24 ticks |
| **Stuck** | No movement and no progress on any goal for 12 ticks, while not resting |
| **Starved beside food** | Died of deprivation while food it knew about was reachable within 2 days of travel |
| **Idle** | More than 8 ticks in a day with no useful action while not resting |

A healthy run keeps each count near zero. Every flagged event is logged with the agent ID and tick, so it can be inspected.

#### 3. Behavioural benchmarks (Week 4)

Golden hashes fail on every parameter change, by design. Benchmarks fail only when behaviour leaves a sensible range. They run on the primary seed with fixed settings, and the ranges are stored in `tests/benchmarks/`:
- survival at day 100 within an expected range;
- mean hunger of living agents below a threshold;
- most homes within a few tiles of fertile land;
- pathology counts below their limits;
- births and deaths both happening, with the population neither collapsing nor hitting the cap.

The ranges are set from the first validated runs and widened or narrowed deliberately, with the reason recorded. Hashes show *whether* anything changed; benchmarks show whether it changed *for the worse*.

#### 4. Baselines (Week 4)

Two reference kinds of agent, chosen in the config, run on the same seed:
- **Random:** picks any valid action at random from its own RNG stream. This is the lower bound.
- **Omniscient:** uses the same decision logic, but sees the whole map and knows every resource. This is the upper bound.

Normal agents should land clearly between them on survival and on travel per unit of food gathered. Scoring like random agents means the decision logic isn't working. Scoring like omniscient agents means the limits on sight and memory aren't constraining anything. Baselines exist only for evaluation and never mix with normal agents in a run.

#### Later: pattern-oriented validation (Phase 2)

Once there are many seeds, check whether the results show broad patterns seen in real pre-modern societies, for example settlements concentrated near rivers and fertile land, and many small settlements with few large ones. These are comparisons of shape, not predictions, and a mismatch is reported rather than tuned away.

### Performance tests

- Benchmark 100, 500, and 1,000 agents over a fixed number of ticks.
- Track ticks per second, memory use, and pathfinding work.
- Budget: at 1,000 agents, observation should cost about 169 tile lookups per agent per observation, agent memory about 4 MB in total, and A\* at most 4,000 expansions per search.
- Avoid per-agent full-map searches every tick; cache paths or restrict search to known regions.
- Profile before optimizing. A 128×128 map and 100 agents should be an approachable laptop target, but exact performance depends on implementation and hardware.
- Run the benchmarks on both the Apple Silicon Mac and a Windows laptop, and record results for each.

### Cross-platform tests

- `cargo test --workspace` passes on both macOS and Windows.
- A snapshot saved on macOS loads on Windows, and one saved on Windows loads on macOS.
- **Golden hash tests:** for the primary validation seed (more seeds in Phase 2), the world-generation hash and the state hashes at ticks 0, 100, and 1,000 match the values in `tests/golden/`. The same files are checked on both OSes, so any difference between platforms fails the test.
- A snapshot saved on one OS and resumed on the other reaches the same state hash as an uninterrupted run.
- Release and debug builds produce identical hashes.
- `cargo clippy --workspace --all-targets` reports no disallowed float methods in `accrete-core`.
- `cargo build --offline` succeeds once dependencies are fetched or vendored.

### Known risks and mitigations

| Risk | Mitigation |
|---|---|
| Agents starve immediately | Tune starting conditions and verify resource accessibility |
| Population explodes or collapses | Tune reproduction thresholds and cooldown; safety cap at 1,000; flag capped runs |
| Random droughts too frequent or too harsh | Tune the daily chance, size, and severity; cooldown between droughts; config switch to turn them off |
| Agents hoard instead of trading | Spoilage, carry limits, and finite chest space; check with the barter on/off experiment |
| Homes become permanently overcrowded | Overcrowding cost, moving out, and shelter improvements |
| Initial agents all age out at once | Give initial agents seeded starting ages spread between maturity and elder age |
| Agents oscillate between goals | Goal commitment, hysteresis, cooldowns |
| Agents appear omniscient | Strict observation/memory interface |
| Settlements never form | Diagnose incentives; do not force settlements |
| Trails produce runaway routing | Cap trail benefits, use decay and realistic terrain costs |
| Replays diverge | Stable iteration order, isolated seeded RNG, snapshot RNG state |
| macOS and Windows results drift apart | Deterministic-float rules, `libm`, clippy bans, golden hashes tested on both OSes |
| Scope expands too quickly | Lock each week's acceptance tests before adding systems |

## 11. Visualization specification

Minimum UI:

- Main map: terrain, rivers, agents, homes, chests, trails, improved roads, and active droughts.
- Top bar: seed, tick/day, agent count, play/pause, step, speed, regenerate.
- Map layers: biome, elevation, food, wood, stone, traffic, settlement clusters.
- Inspector: click tile or agent; show state, local knowledge, current goal, and decision explanation.
- Metrics panel: population, births and deaths by cause, age distribution, average hunger, trade activity, settlement count, travel cost, and pathology counts (click a count to jump to the flagged agents).
- Experiment controls: load snapshot, select drought or route disruption, branch and compare.

Prioritize readability and responsiveness over elaborate graphics. Colored tiles and moving dots are enough for a convincing first version.

## 12. Definition of done

The MVP is complete when a user can:

1. Start Accrete locally on macOS (Apple Silicon) or Windows, offline and without cloud dependencies.
2. Generate a repeatable world from a seed.
3. Watch at least 100 independently acting agents survive, explore, and interact.
4. Inspect and understand individual decisions, and confirm with scenario tests, pathology counts, and baselines that agents behave sensibly.
5. Observe and measure whether stable clusters and transport patterns emerge.
6. Save a world, branch it into control and intervention scenarios, and replay both.
7. Export metrics and reproduce results using the CLI.

**A valid outcome is discovering that an expected behavior does not emerge.** The project's value is in creating a trustworthy environment to investigate why.

## 13. First VS Code / Codex task

Paste the following into Codex once the repository is open:

> Build **Accrete**, a local-first agent-based emergent civilization simulator in Rust. Create a Cargo workspace with `accrete-core` (pure simulation logic), `accrete-viewer` (macroquad visualization), and `accrete-cli` (headless execution). Implement **Week 1 only**: deterministic 128×128 procedural terrain generation with elevation, moisture, biomes, rivers, river-distance-based fertility, patchy food, forests, and clustered finite stone deposits as specified in `accrete.md`, movement-point terrain costs, precomputed line-of-sight masks, and viable starting positions for 100 agents. Add an interactive 2D renderer with pan, zoom, seed input, regenerate, and tile inspection. Keep simulation independent of rendering. Add reproducibility and validity tests. It must build and run offline on both macOS (Apple Silicon) and Windows, with no cloud or ML dependencies and no platform-specific code in `accrete-core`; pin the toolchain with `rust-toolchain.toml` and use `PathBuf` for all file paths. Simulation results must be bit-identical on both OSes: follow the *Cross-platform determinism* rules (no `std` transcendental functions or `mul_add` in `accrete-core`, use `libm`, write the terrain noise yourself, use a stable state hash), and add golden-hash tests under `tests/golden/`. Do not implement economy, trading, roads, or agent behavior yet. Compile and test incrementally.

### Recommended development loop

1. Ask Codex to implement **one bounded milestone**, not the entire roadmap.
2. Run `cargo fmt --all`, `cargo clippy --workspace --all-targets`, and `cargo test --workspace`.
3. Launch the viewer and inspect the result visually.
4. Record unexpected behavior as an issue or test case.
5. Commit the working milestone before adding the next feature.

---

**Long-term direction:** Explore how limited information, social knowledge exchange, and individual incentives influence the resilience and organization of emergent civilizations. Keep the agents simple enough that their collective behavior remains explainable.

---

## Appendix A — Implementation changelog (branch `sb/full-emergent-sim-claudeware`)

This appendix records what was built from this specification, in the order it was built. It also records every place where the specification left a choice open (or was found not to work as written), what was chosen, and why. The base is commit `e5a15ce` (this document and a README only). All numbers below are defaults in `configs/default.toml`, which a test keeps identical to `Config::default()`.

**Status against the plan:** Weeks 1–4 are complete. Phase 2 tooling is in place: the scarcity rerun (A.18), run manifests with `accrete reproduce`, seed ranges, parallel seeds and aggregates (A.19), and grid / Latin-hypercube parameter sweeps (A.20). Cross-PR review fixes for the stack are in A.21. Suggested next work in A.10: a SQLite/DuckDB run index, Parquet metrics, Sobol designs, and larger multi-seed Phase 2 campaigns.

### A.1 Repository and tooling

| Item | What and why |
|---|---|
| Cargo workspace | `accrete-core` (pure simulation), `accrete-viz` (overlay colours shared by viewer and CLI, so both draw identical pictures), `accrete-cli` (headless binary `accrete`), `accrete-viewer` (macroquad). `accrete-viz` was added beyond the spec's three crates to avoid duplicating colour logic. |
| `rust-toolchain.toml` | Pinned to `1.86.0` (the installed stable) with clippy and rustfmt, targets `aarch64-apple-darwin` and `x86_64-pc-windows-msvc`. |
| `Cargo.lock` | Committed. `fontdue` is pinned to 0.9.3 because 0.9.5 needs rustc 1.87. Edition 2024 with `rust-version = 1.86` lets the resolver respect the MSRV. |
| `clippy.toml` | Bans every `std` transcendental (`sin`, `exp`, `powf`, `powi`, …), `mul_add`, and `DefaultHasher::new`. CI runs clippy with `-D warnings`. The CLI renderer's circle drawing is the only explicit `allow`, because rendering never feeds back into the simulation. |
| `.github/workflows/ci.yml` | Runs fmt, clippy, release and debug tests (the same golden hashes on both), a viewer build and a headless smoke run on `macos-latest` and `windows-latest`. Results were verified locally on Windows only; macOS parity relies on CI. |
| `.gitattributes`, `.gitignore` | LF line endings for text, and `results/` is ignored. |
| `configs/default.toml` | Generated by `accrete config`. A unit test fails if it drifts from `Config::default()`. Every section uses `deny_unknown_fields`, so a typo in an experiment config is an error, not a silent default. |

### A.2 Determinism (Section 3, "Cross-platform determinism")

* **No transcendental math anywhere in the core.** Noise uses a quintic fade, freshness is `h/(h+age)`, seasons use a triangle wave, and distances use `sqrt` only, which is exactly specified. As a result `libm` turned out to be unnecessary and is not a dependency.
* **RNG.** One `ChaCha8Rng` per named stream (`WorldGen, Spawn, Agents, Births, Weather, Conflicts, Baseline, Founders`). The 256-bit key is the master seed expanded with SplitMix64, and the stream id sets ChaCha's stream counter. Float and range conversions are our own: a 24-bit mantissa, and Lemire's unbiased bounded integers. The stream id is part of the reproducibility contract, so ids are never renumbered.
* **State hash.** FNV-1a 64 over a canonical little-endian encoding in a fixed field order (`hash.rs`, with reference test vectors). Golden files cover the world and the simulation at ticks 0, 100 and 1000 for seed 1. Debug and release builds both match them.
* **Order independence.** Decisions read a world that is frozen during the proposal phase. Tie-break jitter is a stateless hash of `(agent, tick, option)`, so it consumes no RNG and does not depend on processing order.
* **Performance work was proven behaviour-neutral.** Three optimisations (batched observation, branch-and-bound option shortlists, cached exploration rays) each left the day-300 state hash bit-identical. Throughput went from about 180 to about 260 ticks/s with 600 agents.
* **Seeds** may be decimal, `0x…` hex, or free text (hashed with FNV-1a).

### A.3 World generation (Week 1)

| Decision | Rationale |
|---|---|
| Elevation = domain-warped fBm gradient noise (own implementation, 8 hashed gradients) + ridged noise masked by a low-frequency field − an edge falloff on warped coordinates | The first version produced one central dome with a square coast. Ridges masked to regions give mountain *chains*, and warping the falloff gives irregular coastlines. Verified visually across several seeds. |
| Sea level, hills and mountains set by **quantile** (30 % ocean; top 7 % / next 16 % of land) | Every seed then has usable land and relief, which keeps the simulation viable across seeds. Absolute thresholds made some seeds mostly ocean. |
| Lakes under 10 tiles filled, islands under 24 tiles sunk, 2 passes of a land-only majority filter | The spec asks to prevent isolated artifacts. The filter reads the old grid and writes a new one, so it does not depend on iteration order. |
| Moisture = fBm + a coastal bonus fading over 6 tiles; forest where moisture ≥ 0.56 | Gives coastal forests and an inland mix of plains and forest. |
| **Rivers:** priority-flood depression filling with an ε gradient (Barnes 2014), D8 flow to the lowest filled neighbour, accumulation of `1 + moisture` per tile, river where accumulation ≥ 110 | This is the "explicit treatment of local minima": no pit can trap water. Outlets are the ocean, lakes and border tiles (the map boundary). Accumulation never decreases downstream, so every river reaches an outlet. The validator checks adjacency, strict descent and that each river ends at an outlet. |
| Resources follow the spec formulas exactly; patch and deposit fields are min/max-normalised fBm | A bell-shaped distribution makes "rich" (> 0.65) patches a minority, which is the intent of the patch rule. |
| **Line of sight in metres:** relief 1,500 m, eye height 15 m, forest canopy 15 m blocks sight *through* forest | Plains see about 70 of 169 tiles and forest about 42, so ridges and woods genuinely hide resources. The 169-bit masks are rebuilt on load and tested against an independent reference. |
| **Movement is 8-connected.** A diagonal costs 14/10 of the tile cost (integer), with no corner cutting | The spec gives orthogonal costs only. Charging diagonals ×1.4 keeps distances near-Euclidean and travel time honest, and stays all-integer. |
| Spawns on the largest landmass with ≥ 60 food within 4 tiles, spaced ≥ 4 tiles apart, chosen uniformly | "Do not prebuild cities": nothing is clustered at the start, so any clustering is emergent. |

### A.4 Agents (Week 2)

**Knowledge.**
* Founders know the terrain, not the resources, within 8 tiles of where they start ("they grew up there"). They do not know the rest of the map.
* **The 64 resource-memory slots are partitioned by type: food 40, wood 16, stone 8.** With a single pool, plentiful forest wood evicted every food memory and agents starved beside food. Partitioning keeps the spec's size and its replacement rule (lowest `amount × freshness`, ties by position) within each pool.
* **Known homes (16 slots)** were added because `EstablishHome` "joins a known existing home", which requires remembering homes. Other households' homes are remembered as last seen; an agent's own household is known exactly.
* A known tile's static terrain (biome, river, fertility, lushness) is known. Standing resources are known only once seen.

**Decisions.** `U = Σ wᵢVᵢ − C` as in the spec, with every term recorded for the inspector (`accrete trace … --options`, and the viewer panel).
* *Eat* is one tick, so its urgency weight is set to beat almost anything once the agent is hungry.
* *Rest* includes a small *leisure* value. Without it, satisfied agents alternated between idling and resting and tripped the idle detector.
* *Hysteresis* adds `commitment × (1 + diligence)` to the current goal, **applied before the option list is trimmed**. An early bug trimmed first: a committed target was dropped from the list and the agent flip-flopped between two targets.
* *Walking home to rest* stays committed once chosen. An early bug re-checked the energy margin every tick, so an exhausted agent alternated between moving and resting and never arrived.

**Population regulation — the main calibration.**
* With the spec's food numbers, regrowth potential is about 8,000 units/day against 2 units/agent/day. Food alone never limits the population below the 1,000-agent cap, and every early run hit the cap by about day 200.
* Changing the world was rejected. Instead, four agent-level *motives* weigh against having a child:
  1. **Household food security.** About 6 days of food per household member, more for cautious agents.
  2. **Perceived land.** The known land within 5 tiles must offer about 5 food/day of regrowth potential per person sharing it. Sustainable harvest is roughly half of zero-stock regrowth, and agents do not reach every tile, so 5 ≈ 2.5× need.
  3. **Experienced foraging returns.** Food gathered per tick spent on food trips, as a moving average over days. This makes fertility respond to real local depletion.
  4. **Not when crowded** (per spec).
* All of these weights are configurable. Setting `reproduce_land_weight = 0` is the "no preventive check" experiment and reproduces the runaway growth.

**Where to live.**
* The land term in the site score *satisfices*: `min(ratio, 1) + 0.15 × excess`. A version that kept rewarding abundance sent every adult child to empty valleys, and no hamlets formed.
* Added motives, all individual preferences:
  * **Kin ties:** value living within 3 tiles of the home one grew up in.
  * **Land hunger:** explore further afield when the land at home feels crowded.
  * A river within 2 tiles, sociability-weighted neighbours, and crowding tolerated more by sociable agents.

**Other choices.**
* Reproduction is by a single parent, matching the spec's `parent` field. The birth roll succeeds 35 % of the time, with a one-day wait after a failed roll.
* Newborn food comes from the parent's hands, then the home chests.
* Juveniles stay within 10 tiles of home.
* Health recovers 0.1/day. Frailty grows 0.006 per deprived day. Age decline is 0.012/day after day 150, so a frailty-free agent dies at about 233 days; the observed mean lifespan is about 220.
* **Conflicts:** gathering on a shared tile is split proportionally to each request. Chest operations, homes, building and trades are resolved in a fresh permutation from the `conflicts` stream. Births roll in parent-ID order. No rule favours low IDs.
* **Resting agents re-decide** every 4 ticks (every 2 at full energy) unless hunger crosses their threshold. This only changes how often they reconsider, and saves about 60 % of decision work.
* **Daily phase:** regrowth, spoilage, skill decay, crowding clocks, traffic decay, road upkeep and settlement detection run on the last tick of each day, which is equivalent to the spec's "first tick of the next day".

**Pathology detectors.**
* **Oscillating** counts *abandonments*: a switch away from an unfinished goal **that was still on offer**. The literal "goal switched" rule flagged every explorer whose frontier target disappeared once it had been seen.
* **Stuck** credits accumulating movement points (crossing a mountain takes 6 ticks) as progress.
* **Idle** excludes leisure rest.
* The other two follow the spec.
* All four counters are 0 over 300 days on every seed tested.

### A.5 Social systems and infrastructure (Week 3)

| System | Implementation and rationale |
|---|---|
| Crowding, skills | Per spec: a 7-day grace period, rest penalty, frailty and no births when crowded. Skill aptitude 0.7–1.3, +0.005 per gathering tick up to +0.3, −0.001/day unused. |
| Settlement detection | Measurement only, never read by agents. Single linkage within R = 3, N = 5, M = 15, P = 60; identity is kept while at least half of the homes continue; hamlet, village and town classes. |
| Trails | `traffic = 0.95·traffic + crossings`. A tile with traffic ≥ 20 wears into a trail (cost × 0.8) and fades after 30 days below the threshold. Agents route with the path levels they have *seen* (changed-tile memory), never the true map. |
| **Knowledge sharing** | Agents on the same tile tell each other their best food memories, *keeping the original sighting time*, so nobody learns anything fresher than the teller knew. The number told is 1 + 2 × sociability (rounded). Nobody tells someone it distrusts. This follows the spec's "social knowledge exchange" direction and is the information-centre benefit of living together. |
| **Barter** | Per the trade rule. Each agent values goods by its own needs: units below its food reserve or wanted materials are worth 1 (food, × (1 + hunger)) or 0.6, and units beyond need are worth 0.05. The proposer prices at the midpoint of both sides' marginal rates. Each side must gain more than the time the trade takes, only goods actually held change hands, and nobody receives more than it can carry. A partner that distrusts the proposer demands up to 50 % better terms. Agents try their most-trusted neighbours first. |
| **Reciprocity** | After each trade, each side's score for the other changes by `0.15 × (1 − 4·max(0, partner_share − 0.5))`: up when the split was fair, down when the partner took most of the gain. Scores drift 2 %/day back to neutral. Can be switched off. |
| **Frequent tiles** | 16 slots; each crossing adds 1 and counts decay by a factor of 29/30 per day (≈ crossings over the last 30 days). Housemates read each other's lists. |
| **Roads (`ImproveTrail`)** | Considered only on a worn trail the agent stands on, while holding 2 wood and 2 stone. The spec rule is `benefit = time_saved × (own + household_weight × sociability × household crossings)` over 30 days. Cost is 4 ticks of labour (less for diligent agents) **plus 6 ticks for gathering the materials**; that materials term is an added assumption. Labour is priced at the current value of time, which rises with hunger and fatigue. Roads start at condition 180 and lose 1 per day without traffic. Setting `household_weight = 0` gives strictly selfish agents. |
| **Shelters (`ImproveShelter`)** | 20 wood, 10 stone and 12 ticks, as in the spec. **The materials are staged in the household chests**: 20 wood plus 10 stone weigh 70 units against a 30-unit carry limit, so as first written no shelter could ever be built. Residents deposit spare materials and the builder draws from its own hands, then the chests. Shelters are wanted when the home is full. |
| Building materials | One generic gather-wood / gather-stone option driven by `material_plan`: wood for storage, the shortfall in the household's shelter stock, and materials for roads on promising trails. Each trip is valued as a share of a full load; when it was valued as a share of the total target, no trip ever beat leisure. |

### A.6 Weather and seasons (Week 4, droughts)

* Random droughts follow the spec exactly: drawn on the `weather` stream, chance 1/150 per day, radius 6–16, length 10–40 days, severity 30–70 %, at most one random drought at a time, and a 60-day cooldown.
* **Scripted droughts** (`[[weather.scripted]]`) never draw from `weather`, so an experiment branch keeps the control's random droughts. A test checks this.
* Droughts act only by scaling regrowth (the spec's "interventions change world conditions, not agents").
* Optional **seasons** (`seasons_amplitude`, off by default) use a triangle wave, so they need no trigonometry.

### A.7 Measurement, tools, and visualization

* **Metrics CSV**, one row per day:
  * population and age classes; births and deaths by cause; hunger, energy, health and frailty; lifespan;
  * homes, chests and stores; trails, roads and shelters; trades, food traded and tips shared;
  * all pathology counters; the daily share of time on each goal.
* **Emergence metrics** (measurement only):
  * **Clark-Evans ratio R** of occupied homes: 1 = random, < 1 = clustered.
  * **Depletion ratio:** food near homes relative to elsewhere.
  * **Foraging radius:** mean distance from home at which food is gathered.
  * Mean experienced foraging return.
  * **Division of labour:** 1 − mean cosine similarity of practice vectors.
  * Home age and drought area.
* **CLI** (`accrete`):
  * `world` (stats, validation, hash; several seeds at once), `render` (any overlay, montage).
  * `run` (metrics, death log, maps, `--timelapse 0,75,150,…` strips of terrain plus depletion, `--set section.key=value` overrides, `--check` for per-tick invariants).
  * `trace` (tick-by-tick options and utility terms for chosen agents, or for the agents most recently flagged by a detector), `diag` (settlement perceptions, spatial structure, material bottlenecks), `config`.
* **Viewer** (`accrete-viewer`):
  * Pan, zoom and 12 overlays, including **depletion** (food as a share of capacity: red = stripped, green = untouched) and traffic.
  * The inspector shows every decision term and the reasons reproduction is blocked, and explains each death.
  * Map annotations: droughts, labelled settlements, home state (storage / shelter / vacant), roads, agent routes, and follow mode.
  * A chart of population, homes, hunger and clustering.
  * Seed entry and regeneration, `--play`, and `--screenshot` for headless verification.

### A.8 Tests

* **Unit tests:** hash vectors, RNG streams, noise, geometry, memory bounds and inheritance, movement costs, config round-trips, seasons, settlement linkage.
* **World generation:** determinism, seed variety, the validator on six seeds, river outlets, biome rules for resources, the river-distance fertility gradient, line of sight against brute force, spawn rules, and the golden world hash.
* **Simulation:** determinism over 1,000 ticks, golden state hashes, per-tick conservation and bounds over 20 days, memory bounds, birth rules, population cap and flag, deaths and causes, frailty never decreasing, near-zero pathologies.
* **Weather:** drought ranges and the cooldown; a scripted drought changes regrowth and leaves the `weather` stream untouched.
* **Social:** trade and sharing happen while conserving goods.
* **Scenarios** (`tests/scenarios/*.toml`, a hand-built world format with terrain, trail tiles, food, wood, homes, chests, crowding, agents with inventories, traits, crossings and reciprocity):
  * all 9 Week 2 scenarios from Section 10;
  * all 5 Week 3 scenarios: complementary surpluses trade; identical surpluses don't; a distrusted partner gets worse terms; a busy household trail becomes a road; a selfish agent with few crossings does not build.

### A.9 Results (seed 1–3, default config, 300 days)

| Seed | Population | Births | Deaths (deprivation / age) | Hamlets + forming | Clark-Evans R (day 30 → 300) | Foraging radius (tiles) | Depletion near homes | Shelters | Trail / road tiles | Trades / tips | Drought days |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 100 → 513 | 663 | 0 / 250 | 3 + 3 | 0.95 → 0.52 | 1.6 → 4.2 | 0.93 → 0.82 | 48 | 16 / 0 | 359 / 41,945 | 0 |
| 2 | 100 → 548 | 700 | 0 / 252 | 3 + 4 | 1.08 → 0.56 | 1.4 → 4.5 | 0.93 → 0.82 | 54 | 18 / 0 | 386 / 40,747 | 83 |
| 3 | 100 → 458 | 585 | 0 / 227 | 1 + 2 | 1.08 → 0.45 | 1.4 → 3.1 | 0.94 → 0.78 | 37 | 31 / 1 | 379 / 36,766 | 27 |

All four pathology counters were 0 on every seed. The emergent findings:
1. Homes go from a random pattern to strongly clustered (R ≈ 0.45–0.56), and kin-based hamlets persist for 100+ days.
2. Settlements visibly deplete their surroundings, and foraging trips lengthen by 2–3×.
3. Families upgrade crowded homes (37–54 shelter levels).
4. Knowledge spreads through meetings. Barter happens, though at a low rate.

Earlier calibrations (see A.4) showed the controlled-scarcity response the spec asks for. With food regrowth ×1 / ×0.4 / ×0.2, populations reached 605 / 157 / 42, **through fertility, not starvation**: a preventive check. Those runs predated trade, sharing, shelters and experienced returns. **They were re-run with the full system on seeds 1–3 in A.18:** 498 / 142 / 34 people at day 300, still with no starvation deaths.

**Honest negatives** (valid outcomes, per Section 1):
* **Roads barely appear** (0–1 tiles in 300 days). Under the spec's benefit rule, only trails crossed by a household dozens of times a month qualify, and agents forage close to home.
* **Division of labour stays near zero.** Everyone mostly gathers food; there is no production chain that would reward specialists.
* **Trade volume is modest**, because households living together have similar needs.

These point to the next experiments: longer trips (more scarcity), and goods that only some places produce.

### A.10 Not yet implemented, and suggested next steps

* **Run storage:** manifests and `accrete reproduce` are done (A.19). Still to do: Parquet metrics, periodic snapshots per run, and a SQLite or DuckDB index of runs.
* **Parameter → behaviour mapping:**
  * grid and Latin-hypercube sweeps over parameters, across seeds, are done (A.20); Sobol designs are still to do;
  * surrogate models (Gaussian process or gradient-boosted trees) from parameters to emergence metrics;
  * Sobol or Morris sensitivity indices and partial-dependence curves.

### A.11 Review fixes

Seven issues from code review were fixed. Four of them change behaviour, so the golden state hashes (ticks 100 and 1,000) were re-recorded and the A.9 results were re-run; the world hash is unchanged.

| Fix | Before | After |
|---|---|---|
| **Home choice from memory** | Site scoring read the true number of homes on a tile and the current shelter level of remembered homes. | `HomeMemory` stores the tile's home count and the home's comfortable capacity when seen; scoring uses those. A tile that filled up since is discovered on arrival. |
| **Failed join → scored fallback** | Arriving at a home that had filled up silently founded a new home on that tile, an option never scored. | The agent founds next door only if `site_score` for founding there is at least its score for staying put (always, if homeless); otherwise it re-plans. Logged as `JoinFellBackToFounding`. Removing the fallback entirely halved hamlet formation (seed 1: 4 + 2 → 2 + 2), so "their house is full, so I'll build next door" is kept as a deliberate, scored motive. |
| **Deaths before meetings** | Agents who died in a tick could still trade and share that tick. | Deaths are processed before meetings. |
| **Oscillation threshold** | Flagged at 6 abandonments in 24 ticks. | Flags at *more than* 6, as in Section 10. |
| **Cap event** | `PopulationCapReached` could be skipped when a birth was blocked in the tick the cap was reached. | Always logged once, when the cap is first reached or first blocks a birth. |
| **Trace intents** | `accrete trace` looked intents up by agent index, which shifts after deaths. | Intents are stored by agent ID (`Simulation::intent_of`). |
| **Trade-terms docs** | Comment and A.5 said distrust changes terms by up to 40 %. | Both say 50 %, matching the code (`1 + 0.5 × trust`). Leftover placeholder lines removed. |

Two unit tests in `simulation/engine.rs` cover both outcomes of the join fallback.

### A.12 Repository tooling: CLAUDE.md and CI path filtering

This entry also covers the CLAUDE.md change that was merged before this changelog rule existed.

**CLAUDE.md and the reminder hook (merged as PR 3).**
* `CLAUDE.md` gives Claude Code sessions the commands (including single tests and the exact CI clippy line), the crate architecture and tick pipeline, the belief-versus-truth rules for decisions, and the invariants that keep results bit-identical. Those cover banned float methods, RNG streams, state hashing, re-recording golden hashes, keeping `default.toml` in sync, "motives, never outcomes", and the resource ledger.
* A table maps each kind of code change to the CLAUDE.md section it affects. A project Stop hook (`.claude/hooks/claude-md-reminder.sh`) asks Claude once per new set of changes to review CLAUDE.md, when documented files changed on the branch but CLAUDE.md did not.
* A PR template turns the same checks into a checklist.

**CI path filtering (PR 4).**

| Change | Rust job | Debug core tests | Viewer build |
|---|---|---|---|
| Docs only (`*.md`, `docs/`, `.claude/`, PR template, `.gitignore`) | skipped | – | – |
| `accrete-cli` only | runs | skipped | skipped |
| `accrete-viz` or `accrete-viewer` only | runs | skipped | runs |
| Core, `configs/`, `tests/`, toolchain, dependencies, `.gitattributes`, `.github/`, or any unrecognised path | runs | runs | runs |

* **Classification:** `.github/scripts/ci-changes.sh` classifies the diff. Unknown paths deliberately run everything, so a new kind of file cannot skip CI.
* **Front-end-only changes skip the debug pass,** because front ends cannot change simulation results, and the debug pass exists to check that debug builds match the golden hashes.
* **`ci-result` always reports** and treats a skipped Rust job as a pass, so it is the single check to require in branch protection.
* **Triggers:** pull requests (including base changes, so a retargeted stacked PR is re-classified), pushes to `main`, merge-queue checks, and manual runs. Branches with an open PR were previously built twice, once for the push and once for the PR. A newer run for a PR cancels its older one. Every other run has its own concurrency group, so a push to `main` is never cancelled or dropped from the queue.
* **Fail-safe, after review.** A review of the first version found four ways CI could skip tests and still pass:
  * the classifier step ran without `pipefail`, so a broken script still passed;
  * a failed `git diff` gave an empty list, so everything was skipped;
  * `git diff --name-only` lists only the destination of a rename, so moving a scenario file into `docs/` skipped its tests;
  * the gates required `== 'true'`, so a missing output skipped the Rust job.

  The fixes:
  * `shell: bash`;
  * an empty or failed diff runs everything;
  * `--no-renames` with NUL-separated names;
  * the gates are `!= 'false'`;
  * `ci-result` fails if the classifier produced no explicit flag.

  PRs are now compared with the merge commit's first parent. Crate READMEs count as code.
* **Changelog rule:** every PR, including tooling, CI and docs PRs, now adds an Appendix A section like this one. This is recorded in CLAUDE.md and the PR template.

**Verification:**
* The classifier was run on five real commits from this repository's history (spec-only and docs-only commits skip Rust; the simulation commits run everything).
* `.github/scripts/test-ci-changes.sh` covers 16 cases in a throwaway repository and runs in CI before every classification: each category, renames, cross-category moves, deletions, non-ASCII names with spaces, an empty diff, and a missing or unknown base. Run against the first version of the classifier, it fails 5 of them.
* The first live CI run of the `changes` job passed.
### A.13 Snapshots and branching (Week 4 Days 1 and 3)

| Decision | Rationale |
|---|---|
| **Format: versioned JSON** (`*.snap.json`, `SNAPSHOT_FORMAT = 1`) | `serde_json` is already a dependency and is built with `float_roundtrip`, so every float reloads to the same bits on both platforms. A binary format would be smaller (a day-100 snapshot of seed 1 with about 280 agents is 8 MB), but would add a dependency; it can replace JSON later behind the version number. |
| **What is saved** | Everything that affects later ticks or the exported metrics: tick, config, world, agents with their memories, all four RNG streams, weather, ledger, settlement tracking, the day's counters, daily metrics, death records, pathology counts and the run totals. |
| **What is not saved** | Derived data, rebuilt on load: line-of-sight masks, the tile-to-homes index, exploration-ray caches, and per-tick rates. Inspection buffers, which start empty: the event log, the graveyard, decision traces and intents, and the pathology log. Pathfinding scratch. None of these affects results. |
| **Integrity check** | Each snapshot stores the state hash at save time. Loading recomputes it and rejects a mismatch, so an edited, truncated or incompatible file fails loudly instead of diverging. |
| **Interventions through the config only** | `Simulation::set_config` swaps the configuration of a resumed run and rebuilds rates and derived data. A drought branch adds a `[[weather.scripted]]` entry. Scripted droughts never draw from the `weather` stream, so both branches keep identical random droughts (tested). |
| **CLI** | `run --save` and `run --load`, and `branch`, which loads one snapshot twice, runs both branches for the same number of days, and writes a per-day comparison CSV (control, intervention, and the difference for population, births, deaths, food, homes, settlements, clustering, foraging radius, trades and drought area). |

**Verification:**
* Saving at tick 500 and resuming to tick 1,000 reaches the committed golden hash.
* A saved and reloaded snapshot re-serializes byte-identically.
* A straight 300-day run of seed 1 and a run saved at day 100 and resumed both end on hash `04841f5e6f62a0cf`.
* In a first branch on seed 1, a drought from day 100 for 120 days (radius 24, severity 0.6) ended at day 300 with 483 people against 513 in the control, 2 hamlets against 3, and slightly longer foraging trips. That is one seed and one intervention, not yet a finding.

**Review fixes** (from a review agent's pass over this PR; restore itself held up, with mid-day, in-drought, at-the-cap and post-extinction snapshots all resuming bit-identically):
* **Extinct branches.** When one branch died out, the comparison CSV stopped at that day and the summary compared different days. Branches now keep running after extinction (population 0), rows are aligned by day with empty cells for a missing side, and the summary names both days if they differ.
* **Labels.** A label containing `/` panicked after the control had run, and identical labels silently overwrote each other's files. Labels are validated before anything runs, and `branch` rejects identical labels.
* **Population cap.** Lowering it below the current population tripped the invariant check. `set_config` now returns an error for that, and for any change to `[world]`, which is already generated.
* **Unloadable files.** A NaN anywhere in the state was saved as JSON `null`, giving a file that could never be loaded. `save_snapshot` now checks that the file loads back to the same state before writing it.
* **Late save errors.** An unwritable `--save` path was only reported after the run. It is now checked first.
* **Partial integrity check.** The state hash does not cover metrics, the ledger, settlement tracking and other bookkeeping, so edits to those loaded silently. Snapshots now also carry a hash of their whole JSON body, checked on load. A file from a build with a different config schema is reported as such, not as "modified".
* **Debug settings counted as interventions.** `--check` and the event-log size changed the config digest, so a resumed run with `--check` was reported as an intervention and hashed differently. `Config::digest` now ignores debugging-only settings. No golden hash changed, because default runs use the defaults.
* **Pinned layout.** The snapshot file's bytes for seed 1 at tick 100 are now a golden value, so any change to the layout or a field's meaning forces a deliberate `SNAPSHOT_FORMAT` bump.
* **CLI checks:** `--seed` with `--load` is rejected; zero-day droughts are rejected; a drought starting after the branches end gets a warning.
* **New tests:**
  * body-hash rejection of edited metrics and ledger;
  * schema messages;
  * refusing non-finite state;
  * `set_config` validation;
  * debug settings not changing hashes;
  * mid-day, in-drought and at-cap resumes;
  * golden snapshot bytes;
  * a drought starting on the snapshot day taking effect the next day;
  * label and save-path checks;
  * day-aligned comparison rows.
* **Windows CI fix.** The save-path test used `/dev/null/x.snap.json` as an unwritable path. On Windows that resolves to `C:\dev\null\x.snap.json`, which can be created, so the test failed there. The test now uses a path under a regular file, which no platform can create. Only the test changed; no hash moved.

### A.14 Baselines and behavioural benchmarks (Section 10, layers 3 and 4)

| Decision | Rationale |
|---|---|
| **Random baseline** (`sim.agent_kind = "random"`) | Shuffles the scored options, moves Idle last, and takes the first one with a reachable target, so each action is valid but chosen at random, and the agent idles only when nothing else is reachable. It draws from the `agents` stream, which normal agents never use, so it has its own stream (as Section 10 asks) without changing any golden hash. |
| **Omniscient baseline** (`sim.agent_kind = "omniscient"`) | Same decision logic, perfect knowledge, the **same memory limits**. On every observation the agent knows the whole map's terrain, and for each resource it remembers the *nearest* tiles holding it (true amounts, ring scan nearest-first, up to its pool size), likewise the nearest homes. Routes use true costs. Keeping the memory limits means the gap to normal agents measures what limited *sight and stale memory* cost, not what extra memory would buy. |
| **Benchmark ranges** in `tests/benchmarks/primary_seed.toml` | Seed 1 at day 100: population, starvation deaths, hunger, homed share, the four pathology counters, walking per unit of food (with a lower bound, to catch agents that stop moving), how much more often homes sit near good land than land in general does, generational turnover (births and old-age deaths), and not hitting the cap. The observed values when the ranges were set are written next to each range. Golden hashes show that something changed; these show whether behaviour got worse. |
| **Baseline ordering** as a test | At day 60, normal agents must reach at least 2× the random population and at most 1.05× the omniscient one. |

**First results** (seed 1, 100 days):

| Agents | Population | Starvation deaths | Tiles walked per unit of food |
|---|---|---|---|
| Random | 34 | 58 | 5.49 |
| Normal | 284 | 0 | 0.48 |
| Omniscient | 318 | 0 | 0.25 |

Normal agents are far better than random, so the decision logic works. Limited sight and memory cost about 12 % of the population and nearly double the walking, so the knowledge limits genuinely constrain behaviour. Random agents also trip the oscillation detector thousands of times, a useful check that the detector responds.

**Review fixes** (from a review agent's pass over this PR):
* **Roads invisible when choosing.** Omniscient agents routed with true costs but *scored* options with no-road costs, ignored roads when deciding whether to upgrade a trail, and checked "starved beside food" with remembered costs. `decision::omniscient` and `decision::believed_path` now give them the true world wherever a belief is read.
* **Wrong home evicted.** A tile holding several homes could overflow the home pool and evict the home the agent stood on. The cap is now checked per home.
* **Full-map scans.** The ring scan visited every cell of each ring's square and never stopped while the world had fewer homes or resource tiles than a pool holds, so it scanned the whole map on every observation (about 1.1 s per simulated day early in a run). It now walks only each ring's perimeter and caps each pool at what the world actually holds. Thirty omniscient days take 1.3 s, down from 3.5 s.
* **Unreachable food crowding out reachable food.** A nearest-but-unreachable resource (across water, say) was re-added on every observation. Entries still on their unreachable cooldown are now skipped, so the pool fills with other places; they are offered again after the cooldown. The skip must consult `failed_targets`, not the resource pool: `set_resources` replaces that pool on every re-observe, so a cooldown stored only there was wiped after `reobserve_ticks` (6) instead of lasting the full window (48). The existing test now re-observes inside the window and fails without the fix.
* **Idle not last.** The random baseline's comment said Idle was a last resort, but it was shuffled like everything else. It is now moved last, so random agents idle only when nothing else is reachable. This raised the random baseline at day 100 from 24 to 34 people.
* **Benchmarks:**
  * "homes near fertile land" was nearly trivial, because 94 % of all land qualifies. It is now the lift over the land average (0.10 observed, range ≥ 0.03).
  * Walking per unit of food gained a lower bound.
  * Boolean metrics such as `at_cap` can be range keys.
  * A malformed ranges file names the key at fault.
  * The observed baseline ratios are recorded next to their limits.
* **New tests:** the home pool on shared tiles; deterministic, pool-respecting omniscient observation; the cooldown skip; omniscient travel estimates using true roads; omniscient runs staying on land and round-tripping through snapshots; and the random baseline being deterministic with Idle last.

### A.15 Route disruptions (Week 4 Day 2)

| Decision | Rationale |
|---|---|
| **Effects:** `Blocked` (impassable) or `Slowed { factor }` (entering costs `factor` times the movement points) | The spec's "temporarily impassable or expensive path tiles". Integer factors keep movement exactly reproducible. |
| **Scheduled in `[[disruption.scripted]]`**, started and ended in the daily phase | The same pattern as scripted droughts. A disruption covers either the land tiles in a `center`/`radius` area, or the `busiest_tiles` most-travelled land tiles **without homes** on the start day. |
| **Busiest tiles exclude homes** | The first version picked home tiles, which carry the most traffic, so it flooded villages instead of cutting routes, and one agent stayed on its blocked home tile for the whole disruption. A route failure should hit paths. An explicit area still covers everything in it, homes included (a local flood). |
| **State outside `Tile`** | `Simulation::disruptions` is authoritative, hashed and saved in snapshots (`SNAPSHOT_FORMAT` 3). `World::disruption_at` is a derived index. Keeping it out of `Tile` leaves the world-generation hash unchanged. |
| **Agents are never told** | Agents plan with what they believe. Seen disruptions live in their own memory pool (separate from trails and roads); a planned step that turns out to be blocked is also recorded; and the agent re-plans. A restored tile looks normal when next seen. Agents caught in a blocked area wade out: a blocked tile can be entered only from another blocked tile, at 4× its cost (`movement::WADE_FACTOR`), so nobody wades in. |
| **CLI:** `branch --disrupt DAY:DAYS:block\|slowF:X:Y:RADIUS` or `…:busiest:N` | Comparison CSVs gain `disrupted_tiles`, `tiles_moved` and `route_failures`, and the map frames draw disrupted tiles. A daily `disrupted_tiles` metric is added. |

**Golden hashes were re-recorded with no behaviour change.** The new `[disruption]` config section changes the config digest, and changed-tile memory hashes the new field. A 300-day run of seed 1 against the previous build produced identical metrics on every day, apart from the new all-zero `disrupted_tiles` column, and an identical death log.

**Tests:**
* **Unit:** cost rules, area and busiest-tile selection (busiest first, homes skipped), overlapping effects keep the worst, and the index clears when a disruption ends.
* **Scenario `blocked_route`:** an agent that planned through a gap notices the block once the gap is in sight, never steps on it, and reaches the food through the other gap.
* **Whole world:** blocking the 12 busiest tiles from day 60 for 15 days on seed 1. Nobody steps onto a blocked tile, the 4 agents caught on them leave within hours, invariants hold, the tiles are restored afterwards, and random droughts match the control.

**First result** (seed 1, the 20 busiest non-home tiles blocked from day 100 for 20 days): failed route plans rose from 6 to 87 and walking rose by 5 % during the disruption, while population barely changed (339 against 341 at day 140). The road network is still thin by then, so there are many alternative routes. One seed, not yet a finding.

**Rebased onto the reviewed snapshot and baseline PRs:**
* **Golden snapshot bytes re-recorded.** `snapshot_seed1_t100` was re-recorded for `SNAPSHOT_FORMAT` 3 (disruption memory layout).
* **`--disrupt` checks match `--drought`.** A zero-day disruption and `busiest:0` are rejected; `busiest:0` would otherwise have become a radius-0 disruption at tile (0, 0). A disruption starting after the branches end gets a warning. A disruption-only intervention is detected with `Config::differs_in_results`, like every other intervention.
* **Checked `set_config`.** The whole-world disruption test now fails if `set_config` rejects its config, instead of ignoring the `Result`.

**Review fixes** (from a review agent's pass over this PR; it found no determinism, snapshot or blocked-tile violations, and resuming mid-disruption, including overlapping disruptions, matched an uninterrupted run):
* **Flooded homes retried every tick.** A route failure records the target in `failed_targets`, but only exploration checked it, so residents locked out of a flooded home re-planned to it almost every tick (113 failures in 10 days for one agent). `decision::evaluate` now drops any option whose target is on that cooldown, unless the agent is already there. No golden hash changed: normal runs never hit this.
* **`slow255` could cancel a block.** A slow factor of 255 tied with `Blocked` when overlapping effects keep the worst, so the tile could stay passable. Slow factors are now limited to 2–254.
* **Disruptions from TOML were not validated.** `ScriptedDisruption::validate` rejects zero days, slow factors outside 2–254, and entries setting both `busiest_tiles` and `radius`. It runs on config load, in `set_config` and for `--disrupt`.
* **Huge radii.** The radius is clamped to the map size, so a radius beyond 46,340 can no longer overflow or loop for minutes.
* **Overlays.** The move-cost overlay and the viewer's tile inspector now show disrupted costs.
* **New tests:** validation (including through TOML and `set_config`), the radius clamp, residents of a flooded home waiting out the cooldown (fails without the fix) and agents caught inside wading out, wading rules for costs and A\*, and the busiest-trails test now checking that nobody enters a blocked tile from open ground.
* **Trapped agents wade out.** An agent inside a blocked *area* (not at its edge) could not move until the flood ended, and kept re-trying routes because its 8 failed-target slots overflowed. Now a blocked tile can be entered from another blocked tile at `WADE_FACTOR` (4) times its cost, in both A\* and movement, so agents wade out but nobody wades in. In the flooded-home test, 3 of the 4 agents caught inside left within two days; the fourth chose to rest where it stood. No golden hash changed, and a 200-day run of seed 1 took the same time.
* **Dedicated disruption memory.** Disruptions used to share the 32 changed-tile slots with trails and roads, so a large flood could push path memories out and agents could forget blocked tiles for lack of room. Disruptions now have their own pool (`memory.disruption_slots = 1024`), kept sorted by tile for binary-search lookups. When full, the entry farthest from the agent is replaced. Trails and roads stay in the 32-slot changed pool. A radius-15 flood (~519 tiles) fits whole. `SNAPSHOT_FORMAT` is 3. Golden hashes were re-recorded; a 200-day run of seed 1 matched the previous build on every metric and the death log, apart from the new memory layout.
* **New tests:** floods do not crowd out roads (unit and whole-world); farthest-first eviction; equal-distance ties; the pool stays sorted; newborns inherit disruptions; re-seeing a restored tile clears the belief; agents hold more than 32 disruption tiles during a large flood; after a busiest-tile flood ends, at least one agent still remembers a blocked tile.
### A.16 Viewer experiment controls (Week 4 Day 5)

The spec's "load snapshot, select drought or route disruption, branch and compare", in the interactive viewer, all keyboard-driven:

| Key or flag | Action |
|---|---|
| `K` / `L` | Save a snapshot to `results/viewer_seed…_day…_t….snap.json`; reload the last one (a rewind). |
| `--load FILE` | Start from a snapshot. |
| `I` | Cycle intervention presets: a drought around the selected tile (radius 16, 60 days, −60 % regrowth), cut the 20 busiest routes (20 days), flood the area round the selected tile (radius 3, 30 days, impassable), or halve food regrowth. |
| `B` | Branch from now. An exact copy (through a snapshot round trip) gets the intervention, and both simulations then step in lockstep. |
| `V` / `X` | Switch the branch on screen; drop the branch. |
| `--intervene N` (`--view-b`) | Branch at start, for headless screenshots. |

**Decisions:**
* **Presets change world conditions only,** through the same config paths as `accrete branch` (scripted droughts, `[[disruption.scripted]]`, regrowth rate), starting on the branch day. Interventions centre on the selected tile, or on the map centre when nothing is selected.
* **What's shown:** the banner names the branch, the preset and which side is on screen; the chart overlays the other branch's population in orange with the live difference; disrupted tiles are drawn in red (blocked) or orange (slowed).
* **Speed:** while branched, the per-frame step budget is halved, because two simulations step per tick.

**Verification:** headless screenshots after loading a day-60 snapshot of seed 1 and branching with `--intervene 1` (cut routes, viewing the intervention) and `--intervene 0` (drought, viewing the control) show the banner, the red disrupted tiles and the overlaid population line. The viewer only calls `Simulation::step`, `snapshot_json` and `set_config`, all of which are tested in core.

**Rebased onto the reviewed PRs:** `set_config` now returns a `Result`, and the viewer ignored it. A rejected intervention would have shown an unchanged copy labelled as the intervention. Branching now reports the error and does not branch. CI does not lint the viewer, so the compiler warning alone would not have caught this.

**Review fixes** (from a review agent's pass over this PR; the control really is unchanged, both simulations stay on the same tick, and nothing in the viewer feeds back into simulation state):
* **`--intervene` ignored `--select`.** The branch was made before the selection was set, so area presets centred on the map centre. The selection is now set first.
* **`--load` with `--day`, `--seed` or `--config`.** `--day` ran that many extra days on top of the snapshot, and `--seed` and `--config` were silently dropped. `--day` is now the day to show (a warning is printed if the snapshot is already past it), and `--seed` or `--config` with `--load` is an error.
* **`--intervene` failed silently.** An unparseable or out-of-range preset now exits with an error, and so does a branch that fails at startup.
* **Snapshots of a branch.** `K` while branched now adds `_control` or `_intervention` to the file name, so saving both sides at one tick no longer overwrites. `L` says when the reloaded state carries an intervention.
* **New worlds after `--load`.** `G` and `S` no longer carry the snapshot's scripted droughts and disruptions, pinned to its days and tiles, into a new world. `L` now does the same: it syncs `seed`/`config` from the reloaded simulation and strips scripted interventions, matching `--load`.
* **Agent ids across branches.** Each branch numbers its own newborns, so after `V` (or `L`) the selection is kept only when `id < next_id` at the split (and the agent still exists). Using `born_tick < from_tick` was wrong: founders have `born_tick = 0`, so a branch at tick 0 cleared every selection, and agents born on the last step before the split were dropped too.
* **`--day` on an exact-day snapshot** no longer prints a spurious "past --day" warning.
* **Chart.** The population scale covers both branches, and the legend mentions the orange line only when branched. `V` with no branch now says so.
* **New test, now run in CI:** every preset is accepted by `set_config`, starts on the branch day and changes the outcome, and a control stepped alongside a branch matches an unbranched run. CI's viewer step now runs `cargo test --release -p accrete-viewer`.

### A.17 Emergence experiment suite (Week 3 Day 5)

The three paired ablations from the Day 5 plan, as `accrete experiments`:

| Experiment | Intervention | What it measures |
|---|---|---|
| `trade` | `social.trade_enabled = false` | Whether barter changes population, foraging, and clustering |
| `trails` | `trails.enabled = false` (formation off; traffic still counted) | Whether worn trails thicken the path network and shorten trips |
| `selfish` | `infrastructure.household_weight = 0` and `social.reciprocity_enabled = false` | Whether household concern and reciprocity change outcomes |

**Design:**
* **Fresh worlds, shared control.** Each seed starts from world-gen (not a snapshot), so early emergence is in scope. The control runs once per seed and is reused against every intervention.
* **Config-only interventions.** Each ablation goes through the same `Config` knobs a manual `--set` would use, and `Config::differs_in_results` asserts that the intervention actually changes results.
* **Outputs.** Per-day `control_vs_<label>_seedN_compare.csv` (same columns as `branch`, plus travel-per-food, food traded, tips shared, trail/road tiles and pathology counters), plus `emergence_summary.csv` with final-day metrics and **cumulative** trades across the run. State hashes are recorded so a non-diverging pair is visible.
* **CLI:** `accrete experiments --seed 1,2 --days 100 -o results/emergence`, with `--only trade,trails,selfish`, `--every`, `--check` and `--set` on the shared base.

The Week 3 behavioural scenarios (complementary and identical trade, reciprocity terms, busy-trail roads, selfish road reluctance) were already in the tree; this suite is the multi-day headless comparison the Day 5 plan asked for.

**First results** (seed 1, 100 days; trades are run totals):

| Pair | Population | Trades | Trail tiles | Notable |
|---|---|---|---|---|
| trade | 284 → 286 | 107 → 0 | 7 → 2 | No barter under the intervention; hashes diverge |
| trails | 284 → 283 | 107 → 95 | 7 → 0 | No worn trails; forage radius 2.92 → 3.11 |
| selfish | 284 → 283 | 107 → 87 | 7 → 3 | Hashes diverge; slightly fewer trails |

One seed and 100 days, not yet a finding. Phase 2 (Section 9) is multi-seed averages.

**Tests:** each ablation changes the config; `--only` rejects unknown names and dedupes; a 40-day paired run on seed 1 diverges for every experiment, with zero trades under `no-trade` and zero trail tiles under `no-trails`.

**Review fixes:**
* `--days 0` and an empty seed list are rejected before any run.
* An ablation that is a no-op under the base config (e.g. `--set social.trade_enabled=false` then `--only trade`) exits with an error instead of panicking on `assert!`.
* `--only` duplicates are dropped in order; the trails description matches what `trails.enabled` actually does (formation, not a separate cost gate).

### A.18 Scarcity rerun with the full system (Section 9, "Scarcity")

The scarcity results quoted in A.9 came from the calibration runs described in A.4, made before trade, knowledge sharing, shelters and experienced foraging returns existed. This entry re-runs them with everything in place.

**How:**
* `accrete experiments` gains two levels, `scarcity40` and `scarcity20`, which scale the base config's `resources.food_regrowth` by 0.4 and 0.2. They are the original calibration's levels, and they compose with `--set`. A base regrowth of 0 is rejected up front.
* Run: `accrete experiments --seed N --days 300 --only scarcity40,scarcity20` for seeds 1–3, each from a fresh world with a shared control. To check for equilibrium, both levels were also run alone for **900 days** (`run --set resources.food_regrowth=0.06` and `=0.03`).
* **Summary fixes made along the way:**
  * the `deaths_deprivation_*` columns held the **final day's** count, not the run's;
  * the summary now has run totals (births, deaths by cause, home moves), **births per adult-day** (per-capita fertility), and mean lifespan, which is left empty when no one born in the run has died;
  * the header is a single constant, and tests check rows against it by name and against each side's metrics file.

**Results at day 300** (seeds 1 / 2 / 3; the means are of the three seeds):

| Food regrowth | Population | Births | Births per adult-day | Starvation deaths | Settlements (hamlets + forming) | Clustering R |
|---|---|---|---|---|---|---|
| ×1.0 | 513 / 522 / 458 (mean 498), still growing | 641 | 0.0122 | **0** | 1.7 + 2.0 | 0.49 |
| ×0.4 | 148 / 146 / 133 (mean 142) | 219 | 0.0096 | **0** | 0 | 0.94 |
| ×0.2 | 40 / 35 / 26 (mean 34) | 58 | 0.0055 | **0** | 0 | 1.17 (n = 11–18 homes) |

**900-day runs:** ×0.4 stays between 128 and 158 people from day 300 to 900 on every seed. ×0.2 drifts from 25–40 down to 22–30 by about day 500, then holds to day 900. There were no extinctions and no starvation deaths.

**What this shows** (three seeds: a pattern, not yet a Phase 2 finding):
1. **Population adjusts through births, not starvation.** No agent died of deprivation in any of the 15 runs (nine 300-day runs, six 900-day runs). Total births fall about 11×, but **per-capita fertility falls only about 2.2×** (0.0122 → 0.0096 → 0.0055 births per adult-day). The rest comes from having fewer adults, a feedback in which fewer births mean fewer future parents.
   * The fall in fertility comes mostly from **food**, partly through the hard reproduction requirement and partly through the food-security motive. A review measured seed 1 over every adult-day: the "no food surplus" requirement (`life.reproduce_food_carried` / `reproduce_food_stored`) blocks 8 % / 12 % / 23 % of adult-days at ×1 / ×0.4 / ×0.2, and the food-security motive falls below 1 for 27 % → 44 % of eligible adults. Perceived land is below 1 for 94–99 % of eligible adults at *every* level, so it barely tells the levels apart.
   * **Caveat:** an agent worn down by deprivation dies when its health ceiling reaches 0, which is recorded as an "old age" death. So "no starvation deaths" means nobody's health hit 0 from hunger; it does not mean deprivation had no effect.
2. **Scarce populations reach a steady state.** At ×0.4 it is about 130–160 people, and at ×0.2 about 22–30, both from roughly day 300–500 (see the 900-day runs). The full-food control is still growing at day 300 and is *not* at equilibrium, so the day-300 ratios understate the eventual gap.
3. **At ×0.4, no settlements form and homes are randomly placed** (R ≈ 0.94, against 0.49 with full food). Clustering seems to depend on local surplus. At ×0.2 no claim is made: a hamlet needs at least 5 homes and 15 residents, 40–60 % of the whole population, so the threshold alone rules hamlets out. R there comes from only 11–18 homes, with a standard error of about 0.12–0.16, and seed 2 gives 0.88.
4. **Migration does not clearly rise.** Total home moves fall from 241 to 26 and 19. Per final-day resident the picture is mixed (0.48, 0.18 and 0.56, the last on a tiny denominator). The spec expected scarcity to drive migration. A proper measure (moves per person-year, distance moved) is needed before drawing a conclusion; left open for Phase 2.
5. **Other measures, with their spread:**
   * **Hunger:** mean hunger over the last 30 days is similar on average (0.19 / 0.18 / 0.18), but individual seeds range from 0.16 to 0.22 at ×0.2.
   * **Foraging radius:** shorter on average (3.65 → 2.97 → 2.63 tiles), though not on every seed (seed 3 is longer at ×0.2).
   * **Standing food:** slightly *higher* under scarcity at day 300, because the much smaller population leaves most tiles near capacity.
   * **Lifespan** (221–225 days) cannot detect scarcity here. Only cohorts born by about day 60–90 can have died by day 300, and they grew up while food was still plentiful; at ×0.2 every one of them reached the frailty-free maximum (225 days).
6. **Comparison with the original calibration:** 142 / 34 against 157 / 42. The two use different code versions (experienced returns is itself a newer reproduction motive), so this does not show that trade, sharing or shelters leave the scarcity response unchanged. Testing that needs the trade and selfish ablations run at a scarce regrowth rate (`--set resources.food_regrowth=0.03`).

**Tests:**
* Both levels scale the base rate and have valid labels.
* A 30-day ×0.2 pair has less standing food, no more people, and a diverging hash.
* Summary rows match the header by column name and use run totals.
* An end-to-end `run_suite` test checks the summary's totals against each side's metrics file.
* A no-op scarcity (base regrowth 0) is rejected before anything runs.

**Review fixes** (from a review agent that reproduced the day-300 table exactly):
* Test directories could collide on macOS's microsecond clock (a flaky failure about 1 in 19 runs); they now use a counter.
* The no-op guard ran after the control and earlier pairs had already run; it now checks every experiment before any run.
* Several claims were narrowed to match the evidence: the mechanism behind the fertility fall, equilibrium (now backed by the 900-day runs), settlements at ×0.2, lifespan, the comparison with the calibration, and per-seed spread.

### A.19 Phase 2 tooling: manifests, seed ranges, parallel seeds, aggregates

Phase 2 (Section 9) asks for results across many seeds with uncertainty, and A.10 asks for runs that can be reproduced from a record. This entry adds both. Parameter sweeps come next.

| Piece | What and why |
|---|---|
| **Run manifests** (`<label>_seed<N>_manifest.json`) | Written by `run::drive`, so every `run`, `branch` and `experiments` run has one. It records how to recreate the run (a fresh world from seed and config, or a snapshot file with its byte hash plus the config applied on load), the tick count, the start and final state hashes, the full config, the command line, and provenance: accrete version, git commit and dirty flag (embedded by `crates/accrete-cli/build.rs`, which works offline), and a hash of `Cargo.lock`. Snapshot paths are stored absolute; if one has moved, the copy next to the manifest is used. |
| **`accrete reproduce <manifest>`** | Re-runs the manifest and checks the final state hash, exiting 1 if it differs. It notes when the current build's commit or `Cargo.lock` differs from the recording one, or when the recording build was dirty. It refuses a snapshot whose bytes have changed. Determinism makes the final hash a complete check. |
| **Seed ranges** (`--seed 1-10`) | `seed::parse_seed_list` in core, for `run`, `experiments`, `world` and `render`. An integer range expands; anything else is one seed, so text seeds with a dash (`river-valley`) still work. Duplicates are dropped; backwards or oversized ranges (over 10,000) are rejected. |
| **Parallel seeds** (`experiments --jobs N`) | Seeds are independent simulations, which the determinism rules allow to run in parallel; each seed's runs stay sequential. `parallel::map_ordered` returns results in seed order, so every output file is byte-identical to `--jobs 1` (tested). Four seeds ran in 2.2 s with `--jobs 4`, against 6.4 s sequentially. |
| **Aggregates** (`emergence_aggregate.csv`) | Per experiment and outcome, over seeds: the mean control and intervention values, the mean **paired** difference (intervention minus control, same seed) with its sample SD and 95 % t-interval, and how many seeds went up or down. Cells are left empty where a statistic is undefined (one seed). Outcomes: population, total births, births per adult-day, starvation deaths, home moves, trades, trail tiles, road tiles, hamlets, clustering R, foraging radius, and walking per unit of food. A summary is printed at the end. |

**Tests:**
* A fresh run and a run resumed with an intervention both reproduce from their manifests; a manifest with one tick fewer does not; a changed snapshot file is refused.
* Manifests round-trip, and other format versions are rejected.
* Seed-list parsing covers ranges, dashed text seeds, deduplication, and backwards, oversized and empty lists.
* `map_ordered` keeps order for 1–64 jobs.
* The t-interval is checked against hand-computed values.
* The aggregate pairs differences by seed and leaves undefined cells empty.
* Three seeds give byte-identical summary, aggregate and metrics files with `--jobs 1` and `--jobs 3`.

**Review fixes** (from a review agent's pass over this PR):
* **Stale dirty flag.** The build script re-ran only on git HEAD and index changes, so a build made after an unstaged edit could record `git_dirty: false`. It now re-runs when any workspace source, manifest, config or the lock file changes. It finds git's files with `git rev-parse`, so a git worktree (where `.git` is a file) no longer forces a full rebuild every time, and it sets `GIT_OPTIONAL_LOCKS=0` so `git status` can't trigger its own re-run.
* **Manifests depended on `--jobs`.** The recorded command line included it; `--jobs` is now left out (it cannot change results), and arguments are recorded lossily, so a non-UTF-8 argument no longer panics after a run. An end-to-end test now compares *every* output file, manifests included, between `--jobs 1` and `--jobs 3`.
* **Unreproducible `run --load X --save X`.** The start snapshot was overwritten. This is now refused before running.
* **Moved results folders.** Manifests also store the snapshot path relative to the manifest, and `reproduce` tries each candidate (relative, as recorded, next to the manifest) and takes the first whose bytes match the recorded hash. Output paths are relative to the manifest and include every image written.
* **`reproduce` checks more.** It now checks that the snapshot's seed matches, that start + ticks = final, and it fails (exit 1) when the start state differs.
* **A failing seed under `--jobs`.** Other seeds used to keep starting. Now no new item starts after a failure, and the error names the item that failed. `--jobs 0` is rejected.
* **t-values above 30 degrees of freedom** jumped to 1.96, about 4 % too narrow. They now use a Cornish-Fisher expansion, within 0.001 of exact (2.040 at df 31). Aggregates count only seeds where both sides are finite.
* **Seed lists:**
  * `1 - 3` (spaces) is a range;
  * half-written (`1-`, `-3`) and hex (`0x1-0x3`) ranges are rejected instead of silently hashed;
  * deduplication uses a set.
  * **Behaviour change:** an argument like `2-3` used to be a hashed text seed and is now the range 2–3.
* **New end-to-end tests** (`crates/accrete-cli/tests/cli.rs`, using the real binary):
  * `reproduce` exit codes 0 and 1;
  * every output file identical across `--jobs`;
  * `--save` over `--load` refused;
  * a moved results folder and both branch arms reproducing;
  * `--jobs 0` rejected.
### A.20 Parameter sweeps (A.10, "parameter → behaviour mapping")

`accrete sweep` runs the model over a set of parameter points, each on several seeds, and reports how the outcomes respond.

| Piece | What and why |
|---|---|
| **Grid** (`--param section.key=a,b,c`, repeatable) | The cartesian product of the listed values, with the first parameter varying slowest. Booleans and other non-numeric keys work here. |
| **Latin hypercube** (`--lhs N --param section.key=lo..hi`) | N points that cover each parameter's range evenly: every parameter gets exactly one point in each of its N strata, at a random spot inside the stratum. Numeric keys only; integer keys stratify the integers in `[ceil(lo), floor(hi)]` (with repetition when N exceeds that count). The design draws from a new RNG stream, `Design = 9`, appended under the never-renumber rule. Simulations never use it, so no hash changes. `--design-seed` makes a design reproducible. |
| **Validation first** | Every point is applied to the base config (`run::try_apply_override`) and must pass `Config::validate` before anything runs. A bad key or value, a non-finite number, a range in a grid, a list in an LHS, a duplicate key, a debugging-only grid that does not change results, `--jobs 0`, or more than 100,000 runs/points is an error up front. `--set` now exits with status 2 on a bad override instead of panicking. |
| **Parallel, deterministic** | (point, seed) pairs run through `parallel::map_ordered`, and outputs are byte-identical whatever `--jobs` is (tested). Each run writes `pNNN_seedS_*` metrics and a manifest, so any single run can be checked with `accrete reproduce`. A panic in one run names the point and seed and writes no summary. |
| **Outputs** | `sweep_runs.csv` and `sweep_points.csv` first; `sweep_design.json` last (and only on success), including the `--set` list and the full base config so a finished design can be audited. Parameter values in the CSVs are canonical (parsed and re-printed), so `0.10` and `0.1` are one grid point. |

**Demo** (`--param resources.food_regrowth=0.03,0.06,0.1,0.15 --seed 1-3 --days 150 --jobs 8`; 12 runs in 15 s):

| Food regrowth | Population, day 150 (mean, 95 % CI) | Births per adult-day | Clustering R |
|---|---|---|---|
| 0.03 | 71 [52, 89] | 0.0046 | 1.06 |
| 0.06 | 165 [141, 189] | 0.0108 | 0.93 |
| 0.10 | 263 [194, 331] | 0.0147 | 0.82 |
| 0.15 | 353 [271, 435] | 0.0185 | 0.68 |

A smooth dose-response: more food gives more people, higher per-capita fertility and tighter clustering, consistent with A.18. Three seeds and 150 days, so this demonstrates the tool rather than establishing a result.

**Tests:**
* Parameter parsing: values, ranges, and bad input.
* Grids are cartesian products in the stated order; numeric aliases (`0.1`, `0.10`, `+0.1`) collapse to one point.
* A Latin hypercube puts one point in every stratum, is reproducible from its design seed, and gives integers for integer keys.
* Invalid designs and points are rejected, including huge grids (before they are built), non-finite values, and configs `Config::validate` refuses.
* An end-to-end sweep writes the expected rows and per-run manifests, with byte-identical files for `--jobs 1` and `--jobs 4`; each run reproduces from its manifest.
* CLI: `--jobs 0` and semantically impossible / no-op points fail before anything is written.

**Review fixes:**
* **`Config::validate`.** Floats must be finite; non-preference quantities non-negative; fractions in `[0, 1]`; every `*_min` ≤ `*_max`; world size and movement points at least 1; `memory.disruption_slots ≥ 1`. Called from TOML load, `set_config`, and every `--set` / sweep point.
* **Canonical grid values** and **integer LHS strata** that stay inside the stated range.
* **Size checks** before allocating a huge cartesian product; **overflow-safe** `points × seeds`.
* **No-op grid points** (debugging-only settings) refused; overlapping `--set` / `--param` keys warned.
* **`--set` recorded** in the design; design file written only after a successful sweep; label width grows past 999 points.
* A failing run under `--jobs` names the point and seed; `--set` errors exit 2 instead of panicking.

### A.21 Cross-PR review fixes (stack tip)

Findings from review agents over PRs 5–12, fixed on `feat/sweeps` (the tip contains every layer):

**Core (PRs 5–7):**
* Omniscient observation skips `Blocked` tiles (except the agent's own), so the baseline no longer plans Rest into flooded homes.
* `set_config` refuses a mid-run `sim.agent_kind` change, and refuses new scripted droughts/disruptions whose `start_day` is already past. Active disruptions stay in saved state when a scripted entry is removed.
* Lowering `memory.disruption_slots` trims each agent's pool (farthest first).
* A blocked movement step no longer counts as pathology "progress", so the stuck detector can fire.
* `memory.disruption_slots = 0` is rejected by `Config::validate`.

**CLI / Phase 2 (PRs 9–12):**
* Writing a manifest refuses to overwrite a different run (same label/seed/out); pick another `--label` or `-o`.
* Under `--jobs`, write/save failures `panic` instead of `process::exit`, so sibling seeds are not killed mid-run.
* Aggregate (and summary) treat `clark_evans` / `forage_radius` as undefined when the population is extinct or has fewer than two homes; `travel_per_food_run` is shared between summary and aggregate.
* Stale `emergence_summary.csv` / `emergence_aggregate.csv` are removed at the start of a suite.
* `branch` rejects a silent global `--config` (use `--config-b`).
* Snapshot `relative_path` is stored with `/`, and reproduce joins / splits on both separators, so a Windows-written manifest still finds its snapshot on macOS.
* Viewer: a no-op intervention (e.g. halving already-zero regrowth) refuses to branch.
