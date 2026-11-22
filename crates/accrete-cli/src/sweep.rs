//! Parameter sweeps (accrete.md A.10, "parameter → behaviour mapping").
//!
//! `accrete sweep` runs the simulation over a set of parameter points, each
//! on several seeds, and reports how the outcomes respond:
//!
//! - **Grid:** `--param key=a,b,c` (repeatable) gives the cartesian product
//!   of the listed values; the first parameter varies slowest.
//! - **Latin hypercube:** `--lhs N` with `--param key=lo..hi` (numeric keys)
//!   gives N points that cover each range evenly (one point per stratum of
//!   every parameter), drawn from the `Design` RNG stream with
//!   `--design-seed`, so a design is reproducible.
//!
//! Parameters are ordinary config overrides, applied in order on top of the
//! base config; every point is checked before anything runs. Runs go in
//! parallel (`--jobs`), and outputs are byte-identical whatever the number
//! of jobs. Each run writes its metrics and manifest as `pNNN_seedS_*`.
//!
//! Outputs: `sweep_design.json` (the design and its points),
//! `sweep_runs.csv` (one row per point and seed: parameters, outcomes,
//! final state hash), and `sweep_points.csv` (per point: mean and 95 %
//! interval of each outcome across seeds).

use std::fmt::Write as _;
use std::path::PathBuf;

use accrete_core::rng::{Rng, Stream};
use accrete_core::{Config, hash};
use serde::Serialize;

use crate::experiments::{OUTCOMES, outcomes};
use crate::parallel;
use crate::run::{self, KeyKind, RunOpts};

/// One swept parameter.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Param {
    /// Explicit values (grid sweeps).
    Values { key: String, values: Vec<String> },
    /// A numeric range (Latin-hypercube sweeps).
    Range { key: String, lo: f64, hi: f64 },
}

impl Param {
    pub fn key(&self) -> &str {
        match self {
            Param::Values { key, .. } | Param::Range { key, .. } => key,
        }
    }
}

/// Parse `key=a,b,c` or `key=lo..hi`.
pub fn parse_param(s: &str) -> Result<Param, String> {
    let (key, rest) = s
        .split_once('=')
        .ok_or_else(|| format!("--param expects key=values, got {s}"))?;
    let key = key.trim().to_string();
    if !key.contains('.') {
        return Err(format!("--param key must be section.key, got {key}"));
    }
    if let Some((lo, hi)) = rest.split_once("..") {
        let num = |x: &str| {
            x.trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("--param {key}: {x:?} is not a number"))
        };
        let (lo, hi) = (num(lo)?, num(hi)?);
        if lo >= hi {
            return Err(format!("--param {key}: range {lo}..{hi} must have lo < hi"));
        }
        return Ok(Param::Range { key, lo, hi });
    }
    let mut values: Vec<String> = Vec::new();
    for v in rest.split(',').map(str::trim) {
        if v.is_empty() {
            return Err(format!("--param {key}: empty value in {rest:?}"));
        }
        if !values.iter().any(|x| x == v) {
            values.push(v.to_string());
        }
    }
    Ok(Param::Values { key, values })
}

/// One point: (key, value) overrides in parameter order.
pub type Point = Vec<(String, String)>;

