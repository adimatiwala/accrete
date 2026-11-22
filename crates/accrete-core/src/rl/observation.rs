//! Observation encoding for the controlled agent.
//!
//! **Information rule:** the observation contains only what the agent itself
//! knows or currently sees, exactly the information its scripted decision
//! logic is allowed to use. It never reads undiscovered resources, other
//! households' private state, or global statistics.
//!
//! Three parts, all `f32`, fixed sizes:
//!
//! * `grid`: `GRID_CHANNELS × 13 × 13`, centred on the agent (row-major,
//!   channel first). Off-map cells are zero.
//! * `vector`: `VECTOR_SIZE` scalar features about the agent's body,
//!   inventory, household and traits.
//! * `slots`: `N_ACTIONS × SLOT_FEATURES`, describing the option behind each
//!   action slot (see [`super::actions`]).

use crate::agents::agent::Agent;
use crate::agents::decision::{self, Candidate, Ctx};
use crate::agents::memory::freshness;
use crate::inventory::Resource;
use crate::simulation::Simulation;
use crate::world::visibility::SIGHT_RADIUS;
use crate::world::{Biome, PathLevel};

use super::actions::N_ACTIONS;

pub const GRID_SIDE: usize = (2 * SIGHT_RADIUS + 1) as usize;
pub const GRID_CHANNELS: usize = 14;
pub const VECTOR_SIZE: usize = 36;
pub const SLOT_FEATURES: usize = 5;

/// Names of the grid channels, in order (for documentation and tooling).
pub const GRID_CHANNEL_NAMES: [&str; GRID_CHANNELS] = [
    "known",
    "passable",
    "river",
    "forest",
    "relief",
    "move_cost",
    "food_remembered",
    "wood_remembered",
    "stone_remembered",
    "own_home",
    "other_homes",
    "path",
    "agents_visible",
    "visible_now",
];

/// Names of the vector features, in order.
pub const VECTOR_NAMES: [&str; VECTOR_SIZE] = [
    "hunger",
    "energy",
    "health",
    "health_ceiling",
    "frailty",
    "age",
    "juvenile",
    "food",
    "wood",
    "stone",
    "load",
    "has_home",
    "at_home",
    "home_dx",
    "home_dy",
    "home_food",
    "home_residents",
    "home_capacity",
    "home_chests",
    "home_shelter",
    "home_crowded",
    "forage_rate",
    "reserve_target",
    "eat_threshold",
    "rest_threshold",
    "curiosity",
    "caution",
    "sociability",
    "diligence",
    "skill_food",
    "skill_wood",
    "skill_stone",
    "since_last_birth",
    "can_reproduce",
    "known_tiles",
    "land_ratio",
];

pub const SLOT_NAMES: [&str; SLOT_FEATURES] = ["valid", "eta", "dx", "dy", "utility"];

#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    pub grid: Vec<f32>,
    pub vector: Vec<f32>,
    pub slots: Vec<f32>,
    pub mask: Vec<bool>,
}

impl Observation {
    pub fn zeros() -> Self {
        Self {
            grid: vec![0.0; GRID_CHANNELS * GRID_SIDE * GRID_SIDE],
            vector: vec![0.0; VECTOR_SIZE],
            slots: vec![0.0; N_ACTIONS * SLOT_FEATURES],
            mask: vec![false; N_ACTIONS],
        }
    }
}

#[inline]
fn clip(v: f32, lo: f32, hi: f32) -> f32 {
    v.clamp(lo, hi)
}

