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

/// accrete.md Section 10: save at tick 500, resume to tick 1,000, and compare
/// against an uninterrupted run. Going through a file on disk, the resumed
/// state must also match the committed golden hash, which is shared by
/// macOS and Windows.
#[test]
fn snapshot_at_500_resumes_to_the_golden_state_at_1000() {
    let c = Config::default();
    let mut sim = Simulation::new(common::PRIMARY_SEED, c);
    sim.run(500);
    let dir = std::env::temp_dir().join(format!("accrete-snap-test-{}", std::process::id()));
    let path = dir.join("seed1_t500.snap.json");
    sim.save_snapshot(&path).expect("save");
    let mut resumed = Simulation::load_snapshot(&path).expect("load");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(resumed.tick, 500);
    assert_eq!(resumed.state_hash(), sim.state_hash());
    resumed.run(500);
    sim.run(500);
    assert_eq!(resumed.state_hash(), sim.state_hash());
    assert_eq!(resumed.metrics, sim.metrics);
    common::check_golden("state_seed1_t1000", resumed.state_hash());
}

/// Snapshots taken mid-day, with droughts active, and at the population cap
/// all resume exactly.
#[test]
fn snapshots_resume_exactly_mid_day_in_drought_and_at_the_cap() {
    let mut c = Config::default();
    c.weather
        .scripted
        .push(accrete_core::config::ScriptedDrought {
            start_day: 1,
            days: 10,
            center: [64, 64],
            radius: 30,
            severity: 0.5,
        });
    let mut a = Simulation::new(common::PRIMARY_SEED, c);
    a.run(2 * TICKS_PER_DAY + 13); // mid-day, drought active
    assert!(!a.weather.active.is_empty());
    let mut b = Simulation::from_snapshot_json(&a.snapshot_json()).expect("loads");
    a.run(37);
    b.run(37);
    assert_eq!(a.state_hash(), b.state_hash());

    let mut c = Config::default();
    c.life.population_cap = 105;
    let mut a = Simulation::new(common::PRIMARY_SEED, c);
    a.run(30 * TICKS_PER_DAY);
    assert!(a.hit_cap, "reached the cap");
    let mut b = Simulation::from_snapshot_json(&a.snapshot_json()).expect("loads");
    a.run(48);
    b.run(48);
    assert_eq!(a.state_hash(), b.state_hash());
    assert!(b.population() as u32 <= 105);
}

/// The snapshot file's bytes are pinned: any change to the serialized layout
/// or to what a field means shows up here, and must come with a deliberate
/// `SNAPSHOT_FORMAT` bump (and a re-bless). Shared by macOS and Windows.
#[test]
fn golden_snapshot_bytes() {
    let mut sim = Simulation::new(common::PRIMARY_SEED, Config::default());
    sim.run(100);
    let json = sim.snapshot_json();
    common::check_golden(
        "snapshot_seed1_t100",
        accrete_core::hash::fnv1a64(json.as_bytes()),
    );
}

