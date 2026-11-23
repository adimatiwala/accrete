# Do trading places and shared roads produce larger settlements?

Accrete previously found that tools expanded barter without producing specialists. That leaves two different explanations. People may lack ways to find a good trading partner, or they may still need to spend most of their time securing food even when trade is easy. This study gives agents a way to return to a remembered trading place and to value roads used by other people. We test whether those changes increase **exchange**, concentrate **where exchange happens**, and enlarge **where people live**. These are three distinct outcomes.

With both changes on, trades per person rose by 37% while the number of trades per world-day fell by 21%. Trading places concentrated actual exchanges. The combined treatment produced smaller groups in all three matched worlds. When food regrowth was 50% higher, shared roads increased village persistence in all three worlds through day 400. These results separate the effects of market access and transport from overall economic scale and settlement growth.

## Treatments

| Arm | Remember and seek trading places | Value other travellers' use of a trail |
| --- | --- | --- |
| Control | Off | Off |
| Markets | On | Off |
| Shared roads | Off | On |
| Both | On | On |

Market memory gains one point at the proposer's location for both participants in a completed barter. Agents can learn about a contact's best location through ordinary sharing and remember up to eight locations. Scores decay by 3% per day. An adult with goods to exchange or a reason to want a tool can plan a trip to a remembered location. The utility accounts for the trip and an age penalty once there. Home choice gains a separate preference for access to remembered places. Actual exchange still requires a contact and a mutually acceptable offer.

The shared-road arm gives a sociable builder some value for time saved by other travellers. It uses wear on the current tile or last-seen wear elsewhere, subtracts the builder's own estimated crossings, and applies the existing cost of labour and materials. Shared benefit is in addition to the value the builder puts on trips by their own household. Agents use the same construction and upkeep rules across all arms. Villages, towers, and market-stall markers are downstream measurements or viewer displays.

## Protocol and outcomes

- The main study crosses the two switches at their defaults when enabled. Tools are on in all arms. Each seed (1, 2, and 3) starts from the same generated world for all four arms and runs for 1,000 days. The world seed is the unit of replication.
- The main comparison uses days 901–1,000 to reduce dependence on the final day. Report each arm's mean and the signed contrasts within seeds. The joint interaction is `both − markets − roads + control`. Show the observed seed range beside each contrast. Three fixed seeds support a descriptive result and expose heterogeneity.
- Report trades per world-day and per 1,000 person-days using end-of-day populations. Spatial concentration is the probability that two distinct trades in the trailing 30 days used the same proposer tile. The raw metric is HHI, the sum of squared tile shares. The analysis corrects for trade count as `(n × HHI − 1) / (n − 1)` so a small number of trades does not mechanically look more concentrated. This is a tile-scale measure. Nearby trade locations on different tiles count separately.
- Measure road supply as maintained road tiles. Measure residential aggregation as residents in the largest tracked group, including candidates, and count days with any village or town over the full run. The existing classifier requires at least 50 residents and 15 homes for a village, 150 and 50 for a town, and 60 days of group persistence for either. It applies the same criteria in all arms.
- Also report population, the share of homed residents in the largest home cluster, practice-based specialisation, and the average daily share of agent-hours assigned to the market goal. The latter combines travel and waiting. It can show a time allocation shift but cannot isolate the cost of either activity. Inspect whether any arm reaches the simulation's population cap.
- An exploratory ecological check runs the same four arms and seeds to day 400 with 50% faster food regrowth. Compare the same horizon and window in the main design. This probes whether supply changes the result. It does not model cultivation or identify food availability as the sole mechanism.

## Results at default food supply

Means across three worlds, days 901–1,000 except for village-days.

| Outcome | Control | Markets | Shared roads | Both |
| --- | ---: | ---: | ---: | ---: |
| Trades per 1,000 person-days | 73.4 | 79.5 | 94.1 | 100.4 |
| Trades per world-day | 61.3 | 50.5 | 84.5 | 48.3 |
| Recent trade pairs on the same tile | 0.27% | 1.29% | 0.18% | 1.23% |
| Road tiles | 5.1 | 9.1 | 33.6 | 24.0 |
| Specialisation | 0.022 | 0.018 | 0.024 | 0.019 |
| Residents in largest tracked group | 40.0 | 29.2 | 35.4 | 30.5 |
| Homed population in largest cluster | 4.7% | 4.6% | 4.0% | 6.3% |
| Population | 830.6 | 637.6 | 896.9 | 479.3 |
| Agent-hours on market goals | 0% | 7.1% | 0% | 12.6% |
| Days with any village, full run | 70 | 0 | 19 | 0 |

