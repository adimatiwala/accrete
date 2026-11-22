//! Making the world legible to someone who has never seen it (A.28).
//!
//! Everything here is presentation: it reads the simulation (state and the
//! event log) and never changes it. It provides:
//! * stable, pronounceable **names** for people and settlements (from ids
//!   and terrain, via a stateless hash);
//! * a **narrator** that turns notable events into a short story ticker;
//! * **roles** (forager, builder, crafter, explorer, settler, resting) that
//!   colour people by what they are doing rather than by internal goal ids;
//! * short-lived **effects** (trade arcs, birth pulses, new-home rings);
//! * **motion trails** and smooth movement between ticks;
//! * a plain-language **biography** of a selected person.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use accrete_core::Simulation;
use accrete_core::agents::agent::{Agent, GoalKind};
use accrete_core::geom::Pos;
use accrete_core::simulation::events::{DeathCause, EventKind};
use accrete_core::simulation::settlement::SizeClass;
use accrete_core::world::Biome;
use macroquad::prelude::*;

const ONSETS: [&str; 16] = [
    "", "b", "d", "f", "k", "l", "m", "n", "r", "s", "t", "v", "z", "sh", "th", "br",
];
const VOWELS: [&str; 8] = ["a", "e", "i", "o", "u", "ai", "ea", "ou"];
const CODAS: [&str; 8] = ["", "n", "r", "l", "s", "m", "th", "k"];

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn word(seed: u64, syllables: u32) -> String {
    let mut h = mix(seed);
    let mut s = String::new();
    for i in 0..syllables {
        s.push_str(ONSETS[(h % 16) as usize]);
        h /= 16;
        s.push_str(VOWELS[(h % 8) as usize]);
        h /= 8;
        if i + 1 == syllables {
            s.push_str(CODAS[(h % 8) as usize]);
        }
        h /= 8;
    }
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}

/// A person's name, stable for the run (seeded by world seed and id).
pub fn person_name(world_seed: u64, id: u32) -> String {
    let h = mix(world_seed ^ (u64::from(id) << 20) ^ 0xA11CE);
    word(h, 2 + (h % 2) as u32)
}

/// A settlement's name: a stem plus a suffix from the terrain at its centre.
pub fn settlement_name(sim: &Simulation, id: u32, centre: Pos) -> String {
    let w = &sim.world;
    let suffix = if !w.contains(centre) {
        "field"
    } else {
        let t = w.tile(centre);
        if t.river_distance <= 2 {
            "ford"
        } else {
            match t.biome {
                Biome::Forest => "wood",
                Biome::Hills => "hill",
                Biome::Mountains => "crag",
                _ => "field",
            }
        }
    };
    let stem = word(mix(w.seed ^ u64::from(id) ^ 0x5E77), 1);
    format!("{stem}{suffix}")
}

/// What someone is doing, in plain words, for colouring and the legend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Forager,
    Builder,
    Crafter,
    Explorer,
    Settler,
    Family,
    Resting,
}

impl Role {
    pub const ALL: [Role; 7] = [
        Role::Forager,
        Role::Builder,
        Role::Crafter,
        Role::Explorer,
        Role::Settler,
        Role::Family,
        Role::Resting,
    ];

