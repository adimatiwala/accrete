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

use accrete_core::config::{DisruptionKind, ScriptedDisruption, ScriptedDrought};
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
    pub disruptions_b: Vec<String>,
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
    let days = u(1)?;
    if days == 0 {
        return Err("--drought DAYS must be at least 1".into());
    }
    Ok(ScriptedDrought {
        start_day: u(0)?,
        days,
        center: [i(2)?, i(3)?],
        radius: u(4)?,
        severity,
    })
}

/// Parse `--disrupt DAY:DAYS:KIND:X:Y:RADIUS` or `DAY:DAYS:KIND:busiest:N`,
/// where KIND is `block` or `slowF` (entering costs F times as much).
pub fn parse_disruption(s: &str) -> Result<ScriptedDisruption, String> {
    let p: Vec<&str> = s.split(':').map(str::trim).collect();
    let err = || {
        format!(
            "--disrupt expects DAY:DAYS:block|slowF:X:Y:RADIUS or DAY:DAYS:block|slowF:busiest:N, got {s}"
        )
    };
    let u = |x: &str| x.parse::<u32>().map_err(|_| err());
    let (kind, slow_factor) = match p.get(2).copied() {
        Some("block") => (DisruptionKind::Block, 3),
        Some(k) if k.starts_with("slow") => {
            let f: u8 = k[4..].parse().map_err(|_| err())?;
            if f < 2 {
                return Err(format!("--disrupt slow factor must be at least 2, got {f}"));
            }
            (DisruptionKind::Slow, f)
        }
        _ => return Err(err()),
    };
    let mut d = ScriptedDisruption {
        start_day: u(p.first().copied().ok_or_else(err)?)?,
        days: u(p.get(1).copied().ok_or_else(err)?)?,
        kind,
        slow_factor,
        center: [0, 0],
        radius: 0,
        busiest_tiles: 0,
    };
    match p.len() {
        5 if p[3] == "busiest" => d.busiest_tiles = u(p[4])?,
        6 => {
            d.center = [
                p[3].parse().map_err(|_| err())?,
                p[4].parse().map_err(|_| err())?,
            ];
            d.radius = u(p[5])?;
        }
        _ => return Err(err()),
    }
    if p[3] == "busiest" && d.busiest_tiles == 0 {
        return Err("--disrupt busiest:N needs N of at least 1".into());
    }
    d.validate().map_err(|e| format!("--disrupt: {e}"))?;
    Ok(d)
}

pub fn branch(a: BranchArgs) {
    let fail = |msg: String| -> ! {
        eprintln!("error: {msg}");
        std::process::exit(2)
    };
    for l in [&a.label_a, &a.label_b] {
        run::check_label(l).unwrap_or_else(|e| fail(e));
    }
    if a.label_a == a.label_b {
        fail(format!(
            "--label-a and --label-b are both {:?}; their output files would overwrite each other",
            a.label_a
        ));
    }
    std::fs::create_dir_all(&a.out).expect("create output dir");
    let mut control = run::load(&a.load, None, &a.label_a);
    let start_day = control.day();
    let end_day = start_day + a.days;

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
        if d.start_day >= end_day {
            eprintln!(
                "warning: drought starts on day {} but the branches stop on day {end_day}; it will not happen",
                d.start_day
            );
        }
        cfg.weather.scripted.push(d);
    }
    for d in &a.disruptions_b {
        let d = parse_disruption(d).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(2)
        });
        if d.start_day < start_day {
            eprintln!(
                "error: disruption starts on day {} but the snapshot is at day {start_day}",
                d.start_day
            );
            std::process::exit(2);
        }
        if d.start_day >= end_day {
            eprintln!(
                "warning: disruption starts on day {} but the branches stop on day {end_day}; it will not happen",
                d.start_day
            );
        }
        cfg.disruption.scripted.push(d);
    }
    if !cfg.differs_in_results(&control.config) {
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
            stop_on_extinction: false,
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
            stop_on_extinction: false,
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
    ("travel_per_food", |m| f64::from(m.travel_per_food)),
    ("trades", |m| f64::from(m.trades)),
    ("food_traded", |m| f64::from(m.food_traded)),
    ("tips_shared", |m| f64::from(m.tips_shared)),
    ("trail_tiles", |m| f64::from(m.trail_tiles)),
    ("road_tiles", |m| f64::from(m.road_tiles)),
    ("drought_area", |m| f64::from(m.drought_area)),
    ("disrupted_tiles", |m| f64::from(m.disrupted_tiles)),
    ("tiles_moved", |m| m.tiles_moved as f64),
    ("route_failures", |m| f64::from(m.route_failures)),
    ("pathology_oscillating", |m| m.pathology_oscillating as f64),
    ("pathology_stuck", |m| m.pathology_stuck as f64),
    ("pathology_starved_beside_food", |m| {
        m.pathology_starved_beside_food as f64
    }),
    ("pathology_idle", |m| m.pathology_idle as f64),
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
    // Align by day; a day missing on one side leaves its cells empty.
    fn by_day(sim: &Simulation) -> std::collections::BTreeMap<u32, &DailyMetrics> {
        sim.metrics.iter().map(|m| (m.day, m)).collect()
    }
    let (da, db) = (by_day(a), by_day(b));
    let days: std::collections::BTreeSet<u32> = da.keys().chain(db.keys()).copied().collect();
    for day in days.into_iter().filter(|&d| d > from_day) {
        let _ = write!(s, "{day}");
        for (_, f) in COLUMNS {
            match (da.get(&day), db.get(&day)) {
                (Some(ma), Some(mb)) => {
                    let (x, y) = (f(ma), f(mb));
                    let _ = write!(s, ",{x},{y},{}", y - x);
                }
                (Some(ma), None) => {
                    let _ = write!(s, ",{},,", f(ma));
                }
                (None, Some(mb)) => {
                    let _ = write!(s, ",,{},", f(mb));
                }
                (None, None) => unreachable!("day came from one of the two maps"),
            }
        }
        s.push('\n');
    }
    s
}

