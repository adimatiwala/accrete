//! Fixed-tick reinforcement-learning task built on the ordinary simulation.

use std::ops::Range;

use crate::agents::agent::{Agent, GoalKind};
use crate::agents::decision::{self, Candidate};
use crate::config::{ScriptedDrought, TICKS_PER_DAY};
use crate::hash::mix64;
use crate::inventory::Resource;
use crate::scenario::Scenario;
use crate::{Config, Pos, Simulation};

pub const TASK_VERSION: u32 = 0;
pub const MAX_DAYS: u32 = 365;
pub const STATE_FIELDS: [&str; 18] = [
    "hunger",
    "energy",
    "health",
    "food",
    "wood",
    "stone",
    "tools",
    "food_room",
    "home_dx",
    "home_dy",
    "home_food",
    "chest_room",
    "at_home",
    "nearby_agents",
    "household_size",
    "hour",
    "elapsed",
    "path_level",
];
pub const TARGET_FIELDS: [&str; 5] = ["dx", "dy", "amount", "age", "travel"];
pub const STATE_SIZE: usize = STATE_FIELDS.len();
pub const TARGET_SIZE: usize = TARGET_FIELDS.len();
pub const ACTIONS: [&str; 17] = [
    "wait",
    "eat",
    "rest",
    "continue",
    "return_home",
    "deposit",
    "withdraw",
    "food_0",
    "food_1",
    "food_2",
    "food_3",
    "wood_0",
    "wood_1",
    "market_0",
    "market_1",
    "scout_0",
    "scout_1",
];

const WAIT: usize = 0;
const EAT: usize = 1;
const REST: usize = 2;
const CONTINUE: usize = 3;
const HOME: usize = 4;
const DEPOSIT: usize = 5;
const WITHDRAW: usize = 6;
const FOOD: Range<usize> = 7..11;
const WOOD: Range<usize> = 11..13;
const MARKET: Range<usize> = 13..15;
const SCOUT: Range<usize> = 15..17;
const WIDTH: i32 = 17;
const HEIGHT: i32 = 9;
const FOCAL: u32 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weather {
    Mixed,
    Steady,
    Drought,
}

