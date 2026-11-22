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
//!   index (`World::rebuild_derived`), the per-tile disruption index
//!   (`DisruptionState::rebuild_index` — not `rebuild_derived`), agents'
//!   exploration-ray caches, and the per-tick rates (`TickRates`, recomputed
//!   from the config);
//! - debugging and inspection buffers, which start empty after a load: the
//!   rolling event log, the graveyard of recently dead agents, the latest
//!   decision traces and intents, and the bounded pathology log;
//! - pathfinding scratch buffers.
//!
//! Floats round-trip exactly (`serde_json` is built with `float_roundtrip`),
//! so a loaded simulation continues bit-identically, on any platform.
//!
//! Integrity: every snapshot records the simulation's state hash and a hash
//! of its whole serialized body (metrics, ledger, settlements and other
//! bookkeeping included). Loading checks both, so a file that was edited,
//! truncated, or written by an incompatible build is rejected instead of
//! silently diverging, with a clear message when the config schema differs.
//! Saving checks that the file will load (JSON cannot hold NaN or infinity),
//! so an unloadable snapshot is never written.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agents::agent::Agent;
use crate::config::{Config, TickRates};
use crate::hash;
use crate::rng::Rng;
use crate::simulation::Simulation;
use crate::simulation::disruption::DisruptionState;
use crate::simulation::events::{DeathRecord, EventLog, Ledger};
use crate::simulation::metrics::{DailyMetrics, DayCounters};
use crate::simulation::settlement::SettlementTracker;
use crate::simulation::weather::WeatherState;
use crate::world::World;

/// Snapshot format version. Bump it whenever the serialized layout or the
/// meaning of a saved field changes; older files are then rejected.
pub const SNAPSHOT_FORMAT: u32 = 3;

/// Everything needed to resume a simulation exactly.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub format: u32,
    /// `accrete-core` version that wrote the file (informational).
    pub accrete_version: String,
    /// State hash at save time (hex); checked again on load.
    pub state_hash: String,
    /// FNV-1a hash (hex) of this snapshot's JSON with `state_hash` and
    /// `body_hash` empty; covers every saved field. Checked on load.
    #[serde(default)]
    pub body_hash: String,
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
    pub disruptions: DisruptionState,
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
            body_hash: String::new(),
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
            disruptions: self.disruptions.clone(),
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
        s.disruptions.rebuild_index(&mut sim.world);
        sim.disruptions = s.disruptions;
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

    /// Serialize the current state to JSON (with its body hash filled in).
    pub fn snapshot_json(&self) -> String {
        let mut s = self.snapshot();
        s.body_hash = body_hash(&s);
        serde_json::to_string(&s).expect("snapshot serializes")
    }

    /// Restore from JSON produced by [`Simulation::snapshot_json`].
    pub fn from_snapshot_json(text: &str) -> Result<Self, String> {
        let s: Snapshot = match serde_json::from_str(text) {
            Ok(s) => s,
            Err(e) => {
                // Name the likely cause when the file's layout is foreign.
                let version = serde_json::from_str::<serde_json::Value>(text)
                    .ok()
                    .and_then(|v| v.get("accrete_version")?.as_str().map(str::to_string));
                return Err(match version {
                    Some(v) => format!(
                        "snapshot does not parse with this build (written by accrete {v}; \
                         the layout or config schema differs): {e}"
                    ),
                    None => format!("snapshot does not parse: {e}"),
                });
            }
        };
        if s.format == SNAPSHOT_FORMAT && !s.body_hash.is_empty() {
            let expected = body_hash(&s);
            if expected != s.body_hash {
                return Err(body_mismatch(text, &s));
            }
        }
        Self::from_snapshot(s)
    }

    /// Write a snapshot file (conventionally `*.snap.json`). The file is
    /// checked to load back to the same state before it is written, so a
    /// state that JSON cannot represent (NaN or infinity) is an error here
    /// rather than an unloadable file later.
    pub fn save_snapshot(&self, path: &Path) -> Result<(), String> {
        let json = self.snapshot_json();
        let back = Self::from_snapshot_json(&json).map_err(|e| {
            format!(
                "refusing to write {}: the state does not round-trip ({e}); \
                 a NaN or infinite value is the usual cause",
                path.display()
            )
        })?;
        if back.state_hash() != self.state_hash() {
            return Err(format!(
                "refusing to write {}: the state does not round-trip exactly",
                path.display()
            ));
        }
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        }
        std::fs::write(path, json).map_err(|e| format!("writing {}: {e}", path.display()))
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
    ///
    /// Rejected: a different `[world]` section (the world already exists),
    /// and a population cap below the current population.
    pub fn set_config(&mut self, config: Config) -> Result<(), String> {
        if config.world != self.config.world {
            return Err(
                "the [world] config cannot change mid-run: the world is already generated".into(),
            );
        }
        config.validate()?;
        if (self.agents.len() as u32) > config.life.population_cap {
            return Err(format!(
                "life.population_cap {} is below the current population {}",
                config.life.population_cap,
                self.agents.len()
            ));
        }
        self.rates = TickRates::new(&config);
        self.world.rebuild_derived(&config);
        self.disruptions.rebuild_index(&mut self.world);
        self.events = EventLog::new(config.sim.event_log_capacity);
        self.config = config;
        Ok(())
    }
}

