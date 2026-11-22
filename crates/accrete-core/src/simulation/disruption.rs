//! Route disruptions (accrete.md, Week 4 Day 2): an experiment makes tiles
//! temporarily impassable (a flood, a landslide, a washed-out ford) or slow,
//! through ordinary movement costs only.
//!
//! Nothing here touches agents. Agents keep routing by what they believe; they
//! learn of a disruption when the tile comes into sight (the changed-tile
//! memory records it) or when a planned step turns out to be blocked, and
//! re-plan from there. Restored tiles look normal again when next seen.
//!
//! Disruptions start and end in the daily phase, like droughts. The state here
//! is authoritative (hashed and saved in snapshots); `World::disruption_at`
//! is a derived per-tile index rebuilt from it.

use serde::{Deserialize, Serialize};

use crate::config::{DisruptionKind, ScriptedDisruption};
use crate::hash::{StableHash, StableHasher};
use crate::world::{Disruption, World};

/// A disruption in progress.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveDisruption {
    /// Affected tile indices, ascending.
    pub tiles: Vec<u32>,
    pub effect: Disruption,
    pub start_day: u32,
    /// First day after the disruption.
    pub end_day: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DisruptionState {
    pub active: Vec<ActiveDisruption>,
    /// Every disruption that has started (for logs and analysis).
    pub history: Vec<ActiveDisruption>,
}

impl StableHash for DisruptionState {
    fn stable_hash(&self, h: &mut StableHasher) {
        for list in [&self.active, &self.history] {
            h.len(list.len());
            for d in list {
                h.len(d.tiles.len());
                for &t in &d.tiles {
                    h.u32(t);
                }
                h.u8(d.effect.code());
                h.u32(d.start_day);
                h.u32(d.end_day);
            }
        }
    }
}

/// Tiles a scripted disruption affects, given the world on its start day.
pub fn select_tiles(world: &World, s: &ScriptedDisruption) -> Vec<u32> {
    let mut tiles: Vec<u32> = if s.busiest_tiles > 0 {
        let mut trails: Vec<(f32, u32)> = world
            .tiles
            .iter()
            .enumerate()
            // Routes, not settlements: tiles with homes on them are skipped.
            .filter(|&(i, t)| {
                t.is_land()
                    && t.traffic > 0.0
                    && world.homes_on(world.dims.pos(i as u32)).is_empty()
            })
            .map(|(i, t)| (t.traffic, i as u32))
            .collect();
        // Busiest first; ties by tile index.
        trails.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        trails
            .into_iter()
            .take(s.busiest_tiles as usize)
            .map(|(_, i)| i)
            .collect()
    } else {
        let r = s.radius.min(world.dims.width.max(world.dims.height)) as i32;
        let c = crate::geom::Pos::new(s.center[0], s.center[1]);
        let mut v = Vec::new();
        for dy in -r..=r {
            for dx in -r..=r {
                let p = c.offset(dx, dy);
                if world.contains(p) && p.dist2(c) <= r * r && world.tile(p).is_land() {
                    v.push(world.dims.idx(p));
                }
            }
        }
        v
    };
    tiles.sort_unstable();
    tiles.dedup();
    tiles
}

fn effect_of(s: &ScriptedDisruption) -> Disruption {
    match s.kind {
        DisruptionKind::Block => Disruption::Blocked,
        DisruptionKind::Slow => Disruption::Slowed {
            factor: s.slow_factor.max(1),
        },
    }
}

impl DisruptionState {
    /// Advance to `day` (once per day, in the daily phase): end expired
    /// disruptions and start scheduled ones. Returns (started, ended).
    pub fn update(
        &mut self,
        day: u32,
        world: &mut World,
        scripted: &[ScriptedDisruption],
    ) -> (Vec<ActiveDisruption>, Vec<ActiveDisruption>) {
        let mut ended = Vec::new();
        let mut i = 0;
        while i < self.active.len() {
            if day >= self.active[i].end_day {
                ended.push(self.active.remove(i));
            } else {
                i += 1;
            }
        }
        let mut started = Vec::new();
        for s in scripted.iter().filter(|s| s.start_day == day && s.days > 0) {
            let d = ActiveDisruption {
                tiles: select_tiles(world, s),
                effect: effect_of(s),
                start_day: day,
                end_day: day + s.days,
            };
            self.active.push(d.clone());
            self.history.push(d.clone());
            started.push(d);
        }
        if !started.is_empty() || !ended.is_empty() {
            self.rebuild_index(world);
        }
        (started, ended)
    }

