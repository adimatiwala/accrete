"""Train Maskable PPO on fresh Accrete household episodes."""

import argparse
import json
import math
from importlib.metadata import version
from pathlib import Path
from time import perf_counter, time_ns

from .env import GENERATED_SEED_MIN, MAX_DAYS, TASK_VERSION, HouseholdEnv


class TrainingHousehold(HouseholdEnv):
    """Use a disjoint range of world seeds even on the first vector reset."""

    def reset(self, *, seed=None, options=None):
        if seed is not None:
            seed += GENERATED_SEED_MIN
        return super().reset(seed=seed, options=options)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--steps", type=int, default=250_000, help="additional environment steps")
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--rollout", type=int, default=512, help="steps per worker per update")
    parser.add_argument("--days", type=int, default=21)
    parser.add_argument("--weather", choices=("mixed", "steady", "drought"), default="mixed")
    parser.add_argument("--seed", type=int, default=42, help="optimiser and rollout seed")
    parser.add_argument("--out", type=Path, default=Path("results/rl/train"))
    parser.add_argument("--checkpoint-every", type=int, default=50_000)
    parser.add_argument("--resume", type=Path)
    args = parser.parse_args()
    if min(args.steps, args.workers, args.checkpoint_every) < 1 or args.rollout < 2:
        parser.error("steps, workers and checkpoint interval must be positive, rollout at least 2")
    if not 0 <= args.seed < (1 << 32) - args.workers:
        parser.error("seed and worker offsets must fit in 32 bits")
    if not 1 <= args.days <= MAX_DAYS:
        parser.error(f"days must be in 1..={MAX_DAYS}")

    try:
        import torch
        from sb3_contrib import MaskablePPO
        from stable_baselines3.common.callbacks import CheckpointCallback
        from stable_baselines3.common.env_util import make_vec_env
        from stable_baselines3.common.vec_env import DummyVecEnv, SubprocVecEnv
    except ImportError as error:
        parser.exit(1, f"Install the accrete-rl train extra before training. {error}\n")

    # Tiny networks and independent workers run well with one Torch CPU thread.
    torch.set_num_threads(1)
    out = args.out
    if out.is_dir() and any(out.iterdir()):
        out /= f"session-{time_ns()}"
    out.mkdir(parents=True, exist_ok=True)
    print(f"Training artifacts will be saved in {out}", flush=True)
    vec = make_vec_env(
        TrainingHousehold,
        n_envs=args.workers,
        seed=args.seed,
        env_kwargs={"days": args.days, "weather": args.weather},
        monitor_dir=str(out / "monitor"),
        vec_env_cls=SubprocVecEnv if args.workers > 1 else DummyVecEnv,
    )
    start = perf_counter()
    try:
        if args.resume:
            model = MaskablePPO.load(args.resume, env=vec, seed=args.seed, device="cpu")
        else:
            batch = math.gcd(256, args.rollout * args.workers)
            if batch < 2:
                batch = args.rollout
            model = MaskablePPO(
                "MultiInputPolicy",
                vec,
                seed=args.seed,
                n_steps=args.rollout,
                batch_size=batch,
                gamma=0.995,
                verbose=1,
                device="cpu",
            )
        initial_steps = model.num_timesteps
        config = {
            **vars(args),
            "task_version": TASK_VERSION,
            "initial_steps": initial_steps,
            "rollout": model.n_steps,
            "dependencies": {
                name: version(name)
                for name in ("accrete-rl", "gymnasium", "numpy", "sb3-contrib", "stable-baselines3", "torch")
            },
        }
        (out / "run.json").write_text(json.dumps(config, indent=2, default=str) + "\n", encoding="utf-8")
        checkpoint = CheckpointCallback(
            save_freq=max(1, args.checkpoint_every // args.workers),
            save_path=str(out / "checkpoints"),
            name_prefix="household",
        )
        model.learn(args.steps, callback=checkpoint, reset_num_timesteps=not args.resume, log_interval=10)
        model.save(out / "final.zip")
        steps = model.num_timesteps - initial_steps
        seconds = perf_counter() - start
        print(f"{steps:,} new steps in {seconds:.1f}s ({steps / seconds:,.0f} steps/s)")
        print(f"Saved {out / 'final.zip'}")
    finally:
        vec.close()


if __name__ == "__main__":
    main()
