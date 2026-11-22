//! Snapshots: save the complete simulation state and resume it exactly
//! (accrete.md, Week 4 Day 1).
//!
//! A snapshot is a versioned JSON document (`*.snap.json`). It holds every
//! piece of state that can influence later ticks or the exported metrics:
//! tick, configuration, world, agents (with their memories), every RNG stream,
//! weather, the resource ledger, settlement tracking, the day's counters, the
//! daily metrics so far, death records and pathology counts.
//!
//! **Not saved**, because they never affect results:
//! - derived data, rebuilt on load: line-of-sight masks and the tile-to-homes
//!   index (`World::rebuild_derived`), agents' exploration-ray caches, and the
//!   per-tick rates (`TickRates`, recomputed from the config);
//! - debugging and inspection buffers, which start empty after a load: the
//!   rolling event log, the graveyard of recently dead agents, the latest
//!   decision traces and intents, and the bounded pathology log;
//! - pathfinding scratch buffers.
//!
//! Floats round-trip exactly (`serde_json` is built with `float_roundtrip`),
//! so a loaded simulation continues bit-identically, on any platform. Every
//! snapshot records the state hash at save time, and loading recomputes it: a
//! file that was edited, truncated, or written by an incompatible version is
//! rejected instead of silently diverging.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agents::agent::Agent;
use crate::config::{Config, TickRates};
use crate::hash;
use crate::rng::Rng;
use crate::simulation::Simulation;
use crate::simulation::events::{DeathRecord, EventLog, Ledger};
use crate::simulation::metrics::{DailyMetrics, DayCounters};
use crate::simulation::settlement::SettlementTracker;
use crate::simulation::weather::WeatherState;
use crate::world::World;

/// Snapshot format version. Bump it whenever the serialized layout or the
/// meaning of a saved field changes; older files are then rejected.
pub const SNAPSHOT_FORMAT: u32 = 1;

/// Everything needed to resume a simulation exactly.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub format: u32,
    /// `accrete-core` version that wrote the file (informational).
    pub accrete_version: String,
    /// State hash at save time (hex); checked again on load.
    pub state_hash: String,
    pub tick: u64,
    pub config: Config,
    pub world: World,
    pub agents: Vec<Agent>,
    pub next_id: u32,
    pub rng_agents: Rng,
    pub rng_births: Rng,
    pub rng_conflicts: Rng,
    pub rng_weather: Rng,
    pub weather: WeatherState,
    pub ledger: Ledger,
    pub settlements: SettlementTracker,
    pub today: DayCounters,
    pub metrics: Vec<DailyMetrics>,
    pub deaths: Vec<DeathRecord>,
    pub pathologies: [u64; 4],
    pub hit_cap: bool,
    pub total_births: u32,
    pub total_deaths: [u32; 2],
    pub lifespans: (f64, u32),
}

