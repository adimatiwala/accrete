//! Fertility and resource placement, and daily regrowth.
//!
//! - Fertility = clamp(biome_base + 0.3·moisture + river_bonus(d), 0, 1).
//! - Food (renewable) grows where fertility >= 0.15, except on river and ocean
//!   tiles: capacity = 20·fertility·patch, starting at 80 %.
//! - Wood (renewable, slow): 40 on forest, 5 on plains with moisture > 0.5.
//! - Stone (finite): clustered deposits on hills and mountains where a deposit
//!   noise field exceeds 0.6, 50–200 units drawn from the world-gen stream.

use crate::config::ResourceConfig;
use crate::rng::Rng;
use crate::world::noise::{fbm, normalize};
use crate::world::{Biome, Tile, World};

pub fn biome_fertility(b: Biome, c: &ResourceConfig) -> f32 {
    match b {
        Biome::Ocean => 0.0,
        Biome::Plains => c.fertility_plains,
        Biome::Forest => c.fertility_forest,
        Biome::Hills => c.fertility_hills,
        Biome::Mountains => c.fertility_mountains,
    }
}

pub fn river_bonus(distance: u8, c: &ResourceConfig) -> f32 {
    match distance {
        1..=4 => c.river_bonus[distance as usize - 1],
        _ => 0.0,
    }
}

pub fn fertility(t: &Tile, c: &ResourceConfig) -> f32 {
    if !t.is_land() {
        return 0.0;
    }
    (biome_fertility(t.biome, c)
        + c.moisture_weight * t.moisture
        + river_bonus(t.river_distance, c))
    .clamp(0.0, 1.0)
}

pub fn grows_food(t: &Tile, c: &ResourceConfig) -> bool {
    t.is_land() && !t.is_river && t.fertility >= c.min_food_fertility
}

pub fn wood_capacity(t: &Tile, c: &ResourceConfig) -> f32 {
    match t.biome {
        Biome::Forest => c.forest_wood,
        Biome::Plains if t.moisture > c.plains_wood_min_moisture => c.plains_wood,
        _ => 0.0,
    }
}

pub fn place(world: &mut World, c: &ResourceConfig, rng: &mut Rng) {
    let dims = world.dims;
    // Seeds drawn in fixed order after terrain's.
    let seed_patch = rng.next_u64();
    let seed_deposit = rng.next_u64();

    let n = dims.len();
    let mut patch_noise = vec![0.0f32; n];
    let mut deposit_noise = vec![0.0f32; n];
    let inv_p = 1.0 / c.patch_scale.max(1.0);
    let inv_s = 1.0 / c.stone_scale.max(1.0);
    for i in 0..n as u32 {
        let p = dims.pos(i);
        let (x, y) = (p.x as f32, p.y as f32);
        patch_noise[i as usize] = fbm(seed_patch, x * inv_p, y * inv_p, 2);
        deposit_noise[i as usize] = fbm(seed_deposit, x * inv_s, y * inv_s, 3);
    }
    normalize(&mut patch_noise);
    normalize(&mut deposit_noise);

    for (t, &pn) in world.tiles.iter_mut().zip(&patch_noise) {
        t.fertility = fertility(t, c);
        t.patch = if pn > c.patch_rich_above {
            c.patch_rich_factor
        } else if pn < c.patch_poor_below {
            c.patch_poor_factor
        } else {
            1.0
        };
        if grows_food(t, c) {
            t.food_capacity = c.food_capacity_scale * t.fertility * t.patch;
            t.food = c.initial_food_fraction * t.food_capacity;
        }
        t.wood_capacity = wood_capacity(t, c);
        t.wood = t.wood_capacity;
    }

    // Stone: draws happen in ascending tile order, only for qualifying tiles.
    for (t, &dn) in world.tiles.iter_mut().zip(&deposit_noise) {
        let rocky = matches!(t.biome, Biome::Hills | Biome::Mountains);
        if rocky && dn > c.stone_threshold {
            t.stone = rng.range_u32_inclusive(c.stone_min, c.stone_max) as f32;
        }
    }
}

/// Daily regrowth of food and wood. `food_factor(i)` scales food regrowth per
/// tile (1.0 normally; reduced inside droughts). Returns `(food, wood)` grown.
pub fn regrow(
    world: &mut World,
    c: &ResourceConfig,
    food_factor: impl Fn(usize) -> f32,
) -> (f64, f64) {
    let mut grown_food = 0.0f64;
    let mut grown_wood = 0.0f64;
    for (i, t) in world.tiles.iter_mut().enumerate() {
        if t.food_capacity > 0.0 && t.food < t.food_capacity {
            let g = (c.food_regrowth * t.fertility * (t.food_capacity - t.food) * food_factor(i))
                .max(0.0);
            let next = (t.food + g).min(t.food_capacity);
            grown_food += f64::from(next - t.food);
            t.food = next;
        }
        if t.wood_capacity > 0.0 && t.wood < t.wood_capacity {
            let g = c.wood_regrowth * (t.wood_capacity - t.wood);
            let next = (t.wood + g).min(t.wood_capacity);
            grown_wood += f64::from(next - t.wood);
            t.wood = next;
        }
    }
    (grown_food, grown_wood)
}
