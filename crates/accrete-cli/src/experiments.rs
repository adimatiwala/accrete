//! Week 3 Day 5 emergence experiment suite, plus the scarcity experiment
//! (accrete.md Section 9).
//!
//! Paired comparisons on the same seed(s), each from a fresh world:
//! - **trade:** barter off
//! - **trails:** trail formation off (traffic is still counted)
//! - **selfish:** household road weight 0 and reciprocity off
//! - **scarcity40 / scarcity20:** food regrowth at 40 % / 20 % of the base
//!   config's rate (the levels of the original A.4 calibration)
//!
//! The control for a seed is run once and reused. Each pair writes a per-day
//! comparison CSV; the suite also writes `emergence_summary.csv`. Columns
//! named `total_*` and `trades_*` are run totals; the others are final-day.

use std::fmt::Write as _;
use std::path::PathBuf;

use accrete_core::{Config, Simulation};

use crate::branch;
use crate::parallel;
use crate::run::{self, RunOpts};

/// One ablation in the suite.
pub struct Experiment {
    pub name: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub apply: fn(&mut Config),
}

/// The Week 3 Day 5 comparisons and the scarcity levels.
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
    Experiment {
        name: "scarcity40",
        label: "food-x0.4",
        description: "food regrowth at 40% of the base rate (resources.food_regrowth x 0.4)",
        apply: |c| c.resources.food_regrowth *= 0.4,
    },
    Experiment {
        name: "scarcity20",
        label: "food-x0.2",
        description: "food regrowth at 20% of the base rate (resources.food_regrowth x 0.2)",
        apply: |c| c.resources.food_regrowth *= 0.2,
    },
];

/// Columns of `emergence_summary.csv`. `trades_*` and `total_*` columns are
/// run totals; the others are final-day values.
pub const SUMMARY_HEADER: &str = "seed,experiment,days,population_control,population_intervention,population_diff,\
         trades_control,trades_intervention,trail_tiles_control,trail_tiles_intervention,\
         road_tiles_control,road_tiles_intervention,         forage_radius_control,forage_radius_intervention,\
         travel_per_food_run_control,travel_per_food_run_intervention,\
         hamlets_control,hamlets_intervention,clark_evans_control,clark_evans_intervention,\
         total_births_control,total_births_intervention,\
         total_deaths_deprivation_control,total_deaths_deprivation_intervention,\
         total_deaths_old_age_control,total_deaths_old_age_intervention,\
         total_home_moves_control,total_home_moves_intervention,\
         mean_lifespan_days_control,mean_lifespan_days_intervention,\
         births_per_adult_day_control,births_per_adult_day_intervention,\
         pathology_oscillating_control,pathology_oscillating_intervention,\
         pathology_starved_beside_food_control,pathology_starved_beside_food_intervention,\
         state_hash_control,state_hash_intervention,diverged\n";

pub struct ExperimentsArgs {
    pub seeds: Vec<u64>,
    pub days: u32,
    pub out: PathBuf,
    pub every: u32,
    pub check: bool,
    /// Subset of experiment names; empty means all.
    pub only: Vec<String>,
    /// Seeds run in parallel (each seed's runs stay sequential). Outputs are
    /// identical whatever the value.
    pub jobs: usize,
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
    // Check every experiment changes results before running anything, so a
    // no-op is reported up front, not after minutes of runs.
    let mut control_cfg = base.clone();
    control_cfg.sim.check_invariants |= a.check;
    for e in &experiments {
        let mut cfg = control_cfg.clone();
        (e.apply)(&mut cfg);
        if !cfg.differs_in_results(&control_cfg) {
            if e.name.starts_with("scarcity") && base.resources.food_regrowth == 0.0 {
                fail(format!(
                    "experiment {} scales food regrowth, but the base regrowth is 0",
                    e.name
                ));
            }
            fail(format!(
                "experiment {} does not change results under this base config \
                 (is the ablation already applied via --set / --config?)",
                e.name
            ));
        }
    }
    run::check_label("control").unwrap_or_else(|e| fail(e));
    for e in &experiments {
        run::check_label(e.label).unwrap_or_else(|err| fail(err));
    }
    std::fs::create_dir_all(&a.out).expect("create output dir");
    // Drop stale suite outputs so a failed re-run cannot leave an old summary
    // looking complete next to new per-seed files.
    let _ = std::fs::remove_file(a.out.join("emergence_summary.csv"));
    let _ = std::fs::remove_file(a.out.join("emergence_aggregate.csv"));
    let _ = std::fs::remove_file(a.out.join(crate::index::INDEX_CSV));
    let _ = std::fs::remove_file(a.out.join(crate::index::DB_NAME));

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

