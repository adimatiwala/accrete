# ACCRETE

**An emergent civilization simulator**  
*A world that builds itself, one decision at a time.*

**Status:** Implementation blueprint · **Target:** four-week laptop-scale MVP · **Stack:** Rust + macroquad

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

## 2. Scope and constraints

| Dimension | MVP | Stretch goal |
|---|---|---|
| World | 128×128 tiles | 256×256 with chunking |
| Initial agents | 100 | 500–1,000 |
| Resources | Food, wood, stone | Specialized goods and production chains |
| Agent intelligence | Interpretable utility scoring, local perception, memory | Learning, social knowledge exchange |
| World model | 2D terrain, biomes, rivers | Seasons, more sophisticated hydrology |
| Social systems | Barter, homes, emergent clusters | Institutions and specialization |
| Transportation | Traffic-worn trails, improved roads | Capacity, congestion, collective investment |
| Experiments | Snapshots, drought, route disruption | Parameter sweeps, uncertainty estimates |
| Execution | Windows laptop, CPU, offline | Parallel batch execution |

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
├── experiments/
└── results/                  # gitignored generated outputs
```

**Libraries:** `macroquad` (2D viewer), `noise` (terrain), `rand_chacha` (seeded RNG), `serde`/`serde_json` (serialization), `clap` (CLI), and optionally `toml` (configuration). Pin dependency versions in `Cargo.lock`.

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
    food: f32,
    wood: f32,
    stone: f32,
    traffic: f32,
}

struct Agent {
    id: AgentId,
    position: Position,
    hunger: f32,
    energy: f32,
    health: f32,
    inventory: Inventory,
    memory: AgentMemory,
    traits: Personality,
    goal: Goal,
    home: Option<Position>,
}

struct Simulation {
    tick: u64,
    world: World,
    agents: Vec<Agent>,
    // Separate deterministic RNG streams and configuration.
}
```

Suggested initial actions: `Move`, `Explore`, `GatherFood`, `GatherWood`, `GatherStone`, `Eat`, `Rest`. Add `EstablishHome`, `Trade`, and `ImproveTrail` only after survival mechanics work.

### Decision policy

Use utility-based decisions, with state-dependent weights:

\[
U(a,s) = \sum_i w_i(s) V_i(a,s) - C(a,s)
\]

Examples: hunger raises the value of eating and gathering food; low energy raises the value of resting; an unfamiliar nearby tile can raise the value of exploration; distance and terrain increase travel cost. Include a small, seeded personality variation to prevent identical agents from making identical decisions.

For debugging, record the chosen action, top competing actions, and the major utility terms. Avoid unbounded action oscillation by introducing action commitment or hysteresis if needed.

### Resource conservation

Every resource change should correspond to a defined event: regeneration, gathering, consumption, transfer, construction, or destruction. Write invariant checks so an agent cannot gather more than a tile contains or trade more than its inventory holds.

## 5. Week 1 — Procedural world generation

**Goal:** Create a repeatable, visually interesting environment with meaningful geographic constraints.

### Day 1: Workspace and world representation

- Create a Cargo workspace with `accrete-core`, `accrete-viewer`, and `accrete-cli`.
- Define grid coordinates, tiles, biomes, and simulation configuration.
- Implement seed parsing and deterministic RNG initialization.
- Add a minimal `generate_world(seed, config)` API.
- Verify `cargo build --workspace` and `cargo test --workspace`.

### Day 2: Terrain and biomes

- Generate elevation using seeded multi-octave noise.
- Generate a separate moisture field.
- Classify ocean, plains, forest, hills, and mountains using thresholds.
- Ensure neighboring cells are reasonably continuous; prevent isolated artifacts where practical.
- Add a color-coded biome renderer.

### Day 3: Rivers and resources

- Build a simplified downhill river routing algorithm, with explicit treatment of local minima and map boundaries.
- Place food according to fertility/moisture, wood in forests, and stone in rocky areas.
- Distinguish renewable resources (food, some wood) from finite deposits (stone).
- Define passability and travel cost for each biome.

### Day 4: Visualization

- Render a 128×128 tile grid using macroquad.
- Implement pan, zoom, and tile inspection.
- Add overlay modes: biome, elevation, moisture, food, wood, stone, and passability.
- Add seed input and regenerate control; show seed in the interface.

### Day 5: Starting positions and validation

- Place 100 initial agents in viable locations; do **not** prebuild cities.
- Test that spawn positions are traversable and have plausible nearby resources.
- Add deterministic snapshot tests or state hashing for generation.
- Benchmark generation time and viewer responsiveness.

**Week 1 deliverable:** A navigable, seeded world map with meaningful terrain and resource distributions.

**Acceptance criteria:** Same seed/config gives the same state; different seeds differ; all tile properties are in valid ranges; rivers obey routing rules; resources obey biome constraints; generation runs headlessly.