    pub fn of(a: &Agent) -> Role {
        match a.goal.kind {
            GoalKind::GatherFood | GoalKind::Eat | GoalKind::Withdraw | GoalKind::Deposit => {
                Role::Forager
            }
            GoalKind::GatherWood
            | GoalKind::GatherStone
            | GoalKind::BuildStorage
            | GoalKind::ImproveTrail
            | GoalKind::ImproveShelter => Role::Builder,
            GoalKind::Craft => Role::Crafter,
            GoalKind::Explore => Role::Explorer,
            GoalKind::EstablishHome => Role::Settler,
            GoalKind::Reproduce => Role::Family,
            GoalKind::Rest | GoalKind::Idle => Role::Resting,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::Forager => "gathering food",
            Role::Builder => "building / gathering materials",
            Role::Crafter => "crafting tools",
            Role::Explorer => "exploring",
            Role::Settler => "moving house",
            Role::Family => "starting a family",
            Role::Resting => "resting",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Role::Forager => Color::from_rgba(255, 170, 60, 255),
            Role::Builder => Color::from_rgba(190, 120, 70, 255),
            Role::Crafter => Color::from_rgba(80, 230, 255, 255),
            Role::Explorer => Color::from_rgba(255, 240, 90, 255),
            Role::Settler => Color::from_rgba(240, 110, 255, 255),
            Role::Family => Color::from_rgba(255, 90, 140, 255),
            Role::Resting => Color::from_rgba(150, 165, 200, 200),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Fx {
    TradeArc { a: u32, b: u32 },
    Pulse { at: Pos, color: Color },
    Ring { at: Pos, color: Color },
}

pub struct Story {
    /// Event-log position already narrated.
    seen_total: u64,
    pub lines: VecDeque<(u32, String, Color)>,
    pub effects: Vec<(f64, Fx)>,
    /// Recent tile positions per agent (newest last).
    pub trails: BTreeMap<u32, VecDeque<Pos>>,
    /// Positions before the latest step, for smooth movement.
    pub prev: BTreeMap<u32, Pos>,
    traded_pairs: BTreeSet<(u32, u32)>,
    named_settlements: BTreeSet<u32>,
    day_trades: u32,
    day_births: u32,
    last_day: u32,
    pub enabled: bool,
    pub kin_lines: bool,
    pub intro: bool,
}

const MAX_LINES: usize = 9;
const TRAIL_LEN: usize = 8;
const FX_SECONDS: f64 = 2.5;

impl Story {
    pub fn new(sim: &Simulation) -> Self {
        Self {
            seen_total: sim.events.total,
            lines: VecDeque::new(),
            effects: Vec::new(),
            trails: BTreeMap::new(),
            prev: sim.agents.iter().map(|a| (a.id, a.pos)).collect(),
            traded_pairs: BTreeSet::new(),
            named_settlements: sim.settlements.ever_confirmed.iter().copied().collect(),
            day_trades: 0,
            day_births: 0,
            last_day: sim.day(),
            enabled: true,
            kin_lines: false,
            intro: true,
        }
    }

    fn say(&mut self, day: u32, text: String, color: Color) {
        self.lines.push_front((day, text, color));
        self.lines.truncate(MAX_LINES);
    }

    /// Record positions before a step (for smooth movement).
    pub fn before_step(&mut self, sim: &Simulation) {
        self.prev.clear();
        for a in &sim.agents {
            self.prev.insert(a.id, a.pos);
        }
    }

    /// Read what happened in the latest step(s).
    pub fn after_step(&mut self, sim: &Simulation, now: f64) {
        for a in &sim.agents {
            let t = self.trails.entry(a.id).or_default();
            if t.back() != Some(&a.pos) {
                t.push_back(a.pos);
                if t.len() > TRAIL_LEN {
                    t.pop_front();
                }
            }
        }
        if self.trails.len() > sim.agents.len() + 64 {
            let alive: BTreeSet<u32> = sim.agents.iter().map(|a| a.id).collect();
            self.trails.retain(|id, _| alive.contains(id));
        }
        let day = sim.day();
        if day != self.last_day {
            if self.day_trades + self.day_births > 0 {
                let text = format!(
                    "Day {}: {} birth(s), {} trade(s) across the land",
                    self.last_day, self.day_births, self.day_trades
                );
                self.say(self.last_day, text, Color::from_rgba(150, 160, 180, 255));
            }
            self.day_trades = 0;
            self.day_births = 0;
            self.last_day = day;
        }
        let new = sim.events.total.saturating_sub(self.seen_total) as usize;
        self.seen_total = sim.events.total;
        let n = new.min(sim.events.events.len());
        let start = sim.events.events.len() - n;
        let seed = sim.world.seed;
        let name =
            |id: Option<u32>| id.map_or_else(|| "someone".to_string(), |i| person_name(seed, i));
        let events: Vec<_> = sim.events.events.range(start..).cloned().collect();
        for e in events {
            match e.kind {
                EventKind::Born { parent, .. } => {
                    self.day_births += 1;
                    self.effects.push((
                        now,
                        Fx::Pulse {
                            at: e.pos,
                            color: Role::Family.color(),
                        },
                    ));
                    if self.day_births <= 2 {
                        let text = format!("{} was born to {}", name(e.agent), name(Some(parent)));
                        self.say(day, text, Role::Family.color());
                    }
                }
                EventKind::Died { cause } => {
                    let why = match cause {
                        DeathCause::Deprivation => "of hunger and exhaustion",
                        DeathCause::OldAge => "of old age",
                    };
                    self.effects.push((
                        now,
                        Fx::Pulse {
                            at: e.pos,
                            color: GRAY,
                        },
                    ));
                    self.say(
                        day,
                        format!("{} died {why}", name(e.agent)),
                        Color::from_rgba(170, 170, 170, 255),
                    );
                }
                EventKind::FoundedHome { .. } => {
                    self.effects.push((
                        now,
                        Fx::Ring {
                            at: e.pos,
                            color: GOLD,
                        },
                    ));
                    if sim.day() > 3 {
                        self.say(day, format!("{} built a new home", name(e.agent)), GOLD);
                    }
                }
                EventKind::Traded {
                    partner,
                    gave,
                    gave_qty,
                    got,
                    got_qty,
                } => {
                    self.day_trades += 1;
                    if let Some(a) = e.agent {
                        self.effects.push((now, Fx::TradeArc { a, b: partner }));
                        let key = (a.min(partner), a.max(partner));
                        if self.traded_pairs.insert(key) && self.traded_pairs.len() % 4 == 1 {
                            let text = format!(
                                "{} traded {:.0} {} to {} for {:.0} {}",
                                name(Some(a)),
                                gave_qty,
                                gave.name(),
                                name(Some(partner)),
                                got_qty,
                                got.name()
                            );
                            self.say(day, text, Color::from_rgba(255, 215, 80, 255));
                        }
                    }
                }
                EventKind::Crafted { amount } if amount > 0.0 => {
                    self.effects.push((
                        now,
                        Fx::Ring {
                            at: e.pos,
                            color: Role::Crafter.color(),
                        },
                    ));
                }
                EventKind::RoadBuilt => {
                    self.say(
                        day,
                        format!("{} paved a stretch of road", name(e.agent)),
                        Role::Builder.color(),
                    );
                }
                EventKind::ShelterImproved { .. } => {
                    self.say(
                        day,
                        format!("{} enlarged their family's home", name(e.agent)),
                        Role::Builder.color(),
                    );
                }
                EventKind::BuiltStorage { .. } => {
                    self.effects.push((
                        now,
                        Fx::Ring {
                            at: e.pos,
                            color: Role::Builder.color(),
                        },
                    ));
                }
                EventKind::DroughtStarted { radius, days, .. } => {
                    self.say(
                        day,
                        format!("A drought begins: {radius} km across, for {days} days"),
                        Color::from_rgba(255, 140, 0, 255),
                    );
                }
                EventKind::DroughtEnded => {
                    self.say(
                        day,
                        "The rains return".to_string(),
                        Color::from_rgba(120, 180, 255, 255),
                    );
                }
                _ => {}
            }
        }
        for s in &sim.settlements.tracked {
            if s.class != SizeClass::Candidate && self.named_settlements.insert(s.id) {
                let c = Pos::new(s.centre.0.round() as i32, s.centre.1.round() as i32);
                let text = format!(
                    "A hamlet has formed: {} ({} homes, {} people)",
                    settlement_name(sim, s.id, c),
                    s.homes.len(),
                    s.residents
                );
                self.say(day, text, Color::from_rgba(255, 110, 210, 255));
            }
        }
        self.effects.retain(|(t, _)| now - *t < FX_SECONDS);
    }
}

/// Plain-language summary of a person: who they are and why they are doing
/// what they are doing.
pub fn biography(sim: &Simulation, a: &Agent) -> Vec<String> {
    let seed = sim.world.seed;
    let mut out = Vec::new();
    let age = a.age_days();
    let stage = if a.is_juvenile(&sim.rates) {
        "a child"
    } else if a.age_ticks >= sim.rates.elder_ticks {
        "an elder"
    } else {
        "an adult"
    };
    out.push(format!(
        "{}, {} ({:.0} days old)",
        person_name(seed, a.id),
        stage,
        age
    ));
    let home_line = match a.home {
        None => "has no home yet".to_string(),
        Some(h) => {
            let home = &sim.world.homes[h as usize];
            let others = home.residents.len().saturating_sub(1);
            let place = sim
                .settlements
                .tracked
                .iter()
                .find(|s| s.homes.contains(&h))
                .map(|s| {
                    let c = Pos::new(s.centre.0.round() as i32, s.centre.1.round() as i32);
                    format!("in {}", settlement_name(sim, s.id, c))
                })
                .unwrap_or_else(|| "in a lone home".to_string());
            format!(
                "lives {place} with {others} other{}",
                if others == 1 { "" } else { "s" }
            )
        }
    };
    out.push(home_line);
    if let Some(p) = a.parent {
        out.push(format!(
            "child of {}; has {} children",
            person_name(seed, p),
            a.stats.births
        ));
    } else {
        out.push(format!(
            "one of the first settlers; has {} children",
            a.stats.births
        ));
    }
    // Why: the strongest positive reason behind the current choice.
    let why = sim
        .traces
        .get(&a.id)
        .and_then(|t| t.chosen.as_ref())
        .and_then(|c| {
            c.terms
                .iter()
                .filter(|t| t.value > 0.0)
                .max_by(|x, y| x.value.total_cmp(&y.value))
                .map(|t| reason(t.name))
        })
        .unwrap_or("nothing pressing");
    out.push(format!("now {} because {why}", Role::of(a).name()));
    let mut feel = Vec::new();
    if a.hunger > 0.5 {
        feel.push("hungry");
    }
    if a.energy < 0.3 {
        feel.push("tired");
    }
    if a.health < 0.6 {
        feel.push("in poor health");
    }
    if a.inventory.tools > 0.0 {
        feel.push("carrying a tool");
    }
    if !feel.is_empty() {
        out.push(format!("feels {}", feel.join(", ")));
    }
    out
}

fn reason(term: &str) -> &'static str {
    match term {
        "hunger" | "base" => "they are getting hungry",
        "fatigue" => "they are tired",
        "leisure" | "home bed" => "there is nothing more pressing",
        "food" => "they want to stock up on food",
        "curiosity" => "they are curious about unexplored land",
        "desperation" => "they cannot find food nearby",
        "land hunger" => "the land around home feels crowded",
        "want home" | "site" | "better site" => "they want a better place to live",
        "offspring" | "comfort" => "they want a child",
        "materials" | "storage" => "they are gathering materials to build",
        "own tool" | "tools to trade" => "a tool makes work faster",
        "time saved" => "a road would save the family time",
        "room for family" => "the home is getting crowded",
        "surplus" | "full hands" => "their hands are full",
        _ => "it seems worthwhile",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_stable_and_varied() {
        assert_eq!(person_name(1, 42), person_name(1, 42));
        let names: BTreeSet<String> = (0..200).map(|i| person_name(1, i)).collect();
        assert!(
            names.len() > 150,
            "names should rarely collide: {}",
            names.len()
        );
        assert!(
            names
                .iter()
                .all(|n| n.chars().next().is_some_and(char::is_uppercase))
        );
    }
}
