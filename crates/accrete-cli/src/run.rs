//! `accrete run`: headless simulation with metrics export and map snapshots.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use accrete_core::agents::agent::GoalKind;
use accrete_core::config::TICKS_PER_DAY;
use accrete_core::simulation::metrics::DailyMetrics;
use accrete_core::{Config, Simulation, hash};
use accrete_viz::{Overlay, goal_color};

use crate::image::Canvas;

#[allow(clippy::too_many_arguments)] // a flat CLI entry point
pub fn run(
    seed: u64,
    config: &Config,
    days: u32,
    out: &Path,
    every: u32,
    snap_every: u32,
    label: &str,
    timelapse: &[u32],
) {
    std::fs::create_dir_all(out).expect("create output dir");
    let t0 = Instant::now();
    let mut sim = Simulation::new(seed, config.clone());
    let gen_ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!(
        "[{label}] seed {seed}: world {} generated in {gen_ms:.0} ms, {} agents",
        hash::hex(sim.world.state_hash()),
        sim.population()
    );
    let t1 = Instant::now();
    let mut frames: Vec<(Canvas, Canvas)> = Vec::new();
    if timelapse.contains(&0) {
        frames.push((
            frame(&sim, Overlay::Biome, 3),
            frame(&sim, Overlay::Depletion, 3),
        ));
    }
    for d in 1..=days {
        sim.run(TICKS_PER_DAY);
        let m = sim.metrics.last().expect("daily metrics").clone();
        if every > 0 && d % every == 0 {
            print_line(&m);
        }
        if timelapse.contains(&d) {
            frames.push((
                frame(&sim, Overlay::Biome, 3),
                frame(&sim, Overlay::Depletion, 3),
            ));
        }
        if snap_every > 0 && d % snap_every == 0 {
            snapshot(&sim, out, &format!("{label}_seed{seed}_day{d:03}"));
        }
        if sim.population() == 0 {
            println!("  population extinct on day {d}");
            break;
        }
    }
    let secs = t1.elapsed().as_secs_f64();
    let ticks = sim.tick;
    println!(
        "  ran {ticks} ticks in {secs:.2} s ({:.0} ticks/s), final hash {}",
        ticks as f64 / secs.max(1e-9),
        hash::hex(sim.state_hash())
    );
    if let Some(m) = sim.metrics.last() {
        print_line(m);
    }
    summary(&sim);

    // Exports.
    let base = format!("{label}_seed{seed}");
    let mut csv = DailyMetrics::csv_header();
    csv.push('\n');
    for m in &sim.metrics {
        csv.push_str(&m.csv_row());
        csv.push('\n');
    }
    std::fs::write(out.join(format!("{base}_metrics.csv")), csv).expect("write metrics");
    let mut deaths = String::from(
        "agent,tick,day,x,y,cause,age_days,age_decline,frailty,days_deprived,children,starved_beside_food,explanation\n",
    );
    for d in &sim.deaths {
        let _ = writeln!(
            deaths,
            "{},{},{},{},{},{},{:.1},{:.3},{:.3},{:.1},{},{},\"{}\"",
            d.agent,
            d.tick,
            d.tick / TICKS_PER_DAY,
            d.pos.x,
            d.pos.y,
            d.cause.name(),
            d.age_days,
            d.age_decline,
            d.frailty,
            d.days_deprived,
            d.children,
            d.starved_beside_food,
            d.explanation
        );
    }
    std::fs::write(out.join(format!("{base}_deaths.csv")), deaths).expect("write deaths");
    snapshot(&sim, out, &format!("{base}_final"));
    if !frames.is_empty() {
        let (biome, depl): (Vec<Canvas>, Vec<Canvas>) = frames.into_iter().unzip();
        let p1 = out.join(format!("{base}_timelapse_biome.png"));
        let p2 = out.join(format!("{base}_timelapse_depletion.png"));
        Canvas::strip(&biome).save(&p1);
        Canvas::strip(&depl).save(&p2);
        println!(
            "  timelapse days {:?}: {} and {}",
            timelapse,
            p1.display(),
            p2.display()
        );
    }
}

fn print_line(m: &DailyMetrics) {
    let mut goals: Vec<(f32, &str)> = GoalKind::ALL
        .iter()
        .zip(&m.goal_share)
        .map(|(k, &s)| (s, k.name()))
        .collect();
    goals.sort_by(|a, b| b.0.total_cmp(&a.0));
    let top: Vec<String> = goals
        .iter()
        .take(5)
        .map(|(s, n)| format!("{n} {:.0}%", s * 100.0))
        .collect();
    println!(
        "  d{:>3} pop {:>4} (j{:>3} e{:>3}) +{:<2} -{}/{} | H {:.2} E {:.2} hp {:.2} fr {:.3} | homed {:>3.0}% homes {:>3} ({:.1}/home) chests {:>3} stored {:>5.0} | trails {:>3} roads {:>3} shel {:>2} | trades {:>3} tips {:>4} | R {:.2} depl {:.2} spec {:.2} fr {:.1}t rate {:.2} drought {:>2.0}% | set c{} h{} v{} | path o{} s{} sb{} i{} | {}",
        m.day,
        m.population,
        m.juveniles,
        m.elders,
        m.births,
        m.deaths_deprivation,
        m.deaths_old_age,
        m.mean_hunger,
        m.mean_energy,
        m.mean_health,
        m.mean_frailty,
        m.homed_fraction * 100.0,
        m.homes_occupied,
        m.mean_residents,
        m.chests,
        m.food_stored,
        m.trail_tiles,
        m.road_tiles,
        m.shelters,
        m.trades,
        m.tips_shared,
        m.clark_evans,
        m.depletion_ratio,
        m.specialization,
        m.forage_radius,
        m.mean_forage_rate,
        m.drought_area * 100.0,
        m.settlement_candidates,
        m.hamlets,
        m.villages,
        m.pathology_oscillating,
        m.pathology_stuck,
        m.pathology_starved_beside_food,
        m.pathology_idle,
        top.join(", ")
    );
}

