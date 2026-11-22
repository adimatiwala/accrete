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

/// Options shared by `run` and `branch`.
pub struct RunOpts<'a> {
    /// Days to simulate from the starting state.
    pub days: u32,
    pub out: &'a Path,
    pub every: u32,
    pub snap_every: u32,
    /// Write a state snapshot (`*.snap.json`) every N days (0 = none).
    /// Distinct from `snap_every`, which renders PNG maps.
    pub save_every: u32,
    pub label: &'a str,
    /// Absolute days to capture for the timelapse strip.
    pub timelapse: &'a [u32],
    /// Write a snapshot of the final state here.
    pub save: Option<&'a Path>,
    /// Stop early when the population dies out. Branch comparisons keep
    /// going, so both series cover the same days (population 0 after
    /// extinction; the world keeps changing).
    pub stop_on_extinction: bool,
    /// The snapshot this run was loaded from, if any (for its manifest).
    pub loaded_from: Option<&'a Path>,
}

/// A run label is used in output file names: it must be a plain name.
pub fn check_label(label: &str) -> Result<(), String> {
    if label.is_empty()
        || label.contains(['/', '\\', ':', ','])
        || label == "."
        || label == ".."
        || label.chars().any(char::is_control)
    {
        return Err(format!(
            "label {label:?} must be a plain file-name part (no path separators or commas)"
        ));
    }
    Ok(())
}

/// Fail before a long run if a snapshot cannot be written to `path`.
pub fn check_writable(path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let existed = path.exists();
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    if !existed {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

/// `accrete run` from a fresh world.
pub fn run(seed: u64, config: &Config, opts: &RunOpts) -> Simulation {
    std::fs::create_dir_all(opts.out).expect("create output dir");
    let t0 = Instant::now();
    let sim = Simulation::new(seed, config.clone());
    let gen_ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!(
        "[{}] seed {seed}: world {} generated in {gen_ms:.0} ms, {} agents",
        opts.label,
        hash::hex(sim.world.state_hash()),
        sim.population()
    );
    drive(sim, opts)
}

/// Load a snapshot, optionally replacing its config (an intervention), and
/// print where it resumes.
pub fn load(path: &Path, config: Option<Config>, label: &str) -> Simulation {
    let mut sim = Simulation::load_snapshot(path).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1)
    });
    println!(
        "[{label}] loaded {} (seed {}): day {}, tick {}, {} agents, hash {}",
        path.display(),
        sim.world.seed,
        sim.day(),
        sim.tick,
        sim.population(),
        hash::hex(sim.state_hash())
    );
    if let Some(c) = config {
        if c != sim.config {
            println!("[{label}] config changed from the snapshot's (intervention)");
            sim.set_config(c).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                std::process::exit(2)
            });
        }
    }
    sim
}

