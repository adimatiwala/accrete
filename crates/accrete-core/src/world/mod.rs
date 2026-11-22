//! The physical world: terrain, rivers, resources, homes, and dropped piles.
//!
//! Generation is split by concern: [`terrain`] (elevation, moisture, biomes),
//! [`rivers`] (drainage and river distance), [`resources`] (fertility, food,
//! wood, stone, and daily regrowth), [`visibility`] (precomputed line-of-sight
//! masks), [`movement`] (movement-point costs), and [`spawn`] (viable starting
//! tiles). [`validate`] checks every Week 1 acceptance property.

pub mod movement;
pub mod noise;
pub mod resources;
pub mod rivers;
pub mod spawn;
pub mod stats;
pub mod terrain;
pub mod validate;
pub mod visibility;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::geom::{Dims, Pos};
use crate::hash::{StableHash, StableHasher};
use crate::inventory::Inventory;
use crate::rng::{Rng, Stream};
use visibility::Visibility;

pub type HomeId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Biome {
    Ocean,
    Plains,
    Forest,
    Hills,
    Mountains,
}

impl Biome {
    pub const ALL: [Biome; 5] = [
        Biome::Ocean,
        Biome::Plains,
        Biome::Forest,
        Biome::Hills,
        Biome::Mountains,
    ];

    pub fn is_land(self) -> bool {
        self != Biome::Ocean
    }

    pub fn name(self) -> &'static str {
        match self {
            Biome::Ocean => "ocean",
            Biome::Plains => "plains",
            Biome::Forest => "forest",
            Biome::Hills => "hills",
            Biome::Mountains => "mountains",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum PathLevel {
    #[default]
    None,
    Trail,
    Road,
}

/// A temporary route failure on a tile (accrete.md, Week 4 Day 2): a flood,
/// landslide or washed-out ford. Set and cleared by scripted interventions
/// (`simulation::disruption`); agents learn of it only by seeing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Disruption {
    #[default]
    None,
    /// Entering costs `factor` times the normal movement points.
    Slowed { factor: u8 },
    /// Impassable.
    Blocked,
}

impl Disruption {
    pub fn code(self) -> u8 {
        match self {
            Disruption::None => 0,
            Disruption::Slowed { factor } => factor.max(1),
            Disruption::Blocked => 255,
        }
    }

    /// The more severe of two disruptions.
    pub fn worst(self, other: Disruption) -> Disruption {
        if other.code() > self.code() {
            other
        } else {
            self
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    /// Normalized elevation in `[0, 1]`; land is at or above `World::sea_level`.
    pub elevation: f32,
    /// Normalized moisture in `[0, 1]`.
    pub moisture: f32,
    pub fertility: f32,
    pub biome: Biome,
    pub is_river: bool,
    /// Rain units draining through this tile (drives river size).
    pub flow: f32,
    /// Chebyshev tiles to the nearest river tile, capped at 255.
    pub river_distance: u8,
    /// Crop-patch multiplier applied to food capacity (0.4, 1.0 or 1.5).
    pub patch: f32,
    pub food: f32,
    pub food_capacity: f32,
    pub wood: f32,
    pub wood_capacity: f32,
    /// Finite deposit; never regenerates.
    pub stone: f32,
    /// Decayed crossing count: `traffic_next = decay * traffic + crossings`.
    pub traffic: f32,
    pub path_level: PathLevel,
    /// Consecutive days a worn trail has been below the traffic threshold.
    pub trail_fade_days: u16,
    /// Condition of a built road (0 when there is none); falls by one on
    /// each day without traffic, and the road reverts at 0.
    pub road_condition: u16,
    /// Someone crossed this tile today (reset daily).
    pub crossed_today: bool,
}

impl Tile {
    pub fn is_land(&self) -> bool {
        self.biome.is_land()
    }
}

impl StableHash for Tile {
    fn stable_hash(&self, h: &mut StableHasher) {
        h.f32(self.elevation);
        h.f32(self.moisture);
        h.f32(self.fertility);
        h.u8(self.biome as u8);
        h.bool(self.is_river);
        h.f32(self.flow);
        h.u8(self.river_distance);
        h.f32(self.patch);
        h.f32(self.food);
        h.f32(self.food_capacity);
        h.f32(self.wood);
        h.f32(self.wood_capacity);
        h.f32(self.stone);
        h.f32(self.traffic);
        h.u8(self.path_level as u8);
        h.u16(self.trail_fade_days);
        h.u16(self.road_condition);
        h.bool(self.crossed_today);
    }
}

/// A dwelling. Residents are kept sorted for deterministic iteration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Home {
    pub id: HomeId,
    pub pos: Pos,
    pub residents: Vec<u32>,
    pub chests: Vec<Inventory>,
    pub shelter_level: u8,
    pub founded_tick: u64,
    pub over_capacity_since: Option<u64>,
}

impl Home {
    pub fn is_vacant(&self) -> bool {
        self.residents.is_empty()
    }