pub fn print_comparison(a: &Simulation, b: &Simulation, la: &str, lb: &str) {
    let (Some(ma), Some(mb)) = (a.metrics.last(), b.metrics.last()) else {
        return;
    };
    if ma.day == mb.day {
        println!("  day {}: {la} vs {lb}", ma.day);
    } else {
        println!("  {la} (day {}) vs {lb} (day {})", ma.day, mb.day);
    }
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

    #[test]
    fn comparison_rows_line_up_by_day_when_a_branch_ends_early() {
        use accrete_core::simulation::metrics::DailyMetrics;
        let world = accrete_core::scenario::Scenario::from_toml_str(
            "name = \"tiny\"\nmap = \"\"\"\n...\n\"\"\"\n",
        )
        .unwrap()
        .world(&Default::default())
        .unwrap();
        let mut a = Simulation::from_world(world.clone(), Default::default());
        let mut b = Simulation::from_world(world, Default::default());
        let row = |day: u32, population: u32| DailyMetrics {
            day,
            population,
            ..DailyMetrics::default()
        };
        a.metrics = (1..=5).map(|d| row(d, 10)).collect();
        b.metrics = (1..=3).map(|d| row(d, 7)).collect();
        let csv = compare_csv(&a, &b, 1, "control", "x");
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 1 + 4, "header + days 2..=5");
        assert!(lines[1].starts_with("2,10,7,-3"), "{}", lines[1]);
        assert!(
            lines[4].starts_with("5,10,,"),
            "missing side left empty: {}",
            lines[4]
        );
    }

    #[test]
    fn zero_day_droughts_are_rejected() {
        assert!(parse_drought("100:0:64:60:20:0.6").is_err());
    }

    #[test]
    fn disruption_spec_parses_and_validates() {
        let d = parse_disruption("60:15:block:busiest:12").unwrap();
        assert_eq!((d.start_day, d.days, d.busiest_tiles), (60, 15, 12));
        assert_eq!(d.kind, DisruptionKind::Block);
        let d = parse_disruption("60:15:slow4:64:50:3").unwrap();
        assert_eq!(
            (d.kind, d.slow_factor, d.center, d.radius),
            (DisruptionKind::Slow, 4, [64, 50], 3)
        );
        for bad in [
            "60:15:flood:busiest:3",
            "60:15:slow1:1:1:1",
            "60:15:block:1:1",
            "x:15:block:busiest:3",
            "60:0:block:busiest:3",
            "60:15:block:busiest:0",
            "60:15:slow255:busiest:3",
        ] {
            assert!(parse_disruption(bad).is_err(), "{bad}");
        }
    }
}
