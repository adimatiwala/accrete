//! Roads and shelters (Week 3): built by individual agents, tile by tile.
//!
//! `ImproveTrail` turns the worn trail an agent stands on into a road (MP ×
//! 0.5) for 2 wood, 2 stone and 4 ticks of work, when the agent's perceived
//! benefit exceeds the cost (see `decision::road_value`). Roads start at
//! condition 180 and lose one point per day without traffic; at 0 they revert
//! to a trail (if still busy) or bare ground.
//!
//! `ImproveShelter` raises a home's shelter level (max 2) for 20 wood, 10 stone
//! and 12 ticks: +2 comfortable capacity and +10 % rest recovery.

use crate::agents::agent::Goal;
use crate::inventory::Resource;
use crate::simulation::Simulation;
use crate::simulation::events::EventKind;
use crate::world::PathLevel;

impl Simulation {
    pub(crate) fn do_improve_trail(&mut self, i: usize, tick: u64) {
        let ic = self.config.infrastructure.clone();
        let pos = self.agents[i].pos;
        let a = &mut self.agents[i];
        let tile_ok = self.world.tile(pos).path_level == PathLevel::Trail;
        if !ic.roads_enabled
            || !tile_ok
            || a.inventory.wood < ic.road_wood
            || a.inventory.stone < ic.road_stone
        {
            a.goal = Goal::idle(a.pos, tick);
            return;
        }
        a.goal.work += 1;
        a.pathology.last_progress_tick = tick;
        if a.goal.work < ic.road_ticks {
            return;
        }
        let w = a.inventory.take(Resource::Wood, ic.road_wood);
        let s = a.inventory.take(Resource::Stone, ic.road_stone);
        self.ledger.construction[Resource::Wood.index()] += f64::from(w);
        self.ledger.construction[Resource::Stone.index()] += f64::from(s);
        a.stats.roads_built += 1;
        a.goal = Goal::idle(a.pos, tick);
        let id = a.id;
        let t = self.world.tile_mut(pos);
        t.path_level = PathLevel::Road;
        t.road_condition = ic.road_condition.min(u32::from(u16::MAX)) as u16;
        t.trail_fade_days = 0;
        self.today.roads_built += 1;
        self.events.push(tick, Some(id), pos, EventKind::RoadBuilt);
    }

    pub(crate) fn do_improve_shelter(&mut self, i: usize, tick: u64) {
        let ic = self.config.infrastructure.clone();
        let a = &mut self.agents[i];
        let Some(hid) = a.home else {
            a.goal = Goal::idle(a.pos, tick);
            return;
        };
        let home = &mut self.world.homes[hid as usize];
        let stored = home.stored();
        if home.pos != a.pos
            || u32::from(home.shelter_level) >= ic.max_shelter
            || a.inventory.wood + stored.wood < ic.shelter_wood
            || a.inventory.stone + stored.stone < ic.shelter_stone
        {
            a.goal = Goal::idle(a.pos, tick);
            return;
        }
        a.goal.work += 1;
        a.pathology.last_progress_tick = tick;
        if a.goal.work < ic.shelter_ticks {
            return;
        }
        // Materials come from the builder's hands first, then the household chests.
        for (r, need) in [
            (Resource::Wood, ic.shelter_wood),
            (Resource::Stone, ic.shelter_stone),
        ] {
            let mut used = a.inventory.take(r, need);
            for chest in home.chests.iter_mut() {
                if used >= need {
                    break;
                }
                used += chest.take(r, need - used);
            }
            self.ledger.construction[r.index()] += f64::from(used);
        }
        home.shelter_level += 1;
        let level = home.shelter_level;
        a.stats.shelters_built += 1;
        a.goal = Goal::idle(a.pos, tick);
        self.today.shelters_built += 1;
        self.events.push(
            tick,
            Some(a.id),
            a.pos,
            EventKind::ShelterImproved { home: hid, level },
        );
    }

    /// Daily road upkeep: roads without traffic today lose condition; worn-out
    /// roads revert. Also clears the per-day crossing flags.
    pub(crate) fn daily_roads(&mut self, tick: u64) {
        let threshold = self.config.trails.threshold;
        let mut worn = Vec::new();
        for (i, t) in self.world.tiles.iter_mut().enumerate() {
            if t.path_level == PathLevel::Road && !t.crossed_today {
                t.road_condition = t.road_condition.saturating_sub(1);
                if t.road_condition == 0 {
                    t.path_level = if t.traffic >= threshold {
                        PathLevel::Trail
                    } else {
                        PathLevel::None
                    };
                    worn.push(i as u32);
                }
            }
            t.crossed_today = false;
        }
        for i in worn {
            let p = self.world.dims.pos(i);
            self.events.push(tick, None, p, EventKind::RoadWornAway);
        }
    }
}
