//! Accrete interactive viewer.
//!
//! The simulation runs at a fixed number of ticks per second chosen by the
//! user; rendering frame rate never changes simulation outcomes (the viewer
//! only calls `Simulation::step`).
//!
//! ```text
//! accrete-viewer [--seed S] [--config FILE] [--day D] [--load SNAPSHOT]
//!                [--play] [--select AGENT] [--intervene N [--view-b]]
//!                [--screenshot out.png [--frames N]]   # render and exit
//! ```
//!
//! Controls: Space pause · `.` step · `+`/`-` speed · Tab / 1-0 overlays ·
//! WASD/arrows/right-drag pan · wheel zoom · click to inspect · F follow ·
//! G regenerate (random seed) · S type a seed · H help.
//!
//! Experiments (accrete.md, Week 4 Day 5): K save a snapshot · L reload the
//! last snapshot · I choose an intervention · B branch: run a control and the
//! intervention side by side from now · V switch the branch on screen ·
//! X drop the branch. Interventions change world conditions only.

use std::path::PathBuf;

use accrete_core::agents::agent::{Agent, GoalKind};
use accrete_core::agents::decision::{Ctx, reproduce_blockers};
use accrete_core::config::TICKS_PER_DAY;
use accrete_core::inventory::Resource;
use accrete_core::seed::parse_seed;
use accrete_core::{Config, Pos, Simulation};
use accrete_viz::{Overlay, goal_color, render_tiles};
use macroquad::prelude::*;

const PANEL_W: f32 = 470.0;
const TOP_H: f32 = 30.0;
const SPEEDS: [f32; 8] = [1.0, 4.0, 12.0, 24.0, 72.0, 240.0, 720.0, 2400.0];

fn window_conf() -> Conf {
    Conf {
        window_title: "Accrete".to_owned(),
        window_width: 1400,
        window_height: 860,
        high_dpi: false,
        sample_count: 1,
        ..Default::default()
    }
}

struct Args {
    seed: u64,
    config: Config,
    day: u32,
    load: Option<PathBuf>,
    intervene: Option<usize>,
    view_b: bool,
    screenshot: Option<PathBuf>,
    frames: u32,
    select: Option<u32>,
    play: bool,
    /// `--seed` or `--config` given: these describe a new world, so they
    /// conflict with `--load`.
    new_world_args: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        seed: 1,
        config: Config::default(),
        day: 0,
        load: None,
        intervene: None,
        view_b: false,
        screenshot: None,
        frames: 3,
        select: None,
        play: false,
        new_world_args: false,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let next = argv.get(i + 1).cloned().unwrap_or_default();
        match argv[i].as_str() {
            "--seed" => {
                a.seed = parse_seed(&next);
                a.new_world_args = true;
            }
            "--day" => a.day = next.parse().unwrap_or(0),
            "--frames" => a.frames = next.parse().unwrap_or(3),
            "--select" => a.select = next.parse().ok(),
            "--play" => {
                a.play = true;
                i -= 1; // flag takes no value
            }
            "--screenshot" => a.screenshot = Some(PathBuf::from(&next)),
            "--load" => a.load = Some(PathBuf::from(&next)),
            "--intervene" => match next.parse::<usize>() {
                Ok(k) if k < INTERVENTIONS.len() => a.intervene = Some(k),
                _ => {
                    eprintln!(
                        "error: --intervene expects 0..={}, got {next:?}",
                        INTERVENTIONS.len() - 1
                    );
                    std::process::exit(2);
                }
            },
            "--view-b" => {
                a.view_b = true;
                i -= 1;
            }
            "--config" => {
                let text = std::fs::read_to_string(&next).expect("read config");
                a.config = Config::from_toml_str(&text).expect("parse config");
                a.new_world_args = true;
            }
            other => {
                eprintln!("unknown argument {other}");
                i -= 1;
            }
        }
        i += 2;
    }
    if a.load.is_some() && a.new_world_args {
        eprintln!("error: --seed and --config describe a new world; a --load snapshot has its own");
        std::process::exit(2);
    }
    a
}

enum Selection {
    None,
    Tile(Pos),
    Agent(u32),
}

/// Intervention presets for branching from the viewer. Each one changes only
/// world conditions (a scripted drought, a route disruption, or a regrowth
/// rate), starting on the day the branch is made.
const INTERVENTIONS: [&str; 4] = [
    "drought around the selected tile (r 16, 60 days, -60% regrowth)",
    "cut the 20 busiest routes (20 days)",
    "flood around the selected tile (r 3, 30 days, impassable)",
    "halve food regrowth from now on",
];

fn intervention_config(base: &Config, which: usize, day: u32, at: Pos) -> Config {
    use accrete_core::config::{DisruptionKind, ScriptedDisruption, ScriptedDrought};
    let mut c = base.clone();
    let disruption = |radius: u32, busiest: u32, days: u32| ScriptedDisruption {
        start_day: day,
        days,
        kind: DisruptionKind::Block,
        slow_factor: 3,
        center: [at.x, at.y],
        radius,
        busiest_tiles: busiest,
    };
    match which {
        0 => c.weather.scripted.push(ScriptedDrought {
            start_day: day,
            days: 60,
            center: [at.x, at.y],
            radius: 16,
            severity: 0.6,
        }),
        1 => c.disruption.scripted.push(disruption(0, 20, 20)),
        2 => c.disruption.scripted.push(disruption(3, 0, 30)),
        _ => c.resources.food_regrowth *= 0.5,
    }
    c
}

