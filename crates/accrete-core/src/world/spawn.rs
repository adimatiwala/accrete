//! Viable starting positions for the initial agents.
//!
//! A tile is viable when it is passable land, not a river, lies on the largest
//! connected landmass (so nobody starts stranded on an island), and has at
//! least `min_nearby_food` units of food within `food_radius`. Agents are
//! spread out with a minimum spacing, chosen uniformly from the `spawn`
//! stream; nothing is pre-clustered, so any settlement must emerge.

use crate::config::Config;
use crate::geom::Pos;
use crate::rng::Rng;
use crate::world::World;
use crate::world::terrain::components;

/// Mask of tiles on the largest 4-connected land component.
pub fn main_landmass(world: &World) -> Vec<bool> {
    let elev: Vec<f32> = world
        .tiles
        .iter()
        .map(|t| if t.is_land() { 1.0 } else { 0.0 })
        .collect();
    let comps = components(world.dims, &elev, |e| e > 0.5);
    let mut mask = vec![false; world.dims.len()];
    if let Some(best) = comps
        .iter()
        .max_by(|a, b| a.len().cmp(&b.len()).then(b[0].cmp(&a[0])))
    {
        for &i in best {
            mask[i as usize] = true;
        }
    }
    mask
}

pub fn food_within(world: &World, p: Pos, r: i32) -> f32 {
    let mut total = 0.0;
    for dy in -r..=r {
        for dx in -r..=r {
            let q = p.offset(dx, dy);
            if world.contains(q) {
                total += world.tile(q).food;
            }
        }
    }
    total
}

pub fn is_viable(world: &World, p: Pos, main: &[bool], config: &Config) -> bool {
    let t = world.tile(p);
    t.is_land()
        && !t.is_river
        && main[world.dims.idx(p) as usize]
        && food_within(world, p, config.spawn.food_radius as i32) >= config.spawn.min_nearby_food
}

pub fn choose_spawns(world: &World, config: &Config, rng: &mut Rng) -> Vec<Pos> {
    let want = config.spawn.agents as usize;
    let main = main_landmass(world);
    let mut candidates: Vec<Pos> = (0..world.dims.len() as u32)
        .map(|i| world.dims.pos(i))
        .filter(|&p| is_viable(world, p, &main, config))
        .collect();
    if candidates.is_empty() {
        // Degenerate world: fall back to any passable non-river tile on the main landmass.
        candidates = (0..world.dims.len() as u32)
            .map(|i| world.dims.pos(i))
            .filter(|&p| main[world.dims.idx(p) as usize] && !world.tile(p).is_river)
            .collect();
    }
    if candidates.is_empty() {
        return Vec::new();
    }
    rng.shuffle(&mut candidates);

    let mut chosen: Vec<Pos> = Vec::with_capacity(want);
    let mut spacing = config.spawn.min_spacing as i32;
    loop {
        for &c in &candidates {
            if chosen.len() >= want {
                break;
            }
            if spacing > 0 && chosen.iter().any(|&o| o.chebyshev(c) < spacing) {
                continue;
            }
            if spacing == 0 && chosen.contains(&c) && chosen.len() < candidates.len() {
                continue;
            }
            chosen.push(c);
        }
        if chosen.len() >= want {
            break;
        }
        if spacing == 0 {
            // More agents than tiles: allow sharing, cycling through candidates.
            let mut k = 0;
            while chosen.len() < want {
                chosen.push(candidates[k % candidates.len()]);
                k += 1;
            }
            break;
        }
        spacing -= 1;
    }
    chosen
}
