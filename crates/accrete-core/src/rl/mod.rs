//! Reinforcement-learning environment: one learning agent in a scripted society.
//!
//! The simulation runs exactly as usual, except that one agent's decisions
//! come from outside (see [`crate::simulation::Control`]). Everyone else keeps
//! their scripted utility logic, so the learner faces the same world, rules,
//! competition, and information limits as any agent.
//!
//! * **Actions** are goal-level options (see [`actions`]); one action runs as
//!   a *macro step* until the goal completes, fails, a need becomes urgent,
//!   or `max_ticks_per_step` passes.
//! * **Observations** contain only the agent's own knowledge (see
//!   [`observation`]).
//! * **Rewards** are individual only (being alive, not being deprived,
//!   children, not dying of deprivation), never society-level outcomes: the
//!   learner may *produce* settlements, but is never paid for them.
//! * **Determinism:** the same seed, configuration and action sequence always
//!   produce the same observations and rewards.

pub mod actions;
pub mod observation;

use serde::{Deserialize, Serialize};

use crate::agents::agent::GoalKind;
use crate::agents::decision::Candidate;
use crate::config::{Config, TICKS_PER_DAY};
use crate::rng::{Rng, Stream};
use crate::simulation::events::DeathCause;
use crate::simulation::{Control, Intent, Simulation};

pub use actions::{N_ACTIONS, SLOTS_PER_KIND};
pub use observation::{GRID_CHANNELS, GRID_SIDE, Observation, SLOT_FEATURES, VECTOR_SIZE};

/// Episode and reward settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RlConfig {
    /// Days of scripted simulation before the learner takes over (a lived-in
    /// world with homes, depletion, neighbours).
    pub warmup_days: u32,
    /// Episode length in days after warm-up (truncation).
    pub max_days: u32,
    /// Upper bound on simulation ticks per action (macro step).
    pub max_ticks_per_step: u32,
    /// End a macro step early when hunger crosses the agent's eat threshold
    /// or energy its rest threshold, so the policy can respond.
    pub interrupt_on_needs: bool,
    /// Control an adult no older than this many days past maturity, if one
    /// exists (so episodes are not cut short by old age).
    pub max_adult_age_days: f32,
    /// Include the scripted utility of each option in the slot features.
    pub expose_utility: bool,
    pub reward_alive_per_day: f32,
    pub reward_deprived_per_day: f32,
    pub reward_birth: f32,
    pub reward_deprivation_death: f32,
    /// Reward per unit change of health (shaping; 0 disables).
    pub reward_health: f32,
}

impl Default for RlConfig {
    fn default() -> Self {
        Self {
            warmup_days: 20,
            max_days: 60,
            max_ticks_per_step: 12,
            interrupt_on_needs: true,
            max_adult_age_days: 30.0,
            expose_utility: false,
            reward_alive_per_day: 1.0,
            reward_deprived_per_day: -2.0,
            reward_birth: 5.0,
            reward_deprivation_death: -20.0,
            reward_health: 2.0,
        }
    }
}

impl RlConfig {
    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| e.to_string())
    }
}

/// Diagnostics returned with every step.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct StepInfo {
    pub ticks: u32,
    pub day: u32,
    pub agent: u32,
    pub kind: &'static str,
    pub route_failed: bool,
    pub births: u32,
    pub deprived_ticks: u32,
    /// "deprivation" / "old age" when the agent died this step.
    pub death: Option<&'static str>,
    /// Why the macro step ended.
    pub ended_by: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepResult {
    pub obs: Observation,
    pub reward: f32,
    pub terminated: bool,
    pub truncated: bool,
    pub info: StepInfo,
}

#[derive(Debug)]
pub enum EnvError {
    NotReset,
    EpisodeOver,
    InvalidAction(usize),
    MaskedAction(usize),
    NoAgent,
}

impl std::fmt::Display for EnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvError::NotReset => write!(f, "call reset() before step()"),
            EnvError::EpisodeOver => write!(f, "episode is over; call reset()"),
            EnvError::InvalidAction(a) => write!(f, "action {a} out of range 0..{N_ACTIONS}"),
            EnvError::MaskedAction(a) => write!(f, "action {a} is masked (not available)"),
            EnvError::NoAgent => write!(f, "no living agent available to control"),
        }
    }
}

