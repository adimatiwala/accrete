"""Gymnasium wrapper around the Rust environment (`accrete_rl._core.Env`)."""

from __future__ import annotations

import os
from typing import Any

import gymnasium as gym
import numpy as np
from gymnasium import spaces

from . import _core

#: Action 0 is ``idle#0``: wait one tick in place. It is always available.
WAIT_ACTION = 0


def load_toml_text(config: str | os.PathLike | None) -> str | None:
    """Accept a TOML string, a path to a TOML file, or None (defaults)."""
    if config is None:
        return None
    text = os.fspath(config)
    if "\n" not in text and os.path.isfile(text):
        with open(text, encoding="utf-8") as f:
            return f.read()
    return text


class AccreteEnv(gym.Env):
    """One learning agent in a scripted Accrete society.

    Observation (``Dict``):
      * ``grid``: float32 ``[GRID_CHANNELS, 13, 13]``, the agent's own
        knowledge around it (terrain, remembered resources, homes, paths, agents
        it can see); channel names in ``accrete_rl.GRID_CHANNEL_NAMES``.
      * ``vector``: float32 ``[VECTOR_SIZE]`` body, inventory, household, traits.
      * ``slots``: float32 ``[N_ACTIONS, SLOT_FEATURES]`` the option behind
        each action (valid, eta, dx, dy, utility if exposed).
      * ``action_mask``: int8 ``[N_ACTIONS]``, 1 where the action is available.

    Action: ``Discrete(N_ACTIONS)``; action ``k * 3 + j`` is the ``j``-th best
    option of goal kind ``k`` (names in ``accrete_rl.ACTION_NAMES``). Use
    ``action_masks()`` to pick available actions. A masked action is replaced by
    ``WAIT_ACTION`` (wait one tick; always available) and flagged with
    ``info["masked_action"] = True``, unless ``strict_actions=True``, in which
    case it raises ``ValueError``.

    Episodes run ``max_days`` simulated days (truncation) or until the agent
    dies (termination). ``reset(seed=s)`` with an int uses world seed ``s``
    exactly, so episodes are reproducible; with ``seed=None`` a world seed is
    drawn from the environment's ``np_random``.
    """

    metadata = {"render_modes": ["ansi"]}

    def __init__(
        self,
        sim_config: str | os.PathLike | None = None,
        rl_config: str | os.PathLike | None = None,
        render_mode: str | None = None,
        strict_actions: bool = False,
    ) -> None:
        super().__init__()
        self.strict_actions = strict_actions
        self._env = _core.Env(load_toml_text(sim_config), load_toml_text(rl_config))
        self.render_mode = render_mode
        n, c, side = _core.N_ACTIONS, _core.GRID_CHANNELS, _core.GRID_SIDE
        self.observation_space = spaces.Dict(
            {
                "grid": spaces.Box(0.0, 1.0, (c, side, side), np.float32),
                "vector": spaces.Box(-1.0, 1.0, (_core.VECTOR_SIZE,), np.float32),
                "slots": spaces.Box(-1.0, 1.0, (n, _core.SLOT_FEATURES), np.float32),
                "action_mask": spaces.MultiBinary(n),
            }
        )
        self.action_space = spaces.Discrete(n)
        self._world_seed: int | None = None

    # -- Gymnasium API ------------------------------------------------------
    def reset(self, *, seed: int | None = None, options: dict[str, Any] | None = None):
        super().reset(seed=seed)
        if seed is not None:
            world_seed = int(seed)
        else:
            world_seed = int(self.np_random.integers(0, 2**62))
        self._world_seed = world_seed
        obs = self._env.reset(world_seed)
        return obs, {"world_seed": world_seed, **self._env.status()}

    def step(self, action):
        action = int(action)
        masked = False
        if not self.strict_actions and not self._env.action_mask()[action]:
            action, masked = WAIT_ACTION, True
        obs, reward, terminated, truncated, info = self._env.step(action)
        info["masked_action"] = masked
        return obs, float(reward), bool(terminated), bool(truncated), info

    def render(self):
        if self.render_mode == "ansi":
            s = self._env.status()
            return (
                f"day {s.get('day')} pop {s.get('population')} | agent {s.get('agent')} "
                f"at {s.get('pos')} goal {s.get('goal')} hunger {s.get('hunger', 0):.2f} "
                f"energy {s.get('energy', 0):.2f} health {s.get('health', 0):.2f}"
            )
        return None

    # -- Extras ---------------------------------------------------------------
    def action_masks(self) -> np.ndarray:
        """Boolean mask of available actions (sb3-contrib ``MaskablePPO`` API)."""
        return self._env.action_mask().astype(bool)

    def heuristic_action(self) -> int:
        """The action the agent's own scripted logic would take."""
        return int(self._env.heuristic_action())

    def describe_action(self, action: int) -> str | None:
        return self._env.describe_action(int(action))

    def native_episode(self, seed: int) -> dict:
        """Score the same episode with the agent left to its scripted logic."""
        return dict(self._env.native_episode(int(seed)))

    @property
    def world_seed(self) -> int | None:
        return self._world_seed
