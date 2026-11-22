//! Accrete interactive viewer.
//!
//! The simulation runs at a fixed number of ticks per second chosen by the
//! user; rendering frame rate never changes simulation outcomes (the viewer
//! only calls `Simulation::step`).
//!
//! ```text
//! accrete-viewer [--seed S] [--config FILE] [--day D]
//!                [--play] [--select AGENT]
//!                [--screenshot out.png [--frames N]]   # render and exit
//! ```
//!
//! Controls: Space pause · `.` step · `+`/`-` speed · Tab / 1-0 overlays ·
//! WASD/arrows/right-drag pan · wheel zoom · click to inspect · F follow ·
//! G regenerate (random seed) · S type a seed · H help.

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
    screenshot: Option<PathBuf>,
    frames: u32,
    select: Option<u32>,
    play: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        seed: 1,
        config: Config::default(),
        day: 0,
        screenshot: None,
        frames: 3,
        select: None,
        play: false,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let next = argv.get(i + 1).cloned().unwrap_or_default();
        match argv[i].as_str() {
            "--seed" => a.seed = parse_seed(&next),
            "--day" => a.day = next.parse().unwrap_or(0),
            "--frames" => a.frames = next.parse().unwrap_or(3),
            "--select" => a.select = next.parse().ok(),
            "--play" => {
                a.play = true;
                i -= 1; // flag takes no value
            }
            "--screenshot" => a.screenshot = Some(PathBuf::from(&next)),
            "--config" => {
                let text = std::fs::read_to_string(&next).expect("read config");
                a.config = Config::from_toml_str(&text).expect("parse config");
            }
            other => {
                eprintln!("unknown argument {other}");
                i -= 1;
            }
        }
        i += 2;
    }
    a
}

enum Selection {
    None,
    Tile(Pos),
    Agent(u32),
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
        };
        app.fit();
        app
    }

    fn regenerate(&mut self, seed: u64) {
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
            self.sim.step();
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
        while self.accumulator >= 1.0 && budget < 2000 {
            self.sim.step();
            self.accumulator -= 1.0;
            budget += 1;
        }
        if budget == 2000 {
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
        draw_text(
            "pop (white), homes (gold), hunger x500 (red), clustering R x100 (cyan)",
            x + 4.0,
            y + 14.0,
            14.0,
            GRAY,
        );
        if m.len() < 2 {
            return;
        }
        let max_pop = m.iter().map(|d| d.population).max().unwrap_or(1).max(10) as f32;
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
    let mut app = App::new(args.seed, args.config.clone());
    if args.day > 0 {
        app.sim.run(u64::from(args.day) * TICKS_PER_DAY);
        app.refresh_texture();
    }
    if let Some(id) = args.select {
        app.selection = Selection::Agent(id);
    } else if args.screenshot.is_some() {
        if let Some(a) = app.sim.agents.get(app.sim.agents.len() / 2) {
            app.selection = Selection::Agent(a.id);
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
