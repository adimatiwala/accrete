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
        values.push(v.to_string());
    }
    Ok(Param::Values { key, values })
}

/// One point: (key, value) overrides in parameter order.
pub type Point = Vec<(String, String)>;

/// Most runs (points × seeds) a sweep may have.
pub const MAX_RUNS: usize = 100_000;

/// The canonical text of `value` for `key`: what is applied, recorded and
/// written to the CSVs. Numbers are parsed and re-printed (so `0.10`, `+0.1`
/// and `0.1` are the same point), floats at the config's f32 precision,
/// and non-finite numbers are rejected.
fn canonical(base: &Config, key: &str, value: &str) -> Result<String, String> {
    let bad = |what: &str| format!("--param {key}: expects {what}, got {value:?}");
    Ok(match run::key_kind(base, key)? {
        KeyKind::Float => {
            let x: f32 = value.parse().map_err(|_| bad("a number"))?;
            if !x.is_finite() {
                return Err(bad("a finite number"));
            }
            format!("{x}")
        }
        KeyKind::Integer => format!("{}", value.parse::<i64>().map_err(|_| bad("an integer"))?),
        KeyKind::Boolean => format!(
            "{}",
            value.parse::<bool>().map_err(|_| bad("true or false"))?
        ),
        KeyKind::Other => value.to_string(),
    })
}

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
            let mut lists: Vec<(&str, Vec<String>)> = Vec::new();
            let mut total: usize = 1;
            for p in params {
                let Param::Values { key, values } = p else {
                    return Err(format!(
                        "--param {}: a range (lo..hi) needs --lhs N; a grid sweep lists values (a,b,c)",
                        p.key()
                    ));
                };
                let mut list: Vec<String> = Vec::new();
                for v in values {
                    let c = canonical(base, key, v)?;
                    if !list.contains(&c) {
                        list.push(c);
                    }
                }
                // Checked before anything is built, so a huge grid fails fast.
                total = total
                    .checked_mul(list.len())
                    .filter(|&t| t <= MAX_RUNS)
                    .ok_or_else(|| {
                        format!("the grid has more than {MAX_RUNS} points; narrow the sweep")
                    })?;
                lists.push((key, list));
            }
            let mut points: Vec<Point> = vec![Vec::new()];
            for (key, values) in &lists {
                points = points
                    .into_iter()
                    .flat_map(|pt| {
                        values.iter().map(move |v| {
                            let mut q = pt.clone();
                            q.push((key.to_string(), v.clone()));
                            q
                        })
                    })
                    .collect();
            }
            Ok(points)
        }
        Some(0) => Err("--lhs needs at least 1 point".into()),
        Some(n) if n > MAX_RUNS => Err(format!(
            "--lhs {n} is more than the {MAX_RUNS} points allowed"
        )),
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
                // Integer keys stratify the integers in [ceil(lo), floor(hi)]
                // (each equally likely), so every value stays in range and the
                // end points are not under-sampled. With more points than
                // integers, values repeat.
                let (lo, hi) = match kind {
                    KeyKind::Float => (*lo, *hi),
                    KeyKind::Integer => {
                        let (a, b) = (lo.ceil(), hi.floor());
                        if a > b {
                            return Err(format!(
                                "--param {key}: range {lo}..{hi} holds no integer"
                            ));
                        }
                        (a, b + 1.0)
                    }
                    _ => return Err(format!("--param {key}: only numeric keys can take a range")),
                };
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
                            KeyKind::Integer => {
                                format!("{}", (v.floor() as i64).min(hi as i64 - 1))
                            }
                            // Recorded exactly as the f32 the run will use.
                            _ => format!("{}", v as f32),
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
    /// The `--set` overrides already applied to the base config (recorded).
    pub sets: Vec<String>,
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
    /// `--set` overrides, and the full base config they produced.
    sets: &'a [String],
    base_config: &'a Config,
    points: &'a [Point],
}

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
    let runs = points
        .len()
        .checked_mul(a.seeds.len())
        .filter(|&r| r <= MAX_RUNS)
        .unwrap_or_else(|| {
            fail(format!(
                "{} point(s) × {} seed(s) is more than the {MAX_RUNS} runs allowed; narrow the sweep",
                points.len(),
                a.seeds.len()
            ))
        });
    for p in &a.params {
        let prefix = format!("{}=", p.key());
        if a.sets.iter().any(|kv| kv.trim_start().starts_with(&prefix)) {
            eprintln!(
                "warning: --set also sets {}; each point's --param value replaces it",
                p.key()
            );
        }
    }
    // Validate every point before running anything: each must parse, pass
    // `Config::validate`, and (in a grid) differ in results from every other
    // point, so a sweep over a debugging-only setting is refused.
    let mut seen: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
    let configs: Vec<Config> = points
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let mut c = point_config(base, p).unwrap_or_else(|e| fail(format!("point {k}: {e}")));
            if a.lhs.is_none() {
                if let Some(j) = seen.insert(c.digest(), k) {
                    fail(format!(
                        "points {j} and {k} give identical results (a debugging-only setting?); drop one"
                    ));
                }
            }
            c.sim.check_invariants |= a.check;
            c
        })
        .collect();
    std::fs::create_dir_all(&a.out).expect("create output dir");
    if a.out.join("sweep_design.json").exists() {
        eprintln!(
            "warning: {} already holds a sweep; its per-run files that this sweep does not \
             overwrite will remain",
            a.out.display()
        );
    }
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
    let width = (points.len() - 1).to_string().len().max(3);
    let results = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parallel::map_ordered(&jobs, a.jobs, |&(k, seed)| {
            let label = format!("p{k:0width$}");
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
            }));
            run.unwrap_or_else(|e| {
                let msg = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into());
                panic!("point {k} ({}), seed {seed}: {msg}", point_text(&points[k]))
            })
        })
    }))
    .unwrap_or_else(|e| {
        let msg = e
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| "a run panicked".into());
        fail(format!("{msg}; no sweep summary was written"))
    });

    std::fs::write(
        a.out.join("sweep_runs.csv"),
        runs_csv(&a.params, &points, &jobs, &results),
    )
    .expect("write runs");
    let summary = points_csv(&a.params, &points, a.seeds.len(), &results);
    std::fs::write(a.out.join("sweep_points.csv"), &summary).expect("write points");
    // Written last: a design file means the sweep finished.
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
        sets: &a.sets,
        base_config: base,
        points: &points,
    };
    std::fs::write(
        a.out.join("sweep_design.json"),
        serde_json::to_string_pretty(&design).expect("design serializes"),
    )
    .expect("write design");
    println!(
        "  wrote sweep_runs.csv, sweep_points.csv and sweep_design.json to {}",
        a.out.display()
    );
}

