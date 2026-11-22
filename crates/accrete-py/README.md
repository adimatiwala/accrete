# accrete-rl

A reinforcement-learning environment for the [Accrete](../../accrete.md)
emergent-civilization simulator. **One learning agent lives in a society of
scripted agents** and must survive, keep a household, and raise children
using only what it knows.

## Install (from the repository root)

```sh
python -m venv .venv --system-site-packages     # reuse an existing torch if you have one
.venv/Scripts/python -m pip install maturin gymnasium numpy pytest   # (bin/python on macOS)
.venv/Scripts/python -m maturin develop --release -m crates/accrete-py/Cargo.toml
.venv/Scripts/python -m pytest crates/accrete-py/tests
```

## Use

```python
import gymnasium as gym
import accrete_rl

env = gym.make("Accrete-v0", sim_config="configs/rl/small_world.toml", rl_config="configs/rl/env.toml")
obs, info = env.reset(seed=1)                      # world seed 1, fully reproducible
done = False
while not done:
    mask = env.unwrapped.action_masks()            # bool[42]
    action = env.unwrapped.heuristic_action()      # or your policy
    obs, reward, terminated, truncated, info = env.step(action)
    done = terminated or truncated
```

`sim_config` and `rl_config` accept a path, a TOML string, or `None` for the
defaults (`accrete_rl.default_sim_config()`, `accrete_rl.default_rl_config()`).

### Train and evaluate

```sh
python -m accrete_rl.train --total-steps 300000 --num-envs 16 --out runs/ppo
python -m accrete_rl.train --bc-steps 20000 --out runs/ppo_bc   # imitation warm start
python -m accrete_rl.evaluate --checkpoint runs/ppo/best.pt --episodes 10
```

The Rust CLI evaluates the baselines without Python:
`accrete --config configs/rl/small_world.toml rl-eval --rl-config configs/rl/env.toml`.

## The environment

| | |
|---|---|
| **Agent** | A young adult picked from the society on world `seed`, after `warmup_days` of scripted simulation. By then there are homes, neighbours and depleted land. |
| **Action** | `Discrete(42)`: 14 goal kinds × the 3 best targets of each, as ranked by the agent's own scoring (e.g. `gather-food#1` = the second-best remembered food place). Unavailable actions are masked. Action 0 (`idle#0`, wait one tick) is always available. Route planning and movement stay with the simulation. |
| **Step** | One *macro step*: the chosen goal runs until it completes, fails (no route), a need crosses its threshold (`interrupt_on_needs`), or `max_ticks_per_step` (12 hours) passes. |
| **Observation** | `grid` `[14, 13, 13]`: the agent's own knowledge of its sight square (terrain, move cost, remembered food/wood/stone, its home, other homes, trails, agents in view, what is visible now). `vector` `[36]`: body, inventory, household, traits, skills. `slots` `[42, 5]`: the option behind each action (valid, eta, dx, dy, and the scripted utility if `expose_utility`). `action_mask` `[42]`. Names: `accrete_rl.GRID_CHANNEL_NAMES`, `VECTOR_NAMES`, `SLOT_NAMES`, `ACTION_NAMES`. |
| **Reward** | +1 per day alive; −2 per day deprived (very hungry or exhausted); +2 × change in health; +5 per child born; −20 for dying of deprivation (no penalty for old age). Individual only: the agent is never paid for society-level outcomes. |
| **Episode** | `max_days` days (default 60, truncation), or death (termination). |
| **Information** | The agent sees exactly what its scripted counterpart may use: no undiscovered resources, no global statistics. |
| **Determinism** | The same seed, configs and actions give bit-identical observations and rewards. `reset(seed=None)` draws a world seed from the env's `np_random`. |

Masked actions are replaced by waiting (and flagged in `info["masked_action"]`).
With `AccreteEnv(..., strict_actions=True)` they raise `ValueError` instead.

### Baselines (built in)

* `heuristic`: the agent's own scripted logic, acting through this interface (`heuristic_action()`).
* `native`: the same agent left to its scripted logic every tick, with no macro steps (`native_episode(seed)`).
* `random`: uniform over available actions.

A learned policy is meaningful when it beats `random` clearly and approaches
or exceeds `heuristic`.

### The PPO implementation

`accrete_rl.train` is a compact, single-file masked PPO:

* **Network.** The CNN grid encoder and MLP vector encoder feed a shared context. Each action slot gets a logit from `MLP([context, slot features, learned slot embedding])`, so options are judged by what they are, not only by their index.
* **Training.** GAE(λ = 0.95), γ = 0.995, clipped policy and value losses, entropy bonus, reward scale 0.1.
* **Parallelism.** 16 environments stepped on threads; the Rust core releases the GIL.
* **Evaluation.** Uses held-out world seeds (1..N); training uses seeds from 10,000 up. The primary score is the sampled policy, with greedy (argmax) play reported alongside. Greedy play can collapse onto a single action, as a policy that relies on stochasticity often does.
* **Outputs** in `--out`: `log.txt`, `train.csv`, `eval.csv`, `baselines.json`, `best.pt`, `last.pt`.