impl std::error::Error for EnvError {}

/// Outcome of one episode, for evaluation.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct EpisodeSummary {
    pub seed: u64,
    pub agent: u32,
    pub episode_return: f32,
    pub steps: u32,
    pub days_alive: f32,
    pub births: u32,
    pub deprived_days: f32,
    pub death: Option<&'static str>,
}

/// Reward for one simulated tick of the controlled agent: `after` is `None`
/// if it died this tick (with `cause`).
fn tick_reward(
    rl: &RlConfig,
    very_hungry: f32,
    before: &crate::agents::agent::Agent,
    after: Option<&crate::agents::agent::Agent>,
    cause: Option<DeathCause>,
) -> (f32, bool, u32) {
    let per_tick = 1.0 / TICKS_PER_DAY as f32;
    match after {
        None => {
            let r = if cause == Some(DeathCause::Deprivation) {
                rl.reward_deprivation_death
            } else {
                0.0
            };
            (r, false, 0)
        }
        Some(a) => {
            let mut r = rl.reward_alive_per_day * per_tick;
            let deprived = a.hunger >= very_hungry || a.energy <= 0.0;
            if deprived {
                r += rl.reward_deprived_per_day * per_tick;
            }
            r += rl.reward_health * (a.health - before.health);
            let births = a.stats.births.saturating_sub(before.stats.births);
            r += rl.reward_birth * births as f32;
            (r, deprived, births)
        }
    }
}

/// The environment. Create once, then `reset(seed)` / `step(action)`.
pub struct AccreteEnv {
    pub sim_config: Config,
    pub rl: RlConfig,
    sim: Option<Simulation>,
    agent: u32,
    slots: Vec<Option<Candidate>>,
    heuristic: usize,
    start_tick: u64,
    done: bool,
    births_before: u32,
}

impl AccreteEnv {
    pub fn new(sim_config: Config, rl: RlConfig) -> Self {
        Self {
            sim_config,
            rl,
            sim: None,
            agent: 0,
            slots: vec![None; N_ACTIONS],
            heuristic: 0,
            start_tick: 0,
            done: true,
            births_before: 0,
        }
    }

    pub fn sim(&self) -> Option<&Simulation> {
        self.sim.as_ref()
    }

    pub fn agent_id(&self) -> u32 {
        self.agent
    }

    /// Start a new episode on world `seed`.
    pub fn reset(&mut self, seed: u64) -> Result<Observation, EnvError> {
        let mut sim = Simulation::new(seed, self.sim_config.clone());
        sim.run(u64::from(self.rl.warmup_days) * TICKS_PER_DAY);
        // Pick the agent to control from its own stream: a young adult if any.
        let r = &sim.rates;
        let max_age = r.maturity_ticks + (self.rl.max_adult_age_days * TICKS_PER_DAY as f32) as u64;
        let mut pool: Vec<u32> = sim
            .agents
            .iter()
            .filter(|a| !a.is_juvenile(r) && a.age_ticks <= max_age)
            .map(|a| a.id)
            .collect();
        if pool.is_empty() {
            pool = sim
                .agents
                .iter()
                .filter(|a| !a.is_juvenile(r))
                .map(|a| a.id)
                .collect();
        }
        if pool.is_empty() {
            return Err(EnvError::NoAgent);
        }
        let mut rng = Rng::for_stream(seed, Stream::Baseline);
        let agent = pool[rng.below(pool.len() as u32) as usize];
        sim.control = Some(Control {
            agent,
            held: None,
            route_failed: false,
        });
        self.agent = agent;
        self.start_tick = sim.tick;
        self.done = false;
        self.births_before = sim.agent(agent).map_or(0, |a| a.stats.births);
        self.sim = Some(sim);
        Ok(self.refresh())
    }