fn point_text(point: &Point) -> String {
    point
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ")
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
/// `results` holds `n_seeds` consecutive rows per point, in point order.
pub fn points_csv(
    params: &[Param],
    points: &[Point],
    n_seeds: usize,
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
        let rows: Vec<&Vec<f64>> = results[k * n_seeds..(k + 1) * n_seeds]
            .iter()
            .map(|(v, _)| v)
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
            p("resources.food_regrowth=0.03, 0.06"),
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
    fn grid_values_are_canonical_and_deduplicated_numerically() {
        let base = Config::default();
        let pts = design_points(
            &base,
            &[p("resources.food_regrowth=0.1,0.10,+0.1,1e-1,0.2")],
            None,
            1,
        )
        .unwrap();
        let vals: Vec<&str> = pts.iter().map(|pt| pt[0].1.as_str()).collect();
        assert_eq!(vals, ["0.1", "0.2"]);
        let pts = design_points(&base, &[p("life.population_cap=+300,0300,300")], None, 1).unwrap();
        assert_eq!(
            pts,
            vec![vec![("life.population_cap".into(), "300".into())]]
        );
        for bad in [
            "resources.food_regrowth=nan",
            "resources.food_regrowth=inf",
            "resources.food_regrowth=1e40",
            "life.population_cap=1.5",
            "social.trade_enabled=yes",
        ] {
            assert!(design_points(&base, &[p(bad)], None, 1).is_err(), "{bad}");
        }
    }

    #[test]
    fn huge_designs_are_refused_before_they_are_built() {
        let base = Config::default();
        let many = |key: &str| {
            let vals: Vec<String> = (1..=100).map(|i| i.to_string()).collect();
            p(&format!("{key}={}", vals.join(",")))
        };
        // 100^4 = 10^8 points: must fail on the size check, not by allocating.
        let params = [
            many("life.population_cap"),
            many("spawn.agents"),
            many("sight.radius"),
            many("homes.site_radius"),
        ];
        let e = design_points(&base, &params, None, 1).unwrap_err();
        assert!(e.contains("more than"), "{e}");
        let e = design_points(
            &base,
            &[p("resources.food_regrowth=0..1")],
            Some(MAX_RUNS + 1),
            1,
        )
        .unwrap_err();
        assert!(e.contains("more than"), "{e}");
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
        for pt in &a {
            let x: f32 = pt[0].1.parse().unwrap();
            assert_eq!(
                pt[0].1,
                format!("{x}"),
                "floats are recorded at f32 precision"
            );
        }
        assert!(design_points(&base, &[p("social.trade_enabled=0..1")], Some(3), 1).is_err());
        assert!(design_points(&base, &[p("resources.food_regrowth=0.1,0.2")], Some(3), 1).is_err());
        assert!(design_points(&base, &params, Some(0), 1).is_err());
    }

    #[test]
    fn latin_hypercube_integers_stay_in_range_and_cover_every_value() {
        let base = Config::default();
        // 2.4..6.6 holds the integers 3..=6; 40 points give each value 10.
        let pts = design_points(&base, &[p("sight.radius=2.4..6.6")], Some(40), 3).unwrap();
        let mut counts = std::collections::BTreeMap::new();
        for pt in &pts {
            *counts.entry(pt[0].1.parse::<i64>().unwrap()).or_insert(0) += 1;
        }
        assert_eq!(
            counts.into_iter().collect::<Vec<_>>(),
            [(3, 10), (4, 10), (5, 10), (6, 10)]
        );
        // With as many points as integers, each integer appears once.
        let pts = design_points(&base, &[p("sight.radius=1..8")], Some(8), 5).unwrap();
        let mut v: Vec<i64> = pts.iter().map(|pt| pt[0].1.parse().unwrap()).collect();
        v.sort_unstable();
        assert_eq!(v, (1..=8).collect::<Vec<_>>());
        assert!(design_points(&base, &[p("sight.radius=2.2..2.8")], Some(3), 1).is_err());
    }

    #[test]
    fn invalid_points_are_rejected_before_running() {
        let base = Config::default();
        assert!(
            point_config(&base, &vec![("resources.food_regrowth".into(), "x".into())]).is_err()
        );
        assert!(point_config(&base, &vec![("nope.key".into(), "1".into())]).is_err());
        // Semantically impossible settings fail too (Config::validate).
        for (k, v) in [
            ("resources.stone_min", "300"),
            ("world.width", "0"),
            ("resources.food_regrowth", "-0.1"),
            ("life.reproduce_chance", "1.5"),
            ("life.elder_days", "0"),
        ] {
            let e = point_config(&base, &vec![(k.into(), v.into())]).unwrap_err();
            assert!(e.contains(k.split('.').nth(1).unwrap()), "{k}={v}: {e}");
        }
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
                    sets: vec![],
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
            let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&out)
                .unwrap()
                .map(|e| {
                    let e = e.unwrap();
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        std::fs::read(e.path()).unwrap(),
                    )
                })
                .collect();
            files.sort();
            assert!(out.join("p001_seed2_manifest.json").exists());
            let design: serde_json::Value =
                serde_json::from_slice(&std::fs::read(out.join("sweep_design.json")).unwrap())
                    .unwrap();
            assert!(design["base_config"]["resources"]["food_regrowth"].is_number());
            let _ = std::fs::remove_dir_all(&out);
            files
        };
        let one = run_with(1);
        let text = |f: &str| {
            String::from_utf8(one.iter().find(|(n, _)| n == f).unwrap().1.clone()).unwrap()
        };
        assert_eq!(
            text("sweep_runs.csv").lines().count(),
            1 + 4,
            "2 points × 2 seeds"
        );
        let points = text("sweep_points.csv");
        assert_eq!(points.lines().count(), 1 + 2);
        assert!(points.lines().nth(1).unwrap().starts_with("0,2,0.05,"));
        // Every file (manifests and per-run metrics included), except the
        // manifests' absolute output paths, which name the folder.
        let strip = |files: Vec<(String, Vec<u8>)>, dir: &str| {
            files
                .into_iter()
                .map(|(n, b)| (n, String::from_utf8_lossy(&b).replace(dir, "OUT")))
                .collect::<Vec<_>>()
        };
        let dir = |j: usize| {
            std::env::temp_dir()
                .join(format!("accrete-sweep-{}-{j}", std::process::id()))
                .display()
                .to_string()
                .replace('\\', "\\\\")
        };
        assert_eq!(
            strip(one, &dir(1)),
            strip(run_with(4), &dir(4)),
            "byte-identical across job counts"
        );
    }
}
