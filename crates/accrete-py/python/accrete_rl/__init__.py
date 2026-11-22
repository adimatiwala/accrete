"""Reinforcement-learning environment for the Accrete emergent civilization simulator.

One learning agent lives in a society of scripted agents. It chooses
goal-level options (eat, rest, gather at one of several remembered places,
explore, build, reproduce, ...) and sees only what it knows. Rewards are
individual: being alive and healthy, not being deprived, having children.

Quick start::

    import gymnasium as gym
    import accrete_rl

    env = gym.make("Accrete-v0")
    obs, info = env.reset(seed=0)
    obs, reward, terminated, truncated, info = env.step(env.unwrapped.heuristic_action())
"""

from gymnasium.envs.registration import register

from . import _core
from ._core import (
    ACTION_NAMES,
    GRID_CHANNEL_NAMES,
    GRID_CHANNELS,
    GRID_SIDE,
    N_ACTIONS,
    SLOT_FEATURES,
    SLOT_NAMES,
    VECTOR_NAMES,
    VECTOR_SIZE,
    default_rl_config,
    default_sim_config,
)
from .env import AccreteEnv, load_toml_text

__all__ = [
    "AccreteEnv",
    "load_toml_text",
    "default_rl_config",
    "default_sim_config",
    "ACTION_NAMES",
    "GRID_CHANNEL_NAMES",
    "VECTOR_NAMES",
    "SLOT_NAMES",
    "N_ACTIONS",
    "GRID_CHANNELS",
    "GRID_SIDE",
    "VECTOR_SIZE",
    "SLOT_FEATURES",
    "_core",
]

register(id="Accrete-v0", entry_point="accrete_rl.env:AccreteEnv")