/// Encode agent `idx`'s view. `slots` are the options behind each action
/// (from [`super::actions::slot_candidates`]).
pub fn encode(
    sim: &Simulation,
    idx: usize,
    slots: &[Option<Candidate>],
    expose_utility: bool,
) -> Observation {
    let mut obs = Observation::zeros();
    let a = &sim.agents[idx];
    encode_grid(sim, a, &mut obs.grid);
    encode_vector(sim, a, &mut obs.vector);
    for (k, slot) in slots.iter().enumerate() {
        let f = &mut obs.slots[k * SLOT_FEATURES..(k + 1) * SLOT_FEATURES];
        if let Some(c) = slot {
            obs.mask[k] = true;
            f[0] = 1.0;
            f[1] = clip(c.est_ticks / 48.0, 0.0, 1.0);
            f[2] = clip((c.target.x - a.pos.x) as f32 / 16.0, -1.0, 1.0);
            f[3] = clip((c.target.y - a.pos.y) as f32 / 16.0, -1.0, 1.0);
            f[4] = if expose_utility {
                clip(c.score / 4.0, -1.0, 1.0)
            } else {
                0.0
            };
        }
    }
    obs
}

fn encode_grid(sim: &Simulation, a: &Agent, grid: &mut [f32]) {
    let world = &sim.world;
    let side = GRID_SIDE as i32;
    let r = SIGHT_RADIUS;
    let plane = GRID_SIDE * GRID_SIDE;
    let cell = |dx: i32, dy: i32| ((dy + r) * side + (dx + r)) as usize;
    let mc = &sim.config.movement;
    let tick = sim.tick;
    let half_life = sim.rates.freshness_half_life_ticks;

    for dy in -r..=r {
        for dx in -r..=r {
            let p = a.pos.offset(dx, dy);
            if !world.contains(p) || !a.memory.knows(p) {
                continue;
            }
            let c = cell(dx, dy);
            let t = world.tile(p);
            grid[c] = 1.0;
            grid[plane + c] = f32::from(u8::from(t.is_land()));
            grid[2 * plane + c] = f32::from(u8::from(t.is_river));
            grid[3 * plane + c] = f32::from(u8::from(t.biome == Biome::Forest));
            grid[4 * plane + c] = match t.biome {
                Biome::Hills => 0.5,
                Biome::Mountains => 1.0,
                _ => 0.0,
            };
            let believed = a.memory.believed_path(p);
            grid[5 * plane + c] =
                crate::world::movement::cost_for(t.biome, t.is_river, believed, mc)
                    .map_or(1.0, |m| clip(m as f32 / 80.0, 0.0, 1.0));
            grid[11 * plane + c] = match believed {
                PathLevel::None => 0.0,
                PathLevel::Trail => 0.5,
                PathLevel::Road => 1.0,
            };
            grid[13 * plane + c] = f32::from(u8::from(world.vis.is_visible(world, a.pos, p)));
        }
    }
    for m in &a.memory.resources {
        let (dx, dy) = (m.pos.x - a.pos.x, m.pos.y - a.pos.y);
        if dx.abs() > r || dy.abs() > r {
            continue;
        }
        let (ch, scale) = match m.resource {
            Resource::Food => (6, 30.0),
            Resource::Wood => (7, 40.0),
            Resource::Stone => (8, 200.0),
        };
        let v = m.amount * freshness(m.seen_tick, tick, half_life) / scale;
        grid[ch * plane + cell(dx, dy)] = clip(v, 0.0, 1.0);
    }
    for h in &a.memory.homes {
        let (dx, dy) = (h.pos.x - a.pos.x, h.pos.y - a.pos.y);
        if dx.abs() > r || dy.abs() > r {
            continue;
        }
        if Some(h.home) == a.home {
            grid[9 * plane + cell(dx, dy)] = 1.0;
        } else {
            let c = 10 * plane + cell(dx, dy);
            grid[c] = clip(grid[c] + h.residents as f32 / 8.0, 0.0, 1.0);
        }
    }
    if let Some(h) = a.home {
        let hp = world.homes[h as usize].pos;
        let (dx, dy) = (hp.x - a.pos.x, hp.y - a.pos.y);
        if dx.abs() <= r && dy.abs() <= r {
            grid[9 * plane + cell(dx, dy)] = 1.0;
        }
    }
    // Other agents the controlled agent can see right now.
    for o in &sim.agents {
        if o.id == a.id {
            continue;
        }
        let (dx, dy) = (o.pos.x - a.pos.x, o.pos.y - a.pos.y);
        if dx.abs() > r || dy.abs() > r || !world.vis.is_visible(world, a.pos, o.pos) {
            continue;
        }
        let c = 12 * plane + cell(dx, dy);
        grid[c] = clip(grid[c] + 0.25, 0.0, 1.0);
    }
}

