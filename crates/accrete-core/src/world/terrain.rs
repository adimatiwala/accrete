//! Elevation, moisture, and biome classification.
//!
//! 1. Elevation = domain-warped fBm gradient noise, pushed down toward the map
//!    edge so the region reads as a landmass with coasts, then normalized.
//! 2. Sea level is the `ocean_fraction` quantile, so every seed has a usable
//!    amount of land. Small enclosed lakes are filled and tiny islands sunk.
//! 3. Hills and mountains are the highest `hill_fraction` / `mountain_fraction`
//!    of land, again by quantile. Remaining land is forest where moisture is
//!    high, plains elsewhere.
//! 4. Moisture = separate fBm, plus a bonus near the coast.
//! 5. A majority filter removes isolated single-tile biome specks.

use std::collections::VecDeque;

use crate::config::WorldConfig;
use crate::geom::{Dims, Pos};
use crate::rng::Rng;
use crate::world::Biome;
use crate::world::noise::{fbm, normalize, quantile};

pub struct Terrain {
    pub elevation: Vec<f32>,
    pub moisture: Vec<f32>,
    pub biome: Vec<Biome>,
    pub sea_level: f32,
    pub hill_level: f32,
    pub mountain_level: f32,
}

pub fn generate(dims: Dims, c: &WorldConfig, rng: &mut Rng) -> Terrain {
    // Seeds are drawn in a fixed order; do not reorder.
    let seed_elev = rng.next_u64();
    let seed_warp_x = rng.next_u64();
    let seed_warp_y = rng.next_u64();
    let seed_moist = rng.next_u64();
    let seed_ridge = rng.next_u64();
    let seed_ridge_mask = rng.next_u64();

    let n = dims.len();
    let w = dims.width as f32;
    let h = dims.height as f32;

    // --- Elevation ---------------------------------------------------------
    let mut elevation = vec![0.0f32; n];
    let inv_scale = 1.0 / c.elevation_scale.max(1.0);
    let warp_scale = inv_scale * 1.5;
    for i in 0..n as u32 {
        let p = dims.pos(i);
        let x = p.x as f32;
        let y = p.y as f32;
        let wx = x + c.warp_strength * 2.0 * fbm(seed_warp_x, x * warp_scale, y * warp_scale, 3);
        let wy = y + c.warp_strength * 2.0 * fbm(seed_warp_y, x * warp_scale, y * warp_scale, 3);
        let base = fbm(
            seed_elev,
            wx * inv_scale,
            wy * inv_scale,
            c.elevation_octaves,
        );
        // Ridged noise forms mountain chains; a low-frequency mask confines
        // them to some regions instead of one central massif.
        let rs = inv_scale * 1.8;
        let ridge = 1.0 - 2.0 * fbm(seed_ridge, wx * rs, wy * rs, 4).abs();
        let ridge = ridge * ridge;
        let mask = smoothstep(
            -0.05,
            0.3,
            fbm(seed_ridge_mask, x * inv_scale * 0.6, y * inv_scale * 0.6, 2),
        );
        // Edge falloff: 0 in the middle, 1 at the border, cubed so only the
        // outer band is pushed under the sea. It blends a square bump with a
        // round one and uses strongly warped coordinates, so the coastline is
        // irregular rather than following the map frame.
        let cx = x + 2.0 * (wx - x);
        let cy = y + 2.0 * (wy - y);
        let nx = (2.0 * cx / (w - 1.0).max(1.0) - 1.0).clamp(-1.0, 1.0);
        let ny = (2.0 * cy / (h - 1.0).max(1.0) - 1.0).clamp(-1.0, 1.0);
        let square = 1.0 - (1.0 - nx * nx) * (1.0 - ny * ny);
        let round = ((nx * nx + ny * ny) * 0.5).sqrt() * 1.0;
        let edge = (0.5 * square + 0.5 * round).min(1.0);
        elevation[i as usize] =
            base + c.ridge_strength * ridge * mask - c.edge_falloff * edge * edge * edge;
    }
    normalize(&mut elevation);

    // --- Sea level, lakes, islands ----------------------------------------
    let sea_level = quantile(&elevation, c.ocean_fraction);
    clean_water_and_islands(dims, &mut elevation, sea_level, c);

    // --- Moisture ----------------------------------------------------------
    let mut moisture = vec![0.0f32; n];
    let inv_m = 1.0 / c.moisture_scale.max(1.0);
    for i in 0..n as u32 {
        let p = dims.pos(i);
        moisture[i as usize] = fbm(
            seed_moist,
            p.x as f32 * inv_m,
            p.y as f32 * inv_m,
            c.moisture_octaves,
        );
    }
    normalize(&mut moisture);
    let coast = distance_to_water(dims, &elevation, sea_level, c.coast_moisture_range);
    for i in 0..n {
        let d = coast[i];
        if elevation[i] >= sea_level && d <= c.coast_moisture_range {
            let range = c.coast_moisture_range.max(1) as f32;
            let k = 1.0 - (d.saturating_sub(1)) as f32 / range;
            moisture[i] = (moisture[i] + c.coast_moisture * k).clamp(0.0, 1.0);
        }
    }

    // --- Biomes ------------------------------------------------------------
    let land: Vec<f32> = elevation
        .iter()
        .copied()
        .filter(|&e| e >= sea_level)
        .collect();
    let mountain_level = quantile(&land, 1.0 - c.mountain_fraction);
    let hill_level = quantile(&land, 1.0 - c.mountain_fraction - c.hill_fraction);
    let mut biome = vec![Biome::Ocean; n];
    for i in 0..n {
        let e = elevation[i];
        biome[i] = if e < sea_level {
            Biome::Ocean
        } else if e >= mountain_level {
            Biome::Mountains
        } else if e >= hill_level {
            Biome::Hills
        } else if moisture[i] >= c.forest_moisture {
            Biome::Forest
        } else {
            Biome::Plains
        };
    }
    for _ in 0..c.smoothing_passes {
        biome = majority_filter(dims, &biome);
    }

    Terrain {
        elevation,
        moisture,
        biome,
        sea_level,
        hill_level,
        mountain_level,
    }
}