Markets alone increased the same-tile trade-pair probability by 0.90–1.18 percentage points across seeds. Shared roads alone added 18.4–43.7 road tiles and 8.1–32.3 trades per 1,000 person-days. In the combined arm relative to control, every seed had more exchanges per person (+21.9 to +30.1 per 1,000 person-days), fewer exchanges per world-day (−17.6 to −4.3), and fewer residents in the largest tracked group (−12.0 to −4.5). The largest cluster captured a greater share of the smaller population. Neither market arm sustained a village in any seed. Seed 1 accounted for all village-days in the main study, with 210 in control and 58 with shared roads. No arm produced a town or reached the population cap.

These results challenge a simple version of the earlier market-extent hypothesis. Repeat encounters at remembered places concentrated commerce while the practice-based division of labour stayed close to control. The combined arm spent 12.6% of agent-hours pursuing market goals. Compared with control, its rest-goal share fell from 83.1% to 68.9% and its food-gathering-goal share rose from 8.7% to 10.3%. Those allocation shifts do not tell us which change caused the population difference. The effect on births per adult-day varied across seeds, and each combined world had only one to four deprivation deaths over 1,000 days.

## Higher-food comparison

At default food regrowth, there were zero village-days in every arm through day 400. With faster regrowth, controls had villages for 77, 33, and 27 days. Shared roads alone raised those counts to 172, 78, and 36 days on the same seeds. This pattern did not persist in the longer default-food study, where shared roads reduced village-days on seed 1 and the other two seeds never formed a village.

Under higher food regrowth, the joint treatment raised trades per 1,000 person-days from 60.7 to 143.6, with a positive interaction of +13.5 beyond the sum of the separate treatments (seed range +7.1 to +26.2). Largest tracked groups still fell from 51.9 to 37.0 residents on average, and only one combined world formed a village, for 34 days. One higher-food control reached the population cap. This is evidence of supply-dependent outcomes in the model. The cap and the three-world sample limit the precision and generality of the estimate.

## What would distinguish the explanations?

The market arm bundles three changes. Place memory, extra travel or waiting, and altered housing preferences can all affect outcomes. A direct follow-up would hold memory fixed and cross the incentive to visit with the incentive to live nearby, then measure completed trades per trip and time spent waiting. Another would compare remembered trading partners with remembered places at the same search cost. To test the ecological mechanism, hold total potential food production fixed and compare distributed food gains with gains concentrated around plots maintained by households. That distinguishes greater carrying capacity from denser local supply. Use fresh world seeds, report the separate effect on household location and population, and keep the settlement classifier fixed.

## Reproduce

Run from the repository root. The first command produces the main design, and the second produces the higher-food comparison. Both retain daily CSVs, a run manifest per seed and arm, and the sweep design. The analysis needs Python's standard library.

```sh
cargo run --release -p accrete-cli -- sweep --set tools.enabled=true \
  --param markets.enabled=false,true --param infrastructure.public_weight=0,1 \
  --seed 1-3 --days 1000 --jobs 6 -o results/cities_a33
cargo run --release -p accrete-cli -- sweep --set tools.enabled=true \
  --set resources.food_regrowth=0.225 \
  --param markets.enabled=false,true --param infrastructure.public_weight=0,1 \
  --seed 1-3 --days 400 --jobs 6 -o results/cities_a33_high_food
python docs/research/cities_analysis.py results/cities_a33
python docs/research/cities_analysis.py results/cities_a33 --end-day 400
python docs/research/cities_analysis.py results/cities_a33_high_food --end-day 400
```

Inspect the same mechanisms in the 3D viewer, for example on seed 1 at day 300. Grey pavement represents road tiles, stalls mark frequently remembered trading places, and towers mark confirmed villages or towns.

```sh
cargo run --release -p accrete-viewer -- --seed 1 --day 300 --3d --config configs/cities.toml
```