fn encode_vector(sim: &Simulation, a: &Agent, v: &mut [f32]) {
    let c = &sim.config;
    let r = &sim.rates;
    let cx = Ctx {
        world: &sim.world,
        config: c,
        rates: r,
        tick: sim.tick,
        population_at_cap: sim.agents.len() as u32 >= c.life.population_cap,
        agents: &sim.agents,
    };
    let cap = a.carry_capacity(c, r);
    let home = cx.home(a);
    let mut f = vec![
        a.hunger,
        a.energy,
        a.health,
        a.health_ceiling(r),
        clip(a.frailty, 0.0, 1.0),
        clip(a.age_ticks as f32 / (r.elder_ticks as f32 * 1.6), 0.0, 1.0),
        f32::from(u8::from(a.is_juvenile(r))),
        clip(a.inventory.food / 30.0, 0.0, 1.0),
        clip(a.inventory.wood / 15.0, 0.0, 1.0),
        clip(a.inventory.stone / 10.0, 0.0, 1.0),
        clip(a.inventory.weight(&c.inventory) / cap, 0.0, 1.0),
        f32::from(u8::from(home.is_some())),
        f32::from(u8::from(home.is_some_and(|h| h.pos == a.pos))),
    ];
    match home {
        Some(h) => {
            f.push(clip((h.pos.x - a.pos.x) as f32 / 32.0, -1.0, 1.0));
            f.push(clip((h.pos.y - a.pos.y) as f32 / 32.0, -1.0, 1.0));
            f.push(clip(a.memory.home_food_seen / 100.0, 0.0, 1.0));
            f.push(clip(h.residents.len() as f32 / 8.0, 0.0, 1.0));
            f.push(clip(
                h.comfortable_capacity(c.homes.comfortable_capacity) as f32 / 8.0,
                0.0,
                1.0,
            ));
            f.push(h.chests.len() as f32 / c.inventory.max_chests.max(1) as f32);
            f.push(f32::from(h.shelter_level) / 2.0);
            f.push(f32::from(u8::from(decision::is_crowded(h, &cx))));
        }
        None => f.extend_from_slice(&[0.0; 8]),
    }
    f.push(clip(a.forage_rate / 2.0, 0.0, 1.0));
    f.push(clip(decision::reserve_target(a, &cx) / 12.0, 0.0, 1.0));
    f.push(decision::eat_threshold(a, &cx));
    f.push(decision::rest_threshold(a, &cx));
    f.extend_from_slice(&[
        a.traits.curiosity,
        a.traits.caution,
        a.traits.sociability,
        a.traits.diligence,
    ]);
    for s in &a.skills {
        f.push(clip(s.level() / 1.6, 0.0, 1.0));
    }
    let since_birth = a.last_birth_tick.map_or(1.0, |t| {
        clip(
            sim.tick.saturating_sub(t) as f32 / r.reproduce_cooldown_ticks.max(1) as f32,
            0.0,
            1.0,
        )
    });
    f.push(since_birth);
    f.push(f32::from(u8::from(
        decision::reproduce_blockers(a, &cx).is_empty(),
    )));
    f.push(clip(a.memory.known_count() as f32 / 2000.0, 0.0, 1.0));
    f.push(
        a.home_plan
            .map_or(0.0, |p| clip(p.home_land / 2.0, 0.0, 1.0)),
    );
    debug_assert_eq!(f.len(), VECTOR_SIZE);
    v.copy_from_slice(&f);
}