## 6. Week 2 — Individual agents and survival

**Goal:** Make 100 agents survive or fail through their own local decisions.

### Day 1: Agent state and fixed ticks

- Implement needs: hunger, energy, health.
- Implement inventory, movement speed, and basic health decline.
- Add fixed timestep engine and pause/resume/step controls.
- Ensure agents update consistently regardless of rendering FPS.

### Day 2: Local observation and memory

- Give each agent a limited observation radius.
- Record visited tiles and known food locations, including freshness timestamps.
- Distinguish actual world state from an agent's belief about it.
- Prevent the decision system from reading undiscovered resources.

### Day 3: Utility-driven behavior

- Implement explore, move, gather, eat, and rest.
- Add utility scoring with tunable weights and simple agent personality traits.
- Implement movement toward remembered resources and fallback exploration.
- Prevent rapid goal switching from creating movement loops.

### Day 4: Resource conflicts and migration

- Resolve simultaneous claims on the same tile without duplicating resources.
- Add food regeneration, consumption, and death from unmet needs.
- Allow agents to abandon depleted regions and search elsewhere.
- Add a basic home preference or remembered safe location; full home construction is Week 3.

### Day 5: Agent inspection and tests

- Clicking an agent reveals needs, inventory, remembered places, goal, and action score breakdown.
- Record a rolling event log of actions and resource changes.
- Test resource conservation, deterministic updates, death, and bounded agent properties.
- Compare a plentiful-food world against a scarce-food world.

**Week 2 deliverable:** 100 locally informed agents who explore, gather, consume resources, and migrate in response to scarcity.

**Acceptance criteria:** Agents make valid independent decisions; resource constraints are respected; inspection explains choices; same seed/config reproduces the same state at a given tick; survival differs under controlled scarcity conditions.

## 7. Week 3 — Emergent settlements, trade, and infrastructure

**Goal:** Make social organization possible without scheduling settlement formation.

### Day 1: Homes and persistent residence

- Allow agents to choose a home location based on safety, resource access, and travel cost.
- Introduce simple shelters and resource storage if necessary.
- Track how long agents remain associated with a location.
- Detect residential clusters for analytics using distance, occupancy, and persistence thresholds.
- Ensure settlement detection **does not** itself grant benefits or trigger city-building behavior.

### Day 2: Exchange

- Implement food/wood/stone inventories with explicit scarcity and utility.
- Let agents evaluate voluntary barter with nearby agents.
- Only execute exchanges that satisfy both parties' constraints and acceptance rules.
- Log transactions, traded quantities, and distances.

### Day 3: Movement and trails

- Track the number of crossings per tile.
- Apply traffic decay: `traffic_next = lambda * traffic + crossings`.
- Reduce travel costs modestly on heavily traveled, passable trails.
- Use A* or Dijkstra on terrain/trail costs for agent routes.
- Guard against pathological feedback where all traffic collapses onto one route for artificial reasons.

### Day 4: Improved infrastructure

- Allow agents to invest wood/stone/labor into improving repeatedly used paths when expected personal benefit exceeds cost.
- Make constructed roads more durable than naturally worn trails.
- Show traffic heatmaps and improved-road overlays.
- Keep construction local and individually motivated; avoid central planning logic.

### Day 5: Emergence experiments

- Compare runs with and without barter.
- Compare runs with and without trail travel discounts.
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

- Implement drought as reduced food regeneration in a selected region and time window.
- Implement route failure as temporarily impassable or expensive path tiles.
- Apply shocks through ordinary world mechanics, not direct agent behavior overrides.
- Record the timing and parameters of interventions.

### Day 3: Branching experiments

- Save a world at day 100.
- Branch into A: control; B: drought from day 100 onward.
- Continue both through day 300 from the same initial snapshot.
- Keep random streams and update ordering controlled to make comparisons interpretable.

### Day 4: Metrics and headless runner

- Export CSV/JSON metrics: population, health, hunger, settlement count, settlement persistence, trade volume, average travel cost, inventory distribution, and recovery time.
- Add CLI commands for seeded runs, snapshot loading, scenario selection, and result export.
- Run multiple seeds and compare averages and variability; do not rely on a single attractive example.

### Day 5: Final demo and hardening

- Build an interactive viewer with play/pause, tick step, speed control, seed selection, map overlays, and agent inspection.
- Add a scenario selector and control-versus-intervention plots or exports.
- Test replay, snapshot compatibility, performance, and long-run stability.
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
| Disaster | No drought | Regional drought at day 100 | Recovery time, population, relocation |
| Knowledge | Agents remember resources | Memory disabled/limited | Search effort, starvation, travel distance |

