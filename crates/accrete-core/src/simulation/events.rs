//! Event log, death records, and the resource ledger.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::agents::agent::AgentId;
use crate::geom::Pos;
use crate::inventory::{Inventory, Resource};
use crate::world::HomeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeathCause {
    /// Health reached 0 from unmet needs.
    Deprivation,
    /// The health ceiling reached 0 from age and accumulated frailty.
    OldAge,
}

impl DeathCause {
    pub fn name(self) -> &'static str {
        match self {
            DeathCause::Deprivation => "deprivation",
            DeathCause::OldAge => "old age",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeathRecord {
    pub agent: AgentId,
    pub tick: u64,
    pub pos: Pos,
    pub cause: DeathCause,
    pub age_days: f32,
    pub age_decline: f32,
    pub frailty: f32,
    pub frailty_from_crowding: f32,
    pub days_deprived: f32,
    pub home: Option<HomeId>,
    pub children: u32,
    /// Human-readable explanation, e.g. "ceiling reached 0: 70% age, 30% frailty".
    pub explanation: String,
    /// Died of deprivation while knowing reachable food (pathology).
    pub starved_beside_food: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    Born {
        parent: AgentId,
        home: HomeId,
        food: f32,
    },
    BirthBlockedByCap {
        parent: AgentId,
    },
    PopulationCapReached,
    BirthRollFailed,
    Died {
        cause: DeathCause,
    },
    Dropped {
        goods: Inventory,
        into_home: Option<HomeId>,
    },
    FoundedHome {
        home: HomeId,
    },
    JoinedHome {
        home: HomeId,
    },
    LeftHome {
        home: HomeId,
    },
    HomeVacant {
        home: HomeId,
    },
    BuiltStorage {
        home: HomeId,
    },
    Gathered {
        resource: Resource,
        amount: f32,
    },
    Ate {
        amount: f32,
    },
    Deposited {
        home: HomeId,
        goods: Inventory,
    },
    Withdrew {
        home: HomeId,
        food: f32,
    },
    RouteFailed {
        target: Pos,
    },
    Traded {
        partner: AgentId,
        gave: Resource,
        gave_qty: f32,
        got: Resource,
        got_qty: f32,
    },
    RoadBuilt,
    RoadWornAway,
    ShelterImproved {
        home: HomeId,
        level: u8,
    },
    DroughtStarted {
        radius: u32,
        days: u32,
        severity: f32,
        scripted: bool,
    },
    DroughtEnded,
    Pathology {
        kind: PathologyKind,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathologyKind {
    Oscillating,
    Stuck,
    StarvedBesideFood,
    Idle,
}

impl PathologyKind {
    pub const ALL: [PathologyKind; 4] = [
        PathologyKind::Oscillating,
        PathologyKind::Stuck,
        PathologyKind::StarvedBesideFood,
        PathologyKind::Idle,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PathologyKind::Oscillating => "oscillating",
            PathologyKind::Stuck => "stuck",
            PathologyKind::StarvedBesideFood => "starved-beside-food",
            PathologyKind::Idle => "idle",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub tick: u64,
    pub agent: Option<AgentId>,
    pub pos: Pos,
    pub kind: EventKind,
}

/// Rolling log of recent events (oldest dropped first).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EventLog {
    pub capacity: u32,
    pub events: VecDeque<Event>,
    pub total: u64,
}

impl EventLog {
    pub fn new(capacity: u32) -> Self {
        Self {
            capacity,
            events: VecDeque::new(),
            total: 0,
        }
    }

    pub fn push(&mut self, tick: u64, agent: Option<AgentId>, pos: Pos, kind: EventKind) {
        self.total += 1;
        if self.capacity == 0 {
            return;
        }
        if self.events.len() as u32 >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            tick,
            agent,
            pos,
            kind,
        });
    }

    pub fn for_agent(&self, id: AgentId) -> impl Iterator<Item = &Event> {
        self.events.iter().filter(move |e| e.agent == Some(id))
    }
}

/// Accounting of every resource change, accumulated in `f64` in a fixed
/// order. Conservation: `world + carried == initial + regrown - eaten -
/// spoiled - consumed_by_building` for each resource.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    pub initial: [f64; 3],
    pub regrown: [f64; 3],
    pub gathered: [f64; 3],
    pub eaten: f64,
    pub spoiled: [f64; 3],
    pub construction: [f64; 3],
    /// Moved between holders (agents, chests, piles); conserved.
    pub transferred: [f64; 3],
}

impl Ledger {
    pub fn expected_total(&self, r: Resource) -> f64 {
        let i = r.index();
        let eaten = if r == Resource::Food { self.eaten } else { 0.0 };
        self.initial[i] + self.regrown[i] - eaten - self.spoiled[i] - self.construction[i]
    }
}
