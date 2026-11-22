//! Hand-built scenario worlds for behavioural tests.
//!
//! A scenario is a small TOML file (see `tests/scenarios/`) that bypasses
//! procedural generation, so it does not change when terrain generation is
//! tuned. Map characters:
//!
//! | Char | Terrain |
//! |---|---|
//! | `.` | plains |
//! | `f` | forest |
//! | `h` | hills |
//! | `m` | mountains (tall: blocks sight) |
//! | `~` | ocean (impassable) |
//! | `r` | plains with a river |
//! | `t` | plains with a worn trail |
//!
//! Resources are zero unless listed, and do not regrow (fertility 0), so the
//! scenario fully controls what exists.
//!
//! ```toml
//! name = "hungry-known-food"
//! map = """
//! ..........
//! ..........
//! """
//! [[food]]
//! at = [5, 1]
//! amount = 10.0
//! [[agent]]
//! at = [2, 1]
//! hunger = 0.6
//! ```

use serde::Deserialize;

use crate::agents::agent::Traits;
use crate::config::Config;
use crate::geom::{Dims, Pos};
use crate::inventory::Inventory;
use crate::simulation::Simulation;
use crate::world::visibility::Visibility;
use crate::world::{Biome, Home, NO_TILE, PathLevel, Tile, World};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub map: String,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub food: Vec<Placed>,
    #[serde(default)]
    pub wood: Vec<Placed>,
    #[serde(default)]
    pub home: Vec<HomeSpec>,
    #[serde(default)]
    pub agent: Vec<AgentSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placed {
    pub at: [i32; 2],
    pub amount: f32,
    /// Regrowth capacity; defaults to `amount` (with zero fertility it never regrows).
    #[serde(default)]
    pub capacity: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeSpec {
    pub at: [i32; 2],
    /// Food in each chest (one entry per chest).
    #[serde(default)]
    pub chests: Vec<f32>,
    /// Days the home has already been over capacity.
    #[serde(default)]
    pub crowded_days: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub at: [i32; 2],
    #[serde(default = "d_hunger")]
    pub hunger: f32,
    #[serde(default = "d_one")]
    pub energy: f32,
    #[serde(default)]
    pub food: f32,
    #[serde(default)]
    pub wood: f32,
    #[serde(default)]
    pub stone: f32,
    /// Frequent-tile memory: `[x, y, crossings]` entries.
    #[serde(default)]
    pub crossings: Vec<[f32; 3]>,
    /// Reciprocity toward other scenario agents: `[agent index, score]`.
    #[serde(default)]
    pub reciprocity: Vec<[f32; 2]>,
    /// Age in days (default: 60, an adult).
    #[serde(default = "d_age")]
    pub age_days: f32,
    /// Index into `home`, if the agent lives there.
    #[serde(default)]
    pub home: Option<usize>,
    /// "sight" (default): knows only what it sees at tick 0. "all": knows
    /// the whole map's terrain (not resources).
    #[serde(default = "d_knows")]
    pub knows: String,
    /// Traits (curiosity, caution, sociability, diligence); default 0.5 each.
    #[serde(default)]
    pub traits: Option<[f32; 4]>,
}

fn d_hunger() -> f32 {
    0.2
}
fn d_one() -> f32 {
    1.0
}
fn d_age() -> f32 {
    60.0
}
fn d_knows() -> String {
    "sight".into()
}

fn terrain(c: char) -> Result<(Biome, bool, f32), String> {
    Ok(match c {
        '.' | 't' => (Biome::Plains, false, 0.40),
        'f' => (Biome::Forest, false, 0.42),
        'h' => (Biome::Hills, false, 0.65),
        'm' => (Biome::Mountains, false, 0.95),
        '~' => (Biome::Ocean, false, 0.10),
        'r' => (Biome::Plains, true, 0.38),
        _ => return Err(format!("unknown map character {c:?}")),
    })
}

impl Scenario {
    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| e.to_string())
    }

    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_toml_str(&text)
    }

    /// Build the world (without agents).
    pub fn world(&self, config: &Config) -> Result<World, String> {
        let rows: Vec<&str> = self
            .map
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let height = rows.len() as u32;
        let width = rows.first().map_or(0, |r| r.chars().count()) as u32;
        if width == 0 || rows.iter().any(|r| r.chars().count() as u32 != width) {
            return Err("map rows must be non-empty and equal length".into());
        }
        let dims = Dims::new(width, height);
        let mut tiles = Vec::with_capacity(dims.len());
        for row in &rows {
            for c in row.chars() {
                let (biome, is_river, elevation) = terrain(c)?;
                let path_level = if c == 't' {
                    PathLevel::Trail
                } else {
                    PathLevel::None
                };
                tiles.push(Tile {
                    elevation,
                    moisture: 0.5,
                    fertility: 0.0,
                    biome,
                    is_river,
                    flow: 0.0,
                    river_distance: if is_river { 0 } else { u8::MAX },
                    patch: 1.0,
                    food: 0.0,
                    food_capacity: 0.0,
                    wood: 0.0,
                    wood_capacity: 0.0,
                    stone: 0.0,
                    traffic: if c == 't' { 25.0 } else { 0.0 },
                    path_level,
                    trail_fade_days: 0,
                    road_condition: 0,
                    crossed_today: false,
                });
            }
        }
        let mut world = World {
            dims,
            seed: self.seed,
            drain_elevation: tiles.iter().map(|t| t.elevation).collect(),
            tiles,
            sea_level: 0.3,
            hill_level: 0.6,
            mountain_level: 0.85,
            flow_to: vec![NO_TILE; dims.len()],
            spawns: self
                .agent
                .iter()
                .map(|a| Pos::new(a.at[0], a.at[1]))
                .collect(),
            homes: Vec::new(),
            piles: Vec::new(),
            vis: Visibility::default(),
            homes_at: Vec::new(),
        };
        for f in &self.food {
            let p = Pos::new(f.at[0], f.at[1]);
            let t = world.tile_mut(p);
            t.food = f.amount;
            t.food_capacity = f.capacity.unwrap_or(f.amount);
        }
        for w in &self.wood {
            let p = Pos::new(w.at[0], w.at[1]);
            let t = world.tile_mut(p);
            t.wood = w.amount;
            t.wood_capacity = w.capacity.unwrap_or(w.amount);
        }
        for (i, h) in self.home.iter().enumerate() {
            world.homes.push(Home {
                id: i as u32,
                pos: Pos::new(h.at[0], h.at[1]),
                residents: Vec::new(),
                chests: h
                    .chests
                    .iter()
                    .map(|&food| Inventory {
                        food,
                        ..Inventory::EMPTY
                    })
                    .collect(),
                shelter_level: 0,
                founded_tick: 0,
                over_capacity_since: None,
            });
        }
        world.rebuild_derived(config);
        Ok(world)
    }

    /// Build a ready-to-run simulation with the scenario's agents.
    pub fn build(&self, config: &Config) -> Result<Simulation, String> {
        let world = self.world(config)?;
        let mut sim = Simulation::from_world(world, config.clone());
        let ticks_per_day = crate::config::TICKS_PER_DAY as f32;
        for (k, spec) in self.agent.iter().enumerate() {
            let a = &mut sim.agents[k];
            a.hunger = spec.hunger;
            a.energy = spec.energy;
            a.inventory = Inventory {
                food: spec.food,
                wood: spec.wood,
                stone: spec.stone,
            };
            a.age_ticks = (spec.age_days * ticks_per_day) as u64;
            let t = spec.traits.unwrap_or([0.5; 4]);
            a.traits = Traits {
                curiosity: t[0],
                caution: t[1],
                sociability: t[2],
                diligence: t[3],
            };
            for s in a.skills.iter_mut() {
                s.aptitude = 1.0;
            }
            if spec.knows == "all" {
                for i in 0..sim.world.dims.len() as u32 {
                    a.memory.learn(sim.world.dims.pos(i));
                }
            } else {
                // Forget the founder's initial terrain knowledge beyond sight.
                a.memory = crate::agents::memory::AgentMemory::new(sim.world.dims);
            }
            for c in &spec.crossings {
                a.memory.frequent.push(crate::agents::memory::FrequentTile {
                    pos: Pos::new(c[0] as i32, c[1] as i32),
                    count: c[2],
                });
            }
            for r in &spec.reciprocity {
                let other = r[0] as u32; // founders' ids are their scenario indices
                let slots = config.social.social_slots as usize;
                if let Some(k) = a.memory.meet(other, 0, slots) {
                    a.memory.social[k].score = r[1];
                }
            }
            if let Some(h) = spec.home {
                a.home = Some(h as u32);
                let home = &mut sim.world.homes[h];
                if let Err(pos) = home.residents.binary_search(&a.id) {
                    home.residents.insert(pos, a.id);
                }
            }
        }
        // Homes that have already been crowded for a while: start the clock
        // late enough that `over_capacity_since` can lie in the past.
        let max_days = self
            .home
            .iter()
            .filter_map(|h| h.crowded_days)
            .fold(0.0f32, f32::max);
        if max_days > 0.0 {
            sim.tick = ((max_days + 1.0) * ticks_per_day) as u64;
            for (i, h) in self.home.iter().enumerate() {
                if let Some(days) = h.crowded_days {
                    sim.world.homes[i].over_capacity_since =
                        Some(sim.tick - (days * ticks_per_day) as u64);
                }
            }
            for a in sim.agents.iter_mut() {
                a.goal.since = sim.tick;
                a.pathology.last_progress_tick = sim.tick;
            }
        }
        // Rebuild the ledger baseline for the scenario's resources.
        sim.reset_ledger();
        for i in 0..sim.agents.len() {
            let tick = sim.tick;
            crate::simulation::engine::observe(
                &mut sim.agents[i],
                &sim.world,
                &sim.config,
                &sim.rates,
                tick,
            );
        }
        Ok(sim)
    }
}