/// Cubic Hermite step from 0 at `lo` to 1 at `hi`.
#[inline]
fn smoothstep(lo: f32, hi: f32, v: f32) -> f32 {
    let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Fill enclosed water bodies smaller than `min_lake_tiles` and submerge land
/// masses smaller than `min_island_tiles`. Uses 4-connectivity, which matches
/// movement connectivity (diagonal steps never cut corners).
fn clean_water_and_islands(dims: Dims, elev: &mut [f32], sea: f32, c: &WorldConfig) {
    let nudge = 1.0e-4;
    // Lakes first.
    for comp in components(dims, elev, |e| e < sea) {
        let touches_border = comp.iter().any(|&i| dims.is_border(dims.pos(i)));
        if !touches_border && (comp.len() as u32) < c.min_lake_tiles {
            for &i in &comp {
                elev[i as usize] = sea + nudge;
            }
        }
    }
    // Then islands.
    for comp in components(dims, elev, |e| e >= sea) {
        if (comp.len() as u32) < c.min_island_tiles {
            for &i in &comp {
                elev[i as usize] = (sea - nudge).max(0.0);
            }
        }
    }
}

/// 4-connected components of tiles whose elevation satisfies `pred`,
/// discovered in ascending tile order.
pub fn components(dims: Dims, elev: &[f32], pred: impl Fn(f32) -> bool) -> Vec<Vec<u32>> {
    let n = dims.len();
    let mut seen = vec![false; n];
    let mut out = Vec::new();
    let mut queue = VecDeque::new();
    for start in 0..n {
        if seen[start] || !pred(elev[start]) {
            continue;
        }
        let mut comp = Vec::new();
        seen[start] = true;
        queue.push_back(start as u32);
        while let Some(i) = queue.pop_front() {
            comp.push(i);
            for q in dims.neighbors4(dims.pos(i)) {
                let j = dims.idx(q) as usize;
                if !seen[j] && pred(elev[j]) {
                    seen[j] = true;
                    queue.push_back(j as u32);
                }
            }
        }
        out.push(comp);
    }
    out
}

/// Chebyshev distance from each tile to the nearest water tile, capped at `cap + 1`.
fn distance_to_water(dims: Dims, elev: &[f32], sea: f32, cap: u32) -> Vec<u32> {
    let n = dims.len();
    let mut dist = vec![u32::MAX; n];
    let mut queue = VecDeque::new();
    for i in 0..n {
        if elev[i] < sea {
            dist[i] = 0;
            queue.push_back(i as u32);
        }
    }
    while let Some(i) = queue.pop_front() {
        let d = dist[i as usize];
        if d > cap {
            continue;
        }
        for q in dims.neighbors8(dims.pos(i)) {
            let j = dims.idx(q) as usize;
            if dist[j] == u32::MAX {
                dist[j] = d + 1;
                queue.push_back(j as u32);
            }
        }
    }
    dist
}

/// One pass of a land-only majority filter: a land tile with at most one
/// same-biome neighbour adopts the most common land biome among its
/// neighbours (ties broken by biome order). Ocean is never changed, and land
/// never becomes ocean. Reads the old grid and writes a new one, so the result
/// does not depend on iteration order.
fn majority_filter(dims: Dims, biome: &[Biome]) -> Vec<Biome> {
    let mut out = biome.to_vec();
    for i in 0..dims.len() as u32 {
        let b = biome[i as usize];
        if !b.is_land() {
            continue;
        }
        let p: Pos = dims.pos(i);
        let mut counts = [0u8; 5];
        let mut same = 0;
        for q in dims.neighbors8(p) {
            let nb = biome[dims.idx(q) as usize];
            if nb == b {
                same += 1;
            }
            if nb.is_land() {
                counts[nb as usize] += 1;
            }
        }
        if same <= 1 {
            let mut best = b;
            let mut best_count = 0;
            for cand in [Biome::Plains, Biome::Forest, Biome::Hills, Biome::Mountains] {
                if counts[cand as usize] > best_count {
                    best = cand;
                    best_count = counts[cand as usize];
                }
            }
            out[i as usize] = best;
        }
    }
    out
}
