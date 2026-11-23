import gymnasium as gym
import numpy as np
import pytest
from gymnasium.error import ResetNeeded
from gymnasium.utils.env_checker import check_env

import accrete_rl
from accrete_rl import ACTIONS, STATE_FIELDS, HouseholdEnv


def test_gymnasium_contract():
    check_env(HouseholdEnv(days=1), skip_render_check=True)
    with gym.make(accrete_rl.ENV_ID, days=1) as env:
        obs, _ = env.reset(seed=2)
        assert env.observation_space.contains(obs)


def test_same_seed_and_actions_replay():
    a = HouseholdEnv(days=3, weather="drought", check=True)
    b = HouseholdEnv(days=3, weather="drought", check=True)
    left, right = a.reset(seed=77)[0], b.reset(seed=77)[0]
    for _ in range(72):
        for key in left:
            np.testing.assert_array_equal(left[key], right[key])
        action = a.reserve_action()
        assert a.action_masks()[action]
        left, *result = a.step(action)
        right, *other = b.step(action)
        assert result == other
        assert a.episode_stats() == b.episode_stats()
    assert result[1:3] == [False, True]
    assert a.episode_stats()["ticks"] == 72
    with pytest.raises(ResetNeeded):
        a.step(0)


def test_masked_action_waits_and_bad_index_does_not_advance():
    env = HouseholdEnv(days=1)
    obs, _ = env.reset(seed=1)
    unavailable = next(i for i, legal in enumerate(obs["action_mask"]) if not legal)
    with pytest.raises(ValueError):
        env.step(len(ACTIONS))
    assert env.episode_stats()["ticks"] == 0
    _, _, _, _, info = env.step(unavailable)
    assert info == {"masked_action": True}
    assert env.episode_stats()["ticks"] == 1


def test_seeded_market_is_reachable_and_reveals_distant_food():
    env = HouseholdEnv(days=2, check=True)
    obs, _ = env.reset(seed=1)
    food = slice(ACTIONS.index("food_0"), ACTIONS.index("wood_0"))
    home_dx = STATE_FIELDS.index("home_dx")

    def knows_distant_food(observation):
        distance_from_home = abs(observation["targets"][food, 0] - observation["state"][home_dx])
        return np.any((distance_from_home > 0.4) & observation["action_mask"][food].astype(bool))

    assert not knows_distant_food(obs)
    market = ACTIONS.index("market_0")
    for _ in range(18):
        assert env.action_masks()[market]
        obs, _, terminated, truncated, _ = env.step(market)
        assert not (terminated or truncated)
    assert env.episode_stats()["tiles_moved"] > 0
    assert knows_distant_food(obs)


def test_death_terminates_independently_of_time_limit():
    env = HouseholdEnv(days=21, weather="drought")
    env.reset(seed=3)
    for _ in range(21 * 24):
        _, _, terminated, truncated, _ = env.step(0)
        if terminated or truncated:
            break
    assert terminated and not truncated
    assert not env.episode_stats()["focal_alive"]


def test_unobserved_future_weather_is_absent_from_policy_inputs():
    a = HouseholdEnv(days=21, weather="steady")
    b = HouseholdEnv(days=21, weather="drought")
    left, right = a.reset(seed=12)[0], b.reset(seed=12)[0]
    for _ in range(48):
        for key in left:
            np.testing.assert_array_equal(left[key], right[key])
        action = a.reserve_action()
        left, *result = a.step(action)
        right, *other = b.step(action)
        assert result == other