fn summary(sim: &Simulation) {
    let Some(m) = sim.metrics.last() else { return };
    println!(
        "  totals: births {} deaths {} deprivation / {} old age, mean lifespan (born in run) {:.1} d, travel/food {:.2}",
        m.total_births,
        m.total_deaths_deprivation,
        m.total_deaths_old_age,
        m.mean_lifespan_days,
        m.travel_per_food
    );
    for s in sim.settlements.tracked.iter().take(8) {
        println!(
            "    settlement #{} {}: {} homes, {} residents, centre ({:.0},{:.0}), since day {}",
            s.id,
            s.class.name(),
            s.homes.len(),
            s.residents,
            s.centre.0,
            s.centre.1,
            s.since_day
        );
    }
}

/// Annotated map frame: terrain (or another overlay), traffic, roads,
/// droughts, settlements, homes, and agents.
pub fn frame(sim: &Simulation, overlay: Overlay, scale: u32) -> Canvas {
    let mut img = Canvas::from_world(&sim.world, &sim.config, overlay, scale);
    if overlay == Overlay::Biome {
        // Dim the terrain so people and their works stand out.
        for px in img.pixels.iter_mut() {
            *px = (*px as f32 * 0.7) as u8;
        }
        for t in 0..sim.world.dims.len() as u32 {
            let tile = &sim.world.tiles[t as usize];
            let p = sim.world.dims.pos(t);
            if tile.path_level == accrete_core::world::PathLevel::Road {
                img.dot(p, [70, 40, 20], scale);
            } else if tile.traffic > 3.0 {
                let k = (tile.traffic / 30.0).min(1.0);
                let c = [(200.0 + 55.0 * k) as u8, (170.0 + 30.0 * k) as u8, 110];
                img.dot(p, c, 1 + (k * 2.0) as u32);
            }
        }
    }
    for d in &sim.weather.active {
        img.circle(d.center, d.radius as f32, [255, 140, 0]);
    }
    for s in sim.settlements.tracked.iter() {
        let c = match s.class {
            accrete_core::simulation::settlement::SizeClass::Candidate => [180, 180, 255],
            accrete_core::simulation::settlement::SizeClass::Hamlet => [255, 80, 200],
            _ => [255, 40, 40],
        };
        let centre = accrete_core::Pos::new(s.centre.0.round() as i32, s.centre.1.round() as i32);
        img.circle(centre, 3.0 + (s.homes.len() as f32).sqrt(), c);
    }
    for h in &sim.world.homes {
        if h.is_vacant() {
            img.ring(h.pos, [90, 90, 90]);
        } else {
            let c = match (h.chests.is_empty(), h.shelter_level) {
                (_, l) if l > 0 => [120, 255, 140],
                (true, _) => [240, 240, 240],
                (false, _) => [255, 215, 0],
            };
            img.ring(h.pos, c);
        }
    }
    if overlay == Overlay::Biome {
        for a in &sim.agents {
            img.dot(a.pos, goal_color(a.goal.kind as usize), 2);
        }
    }
    img
}

/// Map with homes, roads, droughts, settlements and agents.
pub fn snapshot(sim: &Simulation, out: &Path, name: &str) {
    let path = out.join(format!("{name}.png"));
    frame(sim, Overlay::Biome, 5).save(&path);
}

/// Apply a `section.key=value` override by round-tripping the config through TOML.
pub fn apply_override(config: &Config, kv: &str) -> Config {
    let (path, value) = kv
        .split_once('=')
        .unwrap_or_else(|| panic!("--set expects key=value, got {kv}"));
    let (section, key) = path
        .split_once('.')
        .unwrap_or_else(|| panic!("--set key must be section.key, got {path}"));
    let mut doc: toml::Table = toml::from_str(&config.to_toml_string()).expect("config toml");
    let table = doc
        .get_mut(section)
        .and_then(|t| t.as_table_mut())
        .unwrap_or_else(|| panic!("unknown config section {section}"));
    let old = table
        .get(key)
        .unwrap_or_else(|| panic!("unknown key {section}.{key}"));
    let new = match old {
        toml::Value::Float(_) => toml::Value::Float(value.parse().expect("float value")),
        toml::Value::Integer(_) => toml::Value::Integer(value.parse().expect("integer value")),
        toml::Value::Boolean(_) => toml::Value::Boolean(value.parse().expect("bool value")),
        _ => toml::Value::String(value.to_string()),
    };
    table.insert(key.to_string(), new);
    Config::from_toml_str(&toml::to_string(&doc).expect("toml"))
        .unwrap_or_else(|e| panic!("override {kv}: {e}"))
}
