//! Headless Accrete runner.
//!
//! ```text
//! accrete world  --seed 1                       # stats, validation, hash
//! accrete render --seed 1 --overlay biome -o results/biome.png
//! accrete run --seed 1 --days 100 --save results/seed1_day100.snap.json
//! accrete run --load results/seed1_day100.snap.json --days 200
//! accrete branch --load results/seed1_day100.snap.json --days 200 \
//!     --drought 100:200:64:64:20:0.6 --label-b drought
//! accrete branch --load results/seed1_day100.snap.json --days 60 \
//!     --disrupt 100:20:block:busiest:12 --label-b cut
//! accrete experiments --seed 1 --days 100 -o results/emergence
//! ```

mod branch;
mod experiments;
mod image;
mod manifest;
mod parallel;
mod run;
mod sweep;
mod trace;

use std::path::PathBuf;
use std::time::Instant;

use accrete_core::seed::{parse_seed, parse_seed_list};
use accrete_core::world::stats::world_stats;
use accrete_core::world::validate::validate_world;
use accrete_core::{Config, generate_world, hash};
use accrete_viz::Overlay;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "accrete",
    about = "Accrete: an emergent civilization simulator (headless)"
)]
struct Cli {
    /// Configuration file (TOML). Defaults to built-in defaults.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Generate a world and print statistics, validation results, and its hash.
    World {
        /// One or more seeds (comma separated; decimal, 0x-hex, text, or a
        /// range like 1-10).
        #[arg(long, default_value = "1", value_delimiter = ',')]
        seed: Vec<String>,
        /// Print full statistics as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Run the simulation headlessly and export metrics.
    Run {
        /// One or more seeds (comma separated, ranges like 1-10; default 1).
        /// Not allowed with
        /// `--load`, which resumes the snapshot's own world.
        #[arg(long, value_delimiter = ',')]
        seed: Vec<String>,
        /// Number of days to simulate (from the snapshot's day with `--load`).
        #[arg(long, default_value_t = 100)]
        days: u32,
        /// Resume from a snapshot instead of generating a world. `--set` and
        /// `--config` then change the snapshot's config (an intervention).
        #[arg(long)]
        load: Option<PathBuf>,
        /// Write a snapshot of the final state to this path (one seed only).
        #[arg(long)]
        save: Option<PathBuf>,
        /// Output directory for metrics and images.
        #[arg(short, long, default_value = "results")]
        out: PathBuf,
        /// Print a summary line every N days (0 = only the end).
        #[arg(long, default_value_t = 10)]
        every: u32,
        /// Render a map with agents and homes every N days (0 = only the end).
        #[arg(long, default_value_t = 0)]
        snap_every: u32,
        /// Check resource conservation and bounds every tick (slow).
        #[arg(long)]
        check: bool,
        /// Config overrides, e.g. `--set decision.time_cost=0.03` (repeatable).
        #[arg(long = "set")]
        sets: Vec<String>,
        /// Run label used in output file names.
        #[arg(long, default_value = "run")]
        label: String,
        /// Days to capture for a side-by-side timelapse (terrain + depletion),
        /// e.g. `--timelapse 0,75,150,225,300`.
        #[arg(long, value_delimiter = ',')]
        timelapse: Vec<u32>,
    },
    /// Week 3 Day 5 emergence suite: paired ablations (trade, trails, selfish).
    Experiments {
        /// One or more seeds (comma separated, ranges like 1-10).
        #[arg(long, default_value = "1", value_delimiter = ',')]
        seed: Vec<String>,
        /// Days to simulate for each run.
        #[arg(long, default_value_t = 100)]
        days: u32,
        #[arg(short, long, default_value = "results/emergence")]
        out: PathBuf,
        #[arg(long, default_value_t = 25)]
        every: u32,
        /// Check conservation and bounds every tick (slow).
        #[arg(long)]
        check: bool,
        /// Subset of experiments: trade, trails, selfish (comma separated).
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Seeds to run in parallel (default 1). Each seed's runs stay
        /// sequential; outputs are identical whatever the value.
        #[arg(long, default_value_t = 1)]
        jobs: usize,
        /// Config overrides applied to the shared base (repeatable).
        #[arg(long = "set")]
        sets: Vec<String>,
    },
    /// Sweep config parameters over seeds: a grid (`--param key=a,b,c`) or a
    /// Latin hypercube (`--lhs N` with `--param key=lo..hi`).
    Sweep {
        /// A parameter to vary (repeatable): `section.key=a,b,c` or
        /// `section.key=lo..hi` (ranges need --lhs).
        #[arg(long = "param", required = true)]
        params: Vec<String>,
        /// Latin-hypercube points (all parameters must be ranges).
        #[arg(long)]
        lhs: Option<usize>,
        /// Seed for the Latin-hypercube design (the `Design` RNG stream).
        #[arg(long, default_value_t = 1)]
        design_seed: u64,
        /// Seeds per point (comma separated, ranges like 1-10).
        #[arg(long, default_value = "1", value_delimiter = ',')]
        seed: Vec<String>,
        #[arg(long, default_value_t = 100)]
        days: u32,
        #[arg(short, long, default_value = "results/sweep")]
        out: PathBuf,
        /// Runs in parallel; outputs are identical whatever the value.
        #[arg(long, default_value_t = 1)]
        jobs: usize,
        /// Check conservation and bounds every tick (slow).
        #[arg(long)]
        check: bool,
        /// Config overrides applied to the base before the sweep (repeatable).
        #[arg(long = "set")]
        sets: Vec<String>,
    },
    /// Re-run a run manifest and check it ends on the recorded state hash.
    Reproduce {
        /// A `*_manifest.json` written by run, branch or experiments.
        manifest: PathBuf,
    },
    /// Load a snapshot and run a control and an intervention side by side.
    Branch {
        /// Snapshot to branch from.
        #[arg(long)]
        load: PathBuf,
        /// Days to run each branch.
        #[arg(long, default_value_t = 200)]
        days: u32,
        #[arg(short, long, default_value = "results")]
        out: PathBuf,
        #[arg(long, default_value_t = 50)]
        every: u32,
        /// Scripted drought for the intervention:
        /// `DAY:DAYS:X:Y:RADIUS:SEVERITY` (repeatable).
        #[arg(long = "drought")]
        droughts: Vec<String>,
        /// Route disruption for the intervention:
        /// `DAY:DAYS:block|slowF:X:Y:RADIUS` or `DAY:DAYS:block|slowF:busiest:N`
        /// (repeatable).
        #[arg(long = "disrupt")]
        disruptions: Vec<String>,
        /// Config overrides for the intervention (repeatable).
        #[arg(long = "set")]
        sets: Vec<String>,
        /// Whole config file for the intervention (before `--set`/`--drought`).
        #[arg(long)]
        config_b: Option<PathBuf>,
        #[arg(long, default_value = "control")]
        label_a: String,
        #[arg(long, default_value = "intervention")]
        label_b: String,
    },
    /// Run to a day, then trace one or more agents tick by tick.
    Trace {
        #[arg(long, default_value = "1")]
        seed: String,
        /// Day to start tracing.
        #[arg(long, default_value_t = 10)]
        day: u32,
        /// Ticks to trace.
        #[arg(long, default_value_t = 48)]
        ticks: u32,
        /// Agent ids (comma separated). Default: agents flagged by detectors.
        #[arg(long, value_delimiter = ',')]
        agent: Vec<u32>,
        #[arg(long = "set")]
        sets: Vec<String>,
        /// Print the full option list for each tick.
        #[arg(long)]
        options: bool,
    },
    /// Print the distribution of agents' settlement perceptions on a day.
    Diag {
        #[arg(long, default_value = "1")]
        seed: String,
        #[arg(long, default_value_t = 100)]
        day: u32,
        #[arg(long = "set")]
        sets: Vec<String>,
    },
    /// Print the effective configuration as TOML (defaults merged with --config).
    Config,
    /// Render world overlays to PNG images.
    Render {
        /// One or more seeds (comma separated, ranges like 1-10). With several seeds and
        /// `--montage`, the images are tiled into one picture per overlay.
        #[arg(long, default_value = "1", value_delimiter = ',')]
        seed: Vec<String>,
        #[arg(long)]
        montage: bool,
        /// Overlay names (comma separated), or "all".
        #[arg(long, default_value = "biome", value_delimiter = ',')]
        overlay: Vec<String>,
        /// Output directory.
        #[arg(short, long, default_value = "results")]
        out: PathBuf,
        /// Pixels per tile.
        #[arg(long, default_value_t = 4)]
        scale: u32,
        /// Mark spawn positions.
        #[arg(long)]
        spawns: bool,
    },
}

/// Print an error and exit with status 2 (bad arguments).
fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2)
}

