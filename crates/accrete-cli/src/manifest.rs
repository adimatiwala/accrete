//! Run manifests (accrete.md Section 9, Phase 2; A.10 "run storage").
//!
//! Every run writes `<label>_seed<N>_manifest.json` next to its outputs. A
//! manifest records exactly how to recreate the run (a fresh world from seed
//! and config, or a snapshot file plus the config applied on load, then a
//! number of ticks) and the state hash it ended on, together with
//! provenance: the accrete version, the git commit and dirty flag the binary
//! was built from, and a hash of `Cargo.lock`. Because the simulation is
//! deterministic, `accrete reproduce <manifest>` re-runs it and checks the
//! final state hash.

use std::path::{Path, PathBuf};

use accrete_core::{Config, Simulation, hash};
use serde::{Deserialize, Serialize};

pub const MANIFEST_FORMAT: u32 = 1;

/// Provenance of the running binary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Build {
    pub accrete_version: String,
    pub git_commit: String,
    /// Built from a tree with uncommitted changes to tracked files.
    pub git_dirty: bool,
    /// FNV-1a 64 (hex) of the `Cargo.lock` the binary was built with.
    pub cargo_lock_hash: String,
}

impl Build {
    pub fn current() -> Self {
        Build {
            accrete_version: env!("CARGO_PKG_VERSION").to_string(),
            git_commit: env!("ACCRETE_GIT_COMMIT").to_string(),
            git_dirty: env!("ACCRETE_GIT_DIRTY") == "true",
            cargo_lock_hash: hash::hex(hash::fnv1a64(include_bytes!("../../../Cargo.lock"))),
        }
    }
}

/// Where a run started.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Start {
    /// A fresh world generated from `seed` with the manifest's config.
    Fresh { seed: u64 },
    /// A snapshot file; the manifest's config was applied after loading.
    Snapshot {
        /// Absolute path when the run was recorded.
        path: PathBuf,
        /// The same file relative to the manifest's directory, if one exists
        /// (so a results folder can be moved or copied as a whole).
        #[serde(default)]
        relative_path: Option<PathBuf>,
        /// FNV-1a 64 (hex) of the snapshot file's bytes.
        file_hash: String,
    },
}

/// The command line, as recorded: lossy UTF-8 (never panics), and without
/// `--jobs`, which cannot change any result, so manifests are identical
/// whatever the number of jobs.
pub fn recorded_command() -> Vec<String> {
    let args: Vec<String> = std::env::args_os()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let mut out = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for a in args {
        if skip_next {
            skip_next = false;
        } else if a == "--jobs" {
            skip_next = true;
        } else if !a.starts_with("--jobs=") {
            out.push(a);
        }
    }
    out
}

/// `target` relative to the directory `from`, if both can be resolved; uses
/// `..` components as needed.
pub fn relative_to(from: &Path, target: &Path) -> Option<PathBuf> {
    let from = std::fs::canonicalize(from).ok()?;
    let target = std::fs::canonicalize(target).ok()?;
    let f: Vec<_> = from.components().collect();
    let t: Vec<_> = target.components().collect();
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return None; // different roots (e.g. Windows drives)
    }
    let mut rel = PathBuf::new();
    for _ in common..f.len() {
        rel.push("..");
    }
    for c in &t[common..] {
        rel.push(c.as_os_str());
    }
    Some(rel)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub build: Build,
    /// The command line that produced the run.
    pub command: Vec<String>,
    pub label: String,
    pub seed: u64,
    pub start: Start,
    pub start_tick: u64,
    /// State hash (hex) at `start_tick`, after any config change on load.
    pub start_state_hash: String,
    /// Ticks simulated (fewer than days × 24 if the run stopped early).
    pub ticks: u64,
    pub final_tick: u64,
    pub final_state_hash: String,
    pub config: Config,
    /// Output files written for this run, relative to the manifest.
    pub outputs: Vec<String>,
}

impl Manifest {
    pub fn path_for(out: &Path, label: &str, seed: u64) -> PathBuf {
        out.join(format!("{label}_seed{seed}_manifest.json"))
    }

    pub fn write(&self, out: &Path) -> Result<PathBuf, String> {
        let path = Self::path_for(out, &self.label, self.seed);
        let json = serde_json::to_string_pretty(self).expect("manifest serializes");
        std::fs::write(&path, json).map_err(|e| format!("writing {}: {e}", path.display()))?;
        Ok(path)
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        let m: Manifest = serde_json::from_str(&text)
            .map_err(|e| format!("{} is not a run manifest: {e}", path.display()))?;
        if m.format != MANIFEST_FORMAT {
            return Err(format!(
                "manifest format {} is not supported (this build reads {MANIFEST_FORMAT})",
                m.format
            ));
        }
        Ok(m)
    }
}

pub fn file_hash(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    Ok(hash::hex(hash::fnv1a64(&bytes)))
}

/// Outcome of re-running a manifest.
#[derive(Debug, PartialEq)]
pub struct Reproduction {
    pub start_matches: bool,
    pub final_hash: String,
    pub matches: bool,
    /// Differences between the recording build and this one (empty if none).
    pub build_differences: Vec<String>,
}