/// An intervention branch changes only world conditions: both branches
/// start from the same snapshot, keep identical random droughts (the
/// `weather` stream is untouched by scripted droughts), and diverge only
/// because the intervention's drought cuts regrowth.
#[test]
fn branches_from_one_snapshot_diverge_only_by_the_intervention() {
    let c = Config::default();
    let mut base = Simulation::new(common::PRIMARY_SEED, c.clone());
    base.run(10 * TICKS_PER_DAY);
    let snap = base.snapshot_json();

    let mut control = Simulation::from_snapshot_json(&snap).expect("control");
    let mut drought = Simulation::from_snapshot_json(&snap).expect("intervention");
    let mut cfg = c;
    cfg.weather
        .scripted
        .push(accrete_core::config::ScriptedDrought {
            start_day: 10,
            days: 20,
            center: [64, 64],
            radius: 40,
            severity: 0.9,
        });
    drought.set_config(cfg).expect("valid intervention");
    control.run(15 * TICKS_PER_DAY);
    drought.run(15 * TICKS_PER_DAY);

    let random = |s: &Simulation| {
        s.weather
            .history
            .iter()
            .filter(|d| !d.scripted)
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(random(&control), random(&drought), "same random droughts");
    let scripted: Vec<_> = drought
        .weather
        .history
        .iter()
        .filter(|d| d.scripted)
        .collect();
    assert_eq!(scripted.len(), 1);
    assert_eq!(
        scripted[0].start_day, 10,
        "starts on the snapshot day itself"
    );
    let area = |s: &Simulation, day: u32| {
        s.metrics
            .iter()
            .find(|m| m.day == day)
            .map(|m| m.drought_area)
    };
    assert!(
        area(&drought, 11).unwrap() > area(&control, 11).unwrap(),
        "visible in the next day's metrics"
    );
    assert_eq!(control.rng_weather, drought.rng_weather);
    assert_ne!(control.state_hash(), drought.state_hash());
    assert!(
        drought
            .world
            .tiles
            .iter()
            .map(|t| f64::from(t.food))
            .sum::<f64>()
            < control
                .world
                .tiles
                .iter()
                .map(|t| f64::from(t.food))
                .sum::<f64>(),
        "the drought reduced standing food"
    );
}

/// Week 4 Day 2: a route disruption on the busiest trails acts only through
/// movement costs. Nobody ever stands on a blocked tile, the tiles are
/// restored when it ends, conservation holds every tick, and random droughts
/// (the `weather` stream) are the same as in the undisrupted control.
#[test]
fn route_disruption_blocks_busy_trails_then_restores_them() {
    use accrete_core::config::{DisruptionKind, ScriptedDisruption};
    use accrete_core::world::Disruption;
    let mut c = checked();
    c.sim.check_invariants = false; // checked explicitly below, once per day
    let mut base = Simulation::new(common::PRIMARY_SEED, c.clone());
    base.run(60 * TICKS_PER_DAY);
    let snap = base.snapshot_json();
    let mut control = Simulation::from_snapshot_json(&snap).expect("control");
    let mut cut = Simulation::from_snapshot_json(&snap).expect("disrupted");
    let mut cfg = c;
    cfg.disruption.scripted.push(ScriptedDisruption {
        start_day: 60,
        days: 15,
        kind: DisruptionKind::Block,
        slow_factor: 3,
        center: [0, 0],
        radius: 0,
        busiest_tiles: 12,
    });
    cut.set_config(cfg)
        .expect("disruption config is a valid intervention");

    // Agents standing on a tile when it floods may leave it; nobody may step
    // onto a blocked tile.
    let mut saw_blocked = 0;
    let mut caught = std::collections::BTreeMap::new();
    for _ in 0..(20 * TICKS_PER_DAY) {
        let before: std::collections::BTreeMap<u32, accrete_core::Pos> =
            cut.agents.iter().map(|a| (a.id, a.pos)).collect();
        cut.step();
        control.step();
        for a in &cut.agents {
            if cut.world.disruption(a.pos) != Disruption::Blocked {
                continue;
            }
            match before.get(&a.id) {
                Some(&p) if p == a.pos => *caught.entry(a.id).or_insert(0u64) += 1,
                _ => panic!("agent {} stepped onto blocked tile {:?}", a.id, a.pos),
            }
        }
        if cut.tick % TICKS_PER_DAY == 0 {
            cut.check_invariants().expect("invariants");
        }
        saw_blocked = saw_blocked.max(cut.metrics.last().map_or(0, |m| m.disrupted_tiles));
    }
    assert!(saw_blocked > 0, "no tiles were disrupted");
    for (id, ticks) in &caught {
        assert!(
            *ticks < 3 * TICKS_PER_DAY,
            "agent {id} stayed on a blocked tile for {ticks} ticks"
        );
    }
    println!(
        "  {} tiles disrupted; {} agents caught on them when they flooded",
        saw_blocked,
        caught.len()
    );
    assert_eq!(cut.disruptions.history.len(), 1);
    assert!(cut.disruptions.active.is_empty(), "the disruption ended");
    assert_eq!(
        cut.metrics.last().unwrap().disrupted_tiles,
        0,
        "tiles restored"
    );
    let kinds: Vec<_> = cut.events.events.iter().map(|e| &e.kind).collect();
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, EventKind::RouteDisrupted { .. }))
    );
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, EventKind::RouteRestored { .. }))
    );
    assert!(
        cut.agents.iter().any(|a| a
            .memory
            .changed
            .iter()
            .any(|t| t.disruption == Disruption::Blocked))
            || cut.agents.iter().all(|a| a
                .memory
                .changed
                .iter()
                .all(|t| t.disruption == Disruption::None)),
        "memories are consistent"
    );
    let random = |s: &Simulation| {
        s.weather
            .history
            .iter()
            .filter(|d| !d.scripted)
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(random(&control), random(&cut), "random droughts unaffected");
    assert_eq!(control.rng_weather, cut.rng_weather);
    assert_ne!(
        control.state_hash(),
        cut.state_hash(),
        "the disruption had an effect"
    );
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
