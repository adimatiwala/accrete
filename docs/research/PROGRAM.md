# Accrete research program: the conditions for emergent social structure

**Status:** proposal and roadmap (A.23). It governs the PRs listed in §8.
**Audience:** anyone extending Accrete's mechanics, measurement, or experiments.

---

## 1. Where we are, honestly

Accrete is now a mature, reproducible instrument:
- deterministic, bit-identical worlds;
- snapshots, branching and manifests;
- paired ablations with 95 % intervals;
- grid and Latin-hypercube sweeps;
- a run index.

The **science it produces is thin.** Across every run so far (A.9, A.17, A.18, A.20):

| Phenomenon | Observed | Why it is thin |
|---|---|---|
| Aggregation | Homes cluster (Clark-Evans R ≈ 0.45–0.56) into kin hamlets of 5–12 homes | The only reasons to live together are kinship and a small sociability term. Hamlets are families, not communities. |
| Population regulation | Smooth logistic growth; scarcity acts only through fertility | No starvation, no boom–bust, no migration waves. The preventive check is so effective that nothing is ever at stake. |
| Trade | About 1–4 barters per day for 500 people | Every household needs the same thing (food) and can get it alone, so there are no gains from trade. |
| Division of labour | Specialisation index ≈ 0.001 | Nothing rewards being a specialist; there is no production chain. |
| Roads | 0–1 road tiles in 300 days | Trips are too short to justify paving. |
| Culture | None | Traits are inherited with noise; nothing is learned from others. |

The model has the *machinery* for complex society: memory, households, storage, barter, reciprocity, trails, roads. But the *incentive landscape* never makes cooperation, specialisation or aggregation pay. **The research problem is not to add more machinery. It is to find the individual-level conditions under which collective structure becomes the agents' own best response, and to measure that transition rigorously.**

## 2. Central question and principles

> **Q.** Which individual-level motives and environmental conditions are necessary and sufficient for durable aggregation, exchange networks, division of labour and cultural regions to emerge from locally informed agents, and how sharp are the transitions between regimes?

Principles, all binding on every PR:

1. **Motives, never outcomes** (spec §1; CLAUDE.md rule 5). Every new rule is one agent's preference, perception or ability using local knowledge. Nothing reads settlements, metrics or population statistics.
2. **Mechanism, then measurement, then claim.** A mechanism PR ships with the metric that would detect its effect *and* an ablation that turns it off. Claims require multi-seed paired evidence (§6).
3. **Report nulls.** "X does not emerge under Y" is a finding. Thresholds are never tuned to make a pattern appear.
4. **Phase diagrams over anecdotes.** The unit of evidence is a response surface over parameters, not a single striking run.

## 3. Hypotheses

Each hypothesis names its mechanism, its prediction, the metric (§4) that tests it, and the experiment (§5) that does so.

| ID | Hypothesis | Mechanism | Prediction (falsifiable) | Metric | Experiment |
|---|---|---|---|---|---|
| **H1** | *Surplus threshold.* Aggregation needs local surplus. | Existing: site scoring, kin ties, land perception | The clustering index R falls with food productivity, with a **sharp transition** (logistic slope > 5 in log-regrowth) rather than a linear trend; below the transition, hamlets = 0 on every seed | R, hamlets, settlement size distribution | E1: a 2D phase diagram (regrowth × sociability) |
| **H2** | *Comparative advantage drives specialisation.* | **New:** tools (PR 5) made from wood + stone, which sit in different biomes; crafting skill improves with practice | With tools: specialisation index > 0.1, a stable minority of crafters (< 25 % of adults who craft ≥ 50 % of tools), trades per capita up ≥ 5×. Ablating trade drives specialisation back toward the no-tools level. | specialisation, crafter share, trade volume, trade-network degree Gini | E2: tools × trade 2×2 ablation |
| **H3** | *Information centres.* Shared knowledge makes living together pay. | Existing knowledge sharing | Turning sharing off **raises** R (less clustering) and lengthens foraging trips | R, forage radius, tips | E3: sharing ablation at full and ×0.4 food |
| **H4** | *Public goods need both travel and household concern.* | Existing roads; longer trips come from tools and scarcity | Road tiles scale with mean trip length; the selfish ablation (household weight 0) cuts road tiles by ≥ 50 % *in the regime where roads exist* | road tiles, network efficiency, trip length | E4: roads × household weight × regrowth |
| **H5** | *Cultural regions.* Payoff-biased imitation creates spatially coherent norms *beyond what kinship alone produces*. | **New:** social learning (PR 6) | **Revised after A.24:** trait Moran's I is already 0.37–0.59 with no learning, from kin budding plus inheritance. The prediction is that learning raises Moran's I above this kinship baseline (paired difference > 0, 95 % CI) and shifts trait means toward locally adaptive values (caution higher where droughts hit) | Moran's I (paired), trait means by region | E5: learning ablation, with and without droughts |
| **H6** | *Resilience through institutions.* Storage and trade buffer shocks. | Existing chests and barter; new tools | After a scripted drought, recovery time (§4) is shorter with trade and storage on; the no-trade arm has a larger population trough | recovery time, trough depth | E6: drought branch × trade ablation |
| **H7** | *Rank-size structure.* Settlement sizes become heavy-tailed as aggregation strengthens. | Emergent | The rank-size exponent ζ moves toward ≈ 1 (Zipf) in the high-surplus regime; the largest-settlement share grows | ζ, top share | E1 (same runs), plus 900-day runs |
| **H8** | *Scripted motives are near-optimal for individuals.* | RL agent (branch `sb/rl-env`) | A learned policy exceeds the heuristic by < 10 % in return; the actions where it differs reveal mis-weighted motives | return gap, per-goal action shares | E8: learned vs heuristic on held-out seeds |