/// A second simulation running in lockstep with the first, for comparison.
struct Branch {
    /// The simulation not on screen (control or intervention).
    other: Simulation,
    /// Whether the intervention branch is the one on screen.
    viewing_intervention: bool,
    intervention: usize,
    from_day: u32,
    /// Next agent id at the split: agents with `id < from_next_id` exist on
    /// both sides with the same identity; later ids are branch-local newborns.
    from_next_id: u32,
}

struct App {
    sim: Simulation,
    seed: u64,
    config: Config,
    overlay: Overlay,
    texture: Texture2D,
    texture_tick: u64,
    zoom: f32,
    offset: Vec2,
    paused: bool,
    speed: usize,
    accumulator: f32,
    selection: Selection,
    follow: bool,
    help: bool,
    seed_entry: Option<String>,
    last_drag: Option<Vec2>,
    branch: Option<Branch>,
    intervention: usize,
    last_snapshot: Option<PathBuf>,
    status: Option<(String, f64)>,
}

impl App {
    fn new(seed: u64, config: Config) -> Self {
        let sim = Simulation::new(seed, config.clone());
        let texture = map_texture(&sim, Overlay::Biome);
        let mut app = Self {
            sim,
            seed,
            config,
            overlay: Overlay::Biome,
            texture,
            texture_tick: 0,
            zoom: 1.0,
            offset: Vec2::ZERO,
            paused: true,
            speed: 3,
            accumulator: 0.0,
            selection: Selection::None,
            follow: false,
            help: false,
            seed_entry: None,
            last_drag: None,
            branch: None,
            intervention: 0,
            last_snapshot: None,
            status: None,
        };
        app.fit();
        app
    }