    fn index(&self) -> Option<usize> {
        let sim = self.sim.as_ref()?;
        sim.agents.binary_search_by_key(&self.agent, |a| a.id).ok()
    }

    /// Recompute the options and the observation for the current state.
    fn refresh(&mut self) -> Observation {
        let Some(i) = self.index() else {
            self.slots = vec![None; N_ACTIONS];
            return Observation::zeros();
        };
        let sim = self.sim.as_ref().expect("reset");
        let (slots, heuristic) = actions::slot_candidates(sim, i);
        let obs = observation::encode(sim, i, &slots, self.rl.expose_utility);
        self.slots = slots;
        self.heuristic = heuristic;
        obs
    }

    /// Which actions are currently available.
    pub fn action_mask(&self) -> Vec<bool> {
        self.slots.iter().map(Option::is_some).collect()
    }

    /// The action the agent's own scripted logic would choose (baseline,
    /// imitation-learning target).
    pub fn heuristic_action(&self) -> usize {
        self.heuristic
    }

    /// The option behind an action slot (for debugging and inspection).
    pub fn slot(&self, action: usize) -> Option<&Candidate> {
        self.slots.get(action).and_then(Option::as_ref)
    }

    /// Run one macro step with `action`.
    pub fn step(&mut self, action: usize) -> Result<StepResult, EnvError> {
        if self.sim.is_none() {
            return Err(EnvError::NotReset);
        }
        if self.done {
            return Err(EnvError::EpisodeOver);
        }
        if action >= N_ACTIONS {
            return Err(EnvError::InvalidAction(action));
        }
        let chosen = self.slots[action]
            .clone()
            .ok_or(EnvError::MaskedAction(action))?;
        let rl = self.rl.clone();
        let agent_id = self.agent;
        let sim = self.sim.as_mut().expect("checked");
        if let Some(c) = sim.control.as_mut() {
            c.held = Some(chosen.clone());
            c.route_failed = false;
        }

        let mut info = StepInfo {
            agent: agent_id,
            kind: chosen.kind.name(),
            ..StepInfo::default()
        };
        let mut reward = 0.0f32;
        let mut terminated = false;
        let mut truncated = false;
        let eat_thr;
        let rest_thr;
        {
            let i = sim
                .agents
                .binary_search_by_key(&agent_id, |a| a.id)
                .map_err(|_| EnvError::NoAgent)?;
            let cx = sim.ctx(sim.tick);
            eat_thr = crate::agents::decision::eat_threshold(&sim.agents[i], &cx);
            rest_thr = crate::agents::decision::rest_threshold(&sim.agents[i], &cx);
        }
        let very_hungry = sim.config.needs.very_hungry;
        info.ended_by = "tick limit";
        for _ in 0..rl.max_ticks_per_step.max(1) {
            let before = sim.agent(agent_id).cloned().expect("alive at step start");
            sim.step();
            info.ticks += 1;
            let Some(a) = sim.agent(agent_id) else {
                // Died this tick.
                let cause = death_cause(sim, agent_id);
                info.death = cause.map(DeathCause::name);
                reward += tick_reward(&rl, very_hungry, &before, None, cause).0;
                terminated = true;
                info.ended_by = "death";
                break;
            };
            let (r, deprived, births) = tick_reward(&rl, very_hungry, &before, Some(a), None);
            reward += r;
            info.deprived_ticks += u32::from(deprived);
            info.births += births;

            if sim.tick - self.start_tick >= u64::from(rl.max_days) * TICKS_PER_DAY {
                truncated = true;
                info.ended_by = "time limit";
                break;
            }
            let route_failed = sim.control.as_ref().is_some_and(|c| c.route_failed);
            if route_failed {
                info.route_failed = true;
                info.ended_by = "no route";
                break;
            }
            // Goal finished: the engine resets completed one-shot goals.
            if !a.goal.same_intent(chosen.kind, chosen.target, chosen.join) {
                info.ended_by = "goal complete";
                break;
            }
            let at_target = a.pos == chosen.target;
            let idx = sim
                .agents
                .binary_search_by_key(&agent_id, |x| x.id)
                .expect("alive");
            let intent = sim.intents.get(idx).copied();
            let finished = match chosen.kind {
                GoalKind::Idle | GoalKind::Explore => at_target,
                GoalKind::Rest => at_target && a.energy >= sim.config.decision.rest_until,
                GoalKind::GatherFood | GoalKind::GatherWood | GoalKind::GatherStone => {
                    // Gathering stops paying when the tile is empty or hands are full.
                    at_target
                        && matches!(intent, Some(Intent::Gather(_)))
                        && a.inventory.weight(&sim.config.inventory)
                            <= before.inventory.weight(&sim.config.inventory) + 1e-6
                }
                _ => false,
            };
            if finished {
                info.ended_by = "goal complete";
                break;
            }
            if rl.interrupt_on_needs {
                let crossed_hunger = before.hunger < eat_thr && a.hunger >= eat_thr;
                let crossed_energy = before.energy >= rest_thr && a.energy < rest_thr;
                if (crossed_hunger && chosen.kind != GoalKind::Eat)
                    || (crossed_energy && chosen.kind != GoalKind::Rest)
                {
                    info.ended_by = "need interrupt";
                    break;
                }
            }
        }
        if let Some(c) = sim.control.as_mut() {
            c.held = None;
        }
        info.day = (sim.tick / TICKS_PER_DAY) as u32;
        self.done = terminated || truncated;
        let obs = if terminated {
            Observation::zeros()
        } else {
            self.refresh()
        };
        if terminated {
            self.slots = vec![None; N_ACTIONS];
        }
        Ok(StepResult {
            obs,
            reward,
            terminated,
            truncated,
            info,
        })
    }

