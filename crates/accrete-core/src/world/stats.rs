//! Summary statistics of a generated world, for the CLI, viewer, and tests.

use serde::Serialize;

use crate::world::{Biome, World};

#[derive(Clone, Debug, Serialize)]
pub struct WorldStats {
    pub seed: u64,
    pub state_hash: String,
    pub tiles: u32,
    pub biome_fraction: Vec<(String, f32)>,
    pub land_fraction: f32,
    pub river_tiles: u32,
    pub river_fraction_of_land: f32,
    pub mean_land_fertility: f32,
    pub food_tiles: u32,
    pub total_food: f32,
    pub total_food_capacity: f32,
    /// Upper bound on food regrowth per day at zero standing stock (units/day).
    pub max_food_regrowth_per_day: f32,
    pub total_wood: f32,
    pub stone_tiles: u32,
    pub total_stone: f32,
    pub mean_visible_tiles_land: f32,
    pub mean_visible_tiles_by_biome: Vec<(String, f32)>,
    pub spawns: u32,
}

pub fn world_stats(w: &World, food_regrowth: f32) -> WorldStats {
    let n = w.tiles.len() as f32;
    let mut biome_count = [0u32; 5];
    let mut vis_sum = [0f64; 5];
    let (mut rivers, mut food_tiles, mut stone_tiles) = (0u32, 0u32, 0u32);
    let (mut fert, mut food, mut cap, mut regrow, mut wood, mut stone) =
        (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
    for (i, t) in w.tiles.iter().enumerate() {
        biome_count[t.biome as usize] += 1;
        if !w.vis.masks.is_empty() {
            vis_sum[t.biome as usize] += f64::from(w.vis.visible_count(i as u32));
        }
        if t.is_river {
            rivers += 1;
        }
        if t.is_land() {
            fert += f64::from(t.fertility);
        }
        if t.food_capacity > 0.0 {
            food_tiles += 1;
        }
        if t.stone > 0.0 {
            stone_tiles += 1;
        }
        food += f64::from(t.food);
        cap += f64::from(t.food_capacity);
        regrow += f64::from(food_regrowth * t.fertility * t.food_capacity);
        wood += f64::from(t.wood);
        stone += f64::from(t.stone);
    }
    let land = n as u32 - biome_count[Biome::Ocean as usize];
    let landf = land.max(1) as f32;
    let land_vis: f64 = (1..5).map(|b| vis_sum[b]).sum();
    WorldStats {
        seed: w.seed,
        state_hash: crate::hash::hex(w.state_hash()),
        tiles: n as u32,
        biome_fraction: Biome::ALL
            .iter()
            .map(|&b| (b.name().to_string(), biome_count[b as usize] as f32 / n))
            .collect(),
        land_fraction: landf / n,
        river_tiles: rivers,
        river_fraction_of_land: rivers as f32 / landf,
        mean_land_fertility: (fert / f64::from(landf)) as f32,
        food_tiles,
        total_food: food as f32,
        total_food_capacity: cap as f32,
        max_food_regrowth_per_day: regrow as f32,
        total_wood: wood as f32,
        stone_tiles,
        total_stone: stone as f32,
        mean_visible_tiles_land: (land_vis / f64::from(landf)) as f32,
        mean_visible_tiles_by_biome: Biome::ALL
            .iter()
            .map(|&b| {
                let c = biome_count[b as usize].max(1);
                (
                    b.name().to_string(),
                    (vis_sum[b as usize] / f64::from(c)) as f32,
                )
            })
            .collect(),
        spawns: w.spawns.len() as u32,
    }
}