    /// Start from a snapshot file.
    fn from_snapshot(path: &std::path::Path) -> Result<Self, String> {
        let sim = Simulation::load_snapshot(path)?;
        let mut app = App::new(sim.world.seed, sim.config.clone());
        app.apply_loaded_sim(sim);
        app.last_snapshot = Some(path.to_path_buf());
        app.refresh_texture();
        app.fit();
        Ok(app)
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), get_time()));
    }

    /// The tile interventions centre on: the selected tile or agent, else
    /// the middle of the map.
    fn focus(&self) -> Pos {
        match self.selection {
            Selection::Tile(p) => p,
            Selection::Agent(id) => self.sim.agent(id).map_or(self.centre(), |a| a.pos),
            Selection::None => self.centre(),
        }
    }

    fn centre(&self) -> Pos {
        let d = self.sim.world.dims;
        Pos::new(d.width as i32 / 2, d.height as i32 / 2)
    }

    fn save_snapshot(&mut self) {
        let side = match &self.branch {
            Some(b) if b.viewing_intervention => "_intervention",
            Some(_) => "_control",
            None => "",
        };
        let path = PathBuf::from("results").join(format!(
            "viewer_seed{}_day{:03}_t{}{side}.snap.json",
            self.seed,
            self.sim.day(),
            self.sim.tick
        ));
        match self.sim.save_snapshot(&path) {
            Ok(()) => {
                self.say(format!("saved {}", path.display()));
                self.last_snapshot = Some(path);
            }
            Err(e) => self.say(format!("save failed: {e}")),
        }
    }

    fn reload_snapshot(&mut self) {
        let Some(path) = self.last_snapshot.clone() else {
            self.say("no snapshot yet: press K to save one");
            return;
        };
        match Simulation::load_snapshot(&path) {
            Ok(sim) => {
                let note = if sim.config.differs_in_results(&self.config) {
                    " - it carries an intervention"
                } else {
                    ""
                };
                self.apply_loaded_sim(sim);
                self.branch = None;
                self.keep_selection_if_present();
                self.refresh_texture();
                self.say(format!(
                    "reloaded {} (day {}){note}",
                    path.display(),
                    self.sim.day()
                ));
            }
            Err(e) => self.say(format!("load failed: {e}")),
        }
    }

    /// Mirror `from_snapshot`: G/S must not replay scripted interventions
    /// pinned to this world's days and tiles.
    fn apply_loaded_sim(&mut self, sim: Simulation) {
        self.seed = sim.world.seed;
        self.config = sim.config.clone();
        self.config.weather.scripted.clear();
        self.config.disruption.scripted.clear();
        self.sim = sim;
    }

    /// Branch from the current state: the simulation on screen becomes the
    /// control, and an identical copy gets the chosen intervention.
    fn start_branch(&mut self) -> Result<(), String> {
        let copy = match Simulation::from_snapshot_json(&self.sim.snapshot_json()) {
            Ok(s) => s,
            Err(e) => {
                self.say(format!("branch failed: {e}"));
                return Err(e);
            }
        };
        let day = self.sim.day();
        let cfg = intervention_config(&self.sim.config, self.intervention, day, self.focus());
        if !cfg.differs_in_results(&self.sim.config) {
            let e = "intervention does not change results (e.g. food regrowth is already 0)".into();
            self.say(format!("branch failed: {e}"));
            return Err(e);
        }
        let mut intervention = copy;
        if let Err(e) = intervention.set_config(cfg) {
            self.say(format!("branch failed: {e}"));
            return Err(e);
        }
        // Show the intervention; the control keeps running alongside.
        let from_next_id = self.sim.next_id;
        let control = std::mem::replace(&mut self.sim, intervention);
        self.branch = Some(Branch {
            other: control,
            viewing_intervention: true,
            intervention: self.intervention,
            from_day: day,
            from_next_id,
        });
        self.refresh_texture();
        self.say(format!(
            "branched on day {day}: {}",
            INTERVENTIONS[self.intervention]
        ));
        Ok(())
    }

    fn switch_branch(&mut self) {
        if let Some(b) = self.branch.as_mut() {
            std::mem::swap(&mut self.sim, &mut b.other);
            b.viewing_intervention = !b.viewing_intervention;
            self.keep_selection_if_present();
            self.refresh_texture();
        } else {
            self.say("no branch: press B to branch");
        }
    }

    /// Each branch numbers its own newborns, so an agent id from one
    /// simulation may be someone else, or no one, in the other. Keep the
    /// selection only for agents that existed before the simulations split
    /// (`id < from_next_id`).
    fn keep_selection_if_present(&mut self) {
        if let Selection::Agent(id) = self.selection {
            let split = self.branch.as_ref().map(|b| b.from_next_id);
            let same_person = split.is_none_or(|n| id < n);
            if !(same_person && self.sim.agent(id).is_some()) {
                self.selection = Selection::None;
                self.follow = false;
            }
        }
    }

    /// Run one tick, keeping a branch in lockstep.
    fn step_all(&mut self) {
        self.sim.step();
        if let Some(b) = self.branch.as_mut() {
            b.other.step();
        }
    }

    fn regenerate(&mut self, seed: u64) {
        // A new world drops any branch.
        let config = self.config.clone();
        *self = App {
            help: self.help,
            ..App::new(seed, config)
        };
    }

    fn map_area(&self) -> Rect {
        Rect::new(
            0.0,
            TOP_H,
            screen_width() - PANEL_W,
            screen_height() - TOP_H,
        )
    }

    fn fit(&mut self) {
        let area = self.map_area();
        let w = self.sim.world.dims.width as f32;
        let h = self.sim.world.dims.height as f32;
        self.zoom = (area.w / w).min(area.h / h);
        self.offset = vec2(
            area.x + (area.w - w * self.zoom) / 2.0,
            area.y + (area.h - h * self.zoom) / 2.0,
        );
    }

    fn tile_to_screen(&self, p: Pos) -> Vec2 {
        self.offset + vec2(p.x as f32 + 0.5, p.y as f32 + 0.5) * self.zoom
    }

    fn screen_to_tile(&self, s: Vec2) -> Option<Pos> {
        let t = (s - self.offset) / self.zoom;
        let p = Pos::new(t.x.floor() as i32, t.y.floor() as i32);
        self.sim.world.contains(p).then_some(p)
    }

    fn refresh_texture(&mut self) {
        self.texture = map_texture(&self.sim, self.overlay);
        self.texture_tick = self.sim.tick;
    }

    fn handle_input(&mut self) {
        // Seed entry mode captures the keyboard.
        if let Some(buf) = self.seed_entry.as_mut() {
            while let Some(c) = get_char_pressed() {
                if !c.is_control() {
                    buf.push(c);
                }
            }
            if is_key_pressed(KeyCode::Backspace) {
                buf.pop();
            }
            if is_key_pressed(KeyCode::Escape) {
                self.seed_entry = None;
            } else if is_key_pressed(KeyCode::Enter) {
                let s = parse_seed(buf);
                self.seed_entry = None;
                self.regenerate(s);
            }
            return;
        }
        while get_char_pressed().is_some() {}

        if is_key_pressed(KeyCode::Space) {
            self.paused = !self.paused;
        }
        if is_key_pressed(KeyCode::Period) {
            self.step_all();
        }
        if is_key_pressed(KeyCode::K) {
            self.save_snapshot();
        }
        if is_key_pressed(KeyCode::L) {
            self.reload_snapshot();
        }
        if is_key_pressed(KeyCode::I) {
            self.intervention = (self.intervention + 1) % INTERVENTIONS.len();
            self.say(format!(
                "intervention: {}",
                INTERVENTIONS[self.intervention]
            ));
        }
        if is_key_pressed(KeyCode::B) {
            if self.branch.is_some() {
                self.say("already branched: X drops the branch");
            } else {
                // A failure is reported on screen.
                let _ = self.start_branch();
            }
        }
        if is_key_pressed(KeyCode::V) {
            self.switch_branch();
        }
        if is_key_pressed(KeyCode::X) && self.branch.is_some() {
            if self.branch.as_ref().is_some_and(|b| b.viewing_intervention) {
                self.switch_branch();
            }
            self.branch = None;
            self.refresh_texture();
            self.say("branch dropped; showing the control");
        }
        if is_key_pressed(KeyCode::Equal) || is_key_pressed(KeyCode::KpAdd) {
            self.speed = (self.speed + 1).min(SPEEDS.len() - 1);
        }
        if is_key_pressed(KeyCode::Minus) || is_key_pressed(KeyCode::KpSubtract) {
            self.speed = self.speed.saturating_sub(1);
        }
        if is_key_pressed(KeyCode::Tab) {
            self.overlay = self.overlay.next();
            self.refresh_texture();
        }
        let digit_keys = [
            KeyCode::Key1,
            KeyCode::Key2,
            KeyCode::Key3,
            KeyCode::Key4,
            KeyCode::Key5,
            KeyCode::Key6,
            KeyCode::Key7,
            KeyCode::Key8,
            KeyCode::Key9,
            KeyCode::Key0,
        ];
        for (k, key) in digit_keys.iter().enumerate() {
            if is_key_pressed(*key) {
                if let Some(&o) = Overlay::ALL.get(k) {
                    self.overlay = o;
                    self.refresh_texture();
                }
            }
        }
        if is_key_pressed(KeyCode::G) {
            let s = macroquad::rand::rand() as u64 ^ ((get_time() * 1e6) as u64);
            self.regenerate(s);
        }
        if is_key_pressed(KeyCode::S) {
            self.seed_entry = Some(String::new());
        }
        if is_key_pressed(KeyCode::H) {
            self.help = !self.help;
        }
        if is_key_pressed(KeyCode::F) {
            self.follow = !self.follow;
        }
        if is_key_pressed(KeyCode::C) {
            self.fit();
        }

        // Pan.
        let pan = 600.0 * get_frame_time();
        if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
            self.offset.x += pan;
        }
        if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
            self.offset.x -= pan;
        }
        if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up) {
            self.offset.y += pan;
        }
        if is_key_down(KeyCode::Down) {
            self.offset.y -= pan;
        }
        let mouse = Vec2::from(mouse_position());
        if is_mouse_button_down(MouseButton::Right) || is_mouse_button_down(MouseButton::Middle) {
            if let Some(prev) = self.last_drag {
                self.offset += mouse - prev;
            }
            self.last_drag = Some(mouse);
        } else {
            self.last_drag = None;
        }

        // Zoom around the cursor.
        let (_, wheel) = mouse_wheel();
        if wheel != 0.0 && self.map_area().contains(mouse) {
            let factor = if wheel > 0.0 { 1.15 } else { 1.0 / 1.15 };
            let new_zoom = (self.zoom * factor).clamp(2.0, 80.0);
            let k = new_zoom / self.zoom;
            self.offset = mouse - (mouse - self.offset) * k;
            self.zoom = new_zoom;
        }

        // Selection.
        if is_mouse_button_pressed(MouseButton::Left) && self.map_area().contains(mouse) {
            if let Some(tile) = self.screen_to_tile(mouse) {
                let picked = self
                    .sim
                    .agents
                    .iter()
                    .map(|a| (a.id, self.tile_to_screen(a.pos).distance(mouse)))
                    .filter(|&(_, d)| d < (self.zoom * 0.6).max(6.0))
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                self.selection = match picked {
                    Some((id, _)) => Selection::Agent(id),
                    None => Selection::Tile(tile),
                };
            }
        }
    }

    fn advance(&mut self) {
        if self.paused || self.seed_entry.is_some() {
            return;
        }
        self.accumulator += SPEEDS[self.speed] * get_frame_time().min(0.1);
        let mut budget = 0;
        let budget_max = if self.branch.is_some() { 1000 } else { 2000 };
        while self.accumulator >= 1.0 && budget < budget_max {
            self.step_all();
            self.accumulator -= 1.0;
            budget += 1;
        }
        if budget == budget_max {
            self.accumulator = 0.0;
        }
        // Resource overlays change daily; others only on regenerate.
        if self.sim.tick / TICKS_PER_DAY != self.texture_tick / TICKS_PER_DAY {
            self.refresh_texture();
        }
        if self.follow {
            if let Selection::Agent(id) = self.selection {
                if let Some(a) = self.sim.agent(id) {
                    let area = self.map_area();
                    let target = vec2(area.x + area.w / 2.0, area.y + area.h / 2.0);
                    self.offset += target - self.tile_to_screen(a.pos);
                }
            }
        }
    }

    fn draw(&self) {
        clear_background(Color::from_rgba(16, 18, 22, 255));
        let w = self.sim.world.dims.width as f32 * self.zoom;
        let h = self.sim.world.dims.height as f32 * self.zoom;
        draw_texture_ex(
            &self.texture,
            self.offset.x,
            self.offset.y,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(w, h)),
                ..Default::default()
            },
        );
        self.draw_homes_and_agents();
        self.draw_selection_marker();
        self.draw_top_bar();
        self.draw_panel();
        if self.help {
            self.draw_help();
        }
        if let Some(buf) = &self.seed_entry {
            let area = self.map_area();
            draw_rectangle(
                area.x + 20.0,
                area.y + 20.0,
                420.0,
                40.0,
                Color::from_rgba(0, 0, 0, 220),
            );
            draw_text(
                format!("Seed: {buf}_  (Enter to generate, Esc to cancel)"),
                area.x + 30.0,
                area.y + 46.0,
                20.0,
                WHITE,
            );
        }
    }

    fn draw_homes_and_agents(&self) {
        let z = self.zoom;
        // Active droughts.
        for d in &self.sim.weather.active {
            let c = self.tile_to_screen(d.center);
            draw_circle(
                c.x,
                c.y,
                d.radius as f32 * z,
                Color::from_rgba(255, 140, 0, 40),
            );
            draw_circle_lines(
                c.x,
                c.y,
                d.radius as f32 * z,
                2.0,
                Color::from_rgba(255, 140, 0, 200),
            );
        }
        // Route disruptions (blocked or slowed tiles).
        for d in &self.sim.disruptions.active {
            let col = match d.effect {
                accrete_core::world::Disruption::Blocked => Color::from_rgba(230, 30, 30, 170),
                _ => Color::from_rgba(230, 140, 30, 150),
            };
            for &t in &d.tiles {
                let o = self.offset + vec2(0.0, 0.0);
                let p = self.sim.world.dims.pos(t);
                let tl = o + vec2(p.x as f32, p.y as f32) * z;
                draw_rectangle(tl.x, tl.y, z.max(2.0), z.max(2.0), col);
            }
        }
        // Detected settlements (measurement only), labelled by class and size.
        for st in &self.sim.settlements.tracked {
            let c = self.offset + vec2(st.centre.0 + 0.5, st.centre.1 + 0.5) * z;
            let (col, label) = match st.class {
                accrete_core::simulation::settlement::SizeClass::Candidate => {
                    (Color::from_rgba(170, 170, 255, 200), "forming")
                }
                accrete_core::simulation::settlement::SizeClass::Hamlet => {
                    (Color::from_rgba(255, 80, 200, 230), "hamlet")
                }
                accrete_core::simulation::settlement::SizeClass::Village => {
                    (Color::from_rgba(255, 50, 50, 240), "village")
                }
                accrete_core::simulation::settlement::SizeClass::Town => {
                    (Color::from_rgba(255, 0, 0, 255), "town")
                }
            };
            let r = (3.0 + (st.homes.len() as f32).sqrt()) * z;
            draw_circle_lines(c.x, c.y, r, 2.0, col);
            draw_text(
                format!("{label} {}p", st.residents),
                c.x - r * 0.7,
                c.y - r - 4.0,
                16.0,
                col,
            );
        }
        for home in &self.sim.world.homes {
            let c = self.tile_to_screen(home.pos);
            let s = (z * 0.8).max(3.0);
            let color = if home.is_vacant() {
                Color::from_rgba(120, 120, 120, 200)
            } else if home.shelter_level > 0 {
                Color::from_rgba(120, 255, 140, 240)
            } else if home.chests.is_empty() {
                Color::from_rgba(240, 240, 240, 230)
            } else {
                Color::from_rgba(255, 210, 40, 240)
            };
            draw_rectangle_lines(
                c.x - s / 2.0,
                c.y - s / 2.0,
                s,
                s,
                (z * 0.12).clamp(1.0, 3.0),
                color,
            );
        }
        for a in &self.sim.agents {
            let c = self.tile_to_screen(a.pos);
            // Spread agents that share a tile so they remain visible.
            let jitter = ((a.id.wrapping_mul(2_654_435_761) >> 16) & 0xff) as f32 / 255.0 - 0.5;
            let jitter2 = ((a.id.wrapping_mul(40_503) >> 8) & 0xff) as f32 / 255.0 - 0.5;
            let p = c + vec2(jitter, jitter2) * z * 0.6;
            let rgb = goal_color(a.goal.kind as usize);
            let r = if a.is_juvenile(&self.sim.rates) {
                0.18
            } else {
                0.26
            };
            draw_circle(
                p.x,
                p.y,
                (z * r).max(1.5),
                Color::from_rgba(rgb[0], rgb[1], rgb[2], 255),
            );
        }
    }

    fn draw_selection_marker(&self) {
        match self.selection {
            Selection::Agent(id) => {
                if let Some(a) = self.sim.agent(id) {
                    let c = self.tile_to_screen(a.pos);
                    draw_circle_lines(c.x, c.y, (self.zoom * 0.9).max(8.0), 2.0, RED);
                    if let Some(r) = &a.route {
                        let mut prev = c;
                        for &s in r.remaining() {
                            let q = self.tile_to_screen(s);
                            draw_line(
                                prev.x,
                                prev.y,
                                q.x,
                                q.y,
                                2.0,
                                Color::from_rgba(255, 80, 80, 200),
                            );
                            prev = q;
                        }
                    }
                    if let Some(h) = a.home {
                        let hp = self.tile_to_screen(self.sim.world.homes[h as usize].pos);
                        draw_line(
                            c.x,
                            c.y,
                            hp.x,
                            hp.y,
                            1.0,
                            Color::from_rgba(255, 210, 40, 160),
                        );
                    }
                }
            }
            Selection::Tile(p) => {
                let o = self.offset + vec2(p.x as f32, p.y as f32) * self.zoom;
                draw_rectangle_lines(o.x, o.y, self.zoom, self.zoom, 2.0, RED);
            }
            Selection::None => {}
        }
    }

    fn draw_top_bar(&self) {
        draw_rectangle(
            0.0,
            0.0,
            screen_width(),
            TOP_H,
            Color::from_rgba(8, 9, 12, 240),
        );
        let m = self.sim.metrics.last();
        let text = format!(
            "seed {}  |  day {} tick {}  |  pop {}  homes {}  |  {}  {} ticks/s  |  overlay [{}] {}  |  H help",
            self.seed,
            self.sim.day(),
            self.sim.tick,
            self.sim.population(),
            m.map_or(0, |m| m.homes_occupied),
            if self.paused { "PAUSED" } else { "running" },
            SPEEDS[self.speed],
            Overlay::ALL
                .iter()
                .position(|&o| o == self.overlay)
                .map_or(0, |i| (i + 1) % 10),
            self.overlay.name(),
        );
        draw_text(&text, 10.0, 21.0, 20.0, WHITE);
        let mut right = String::new();
        if let Some(b) = &self.branch {
            right = format!(
                "BRANCH day {}: {} | viewing {} (V switch, X drop)",
                b.from_day,
                INTERVENTIONS[b.intervention],
                if b.viewing_intervention {
                    "intervention"
                } else {
                    "control"
                }
            );
        }
        if let Some((msg, t)) = &self.status {
            if get_time() - t < 6.0 {
                if !right.is_empty() {
                    right.push_str("  |  ");
                }
                right.push_str(msg);
            }
        }
        if !right.is_empty() {
            let area = self.map_area();
            draw_rectangle(
                area.x,
                area.y,
                area.w,
                24.0,
                Color::from_rgba(40, 10, 10, 210),
            );
            draw_text(
                &right,
                area.x + 10.0,
                area.y + 17.0,
                18.0,
                Color::from_rgba(255, 200, 120, 255),
            );
        }
    }

    fn draw_panel(&self) {
        let x0 = screen_width() - PANEL_W;
        draw_rectangle(
            x0,
            TOP_H,
            PANEL_W,
            screen_height() - TOP_H,
            Color::from_rgba(12, 14, 18, 250),
        );
        let mut y = TOP_H + 20.0;
        let mut line = |s: &str, color: Color| {
            for chunk in wrap(s, 54) {
                draw_text(&chunk, x0 + 10.0, y, 16.0, color);
                y += 16.0;
            }
        };
        let dim = Color::from_rgba(150, 156, 170, 255);
        match &self.selection {
            Selection::Agent(id) => match self.sim.agent(*id) {
                Some(a) => self.agent_lines(a, &mut line),
                None => {
                    line(&format!("Agent {id} (dead)"), WHITE);
                    if let Some(d) = self.sim.deaths.iter().rev().find(|d| d.agent == *id) {
                        line(
                            &format!(
                                "died day {} at age {:.0} d",
                                d.tick / TICKS_PER_DAY,
                                d.age_days
                            ),
                            dim,
                        );
                        line(&format!("cause: {}", d.cause.name()), WHITE);
                        for chunk in wrap(&d.explanation, 44) {
                            line(&chunk, dim);
                        }
                    }
                }
            },
            Selection::Tile(p) => self.tile_lines(*p, &mut line),
            Selection::None => {
                line("Click an agent or tile to inspect.", dim);
            }
        }
        self.draw_chart(x0 + 10.0, screen_height() - 210.0, PANEL_W - 20.0, 170.0);
    }

    fn agent_lines(&self, a: &Agent, line: &mut impl FnMut(&str, Color)) {
        let r = &self.sim.rates;
        let dim = Color::from_rgba(150, 156, 170, 255);
        let accent = Color::from_rgba(255, 210, 120, 255);
        line(
            &format!(
                "Agent {}  {}  age {:.0} d",
                a.id,
                if a.is_juvenile(r) { "(juvenile)" } else { "" },
                a.age_days()
            ),
            WHITE,
        );
        line(
            &format!(
                "hunger {:.2}  energy {:.2}  health {:.2}/{:.2}",
                a.hunger,
                a.energy,
                a.health,
                a.health_ceiling(r)
            ),
            WHITE,
        );
        line(
            &format!("frailty {:.3}  at ({}, {})", a.frailty, a.pos.x, a.pos.y),
            dim,
        );
        line(
            &format!(
                "carry food {:.1} wood {:.1} stone {:.1}",
                a.inventory.food, a.inventory.wood, a.inventory.stone
            ),
            dim,
        );
        line(
            &format!(
                "traits cur {:.2} cau {:.2} soc {:.2} dil {:.2}",
                a.traits.curiosity, a.traits.caution, a.traits.sociability, a.traits.diligence
            ),
            dim,
        );
        line(
            &format!(
                "skill food {:.2} wood {:.2} stone {:.2}",
                a.skills[Resource::Food.index()].level(),
                a.skills[Resource::Wood.index()].level(),
                a.skills[Resource::Stone.index()].level()
            ),
            dim,
        );
        match a.home {
            Some(h) => {
                let home = &self.sim.world.homes[h as usize];
                line(
                    &format!(
                        "home #{h} at ({}, {}) - {} residents, {} chests, {:.0} food",
                        home.pos.x,
                        home.pos.y,
                        home.residents.len(),
                        home.chests.len(),
                        home.stored().food
                    ),
                    dim,
                );
            }
            None => line("homeless", dim),
        }
        line(
            &format!(
                "knows {} tiles, {} food / {} wood memories, {} homes",
                a.memory.known_count(),
                a.memory.resource_count(Resource::Food),
                a.memory.resource_count(Resource::Wood),
                a.memory.homes.len()
            ),
            dim,
        );
        line(
            &format!("children {}  parent {:?}", a.stats.births, a.parent),
            dim,
        );
        let cx = Ctx {
            world: &self.sim.world,
            config: &self.sim.config,
            rates: r,
            tick: self.sim.tick,
            population_at_cap: self.sim.population() as u32 >= self.sim.config.life.population_cap,
            agents: &self.sim.agents,
        };
        let blockers = reproduce_blockers(a, &cx);
        line(
            &format!(
                "reproduce: {}",
                if blockers.is_empty() {
                    "eligible".to_string()
                } else {
                    blockers.join(", ")
                }
            ),
            dim,
        );
        line("", dim);
        line(
            &format!(
                "GOAL {} -> ({}, {})",
                a.goal.kind.name(),
                a.goal.target.x,
                a.goal.target.y
            ),
            accent,
        );
        if let Some(tr) = self.sim.traces.get(&a.id) {
            line(&format!("options at tick {}:", tr.tick), dim);
            for o in tr.options.iter().take(6) {
                let rgb = goal_color(o.kind as usize);
                line(
                    &format!(
                        "{:+.2} {} ({},{})",
                        o.score,
                        o.kind.name(),
                        o.target.x,
                        o.target.y
                    ),
                    Color::from_rgba(rgb[0], rgb[1], rgb[2], 255),
                );
                let terms: Vec<String> = o
                    .terms
                    .iter()
                    .filter(|t| t.value != 0.0)
                    .map(|t| format!("{} {:+.2}", t.name, t.value))
                    .collect();
                for chunk in wrap(&terms.join(", "), 50) {
                    line(&format!("    {chunk}"), dim);
                }
            }
        }
    }

    fn tile_lines(&self, p: Pos, line: &mut impl FnMut(&str, Color)) {
        let w = &self.sim.world;
        let t = w.tile(p);
        let dim = Color::from_rgba(150, 156, 170, 255);
        line(
            &format!("Tile ({}, {})  {}", p.x, p.y, t.biome.name()),
            WHITE,
        );
        line(
            &format!("elevation {:.3}  moisture {:.3}", t.elevation, t.moisture),
            dim,
        );
        line(
            &format!(
                "fertility {:.3}  river {}  river distance {}",
                t.fertility, t.is_river, t.river_distance
            ),
            dim,
        );
        line(
            &format!(
                "food {:.1}/{:.1} (patch x{:.1})",
                w.food_at(p),
                t.food_capacity,
                t.patch
            ),
            dim,
        );
        line(
            &format!(
                "wood {:.1}/{:.1}  stone {:.0}",
                t.wood, t.wood_capacity, t.stone
            ),
            dim,
        );
        let mp = w.enter_cost(p, &self.sim.config.movement);
        line(
            &format!(
                "move cost {}  traffic {:.1}  visible tiles {}",
                mp.map_or("impassable".into(), |m| format!("{m} MP")),
                t.traffic,
                w.vis.visible_count(w.dims.idx(p))
            ),
            dim,
        );
        for &h in w.homes_on(p) {
            let home = &w.homes[h as usize];
            line(
                &format!(
                    "home #{h}: {} residents, {} chests, {:.0} food stored",
                    home.residents.len(),
                    home.chests.len(),
                    home.stored().food
                ),
                WHITE,
            );
        }
        let here: Vec<u32> = self
            .sim
            .agents
            .iter()
            .filter(|a| a.pos == p)
            .map(|a| a.id)
            .collect();
        if !here.is_empty() {
            line(&format!("agents here: {here:?}"), dim);
        }
    }

    fn draw_chart(&self, x: f32, y: f32, w: f32, h: f32) {
        draw_rectangle(x, y, w, h, Color::from_rgba(20, 23, 29, 255));
        let m = &self.sim.metrics;
        let legend = if self.branch.is_some() {
            "pop (white; other branch orange), homes, hunger x500, R x100"
        } else {
            "pop (white), homes, hunger x500, R x100"
        };
        draw_text(legend, x + 4.0, y + 14.0, 14.0, GRAY);
        if m.len() < 2 {
            return;
        }
        let other = self
            .branch
            .as_ref()
            .map_or(&[][..], |b| &b.other.metrics[..]);
        let max_pop = m
            .iter()
            .chain(other)
            .map(|d| d.population)
            .max()
            .unwrap_or(1)
            .max(10) as f32;
        let n = m.len() as f32;
        let plot = |f: &dyn Fn(&accrete_core::simulation::metrics::DailyMetrics) -> f32,
                    color: Color| {
            let mut prev: Option<Vec2> = None;
            for (i, d) in m.iter().enumerate() {
                let px = x + 4.0 + (w - 8.0) * i as f32 / (n - 1.0);
                let py = y + h - 6.0 - (h - 26.0) * (f(d) / max_pop).min(1.0);
                let p = vec2(px, py);
                if let Some(q) = prev {
                    draw_line(q.x, q.y, p.x, p.y, 1.5, color);
                }
                prev = Some(p);
            }
        };
        plot(&|d| d.population as f32, WHITE);
        if let Some(b) = &self.branch {
            // The other branch's population, dashed orange.
            let om = &b.other.metrics;
            let mut prev: Option<Vec2> = None;
            for (i, d) in om.iter().enumerate() {
                let px = x + 4.0 + (w - 8.0) * i as f32 / (n - 1.0).max(1.0);
                let py = y + h - 6.0 - (h - 26.0) * (d.population as f32 / max_pop).min(1.0);
                let p = vec2(px, py);
                if let Some(q) = prev {
                    if i % 2 == 0 {
                        draw_line(q.x, q.y, p.x, p.y, 1.5, Color::from_rgba(255, 150, 60, 255));
                    }
                }
                prev = Some(p);
            }
            let (a, bpop) = (self.sim.population(), b.other.population());
            let (ctrl, intv) = if b.viewing_intervention {
                (bpop, a)
            } else {
                (a, bpop)
            };
            draw_text(
                format!(
                    "{} | control {ctrl} vs intervention {intv} ({:+})",
                    if b.viewing_intervention {
                        "viewing intervention"
                    } else {
                        "viewing control"
                    },
                    intv as i64 - ctrl as i64
                ),
                x + 4.0,
                y - 6.0,
                14.0,
                Color::from_rgba(255, 170, 90, 255),
            );
        }
        plot(
            &|d| d.homes_occupied as f32,
            Color::from_rgba(255, 210, 40, 255),
        );
        plot(
            &|d| d.mean_hunger * 500.0,
            Color::from_rgba(255, 90, 90, 255),
        );
        plot(
            &|d| d.clark_evans * 100.0,
            Color::from_rgba(80, 220, 255, 255),
        );
        let last = m.last().unwrap();
        draw_text(
            format!(
                "births {}  deaths {}/{}  hamlets {}  villages {}  pathologies {}",
                last.total_births,
                last.total_deaths_deprivation,
                last.total_deaths_old_age,
                last.hamlets,
                last.villages,
                last.pathology_oscillating
                    + last.pathology_stuck
                    + last.pathology_starved_beside_food
                    + last.pathology_idle
            ),
            x + 4.0,
            y + h + 14.0,
            14.0,
            GRAY,
        );
        draw_text(
            format!(
                "R {:.2} depl {:.2} forage {:.1}t trails {} roads {} shel {} trade {} tips {}{}",
                last.clark_evans,
                last.depletion_ratio,
                last.forage_radius,
                last.trail_tiles,
                last.road_tiles,
                last.shelters,
                last.trades,
                last.tips_shared,
                if last.drought_area > 0.0 {
                    "  DROUGHT"
                } else {
                    ""
                }
            ),
            x + 4.0,
            y + h + 30.0,
            14.0,
            GRAY,
        );
    }

    fn draw_help(&self) {
        let area = self.map_area();
        let lines = [
            "Space  pause / run          .  step one tick",
            "+ / -  speed                Tab / 1-0  overlay",
            "WASD / arrows / right-drag  pan     wheel  zoom",
            "click  inspect agent/tile   F  follow selected agent",
            "C  fit map    G  new random seed    S  type a seed",
            "K  save snapshot   L  reload it   I  pick intervention",
            "B  branch (control + intervention)   V  switch   X  drop",
            "Agent colours = current goal (see legend). Homes: white,",
            "gold = has storage, grey = vacant. Juveniles are smaller.",
        ];
        let h = 26.0 + 20.0 * (lines.len() + GoalKind::ALL.len() / 2 + 1) as f32;
        draw_rectangle(
            area.x + 20.0,
            area.y + 20.0,
            560.0,
            h,
            Color::from_rgba(0, 0, 0, 220),
        );
        let mut y = area.y + 44.0;
        for l in lines {
            draw_text(l, area.x + 34.0, y, 18.0, WHITE);
            y += 20.0;
        }
        for (k, g) in GoalKind::ALL.iter().enumerate() {
            let rgb = goal_color(*g as usize);
            let col = (k % 2) as f32 * 260.0;
            let yy = y + (k / 2) as f32 * 20.0;
            draw_circle(
                area.x + 40.0 + col,
                yy - 5.0,
                6.0,
                Color::from_rgba(rgb[0], rgb[1], rgb[2], 255),
            );
            draw_text(g.name(), area.x + 52.0 + col, yy, 18.0, WHITE);
        }
    }
}

fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split(' ') {
        if !cur.is_empty() && cur.len() + word.len() + 1 > width {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn map_texture(sim: &Simulation, overlay: Overlay) -> Texture2D {
    let colors = render_tiles(&sim.world, &sim.config, overlay);
    let mut bytes = Vec::with_capacity(colors.len() * 4);
    for c in colors {
        bytes.extend_from_slice(&[c[0], c[1], c[2], 255]);
    }
    let tex = Texture2D::from_rgba8(
        sim.world.dims.width as u16,
        sim.world.dims.height as u16,
        &bytes,
    );
    tex.set_filter(FilterMode::Nearest);
    tex
}

#[macroquad::main(window_conf)]
async fn main() {
    let args = parse_args();
    let mut app = match &args.load {
        Some(path) => App::from_snapshot(path).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1)
        }),
        None => App::new(args.seed, args.config.clone()),
    };
    // `--day` is the day to show; a loaded snapshot may already be past it.
    let target = u64::from(args.day) * TICKS_PER_DAY;
    if target > app.sim.tick {
        app.sim.run(target - app.sim.tick);
        app.refresh_texture();
    } else if args.load.is_some() && args.day > 0 && target < app.sim.tick {
        eprintln!(
            "warning: the snapshot is already at day {}, past --day {}",
            app.sim.day(),
            args.day
        );
    }
    if let Some(id) = args.select {
        app.selection = Selection::Agent(id);
    } else if args.screenshot.is_some() {
        if let Some(a) = app.sim.agents.get(app.sim.agents.len() / 2) {
            app.selection = Selection::Agent(a.id);
        }
    }
    if let Some(k) = args.intervene {
        app.intervention = k;
        if let Err(e) = app.start_branch() {
            eprintln!("error: --intervene {k}: {e}");
            std::process::exit(1);
        }
        if !args.view_b {
            app.switch_branch();
        }
    }
    app.paused = !args.play;
    let mut frame = 0u32;
    loop {
        app.handle_input();
        app.advance();
        app.draw();
        frame += 1;
        if let Some(path) = &args.screenshot {
            if frame >= args.frames {
                get_screen_data().export_png(&path.to_string_lossy());
                break;
            }
        }
        next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every preset is a valid intervention that starts on the branch day and
    /// takes effect, and branching leaves the control exactly as an
    /// unbranched run.
    #[test]
    fn presets_branch_without_touching_the_control() {
        let mut base = Simulation::new(1, Config::default());
        base.run(20 * TICKS_PER_DAY + 5);
        let mut plain = Simulation::from_snapshot_json(&base.snapshot_json()).unwrap();
        plain.run(3 * TICKS_PER_DAY);
        let at = base.agents[0].pos;
        for k in 0..INTERVENTIONS.len() {
            let mut control = Simulation::from_snapshot_json(&base.snapshot_json()).unwrap();
            let mut branch = Simulation::from_snapshot_json(&control.snapshot_json()).unwrap();
            let cfg = intervention_config(&control.config, k, control.day(), at);
            assert!(cfg.differs_in_results(&control.config), "preset {k}");
            for d in &cfg.weather.scripted {
                assert_eq!(d.start_day, control.day(), "preset {k}");
            }
            for d in &cfg.disruption.scripted {
                assert_eq!(d.start_day, control.day(), "preset {k}");
            }
            branch.set_config(cfg).expect("preset accepted");
            for _ in 0..3 * TICKS_PER_DAY {
                control.step();
                branch.step();
            }
            assert_eq!(control.tick, branch.tick);
            assert_eq!(control.state_hash(), plain.state_hash(), "preset {k}");
            assert_ne!(
                branch.state_hash(),
                plain.state_hash(),
                "preset {k} had no effect"
            );
        }
    }
}
