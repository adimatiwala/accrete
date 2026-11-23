"""Gymnasium interface to the Rust task."""

import operator

import gymnasium as gym
import numpy as np
from gymnasium import spaces
from gymnasium.error import ResetNeeded

from . import _core

ACTIONS = tuple(_core.ACTIONS)
STATE_FIELDS = tuple(_core.STATE_FIELDS)
TARGET_FIELDS = tuple(_core.TARGET_FIELDS)
TASK_VERSION = _core.TASK_VERSION
MAX_DAYS = _core.MAX_DAYS
GENERATED_SEED_MIN = 1 << 40
_SCORE_FIELDS = (
    "ticks", "alive", "focal_alive", "deprived_hours", "focal_trades", "tiles_moved", "home_food"
)


class HouseholdEnv(gym.Env):
    """Control one adult in a native Accrete household."""

    metadata = {"render_modes": []}
    render_mode = None

    def __init__(self, *, days=21, weather="mixed", check=False):
        days = operator.index(days)
        if not 1 <= days <= MAX_DAYS:
            raise ValueError(f"days must be in 1..={MAX_DAYS}")
        if weather not in ("mixed", "steady", "drought"):
            raise ValueError("weather must be mixed, steady or drought")
        self.days = days
        self.weather = weather
        self.check = check
        self.action_space = spaces.Discrete(len(ACTIONS))
        self.observation_space = spaces.Dict({
            "state": spaces.Box(-1.0, 1.0, (len(STATE_FIELDS),), dtype=np.float32),
            "targets": spaces.Box(
                -1.0, 1.0, (len(ACTIONS), len(TARGET_FIELDS)), dtype=np.float32
            ),
            "action_mask": spaces.MultiBinary(len(ACTIONS)),
        })
        self._native = None
        self._done = False
        self._mask = None

    def reset(self, *, seed=None, options=None):
        if seed is not None:
            seed = operator.index(seed)
            if not 0 <= seed < 1 << 64:
                raise ValueError("seed must fit in an unsigned 64-bit integer")
        super().reset(seed=seed)
        if seed is None:
            seed = int(self.np_random.integers(GENERATED_SEED_MIN, 1 << 63))
        self._native = _core.NativeHousehold(seed, self.days, self.weather, self.check)
        self._done = False
        return self._observe(), {}

    def _observe(self):
        state, targets, mask = self._native.observation()
        self._mask = np.asarray(mask, dtype=np.bool_)
        return {
            "state": np.asarray(state, dtype=np.float32),
            "targets": np.asarray(targets, dtype=np.float32),
            "action_mask": self._mask.astype(np.int8),
        }

    def _require_reset(self):
        if self._native is None:
            raise ResetNeeded("reset before using the environment")

    def step(self, action):
        self._require_reset()
        if self._done:
            raise ResetNeeded("reset after the episode ends")
        if not self.action_space.contains(action):
            raise ValueError(f"action {action!r} is outside the action space")
        reward, terminated, truncated, masked = self._native.step(int(action))
        self._done = terminated or truncated
        return self._observe(), reward, terminated, truncated, {"masked_action": masked}

    def action_masks(self):
        """Boolean mask consumed by sb3-contrib and other masked policies."""
        self._require_reset()
        return self._mask.copy()

    def reserve_action(self):
        self._require_reset()
        return self._native.reserve_action()

    def episode_stats(self):
        """Evaluator-only aggregates. Keep these out of the policy's inputs."""
        self._require_reset()
        return dict(zip(_SCORE_FIELDS, self._native.score()))
