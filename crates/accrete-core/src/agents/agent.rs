//! Agent state.

use serde::{Deserialize, Serialize};

use crate::agents::memory::AgentMemory;
use crate::config::{Config, TickRates};
use crate::geom::Pos;
use crate::hash::{StableHash, StableHasher};
use crate::inventory::{Inventory, Resource};
use crate::world::HomeId;

pub type AgentId = u32;

/// Personality traits in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Traits {
    /// Exploring unknown land vs. using known resources; frontier range.
    pub curiosity: f32,
    /// Food reserve kept, how early it eats and rests.
    pub caution: f32,
    /// Value of living near others; tolerance of crowding; household concern.
    pub sociability: f32,
    /// How long it works before resting; willingness to build.
    pub diligence: f32,
}

impl Traits {
    pub fn clamped(self) -> Self {
        Self {
            curiosity: self.curiosity.clamp(0.0, 1.0),
            caution: self.caution.clamp(0.0, 1.0),
            sociability: self.sociability.clamp(0.0, 1.0),
            diligence: self.diligence.clamp(0.0, 1.0),
        }
    }
}

/// Gathering skill for one resource: `aptitude + practice`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    pub aptitude: f32,
    pub practice: f32,
}

impl Skill {
    pub fn level(self) -> f32 {
        self.aptitude + self.practice
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum GoalKind {
    Idle,
    Explore,
    GatherFood,
    GatherWood,
    GatherStone,
    Eat,
    Rest,
    EstablishHome,
    BuildStorage,
    Deposit,
    Withdraw,
    Reproduce,
    ImproveTrail,
    ImproveShelter,
    Craft,
    Market,
}

impl GoalKind {
    pub const COUNT: usize = 16;
    pub const ALL: [GoalKind; GoalKind::COUNT] = [
        GoalKind::Idle,
        GoalKind::Explore,
        GoalKind::GatherFood,
        GoalKind::GatherWood,
        GoalKind::GatherStone,
        GoalKind::Eat,
        GoalKind::Rest,
        GoalKind::EstablishHome,
        GoalKind::BuildStorage,
        GoalKind::Deposit,
        GoalKind::Withdraw,
        GoalKind::Reproduce,
        GoalKind::ImproveTrail,
        GoalKind::ImproveShelter,
        GoalKind::Craft,
        GoalKind::Market,
    ];

    pub fn name(self) -> &'static str {
        match self {
            GoalKind::Idle => "idle",
            GoalKind::Explore => "explore",
            GoalKind::GatherFood => "gather-food",
            GoalKind::GatherWood => "gather-wood",
            GoalKind::GatherStone => "gather-stone",
            GoalKind::Eat => "eat",
            GoalKind::Rest => "rest",
            GoalKind::EstablishHome => "establish-home",
            GoalKind::BuildStorage => "build-storage",
            GoalKind::Deposit => "deposit",
            GoalKind::Withdraw => "withdraw",
            GoalKind::Reproduce => "reproduce",
            GoalKind::ImproveTrail => "improve-trail",
            GoalKind::ImproveShelter => "improve-shelter",
            GoalKind::Craft => "craft",
            GoalKind::Market => "market",
        }
    }

    pub fn resource(self) -> Option<Resource> {
        match self {
            GoalKind::GatherFood => Some(Resource::Food),
            GoalKind::GatherWood => Some(Resource::Wood),
            GoalKind::GatherStone => Some(Resource::Stone),
            _ => None,
        }
    }

    pub fn gather(r: Resource) -> GoalKind {
        match r {
            Resource::Food => GoalKind::GatherFood,
            Resource::Wood => GoalKind::GatherWood,
            Resource::Stone => GoalKind::GatherStone,
            Resource::Tools => GoalKind::Craft,
        }
    }
}

/// What the agent is currently trying to do, and where.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    pub kind: GoalKind,
    pub target: Pos,
    /// For `EstablishHome`: the existing home to join, if any.
    pub join: Option<HomeId>,
    pub since: u64,
    /// Ticks of work completed toward a multi-tick action (e.g. storage).
    pub work: u32,
}

