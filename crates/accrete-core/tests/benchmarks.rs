//! Behavioural benchmarks and baselines (accrete.md Section 10, layers 3 and 4).
//!
//! Ranges live in `tests/benchmarks/primary_seed.toml`. Unlike golden hashes,
//! they only fail when behaviour leaves a sensible range.

mod common;

use accrete_core::config::{AgentKind, TICKS_PER_DAY};
use accrete_core::{Config, Simulation};

const RANGES: &str = "tests/benchmarks/primary_seed.toml";

fn ranges() -> toml::Table {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(RANGES);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{RANGES}: {e}"));
    toml::from_str(&text).unwrap_or_else(|e| panic!("{RANGES} does not parse: {e}"))
}

/// A number from the ranges file, with a clear error naming the key.
fn num(v: &toml::Value, what: &str) -> f64 {
    v.as_float()
        .or_else(|| v.as_integer().map(|i| i as f64))
        .unwrap_or_else(|| panic!("{RANGES}: {what} must be a number, got {v}"))
}

fn get<'a>(t: &'a toml::Table, key: &str) -> &'a toml::Value {
    t.get(key)
        .unwrap_or_else(|| panic!("{RANGES}: missing key `{key}`"))
}

/// Share of `tiles` within `radius` of a tile with fertility >= `min_fert`.
fn near_fertile_share(
    sim: &Simulation,
    tiles: &[accrete_core::Pos],
    radius: i32,
    min_fert: f32,
) -> f64 {
    let w = &sim.world;
    let near = tiles
        .iter()
        .filter(|&&q| {
            (-radius..=radius).any(|dy| {
                (-radius..=radius).any(|dx| {
                    let p = q.offset(dx, dy);
                    w.contains(p) && w.tile(p).fertility >= min_fert
                })
            })
        })
        .count();
    near as f64 / tiles.len().max(1) as f64
}

fn run(kind: AgentKind, days: u32) -> Simulation {
    let mut c = Config::default();
    c.sim.agent_kind = kind;
    let mut sim = Simulation::new(common::PRIMARY_SEED, c);
    sim.run(u64::from(days) * TICKS_PER_DAY);
    sim
}

/// Derived measures named in the ranges file.
fn derived(sim: &Simulation, key: &str) -> Option<f64> {
    match key {
        "travel_per_food_run" => {
            let moved: f64 = sim.metrics.iter().map(|m| m.tiles_moved as f64).sum();
            let food: f64 = sim.metrics.iter().map(|m| f64::from(m.food_gathered)).sum();
            Some(moved / food.max(1.0))
        }
        "homes_near_fertile_lift" => {
            // How much more often occupied homes sit near good land than
            // land tiles in general do (0 = no better than random placement).
            let w = &sim.world;
            let homes: Vec<_> = w
                .homes
                .iter()
                .filter(|h| !h.is_vacant())
                .map(|h| h.pos)
                .collect();
            let land: Vec<_> = (0..w.dims.len() as u32)
                .map(|i| w.dims.pos(i))
                .filter(|&p| w.tile(p).is_land())
                .collect();
            Some(near_fertile_share(sim, &homes, 2, 0.5) - near_fertile_share(sim, &land, 2, 0.5))
        }
        "births_and_old_age_deaths" => {
            let m = sim.metrics.last()?;
            Some(f64::from(u8::from(
                m.total_births > 0 && m.total_deaths_old_age > 0,
            )))
        }
        _ => None,
    }
}

#[test]
fn behaviour_stays_within_benchmark_ranges() {
    let cfg = ranges();
    assert_eq!(
        num(get(&cfg, "seed"), "seed") as u64,
        common::PRIMARY_SEED,
        "benchmarks are set for the primary seed"
    );
    let day = num(get(&cfg, "day"), "day") as u32;
    let sim = run(AgentKind::Normal, day);
    let m = sim.metrics.last().expect("metrics");
    assert_eq!(m.day, day);
    let row = serde_json::to_value(m).expect("metrics serialize");

    let mut failures = Vec::new();
    let table = get(&cfg, "ranges")
        .as_table()
        .unwrap_or_else(|| panic!("{RANGES}: [ranges] must be a table"));
    for (key, range) in table {
        let r = match range.as_array() {
            Some(r) if r.len() == 2 => r,
            _ => panic!("{RANGES}: {key} must be [lo, hi], got {range}"),
        };
        let (lo, hi) = (num(&r[0], key), num(&r[1], key));
        assert!(lo <= hi, "{RANGES}: {key} has lo > hi");
        // Booleans in the metrics row count as 0 or 1.
        let value = derived(&sim, key)
            .or_else(|| {
                row.get(key).and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_bool().map(|b| f64::from(u8::from(b))))
                })
            })
            .unwrap_or_else(|| panic!("{RANGES}: unknown benchmark key `{key}`"));
        println!("  {key:<32} {value:>10.3}   range [{lo}, {hi}]");
        if !(lo..=hi).contains(&value) {
            failures.push(format!("{key} = {value:.3} outside [{lo}, {hi}]"));
        }
    }
    assert!(
        failures.is_empty(),
        "benchmarks failed:\n  {}",
        failures.join("\n  ")
    );
}

#[test]
fn normal_agents_sit_between_random_and_omniscient_baselines() {
    let cfg = ranges();
    let b = get(&cfg, "baselines")
        .as_table()
        .unwrap_or_else(|| panic!("{RANGES}: [baselines] must be a table"));
    let days = num(get(b, "baseline_day"), "baseline_day") as u32;
    let pop = |k| run(k, days).population() as f64;
    let (random, normal, omniscient) = (
        pop(AgentKind::Random),
        pop(AgentKind::Normal),
        pop(AgentKind::Omniscient),
    );
    println!("  day {days}: random {random}, normal {normal}, omniscient {omniscient}");
    let lo = num(
        get(b, "min_population_ratio_normal_over_random"),
        "min_population_ratio_normal_over_random",
    );
    let hi = num(
        get(b, "max_population_ratio_normal_over_omniscient"),
        "max_population_ratio_normal_over_omniscient",
    );
    assert!(
        normal >= lo * random.max(1.0),
        "normal agents ({normal}) are not clearly better than random ({random}); the decision logic may be broken"
    );
    assert!(
        normal <= hi * omniscient,
        "normal agents ({normal}) beat omniscient ones ({omniscient}); limited knowledge is not constraining them"
    );
}
