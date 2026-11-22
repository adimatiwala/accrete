"""Masked PPO for the Accrete environment.

    python -m accrete_rl.train --total-steps 200000 --out runs/ppo

Training worlds use seeds from ``--train-seed-base`` upward; evaluation uses a
disjoint, fixed set of seeds (1..eval_episodes), so the reported numbers
measure generalisation to unseen worlds. Every evaluation is compared with
three baselines on the same seeds: the agent's own scripted logic acting
through the same interface ("heuristic"), the agent left entirely to its
scripted logic ("native"), and uniformly random valid actions ("random").
"""

from __future__ import annotations

import argparse
import csv
import json
import time
from collections import deque
from concurrent.futures import ThreadPoolExecutor
from dataclasses import asdict, dataclass, field
from pathlib import Path

import numpy as np
import torch
from torch import nn

from .env import AccreteEnv
from .evaluate import evaluate_baselines, evaluate_policy
from .model import ActorCritic, obs_to_tensors

REPO_CONFIGS = Path(__file__).resolve().parents[4] / "configs" / "rl"


@dataclass
class PPOConfig:
    total_steps: int = 200_000
    num_envs: int = 8
    rollout_steps: int = 256
    epochs: int = 4
    minibatch_size: int = 512
    lr: float = 3e-4
    gamma: float = 0.995
    gae_lambda: float = 0.95
    clip: float = 0.2
    ent_coef: float = 0.01
    vf_coef: float = 0.5
    max_grad_norm: float = 0.5
    reward_scale: float = 0.1
    bc_steps: int = 0
    eval_every: int = 10
    eval_episodes: int = 5
    train_seed_base: int = 10_000
    seed: int = 0
    sim_config: str | None = None
    rl_config: str | None = None
    out: str = "runs/ppo"
    device: str = "cpu"
    extra: dict = field(default_factory=dict)


def stack(obs_list: list[dict]) -> dict[str, np.ndarray]:
    return {k: np.stack([o[k] for o in obs_list]) for k in obs_list[0]}


class VecEnv:
    """Minimal synchronous vector env with auto-reset on fresh training seeds."""

    def __init__(self, cfg: PPOConfig) -> None:
        self.envs = [AccreteEnv(cfg.sim_config, cfg.rl_config) for _ in range(cfg.num_envs)]
        # The Rust core releases the GIL, so envs step in parallel on threads.
        self.pool = ThreadPoolExecutor(max_workers=cfg.num_envs)
        self.next_seed = cfg.train_seed_base
        seeds = list(range(self.next_seed, self.next_seed + cfg.num_envs))
        self.next_seed += cfg.num_envs
        self.obs = list(self.pool.map(lambda es: es[0].reset(seed=es[1])[0], zip(self.envs, seeds)))
        self.ep_return = np.zeros(cfg.num_envs)
        self.finished: deque[dict] = deque(maxlen=50)

    def _reset(self, env: AccreteEnv) -> dict:
        obs, _ = env.reset(seed=self.next_seed)
        self.next_seed += 1
        return obs

    def step(self, actions: np.ndarray):
        rewards = np.zeros(len(self.envs), dtype=np.float32)
        dones = np.zeros(len(self.envs), dtype=np.float32)
        results = list(self.pool.map(lambda ea: ea[0].step(int(ea[1])), zip(self.envs, actions)))
        for i, (env, (obs, r, term, trunc, info)) in enumerate(zip(self.envs, results)):
            rewards[i] = r
            self.ep_return[i] += r
            if term or trunc:
                dones[i] = 1.0
                self.finished.append(
                    {"return": self.ep_return[i], "died": info["death"] is not None, "day": info["day"]}
                )
                self.ep_return[i] = 0.0
                obs = self._reset(env)
            self.obs[i] = obs
        return rewards, dones


def behaviour_cloning(model: ActorCritic, opt, cfg: PPOConfig, log) -> None:
    """Optional warm start: imitate the scripted policy for ``bc_steps`` steps."""
    env = AccreteEnv(cfg.sim_config, cfg.rl_config)
    obs, _ = env.reset(seed=cfg.train_seed_base - 1)
    seed = cfg.train_seed_base - 2
    batch_obs, batch_act = [], []
    for step in range(cfg.bc_steps):
        a = env.heuristic_action()
        batch_obs.append(obs)
        batch_act.append(a)
        obs, _, term, trunc, _ = env.step(a)
        if term or trunc:
            obs, _ = env.reset(seed=seed)
            seed -= 1
        if len(batch_obs) == 1024 or step == cfg.bc_steps - 1:
            t = obs_to_tensors(stack(batch_obs), cfg.device)
            acts = torch.as_tensor(batch_act, device=cfg.device)
            for _ in range(4):
                logits, _ = model(t)
                loss = nn.functional.cross_entropy(logits, acts)
                opt.zero_grad()
                loss.backward()
                opt.step()
            acc = (logits.argmax(-1) == acts).float().mean().item()
            log(f"  bc step {step + 1}: loss {loss.item():.3f} imitation accuracy {acc:.2%}")
            batch_obs, batch_act = [], []