/// Simulate `opts.days` days from the current state, print progress, and
/// export metrics, deaths, maps, and optionally a snapshot.
pub fn drive(mut sim: Simulation, opts: &RunOpts) -> Simulation {
    std::fs::create_dir_all(opts.out).expect("create output dir");
    let (label, seed, timelapse, out) = (opts.label, sim.world.seed, opts.timelapse, opts.out);
    let t1 = Instant::now();
    let tick0 = sim.tick;
    let start_state_hash = hash::hex(sim.state_hash());
    let start = match opts.loaded_from {
        Some(p) => crate::manifest::Start::Snapshot {
            // Absolute, so the manifest works from any directory; and relative
            // to the output folder, so a moved results folder still works.
            path: std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()),
            relative_path: crate::manifest::relative_to(opts.out, p),
            // Panic (not process::exit) so a failure under --jobs is caught
            // by parallel::map_ordered instead of killing sibling seeds.
            file_hash: crate::manifest::file_hash(p).unwrap_or_else(|e| panic!("{e}")),
        },
        None => crate::manifest::Start::Fresh { seed },
    };
    // Refuse a conflicting prior manifest *before* --save-every can clobber
    // mid-run snapshots. Start/config must match; ticks must match the planned
    // length unless this run may stop early on extinction (then the end-of-run
    // write still enforces an exact match).
    let planned_ticks = u64::from(opts.days) * TICKS_PER_DAY;
    let manifest_path = crate::manifest::Manifest::path_for(out, label, seed);
    if let Ok(old) = crate::manifest::Manifest::read(&manifest_path) {
        let same_start = old.start == start
            && old.start_tick == tick0
            && old.config.digest() == sim.config.digest();
        let ticks_ok =
            old.ticks == planned_ticks || (opts.stop_on_extinction && old.ticks <= planned_ticks);
        if !same_start || !ticks_ok {
            panic!(
                "{} already records a different run; use a different --label or -o",
                manifest_path.display()
            );
        }
    }
    let mut frames: Vec<(Canvas, Canvas)> = Vec::new();
    let mut extinct = sim.population() == 0;
    let mut snap_pngs: Vec<String> = Vec::new();
    let mut state_snaps: Vec<String> = Vec::new();
    let mut frames_written: Vec<String> = Vec::new();
    if timelapse.contains(&sim.day()) {
        frames.push((
            frame(&sim, Overlay::Biome, 3),
            frame(&sim, Overlay::Depletion, 3),
        ));
    }
    for _ in 0..opts.days {
        sim.run(TICKS_PER_DAY);
        let d = sim.day();
        let m = sim.metrics.last().expect("daily metrics").clone();
        if opts.every > 0 && d % opts.every == 0 {
            print_line(&m);
        }
        if timelapse.contains(&d) {
            frames.push((
                frame(&sim, Overlay::Biome, 3),
                frame(&sim, Overlay::Depletion, 3),
            ));
        }
        if opts.snap_every > 0 && d % opts.snap_every == 0 {
            let name = format!("{label}_seed{seed}_day{d:03}");
            snapshot(&sim, opts.out, &name);
            snap_pngs.push(format!("{name}.png"));
        }
        if opts.save_every > 0 && d % opts.save_every == 0 {
            let name = format!("{label}_seed{seed}_day{d:03}.snap.json");
            let path = out.join(&name);
            sim.save_snapshot(&path)
                .unwrap_or_else(|e| panic!("saving periodic snapshot: {e}"));
            state_snaps.push(name);
        }
        if sim.population() == 0 && !extinct {
            extinct = true;
            println!("  population extinct on day {d}");
            if opts.stop_on_extinction {
                break;
            }
        }
    }
    let secs = t1.elapsed().as_secs_f64();
    let ticks = sim.tick - tick0;
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
        frames_written.push(format!("{base}_timelapse_biome.png"));
        frames_written.push(format!("{base}_timelapse_depletion.png"));
        println!(
            "  timelapse days {:?}: {} and {}",
            timelapse,
            p1.display(),
            p2.display()
        );
    }
    if let Some(path) = opts.save {
        sim.save_snapshot(path)
            .unwrap_or_else(|e| panic!("saving snapshot: {e}"));
        println!(
            "  snapshot (day {}) written to {}",
            sim.day(),
            path.display()
        );
    }
    // Paths relative to the manifest's directory (the output folder).
    let mut outputs = vec![
        format!("{base}_metrics.csv"),
        format!("{base}_deaths.csv"),
        format!("{base}_final.png"),
    ];
    outputs.extend(snap_pngs);
    outputs.extend(state_snaps);
    if !frames_written.is_empty() {
        outputs.extend(frames_written);
    }
    if let Some(p) = opts.save {
        outputs.push(
            crate::manifest::relative_to(out, p)
                .unwrap_or_else(|| p.to_path_buf())
                .display()
                .to_string(),
        );
    }
    let manifest = crate::manifest::Manifest {
        format: crate::manifest::MANIFEST_FORMAT,
        build: crate::manifest::Build::current(),
        command: crate::manifest::recorded_command(),
        label: label.to_string(),
        seed,
        start,
        start_tick: tick0,
        start_state_hash,
        ticks: sim.tick - tick0,
        final_tick: sim.tick,
        final_state_hash: hash::hex(sim.state_hash()),
        config: sim.config.clone(),
        outputs,
    };
    match manifest.write(out) {
        Ok(p) => {
            println!("  manifest written to {}", p.display());
            crate::index::upsert_after_run(out, &p);
        }
        Err(e) => panic!("{e}"),
    }
    sim
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
    for d in &sim.disruptions.active {
        for &t in &d.tiles {
            img.dot(sim.world.dims.pos(t), [230, 30, 30], scale);
        }
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

/// Apply a `section.key=value` override by round-tripping the config through
/// TOML. On a bad override (for `--set`) it prints the reason and exits
/// with status 2, like every other command-line error.
pub fn apply_override(config: &Config, kv: &str) -> Config {
    try_apply_override(config, kv).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(2)
    })
}

