//! Droughts and seasons, applied only through ordinary world mechanics: they
//! scale daily food regrowth. Nothing here touches agents directly.
//!
//! **Random droughts** (accrete.md, "Random droughts"): checked once per day
//! on the dedicated `weather` stream when no random drought is active and the
//! cooldown has passed; chance 1/150 per day, radius 6–16 tiles centred on a
//! random land tile, 10–40 days, regrowth reduced by 30–70 %, then a 60-day
//! cooldown. **Scripted droughts** come from the experiment config and never
//! consume `weather` draws, so a branch with a scripted drought keeps the same
//! random droughts as its control.
//!
//! **Seasons** (optional, off by default): regrowth × `1 ± amplitude` along a
//! triangle wave (no trigonometry, so it is exactly reproducible).

use serde::{Deserialize, Serialize};

use crate::config::{Config, WeatherConfig};
use crate::geom::Pos;
use crate::hash::{StableHash, StableHasher};
use crate::rng::Rng;
use crate::world::World;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drought {
    pub center: Pos,
    pub radius: u32,
    pub start_day: u32,
    /// First day after the drought.
    pub end_day: u32,
    pub severity: f32,
    pub scripted: bool,
}

impl Drought {
    pub fn covers(&self, p: Pos) -> bool {
        let r = self.radius as i32;
        p.dist2(self.center) <= r * r
    }

    pub fn active_on(&self, day: u32) -> bool {
        day >= self.start_day && day < self.end_day
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WeatherState {
    /// Droughts in progress (at most one random, plus any scripted).
    pub active: Vec<Drought>,
    /// No random drought may start before this day.
    pub cooldown_until: u32,
    /// Every drought that has started, for logs and validation.
    pub history: Vec<Drought>,
}

impl StableHash for WeatherState {
    fn stable_hash(&self, h: &mut StableHasher) {
        h.u32(self.cooldown_until);
        for list in [&self.active, &self.history] {
            h.len(list.len());
            for d in list {
                d.center.stable_hash(h);
                h.u32(d.radius);
                h.u32(d.start_day);
                h.u32(d.end_day);
                h.f32(d.severity);
                h.bool(d.scripted);
            }
        }
    }
}

/// Seasonal regrowth multiplier on `day` (triangle wave, period = 2 seasons
/// of `season_length_days`, peaking mid "summer").
pub fn season_factor(day: u32, c: &WeatherConfig) -> f32 {
    if c.seasons_amplitude <= 0.0 || c.season_length_days == 0 {
        return 1.0;
    }
    let period = 2 * c.season_length_days;
    let phase = (day % period) as f32 / period as f32; // [0, 1)
    // Triangle in [-1, 1]: -1 at phase 0, +1 at phase 0.5.
    let tri = if phase < 0.5 {
        4.0 * phase - 1.0
    } else {
        3.0 - 4.0 * phase
    };
    1.0 + c.seasons_amplitude * tri
}

impl WeatherState {
    /// Advance to `day` (called once per day, before regrowth). Returns
    /// droughts that started and ended today.
    pub fn update(
        &mut self,
        day: u32,
        world: &World,
        config: &Config,
        rng: &mut Rng,
    ) -> (Vec<Drought>, Vec<Drought>) {
        let c = &config.weather;
        let mut ended = Vec::new();
        let mut i = 0;
        while i < self.active.len() {
            if day >= self.active[i].end_day {
                let d = self.active.remove(i);
                if !d.scripted {
                    self.cooldown_until = day + c.cooldown_days;
                }
                ended.push(d);
            } else {
                i += 1;
            }
        }
        let mut started = Vec::new();
        for s in &c.scripted {
            if s.start_day == day {
                let d = Drought {
                    center: Pos::new(s.center[0], s.center[1]),
                    radius: s.radius,
                    start_day: day,
                    end_day: day + s.days,
                    severity: s.severity.clamp(0.0, 1.0),
                    scripted: true,
                };
                self.active.push(d.clone());
                started.push(d);
            }
        }
        let random_active = self.active.iter().any(|d| !d.scripted);
        if c.random_droughts
            && !random_active
            && day >= self.cooldown_until
            && rng.chance(c.daily_chance)
        {
            // Centre on a random land tile (bounded retries keep draws finite).
            let n = world.dims.len() as u32;
            let mut center = None;
            for _ in 0..256 {
                let p = world.dims.pos(rng.below(n));
                if world.tile(p).is_land() {
                    center = Some(p);
                    break;
                }
            }
            if let Some(center) = center {
                let radius = rng.range_u32_inclusive(c.radius_min, c.radius_max.max(c.radius_min));
                let len = rng.range_u32_inclusive(
                    c.length_min_days,
                    c.length_max_days.max(c.length_min_days),
                );
                let severity = rng.range_f32(c.severity_min, c.severity_max);
                let d = Drought {
                    center,
                    radius,
                    start_day: day,
                    end_day: day + len,
                    severity,
                    scripted: false,
                };
                self.active.push(d.clone());
                started.push(d);
            }
        }
        self.history.extend(started.iter().cloned());
        (started, ended)
    }

    /// Food-regrowth multiplier per tile index for `day`.
    pub fn regrowth_factors(&self, world: &World, day: u32, c: &WeatherConfig) -> Vec<f32> {
        let season = season_factor(day, c);
        let mut f = vec![season; world.dims.len()];
        for d in &self.active {
            let r = d.radius as i32;
            for dy in -r..=r {
                for dx in -r..=r {
                    let p = d.center.offset(dx, dy);
                    if world.contains(p) && d.covers(p) {
                        f[world.dims.idx(p) as usize] *= 1.0 - d.severity;
                    }
                }
            }
        }
        f
    }

    pub fn in_drought(&self, p: Pos) -> bool {
        self.active.iter().any(|d| d.covers(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seasons_are_a_bounded_triangle() {
        let c = WeatherConfig {
            seasons_amplitude: 0.3,
            season_length_days: 10,
            ..WeatherConfig::default()
        };
        let vals: Vec<f32> = (0..20).map(|d| season_factor(d, &c)).collect();
        assert!((vals[0] - 0.7).abs() < 1e-6);
        assert!((vals[10] - 1.3).abs() < 1e-6);
        assert!(vals.iter().all(|v| (0.7..=1.3).contains(v)));
        assert_eq!(season_factor(3, &WeatherConfig::default()), 1.0);
    }
}