impl Goal {
    pub fn idle(at: Pos, tick: u64) -> Self {
        Self {
            kind: GoalKind::Idle,
            target: at,
            join: None,
            since: tick,
            work: 0,
        }
    }

    pub fn same_intent(&self, kind: GoalKind, target: Pos, join: Option<HomeId>) -> bool {
        self.kind == kind && self.target == target && self.join == join
    }
}

/// A planned route; `steps[next..]` remain to be walked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub target: Pos,
    pub steps: Vec<Pos>,
    pub next: u32,
    pub planned_tick: u64,
    pub cost: u32,
}

impl Route {
    pub fn remaining(&self) -> &[Pos] {
        &self.steps[self.next as usize..]
    }
    pub fn done(&self) -> bool {
        self.next as usize >= self.steps.len()
    }
}

/// Where the agent last decided it would prefer to live (refreshed periodically).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HomePlan {
    pub target: Pos,
    pub join: Option<HomeId>,
    pub score: f32,
    /// Score of the current home (or `None` when homeless).
    pub current: Option<f32>,
    /// Perceived food potential per person around the current home (or the
    /// agent's position when homeless); below 1.0 the land feels crowded.
    pub home_land: f32,
    pub evaluated_tick: u64,
}

/// Rolling counters used by pathology detectors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct PathologyState {
    /// Ticks of the most recent goal switches (ring buffer).
    pub switches: Vec<u64>,
    pub last_progress_tick: u64,
    pub stuck_flagged: bool,
    pub idle_today: u32,
}

/// Lifetime statistics (for evaluation, inspection and metrics).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AgentStats {
    pub food_gathered: f32,
    pub food_eaten: f32,
    pub tiles_moved: u32,
    pub ticks_moving: u32,
    pub ticks_deprived: u32,
    pub births: u32,
    pub homes_founded: u32,
    pub home_moves: u32,
    pub goal_switches: u32,
    pub trades: u32,
    pub roads_built: u32,
    pub shelters_built: u32,
    pub tips_received: u32,
    pub tools_crafted: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub pos: Pos,
    pub hunger: f32,
    pub energy: f32,
    pub health: f32,
    pub frailty: f32,
    pub age_ticks: u64,
    pub born_tick: u64,
    pub last_birth_tick: Option<u64>,
    pub next_reproduce_try: u64,
    pub parent: Option<AgentId>,
    pub inventory: Inventory,
    pub movement_points: u32,
    pub memory: AgentMemory,
    pub traits: Traits,
    /// Indexed by `Resource::index()`.
    pub skills: [Skill; 3],
    /// Crafting skill (A.26): aptitude plus practice, like gathering skills.
    pub craft: Skill,
    pub goal: Goal,
    pub route: Option<Route>,
    pub home: Option<HomeId>,
    pub home_since: u64,
    /// The home the agent grew up in, if born in the simulation (kin ties).
    pub birth_home: Option<(HomeId, Pos)>,
    pub home_plan: Option<HomePlan>,
    pub last_observe_tick: u64,
    pub moved: bool,
    /// Frailty accumulated from crowding (tracked to explain deaths).
    pub frailty_crowding: f32,
    pub stats: AgentStats,
    pub pathology: PathologyState,
    /// Bit per resource gathered today (practice decays for the others).
    pub practiced_today: u8,
    /// Experienced foraging return: food gathered per tick spent on food
    /// trips (travel + gathering), as an exponential moving average over days.
    pub forage_rate: f32,
    pub forage_today_food: f32,
    pub forage_today_ticks: u32,
    /// Last tick this agent swapped knowledge with someone.
    pub last_share_tick: u64,
    /// Derived cache of exploration rays (see `decision::frontier_targets`),
    /// valid while position and known-tile count are unchanged. Not state:
    /// never serialized or hashed.
    #[serde(skip)]
    pub frontier_cache: Option<FrontierCache>,
    /// Derived memo of believed line costs (A.31). Not state: never
    /// serialized or hashed.
    #[serde(skip)]
    pub line_memo: crate::agents::pathfinding::MemoCell,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct FrontierCache {
    pub pos: Pos,
    pub known: u32,
    pub targets: Vec<(Pos, f32)>,
}

