"""Compare policies on identical Accrete episode seeds."""

import argparse
import csv
from pathlib import Path
from statistics import mean
from time import perf_counter

import numpy as np

from .env import MAX_DAYS, HouseholdEnv


def evaluate(env, choose, name, seed):
    obs, _ = env.reset(seed=seed)
    rng = np.random.default_rng(seed)
    total = 0.0
    while True:
        action = choose(obs, env.action_masks(), rng)
        obs, reward, terminated, truncated, _ = env.step(action)
        total += reward
        if terminated or truncated:
            break
    return {"policy": name, "seed": seed, "return": total, **env.episode_stats()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", type=int, default=1, help="first held-out world seed")
    parser.add_argument("--episodes", type=int, default=32, help="worlds per policy")
    parser.add_argument("--days", type=int, default=21)
    parser.add_argument("--weather", choices=("mixed", "steady", "drought"), default="mixed")
    parser.add_argument("--model", type=Path, help="also evaluate this MaskablePPO checkpoint")
    parser.add_argument("--greedy", action="store_true", help="take the most likely model action instead of sampling")
    parser.add_argument("--out", type=Path, default=Path("results/rl/eval.csv"))
    args = parser.parse_args()
    if args.episodes < 1 or args.seed < 0 or args.seed + args.episodes > 1 << 64:
        parser.error("episode count must be positive and world seeds must fit in 64 bits")
    if not 1 <= args.days <= MAX_DAYS:
        parser.error(f"days must be in 1..={MAX_DAYS}")

    env = HouseholdEnv(days=args.days, weather=args.weather)
    policies = {
        "random": lambda obs, mask, rng: rng.choice(np.flatnonzero(mask)),
        "reserve": lambda obs, mask, rng: env.reserve_action(),
    }
    sampled_policy = None
    if args.model is not None:
        import torch
        from sb3_contrib import MaskablePPO

        torch.set_num_threads(1)
        model = MaskablePPO.load(args.model, device="cpu")

        def learned(obs, mask, rng):
            action, _ = model.predict(obs, action_masks=mask, deterministic=args.greedy)
            return action.item()

        name = "learned-greedy" if args.greedy else "learned-sampled"
        policies[name] = learned
        if not args.greedy:
            sampled_policy = name
    start = perf_counter()
    rows = []
    for name, choose in policies.items():
        runs = []
        for seed in range(args.seed, args.seed + args.episodes):
            if name == sampled_policy:
                torch.manual_seed(seed)
            runs.append(evaluate(env, choose, name, seed))
        rows.extend(runs)
        print(
            f"{name:15s} return {mean(r['return'] for r in runs):7.2f}  "
            f"focal survival {mean(r['focal_alive'] for r in runs):6.1%}  "
            f"ticks {mean(r['ticks'] for r in runs):5.0f}  "
            f"household deprivation hours {mean(r['deprived_hours'] for r in runs):6.1f}  "
            f"focal trades {mean(r['focal_trades'] for r in runs):5.1f}"
        )
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", newline="", encoding="utf-8") as file:
        writer = csv.DictWriter(file, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)
    elapsed = perf_counter() - start
    print(f"{len(rows)} episodes in {elapsed:.1f}s. Saved {args.out}")


if __name__ == "__main__":
    main()