impl Simulation {
    /// Capture the current state.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            format: SNAPSHOT_FORMAT,
            accrete_version: env!("CARGO_PKG_VERSION").to_string(),
            state_hash: hash::hex(self.state_hash()),
            tick: self.tick,
            config: self.config.clone(),
            world: self.world.clone(),
            agents: self.agents.clone(),
            next_id: self.next_id,
            rng_agents: self.rng_agents.clone(),
            rng_births: self.rng_births.clone(),
            rng_conflicts: self.rng_conflicts.clone(),
            rng_weather: self.rng_weather.clone(),
            weather: self.weather.clone(),
            ledger: self.ledger.clone(),
            settlements: self.settlements.clone(),
            today: self.today.clone(),
            metrics: self.metrics.clone(),
            deaths: self.deaths.clone(),
            pathologies: self.pathologies,
            hit_cap: self.hit_cap,
            total_births: self.total_births,
            total_deaths: self.total_deaths,
            lifespans: self.lifespans,
        }
    }

    /// Rebuild a simulation from a snapshot, checking its format version and
    /// that the restored state hashes to what was saved.
    pub fn from_snapshot(s: Snapshot) -> Result<Self, String> {
        if s.format != SNAPSHOT_FORMAT {
            return Err(format!(
                "snapshot format {} is not supported (this build reads format {SNAPSHOT_FORMAT}); \
                 it was written by accrete {}",
                s.format, s.accrete_version
            ));
        }
        let expected = s.state_hash.clone();
        let mut world = s.world;
        world.rebuild_derived(&s.config);
        let mut sim = Self::from_world_parts(world, s.config, s.agents);
        sim.tick = s.tick;
        sim.next_id = s.next_id;
        sim.rng_agents = s.rng_agents;
        sim.rng_births = s.rng_births;
        sim.rng_conflicts = s.rng_conflicts;
        sim.rng_weather = s.rng_weather;
        sim.weather = s.weather;
        sim.ledger = s.ledger;
        sim.settlements = s.settlements;
        sim.today = s.today;
        sim.metrics = s.metrics;
        sim.deaths = s.deaths;
        sim.pathologies = s.pathologies;
        sim.hit_cap = s.hit_cap;
        sim.total_births = s.total_births;
        sim.total_deaths = s.total_deaths;
        sim.lifespans = s.lifespans;
        let actual = hash::hex(sim.state_hash());
        if actual != expected {
            return Err(format!(
                "snapshot state hash mismatch: file says {expected}, restored state hashes to \
                 {actual} (the file was modified, or written by an incompatible build)"
            ));
        }
        Ok(sim)
    }

    /// Serialize the current state to JSON.
    pub fn snapshot_json(&self) -> String {
        serde_json::to_string(&self.snapshot()).expect("snapshot serializes")
    }

    /// Restore from JSON produced by [`Simulation::snapshot_json`].
    pub fn from_snapshot_json(text: &str) -> Result<Self, String> {
        let s: Snapshot =
            serde_json::from_str(text).map_err(|e| format!("snapshot does not parse: {e}"))?;
        Self::from_snapshot(s)
    }

    /// Write a snapshot file (conventionally `*.snap.json`).
    pub fn save_snapshot(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        }
        std::fs::write(path, self.snapshot_json())
            .map_err(|e| format!("writing {}: {e}", path.display()))
    }

    /// Read and restore a snapshot file.
    pub fn load_snapshot(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        Self::from_snapshot_json(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Replace the configuration mid-run, for an intervention branch (for
    /// example adding a scripted drought, or changing a regrowth rate).
    /// Per-tick rates and derived world data are rebuilt from the new config.
    /// Interventions act only through the config, never on agents directly.
    pub fn set_config(&mut self, config: Config) {
        self.rates = TickRates::new(&config);
        self.world.rebuild_derived(&config);
        self.events = EventLog::new(config.sim.event_log_capacity);
        self.config = config;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short_config() -> Config {
        let mut c = Config::default();
        c.sim.check_invariants = true;
        c
    }

    #[test]
    fn snapshot_round_trips_byte_identically() {
        let mut sim = Simulation::new(3, short_config());
        sim.run(24 * 3);
        let a = sim.snapshot_json();
        let restored = Simulation::from_snapshot_json(&a).expect("loads");
        assert_eq!(restored.state_hash(), sim.state_hash());
        assert_eq!(restored.snapshot_json(), a, "re-saving is byte-identical");
    }

    #[test]
    fn resumed_run_matches_uninterrupted_run() {
        let mut straight = Simulation::new(3, short_config());
        straight.run(200);
        let mut first = Simulation::new(3, short_config());
        first.run(80);
        let mut resumed = Simulation::from_snapshot_json(&first.snapshot_json()).expect("loads");
        resumed.run(120);
        assert_eq!(resumed.tick, straight.tick);
        assert_eq!(resumed.state_hash(), straight.state_hash());
        assert_eq!(resumed.metrics, straight.metrics);
        assert_eq!(resumed.deaths, straight.deaths);
    }

    #[test]
    fn tampered_or_wrong_format_snapshots_are_rejected() {
        let sim = Simulation::new(3, short_config());
        let mut s = sim.snapshot();
        s.format = SNAPSHOT_FORMAT + 1;
        let e = Simulation::from_snapshot(s).err().expect("rejected");
        assert!(e.contains("format"), "{e}");

        let mut s = sim.snapshot();
        s.agents[0].hunger += 0.25;
        let e = Simulation::from_snapshot(s).err().expect("rejected");
        assert!(e.contains("hash mismatch"), "{e}");

        let e = Simulation::from_snapshot_json("{not json")
            .err()
            .expect("rejected");
        assert!(e.contains("does not parse"), "{e}");
    }
}
