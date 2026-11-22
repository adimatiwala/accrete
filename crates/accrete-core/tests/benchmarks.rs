//! Behavioural benchmarks and baselines (accrete.md Section 10, layers 3 and 4).
//!
//! Ranges live in `tests/benchmarks/primary_seed.toml`. Unlike golden hashes,
//! they only fail when behaviour leaves a sensible range.

mod common;

use accrete_core::config::{AgentKind, TICKS_PER_DAY};
use accrete_core::{Config, Simulation};

fn ranges() -> toml::Table {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/benchmarks/primary_seed.toml");
    let text = std::fs::read_to_string(&path).expect("tests/benchmarks/primary_seed.toml");
    toml::from_str(&text).expect("benchmark ranges parse")
}

fn num(v: &toml::Value) -> f64 {
    v.as_float()
        .or_else(|| v.as_integer().map(|i| i as f64))
        .expect("number")
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
        "homes_near_fertile" => {
            let w = &sim.world;
            let occupied: Vec<_> = w.homes.iter().filter(|h| !h.is_vacant()).collect();
            let near = occupied
                .iter()
                .filter(|h| {
                    (-3..=3).any(|dy| {
                        (-3..=3).any(|dx| {
                            let p = h.pos.offset(dx, dy);
                            w.contains(p) && w.tile(p).fertility >= 0.3
                        })
                    })
                })
                .count();
            Some(near as f64 / occupied.len().max(1) as f64)
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
        cfg["seed"].as_integer(),
        Some(common::PRIMARY_SEED as i64),
        "benchmarks are set for the primary seed"
    );
    let day = cfg["day"].as_integer().expect("day") as u32;
    let sim = run(AgentKind::Normal, day);
    let m = sim.metrics.last().expect("metrics");
    assert_eq!(m.day, day);
    assert!(!m.at_cap, "the run hit the population cap");
    let row = serde_json::to_value(m).expect("metrics serialize");

    let mut failures = Vec::new();
    for (key, range) in cfg["ranges"].as_table().expect("[ranges]") {
        let r = range.as_array().expect("range is [lo, hi]");
        let (lo, hi) = (num(&r[0]), num(&r[1]));
        let value = derived(&sim, key)
            .or_else(|| row.get(key).and_then(|v| v.as_f64()))
            .unwrap_or_else(|| panic!("unknown benchmark key {key}"));
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
    let b = cfg["baselines"].as_table().expect("[baselines]");
    let days = b["baseline_day"].as_integer().expect("baseline_day") as u32;
    let pop = |k| run(k, days).population() as f64;
    let (random, normal, omniscient) = (
        pop(AgentKind::Random),
        pop(AgentKind::Normal),
        pop(AgentKind::Omniscient),
    );
    println!("  day {days}: random {random}, normal {normal}, omniscient {omniscient}");
    let lo = num(&b["min_population_ratio_normal_over_random"]);
    let hi = num(&b["max_population_ratio_normal_over_omniscient"]);
    assert!(
        normal >= lo * random.max(1.0),
        "normal agents ({normal}) are not clearly better than random ({random}); the decision logic may be broken"
    );
    assert!(
        normal <= hi * omniscient,
        "normal agents ({normal}) beat omniscient ones ({omniscient}); limited knowledge is not constraining them"
    );
}
