//! Overlay color mapping shared by the interactive viewer and the CLI image
//! renderer, so both show exactly the same picture. Rendering never feeds back
//! into the simulation, so this crate may use any float operations.

use accrete_core::world::{Biome, PathLevel, Tile, World};
use accrete_core::{Config, Pos};

pub type Rgb = [u8; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    Biome,
    Elevation,
    Moisture,
    Fertility,
    RiverDistance,
    Food,
    Wood,
    Stone,
    MoveCost,
    Visibility,
    Traffic,
    Depletion,
}

impl Overlay {
    pub const ALL: [Overlay; 12] = [
        Overlay::Biome,
        Overlay::Elevation,
        Overlay::Moisture,
        Overlay::Fertility,
        Overlay::RiverDistance,
        Overlay::Food,
        Overlay::Wood,
        Overlay::Stone,
        Overlay::MoveCost,
        Overlay::Visibility,
        Overlay::Traffic,
        Overlay::Depletion,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Overlay::Biome => "biome",
            Overlay::Elevation => "elevation",
            Overlay::Moisture => "moisture",
            Overlay::Fertility => "fertility",
            Overlay::RiverDistance => "river-distance",
            Overlay::Food => "food",
            Overlay::Wood => "wood",
            Overlay::Stone => "stone",
            Overlay::MoveCost => "move-cost",
            Overlay::Visibility => "visibility",
            Overlay::Traffic => "traffic",
            Overlay::Depletion => "depletion",
        }
    }

    pub fn from_name(s: &str) -> Option<Overlay> {
        Overlay::ALL.iter().copied().find(|o| o.name() == s)
    }

    pub fn next(self) -> Overlay {
        let i = Overlay::ALL.iter().position(|&o| o == self).unwrap_or(0);
        Overlay::ALL[(i + 1) % Overlay::ALL.len()]
    }
}

pub const OCEAN_DEEP: Rgb = [22, 44, 84];
pub const OCEAN_SHALLOW: Rgb = [46, 92, 140];
pub const RIVER: Rgb = [70, 140, 215];

pub fn biome_color(b: Biome) -> Rgb {
    match b {
        Biome::Ocean => OCEAN_SHALLOW,
        Biome::Plains => [150, 178, 92],
        Biome::Forest => [52, 108, 56],
        Biome::Hills => [150, 132, 96],
        Biome::Mountains => [128, 124, 122],
    }
}

fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t) as u8,
    ]
}

fn scale(c: Rgb, k: f32) -> Rgb {
    [
        (c[0] as f32 * k).clamp(0.0, 255.0) as u8,
        (c[1] as f32 * k).clamp(0.0, 255.0) as u8,
        (c[2] as f32 * k).clamp(0.0, 255.0) as u8,
    ]
}

/// Sequential ramp from a dark neutral to a bright hue.
fn ramp(t: f32, hi: Rgb) -> Rgb {
    lerp([28, 30, 34], hi, t)
}

/// Five-stop "viridis-like" ramp for continuous scalar fields.
fn viridis(t: f32) -> Rgb {
    const STOPS: [Rgb; 5] = [
        [68, 1, 84],
        [59, 82, 139],
        [33, 145, 140],
        [94, 201, 98],
        [253, 231, 37],
    ];
    let t = t.clamp(0.0, 1.0) * 4.0;
    let i = (t as usize).min(3);
    lerp(STOPS[i], STOPS[i + 1], t - i as f32)
}

/// Simple hillshade from the elevation gradient (light from the north-west).
fn shade(world: &World, p: Pos) -> f32 {
    let e = |q: Pos| {
        if world.contains(q) {
            world.tile(q).elevation.max(world.sea_level)
        } else {
            world.tile(p).elevation.max(world.sea_level)
        }
    };
    let dzdx = e(p.offset(1, 0)) - e(p.offset(-1, 0));
    let dzdy = e(p.offset(0, 1)) - e(p.offset(0, -1));
    (1.0 - 6.0 * (dzdx + dzdy)).clamp(0.7, 1.25)
}