Use multiple seeds for each experiment, hold parameters fixed within a comparison, and record all intervention configurations. Report uncertainty and failed runs rather than selecting only visually appealing results.

### What counts as a settlement?

For measurement purposes only: a spatial cluster of at least *N* long-term homes within radius *R*, persisting for *P* ticks. Expose N/R/P in experiment configuration and test sensitivity to those thresholds. Avoid defining settlement success so loosely that random crowding counts as civilization.

### What counts as a road?

Distinguish (1) raw traffic, (2) an emergent worn trail above a traffic threshold, and (3) a deliberately improved path with construction cost. These should have different durability and movement effects.

## 10. Validation and engineering quality

### Unit tests

- World generation is deterministic and tile values are valid.
- River segments obey routing and boundary constraints.
- Agents cannot spend, consume, gather, or trade unavailable resources.
- Health, hunger, and energy remain within defined bounds.
- Pathfinding respects impassable tiles and route costs.
- Settlement detection produces stable results for known synthetic layouts.

### Integration tests

- Running the same seed/config for 1,000 ticks yields the same state hash.
- Save at tick 500, resume to tick 1,000, and compare against uninterrupted execution.
- Renderer on/off does not affect simulation state.
- Scenario branching starts from identical state.
- A drought modifies regeneration but never directly commands agents to migrate.

### Performance tests

- Benchmark 100, 500, and 1,000 agents over a fixed number of ticks.
- Track ticks per second, memory use, and pathfinding work.
- Avoid per-agent full-map searches every tick; cache paths or restrict search to known regions.
- Profile before optimizing. A 128×128 map and 100 agents should be an approachable laptop target, but exact performance depends on implementation and hardware.

### Known risks and mitigations

| Risk | Mitigation |
|---|---|
| Agents starve immediately | Tune starting conditions and verify resource accessibility |
| Agents oscillate between goals | Goal commitment, hysteresis, cooldowns |
| Agents appear omniscient | Strict observation/memory interface |
| Settlements never form | Diagnose incentives; do not force settlements |
| Trails produce runaway routing | Cap trail benefits, use decay and realistic terrain costs |
| Replays diverge | Stable iteration order, isolated seeded RNG, snapshot RNG state |
| Scope expands too quickly | Lock each week's acceptance tests before adding systems |

## 11. Visualization specification

Minimum UI:

- Main map: terrain, rivers, agents, homes, trails, and improved roads.
- Top bar: seed, tick/day, agent count, play/pause, step, speed, regenerate.
- Map layers: biome, elevation, food, wood, stone, traffic, settlement clusters.
- Inspector: click tile or agent; show state, local knowledge, current goal, and decision explanation.
- Metrics panel: population, average hunger, trade activity, settlement count, travel cost.
- Experiment controls: load snapshot, select drought or route disruption, branch and compare.

Prioritize readability and responsiveness over elaborate graphics. Colored tiles and moving dots are enough for a convincing first version.

## 12. Definition of done

The MVP is complete when a user can:

1. Start Accrete locally without cloud dependencies.
2. Generate a repeatable world from a seed.
3. Watch at least 100 independently acting agents survive, explore, and interact.
4. Inspect and understand individual decisions.
5. Observe and measure whether stable clusters and transport patterns emerge.
6. Save a world, branch it into control and intervention scenarios, and replay both.
7. Export metrics and reproduce results using the CLI.

**A valid outcome is discovering that an expected behavior does not emerge.** The project's value is in creating a trustworthy environment to investigate why.

## 13. First VS Code / Codex task

Paste the following into Codex once the repository is open:

> Build **Accrete**, a local-first agent-based emergent civilization simulator in Rust. Create a Cargo workspace with `accrete-core` (pure simulation logic), `accrete-viewer` (macroquad visualization), and `accrete-cli` (headless execution). Implement **Week 1 only**: deterministic 128×128 procedural terrain generation with elevation, moisture, biomes, rivers, resource distribution, and viable starting positions for 100 agents. Add an interactive 2D renderer with pan, zoom, seed input, regenerate, and tile inspection. Keep simulation independent of rendering. Add reproducibility and validity tests. Target a Windows laptop with no cloud or ML dependencies. Do not implement economy, trading, roads, or agent behavior yet. Compile and test incrementally.

### Recommended development loop

1. Ask Codex to implement **one bounded milestone**, not the entire roadmap.
2. Run `cargo fmt --all`, `cargo clippy --workspace --all-targets`, and `cargo test --workspace`.
3. Launch the viewer and inspect the result visually.
4. Record unexpected behavior as an issue or test case.
5. Commit the working milestone before adding the next feature.

---

**Long-term direction:** Explore how limited information, social knowledge exchange, and individual incentives influence the resilience and organization of emergent civilizations. Keep the agents simple enough that their collective behavior remains explainable.