/// Hash of a snapshot's JSON with both hash fields empty.
fn body_hash(s: &Snapshot) -> String {
    let mut blank = s.clone();
    blank.state_hash = String::new();
    blank.body_hash = String::new();
    let json = serde_json::to_string(&blank).expect("snapshot serializes");
    hash::hex(hash::fnv1a64(json.as_bytes()))
}

/// Explain a body-hash mismatch: a different config schema (the file's
/// config does not re-serialize to itself) or a modified file.
fn body_mismatch(text: &str, s: &Snapshot) -> String {
    let file_config = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("config").cloned());
    let ours = serde_json::to_value(&s.config).ok();
    let schema_differs = match (&file_config, &ours) {
        (Some(f), Some(o)) => !same_keys(f, o),
        _ => false,
    };
    if schema_differs {
        format!(
            "snapshot was written with a different config schema (accrete {}); \
             it cannot be resumed exactly with this build",
            s.accrete_version
        )
    } else {
        "snapshot contents do not match their hash: the file was modified or truncated".into()
    }
}

/// Whether two JSON values have the same structure of object keys (the
/// config *schema*), ignoring the values themselves.
fn same_keys(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value::{Array, Object};
    match (a, b) {
        (Object(x), Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same_keys(v, w)))
        }
        (Array(x), Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(v, w)| same_keys(v, w)),
        (Object(_), _) | (_, Object(_)) => false,
        _ => true,
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

    #[test]
    fn body_hash_catches_edited_bookkeeping() {
        let mut sim = Simulation::new(3, short_config());
        sim.run(24 * 2);
        let good = sim.snapshot_json();
        for (what, from, to) in [
            ("metrics", "\"population\":", "\"population\":1"),
            ("ledger", "\"eaten\":", "\"eaten\":1"),
        ] {
            let bad = good.replacen(from, to, 1);
            assert_ne!(bad, good, "{what} edit applied");
            let e = Simulation::from_snapshot_json(&bad)
                .err()
                .expect("rejected");
            assert!(
                e.contains("do not match their hash") || e.contains("does not parse"),
                "{what}: {e}"
            );
        }
        assert!(Simulation::from_snapshot_json(&good).is_ok());
    }

    #[test]
    fn a_foreign_config_schema_is_named_as_such() {
        let sim = Simulation::new(3, short_config());
        let good = sim.snapshot_json();
        // A file from a newer build, with a config key this build lacks.
        let bad = good.replacen(
            "\"seasons_amplitude\":",
            "\"new_setting\":1,\"seasons_amplitude\":",
            1,
        );
        assert_ne!(bad, good);
        let e = Simulation::from_snapshot_json(&bad)
            .err()
            .expect("rejected");
        assert!(
            e.contains("written by accrete") && e.contains("schema"),
            "{e}"
        );
        // A key missing from an older file is filled with its default; when
        // the run used the default anyway the state is identical and loads.
        let older = good.replacen("\"seasons_amplitude\":0.0,", "", 1);
        assert_ne!(older, good);
        assert!(Simulation::from_snapshot_json(&older).is_ok());
    }

    #[test]
    fn non_finite_state_is_refused_at_save_time() {
        let mut c = short_config();
        c.weather.seasons_amplitude = f32::NAN;
        let sim = Simulation::new(3, c);
        let path =
            std::env::temp_dir().join(format!("accrete-nan-{}.snap.json", std::process::id()));
        let e = sim.save_snapshot(&path).expect_err("refused");
        assert!(e.contains("round-trip"), "{e}");
        assert!(!path.exists(), "no unloadable file was written");
    }

    #[test]
    fn set_config_rejects_world_changes_and_a_cap_below_the_population() {
        let mut sim = Simulation::new(3, short_config());
        let mut c = sim.config.clone();
        c.world.width += 1;
        assert!(sim.set_config(c).unwrap_err().contains("[world]"));
        let mut c = sim.config.clone();
        c.life.population_cap = sim.population() as u32 - 1;
        assert!(sim.set_config(c).unwrap_err().contains("population_cap"));
        let mut c = sim.config.clone();
        c.resources.food_regrowth *= 0.5;
        assert!(sim.set_config(c).is_ok());
    }

    /// Turning on per-tick checks (or resizing the event log) is not an
    /// intervention: the state hash is unchanged and a resumed run with
    /// `--check` matches an uninterrupted one.
    #[test]
    fn debug_settings_do_not_change_results_or_hashes() {
        let mut plain = Simulation::new(3, Config::default());
        plain.run(60);
        let mut checked = Simulation::from_snapshot_json(&plain.snapshot_json()).expect("loads");
        let mut c = checked.config.clone();
        c.sim.check_invariants = true;
        c.sim.event_log_capacity = 7;
        assert!(!c.differs_in_results(&checked.config));
        checked.set_config(c).expect("ok");
        assert_eq!(checked.state_hash(), plain.state_hash());
        plain.run(48);
        checked.run(48);
        assert_eq!(checked.state_hash(), plain.state_hash());
    }
}