pub fn tile_color(world: &World, config: &Config, p: Pos, overlay: Overlay) -> Rgb {
    let t: &Tile = world.tile(p);
    let water = || {
        let depth = ((world.sea_level - t.elevation) / world.sea_level.max(1e-3)).clamp(0.0, 1.0);
        lerp(OCEAN_SHALLOW, OCEAN_DEEP, depth * 1.6)
    };
    if !t.is_land() && overlay != Overlay::Elevation {
        return water();
    }
    match overlay {
        Overlay::Biome => {
            if t.is_river {
                return RIVER;
            }
            let mut c = biome_color(t.biome);
            // Lighten with altitude inside a band so relief reads clearly.
            let rel = ((t.elevation - world.sea_level) / (1.0 - world.sea_level)).clamp(0.0, 1.0);
            if t.biome == Biome::Mountains {
                c = lerp(c, [236, 236, 240], ((rel - 0.75) * 3.0).clamp(0.0, 0.85));
            }
            match t.path_level {
                PathLevel::Trail => c = lerp(c, [196, 170, 120], 0.55),
                PathLevel::Road => c = [92, 62, 38],
                PathLevel::None => {}
            }
            scale(c, shade(world, p))
        }
        Overlay::Elevation => viridis(t.elevation),
        Overlay::Moisture => lerp([170, 140, 80], [40, 110, 200], t.moisture),
        Overlay::Fertility => {
            if t.is_river {
                RIVER
            } else {
                ramp(t.fertility, [120, 230, 60])
            }
        }
        Overlay::RiverDistance => {
            if t.is_river {
                RIVER
            } else {
                viridis(1.0 - (t.river_distance as f32 / 12.0).min(1.0))
            }
        }
        Overlay::Food => {
            if t.is_river {
                RIVER
            } else {
                ramp(world.food_at(p) / 30.0, [250, 200, 40])
            }
        }
        Overlay::Wood => ramp(t.wood / 40.0, [60, 200, 80]),
        Overlay::Stone => ramp(t.stone / 200.0, [220, 220, 235]),
        Overlay::MoveCost => match world.enter_cost(p, &config.movement) {
            None => water(),
            Some(c) => viridis(1.0 - ((c as f32 - 5.0) / 75.0).clamp(0.0, 1.0)),
        },
        Overlay::Visibility => {
            if world.vis.masks.is_empty() {
                [0, 0, 0]
            } else {
                viridis(world.vis.visible_count(world.dims.idx(p)) as f32 / 169.0)
            }
        }
        Overlay::Traffic => ramp((t.traffic / 40.0).min(1.0), [255, 120, 40]),
        // Standing food as a share of capacity: red = stripped, green = untouched.
        Overlay::Depletion => {
            if t.food_capacity <= 0.0 {
                scale(biome_color(t.biome), 0.35)
            } else {
                let f = (world.food_at(p) / t.food_capacity).clamp(0.0, 1.0);
                if f < 0.5 {
                    lerp([200, 40, 40], [230, 200, 60], f * 2.0)
                } else {
                    lerp([230, 200, 60], [60, 170, 70], (f - 0.5) * 2.0)
                }
            }
        }
    }
}

/// Render the whole map into an RGB buffer, one pixel per tile, row-major.
pub fn render_tiles(world: &World, config: &Config, overlay: Overlay) -> Vec<Rgb> {
    (0..world.dims.len() as u32)
        .map(|i| tile_color(world, config, world.dims.pos(i), overlay))
        .collect()
}

/// A distinct, stable color per goal category, for agent dots.
pub fn goal_color(index: usize) -> Rgb {
    const PALETTE: [Rgb; 16] = [
        [255, 255, 255], // idle
        [250, 220, 60],  // explore
        [255, 150, 40],  // food
        [140, 90, 50],   // wood
        [180, 180, 200], // stone
        [255, 90, 90],   // eat
        [120, 160, 255], // rest
        [255, 120, 220], // home
        [200, 120, 255], // build storage
        [120, 255, 200], // deposit
        [80, 220, 255],  // withdraw
        [255, 60, 160],  // reproduce
        [230, 200, 150], // improve trail
        [170, 255, 120], // improve shelter
        [80, 230, 255],  // craft
        [255, 215, 80],  // market
    ];
    PALETTE[index % PALETTE.len()]
}