    pub fn stored(&self) -> Inventory {
        let mut total = Inventory::EMPTY;
        for c in &self.chests {
            total.add_all(c);
        }
        total
    }

    pub fn comfortable_capacity(&self, base: u32) -> u32 {
        base + 2 * u32::from(self.shelter_level)
    }
}

impl StableHash for Home {
    fn stable_hash(&self, h: &mut StableHasher) {
        h.u32(self.id);
        self.pos.stable_hash(h);
        h.len(self.residents.len());
        for &r in &self.residents {
            h.u32(r);
        }
        h.len(self.chests.len());
        for c in &self.chests {
            c.stable_hash(h);
        }
        h.u8(self.shelter_level);
        h.u64(self.founded_tick);
        h.opt_u64(self.over_capacity_since);
    }
}

/// Goods dropped on a tile (by a death with nowhere to store them). Anyone can
/// gather from a pile; pile food spoils at the carried rate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pile {
    pub pos: Pos,
    pub goods: Inventory,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct World {
    pub dims: Dims,
    pub seed: u64,
    pub tiles: Vec<Tile>,
    pub sea_level: f32,
    pub hill_level: f32,
    pub mountain_level: f32,
    /// Downstream neighbour of each land tile (`NO_TILE` for outlets and ocean).
    pub flow_to: Vec<u32>,
    /// Depression-filled elevation used for drainage; strictly decreasing downstream.
    pub drain_elevation: Vec<f32>,
    /// Viable starting tiles for the initial agents.
    pub spawns: Vec<Pos>,
    pub homes: Vec<Home>,
    pub piles: Vec<Pile>,
    /// Derived, rebuilt from terrain on load: never serialized.
    #[serde(skip)]
    pub vis: Visibility,
    /// Derived index: home ids per tile (rebuilt on demand).
    #[serde(skip)]
    pub homes_at: Vec<Vec<HomeId>>,
    /// Derived index: active route disruption per tile, or empty when there
    /// is none. The authoritative state is `Simulation::disruptions`, which
    /// rebuilds this index whenever it changes.
    #[serde(skip)]
    pub disruption_at: Vec<Disruption>,
}

pub const NO_TILE: u32 = u32::MAX;

impl World {
    #[inline]
    pub fn tile(&self, p: Pos) -> &Tile {
        &self.tiles[self.dims.idx(p) as usize]
    }

    #[inline]
    pub fn tile_mut(&mut self, p: Pos) -> &mut Tile {
        let i = self.dims.idx(p) as usize;
        &mut self.tiles[i]
    }

    #[inline]
    pub fn contains(&self, p: Pos) -> bool {
        self.dims.contains(p)
    }

    /// Rebuild all derived (non-serialized) structures.
    pub fn rebuild_derived(&mut self, config: &Config) {
        self.vis = Visibility::compute(self, &config.sight);
        self.rebuild_home_index();
    }

    pub fn rebuild_home_index(&mut self) {
        let mut idx = vec![Vec::new(); self.dims.len()];
        for h in &self.homes {
            idx[self.dims.idx(h.pos) as usize].push(h.id);
        }
        self.homes_at = idx;
    }

    pub fn homes_on(&self, p: Pos) -> &[HomeId] {
        if self.homes_at.is_empty() {
            return &[];
        }
        &self.homes_at[self.dims.idx(p) as usize]
    }

    /// The route disruption on `p` (none on most tiles).
    #[inline]
    pub fn disruption(&self, p: Pos) -> Disruption {
        if self.disruption_at.is_empty() {
            Disruption::None
        } else {
            self.disruption_at[self.dims.idx(p) as usize]
        }
    }

    /// MP to enter `p` by an orthogonal step, including any disruption, or
    /// `None` if impassable.
    #[inline]
    pub fn enter_cost(&self, p: Pos, c: &crate::config::MovementConfig) -> Option<u32> {
        movement::with_disruption(movement::enter_cost(self.tile(p), c), self.disruption(p))
    }

    /// Orthogonal MP to step from `from` into its neighbour `to`; unlike
    /// `enter_cost`, this lets an agent standing in a blocked area wade out.
    pub fn step_in_cost(
        &self,
        from: Pos,
        to: Pos,
        c: &crate::config::MovementConfig,
    ) -> Option<u32> {
        movement::entry_cost(
            movement::enter_cost(self.tile(to), c),
            self.disruption(from),
            self.disruption(to),
        )
    }