/// The points of a design. Grid: every parameter must list values. LHS:
/// every parameter must be a numeric range.
pub fn design_points(
    base: &Config,
    params: &[Param],
    lhs: Option<usize>,
    design_seed: u64,
) -> Result<Vec<Point>, String> {
    if params.is_empty() {
        return Err("provide at least one --param".into());
    }
    let mut keys: Vec<&str> = Vec::new();
    for p in params {
        if keys.contains(&p.key()) {
            return Err(format!("--param {} is given twice", p.key()));
        }
        keys.push(p.key());
    }
    match lhs {
        None => {
            let mut points: Vec<Point> = vec![Vec::new()];
            for p in params {
                let Param::Values { key, values } = p else {
                    return Err(format!(
                        "--param {}: a range (lo..hi) needs --lhs N; a grid sweep lists values (a,b,c)",
                        p.key()
                    ));
                };
                points = points
                    .into_iter()
                    .flat_map(|pt| {
                        values.iter().map(move |v| {
                            let mut q = pt.clone();
                            q.push((key.clone(), v.clone()));
                            q
                        })
                    })
                    .collect();
            }
            Ok(points)
        }
        Some(0) => Err("--lhs needs at least 1 point".into()),
        Some(n) => {
            let mut rng = Rng::for_stream(design_seed, Stream::Design);
            let mut columns: Vec<Vec<String>> = Vec::new();
            for p in params {
                let Param::Range { key, lo, hi } = p else {
                    return Err(format!(
                        "--param {}: a Latin-hypercube sweep needs a numeric range (lo..hi)",
                        p.key()
                    ));
                };
                let kind = run::key_kind(base, key)?;
                if !matches!(kind, KeyKind::Float | KeyKind::Integer) {
                    return Err(format!("--param {key}: only numeric keys can take a range"));
                }
                // One point per stratum, in random order, at a random spot
                // inside its stratum.
                let mut strata: Vec<usize> = (0..n).collect();
                rng.shuffle(&mut strata);
                let col = strata
                    .into_iter()
                    .map(|k| {
                        let u = f64::from(rng.unit_f32());
                        let v = lo + (k as f64 + u) / n as f64 * (hi - lo);
                        match kind {
                            KeyKind::Integer => format!("{}", v.round() as i64),
                            _ => format!("{v}"),
                        }
                    })
                    .collect();
                columns.push(col);
            }
            Ok((0..n)
                .map(|i| {
                    params
                        .iter()
                        .zip(&columns)
                        .map(|(p, col)| (p.key().to_string(), col[i].clone()))
                        .collect()
                })
                .collect())
        }
    }
}

/// The config for a point, or why it is invalid.
pub fn point_config(base: &Config, point: &Point) -> Result<Config, String> {
    let mut c = base.clone();
    for (k, v) in point {
        c = run::try_apply_override(&c, &format!("{k}={v}"))?;
    }
    Ok(c)
}

pub struct SweepArgs {
    pub params: Vec<Param>,
    pub lhs: Option<usize>,
    pub design_seed: u64,
    pub seeds: Vec<u64>,
    pub days: u32,
    pub out: PathBuf,
    pub jobs: usize,
    pub check: bool,
}

#[derive(Serialize)]
struct Design<'a> {
    mode: &'a str,
    design_seed: Option<u64>,
    params: &'a [Param],
    seeds: &'a [u64],
    days: u32,
    points: Vec<Vec<(String, String)>>,
}

const MAX_RUNS: usize = 100_000;

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2)
}

pub fn run_sweep(base: &Config, a: SweepArgs) {
    if a.days == 0 {
        fail("--days must be at least 1");
    }
    if a.seeds.is_empty() {
        fail("provide at least one --seed");
    }
    let points = design_points(base, &a.params, a.lhs, a.design_seed).unwrap_or_else(|e| fail(e));
    let runs = points.len() * a.seeds.len();
    if runs > MAX_RUNS {
        fail(format!(
            "{runs} runs is more than the {MAX_RUNS} allowed; narrow the sweep"
        ));
    }
    // Validate every point before running anything.
    let configs: Vec<Config> = points
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let mut c = point_config(base, p).unwrap_or_else(|e| fail(format!("point {k}: {e}")));
            c.sim.check_invariants |= a.check;
            c
        })
        .collect();
    std::fs::create_dir_all(&a.out).expect("create output dir");
    let design = Design {
        mode: if a.lhs.is_some() {
            "latin_hypercube"
        } else {
            "grid"
        },
        design_seed: a.lhs.map(|_| a.design_seed),
        params: &a.params,
        seeds: &a.seeds,
        days: a.days,
        points: points.clone(),
    };
    std::fs::write(
        a.out.join("sweep_design.json"),
        serde_json::to_string_pretty(&design).expect("design serializes"),
    )
    .expect("write design");
    println!(
        "sweep: {} point(s) × {} seed(s) = {runs} runs of {} days, {} job(s) → {}",
        points.len(),
        a.seeds.len(),
        a.days,
        a.jobs.max(1),
        a.out.display()
    );

    // (point index, seed) in a fixed order; results come back in that order.
    let jobs: Vec<(usize, u64)> = (0..points.len())
        .flat_map(|k| a.seeds.iter().map(move |&s| (k, s)))
        .collect();
    let results = parallel::map_ordered(&jobs, a.jobs, |&(k, seed)| {
        let label = format!("p{k:03}");
        let sim = run::run(
            seed,
            &configs[k],
            &RunOpts {
                days: a.days,
                out: &a.out,
                every: 0,
                snap_every: 0,
                label: &label,
                timelapse: &[],
                save: None,
                stop_on_extinction: false,
                loaded_from: None,
            },
        );
        (outcomes(&sim), hash::hex(sim.state_hash()))
    });

    std::fs::write(
        a.out.join("sweep_runs.csv"),
        runs_csv(&a.params, &points, &jobs, &results),
    )
    .expect("write runs");
    let summary = points_csv(&a.params, &points, &jobs, &results);
    std::fs::write(a.out.join("sweep_points.csv"), &summary).expect("write points");
    println!(
        "  wrote sweep_design.json, sweep_runs.csv and sweep_points.csv to {}",
        a.out.display()
    );
}