    /// Play one full episode on `seed` with `policy(env, obs) -> action`.
    pub fn run_episode(
        &mut self,
        seed: u64,
        mut policy: impl FnMut(&AccreteEnv, &Observation) -> usize,
    ) -> Result<EpisodeSummary, EnvError> {
        let mut obs = self.reset(seed)?;
        let mut sum = EpisodeSummary {
            seed,
            agent: self.agent,
            ..EpisodeSummary::default()
        };
        loop {
            let a = policy(self, &obs);
            let r = self.step(a)?;
            sum.episode_return += r.reward;
            sum.steps += 1;
            sum.births += r.info.births;
            sum.deprived_days += r.info.deprived_ticks as f32 / TICKS_PER_DAY as f32;
            if r.terminated || r.truncated {
                sum.death = r.info.death;
                break;
            }
            obs = r.obs;
        }
        let sim = self.sim.as_ref().expect("reset");
        sum.days_alive = (sim.tick - self.start_tick) as f32 / TICKS_PER_DAY as f32;
        Ok(sum)
    }

    /// The same episode (seed, warm-up, chosen agent) with the agent left to
    /// its own scripted logic every tick, scored with the same reward. This is
    /// the reference a learned policy should be compared against.
    pub fn native_episode(&self, seed: u64) -> Result<EpisodeSummary, EnvError> {
        let mut probe = AccreteEnv::new(self.sim_config.clone(), self.rl.clone());
        probe.reset(seed)?;
        let agent = probe.agent;
        let mut sim = probe.sim.take().expect("reset");
        sim.control = None;
        let start = sim.tick;
        let very_hungry = sim.config.needs.very_hungry;
        let mut sum = EpisodeSummary {
            seed,
            agent,
            ..EpisodeSummary::default()
        };
        let end = start + u64::from(self.rl.max_days) * TICKS_PER_DAY;
        while sim.tick < end {
            let before = sim.agent(agent).cloned().expect("alive");
            sim.step();
            let after = sim.agent(agent);
            let cause = if after.is_none() {
                death_cause(&sim, agent)
            } else {
                None
            };
            let (r, deprived, births) = tick_reward(&self.rl, very_hungry, &before, after, cause);
            sum.episode_return += r;
            sum.births += births;
            sum.deprived_days += f32::from(u8::from(deprived)) / TICKS_PER_DAY as f32;
            sum.steps += 1;
            if after.is_none() {
                sum.death = cause.map(DeathCause::name);
                break;
            }
        }
        sum.days_alive = (sim.tick - start) as f32 / TICKS_PER_DAY as f32;
        Ok(sum)
    }

