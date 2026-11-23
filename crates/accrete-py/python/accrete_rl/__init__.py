"""Accrete's single-agent household and market environment."""

from gymnasium.envs.registration import register

from .env import ACTIONS, STATE_FIELDS, TARGET_FIELDS, TASK_VERSION, HouseholdEnv

ENV_ID = "AccreteHousehold-v0"
register(id=ENV_ID, entry_point="accrete_rl:HouseholdEnv")

__all__ = ["ENV_ID", "HouseholdEnv", "ACTIONS", "STATE_FIELDS", "TARGET_FIELDS", "TASK_VERSION"]