    /// Rebuild `World::disruption_at` from the active disruptions (after any
    /// change, and after loading a snapshot).
    pub fn rebuild_index(&self, world: &mut World) {
        if self.active.is_empty() {
            world.disruption_at.clear();
            return;
        }
        let mut idx = vec![Disruption::None; world.dims.len()];
        for d in &self.active {
            for &t in &d.tiles {
                idx[t as usize] = idx[t as usize].worst(d.effect);
            }
        }
        world.disruption_at = idx;
    }

    /// Number of tiles currently disrupted.
    pub fn disrupted_tiles(world: &World) -> u32 {
        world
            .disruption_at
            .iter()
            .filter(|d| **d != Disruption::None)
            .count() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::geom::Pos;

    fn disc(center: [i32; 2], radius: u32) -> ScriptedDisruption {
        ScriptedDisruption {
            start_day: 3,
            days: 5,
            kind: DisruptionKind::Block,
            slow_factor: 3,
            center,
            radius,
            busiest_tiles: 0,
        }
    }

    #[test]
    fn area_selection_is_land_within_radius() {
        let c = Config::default();
        let world = crate::world::generate_world(1, &c);
        let land = world.spawns[0];
        let tiles = select_tiles(&world, &disc([land.x, land.y], 2));
        assert!(!tiles.is_empty());
        for &t in &tiles {
            let p = world.dims.pos(t);
            assert!(p.dist2(land) <= 4 && world.tile(p).is_land());
        }
        assert!(
            tiles.windows(2).all(|w| w[0] < w[1]),
            "sorted, no duplicates"
        );
    }

    #[test]
    fn busiest_selection_prefers_traffic_and_skips_homes() {
        let c = Config::default();
        let mut sim = crate::Simulation::new(1, c);
        sim.run(30 * 24);
        let mut s = disc([0, 0], 0);
        s.busiest_tiles = 10;
        let tiles = select_tiles(&sim.world, &s);
        assert_eq!(tiles.len(), 10);
        let min_chosen = tiles
            .iter()
            .map(|&t| sim.world.tiles[t as usize].traffic)
            .fold(f32::INFINITY, f32::min);
        let busier_unchosen = sim.world.tiles.iter().enumerate().any(|(i, t)| {
            !tiles.contains(&(i as u32))
                && t.traffic > min_chosen
                && sim.world.homes_on(sim.world.dims.pos(i as u32)).is_empty()
        });
        assert!(!busier_unchosen, "a busier homeless tile was left out");
        for &t in &tiles {
            assert!(sim.world.homes_on(sim.world.dims.pos(t)).is_empty());
        }
    }

    #[test]
    fn index_keeps_the_worst_overlapping_effect_and_clears_on_end() {
        let c = Config::default();
        let mut world = crate::world::generate_world(1, &c);
        let p = world.spawns[0];
        let mut state = DisruptionState::default();
        let mut slow = disc([p.x, p.y], 1);
        slow.kind = DisruptionKind::Slow;
        let block = disc([p.x, p.y], 0);
        state.update(3, &mut world, &[slow, block]);
        assert_eq!(world.disruption(p), Disruption::Blocked);
        assert_eq!(
            world.disruption(Pos::new(p.x + 1, p.y)),
            Disruption::Slowed { factor: 3 }
        );
        assert_eq!(world.enter_cost(p, &c.movement), None);
        state.update(8, &mut world, &[]);
        assert!(state.active.is_empty());
        assert_eq!(world.disruption(p), Disruption::None);
        assert!(world.disruption_at.is_empty());
        assert_eq!(state.history.len(), 2);
    }

    #[test]
    fn huge_radius_is_clamped_to_the_map() {
        let c = Config::default();
        let world = crate::world::generate_world(1, &c);
        let land = (0..world.tiles.len())
            .filter(|&i| world.tiles[i].is_land())
            .count();
        assert_eq!(select_tiles(&world, &disc([64, 64], u32::MAX)).len(), land);
    }

    #[test]
    fn invalid_disruptions_are_rejected() {
        assert!(disc([0, 0], 3).validate().is_ok());
        let mut zero_days = disc([0, 0], 3);
        zero_days.days = 0;
        let mut both = disc([0, 0], 3);
        both.busiest_tiles = 5;
        for bad_factor in [0, 1, 255] {
            let mut s = disc([0, 0], 3);
            s.kind = DisruptionKind::Slow;
            s.slow_factor = bad_factor;
            assert!(s.validate().is_err(), "slow factor {bad_factor}");
        }
        assert!(zero_days.validate().is_err());
        assert!(both.validate().is_err());
        let toml = "[[disruption.scripted]]\nstart_day = 5\ndays = 0\n";
        assert!(Config::from_toml_str(toml).is_err());
        let mut sim = crate::Simulation::new(1, Config::default());
        let mut cfg = sim.config.clone();
        cfg.disruption.scripted.push(both);
        assert!(sim.set_config(cfg).is_err());
    }
}