impl Agent {
    pub fn is_juvenile(&self, r: &TickRates) -> bool {
        self.age_ticks < r.maturity_ticks
    }

    /// Piecewise linear: 0 before elder age, then `age_decline_per_day` per day.
    pub fn age_decline(&self, r: &TickRates) -> f32 {
        if self.age_ticks < r.elder_ticks {
            0.0
        } else {
            (self.age_ticks - r.elder_ticks) as f32 * r.age_decline_per_tick
        }
    }

    /// `clamp(1 - age_decline - frailty, 0, 1)`.
    pub fn health_ceiling(&self, r: &TickRates) -> f32 {
        (1.0 - self.age_decline(r) - self.frailty).clamp(0.0, 1.0)
    }

    pub fn carry_capacity(&self, c: &Config, r: &TickRates) -> f32 {
        if self.is_juvenile(r) {
            c.inventory.carry_juvenile
        } else {
            c.inventory.carry_adult
        }
    }

    pub fn daily_food_need(&self, c: &Config, r: &TickRates) -> f32 {
        let hunger = if self.is_juvenile(r) {
            c.needs.hunger_per_day * c.needs.juvenile_hunger_factor
        } else {
            c.needs.hunger_per_day
        };
        hunger / c.needs.hunger_per_food
    }

    pub fn gather_rate(&self, res: Resource, c: &Config, r: &TickRates) -> f32 {
        let base = match res {
            Resource::Food => c.skills.base_food,
            Resource::Wood => c.skills.base_wood,
            Resource::Stone => c.skills.base_stone,
            Resource::Tools => return 0.0,
        };
        let juvenile = if self.is_juvenile(r) {
            c.life.juvenile_gather_factor
        } else {
            1.0
        };
        base * self.skills[res.index()].level() * juvenile
    }

    pub fn age_days(&self) -> f32 {
        self.age_ticks as f32 / crate::config::TICKS_PER_DAY as f32
    }
}

impl StableHash for Agent {
    fn stable_hash(&self, h: &mut StableHasher) {
        h.u32(self.id);
        self.pos.stable_hash(h);
        h.f32(self.hunger);
        h.f32(self.energy);
        h.f32(self.health);
        h.f32(self.frailty);
        h.u64(self.age_ticks);
        h.u64(self.born_tick);
        h.opt_u64(self.last_birth_tick);
        h.u64(self.next_reproduce_try);
        h.opt_u32(self.parent);
        self.inventory.stable_hash(h);
        h.u32(self.movement_points);
        self.memory.stable_hash(h);
        h.f32(self.traits.curiosity);
        h.f32(self.traits.caution);
        h.f32(self.traits.sociability);
        h.f32(self.traits.diligence);
        for s in &self.skills {
            h.f32(s.aptitude);
            h.f32(s.practice);
        }
        h.u8(self.goal.kind as u8);
        self.goal.target.stable_hash(h);
        h.opt_u32(self.goal.join);
        h.u64(self.goal.since);
        h.u32(self.goal.work);
        match &self.route {
            None => h.u8(0),
            Some(r) => {
                h.u8(1);
                r.target.stable_hash(h);
                h.len(r.steps.len());
                for s in &r.steps {
                    s.stable_hash(h);
                }
                h.u32(r.next);
                h.u64(r.planned_tick);
                h.u32(r.cost);
            }
        }
        h.opt_u32(self.home);
        h.u64(self.home_since);
        match self.birth_home {
            None => h.u8(0),
            Some((id, p)) => {
                h.u8(1);
                h.u32(id);
                p.stable_hash(h);
            }
        }
        h.u64(self.last_observe_tick);
        h.bool(self.moved);
        h.f32(self.frailty_crowding);
        h.u8(self.practiced_today);
        h.f32(self.forage_rate);
        h.f32(self.craft.aptitude);
        h.f32(self.craft.practice);
        h.f32(self.forage_today_food);
        h.u32(self.forage_today_ticks);
        h.u64(self.last_share_tick);
    }
}