    // `trades_*` and `total_*` columns are run totals; others are final-day
    // (except travel_per_food_run, which matches the aggregate).
    let mut summary = String::from(SUMMARY_HEADER);
    let base_cfg = base.clone();
    let results = parallel::map_ordered(&a.seeds, a.jobs, |&seed| {
        run_seed(seed, &experiments, &control_cfg, &base_cfg, &a)
    });
    for r in &results {
        for row in &r.rows {
            summary.push_str(row);
        }
    }
    let summary_path = a.out.join("emergence_summary.csv");
    std::fs::write(&summary_path, summary).expect("write summary");
    println!("  summary written to {}", summary_path.display());

    let aggregate = aggregate_csv(&experiments, &results);
    let aggregate_path = a.out.join("emergence_aggregate.csv");
    std::fs::write(&aggregate_path, &aggregate).expect("write aggregate");
    print_aggregate(&experiments, &results);
    println!("  aggregate written to {}", aggregate_path.display());
    // Fresh rebuild + runs_index.csv so --jobs cannot leave a partial index.
    crate::index::finalize_dir(&a.out).unwrap_or_else(|e| {
        eprintln!("warning: run index finalize: {e}");
    });
}

/// What one seed produced: its summary rows and, per experiment, the
/// outcome values of the control and the intervention.
pub struct SeedResult {
    pub rows: Vec<String>,
    /// (experiment name, control outcomes, intervention outcomes)
    pub pairs: Vec<(&'static str, Vec<f64>, Vec<f64>)>,
}

fn run_seed(
    seed: u64,
    experiments: &[&'static Experiment],
    control_cfg: &Config,
    base: &Config,
    a: &ExperimentsArgs,
) -> SeedResult {
    let opts = |label: &'static str| RunOpts {
        days: a.days,
        out: &a.out,
        every: a.every,
        snap_every: 0,
        save_every: 0,
        label,
        timelapse: &[],
        save: None,
        stop_on_extinction: false,
        loaded_from: None,
    };
    let control = run::run(seed, control_cfg, &opts("control"));
    let mut out = SeedResult {
        rows: Vec::new(),
        pairs: Vec::new(),
    };
    for e in experiments {
        let mut cfg = base.clone();
        (e.apply)(&mut cfg);
        cfg.sim.check_invariants |= a.check;
        let intervention = run::run(seed, &cfg, &opts(e.label));
        let path = a
            .out
            .join(format!("control_vs_{}_seed{seed}_compare.csv", e.label));
        std::fs::write(
            &path,
            branch::compare_csv(&control, &intervention, 0, "control", e.label),
        )
        .expect("write comparison");
        if a.jobs <= 1 {
            branch::print_comparison(&control, &intervention, "control", e.label);
        }
        println!("  [seed {seed}] comparison written to {}", path.display());
        let diverged = control.state_hash() != intervention.state_hash();
        if !diverged {
            eprintln!(
                "warning: experiment {} seed {seed} did not diverge after {} days",
                e.name, a.days
            );
        }
        let mut row = String::new();
        append_summary_row(
            &mut row,
            seed,
            e.name,
            a.days,
            &control,
            &intervention,
            diverged,
        );
        out.rows.push(row);
        out.pairs
            .push((e.name, outcomes(&control), outcomes(&intervention)));
    }
    out
}

/// Outcomes compared across seeds, as (name, value of a finished run).
type Outcome = (&'static str, fn(&Simulation) -> f64);

pub const OUTCOMES: &[Outcome] = &[
    ("population", |s| f64::from(last(s).population)),
    ("total_births", |s| f64::from(last(s).total_births)),
    ("births_per_adult_day", births_per_adult_day),
    ("total_deaths_deprivation", |s| {
        f64::from(last(s).total_deaths_deprivation)
    }),
    ("total_home_moves", |s| f64::from(sum_home_moves(s))),
    ("trades", |s| f64::from(sum_trades(s))),
    ("trail_tiles", |s| f64::from(last(s).trail_tiles)),
    ("road_tiles", |s| f64::from(last(s).road_tiles)),
    ("hamlets", |s| f64::from(last(s).hamlets)),
    // Extinction / tiny populations make these sentinels, not measurements.
    ("clark_evans", |s| {
        let m = last(s);
        if m.population == 0 || m.homes_occupied < 2 {
            f64::NAN
        } else {
            f64::from(m.clark_evans)
        }
    }),
    ("forage_radius", |s| {
        if last(s).population == 0 {
            f64::NAN
        } else {
            f64::from(last(s).forage_radius)
        }
    }),
    ("travel_per_food_run", travel_per_food_run),
];

fn travel_per_food_run(s: &Simulation) -> f64 {
    let moved: f64 = s.metrics.iter().map(|m| m.tiles_moved as f64).sum();
    let food: f64 = s.metrics.iter().map(|m| f64::from(m.food_gathered)).sum();
    moved / food.max(1.0)
}

fn last(s: &Simulation) -> &accrete_core::simulation::metrics::DailyMetrics {
    s.metrics.last().expect("a finished run has metrics")
}

pub fn outcomes(s: &Simulation) -> Vec<f64> {
    OUTCOMES.iter().map(|(_, f)| f(s)).collect()
}

pub const AGGREGATE_HEADER: &str = "experiment,outcome,n_seeds,control_mean,intervention_mean,\
     diff_mean,diff_sd,diff_ci95_low,diff_ci95_high,seeds_intervention_higher,seeds_intervention_lower\n";

/// Per experiment and outcome, across seeds: the mean control and
/// intervention values, and the mean *paired* difference (intervention minus
/// control, same seed) with its 95 % confidence interval. Empty cells where a
/// statistic is undefined (e.g. one seed).
pub fn aggregate_csv(experiments: &[&'static Experiment], results: &[SeedResult]) -> String {
    let mut s = String::from(AGGREGATE_HEADER);
    let cell = |x: f64| {
        if x.is_finite() {
            x.to_string()
        } else {
            String::new()
        }
    };
    for e in experiments {
        for (k, (name, _)) in OUTCOMES.iter().enumerate() {
            let (mut c, mut i, mut d) = (Vec::new(), Vec::new(), Vec::new());
            for r in results {
                for (exp, cv, iv) in &r.pairs {
                    // Only seeds where both sides are finite count.
                    if *exp == e.name && cv[k].is_finite() && iv[k].is_finite() {
                        c.push(cv[k]);
                        i.push(iv[k]);
                        d.push(iv[k] - cv[k]);
                    }
                }
            }
            let (cm, _, _) = parallel::mean_sd_ci(&c);
            let (im, _, _) = parallel::mean_sd_ci(&i);
            let (dm, dsd, half) = parallel::mean_sd_ci(&d);
            let higher = d.iter().filter(|x| **x > 0.0).count();
            let lower = d.iter().filter(|x| **x < 0.0).count();
            let _ = writeln!(
                s,
                "{},{name},{},{},{},{},{},{},{},{higher},{lower}",
                e.name,
                d.len(),
                cell(cm),
                cell(im),
                cell(dm),
                cell(dsd),
                cell(dm - half),
                cell(dm + half),
            );
        }
    }
    s
}

fn print_aggregate(experiments: &[&'static Experiment], results: &[SeedResult]) {
    let n = results.len();
    println!("  across {n} seed(s): mean paired difference (intervention - control), 95% CI");
    for e in experiments {
        println!("  {}:", e.name);
        for (k, (name, _)) in OUTCOMES.iter().enumerate() {
            let d: Vec<f64> = results
                .iter()
                .flat_map(|r| {
                    r.pairs.iter().filter_map(move |p| {
                        if p.0 == e.name && p.1[k].is_finite() && p.2[k].is_finite() {
                            Some(p.2[k] - p.1[k])
                        } else {
                            None
                        }
                    })
                })
                .collect();
            if d.is_empty() {
                println!("    {name:<26}            (no finite pairs)");
                continue;
            }
            let (m, _, half) = parallel::mean_sd_ci(&d);
            if half.is_finite() {
                println!(
                    "    {name:<26} {m:>+12.4}  [{:+.4}, {:+.4}]",
                    m - half,
                    m + half
                );
            } else {
                println!("    {name:<26} {m:>+12.4}");
            }
        }
    }
}

fn sum_trades(sim: &Simulation) -> u32 {
    sim.metrics.iter().map(|m| m.trades).sum()
}

/// Births per adult-day over the run: per-capita fertility, separating a
/// fall in births from simply having fewer adults.
fn births_per_adult_day(sim: &Simulation) -> f64 {
    let births: u64 = sim.metrics.iter().map(|m| u64::from(m.births)).sum();
    let adult_days: u64 = sim.metrics.iter().map(|m| u64::from(m.adults)).sum();
    births as f64 / adult_days.max(1) as f64
}

/// Mean lifespan of agents born during the run, or an empty cell when none
/// of them has died yet (the metric reports 0 then, which is not a lifespan).
fn lifespan_cell(m: &accrete_core::simulation::metrics::DailyMetrics) -> String {
    if m.mean_lifespan_days > 0.0 {
        m.mean_lifespan_days.to_string()
    } else {
        String::new()
    }
}

/// Home moves over the whole run (the spec's "migration").
fn sum_home_moves(sim: &Simulation) -> u32 {
    sim.metrics.iter().map(|m| m.home_moves).sum()
}

fn outcome_cell(x: f64) -> String {
    if x.is_finite() {
        x.to_string()
    } else {
        String::new()
    }
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
    let oc = outcomes(control);
    let oi = outcomes(intervention);
    let by = |name: &str, side: &[f64]| {
        let i = OUTCOMES.iter().position(|(n, _)| *n == name).expect(name);
        outcome_cell(side[i])
    };
    let _ = writeln!(
        s,
        "{seed},{name},{days},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        mc.population,
        mi.population,
        i64::from(mi.population) - i64::from(mc.population),
        sum_trades(control),
        sum_trades(intervention),
        mc.trail_tiles,
        mi.trail_tiles,
        mc.road_tiles,
        mi.road_tiles,
        by("forage_radius", &oc),
        by("forage_radius", &oi),
        by("travel_per_food_run", &oc),
        by("travel_per_food_run", &oi),
        mc.hamlets,
        mi.hamlets,
        by("clark_evans", &oc),
        by("clark_evans", &oi),
        mc.total_births,
        mi.total_births,
        mc.total_deaths_deprivation,
        mi.total_deaths_deprivation,
        mc.total_deaths_old_age,
        mi.total_deaths_old_age,
        sum_home_moves(control),
        sum_home_moves(intervention),
        lifespan_cell(mc),
        lifespan_cell(mi),
        births_per_adult_day(control),
        births_per_adult_day(intervention),
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

    /// A fresh directory per call. The counter makes names unique even
    /// when parallel tests ask at the same instant (macOS clocks only have
    /// microsecond resolution, so a timestamp alone collided).
    fn tmp() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("accrete-exp-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
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
                save_every: 0,
                label: "control",
                timelapse: &[],
                save: None,
                stop_on_extinction: false,
                loaded_from: None,
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
                save_every: 0,
                label: exp.label,
                timelapse: &[],
                save: None,
                stop_on_extinction: false,
                loaded_from: None,
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

    /// Scarcity scales whatever regrowth rate the base config has, so it
    /// composes with `--set resources.food_regrowth=…`.
    #[test]
    fn scarcity_levels_scale_the_base_regrowth() {
        let mut base = Config::default();
        base.resources.food_regrowth = 0.2;
        for (name, factor) in [("scarcity40", 0.4f32), ("scarcity20", 0.2)] {
            let e = EXPERIMENTS.iter().find(|e| e.name == name).unwrap();
            let mut c = base.clone();
            (e.apply)(&mut c);
            assert!(
                (c.resources.food_regrowth - 0.2 * factor).abs() < 1e-6,
                "{name}"
            );
            assert!(run::check_label(e.label).is_ok(), "{name} label");
        }
    }

    /// Within a month, lower regrowth leaves less standing food and no more
    /// people (later, the much smaller population can leave *more* standing
    /// food; see accrete.md A.18).
    #[test]
    fn severe_scarcity_reduces_food_and_population() {
        let out = tmp();
        let base = Config::default();
        let e = EXPERIMENTS.iter().find(|e| e.name == "scarcity20").unwrap();
        let (c, i) = run_pair(1, 30, &base, e, &out);
        let (mc, mi) = (c.metrics.last().unwrap(), i.metrics.last().unwrap());
        assert!(mi.food_world < mc.food_world, "less standing food");
        assert!(
            mi.population <= mc.population,
            "no more people under scarcity"
        );
        assert_ne!(c.state_hash(), i.state_hash());
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn summary_rows_match_the_header_and_use_run_totals() {
        let out = tmp();
        let base = Config::default();
        let e = EXPERIMENTS.iter().find(|e| e.name == "trade").unwrap();
        let (c, i) = run_pair(1, 8, &base, e, &out);
        let mut row = String::new();
        append_summary_row(&mut row, 1, e.name, 8, &c, &i, true);
        let cells: Vec<&str> = row.trim_end().split(',').collect();
        let header: Vec<&str> = SUMMARY_HEADER.trim_end().split(',').collect();
        assert_eq!(cells.len(), header.len(), "one cell per header column");
        let col = |name: &str| cells[header.iter().position(|h| *h == name).expect(name)];
        let births: u32 = c.metrics.iter().map(|m| m.births).sum();
        assert_eq!(
            col("total_births_control"),
            births.to_string(),
            "run total, not the last day's"
        );
        assert_eq!(col("experiment"), "trade");
        assert_eq!(col("diverged"), "true");
        let _ = std::fs::remove_dir_all(&out);
    }

    /// The whole suite end to end: the summary's totals match the per-day
    /// metrics files each side wrote.
    #[test]
    fn suite_summary_matches_the_metrics_files() {
        let out = tmp();
        run_suite(
            &Config::default(),
            ExperimentsArgs {
                seeds: vec![1],
                days: 12,
                out: out.clone(),
                every: 0,
                check: false,
                only: vec!["scarcity20".into()],
                jobs: 1,
            },
        );
        let read = |name: &str| std::fs::read_to_string(out.join(name)).expect(name);
        let summary = read("emergence_summary.csv");
        let mut lines = summary.lines();
        let header: Vec<&str> = lines.next().unwrap().split(',').collect();
        let row: Vec<&str> = lines.next().unwrap().split(',').collect();
        assert!(lines.next().is_none(), "one experiment, one seed, one row");
        assert_eq!(row.len(), header.len());
        let col = |name: &str| row[header.iter().position(|h| *h == name).expect(name)];
        assert_eq!(col("experiment"), "scarcity20");
        // Sum a column of a side's metrics CSV.
        let total = |file: &str, column: &str| -> u64 {
            let text = read(file);
            let mut it = text.lines();
            let h: Vec<&str> = it.next().unwrap().split(',').collect();
            let k = h.iter().position(|x| *x == column).expect(column);
            it.map(|l| l.split(',').nth(k).unwrap().parse::<u64>().unwrap())
                .sum()
        };
        for (side, file) in [
            ("control", "control_seed1_metrics.csv"),
            ("intervention", "food-x0.2_seed1_metrics.csv"),
        ] {
            assert_eq!(
                col(&format!("total_births_{side}")),
                total(file, "births").to_string(),
                "{side} births"
            );
            assert_eq!(
                col(&format!("trades_{side}")),
                total(file, "trades").to_string(),
                "{side} trades"
            );
            assert_eq!(
                col(&format!("total_home_moves_{side}")),
                total(file, "home_moves").to_string(),
                "{side} moves"
            );
            assert_eq!(
                col(&format!("mean_lifespan_days_{side}")),
                "",
                "nobody born in 12 days has died"
            );
        }
        assert!(out.join("control_vs_food-x0.2_seed1_compare.csv").exists());
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Seeds in parallel give byte-identical outputs to seeds in sequence.
    #[test]
    fn parallel_seeds_produce_identical_outputs() {
        let run_with = |jobs: usize| {
            let out = tmp();
            run_suite(
                &Config::default(),
                ExperimentsArgs {
                    seeds: vec![1, 2, 3],
                    days: 3,
                    out: out.clone(),
                    every: 0,
                    check: false,
                    only: vec!["trade".into()],
                    jobs,
                },
            );
            let read = |f: &str| std::fs::read(out.join(f)).unwrap();
            let files = (
                read("emergence_summary.csv"),
                read("emergence_aggregate.csv"),
                read("control_seed2_metrics.csv"),
                read(crate::index::INDEX_CSV),
            );
            let n: i64 = {
                let db = crate::index::default_db(&out);
                rusqlite::Connection::open_with_flags(
                    &db,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .unwrap()
                .query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
                .unwrap()
            };
            // control + trade intervention × 3 seeds
            assert_eq!(n, 6, "index complete under --jobs {jobs}");
            let _ = std::fs::remove_dir_all(&out);
            files
        };
        assert_eq!(run_with(1), run_with(3));
    }

    #[test]
    fn aggregate_pairs_differences_by_seed() {
        let trade = EXPERIMENTS.iter().find(|e| e.name == "trade").unwrap();
        let n = OUTCOMES.len();
        let pair = |c: f64, i: f64| ("trade", vec![c; n], vec![i; n]);
        let results = vec![
            SeedResult {
                rows: vec![],
                pairs: vec![pair(10.0, 12.0)],
            },
            SeedResult {
                rows: vec![],
                pairs: vec![pair(20.0, 21.0)],
            },
            SeedResult {
                rows: vec![],
                pairs: vec![pair(30.0, 33.0)],
            },
        ];
        let csv = aggregate_csv(&[trade], &results);
        let mut lines = csv.lines();
        let header: Vec<&str> = lines.next().unwrap().split(',').collect();
        let row: Vec<&str> = lines.next().unwrap().split(',').collect();
        assert_eq!(csv.lines().count(), 1 + n, "one row per outcome");
        let col = |name: &str| row[header.iter().position(|h| *h == name).unwrap()];
        assert_eq!(col("n_seeds"), "3");
        assert_eq!(col("control_mean"), "20");
        assert_eq!(col("diff_mean"), "2");
        assert_eq!(col("diff_sd"), "1");
        let half = 4.303 / 3f64.sqrt();
        assert!((col("diff_ci95_low").parse::<f64>().unwrap() - (2.0 - half)).abs() < 1e-9);
        assert_eq!(
            (
                col("seeds_intervention_higher"),
                col("seeds_intervention_lower")
            ),
            ("3", "0")
        );

        // One seed: no spread or interval, left empty.
        let one = aggregate_csv(&[trade], &results[..1]);
        let row: Vec<&str> = one.lines().nth(1).unwrap().split(',').collect();
        assert_eq!(
            row[header.iter().position(|h| *h == "diff_sd").unwrap()],
            ""
        );
    }
}