def train(cfg: PPOConfig) -> Path:
    out = Path(cfg.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "config.json").write_text(json.dumps(asdict(cfg), indent=2))
    torch.manual_seed(cfg.seed)
    np.random.seed(cfg.seed)
    log_file = open(out / "log.txt", "w", encoding="utf-8")

    def log(msg: str) -> None:
        print(msg, flush=True)
        log_file.write(msg + "\n")
        log_file.flush()

    model = ActorCritic().to(cfg.device)
    opt = torch.optim.Adam(model.parameters(), lr=cfg.lr, eps=1e-5)
    eval_seeds = list(range(1, cfg.eval_episodes + 1))
    log(f"baselines on held-out seeds {eval_seeds} ...")
    baselines = evaluate_baselines(cfg.sim_config, cfg.rl_config, eval_seeds)
    for name, b in baselines.items():
        log(f"  {name:<9} return {b['return']:8.2f}  days {b['days']:5.1f}  deaths {b['deaths']}/{len(eval_seeds)}  births {b['births']:.2f}")
    (out / "baselines.json").write_text(json.dumps(baselines, indent=2))

    if cfg.bc_steps > 0:
        log(f"behaviour cloning for {cfg.bc_steps} steps")
        behaviour_cloning(model, opt, cfg, log)

    venv = VecEnv(cfg)
    n, t_len = cfg.num_envs, cfg.rollout_steps
    updates = max(1, cfg.total_steps // (n * t_len))
    train_csv = open(out / "train.csv", "w", newline="", encoding="utf-8")
    train_w = csv.writer(train_csv)
    train_w.writerow(["update", "steps", "fps", "ep_return", "ep_death_rate", "policy_loss", "value_loss", "entropy", "approx_kl", "clip_frac"])
    eval_csv = open(out / "eval.csv", "w", newline="", encoding="utf-8")
    eval_w = csv.writer(eval_csv)
    eval_w.writerow(["update", "steps", "return", "days", "deaths", "births", "deprived_days", "greedy_return"])
    best = -float("inf")
    t0 = time.time()
    steps_done = 0

    for update in range(1, updates + 1):
        buf_obs: list[dict] = []
        actions = np.zeros((t_len, n), dtype=np.int64)
        logps = np.zeros((t_len, n), dtype=np.float32)
        values = np.zeros((t_len, n), dtype=np.float32)
        rewards = np.zeros((t_len, n), dtype=np.float32)
        dones = np.zeros((t_len, n), dtype=np.float32)
        model.eval()
        for t in range(t_len):
            batch = stack(venv.obs)
            buf_obs.append(batch)
            with torch.no_grad():
                a, lp, v = model.act(obs_to_tensors(batch, cfg.device))
            actions[t], logps[t], values[t] = a.cpu().numpy(), lp.cpu().numpy(), v.cpu().numpy()
            r, d = venv.step(actions[t])
            rewards[t], dones[t] = r * cfg.reward_scale, d
        steps_done += n * t_len
        with torch.no_grad():
            _, _, last_v = model.act(obs_to_tensors(stack(venv.obs), cfg.device))
        last_v = last_v.cpu().numpy()

        # Generalized advantage estimation. dones[t] = 1 means the step at t
        # ended an episode (the next obs belongs to a fresh episode).
        adv = np.zeros_like(rewards)
        gae = np.zeros(n, dtype=np.float32)
        for t in reversed(range(t_len)):
            next_v = last_v if t == t_len - 1 else values[t + 1]
            nonterminal = 1.0 - dones[t]
            delta = rewards[t] + cfg.gamma * next_v * nonterminal - values[t]
            gae = delta + cfg.gamma * cfg.gae_lambda * nonterminal * gae
            adv[t] = gae
        returns = adv + values

        flat = {k: np.concatenate([b[k] for b in buf_obs]) for k in buf_obs[0]}
        tensors = obs_to_tensors(flat, cfg.device)
        b_act = torch.as_tensor(actions.reshape(-1), device=cfg.device)
        b_logp = torch.as_tensor(logps.reshape(-1), device=cfg.device)
        b_adv = torch.as_tensor(adv.reshape(-1), device=cfg.device)
        b_ret = torch.as_tensor(returns.reshape(-1), device=cfg.device)
        b_val = torch.as_tensor(values.reshape(-1), device=cfg.device)
        total = n * t_len
        model.train()
        stats = {"pl": [], "vl": [], "ent": [], "kl": [], "cf": []}
        for _ in range(cfg.epochs):
            perm = torch.randperm(total, device=cfg.device)
            for start in range(0, total, cfg.minibatch_size):
                idx = perm[start : start + cfg.minibatch_size]
                mb = {k: v[idx] for k, v in tensors.items()}
                new_logp, ent, new_v = model.evaluate(mb, b_act[idx])
                mb_adv = b_adv[idx]
                mb_adv = (mb_adv - mb_adv.mean()) / (mb_adv.std() + 1e-8)
                ratio = (new_logp - b_logp[idx]).exp()
                pl = -torch.min(ratio * mb_adv, ratio.clamp(1 - cfg.clip, 1 + cfg.clip) * mb_adv).mean()
                v_clip = b_val[idx] + (new_v - b_val[idx]).clamp(-cfg.clip, cfg.clip)
                vl = 0.5 * torch.max((new_v - b_ret[idx]) ** 2, (v_clip - b_ret[idx]) ** 2).mean()
                loss = pl + cfg.vf_coef * vl - cfg.ent_coef * ent.mean()
                opt.zero_grad()
                loss.backward()
                nn.utils.clip_grad_norm_(model.parameters(), cfg.max_grad_norm)
                opt.step()
                with torch.no_grad():
                    log_ratio = new_logp - b_logp[idx]
                    stats["kl"].append(((ratio - 1) - log_ratio).mean().item())
                    stats["cf"].append(((ratio - 1).abs() > cfg.clip).float().mean().item())
                stats["pl"].append(pl.item())
                stats["vl"].append(vl.item())
                stats["ent"].append(ent.mean().item())

        fps = steps_done / (time.time() - t0)
        fin = list(venv.finished)
        ep_ret = float(np.mean([f["return"] for f in fin])) if fin else float("nan")
        death_rate = float(np.mean([f["died"] for f in fin])) if fin else float("nan")
        m = {k: float(np.mean(v)) for k, v in stats.items()}
        train_w.writerow([update, steps_done, round(fps, 1), ep_ret, death_rate, m["pl"], m["vl"], m["ent"], m["kl"], m["cf"]])
        train_csv.flush()
        log(
            f"update {update:4d}/{updates} steps {steps_done:8d} fps {fps:6.0f} | "
            f"train return {ep_ret:7.2f} deaths {death_rate:.0%} | "
            f"pl {m['pl']:+.3f} vl {m['vl']:.3f} ent {m['ent']:.2f} kl {m['kl']:.4f}"
        )

        if update % cfg.eval_every == 0 or update == updates:
            ev = evaluate_policy(model, cfg.sim_config, cfg.rl_config, eval_seeds, cfg.device)
            greedy = evaluate_policy(model, cfg.sim_config, cfg.rl_config, eval_seeds, cfg.device, deterministic=True)
            eval_w.writerow(
                [update, steps_done, ev["return"], ev["days"], ev["deaths"], ev["births"], ev["deprived_days"], greedy["return"]]
            )
            eval_csv.flush()
            h = baselines["heuristic"]["return"]
            rnd = baselines["random"]["return"]
            gap = (ev["return"] - rnd) / max(h - rnd, 1e-6)
            log(
                f"  eval: return {ev['return']:.2f} (random {rnd:.2f}, heuristic {h:.2f}; "
                f"{gap:.0%} of the way) days {ev['days']:.1f} deaths {ev['deaths']}/{len(eval_seeds)} "
                f"births {ev['births']:.2f} | greedy {greedy['return']:.2f}"
            )
            torch.save({"model": model.state_dict(), "config": asdict(cfg), "update": update}, out / "last.pt")
            if ev["return"] > best:
                best = ev["return"]
                torch.save({"model": model.state_dict(), "config": asdict(cfg), "update": update}, out / "best.pt")
    log_file.close()
    train_csv.close()
    eval_csv.close()
    return out


def main(argv: list[str] | None = None) -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    defaults = PPOConfig()
    for name, value in asdict(defaults).items():
        if name == "extra":
            continue
        flag = "--" + name.replace("_", "-")
        if value is None:
            p.add_argument(flag, default=None)
        else:
            p.add_argument(flag, type=type(value), default=value)
    args = vars(p.parse_args(argv))
    if args["sim_config"] is None and (REPO_CONFIGS / "small_world.toml").exists():
        args["sim_config"] = str(REPO_CONFIGS / "small_world.toml")
    if args["rl_config"] is None and (REPO_CONFIGS / "env.toml").exists():
        args["rl_config"] = str(REPO_CONFIGS / "env.toml")
    train(PPOConfig(**args))


if __name__ == "__main__":
    main()