fn param_header(params: &[Param]) -> String {
    params.iter().map(|p| format!(",{}", p.key())).collect()
}

fn point_cells(point: &Point) -> String {
    point.iter().map(|(_, v)| format!(",{v}")).collect()
}

/// One row per (point, seed).
pub fn runs_csv(
    params: &[Param],
    points: &[Point],
    jobs: &[(usize, u64)],
    results: &[(Vec<f64>, String)],
) -> String {
    let mut s = format!("point,seed{}", param_header(params));
    for (name, _) in OUTCOMES {
        let _ = write!(s, ",{name}");
    }
    s.push_str(",final_state_hash\n");
    for (&(k, seed), (vals, h)) in jobs.iter().zip(results) {
        let _ = write!(s, "{k},{seed}{}", point_cells(&points[k]));
        for v in vals {
            let _ = write!(s, ",{v}");
        }
        let _ = writeln!(s, ",{h}");
    }
    s
}

/// One row per point: mean and 95 % interval of each outcome across seeds.
pub fn points_csv(
    params: &[Param],
    points: &[Point],
    jobs: &[(usize, u64)],
    results: &[(Vec<f64>, String)],
) -> String {
    let mut s = format!("point,n_seeds{}", param_header(params));
    for (name, _) in OUTCOMES {
        let _ = write!(s, ",{name}_mean,{name}_ci95_low,{name}_ci95_high");
    }
    s.push('\n');
    let cell = |x: f64| {
        if x.is_finite() {
            x.to_string()
        } else {
            String::new()
        }
    };
    for (k, point) in points.iter().enumerate() {
        let rows: Vec<&Vec<f64>> = jobs
            .iter()
            .zip(results)
            .filter(|((p, _), _)| *p == k)
            .map(|(_, (v, _))| v)
            .collect();
        let _ = write!(s, "{k},{}{}", rows.len(), point_cells(point));
        for o in 0..OUTCOMES.len() {
            let xs: Vec<f64> = rows.iter().map(|v| v[o]).collect();
            let (m, _, half) = parallel::mean_sd_ci(&xs);
            let _ = write!(s, ",{},{},{}", cell(m), cell(m - half), cell(m + half));
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Param {
        parse_param(s).unwrap()
    }

    #[test]
    fn params_parse_values_and_ranges() {
        assert_eq!(
            p("resources.food_regrowth=0.03, 0.06,0.03"),
            Param::Values {
                key: "resources.food_regrowth".into(),
                values: vec!["0.03".into(), "0.06".into()]
            }
        );
        assert_eq!(
            p("decision.time_cost=0.01..0.05"),
            Param::Range {
                key: "decision.time_cost".into(),
                lo: 0.01,
                hi: 0.05
            }
        );
        for bad in [
            "nokey=1",
            "a.b",
            "a.b=1,,2",
            "a.b=2..1",
            "a.b=x..1",
            "a.b=1..inf",
        ] {
            assert!(parse_param(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn grids_are_cartesian_products_with_the_first_param_slowest() {
        let base = Config::default();
        let pts = design_points(
            &base,
            &[
                p("resources.food_regrowth=0.1,0.2"),
                p("social.trade_enabled=true,false"),
            ],
            None,
            1,
        )
        .unwrap();
        let vals: Vec<Vec<&str>> = pts
            .iter()
            .map(|pt| pt.iter().map(|(_, v)| v.as_str()).collect())
            .collect();
        assert_eq!(
            vals,
            vec![
                vec!["0.1", "true"],
                vec!["0.1", "false"],
                vec!["0.2", "true"],
                vec!["0.2", "false"]
            ]
        );
        assert!(
            design_points(&base, &[p("a.b=0..1")], None, 1).is_err(),
            "range needs --lhs"
        );
        assert!(
            design_points(&base, &[p("a.b=1"), p("a.b=2")], None, 1).is_err(),
            "duplicate key"
        );
        assert!(design_points(&base, &[], None, 1).is_err());
    }

    #[test]
    fn latin_hypercube_covers_every_stratum_and_is_reproducible() {
        let base = Config::default();
        let params = [
            p("resources.food_regrowth=0.0..1.0"),
            p("life.population_cap=200..1000"),
        ];
        let n = 10;
        let a = design_points(&base, &params, Some(n), 7).unwrap();
        assert_eq!(
            a,
            design_points(&base, &params, Some(n), 7).unwrap(),
            "same seed, same design"
        );
        assert_ne!(a, design_points(&base, &params, Some(n), 8).unwrap());
        let mut strata: Vec<usize> = a
            .iter()
            .map(|pt| (pt[0].1.parse::<f64>().unwrap() * n as f64).floor() as usize)
            .collect();
        strata.sort_unstable();
        assert_eq!(strata, (0..n).collect::<Vec<_>>(), "one point per stratum");
        for pt in &a {
            assert!(
                pt[1].1.parse::<i64>().is_ok(),
                "integer keys get integers: {}",
                pt[1].1
            );
        }
        assert!(design_points(&base, &[p("social.trade_enabled=0..1")], Some(3), 1).is_err());
        assert!(design_points(&base, &[p("resources.food_regrowth=0.1,0.2")], Some(3), 1).is_err());
        assert!(design_points(&base, &params, Some(0), 1).is_err());
    }

    #[test]
    fn invalid_points_are_rejected_before_running() {
        let base = Config::default();
        assert!(
            point_config(&base, &vec![("resources.food_regrowth".into(), "x".into())]).is_err()
        );
        assert!(point_config(&base, &vec![("nope.key".into(), "1".into())]).is_err());
        let c = point_config(
            &base,
            &vec![("resources.food_regrowth".into(), "0.2".into())],
        )
        .unwrap();
        assert!((c.resources.food_regrowth - 0.2).abs() < 1e-6);
    }

    #[test]
    fn sweep_runs_end_to_end_and_ignore_the_job_count() {
        let run_with = |jobs: usize| {
            let out =
                std::env::temp_dir().join(format!("accrete-sweep-{}-{jobs}", std::process::id()));
            let _ = std::fs::remove_dir_all(&out);
            run_sweep(
                &Config::default(),
                SweepArgs {
                    params: vec![p("resources.food_regrowth=0.05,0.15")],
                    lhs: None,
                    design_seed: 1,
                    seeds: vec![1, 2],
                    days: 3,
                    out: out.clone(),
                    jobs,
                    check: false,
                },
            );
            let read = |f: &str| std::fs::read_to_string(out.join(f)).unwrap();
            let files = (
                read("sweep_runs.csv"),
                read("sweep_points.csv"),
                read("sweep_design.json"),
            );
            assert!(out.join("p001_seed2_manifest.json").exists());
            let _ = std::fs::remove_dir_all(&out);
            files
        };
        let (runs, points, _) = run_with(1);
        assert_eq!(runs.lines().count(), 1 + 4, "2 points × 2 seeds");
        assert_eq!(points.lines().count(), 1 + 2);
        assert!(points.lines().nth(1).unwrap().starts_with("0,2,0.05,"));
        assert_eq!(run_with(1), run_with(4), "byte-identical across job counts");
    }
}
