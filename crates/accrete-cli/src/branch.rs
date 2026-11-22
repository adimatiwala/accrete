//! `accrete branch`: run a control and an intervention from one snapshot and
//! compare them day by day (accrete.md, Week 4 Day 3).
//!
//! Both branches load the same snapshot, so they share the world, agents and
//! every RNG stream up to the branch point. The intervention differs only in
//! its configuration: scripted droughts (`--drought`), config overrides
//! (`--set`), or a whole config file (`--config-b`). Interventions change
//! world conditions, never agents directly. Random droughts come from the
//! `weather` stream, which scripted droughts never draw from, so both
//! branches see the same random droughts.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use accrete_core::config::ScriptedDrought;
use accrete_core::simulation::metrics::DailyMetrics;
use accrete_core::{Config, Simulation};

use crate::run::{self, RunOpts};

pub struct BranchArgs {
    pub load: PathBuf,
    pub days: u32,
    pub out: PathBuf,
    pub every: u32,
    pub config_b: Option<Config>,
    pub sets_b: Vec<String>,
    pub droughts_b: Vec<String>,
    pub label_a: String,
    pub label_b: String,
}

/// Parse `--drought DAY:DAYS:X:Y:RADIUS:SEVERITY`.
pub fn parse_drought(s: &str) -> Result<ScriptedDrought, String> {
    let p: Vec<&str> = s.split(':').collect();
    let err = || format!("--drought expects DAY:DAYS:X:Y:RADIUS:SEVERITY, got {s}");
    if p.len() != 6 {
        return Err(err());
    }
    let u = |i: usize| p[i].trim().parse::<u32>().map_err(|_| err());
    let i = |i: usize| p[i].trim().parse::<i32>().map_err(|_| err());
    let severity: f32 = p[5].trim().parse().map_err(|_| err())?;
    if !(0.0..=1.0).contains(&severity) {
        return Err(format!(
            "--drought severity must be in 0..=1, got {severity}"
        ));
    }
    Ok(ScriptedDrought {
        start_day: u(0)?,
        days: u(1)?,
        center: [i(2)?, i(3)?],
        radius: u(4)?,
        severity,
    })
}

pub fn branch(a: BranchArgs) {
    std::fs::create_dir_all(&a.out).expect("create output dir");
    let mut control = run::load(&a.load, None, &a.label_a);
    let start_day = control.day();

    // Intervention config: the snapshot's config (or --config-b), then
    // overrides, then extra scripted droughts.
    let mut cfg = a.config_b.clone().unwrap_or_else(|| control.config.clone());
    for kv in &a.sets_b {
        cfg = run::apply_override(&cfg, kv);
    }
    for d in &a.droughts_b {
        let d = parse_drought(d).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(2)
        });
        if d.start_day < start_day {
            eprintln!(
                "error: drought starts on day {} but the snapshot is at day {start_day}; \
                 a scripted drought only starts on its start day",
                d.start_day
            );
            std::process::exit(2);
        }
        cfg.weather.scripted.push(d);
    }
    if cfg == control.config {
        eprintln!(
            "warning: the intervention has the same config as the control, so the branches will be identical"
        );
    }
    let mut intervention = run::load(&a.load, Some(cfg), &a.label_b);

    let out: &Path = &a.out;
    control = run::drive(
        control,
        &RunOpts {
            days: a.days,
            out,
            every: a.every,
            snap_every: 0,
            label: &a.label_a,
            timelapse: &[],
            save: None,
        },
    );
    intervention = run::drive(
        intervention,
        &RunOpts {
            days: a.days,
            out,
            every: a.every,
            snap_every: 0,
            label: &a.label_b,
            timelapse: &[],
            save: None,
        },
    );

    let seed = control.world.seed;
    let path = out.join(format!(
        "{}_vs_{}_seed{seed}_compare.csv",
        a.label_a, a.label_b
    ));
    std::fs::write(
        &path,
        compare_csv(&control, &intervention, start_day, &a.label_a, &a.label_b),
    )
    .expect("write comparison");
    print_comparison(&control, &intervention, &a.label_a, &a.label_b);
    println!("  comparison written to {}", path.display());
}

/// Metrics compared side by side, as (column name, accessor).
type Col = (&'static str, fn(&DailyMetrics) -> f64);

const COLUMNS: &[Col] = &[
    ("population", |m| f64::from(m.population)),
    ("births", |m| f64::from(m.births)),
    ("deaths_deprivation", |m| f64::from(m.deaths_deprivation)),
    ("deaths_old_age", |m| f64::from(m.deaths_old_age)),
    ("mean_hunger", |m| f64::from(m.mean_hunger)),
    ("mean_frailty", |m| f64::from(m.mean_frailty)),
    ("food_world", |m| f64::from(m.food_world)),
    ("food_stored", |m| f64::from(m.food_stored)),
    ("homes_occupied", |m| f64::from(m.homes_occupied)),
    ("home_moves", |m| f64::from(m.home_moves)),
    ("hamlets", |m| f64::from(m.hamlets)),
    ("settlement_candidates", |m| {
        f64::from(m.settlement_candidates)
    }),
    ("clark_evans", |m| f64::from(m.clark_evans)),
    ("forage_radius", |m| f64::from(m.forage_radius)),
    ("trades", |m| f64::from(m.trades)),
    ("drought_area", |m| f64::from(m.drought_area)),
];

/// One row per day after the branch point: for each metric, the control
/// value, the intervention value, and their difference (intervention minus
/// control).
pub fn compare_csv(a: &Simulation, b: &Simulation, from_day: u32, la: &str, lb: &str) -> String {
    let mut s = String::from("day");
    for (name, _) in COLUMNS {
        let _ = write!(s, ",{name}_{la},{name}_{lb},{name}_diff");
    }
    s.push('\n');
    for (ma, mb) in a.metrics.iter().zip(&b.metrics) {
        debug_assert_eq!(ma.day, mb.day);
        if ma.day <= from_day {
            continue;
        }
        let _ = write!(s, "{}", ma.day);
        for (_, f) in COLUMNS {
            let (x, y) = (f(ma), f(mb));
            let _ = write!(s, ",{x},{y},{}", y - x);
        }
        s.push('\n');
    }
    s
}

fn print_comparison(a: &Simulation, b: &Simulation, la: &str, lb: &str) {
    let (Some(ma), Some(mb)) = (a.metrics.last(), b.metrics.last()) else {
        return;
    };
    println!("  day {}: {la} vs {lb}", ma.day);
    for (name, f) in COLUMNS {
        let (x, y) = (f(ma), f(mb));
        println!("    {name:<22} {x:>10.2} {y:>10.2}   diff {:>+10.2}", y - x);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drought_spec_parses_and_validates() {
        let d = parse_drought("100:200:64:60:20:0.6").unwrap();
        assert_eq!(
            (d.start_day, d.days, d.center, d.radius),
            (100, 200, [64, 60], 20)
        );
        assert!((d.severity - 0.6).abs() < 1e-6);
        assert!(parse_drought("100:200:64:60:20").is_err());
        assert!(parse_drought("100:200:64:60:20:1.5").is_err());
        assert!(parse_drought("a:200:64:60:20:0.5").is_err());
    }
}