/// The kind of value a config key holds, for parsing and sweeping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    Float,
    Integer,
    Boolean,
    Other,
}

/// What kind of value `section.key` holds in `config`.
pub fn key_kind(config: &Config, path: &str) -> Result<KeyKind, String> {
    let (section, key) = path
        .split_once('.')
        .ok_or_else(|| format!("config key must be section.key, got {path}"))?;
    let doc: toml::Table = toml::from_str(&config.to_toml_string()).expect("config toml");
    let old = doc
        .get(section)
        .and_then(|t| t.as_table())
        .ok_or_else(|| format!("unknown config section {section}"))?
        .get(key)
        .ok_or_else(|| format!("unknown key {section}.{key}"))?;
    Ok(match old {
        toml::Value::Float(_) => KeyKind::Float,
        toml::Value::Integer(_) => KeyKind::Integer,
        toml::Value::Boolean(_) => KeyKind::Boolean,
        _ => KeyKind::Other,
    })
}

/// Like `apply_override`, but returns an error instead of panicking.
pub fn try_apply_override(config: &Config, kv: &str) -> Result<Config, String> {
    let (path, value) = kv
        .split_once('=')
        .ok_or_else(|| format!("override expects key=value, got {kv}"))?;
    let (section, key) = path
        .split_once('.')
        .ok_or_else(|| format!("config key must be section.key, got {path}"))?;
    let value = value.trim();
    let mut doc: toml::Table = toml::from_str(&config.to_toml_string()).expect("config toml");
    let table = doc
        .get_mut(section)
        .and_then(|t| t.as_table_mut())
        .ok_or_else(|| format!("unknown config section {section}"))?;
    let old = table
        .get(key)
        .ok_or_else(|| format!("unknown key {section}.{key}"))?;
    let bad = |what: &str| format!("{section}.{key} expects {what}, got {value:?}");
    let new = match old {
        toml::Value::Float(_) => toml::Value::Float(value.parse().map_err(|_| bad("a number"))?),
        toml::Value::Integer(_) => {
            toml::Value::Integer(value.parse().map_err(|_| bad("an integer"))?)
        }
        toml::Value::Boolean(_) => {
            toml::Value::Boolean(value.parse().map_err(|_| bad("true or false"))?)
        }
        _ => toml::Value::String(value.to_string()),
    };
    table.insert(key.to_string(), new);
    Config::from_toml_str(&toml::to_string(&doc).expect("toml"))
        .map_err(|e| format!("override {kv}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_must_be_plain_names() {
        for ok in ["run", "drought-60", "a_b.2"] {
            assert!(check_label(ok).is_ok(), "{ok}");
        }
        for bad in ["", "a/b", "a\\b", "c:x", "a,b", ".", "..", "tab\there"] {
            assert!(check_label(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn unwritable_save_paths_fail_up_front() {
        let dir = std::env::temp_dir().join(format!("accrete-w-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("plain-file");
        std::fs::write(&file, b"").unwrap();
        assert!(check_writable(&file.join("x.snap.json")).is_err());
        let p = dir.join("sub/ok.snap.json");
        assert!(check_writable(&p).is_ok());
        assert!(!p.exists(), "the probe leaves no file behind");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
