//! Movement-point (MP) costs.
//!
//! A moving agent gains `mp_per_tick` (10) MP per tick; entering a tile costs
//! that tile's MP, and unused MP carries over while the agent keeps moving.
//!
//! | Terrain | MP | | Modifier | Effect |
//! |---|---|---|---|---|
//! | Plains | 10 | | River tile | + 20 (fording) |
//! | Forest | 20 | | Worn trail | × 0.8, rounded down |
//! | Hills | 30 | | Road | × 0.5, rounded down |
//! | Mountains | 60 | | Diagonal step | × 1.4, rounded down |
//! | Ocean | impassable | | Floor | `min_mp` (5) |
//!
//! All arithmetic is integer, so travel time is exactly reproducible.

use crate::config::MovementConfig;
use crate::world::{Biome, PathLevel, Tile};

/// MP to enter `t` by an orthogonal step, or `None` if impassable.
pub fn enter_cost(t: &Tile, c: &MovementConfig) -> Option<u32> {
    cost_for(t.biome, t.is_river, t.path_level, c)
}

pub fn cost_for(biome: Biome, river: bool, path: PathLevel, c: &MovementConfig) -> Option<u32> {
    let base = match biome {
        Biome::Ocean => return None,
        Biome::Plains => c.plains_mp,
        Biome::Forest => c.forest_mp,
        Biome::Hills => c.hills_mp,
        Biome::Mountains => c.mountains_mp,
    };
    let raw = base + if river { c.river_extra_mp } else { 0 };
    let modified = match path {
        PathLevel::None => raw,
        PathLevel::Trail => raw * 8 / 10,
        PathLevel::Road => raw / 2,
    };
    Some(modified.max(c.min_mp))
}

/// Cost of a step into a tile with orthogonal cost `cost`.
#[inline]
pub fn step_cost(cost: u32, diagonal: bool, c: &MovementConfig) -> u32 {
    if diagonal {
        cost * c.diagonal_num / 10
    } else {
        cost
    }
}

#[inline]
pub fn passable(t: &Tile) -> bool {
    t.is_land()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_spec() {
        let c = MovementConfig::default();
        assert_eq!(
            cost_for(Biome::Plains, false, PathLevel::None, &c),
            Some(10)
        );
        assert_eq!(
            cost_for(Biome::Forest, false, PathLevel::None, &c),
            Some(20)
        );
        assert_eq!(cost_for(Biome::Hills, false, PathLevel::None, &c), Some(30));
        assert_eq!(
            cost_for(Biome::Mountains, false, PathLevel::None, &c),
            Some(60)
        );
        assert_eq!(cost_for(Biome::Plains, true, PathLevel::None, &c), Some(30));
        assert_eq!(cost_for(Biome::Ocean, false, PathLevel::None, &c), None);
        assert_eq!(
            cost_for(Biome::Plains, false, PathLevel::Trail, &c),
            Some(8)
        );
        assert_eq!(cost_for(Biome::Plains, false, PathLevel::Road, &c), Some(5));
        assert_eq!(step_cost(10, true, &c), 14);
    }
}