    /// Births by the controlled agent since the episode began.
    pub fn episode_births(&self) -> u32 {
        self.sim
            .as_ref()
            .and_then(|s| s.agent(self.agent).map(|a| a.stats.births))
            .unwrap_or(0)
            .saturating_sub(self.births_before)
    }
}

fn death_cause(sim: &Simulation, agent: u32) -> Option<DeathCause> {
    sim.deaths
        .iter()
        .rev()
        .find(|d| d.agent == agent)
        .map(|d| d.cause)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> (Config, RlConfig) {
        let mut c = Config::default();
        c.world.width = 64;
        c.world.height = 64;
        c.spawn.agents = 30;
        let rl = RlConfig {
            warmup_days: 3,
            max_days: 4,
            ..RlConfig::default()
        };
        (c, rl)
    }

    #[test]
    fn reset_then_step_respects_mask_and_is_deterministic() {
        let (c, rl) = small();
        let run = || {
            let mut env = AccreteEnv::new(c.clone(), rl.clone());
            let obs = env.reset(7).unwrap();
            assert_eq!(obs.grid.len(), GRID_CHANNELS * GRID_SIDE * GRID_SIDE);
            assert_eq!(obs.vector.len(), VECTOR_SIZE);
            assert!(obs.mask.iter().any(|&m| m));
            let mut rewards = Vec::new();
            for _ in 0..40 {
                let a = env.heuristic_action();
                assert!(env.action_mask()[a], "heuristic picked a masked action");
                let r = env.step(a).unwrap();
                rewards.push(r.reward.to_bits());
                if r.terminated || r.truncated {
                    break;
                }
            }
            (rewards, env.sim().unwrap().state_hash())
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn heuristic_policy_tracks_the_native_agent() {
        let (c, mut rl) = small();
        rl.max_days = 6;
        let mut env = AccreteEnv::new(c, rl);
        for seed in [1u64, 2, 3] {
            let h = env.run_episode(seed, |e, _| e.heuristic_action()).unwrap();
            let n = env.native_episode(seed).unwrap();
            assert_eq!(h.agent, n.agent);
            assert!(h.death.is_none() && n.death.is_none(), "{h:?} / {n:?}");
            assert!((h.days_alive - n.days_alive).abs() < 1e-3);
        }
    }

    #[test]
    fn env_toml_matches_defaults() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/rl/env.toml");
        let text = std::fs::read_to_string(path).expect("configs/rl/env.toml exists");
        assert_eq!(RlConfig::from_toml_str(&text).unwrap(), RlConfig::default());
    }

    #[test]
    fn masked_and_invalid_actions_are_rejected() {
        let (c, rl) = small();
        let mut env = AccreteEnv::new(c, rl);
        assert!(matches!(env.step(0), Err(EnvError::NotReset)));
        env.reset(3).unwrap();
        assert!(matches!(
            env.step(N_ACTIONS),
            Err(EnvError::InvalidAction(_))
        ));
        if let Some(masked) = env.action_mask().iter().position(|&m| !m) {
            assert!(matches!(env.step(masked), Err(EnvError::MaskedAction(_))));
        }
    }

    #[test]
    fn episodes_truncate_at_max_days() {
        let (c, rl) = small();
        let mut env = AccreteEnv::new(c, rl.clone());
        env.reset(11).unwrap();
        let mut steps = 0;
        loop {
            let r = env.step(env.heuristic_action()).unwrap();
            steps += 1;
            if r.terminated || r.truncated {
                if r.truncated {
                    let sim = env.sim().unwrap();
                    assert_eq!(
                        sim.tick - u64::from(rl.warmup_days) * TICKS_PER_DAY,
                        u64::from(rl.max_days) * TICKS_PER_DAY
                    );
                }
                break;
            }
            assert!(steps < 10_000);
        }
        assert!(matches!(env.step(0), Err(EnvError::EpisodeOver)));
    }
}
