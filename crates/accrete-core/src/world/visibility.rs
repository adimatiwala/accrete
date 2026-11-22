//! Precomputed line of sight.
//!
//! Each tile stores a 169-bit mask (three `u64`s) over the 13×13 square of
//! radius 6 centred on it. Bit `(dy + 6) * 13 + (dx + 6)` is set when the tile
//! at offset `(dx, dy)` is visible from the centre.
//!
//! **Rule:** a target is visible if no tile strictly between viewer and target
//! on the Bresenham line rises above the straight sight line from the viewer's
//! eye (ground height + `eye_height_m`) to the target's ground. Heights are in
//! metres: land is `(elevation - sea) / (1 - sea) · relief_m`, water is 0. A
//! forest tile *between* the two adds `canopy_m`, so forests hide what lies
//! behind them. Adjacent tiles are always visible.
//!
//! Terrain never changes, so masks are computed once (and rebuilt on load).

use crate::config::SightConfig;
use crate::geom::{Pos, bresenham};
use crate::world::{Biome, World};

pub const SIGHT_RADIUS: i32 = 6;
pub const SIGHT_SIDE: i32 = 2 * SIGHT_RADIUS + 1;
pub const SIGHT_BITS: usize = (SIGHT_SIDE * SIGHT_SIDE) as usize;

pub type Mask = [u64; 3];

#[derive(Clone, Debug, Default)]
pub struct Visibility {
    pub masks: Vec<Mask>,
}

#[inline]
pub fn bit_index(dx: i32, dy: i32) -> usize {
    ((dy + SIGHT_RADIUS) * SIGHT_SIDE + (dx + SIGHT_RADIUS)) as usize
}

#[inline]
pub fn mask_get(m: &Mask, bit: usize) -> bool {
    (m[bit >> 6] >> (bit & 63)) & 1 == 1
}

#[inline]
fn mask_set(m: &mut Mask, bit: usize) {
    m[bit >> 6] |= 1u64 << (bit & 63);
}

/// Ground height in metres.
pub fn ground_height(world: &World, p: Pos, c: &SightConfig) -> f32 {
    let t = world.tile(p);
    if !t.is_land() {
        return 0.0;
    }
    let span = (1.0 - world.sea_level).max(1e-6);
    ((t.elevation - world.sea_level) / span).max(0.0) * c.relief_m
}

/// Height of a tile when it stands between viewer and target.
pub fn blocker_height(world: &World, p: Pos, c: &SightConfig) -> f32 {
    let g = ground_height(world, p, c);
    if world.tile(p).biome == Biome::Forest {
        g + c.canopy_m
    } else {
        g
    }
}

impl Visibility {
    pub fn compute(world: &World, c: &SightConfig) -> Self {
        let dims = world.dims;
        let radius = (c.radius as i32).min(SIGHT_RADIUS);
        let n = dims.len();
        let ground: Vec<f32> = (0..n as u32)
            .map(|i| ground_height(world, dims.pos(i), c))
            .collect();
        let block: Vec<f32> = (0..n as u32)
            .map(|i| blocker_height(world, dims.pos(i), c))
            .collect();
        let mut masks = vec![[0u64; 3]; n];
        let mut line: Vec<Pos> = Vec::with_capacity(SIGHT_SIDE as usize);
        for i in 0..n as u32 {
            let o = dims.pos(i);
            let eye = ground[i as usize] + c.eye_height_m;
            let m = &mut masks[i as usize];
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let t = o.offset(dx, dy);
                    if !dims.contains(t) {
                        continue;
                    }
                    line.clear();
                    bresenham(o, t, |p| {
                        line.push(p);
                        true
                    });
                    let steps = (line.len() - 1) as f32;
                    let target_h = ground[dims.idx(t) as usize];
                    let mut visible = true;
                    for (k, &p) in line
                        .iter()
                        .enumerate()
                        .take(line.len().saturating_sub(1))
                        .skip(1)
                    {
                        let frac = k as f32 / steps;
                        let sight = eye + (target_h - eye) * frac;
                        if block[dims.idx(p) as usize] > sight {
                            visible = false;
                            break;
                        }
                    }
                    if visible {
                        mask_set(m, bit_index(dx, dy));
                    }
                }
            }
        }
        Self { masks }
    }

    #[inline]
    pub fn mask(&self, world_idx: u32) -> &Mask {
        &self.masks[world_idx as usize]
    }

    pub fn is_visible(&self, world: &World, from: Pos, to: Pos) -> bool {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        if dx.abs() > SIGHT_RADIUS || dy.abs() > SIGHT_RADIUS || !world.contains(to) {
            return false;
        }
        mask_get(self.mask(world.dims.idx(from)), bit_index(dx, dy))
    }

    /// Visit every visible tile from `from` (including `from` itself).
    pub fn for_each_visible(&self, world: &World, from: Pos, mut f: impl FnMut(Pos)) {
        let m = self.mask(world.dims.idx(from));
        for (w, &word) in m.iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let bit = w * 64 + b;
                if bit >= SIGHT_BITS {
                    break;
                }
                let dx = (bit as i32 % SIGHT_SIDE) - SIGHT_RADIUS;
                let dy = (bit as i32 / SIGHT_SIDE) - SIGHT_RADIUS;
                f(from.offset(dx, dy));
            }
        }
    }

    pub fn visible_count(&self, world_idx: u32) -> u32 {
        self.mask(world_idx).iter().map(|w| w.count_ones()).sum()
    }
}

/// Reference implementation for tests: decides one pair directly from the
/// definition, with an independent slope formulation (no masks, no bit packing).
pub fn brute_force_visible(world: &World, from: Pos, to: Pos, c: &SightConfig) -> bool {
    let r = (c.radius as i32).min(SIGHT_RADIUS);
    if !world.contains(to) || from.chebyshev(to) > r {
        return false;
    }
    let mut pts = Vec::new();
    bresenham(from, to, |p| {
        pts.push(p);
        true
    });
    let n = pts.len() - 1;
    if n <= 1 {
        return true;
    }
    let eye = ground_height(world, from, c) + c.eye_height_m;
    let target = ground_height(world, to, c);
    for (k, &p) in pts.iter().enumerate().take(n).skip(1) {
        let frac = k as f32 / n as f32;
        if blocker_height(world, p, c) > eye + (target - eye) * frac {
            return false;
        }
    }
    true
}
