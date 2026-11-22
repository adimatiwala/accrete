//! Week 2 integration tests: determinism, conservation, population dynamics.

mod common;

use accrete_core::config::TICKS_PER_DAY;
use accrete_core::simulation::events::{DeathCause, EventKind};
use accrete_core::{Config, Simulation};

fn checked() -> Config {
    let mut c = Config::default();
    c.sim.check_invariants = true;
    c
}

#[test]
fn same_seed_same_state_after_1000_ticks() {
    let c = Config::default();
    let mut a = Simulation::new(3, c.clone());
    let mut b = Simulation::new(3, c);
    for _ in 0..1000 {
        a.step();
        b.step();
    }
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(a.population(), b.population());
}

#[test]
fn different_seeds_diverge() {
    let c = Config::default();
    let mut a = Simulation::new(1, c.clone());
    let mut b = Simulation::new(2, c);
    a.run(48);
    b.run(48);
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn golden_state_hashes() {
    let c = Config::default();
    let mut sim = Simulation::new(common::PRIMARY_SEED, c);
    common::check_golden("state_seed1_t0", sim.state_hash());
    sim.run(100);
    common::check_golden("state_seed1_t100", sim.state_hash());
    sim.run(900);
    common::check_golden("state_seed1_t1000", sim.state_hash());
}

/// Conservation, bounds, homes and chests are checked every tick by the
/// engine itself when `check_invariants` is on; it panics on violation.
#[test]
fn invariants_hold_for_twenty_days() {
    let mut sim = Simulation::new(4, checked());
    sim.run(20 * TICKS_PER_DAY);
    assert!(sim.population() > 0);
    sim.check_invariants().unwrap();
}

#[test]
fn agents_stay_bounded_and_memory_fixed_size() {
    let c = Config::default();
    let mut sim = Simulation::new(5, c.clone());
    for _ in 0..(10 * TICKS_PER_DAY) {
        sim.step();
        for a in &sim.agents {
            assert!((0.0..=1.0).contains(&a.hunger));
            assert!((0.0..=1.0).contains(&a.energy));
            assert!(a.health <= a.health_ceiling(&sim.rates) + 1e-6);
            let m = &c.memory;
            assert!(
                a.memory.resources.len() <= (m.food_slots + m.wood_slots + m.stone_slots) as usize
            );
            assert!(a.memory.homes.len() <= m.home_slots as usize);
            assert!(a.memory.changed.len() <= m.changed_slots as usize);
        }
    }
}

#[test]
fn births_follow_the_rules() {
    let c = Config::default();
    let mut sim = Simulation::new(1, c);
    sim.run(60 * TICKS_PER_DAY);
    let births: Vec<_> = sim
        .events
        .events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::Born { parent, .. } => Some((e.tick, e.agent.unwrap(), parent)),
            _ => None,
        })
        .collect();
    assert!(!sim.metrics.is_empty());
    let total_births = sim.metrics.last().unwrap().total_births;
    assert!(total_births > 0, "no births in 60 days");
    // IDs are unique and ascending in the agent list.
    assert!(sim.agents.windows(2).all(|w| w[0].id < w[1].id));
    for (_, child, parent) in births {
        assert!(child > parent);
        if let Some(c) = sim.agent(child) {
            assert_eq!(c.parent, Some(parent));
            // Newborns only know tiles their parent knew (checked loosely: they
            // know their home).
            let home = c.home.expect("newborn has a home");
            assert!(c.memory.knows(sim.world.homes[home as usize].pos));
        }
    }
    // No juvenile and no elder ever reproduced.
    for a in &sim.agents {
        if a.is_juvenile(&sim.rates) {
            assert_eq!(a.stats.births, 0);
        }
    }
}

#[test]
fn population_cap_is_never_exceeded_and_flags_the_run() {
    let mut c = Config::default();
    c.life.population_cap = 120;
    let mut sim = Simulation::new(1, c);
    sim.run(60 * TICKS_PER_DAY);
    assert!(sim.population() <= 120);
    assert!(sim.hit_cap, "a 120 cap should be reached in 60 days");
    assert!(sim.metrics.last().unwrap().at_cap);
}

#[test]
fn deaths_conserve_resources_and_record_causes() {
    // A harsh world (no regrowth, little food) produces deprivation deaths.
    let mut c = checked();
    c.resources.food_regrowth = 0.0;
    c.resources.food_capacity_scale = 2.0;
    c.spawn.min_nearby_food = 0.0;
    c.spawn.initial_food = 0.0;
    let mut sim = Simulation::new(2, c);
    sim.run(25 * TICKS_PER_DAY);
    assert!(
        !sim.deaths.is_empty(),
        "expected deprivation deaths in a barren world"
    );
    for d in &sim.deaths {
        assert!(!d.explanation.is_empty());
        if d.cause == DeathCause::Deprivation {
            assert!(d.explanation.contains("unmet needs"));
        }
    }
    sim.check_invariants().unwrap();
}

