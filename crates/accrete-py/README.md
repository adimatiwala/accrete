# Accrete household RL environment

Train one adult to keep a three-person household fed in an Accrete world with a nearby forage patch, a farther market, and scripted neighbours. The policy can collect food and wood, build a food reserve in the household chest, or travel to a market it remembers. Actual movement, storage, competition, and barter go through the same Rust engine as ordinary Accrete runs.

## Install and run

From the repository root with Rust 1.86 and Python 3.11 or later recommended. On Windows PowerShell:

```powershell
python -m venv .venv
.venv/Scripts/python.exe -m pip install -e "./crates/accrete-py[train,test]"
.venv/Scripts/python.exe -m pytest crates/accrete-py/tests
```

On macOS or Linux, use `.venv/bin/python` for the last two commands. To run the environment without training dependencies, install `./crates/accrete-py` without extras. Maturin builds the native extension during installation.

```python
import gymnasium as gym
import accrete_rl

with gym.make(accrete_rl.ENV_ID, weather="drought") as env:
    obs, info = env.reset(seed=1)
    while True:
        action = env.unwrapped.reserve_action()
        obs, reward, terminated, truncated, info = env.step(action)
        if terminated or truncated:
            break
    print(env.unwrapped.episode_stats())
```

## Train and compare

Use your virtual environment's Python in the commands below.

```sh
python -m accrete_rl.evaluate --episodes 32 --weather drought --out results/rl/baselines.csv
python -m accrete_rl.train --steps 1000000 --workers 4 --out results/rl/first
python -m accrete_rl.evaluate --episodes 32 --weather drought --model results/rl/first/final.zip --out results/rl/first/drought-eval.csv
python -m accrete_rl.train --resume results/rl/first/final.zip --steps 1000000 --seed 43 --workers 4 --out results/rl/second
```

One million steps fits about 1,980 full 21-day episodes. Independent worker processes reset and gather transitions in parallel. Maskable PPO updates the policy after each batch of rollouts. Training writes periodic checkpoints, a final model, settings and per-worker episode returns. A resumed job restores the saved policy, optimizer and step counter, then starts fresh episodes. Set a different `--seed` to collect a fresh training sequence.

Evaluation compares masked random actions, a rule that maintains reserves and forages, and an optional learned checkpoint on the same world seeds. Learned evaluation samples from the policy with a fixed per-episode seed, as during training. Use `--greedy` to test taking the most likely action. The evaluator writes one CSV row per policy and seed with return, episode length, initial household survivors, deprivation hours, focal trades and travel. Evaluate on both steady and drought episodes to see where a gain comes from. The trainer uses world seeds at or above `2^40`, and the default evaluation uses 1–32. Held-out seeds vary initial stock, which side of the map the household is on, and drought timing, length and severity within this scenario family.

## First training run

One four-worker CPU run trained on mixed weather with optimiser seed 42. These are results on the same 32 held-out drought worlds. Return and focal trades are means per episode.

| Policy | Focal survival | Return | Trades |
| --- | ---: | ---: | ---: |
| Random with action masks | 2/32 | 251.30 | 0.9 |
| Sampled PPO after 32,768 steps | 21/32 | 318.24 | 1.6 |
| Sampled PPO after 131,072 steps | 31/32 | 429.18 | 5.4 |
| Reserve-and-forage rule | 32/32 | 437.38 | 1.5 |

Greedy action selection from the final PPO checkpoint survived 0/32. Use the sampled results to assess the policy that PPO actually trained. This pilot uses one optimiser seed and one family of worlds. The reserve rule still had the higher return and fewer deprivation hours.

On the same machine, a separate baseline batch completed 2,048 episodes and 891,646 simulation ticks in 47 seconds. The four-worker PPO pilot took 82.5 seconds for 131,072 steps, including policy updates. A second job resumed the checkpoint for a further 4,096 steps.

## Watch a learned population

```sh
python -m accrete_rl.play --model results/rl/first/final.zip --seed 1 --weather drought
```

The viewer gives every living resident its own observation and samples one action per resident each hour from the checkpoint. Each choice goes through the usual simulator rules. Use **Pause**, **Step**, and **Restart** to inspect a run. **Hours per second** changes playback speed. The policy weights are shared across residents, and this is a test of how a policy trained with scripted neighbours behaves in a population of its peers. The viewer uses Tkinter.

## Task contract

- **Clock.** Each action advances one hour. Focal death terminates the episode. The configurable time limit truncates it. Most experiments should hold the 21-day horizon fixed across policies.
- **Observation.** The `state` vector describes the agent's body, inventory, food-carrying room, home offset, remembered chest food and space (read directly at home), nearby contacts, household size, time, and the path underfoot. The `targets` rows contain relative position, remembered amount or score, age of information, and a travel estimate. Quantities are scaled and clipped. Field orders are exported as `STATE_FIELDS` and `TARGET_FIELDS`.
- **Action.** Choose wait, eat, rest, continue a route, return home, deposit, withdraw, or select a remembered food, wood, market or scouting target. Slots within each family are ordered by distance and coordinates. Deposit and withdrawal use Accrete's normal chest rules and require the agent to be at home. Arrival can fail if the route or resources changed. An unavailable slot acts as a wait and sets `info["masked_action"]`. Use `action_masks()` for training.
- **Reward.** Each surviving member of the initial household contributes `0.45 × (1 − hunger) + 0.25 × energy + 0.25 × health`, averaged over the starting household size. Up to 0.05 is added for food held in the starting chest, capped at 12 units. Focal death costs 2. The reported deprivation total covers the time simulated, so compare it alongside survival and episode length.
- **Information.** Reset gives the adult a surveyed route and an initial market tip. Subsequent information comes from perception and meetings. Drought schedules, live distant resource stocks and whole-world outcomes are absent from policy inputs. `episode_stats()` exposes the evaluator score separately.

The current scenario tests resource and market decisions within a small society. All other residents use the simulator's native policy. Their actions, the market's supply, and barter outcomes continue to evolve while the learner acts. Gymnasium supplies the environment API and Maskable PPO supplies the trainer. [Gymnasium environment contract](https://gymnasium.farama.org/api/env/) · [Maskable PPO](https://sb3-contrib.readthedocs.io/en/master/modules/ppo_mask.html)