/// Re-run a manifest from its start and compare the final state hash.
/// `base_dir` resolves a relative snapshot path (the manifest's directory).
pub fn reproduce(m: &Manifest, base_dir: &Path) -> Result<Reproduction, String> {
    let mut sim = match &m.start {
        Start::Fresh { seed } => {
            if m.start_tick != 0 {
                return Err("a fresh run must start at tick 0".into());
            }
            Simulation::new(*seed, m.config.clone())
        }
        Start::Snapshot {
            path,
            relative_path,
            file_hash: expected,
        } => {
            // Relative to the manifest first (a moved results folder), then
            // as recorded, then next to the manifest; the first file whose
            // bytes match the recorded hash wins.
            let mut candidates: Vec<PathBuf> = Vec::new();
            if let Some(rel) = relative_path {
                candidates.push(base_dir.join(rel));
            }
            candidates.push(path.clone());
            candidates.push(base_dir.join(path.file_name().unwrap_or_default()));
            let existing: Vec<&PathBuf> = candidates.iter().filter(|c| c.is_file()).collect();
            if existing.is_empty() {
                return Err(format!("snapshot {} not found", path.display()));
            }
            let mut found = None;
            for c in &existing {
                if &file_hash(c)? == expected {
                    found = Some((*c).clone());
                    break;
                }
            }
            let p = found.ok_or_else(|| {
                format!(
                    "snapshot {} has changed since the run (no candidate matches hash {expected})",
                    existing[0].display()
                )
            })?;
            let mut sim = Simulation::load_snapshot(&p)?;
            if sim.world.seed != m.seed {
                return Err(format!(
                    "snapshot is seed {}, but the manifest is for seed {}",
                    sim.world.seed, m.seed
                ));
            }
            if sim.config != m.config {
                sim.set_config(m.config.clone())?;
            }
            sim
        }
    };
    if m.final_tick != m.start_tick + m.ticks {
        return Err("the manifest's ticks do not add up (start + ticks != final)".into());
    }
    if sim.tick != m.start_tick {
        return Err(format!(
            "start tick {} does not match the manifest's {}",
            sim.tick, m.start_tick
        ));
    }
    let start_matches = hash::hex(sim.state_hash()) == m.start_state_hash;
    sim.run(m.ticks);
    let final_hash = hash::hex(sim.state_hash());
    let now = Build::current();
    let mut build_differences = Vec::new();
    if now.git_commit != m.build.git_commit {
        build_differences.push(format!(
            "commit {} (recorded {})",
            now.git_commit, m.build.git_commit
        ));
    }
    if now.cargo_lock_hash != m.build.cargo_lock_hash {
        build_differences.push("Cargo.lock differs".into());
    }
    if m.build.git_dirty {
        build_differences.push("recorded from a build with uncommitted changes".into());
    }
    Ok(Reproduction {
        start_matches,
        matches: final_hash == m.final_state_hash,
        final_hash,
        build_differences,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{self, RunOpts};

    fn tmp(name: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("accrete-manifest-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn opts<'a>(
        out: &'a Path,
        label: &'a str,
        loaded_from: Option<&'a Path>,
        save: Option<&'a Path>,
    ) -> RunOpts<'a> {
        RunOpts {
            days: 2,
            out,
            every: 0,
            snap_every: 0,
            label,
            timelapse: &[],
            save,
            stop_on_extinction: true,
            loaded_from,
        }
    }

    #[test]
    fn fresh_and_resumed_runs_reproduce_and_tampering_is_caught() {
        let out = tmp("repro");
        let snap = out.join("d2.snap.json");
        let sim = run::run(
            4,
            &Config::default(),
            &opts(&out, "base", None, Some(&snap)),
        );
        let m = Manifest::read(&Manifest::path_for(&out, "base", 4)).unwrap();
        assert_eq!(m.start, Start::Fresh { seed: 4 });
        assert_eq!((m.start_tick, m.ticks, m.final_tick), (0, 48, 48));
        assert_eq!(m.final_state_hash, hash::hex(sim.state_hash()));
        assert!(m.outputs.iter().any(|o| o.ends_with("_metrics.csv")));
        assert!(reproduce(&m, &out).unwrap().matches);

        // Resumed from the snapshot with an intervention applied on load.
        let mut resumed = Simulation::load_snapshot(&snap).unwrap();
        let mut c = resumed.config.clone();
        c.resources.food_regrowth *= 0.5;
        resumed.set_config(c).unwrap();
        run::drive(resumed, &opts(&out, "cut", Some(&snap), None));
        let m = Manifest::read(&Manifest::path_for(&out, "cut", 4)).unwrap();
        assert!(matches!(m.start, Start::Snapshot { .. }));
        assert_eq!(m.start_tick, 48);
        let r = reproduce(&m, &out).unwrap();
        assert!(r.start_matches && r.matches);

        // A manifest that claims a different run is not reproduced.
        let mut bad = m.clone();
        bad.ticks -= 1;
        assert!(
            reproduce(&bad, &out).unwrap_err().contains("do not add up"),
            "inconsistent tick counts are an error"
        );
        bad.final_tick -= 1;
        assert!(!reproduce(&bad, &out).unwrap().matches);
        // A changed snapshot file is refused.
        std::fs::write(&snap, "{}").unwrap();
        assert!(reproduce(&m, &out).unwrap_err().contains("changed"));
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn manifests_round_trip_and_reject_other_formats() {
        let out = tmp("format");
        run::run(5, &Config::default(), &opts(&out, "r", None, None));
        let path = Manifest::path_for(&out, "r", 5);
        let m = Manifest::read(&path).unwrap();
        m.write(&out).unwrap();
        assert_eq!(Manifest::read(&path).unwrap(), m);
        let text =
            std::fs::read_to_string(&path)
                .unwrap()
                .replacen("\"format\": 1", "\"format\": 99", 1);
        std::fs::write(&path, text).unwrap();
        assert!(Manifest::read(&path).unwrap_err().contains("format"));
        assert!(Manifest::read(&out.join("missing.json")).is_err());
        let b = Build::current();
        assert!(!b.git_commit.is_empty() && b.cargo_lock_hash.len() == 16);
        let _ = std::fs::remove_dir_all(&out);
    }
}
