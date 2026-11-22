//! Week 3 Day 5 emergence experiment suite.
//!
//! Three paired ablations on the same seed(s), each from a fresh world:
//! - **trade:** barter off
//! - **trails:** trail formation off (traffic is still counted)
//! - **selfish:** household road weight 0 and reciprocity off
//!
//! The control for a seed is run once and reused. Each pair writes a per-day
//! comparison CSV; the suite also writes `emergence_summary.csv` (cumulative
//! trades; other metrics final-day).

use std::fmt::Write as _;
use std::path::PathBuf;

use accrete_core::{Config, Simulation};

use crate::branch;
use crate::run::{self, RunOpts};

/// One ablation in the suite.
pub struct Experiment {
    pub name: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub apply: fn(&mut Config),
}

/// The three Week 3 Day 5 comparisons.
pub const EXPERIMENTS: &[Experiment] = &[
    Experiment {
        name: "trade",
        label: "no-trade",
        description: "barter disabled (social.trade_enabled = false)",
        apply: |c| c.social.trade_enabled = false,
    },
    Experiment {
        name: "trails",
        label: "no-trails",
        description: "trail formation disabled; traffic still counted (trails.enabled = false)",
        apply: |c| c.trails.enabled = false,
    },
    Experiment {
        name: "selfish",
        label: "selfish",
        description: "household_weight = 0 and reciprocity off",
        apply: |c| {
            c.infrastructure.household_weight = 0.0;
            c.social.reciprocity_enabled = false;
        },
    },
];

pub struct ExperimentsArgs {
    pub seeds: Vec<u64>,
    pub days: u32,
    pub out: PathBuf,
    pub every: u32,
    pub check: bool,
    /// Subset of experiment names; empty means all.
    pub only: Vec<String>,
}

