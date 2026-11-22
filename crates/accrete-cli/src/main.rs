//! Headless Accrete runner.
//!
//! ```text
//! accrete world  --seed 1                       # stats, validation, hash
//! accrete render --seed 1 --overlay biome -o results/biome.png
//! ```

mod image;
mod rl_eval;
mod run;
mod trace;

use std::path::PathBuf;
use std::time::Instant;

use accrete_core::world::stats::world_stats;
use accrete_core::world::validate::validate_world;
use accrete_core::{Config, generate_world, hash, seed::parse_seed};
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
        /// One or more seeds (comma separated; decimal, 0x-hex, or text).
        #[arg(long, default_value = "1", value_delimiter = ',')]
        seed: Vec<String>,
        /// Print full statistics as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Run the simulation headlessly and export metrics.
    Run {
        /// One or more seeds (comma separated).
        #[arg(long, default_value = "1", value_delimiter = ',')]
        seed: Vec<String>,
        /// Number of days to simulate.
        #[arg(long, default_value_t = 100)]
        days: u32,
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
    /// Evaluate baseline policies in the reinforcement-learning environment.
    RlEval {
        /// Policies: heuristic, random, native (comma separated).
        #[arg(long, default_value = "native,heuristic,random", value_delimiter = ',')]
        policy: Vec<String>,
        /// Number of episodes (seeds 1..=n).
        #[arg(long, default_value_t = 10)]
        episodes: u64,
        /// Environment settings (TOML for `RlConfig`).
        #[arg(long)]
        rl_config: Option<PathBuf>,
        #[arg(long = "set")]
        sets: Vec<String>,
    },
    /// Print the effective configuration as TOML (defaults merged with --config).
    Config,
    /// Render world overlays to PNG images.
    Render {
        /// One or more seeds (comma separated). With several seeds and
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
            out,
            every,
            snap_every,
            check,
            sets,
            label,
            timelapse,
        } => {
            let mut config = config;
            for kv in &sets {
                config = run::apply_override(&config, kv);
            }
            config.sim.check_invariants |= check;
            for s in seed {
                run::run(
                    parse_seed(&s),
                    &config,
                    days,
                    &out,
                    every,
                    snap_every,
                    &label,
                    &timelapse,
                );
            }
        }
        Cmd::World { seed, json } => {
            for s in seed {
                let seed = parse_seed(&s);
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
        Cmd::RlEval {
            policy,
            episodes,
            rl_config,
            sets,
        } => {
            let mut config = config;
            for kv in &sets {
                config = run::apply_override(&config, kv);
            }
            let rl = match rl_config {
                None => accrete_core::rl::RlConfig::default(),
                Some(p) => {
                    let text = std::fs::read_to_string(&p).expect("read rl config");
                    accrete_core::rl::RlConfig::from_toml_str(&text).expect("parse rl config")
                }
            };
            let seeds: Vec<u64> = (1..=episodes).collect();
            rl_eval::rl_eval(&config, &rl, &policy, &seeds);
        }
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
            let worlds: Vec<_> = seed
                .iter()
                .map(|s| generate_world(parse_seed(s), &config))
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