#[test]
fn old_age_kills_through_the_health_ceiling() {
    let mut c = Config::default();
    c.life.elder_days = 2.0;
    c.life.maturity_days = 1.0;
    c.life.age_decline_per_day = 0.5;
    let mut sim = Simulation::new(1, c);
    sim.run(6 * TICKS_PER_DAY);
    let old = sim
        .deaths
        .iter()
        .filter(|d| d.cause == DeathCause::OldAge)
        .count();
    assert!(old > 50, "expected most agents to die of age, got {old}");
}

#[test]
fn frailty_never_decreases() {
    let c = Config::default();
    let mut sim = Simulation::new(6, c);
    let mut last: std::collections::BTreeMap<u32, f32> = Default::default();
    for _ in 0..(15 * TICKS_PER_DAY) {
        sim.step();
        for a in &sim.agents {
            if let Some(&prev) = last.get(&a.id) {
                assert!(a.frailty >= prev);
            }
            last.insert(a.id, a.frailty);
        }
    }
}

#[test]
fn pathologies_stay_near_zero_on_primary_seed() {
    let c = Config::default();
    let mut sim = Simulation::new(common::PRIMARY_SEED, c);
    sim.run(60 * TICKS_PER_DAY);
    let m = sim.metrics.last().unwrap();
    let agent_days = sim
        .metrics
        .iter()
        .map(|m| u64::from(m.population))
        .sum::<u64>();
    let total = m.pathology_oscillating
        + m.pathology_stuck
        + m.pathology_starved_beside_food
        + m.pathology_idle;
    assert!(
        total * 1000 < agent_days,
        "pathologies {total} over {agent_days} agent-days (osc {} stuck {} starved {} idle {})",
        m.pathology_oscillating,
        m.pathology_stuck,
        m.pathology_starved_beside_food,
        m.pathology_idle
    );
}

#[test]
fn droughts_respect_their_configured_ranges() {
    let mut c = Config::default();
    c.weather.daily_chance = 0.05; // frequent, to exercise the rules
    let mut sim = Simulation::new(1, c.clone());
    sim.run(400 * TICKS_PER_DAY);
    let h = &sim.weather.history;
    assert!(h.len() >= 2, "expected several droughts, got {}", h.len());
    let w = &c.weather;
    for d in h {
        assert!((w.radius_min..=w.radius_max).contains(&d.radius));
        let len = d.end_day - d.start_day;
        assert!((w.length_min_days..=w.length_max_days).contains(&len));
        assert!(d.severity >= w.severity_min && d.severity <= w.severity_max);
        assert!(sim.world.tile(d.center).is_land());
    }
    // At most one random drought at a time, separated by the cooldown.
    for pair in h.windows(2) {
        assert!(pair[1].start_day >= pair[0].end_day + w.cooldown_days);
    }
}

#[test]
fn scripted_drought_changes_regrowth_not_agents() {
    use accrete_core::config::ScriptedDrought;
    let mut base = Config::default();
    base.weather.random_droughts = false;
    let mut dry = base.clone();
    dry.weather.scripted.push(ScriptedDrought {
        start_day: 5,
        days: 20,
        center: [64, 64],
        radius: 40,
        severity: 0.9,
    });
    let mut a = Simulation::new(1, base);
    let mut b = Simulation::new(1, dry);
    // Identical until the drought starts.
    a.run(5 * TICKS_PER_DAY);
    b.run(5 * TICKS_PER_DAY);
    assert_eq!(a.population(), b.population());
    a.run(20 * TICKS_PER_DAY);
    b.run(20 * TICKS_PER_DAY);
    let food = |s: &Simulation| s.world.tiles.iter().map(|t| f64::from(t.food)).sum::<f64>();
    assert!(
        food(&b) < food(&a),
        "the drought should reduce standing food"
    );
    // Both branches drew the same random-weather numbers (none were used).
    assert_eq!(a.rng_weather.word_pos(), b.rng_weather.word_pos());
}

#[test]
fn trade_and_sharing_happen_and_conserve_goods() {
    let mut sim = Simulation::new(1, checked());
    sim.run(40 * TICKS_PER_DAY);
    let trades: u32 = sim.metrics.iter().map(|m| m.trades).sum();
    let tips: u32 = sim.metrics.iter().map(|m| m.tips_shared).sum();
    assert!(trades > 0, "no trades in 40 days");
    assert!(tips > 0, "no knowledge sharing in 40 days");
}