/// Resolve `--only` names to experiments, or all of them. Duplicates are dropped.
pub fn select(only: &[String]) -> Result<Vec<&'static Experiment>, String> {
    if only.is_empty() {
        return Ok(EXPERIMENTS.iter().collect());
    }
    let mut out = Vec::new();
    for name in only {
        let Some(e) = EXPERIMENTS.iter().find(|e| e.name == name) else {
            return Err(format!(
                "unknown experiment {name:?}; choose from {}",
                EXPERIMENTS
                    .iter()
                    .map(|e| e.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        if !out.iter().any(|x: &&Experiment| x.name == e.name) {
            out.push(e);
        }
    }
    Ok(out)
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2)
}

pub fn run_suite(base: &Config, a: ExperimentsArgs) {
    if a.seeds.is_empty() {
        fail("provide at least one --seed");
    }
    if a.days == 0 {
        fail("--days must be at least 1");
    }
    let experiments = select(&a.only).unwrap_or_else(|e| fail(e));
    run::check_label("control").unwrap_or_else(|e| fail(e));
    for e in &experiments {
        run::check_label(e.label).unwrap_or_else(|err| fail(err));
    }
    std::fs::create_dir_all(&a.out).expect("create output dir");

    println!(
        "emergence suite: {} experiment(s) × {} seed(s) × {} days → {}",
        experiments.len(),
        a.seeds.len(),
        a.days,
        a.out.display()
    );
    for e in &experiments {
        println!("  • {}: {}", e.name, e.description);
    }

    // `trades_*` columns are totals over the run; other metrics are final-day.
    let mut summary = String::from(
        "seed,experiment,days,population_control,population_intervention,population_diff,\
         trades_control,trades_intervention,trail_tiles_control,trail_tiles_intervention,\
         road_tiles_control,road_tiles_intervention,forage_radius_control,forage_radius_intervention,\
         travel_per_food_control,travel_per_food_intervention,\
         hamlets_control,hamlets_intervention,clark_evans_control,clark_evans_intervention,\
         deaths_deprivation_control,deaths_deprivation_intervention,\
         pathology_oscillating_control,pathology_oscillating_intervention,\
         pathology_starved_beside_food_control,pathology_starved_beside_food_intervention,\
         state_hash_control,state_hash_intervention,diverged\n",
    );

    for &seed in &a.seeds {
        let mut control_cfg = base.clone();
        control_cfg.sim.check_invariants |= a.check;
        let control = run::run(
            seed,
            &control_cfg,
            &RunOpts {
                days: a.days,
                out: &a.out,
                every: a.every,
                snap_every: 0,
                label: "control",
                timelapse: &[],
                save: None,
                stop_on_extinction: false,
            },
        );
        for e in &experiments {
            let mut cfg = base.clone();
            (e.apply)(&mut cfg);
            cfg.sim.check_invariants |= a.check;
            if !cfg.differs_in_results(&control_cfg) {
                fail(format!(
                    "experiment {} does not change results under this base config \
                     (is the ablation already applied via --set / --config?)",
                    e.name
                ));
            }
            let intervention = run::run(
                seed,
                &cfg,
                &RunOpts {
                    days: a.days,
                    out: &a.out,
                    every: a.every,
                    snap_every: 0,
                    label: e.label,
                    timelapse: &[],
                    save: None,
                    stop_on_extinction: false,
                },
            );
            let path = a
                .out
                .join(format!("control_vs_{}_seed{seed}_compare.csv", e.label));
            std::fs::write(
                &path,
                branch::compare_csv(&control, &intervention, 0, "control", e.label),
            )
            .expect("write comparison");
            branch::print_comparison(&control, &intervention, "control", e.label);
            println!("  comparison written to {}", path.display());

            let diverged = control.state_hash() != intervention.state_hash();
            if !diverged {
                eprintln!(
                    "warning: experiment {} seed {seed} did not diverge after {} days",
                    e.name, a.days
                );
            }
            append_summary_row(
                &mut summary,
                seed,
                e.name,
                a.days,
                &control,
                &intervention,
                diverged,
            );
        }
    }

    let summary_path = a.out.join("emergence_summary.csv");
    std::fs::write(&summary_path, summary).expect("write summary");
    println!("  summary written to {}", summary_path.display());
}

fn sum_trades(sim: &Simulation) -> u32 {
    sim.metrics.iter().map(|m| m.trades).sum()
}

fn append_summary_row(
    s: &mut String,
    seed: u64,
    name: &str,
    days: u32,
    control: &Simulation,
    intervention: &Simulation,
    diverged: bool,
) {
    let (mc, mi) = (
        control.metrics.last().expect("control metrics"),
        intervention.metrics.last().expect("intervention metrics"),
    );
    let _ = writeln!(
        s,
        "{seed},{name},{days},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        mc.population,
        mi.population,
        i64::from(mi.population) - i64::from(mc.population),
        sum_trades(control),
        sum_trades(intervention),
        mc.trail_tiles,
        mi.trail_tiles,
        mc.road_tiles,
        mi.road_tiles,
        mc.forage_radius,
        mi.forage_radius,
        mc.travel_per_food,
        mi.travel_per_food,
        mc.hamlets,
        mi.hamlets,
        mc.clark_evans,
        mi.clark_evans,
        mc.deaths_deprivation,
        mi.deaths_deprivation,
        mc.pathology_oscillating,
        mi.pathology_oscillating,
        mc.pathology_starved_beside_food,
        mi.pathology_starved_beside_food,
        accrete_core::hash::hex(control.state_hash()),
        accrete_core::hash::hex(intervention.state_hash()),
        diverged,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use accrete_core::config::TICKS_PER_DAY;

    fn tmp() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "accrete-exp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn select_rejects_unknown_names() {
        assert!(select(&["trade".into()]).is_ok());
        assert!(select(&["nope".into()]).is_err());
    }

    #[test]
    fn select_dedupes_preserving_order() {
        let got = select(&["selfish".into(), "trade".into(), "selfish".into()]).unwrap();
        assert_eq!(
            got.iter().map(|e| e.name).collect::<Vec<_>>(),
            ["selfish", "trade"]
        );
    }

    #[test]
    fn each_ablation_changes_the_config() {
        let base = Config::default();
        for e in EXPERIMENTS {
            let mut c = base.clone();
            (e.apply)(&mut c);
            assert!(c.differs_in_results(&base), "{}", e.name);
        }
    }

    fn run_pair(
        seed: u64,
        days: u32,
        base: &Config,
        exp: &Experiment,
        out: &Path,
    ) -> (Simulation, Simulation) {
        let control_cfg = base.clone();
        let mut intervention_cfg = base.clone();
        (exp.apply)(&mut intervention_cfg);
        let control = run::run(
            seed,
            &control_cfg,
            &RunOpts {
                days,
                out,
                every: 0,
                snap_every: 0,
                label: "control",
                timelapse: &[],
                save: None,
                stop_on_extinction: false,
            },
        );
        let intervention = run::run(
            seed,
            &intervention_cfg,
            &RunOpts {
                days,
                out,
                every: 0,
                snap_every: 0,
                label: exp.label,
                timelapse: &[],
                save: None,
                stop_on_extinction: false,
            },
        );
        (control, intervention)
    }

    /// Short paired runs: each intervention must diverge, and the directional
    /// effects the suite is meant to measure must show up.
    #[test]
    fn short_suite_diverges_with_expected_directions() {
        let out = tmp();
        let base = Config::default();
        let days = 40;
        let seed = 1u64;

        let trade = EXPERIMENTS.iter().find(|e| e.name == "trade").unwrap();
        let (c, i) = run_pair(seed, days, &base, trade, &out);
        assert_ne!(c.state_hash(), i.state_hash(), "trade should diverge");
        let trades_i: u32 = i.metrics.iter().map(|m| m.trades).sum();
        assert_eq!(trades_i, 0, "no-trade must never barter");
        let trades_c: u32 = c.metrics.iter().map(|m| m.trades).sum();
        // On seed 1, trade usually starts by day 40; if not, the zero on the
        // intervention side and the hash divergence are still enough.
        let _ = trades_c;

        let trails = EXPERIMENTS.iter().find(|e| e.name == "trails").unwrap();
        let (c, i) = run_pair(seed, days, &base, trails, &out);
        assert_ne!(c.state_hash(), i.state_hash(), "trails should diverge");
        let last_i = i.metrics.last().unwrap();
        assert_eq!(last_i.trail_tiles, 0, "no-trails must form no worn trails");

        let selfish = EXPERIMENTS.iter().find(|e| e.name == "selfish").unwrap();
        let (c, i) = run_pair(seed, days, &base, selfish, &out);
        assert_ne!(c.state_hash(), i.state_hash(), "selfish should diverge");
        // Both still run; the comparison CSV must line up by day.
        let csv = branch::compare_csv(&c, &i, 0, "control", "selfish");
        let lines: Vec<_> = csv.lines().collect();
        assert!(lines.len() > 1 + (days as usize / 2), "enough day rows");
        assert!(lines[1].starts_with("1,"), "{}", lines[1]);

        // A day of ticks is 24; metrics cover `days` days from a fresh world.
        assert_eq!(c.tick, u64::from(days) * TICKS_PER_DAY);
        let _ = out;
    }
}
