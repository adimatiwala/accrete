# Accrete — code map

The rationale for every design decision, and the changelog, live in
[`accrete.md` Appendix A](../accrete.md#appendix-a--implementation-changelog-branch-sbfull-emergent-sim-claudeware).
This file is a map of where things are.

## `accrete-core`

| Module | Contents |
|---|---|
| `config.rs` | Every tunable (per-day units) and `TickRates`, the single per-day to per-tick conversion. |
| `hash.rs`, `rng.rs`, `seed.rs` | FNV-1a state hashing, named ChaCha8 streams with portable float conversion, seed parsing. |
| `geom.rs`, `inventory.rs` | Positions, 8-neighbourhoods, octile distance, Bresenham lines; weight-limited inventories. |
| `world/` | `terrain` (elevation, moisture, biomes), `rivers` (priority-flood drainage), `resources` (fertility, placement, regrowth), `visibility` (169-bit line-of-sight masks), `movement` (MP costs), `spawn`, `validate`, `stats`. |
| `agents/agent.rs` | Agent state: needs, traits, skills, goal, route, home, foraging returns. |
| `agents/memory.rs` | Bounded memory of known terrain, resources, homes, paths (including their last observed wear), social contacts, frequent tiles and trading places. |
| `agents/pathfinding.rs` | A* over known tiles with believed costs, plus a straight-line travel estimate. |
| `agents/decision.rs` | Utility scoring of every option, with recorded terms; site scoring; reproduction motives; road and shelter value; material plans. |
| `simulation/engine.rs` | The tick pipeline: observe, plan homes, propose, resolve, apply, deaths, meetings, births, daily. Also the observers for the random and omniscient baselines (`sim.agent_kind`). |
| `simulation/social.rs` | Knowledge sharing, including tips about trading places, and barter with reciprocity. |
| `simulation/infrastructure.rs` | Road and shelter construction, and road upkeep. |
| `simulation/weather.rs` | Random and scripted droughts, and optional seasons. |
| `simulation/disruption.rs` | Scripted route disruptions (blocked or slowed tiles), started and ended in the daily phase; the authoritative state behind `World::disruption_at`. |
| `simulation/snapshot.rs` | Save and load the full state as versioned JSON, with a state-hash check on load; `set_config` for intervention branches. |
| `simulation/settlement.rs`, `emergence.rs`, `metrics.rs`, `events.rs` | Measurement only (agents never read these): settlements, clustering, depletion, specialisation, trade networks and locations, daily metrics, events and the resource ledger. |
| `scenario.rs` | Loader for the hand-built TOML worlds in `tests/scenarios/`. |
| `rl.rs`, `rl/household.toml` | Seeded household benchmark, bounded local action and observation contract, and evaluator score. |

## Tick pipeline

1. **Observe:** each agent reads its line-of-sight mask into memory.
2. **Plan homes:** each agent periodically re-evaluates where to live (staggered across agents).
3. **Propose:** every agent scores its options against a frozen world, chooses a goal, plans a route, and emits an intent.
4. **Resolve:** shared gathering is split proportionally; household actions and building are resolved in a random permutation; births are rolled.
5. **Apply:** movement in integer MP, eating, resting, needs, aging.
6. **Deaths:** agents whose health or health ceiling reached 0 are removed, and their goods go to their home or tile.
7. **Meetings:** among the survivors, sharing, then barter. The dead never trade or share.
8. **Newborns join;** they act from the next tick.
9. **Daily phase:** weather, regrowth, spoilage, decay, crowding, trails and roads, settlements, metrics.

The RL environment prepares a tick through observation and home planning,
exposes a local frame per controlled adult, then substitutes their proposals.
Routing, contested actions, needs, barter, and weather run through the regular
engine path. The `accrete-py` crate trains single-adult policies via Gymnasium
and can run a learned policy for every resident in the playback viewer. [Install and train](../crates/accrete-py/README.md)