fn load_config(path: &Option<PathBuf>) -> Config {
    match path {
        None => Config::default(),
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("reading {}: {e}", p.display()));
            Config::from_toml_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", p.display()))
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let config = load_config(&cli.config);
    match cli.cmd {
        Cmd::Run {
            seed,
            days,
            load,
            save,
            out,
            every,
            snap_every,
            check,
            sets,
            label,
            timelapse,
        } => {
            run::check_label(&label).unwrap_or_else(|e| fail(&e));
            if let Some(path) = &save {
                run::check_writable(path).unwrap_or_else(|e| fail(&e));
                if let Some(l) = &load {
                    let same = match (std::fs::canonicalize(l), std::fs::canonicalize(path)) {
                        (Ok(a), Ok(b)) => a == b,
                        _ => l == path,
                    };
                    if same {
                        fail(
                            "--save must differ from --load: overwriting the start snapshot would make the run unreproducible",
                        );
                    }
                }
            }
            let opts = run::RunOpts {
                days,
                out: &out,
                every,
                snap_every,
                label: &label,
                timelapse: &timelapse,
                save: save.as_deref(),
                stop_on_extinction: true,
                loaded_from: load.as_deref(),
            };
            if let Some(path) = &load {
                if !seed.is_empty() {
                    fail("--seed cannot be combined with --load: the snapshot fixes the world");
                }
                // Start from the snapshot's own config; --config replaces it
                // and --set adjusts it (either one makes an intervention).
                let snap = run::load(path, None, &label);
                let mut c = if cli.config.is_some() {
                    config
                } else {
                    snap.config.clone()
                };
                for kv in &sets {
                    c = run::apply_override(&c, kv);
                }
                c.sim.check_invariants |= check;
                let mut sim = snap;
                if c != sim.config {
                    if sim.config.differs_in_results(&c) {
                        println!("[{label}] config changed from the snapshot's (intervention)");
                    }
                    sim.set_config(c).unwrap_or_else(|e| fail(&e));
                }
                run::drive(sim, &opts);
            } else {
                let mut config = config;
                for kv in &sets {
                    config = run::apply_override(&config, kv);
                }
                config.sim.check_invariants |= check;
                let seeds = if seed.is_empty() {
                    vec![1]
                } else {
                    parse_seed_list(&seed).unwrap_or_else(|e| fail(&e))
                };
                if save.is_some() && seeds.len() > 1 {
                    fail("--save needs a single seed");
                }
                for s in seeds {
                    run::run(s, &config, &opts);
                }
            }
        }
        Cmd::Experiments {
            seed,
            days,
            out,
            every,
            check,
            only,
            sets,
            jobs,
        } => {
            let mut config = config;
            for kv in &sets {
                config = run::apply_override(&config, kv);
            }
            let seeds = parse_seed_list(&seed).unwrap_or_else(|e| fail(&e));
            if jobs == 0 {
                fail("--jobs must be at least 1");
            }
            experiments::run_suite(
                &config,
                experiments::ExperimentsArgs {
                    seeds,
                    days,
                    out,
                    every,
                    check,
                    only,
                    jobs,
                },
            );
        }
        Cmd::Sweep {
            params,
            lhs,
            design_seed,
            seed,
            days,
            out,
            jobs,
            check,
            sets,
        } => {
            let mut config = config;
            for kv in &sets {
                config = run::try_apply_override(&config, kv).unwrap_or_else(|e| fail(&e));
            }
            let params = params
                .iter()
                .map(|p| sweep::parse_param(p))
                .collect::<Result<Vec<_>, _>>()
                .unwrap_or_else(|e| fail(&e));
            if jobs == 0 {
                fail("--jobs must be at least 1");
            }
            sweep::run_sweep(
                &config,
                sweep::SweepArgs {
                    sets,
                    params,
                    lhs,
                    design_seed,
                    seeds: parse_seed_list(&seed).unwrap_or_else(|e| fail(&e)),
                    days,
                    out,
                    jobs,
                    check,
                },
            );
        }
        Cmd::Reproduce { manifest } => {
            let m = manifest::Manifest::read(&manifest).unwrap_or_else(|e| fail(&e));
            let base = manifest.parent().unwrap_or(std::path::Path::new("."));
            println!(
                "reproducing {} (seed {}, {} ticks from tick {}, recorded by accrete {} at {})",
                manifest.display(),
                m.seed,
                m.ticks,
                m.start_tick,
                m.build.accrete_version,
                m.build.git_commit
            );
            let r = manifest::reproduce(&m, base).unwrap_or_else(|e| fail(&e));
            for d in &r.build_differences {
                println!("  note: {d}");
            }
            if !r.start_matches {
                eprintln!("  NOT reproduced: the start state differs from the recorded one");
                std::process::exit(1);
            }
            if r.matches {
                println!("  reproduced: final state hash {} matches", r.final_hash);
            } else {
                eprintln!(
                    "  NOT reproduced: final state hash {} (manifest says {})",
                    r.final_hash, m.final_state_hash
                );
                std::process::exit(1);
            }
        }
        Cmd::Branch {
            load,
            days,
            out,
            every,
            droughts,
            disruptions,
            sets,
            config_b,
            label_a,
            label_b,
        } => {
            branch::branch(branch::BranchArgs {
                load,
                days,
                out,
                every,
                config_b: config_b.as_ref().map(|p| load_config(&Some(p.clone()))),
                sets_b: sets,
                droughts_b: droughts,
                disruptions_b: disruptions,
                label_a,
                label_b,
            });
        }
        Cmd::World { seed, json } => {
            for seed in parse_seed_list(&seed).unwrap_or_else(|e| fail(&e)) {
                let t0 = Instant::now();
                let world = generate_world(seed, &config);
                let gen_ms = t0.elapsed().as_secs_f64() * 1000.0;
                let errs = validate_world(&world, &config);
                let stats = world_stats(&world, config.resources.food_regrowth);
                if json {
                    println!("{}", serde_json::to_string_pretty(&stats).unwrap());
                } else {
                    println!(
                        "seed {seed} hash {} gen {gen_ms:.0} ms | land {:.1}% rivers {} ({:.1}% of land) | fert {:.3} | food {:.0} (regrow<= {:.0}/day) wood {:.0} stone {:.0} on {} tiles | vis {:.1} tiles",
                        hash::hex(world.state_hash()),
                        stats.land_fraction * 100.0,
                        stats.river_tiles,
                        stats.river_fraction_of_land * 100.0,
                        stats.mean_land_fertility,
                        stats.total_food,
                        stats.max_food_regrowth_per_day,
                        stats.total_wood,
                        stats.total_stone,
                        stats.stone_tiles,
                        stats.mean_visible_tiles_land,
                    );
                    for (b, f) in &stats.biome_fraction {
                        print!("  {b} {:.1}%", f * 100.0);
                    }
                    println!();
                    for (b, v) in &stats.mean_visible_tiles_by_biome {
                        print!("  vis[{b}] {v:.1}");
                    }
                    println!();
                }
                if errs.is_empty() {
                    println!("  validation: OK");
                } else {
                    println!("  validation: {} problems", errs.len());
                    for e in errs.iter().take(20) {
                        println!("    {e}");
                    }
                    std::process::exit(1);
                }
            }
        }
        Cmd::Trace {
            seed,
            day,
            ticks,
            agent,
            sets,
            options,
        } => {
            let mut config = config;
            for kv in &sets {
                config = run::apply_override(&config, kv);
            }
            trace::trace(parse_seed(&seed), &config, day, ticks, &agent, options);
        }
        Cmd::Config => print!("{}", config.to_toml_string()),
        Cmd::Diag { seed, day, sets } => {
            let mut config = config;
            for kv in &sets {
                config = run::apply_override(&config, kv);
            }
            trace::diag(parse_seed(&seed), &config, day);
        }
        Cmd::Render {
            seed,
            overlay,
            out,
            scale,
            spawns,
            montage,
        } => {
            std::fs::create_dir_all(&out).expect("create output dir");
            let worlds: Vec<_> = parse_seed_list(&seed)
                .unwrap_or_else(|e| fail(&e))
                .iter()
                .map(|&s| generate_world(s, &config))
                .collect();
            let overlays: Vec<Overlay> = if overlay.iter().any(|o| o == "all") {
                Overlay::ALL.to_vec()
            } else {
                overlay
                    .iter()
                    .map(|o| Overlay::from_name(o).unwrap_or_else(|| panic!("unknown overlay {o}")))
                    .collect()
            };
            for ov in overlays {
                let canvases: Vec<image::Canvas> = worlds
                    .iter()
                    .map(|world| {
                        let mut img = image::Canvas::from_world(world, &config, ov, scale);
                        if spawns {
                            for &s in &world.spawns {
                                img.dot(s, [255, 40, 40], scale.max(2) / 2 + 1);
                            }
                        }
                        img
                    })
                    .collect();
                if montage && canvases.len() > 1 {
                    let path = out.join(format!("montage_{}.png", ov.name()));
                    image::Canvas::montage(&canvases).save(&path);
                    println!("wrote {}", path.display());
                } else {
                    for (world, img) in worlds.iter().zip(&canvases) {
                        let path = out.join(format!("seed{}_{}.png", world.seed, ov.name()));
                        img.save(&path);
                        println!("wrote {}", path.display());
                    }
                }
            }
        }
    }
}
