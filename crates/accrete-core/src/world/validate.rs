//! Week 1 acceptance checks: every tile property is in its valid range, rivers
//! obey routing rules, and resources obey biome constraints. Returns a list of
//! human-readable violations (empty = valid).

use crate::config::Config;
use crate::world::resources::{fertility, grows_food, wood_capacity};
use crate::world::spawn::{is_viable, main_landmass};
use crate::world::{Biome, NO_TILE, World};

pub fn validate_world(world: &World, config: &Config) -> Vec<String> {
    let mut errs: Vec<String> = Vec::new();
    let dims = world.dims;
    let rc = &config.resources;
    let n = dims.len();
    let mut push = |s: String| {
        if errs.len() < 50 {
            errs.push(s);
        }
    };

    if world.tiles.len() != n || world.flow_to.len() != n || world.drain_elevation.len() != n {
        push("array lengths do not match dimensions".to_string());
        return errs;
    }
    if world.vis.masks.len() != n {
        push("visibility masks missing".to_string());
    }

    let max_food_cap = rc.food_capacity_scale * rc.patch_rich_factor.max(1.0);
    for i in 0..n {
        let t = &world.tiles[i];
        let p = dims.pos(i as u32);
        let in01 = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        if !in01(t.elevation) || !in01(t.moisture) || !in01(t.fertility) {
            push(format!("{p:?}: elevation/moisture/fertility out of [0,1]"));
        }
        if t.is_land() != (t.elevation >= world.sea_level) {
            push(format!(
                "{p:?}: biome {:?} disagrees with sea level",
                t.biome
            ));
        }
        let exp_f = fertility(t, rc);
        if (exp_f - t.fertility).abs() > 1e-6 {
            push(format!(
                "{p:?}: fertility {} != formula {}",
                t.fertility, exp_f
            ));
        }
        // Food
        if !(t.food.is_finite() && t.food >= 0.0 && t.food <= t.food_capacity + 1e-4) {
            push(format!(
                "{p:?}: food {} outside [0, {}]",
                t.food, t.food_capacity
            ));
        }
        if t.food_capacity > max_food_cap + 1e-4 {
            push(format!(
                "{p:?}: food capacity {} too large",
                t.food_capacity
            ));
        }
        if grows_food(t, rc) {
            let exp_cap = rc.food_capacity_scale * t.fertility * t.patch;
            if (exp_cap - t.food_capacity).abs() > 1e-4 {
                push(format!(
                    "{p:?}: food capacity {} != formula {}",
                    t.food_capacity, exp_cap
                ));
            }
        } else if t.food_capacity != 0.0 {
            push(format!(
                "{p:?}: food capacity on a tile that cannot grow food"
            ));
        }
        // Wood
        let exp_w = wood_capacity(t, rc);
        if t.wood_capacity != exp_w || !(t.wood >= 0.0 && t.wood <= t.wood_capacity + 1e-4) {
            push(format!(
                "{p:?}: wood {} / cap {} invalid (expected cap {exp_w})",
                t.wood, t.wood_capacity
            ));
        }
        // Stone
        let rocky = matches!(t.biome, Biome::Hills | Biome::Mountains);
        if t.stone < 0.0 || (t.stone > 0.0 && !rocky) || t.stone > rc.stone_max as f32 {
            push(format!("{p:?}: stone {} invalid on {:?}", t.stone, t.biome));
        }
        // Rivers
        if t.is_river {
            if !t.is_land() {
                push(format!("{p:?}: river on water"));
            }
            if t.river_distance != 0 {
                push(format!(
                    "{p:?}: river tile with river distance {}",
                    t.river_distance
                ));
            }
            let to = world.flow_to[i];
            if to == NO_TILE {
                if !dims.is_border(p) {
                    push(format!("{p:?}: interior river with no outlet"));
                }
            } else {
                let q = dims.pos(to);
                let tt = &world.tiles[to as usize];
                if p.chebyshev(q) != 1 {
                    push(format!("{p:?}: river flows to non-neighbour {q:?}"));
                }
                if tt.is_land() && !tt.is_river {
                    push(format!("{p:?}: river flows into non-river land {q:?}"));
                }
                if world.drain_elevation[to as usize] >= world.drain_elevation[i] {
                    push(format!("{p:?}: river flows uphill to {q:?}"));
                }
            }
        } else if t.river_distance == 0 {
            push(format!("{p:?}: non-river tile with river distance 0"));
        }
        if t.is_land() && world.flow_to[i] == NO_TILE && !dims.is_border(p) {
            push(format!("{p:?}: interior land tile does not drain"));
        }
        if !t.is_land() && (t.food != 0.0 || t.wood != 0.0 || t.stone != 0.0 || t.is_river) {
            push(format!("{p:?}: resources or river on water"));
        }
    }

    // Spawns
    let main = main_landmass(world);
    if world.spawns.len() != config.spawn.agents as usize {
        push(format!(
            "{} spawns, expected {}",
            world.spawns.len(),
            config.spawn.agents
        ));
    }
    for &s in &world.spawns {
        if !world.contains(s) || !is_viable(world, s, &main, config) {
            push(format!("spawn {s:?} not viable"));
        }
    }
    errs
}
