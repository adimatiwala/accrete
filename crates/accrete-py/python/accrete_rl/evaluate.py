"""Evaluate policies in the Accrete environment.

    python -m accrete_rl.evaluate --checkpoint runs/ppo/best.pt --episodes 10

Prints the checkpoint's results next to three baselines on the same seeds:
``heuristic`` (the scripted logic through the RL interface), ``native`` (the
agent left to its scripted logic every tick) and ``random`` (uniform over
available actions).
"""

from __future__ import annotations

import argparse

import numpy as np
import torch

from .env import AccreteEnv
from .model import obs_to_tensors


def _summarise(rows: list[dict]) -> dict:
    return {
        "return": float(np.mean([r["return"] for r in rows])),
        "days": float(np.mean([r["days"] for r in rows])),
        "deaths": int(sum(r["died"] for r in rows)),
        "births": float(np.mean([r["births"] for r in rows])),
        "deprived_days": float(np.mean([r["deprived_days"] for r in rows])),
    }


def run_episode(env: AccreteEnv, seed: int, choose) -> dict:
    """Play one episode with ``choose(env, obs) -> action``."""
    obs, info = env.reset(seed=seed)
    start_day = info["day"]
    total, births, deprived, died, day = 0.0, 0, 0, False, start_day
    while True:
        obs, r, term, trunc, info = env.step(choose(env, obs))
        total += r
        births += info["births"]
        deprived += info["deprived_ticks"]
        day = info["day"]
        if term or trunc:
            died = info["death"] is not None
            break
    return {"return": total, "days": day - start_day, "died": died, "births": births, "deprived_days": deprived / 24}


def evaluate_policy(model, sim_config, rl_config, seeds, device="cpu", deterministic=False) -> dict:
    """Evaluate ``model`` on ``seeds``.

    By default actions are *sampled* from the policy (with a fixed torch seed
    per episode, so results are reproducible). Greedy (argmax) play is
    available with ``deterministic=True``, but PPO policies here often rely on
    their stochasticity, and greedy play can collapse onto one action.
    """
    env = AccreteEnv(sim_config, rl_config)
    model.eval()

    def choose(_env, obs):
        with torch.no_grad():
            a, _, _ = model.act(obs_to_tensors(obs, device), deterministic=deterministic)
        return int(a.item())

    rows = []
    for s in seeds:
        torch.manual_seed(s)
        rows.append(run_episode(env, s, choose))
    return _summarise(rows)


def evaluate_baselines(sim_config, rl_config, seeds) -> dict:
    env = AccreteEnv(sim_config, rl_config)
    rng = np.random.default_rng(0)
    heuristic = [run_episode(env, s, lambda e, o: e.heuristic_action()) for s in seeds]
    random = [
        run_episode(env, s, lambda e, o: int(rng.choice(np.flatnonzero(o["action_mask"])))) for s in seeds
    ]
    native = []
    for s in seeds:
        n = env.native_episode(s)
        native.append(
            {
                "return": n["episode_return"],
                "days": n["days_alive"],
                "died": n["death"] is not None,
                "births": n["births"],
                "deprived_days": n["deprived_days"],
            }
        )
    return {"heuristic": _summarise(heuristic), "native": _summarise(native), "random": _summarise(random)}


def main(argv: list[str] | None = None) -> None:
    from .model import ActorCritic

    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--checkpoint", default=None)
    p.add_argument("--episodes", type=int, default=10)
    p.add_argument("--first-seed", type=int, default=1)
    p.add_argument("--sim-config", default=None)
    p.add_argument("--rl-config", default=None)
    args = p.parse_args(argv)
    seeds = list(range(args.first_seed, args.first_seed + args.episodes))
    sim_config, rl_config = args.sim_config, args.rl_config
    results = evaluate_baselines(sim_config, rl_config, seeds)
    if args.checkpoint:
        ckpt = torch.load(args.checkpoint, map_location="cpu", weights_only=False)
        cfg = ckpt["config"]
        sim_config = sim_config or cfg.get("sim_config")
        rl_config = rl_config or cfg.get("rl_config")
        results = evaluate_baselines(sim_config, rl_config, seeds)
        model = ActorCritic()
        model.load_state_dict(ckpt["model"])
        results["learned"] = evaluate_policy(model, sim_config, rl_config, seeds)
        results["greedy"] = evaluate_policy(model, sim_config, rl_config, seeds, deterministic=True)
    print(f"{'policy':<11}{'return':>9}{'days':>7}{'deaths':>8}{'births':>8}{'deprived':>10}")
    for name, r in results.items():
        print(f"{name:<11}{r['return']:9.2f}{r['days']:7.1f}{r['deaths']:>5}/{len(seeds):<2}{r['births']:8.2f}{r['deprived_days']:10.2f}")


if __name__ == "__main__":
    main()
