"""Tests for the Python RL environment and training stack.

Run with:  maturin develop -m crates/accrete-py/Cargo.toml && pytest crates/accrete-py/tests
"""

import numpy as np
import pytest
import torch
from gymnasium.utils.env_checker import check_env

import accrete_rl
from accrete_rl import AccreteEnv
from accrete_rl.env import WAIT_ACTION
from accrete_rl.model import ActorCritic, obs_to_tensors

SMALL_WORLD = "[world]\nwidth = 64\nheight = 64\n[spawn]\nagents = 40\n"
SHORT = "warmup_days = 3\nmax_days = 4\n"


def make(**kw) -> AccreteEnv:
    return AccreteEnv(SMALL_WORLD, SHORT, **kw)


def test_gymnasium_api_conformance():
    check_env(make(), skip_render_check=True)


def test_registered_with_gymnasium():
    import gymnasium as gym

    env = gym.make("Accrete-v0", sim_config=SMALL_WORLD, rl_config=SHORT)
    obs, _ = env.reset(seed=1)
    assert env.observation_space.contains(obs)


def test_same_seed_and_actions_reproduce_exactly():
    def rollout():
        env = make()
        obs, _ = env.reset(seed=5)
        trace = [obs["vector"].copy()]
        rewards = []
        for _ in range(60):
            obs, r, term, trunc, _ = env.step(env.heuristic_action())
            trace.append(obs["vector"].copy())
            rewards.append(r)
            if term or trunc:
                break
        return np.stack(trace), np.array(rewards)

    (a_obs, a_r), (b_obs, b_r) = rollout(), rollout()
    np.testing.assert_array_equal(a_obs, b_obs)
    np.testing.assert_array_equal(a_r, b_r)


def test_observations_stay_in_bounds_and_masks_are_consistent():
    env = make()
    obs, _ = env.reset(seed=2)
    rng = np.random.default_rng(0)
    for _ in range(200):
        assert env.observation_space.contains(obs)
        mask = obs["action_mask"].astype(bool)
        assert mask[WAIT_ACTION], "waiting must always be available"
        np.testing.assert_array_equal(mask, env.action_masks())
        assert mask[env.heuristic_action()]
        # Slot features are zero exactly where the action is masked.
        assert np.all(obs["slots"][~mask] == 0)
        a = int(rng.choice(np.flatnonzero(mask)))
        obs, _, term, trunc, info = env.step(a)
        assert not info["masked_action"]
        if term or trunc:
            obs, _ = env.reset(seed=int(rng.integers(1000)))


def test_masked_actions_wait_or_raise():
    env = make()
    obs, _ = env.reset(seed=4)
    masked = np.flatnonzero(obs["action_mask"] == 0)
    assert len(masked) > 0
    _, _, _, _, info = env.step(int(masked[0]))
    assert info["masked_action"]
    strict = make(strict_actions=True)
    obs, _ = strict.reset(seed=4)
    with pytest.raises(ValueError):
        strict.step(int(np.flatnonzero(obs["action_mask"] == 0)[0]))


def test_metadata_names_match_shapes():
    assert len(accrete_rl.ACTION_NAMES) == accrete_rl.N_ACTIONS
    assert len(accrete_rl.GRID_CHANNEL_NAMES) == accrete_rl.GRID_CHANNELS
    assert len(accrete_rl.VECTOR_NAMES) == accrete_rl.VECTOR_SIZE
    assert len(accrete_rl.SLOT_NAMES) == accrete_rl.SLOT_FEATURES
    assert accrete_rl.ACTION_NAMES[WAIT_ACTION] == "idle#0"


def test_policy_never_samples_masked_actions():
    env = make()
    obs, _ = env.reset(seed=6)
    model = ActorCritic()
    t = obs_to_tensors(obs)
    for _ in range(50):
        a, lp, v = model.act(t)
        assert obs["action_mask"][int(a)] == 1
        assert torch.isfinite(lp).all() and torch.isfinite(v).all()


def test_training_smoke(tmp_path):
    from accrete_rl.train import PPOConfig, train

    cfg = PPOConfig(
        total_steps=128,
        num_envs=2,
        rollout_steps=64,
        epochs=1,
        minibatch_size=64,
        eval_every=1,
        eval_episodes=1,
        bc_steps=32,
        sim_config=SMALL_WORLD,
        rl_config=SHORT,
        out=str(tmp_path / "run"),
    )
    out = train(cfg)
    assert (out / "best.pt").exists() and (out / "train.csv").exists()