impl Weather {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "mixed" => Ok(Self::Mixed),
            "steady" => Ok(Self::Steady),
            "drought" => Ok(Self::Drought),
            _ => Err(format!(
                "unknown weather {value:?}; use mixed, steady or drought"
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    pub state: [f32; STATE_SIZE],
    pub targets: [[f32; TARGET_SIZE]; ACTIONS.len()],
    pub mask: [bool; ACTIONS.len()],
}

impl Default for Observation {
    fn default() -> Self {
        Self {
            state: [0.0; STATE_SIZE],
            targets: [[0.0; TARGET_SIZE]; ACTIONS.len()],
            mask: [false; ACTIONS.len()],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Score {
    pub ticks: u64,
    pub alive: usize,
    pub focal_alive: bool,
    pub deprived_hours: u64,
    pub focal_trades: u64,
    pub tiles_moved: u32,
    pub home_food: f32,
}

pub struct Transition {
    pub reward: f32,
    pub terminated: bool,
    pub truncated: bool,
    pub masked: bool,
}

#[derive(Clone, Copy)]
struct Target {
    kind: GoalKind,
    pos: Pos,
    amount: f32,
    seen: u64,
}

type Options = [Option<Target>; ACTIONS.len()];

pub struct Household {
    sim: Simulation,
    cohort: Vec<u32>,
    options: Options,
    observation: Observation,
    score: Score,
    horizon: u64,
    done: bool,
}

impl Household {
    pub fn new(seed: u64, weather: Weather, days: u32, check: bool) -> Result<Self, String> {
        if !(1..=MAX_DAYS).contains(&days) {
            return Err(format!("days must be in 1..={MAX_DAYS}"));
        }
        let sim = build_household(seed, weather, check)?;
        let cohort = sim.world.homes[0].residents.clone();
        let mut env = Self {
            sim,
            cohort,
            options: [None; ACTIONS.len()],
            observation: Observation::default(),
            score: Score::default(),
            horizon: u64::from(days) * TICKS_PER_DAY,
            done: false,
        };
        env.score.alive = env.cohort.len();
        env.score.focal_alive = true;
        env.score.home_food = env.sim.world.homes[0].stored().food;
        env.prepare();
        Ok(env)
    }

    pub fn observation(&self) -> &Observation {
        &self.observation
    }

    /// Privileged aggregate outcomes for an evaluator. Keep these out of policy inputs.
    pub fn score(&self) -> Score {
        self.score
    }

    pub fn step(&mut self, action: usize) -> Result<Transition, String> {
        if self.done {
            return Err("episode is done; reset before stepping again".into());
        }
        let Some(target) = self.options.get(action) else {
            return Err(format!("action {action} outside the action space"));
        };
        let masked = target.is_none();
        let target = target.or(self.options[WAIT]).expect("wait is always legal");
        let tick = self.sim.tick;
        self.sim
            .finish_tick(&[(FOCAL, Candidate::new(target.kind, target.pos))]);
        self.score.focal_trades += self
            .sim
            .trade_log
            .iter()
            .rev()
            .take_while(|trade| trade.tick == tick)
            .filter(|trade| trade.a == FOCAL || trade.b == FOCAL)
            .count() as u64;
        let focal = self.sim.agent(FOCAL);
        self.score.tiles_moved = focal
            .or_else(|| {
                self.sim
                    .graveyard
                    .iter()
                    .rev()
                    .find(|agent| agent.id == FOCAL)
            })
            .map_or(self.score.tiles_moved, |agent| agent.stats.tiles_moved);
        let terminated = focal.is_none();
        self.score.focal_alive = !terminated;
        self.score.ticks += 1;
        self.score.home_food = self.sim.world.homes[0].stored().food;
        let mut welfare = 0.0;
        self.score.alive = 0;
        for &id in &self.cohort {
            if let Some(agent) = self.sim.agent(id) {
                self.score.alive += 1;
                welfare += 0.45 * (1.0 - agent.hunger) + 0.25 * agent.energy + 0.25 * agent.health;
                if agent.hunger >= self.sim.config.needs.very_hungry || agent.energy <= 0.0 {
                    self.score.deprived_hours += 1;
                }
            } else {
                self.score.deprived_hours += 1;
            }
        }
        let reward = welfare / self.cohort.len() as f32
            + 0.05 * (self.score.home_food / 12.0).clamp(0.0, 1.0)
            - if terminated { 2.0 } else { 0.0 };
        let truncated = !terminated && self.score.ticks >= self.horizon;
        self.done = terminated || truncated;
        if terminated {
            self.options = [None; ACTIONS.len()];
            self.observation = Observation::default();
            self.observation.mask[WAIT] = true;
        } else {
            // Time-limit truncation still returns the next observation for bootstrapping.
            self.prepare();
        }
        Ok(Transition {
            reward,
            terminated,
            truncated,
            masked,
        })
    }

    /// A reserve-and-forage baseline acting through exactly the same options.
    pub fn reserve_action(&self) -> usize {
        let Some(a) = self.sim.agent(FOCAL) else {
            return WAIT;
        };
        let state = &self.observation.state;
        let mask = &self.observation.mask;
        if a.hunger > 0.35 && mask[EAT] {
            return EAT;
        }
        if a.energy < 0.6 {
            return REST;
        }
        if a.inventory.food < 1.0 && mask[WITHDRAW] {
            return WITHDRAW;
        }
        if a.inventory.food > 4.0 && state[10] < 12.0 / 50.0 {
            if mask[DEPOSIT] {
                return DEPOSIT;
            }
            if mask[HOME] {
                return HOME;
            }
        }
        if a.inventory.food < 7.0 {
            let cx = self.sim.ctx(self.sim.tick);
            let yield_rate = |target: Target| {
                let travel = decision::travel_ticks(a, &cx, a.pos, target.pos).min(24.0);
                let age = self.sim.tick.saturating_sub(target.seen).min(168) as f32;
                target.amount.min(8.0) / ((2.0 + travel) * (1.0 + age / 96.0))
            };
            if let Some(index) = FOOD.filter(|&index| mask[index]).max_by(|&x, &y| {
                yield_rate(self.options[x].unwrap())
                    .total_cmp(&yield_rate(self.options[y].unwrap()))
            }) {
                return index;
            }
            if a.inventory.food < 2.0 {
                if let Some(index) = MARKET.chain(SCOUT).find(|&index| mask[index]) {
                    return index;
                }
            }
        }
        REST
    }

    fn prepare(&mut self) {
        self.sim.begin_tick();
        let agent = self.sim.agent(FOCAL).expect("live focal agent");
        (self.options, self.observation) = decision_frame(&self.sim, agent);
    }
}

/// Play the same scenario with policy control over every living resident.
pub struct LearnedWorld {
    sim: Simulation,
    frames: Vec<(u32, Options, Observation)>,
}

impl LearnedWorld {
    pub fn new(seed: u64, weather: Weather, check: bool) -> Result<Self, String> {
        let mut sim = build_household(seed, weather, check)?;
        let origin = sim.world.homes[0].pos;
        let market = sim.world.homes[1].pos;
        for agent in &mut sim.agents {
            if agent.id == FOCAL {
                continue;
            }
            for x in origin.x.min(market.x)..=origin.x.max(market.x) {
                agent.memory.learn(Pos::new(x, market.y));
            }
            agent
                .memory
                .note_trade_spot(market, 3.0, 0, sim.config.markets.slots as usize);
        }
        let mut world = Self {
            sim,
            frames: Vec::new(),
        };
        world.prepare();
        Ok(world)
    }

    /// Local observations in agent ID order, also the order expected by `step`.
    pub fn observations(&self) -> impl Iterator<Item = (u32, &Observation)> {
        self.frames.iter().map(|(id, _, obs)| (*id, obs))
    }

    /// Renderer-only state. A policy receives its own observation instead.
    pub fn simulation(&self) -> &Simulation {
        &self.sim
    }

    pub fn step(&mut self, actions: &[usize]) -> Result<(), String> {
        if self.frames.is_empty() {
            return Err("no living residents remain".into());
        }
        if actions.len() != self.frames.len() {
            return Err(format!(
                "expected one action for each of {} residents",
                self.frames.len()
            ));
        }
        let mut proposals = Vec::with_capacity(actions.len());
        for ((id, options, _), &action) in self.frames.iter().zip(actions) {
            let Some(&Some(target)) = options.get(action) else {
                return Err(format!("action {action} is unavailable for resident {id}"));
            };
            proposals.push((*id, Candidate::new(target.kind, target.pos)));
        }
        self.sim.finish_tick(&proposals);
        self.prepare();
        Ok(())
    }

    fn prepare(&mut self) {
        self.sim.begin_tick();
        self.frames.clear();
        self.frames.extend(self.sim.agents.iter().map(|agent| {
            let (options, observation) = decision_frame(&self.sim, agent);
            (agent.id, options, observation)
        }));
    }
}

fn decision_frame(sim: &Simulation, a: &Agent) -> (Options, Observation) {
    let cx = sim.ctx(sim.tick);
    let current = |kind| Target {
        kind,
        pos: a.pos,
        amount: 0.0,
        seen: sim.tick,
    };
    let reserve = decision::reserve_target(a, &cx);
    let food_room = carrying_room(a, sim, Resource::Food);
    let mut options = [None; ACTIONS.len()];
    options[WAIT] = Some(current(GoalKind::Idle));
    options[EAT] = (a.inventory.food > 0.0).then(|| current(GoalKind::Eat));
    options[REST] = Some(current(GoalKind::Rest));
    if a.goal.target != a.pos && a.route.is_some() {
        options[CONTINUE] = Some(Target {
            pos: a.goal.target,
            ..current(a.goal.kind)
        });
    }
    let home = cx.home(a);
    let home_pos = home.map_or(a.pos, |h| h.pos);
    let at_home = home.is_some() && home_pos == a.pos;
    let home_food = if at_home {
        home.map_or(0.0, |h| h.stored().food)
    } else {
        a.memory.home_food_seen
    };
    let home_room = if at_home {
        home.map_or(0.0, |h| {
            h.chests
                .iter()
                .map(|c| {
                    (sim.config.inventory.chest_capacity - c.weight(&sim.config.inventory)).max(0.0)
                })
                .sum()
        })
    } else {
        a.memory.home_room_seen
    };
    if at_home {
        options[DEPOSIT] =
            (a.inventory.food > reserve && home_room > 0.0).then(|| current(GoalKind::Deposit));
        options[WITHDRAW] = (a.inventory.food < reserve && home_food > 0.0 && food_room > 0.0)
            .then(|| current(GoalKind::Withdraw));
    } else if home.is_some() {
        options[HOME] = Some(Target {
            pos: home_pos,
            ..current(GoalKind::Explore)
        });
    }

    for (slots, resource, kind) in [
        (FOOD, Resource::Food, GoalKind::GatherFood),
        (WOOD, Resource::Wood, GoalKind::GatherWood),
    ] {
        if carrying_room(a, sim, resource) > 0.0 {
            let targets = a.memory.resources.iter().filter_map(|m| {
                (m.resource == resource && m.amount > 0.0 && sim.tick >= m.unreachable_until)
                    .then_some(Target {
                        kind,
                        pos: m.pos,
                        amount: m.amount,
                        seen: m.seen_tick,
                    })
            });
            nearest(&mut options, slots, a.pos, targets);
        }
    }
    nearest(
        &mut options,
        MARKET,
        a.pos,
        a.memory.trade_spots.iter().map(|s| Target {
            kind: GoalKind::Market,
            pos: s.pos,
            amount: s.score,
            seen: s.seen_tick,
        }),
    );
    nearest(
        &mut options,
        SCOUT,
        a.pos,
        decision::frontier_targets(a, &cx)
            .into_iter()
            .map(|(pos, novelty)| Target {
                kind: GoalKind::Explore,
                pos,
                amount: novelty,
                seen: sim.tick,
            }),
    );

    let nearby = sim
        .agents
        .iter()
        .filter(|other| {
            other.id != a.id
                && other.pos.chebyshev(a.pos) <= 1
                && sim.world.vis.is_visible(&sim.world, a.pos, other.pos)
        })
        .count();
    let mut observation = Observation {
        state: [
            a.hunger,
            a.energy,
            a.health,
            fraction(a.inventory.food, 20.0),
            fraction(a.inventory.wood, 20.0),
            fraction(a.inventory.stone, 20.0),
            fraction(a.inventory.tools, 5.0),
            fraction(food_room, 20.0),
            (home_pos.x - a.pos.x) as f32 / WIDTH as f32,
            (home_pos.y - a.pos.y) as f32 / HEIGHT as f32,
            fraction(home_food, 50.0),
            fraction(home_room, sim.config.inventory.chest_capacity),
            if at_home { 1.0 } else { 0.0 },
            fraction(nearby as f32, 8.0),
            fraction(home.map_or(0, |h| h.residents.len()) as f32, 8.0),
            (sim.tick % TICKS_PER_DAY) as f32 / (TICKS_PER_DAY - 1) as f32,
            fraction(sim.tick as f32, (21 * TICKS_PER_DAY) as f32),
            sim.world.tile(a.pos).path_level as u8 as f32 / 2.0,
        ],
        ..Observation::default()
    };
    for (index, target) in options.iter().enumerate() {
        if let Some(target) = target {
            observation.mask[index] = true;
            observation.targets[index] = [
                (target.pos.x - a.pos.x) as f32 / WIDTH as f32,
                (target.pos.y - a.pos.y) as f32 / HEIGHT as f32,
                fraction(target.amount, 30.0),
                fraction(sim.tick.saturating_sub(target.seen) as f32, 168.0),
                fraction(decision::travel_ticks(a, &cx, a.pos, target.pos), 24.0),
            ];
        }
    }
    (options, observation)
}

fn nearest(
    options: &mut [Option<Target>],
    slots: Range<usize>,
    from: Pos,
    targets: impl Iterator<Item = Target>,
) {
    let mut targets: Vec<_> = targets.collect();
    targets.sort_by_key(|t| (from.chebyshev(t.pos), t.pos));
    for (slot, target) in options[slots].iter_mut().zip(targets) {
        *slot = Some(target);
    }
}

fn carrying_room(agent: &Agent, sim: &Simulation, resource: Resource) -> f32 {
    agent.inventory.room_for(
        resource,
        agent.carry_capacity(&sim.config, &sim.rates),
        &sim.config.inventory,
    )
}

fn fraction(amount: f32, scale: f32) -> f32 {
    (amount / scale).clamp(0.0, 1.0)
}

fn build_household(seed: u64, weather: Weather, check: bool) -> Result<Simulation, String> {
    let mut scenario = Scenario::from_toml_str(include_str!("rl/household.toml"))?;
    scenario.seed = seed;
    let variant = mix64(seed);
    let mirror = variant & 1 != 0;
    let x = |x| if mirror { WIDTH - 1 - x } else { x };
    if mirror {
        scenario.map = scenario
            .map
            .lines()
            .map(|row| row.chars().rev().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        for resource in scenario.food.iter_mut().chain(&mut scenario.wood) {
            resource.at[0] = x(resource.at[0]);
        }
        for home in &mut scenario.home {
            home.at[0] = x(home.at[0]);
        }
        for agent in &mut scenario.agent {
            agent.at[0] = x(agent.at[0]);
        }
    }
    let variation = ((variant >> 5) % 7) as f32;
    scenario.food[0].amount += variation - 3.0;
    scenario.home[0].chests[0] += (variation - 3.0) * 0.5;
    scenario.agent[3].food += variation - 3.0;

    let mut c = Config::default();
    c.world.width = WIDTH as u32;
    c.world.height = HEIGHT as u32;
    c.sim.check_invariants = check;
    c.life.reproduce_chance = 0.0;
    c.tools.enabled = true;
    c.markets.enabled = true;
    c.infrastructure.public_weight = 1.0;
    c.resources.food_regrowth = 0.23;
    c.weather.random_droughts = false;
    if weather == Weather::Drought || (weather == Weather::Mixed && variant & 2 != 0) {
        c.weather.scripted.push(ScriptedDrought {
            start_day: 3 + ((variant >> 8) % 4) as u32,
            days: 7 + ((variant >> 10) % 4) as u32,
            center: [x(4), 4],
            radius: 3,
            severity: 0.82 + ((variant >> 13) % 5) as f32 * 0.03,
        });
    }
    c.validate()?;
    let mut sim = scenario.build(&c)?;
    sim.threads = 1;
    for patch in &scenario.food {
        sim.world
            .tile_mut(Pos::new(patch.at[0], patch.at[1]))
            .fertility = 1.0;
    }
    for col in 3..=11 {
        sim.agents[0].memory.learn(Pos::new(x(col), 4));
    }
    // The scenario starts with a recent account of a successful trade. The
    // policy can visit the location, and future memories come from play.
    sim.agents[0]
        .memory
        .note_trade_spot(Pos::new(x(11), 4), 3.0, 0, c.markets.slots as usize);
    Ok(sim)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::Intent;

    #[test]
    fn learned_world_requires_and_executes_one_action_per_resident() {
        let mut world = LearnedWorld::new(1, Weather::Drought, true).unwrap();
        let ids: Vec<_> = world.observations().map(|(id, _)| id).collect();
        assert_eq!(ids.len(), 5);
        assert!(world.step(&[WAIT]).is_err());
        assert!(world.step(&[WAIT, EAT, REST, REST, ACTIONS.len()]).is_err());
        assert_eq!(world.sim.tick, 0);
        world.step(&[WAIT, EAT, REST, WAIT, REST]).unwrap();
        assert_eq!(world.sim.tick, 1);
        for (id, intent) in ids.into_iter().zip([
            Intent::Idle,
            Intent::Eat,
            Intent::Rest,
            Intent::Idle,
            Intent::Rest,
        ]) {
            assert_eq!(world.sim.intent_of(id), Some(intent));
        }
    }
}