## 4. Measurement: the Observatory (PR 2)

All of this is measurement only: computed from state, never read by agents, never written into anything an agent reads. Day-level metrics go into `DailyMetrics`; run-level metrics are computed in the CLI from metrics and events.

| Metric | Definition | Notes |
|---|---|---|
| **Wealth Gini** | Gini of per-household wealth: food, wood and stone held by residents plus their chests (weighted by `InventoryConfig` weights), over occupied homes | Fixed-order sum, sort with `total_cmp` |
| **Settlement count by size, largest share** | Sizes (residents) of the clusters from `settlement::clusters` at R = 3, *including* candidates; largest cluster's share of the homed population | Feeds ζ |
| **Rank-size exponent ζ** | OLS slope of log(size) on log(rank) over clusters of at least 2 homes | Computed in the CLI (logarithms are banned in core); undefined below 3 clusters |
| **Migration rate** | Home moves per person-year, and mean move distance (Chebyshev tiles) | Resolves A.18's open question |
| **Trade network** | Over a rolling 30-day window: distinct trading pairs, mean degree, degree Gini, share of trades between households (vs within), repeat-partner share | Needs per-trade records: a bounded `TradeLog` in the simulation, added to snapshot and hash |
| **Crafter share** (with PR 5) | Share of adults whose crafting practice is their top skill | |
| **Moran's I** (traits) | For each trait: global Moran's I over occupied homes (home trait = residents' mean), with weights 1 / (1 + distance) inside 8 tiles | Permutation p-values in the CLI |
| **Recovery time** | Days after a shock's end until the 7-day mean population first returns to within 5 % of its pre-shock 7-day mean | Computed in the CLI from `branch` compare files |

## 5. Experiments (protocols)

All experiments run on **seeds 1–10** (Phase 2), paired where there is a control, reporting the mean paired difference with a 95 % t-interval (`emergence_aggregate.csv`). Every run has a manifest (`accrete reproduce`), and figures are generated from CSVs.

* **E1, phase diagram.**
  * Run: `accrete sweep --param resources.food_regrowth=0.03,0.045,0.06,0.08,0.1,0.125,0.15,0.2 --param decision.social_weight=0,0.35,0.7 --seed 1-10 --days 300`.
  * Fit a logistic in log-regrowth to hamlets > 0 and to R. H1 holds if the transition width is < ½ octave.
* **E2, tools × trade.**
  * Arms: `tools.enabled ∈ {false, true}` × `social.trade_enabled ∈ {false, true}`, seeds 1–10, 300 days.
  * Report specialisation, crafter share, trades per capita, degree Gini and population.
* **E3, sharing ablation.** The `sharing` experiment in the suite, at regrowth 0.15 and 0.06.
* **E4, roads.** A Latin hypercube (N = 40) over `resources.food_regrowth ∈ [0.04, 0.2]`, `infrastructure.household_weight ∈ [0, 1]` and `infrastructure.road_material_ticks ∈ [2, 10]`, followed by Sobol indices (PR 4) on road tiles and trip length.
* **E5, culture.** `culture.enabled` on or off × droughts on or off, seeds 1–10, 600 days. Moran's I with a permutation test (999 permutations).
* **E6, resilience.**
  * Start from a day-150 snapshot (seeds 1–10).
  * Arms: `branch --drought 150:30:<centre of largest settlement>:20:0.7` × `trade_enabled` on/off (a 2×2 design).
  * Report recovery time and trough depth.
* **E8, RL.** Re-land `sb/rl-env` on main, then compare learned and heuristic policies on held-out seeds 1–20.

**Global sensitivity (PR 4).**
- Saltelli design with N = 256 base samples over k ≈ 8 parameters, giving N(k + 2) runs × 3 seeds.
- First-order and total Sobol indices (Jansen / Saltelli 2010 estimators) with bootstrap 95 % intervals, for every outcome in `OUTCOMES`.
- This answers **"which knobs matter for which emergent structure"**: the parameter → behaviour function asked for in A.10.

## 6. Statistical standards

* **Unit of replication:** the world seed. Paired designs share seeds.
* **Uncertainty:** report 95 % t-intervals (n < 30), bootstrap intervals for Sobol indices, and permutation tests for Moran's I.
* **Multiple comparisons:** each experiment names its *primary* outcome up front (the table in §3); everything else is exploratory and labelled as such.
* **Equilibrium:** report whether a run is at steady state (population slope over the last 60 days within ±2 %), and use longer runs where it is not.
* **Reproducibility:** every reported number has a manifest; `accrete reproduce` must pass.

## 7. Mechanisms to add (designs)

### 7.1 Tools and crafting (PR 5): a production chain with comparative advantage

* **Good:** `tools`, a durable crafted good with weight 2. A tool in inventory multiplies the gathering rate for food, wood and stone by `tools.gather_bonus` (default 1.5). Each gathering tick wears one unit of durability.
* **Recipe:** `Craft` at home turns `tools.wood` (2) + `tools.stone` (1) into one tool over `tools.craft_ticks` (4). Crafting has a **skill** with aptitude and practice like the others; output per craft = 1 × skill, so practice makes crafting cheaper per tool.
* **Spatial separation already exists:** stone only in hills and mountains, wood in forests, food near rivers. Households near hills have a comparative advantage in tools.
* **Motives:**
  * *"Gathering is faster with a tool"* raises the value of holding one (an expected-productivity term).
  * The existing barter values tools by need, so tool-rich crafters trade them for food.
  * The existing materials plan covers inputs.
* **Ledger:** crafting is recorded as construction (inputs) plus a new `crafted` line (output); wear is recorded as destruction. Everything is conserved and checked by `--check`.
* **Ablation:** `tools.enabled = false` removes the option and the good.

### 7.2 Cultural transmission (PR 6): payoff-biased social learning

* When two agents share knowledge (same tile, already a "meeting"), each compares its **own experienced foraging return** with the partner's, *as the partner tells it* (local, honest information exchanged in conversation).
* If the partner does better, the listener moves each trait a fraction `culture.rate` (default 0.05) of the way toward the partner's, scaled by the payoff difference and the listener's sociability. This is the standard payoff-biased copying model (Boyd & Richerson).
* Inheritance stays genetic plus noise. Learning is lifelong and local, so **regions** can form.
* **Ablation:** `culture.enabled = false` (the default during PR review; the default is decided in A.27 from E5 results).

### 7.3 Not now (and why)

| Idea | Status |
|---|---|
| Agriculture | The strongest real driver of sedentism, but it changes the economy's foundation. It comes after H1–H6 are mapped, so its effect can be measured against a known baseline. |
| Conflict and territory | Excluded by the spec's scope (§2). |
| Multi-agent RL | Comes after E8 confirms the single-agent environment is sound. |

## 8. Roadmap (PR stack)

| PR | Branch | Appendix | Contents | Changes results? |
|---|---|---|---|---|
| 1 | `research/program` | A.23 | This document | No |
| 2 | `research/observatory` | A.24 | §4 metrics; new `OUTCOMES`; trade log | **No.** The trade log is measurement-only and outside the state hash, so every golden *state* hash is unchanged; only the snapshot-bytes golden changes (format 4) |
| 3 | `research/viewer-story` | A.25 | Viewer legibility: event ticker, roles, family and trade links, named settlements, biography mode, legend | No (viewer only) |
| 4 | `research/sensitivity` | A.26 | `accrete sensitivity`: Saltelli design, Sobol indices, bootstrap intervals | No (CLI only) |
| 5 | `research/tools` | A.27 | §7.1 tools and crafting, plus the E2 experiment in the suite | Yes, behind `tools.enabled` |
| 6 | `research/culture` | A.28 | §7.2 social learning, plus the E5 experiment | Yes, behind `culture.enabled` |

Each PR follows CLAUDE.md: an Appendix A entry, a CLAUDE.md update where commands or rules change, regenerated `default.toml`, golden hashes blessed only with an explanation, and fmt, clippy and both test builds passing.