    pub fn pile_at(&self, p: Pos) -> Option<&Pile> {
        self.piles.iter().find(|pl| pl.pos == p)
    }

    /// Food visible on a tile: standing crop plus any dropped pile.
    pub fn food_at(&self, p: Pos) -> f32 {
        self.tile(p).food + self.pile_at(p).map_or(0.0, |pl| pl.goods.food)
    }

    pub fn resource_at(&self, p: Pos, r: crate::inventory::Resource) -> f32 {
        use crate::inventory::Resource;
        let t = self.tile(p);
        let base = match r {
            Resource::Food => t.food,
            Resource::Wood => t.wood,
            Resource::Stone => t.stone,
            Resource::Tools => 0.0,
        };
        base + self.pile_at(p).map_or(0.0, |pl| pl.goods.get(r))
    }

    pub fn stable_hash_world(&self, h: &mut StableHasher) {
        h.u32(self.dims.width);
        h.u32(self.dims.height);
        h.u64(self.seed);
        h.f32(self.sea_level);
        h.f32(self.hill_level);
        h.f32(self.mountain_level);
        h.len(self.tiles.len());
        for t in &self.tiles {
            t.stable_hash(h);
        }
        for &f in &self.flow_to {
            h.u32(f);
        }
        for &e in &self.drain_elevation {
            h.f32(e);
        }
        h.len(self.spawns.len());
        for s in &self.spawns {
            s.stable_hash(h);
        }
        h.len(self.homes.len());
        for home in &self.homes {
            home.stable_hash(h);
        }
        h.len(self.piles.len());
        for p in &self.piles {
            p.pos.stable_hash(h);
            p.goods.stable_hash(h);
        }
    }

    /// Hash of the full world state (terrain, resources, homes, piles).
    pub fn state_hash(&self) -> u64 {
        let mut h = StableHasher::new();
        self.stable_hash_world(&mut h);
        h.finish()
    }

    /// Total resources lying in the world (tiles, piles, and chests).
    pub fn resource_totals(&self) -> Inventory {
        // f64 accumulation in fixed tile order, for precise conservation checks.
        let (mut f, mut w, mut s, mut k) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for t in &self.tiles {
            f += f64::from(t.food);
            w += f64::from(t.wood);
            s += f64::from(t.stone);
        }
        for p in &self.piles {
            f += f64::from(p.goods.food);
            w += f64::from(p.goods.wood);
            s += f64::from(p.goods.stone);
            k += f64::from(p.goods.tools);
        }
        for home in &self.homes {
            for c in &home.chests {
                f += f64::from(c.food);
                w += f64::from(c.wood);
                s += f64::from(c.stone);
                k += f64::from(c.tools);
            }
        }
        Inventory {
            food: f as f32,
            wood: w as f32,
            stone: s as f32,
            tools: k as f32,
        }
    }
}

/// Generate a world from a seed. Pure function of `(seed, config)`.
pub fn generate_world(seed: u64, config: &Config) -> World {
    let wc = &config.world;
    let dims = Dims::new(wc.width, wc.height);
    let mut rng = Rng::for_stream(seed, Stream::WorldGen);

    let terrain = terrain::generate(dims, wc, &mut rng);
    let drainage = rivers::route(dims, &terrain, wc);

    let mut tiles = Vec::with_capacity(dims.len());
    for i in 0..dims.len() {
        tiles.push(Tile {
            elevation: terrain.elevation[i],
            moisture: terrain.moisture[i],
            fertility: 0.0,
            biome: terrain.biome[i],
            is_river: drainage.is_river[i],
            flow: drainage.flow[i],
            river_distance: drainage.river_distance[i],
            patch: 1.0,
            food: 0.0,
            food_capacity: 0.0,
            wood: 0.0,
            wood_capacity: 0.0,
            stone: 0.0,
            traffic: 0.0,
            path_level: PathLevel::None,
            trail_fade_days: 0,
            road_condition: 0,
            crossed_today: false,
        });
    }

    let mut world = World {
        dims,
        seed,
        tiles,
        sea_level: terrain.sea_level,
        hill_level: terrain.hill_level,
        mountain_level: terrain.mountain_level,
        flow_to: drainage.flow_to,
        drain_elevation: drainage.filled,
        spawns: Vec::new(),
        homes: Vec::new(),
        piles: Vec::new(),
        vis: Visibility::default(),
        homes_at: Vec::new(),
        disruption_at: Vec::new(),
    };
    resources::place(&mut world, &config.resources, &mut rng);
    world.rebuild_derived(config);
    world.spawns = spawn::choose_spawns(&world, config, &mut Rng::for_stream(seed, Stream::Spawn));
    world
}
