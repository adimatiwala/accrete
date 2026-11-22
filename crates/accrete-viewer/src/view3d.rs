//! A 3D "diorama" view of the world (presentation only).
//!
//! The terrain is a low-poly heightfield: one quad per tile with the tile's
//! overlay colour, corners shared between neighbours so the land is
//! continuous, and sunlight baked into vertex colours (macroquad has no
//! lighting). The map sits on an earth base with a translucent sea, forests
//! are cones that thin out as wood is cut, and homes and people are small
//! lit models rebuilt every frame. Nothing here touches the simulation.

use accrete_core::Simulation;
use accrete_core::world::Biome;
use macroquad::models::Vertex;
use macroquad::prelude::*;

/// Vertical scale of the highest land, in tile widths.
const HEIGHT_SCALE: f32 = 16.0;
/// Depth of the deepest sea.
const SEA_DEPTH: f32 = 5.0;
/// Bottom of the diorama base.
const BASE: f32 = -8.0;
/// Per-mesh limits (u16 indices; the draw-call buffer is raised to match in
/// `window_conf`).
pub const MAX_VERTS: usize = 60_000;
pub const MAX_INDICES: usize = 90_000;

fn sun() -> Vec3 {
    vec3(-0.55, 0.85, -0.35).normalize()
}

/// Lambert shading with a little sky light, applied to a base colour.
fn shade(c: Color, n: Vec3) -> Color {
    let k = 0.38 + 0.68 * n.dot(sun()).max(0.0) + 0.10 * n.y.max(0.0);
    Color::new(
        (c.r * k).min(1.0),
        (c.g * k).min(1.0),
        (c.b * k).min(1.0),
        c.a,
    )
}

fn hash(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Accumulates lit faces into as many meshes as the index type allows.
#[derive(Default)]
pub struct MeshBuilder {
    pub meshes: Vec<Mesh>,
}

impl MeshBuilder {
    fn room(&mut self, verts: usize, idx: usize) -> &mut Mesh {
        let need_new = match self.meshes.last() {
            None => true,
            Some(m) => m.vertices.len() + verts > MAX_VERTS || m.indices.len() + idx > MAX_INDICES,
        };
        if need_new {
            self.meshes.push(Mesh {
                vertices: Vec::with_capacity(4096),
                indices: Vec::with_capacity(6144),
                texture: None,
            });
        }
        self.meshes.last_mut().unwrap()
    }

    /// A flat polygon (3 or 4 points), shaded by its normal; `outward` picks
    /// which side faces the light.
    pub fn face(&mut self, pts: &[Vec3], outward: Vec3, color: Color) {
        let n = if pts.len() == 4 {
            (pts[3] - pts[1]).cross(pts[2] - pts[0])
        } else {
            (pts[1] - pts[0]).cross(pts[2] - pts[0])
        };
        let mut n = n.normalize_or_zero();
        if n.dot(outward) < 0.0 {
            n = -n;
        }
        self.face_lit(pts, shade(color, n));
    }

    /// A flat polygon in a fixed colour (no shading).
    pub fn face_lit(&mut self, pts: &[Vec3], c: Color) {
        let col: [u8; 4] = c.into();
        let m = self.room(pts.len(), (pts.len() - 2) * 3);
        let base = m.vertices.len() as u16;
        for &p in pts {
            m.vertices.push(Vertex {
                position: p,
                uv: Vec2::ZERO,
                color: col,
                normal: Vec4::ZERO,
            });
        }
        for k in 1..pts.len() as u16 - 1 {
            m.indices.extend_from_slice(&[base, base + k, base + k + 1]);
        }
    }

    /// An axis-aligned box standing on `foot` (centre of its base).
    pub fn cuboid(&mut self, foot: Vec3, size: Vec3, color: Color) {
        let (hx, hz) = (size.x / 2.0, size.z / 2.0);
        let (x0, x1, z0, z1) = (foot.x - hx, foot.x + hx, foot.z - hz, foot.z + hz);
        let (y0, y1) = (foot.y, foot.y + size.y);
        let v = |x, y, z| vec3(x, y, z);
        self.face(
            &[v(x0, y1, z0), v(x1, y1, z0), v(x1, y1, z1), v(x0, y1, z1)],
            Vec3::Y,
            color,
        );
        self.face(
            &[v(x0, y0, z0), v(x0, y1, z0), v(x0, y1, z1), v(x0, y0, z1)],
            -Vec3::X,
            color,
        );
        self.face(
            &[v(x1, y0, z0), v(x1, y1, z0), v(x1, y1, z1), v(x1, y0, z1)],
            Vec3::X,
            color,
        );
        self.face(
            &[v(x0, y0, z0), v(x1, y0, z0), v(x1, y1, z0), v(x0, y1, z0)],
            -Vec3::Z,
            color,
        );
        self.face(
            &[v(x0, y0, z1), v(x1, y0, z1), v(x1, y1, z1), v(x0, y1, z1)],
            Vec3::Z,
            color,
        );
    }

    /// A pyramid or cone with `sides` faces, base centred on `foot`.
    pub fn cone(
        &mut self,
        foot: Vec3,
        radius: f32,
        height: f32,
        sides: u32,
        turn: f32,
        color: Color,
    ) {
        let apex = foot + vec3(0.0, height, 0.0);
        let ring = |k: u32| {
            let a = turn + k as f32 / sides as f32 * std::f32::consts::TAU;
            foot + vec3(a.cos() * radius, 0.0, a.sin() * radius)
        };
        for k in 0..sides {
            let (a, b) = (ring(k), ring(k + 1));
            let out = ((a + b) * 0.5 - foot).normalize_or_zero() + Vec3::Y * 0.3;
            self.face(&[a, b, apex], out, color);
        }
    }

    pub fn draw(&self) {
        for m in &self.meshes {
            draw_mesh(m);
        }
    }
}

pub struct View3D {
    pub on: bool,
    pub auto_orbit: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub target: Vec3,
    w: usize,
    h: usize,
    /// Heights at tile corners, `(w + 1) × (h + 1)`.
    corners: Vec<f32>,
    land: MeshBuilder,
    water: MeshBuilder,
    last_drag: Option<Vec2>,
}

impl View3D {
    pub fn new(sim: &Simulation) -> Self {
        let (w, h) = (
            sim.world.dims.width as usize,
            sim.world.dims.height as usize,
        );
        let mut v = Self {
            on: false,
            auto_orbit: false,
            yaw: 0.0,
            pitch: 0.0,
            dist: 0.0,
            target: Vec3::ZERO,
            w,
            h,
            corners: Vec::new(),
            land: MeshBuilder::default(),
            water: MeshBuilder::default(),
            last_drag: None,
        };
        v.reset_camera();
        v
    }

    pub fn reset_camera(&mut self) {
        self.yaw = 0.65;
        self.pitch = 0.72;
        self.dist = self.w.max(self.h) as f32 * 1.55;
        self.target = vec3(self.w as f32 / 2.0, 0.0, self.h as f32 / 2.0);
    }

    fn tile_height(sim: &Simulation, i: usize) -> f32 {
        let w = &sim.world;
        let t = &w.tiles[i];
        let s = w.sea_level;
        if t.elevation >= s {
            let u = ((t.elevation - s) / (1.0 - s).max(1e-3)).clamp(0.0, 1.0);
            let mut y = 0.25 + u.powf(1.35) * HEIGHT_SCALE;
            if t.is_river {
                y -= 0.35;
            }
            y
        } else {
            -0.4 - (s - t.elevation) / s.max(1e-3) * SEA_DEPTH
        }
    }

    /// Ground height at a point in tile coordinates (tile `x` spans `[x, x+1]`).
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        if self.corners.is_empty() {
            return 0.0;
        }
        let x = x.clamp(0.0, self.w as f32 - 1e-3);
        let z = z.clamp(0.0, self.h as f32 - 1e-3);
        let (ix, iz) = (x.floor() as usize, z.floor() as usize);
        let (fx, fz) = (x - ix as f32, z - iz as f32);
        let c = |cx: usize, cz: usize| self.corners[cz * (self.w + 1) + cx];
        let top = c(ix, iz) * (1.0 - fx) + c(ix + 1, iz) * fx;
        let bot = c(ix, iz + 1) * (1.0 - fx) + c(ix + 1, iz + 1) * fx;
        top * (1.0 - fz) + bot * fz
    }

    /// Rebuild the static scene (terrain, base, sea, trees, rocks) from the
    /// current world and the tile colours of the active overlay.
    pub fn rebuild(&mut self, sim: &Simulation, colors: &[[u8; 3]], biome_overlay: bool) {
        let (w, h) = (self.w, self.h);
        let heights: Vec<f32> = (0..w * h).map(|i| Self::tile_height(sim, i)).collect();
        let mut corners = vec![0.0f32; (w + 1) * (h + 1)];
        for cz in 0..=h {
            for cx in 0..=w {
                let (mut sum, mut n) = (0.0, 0.0);
                for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    if cx >= dx && cz >= dz && cx - dx < w && cz - dz < h {
                        sum += heights[(cz - dz) * w + (cx - dx)];
                        n += 1.0;
                    }
                }
                corners[cz * (w + 1) + cx] = sum / n;
            }
        }
        self.corners = corners;
        let c = |cx: usize, cz: usize| self.corners[cz * (w + 1) + cx];

        let mut land = MeshBuilder::default();
        let snow = Color::from_rgba(236, 240, 246, 255);
        for z in 0..h {
            for x in 0..w {
                let i = z * w + x;
                let rgb = colors[i];
                let mut col = Color::from_rgba(rgb[0], rgb[1], rgb[2], 255);
                let p = [
                    vec3(x as f32, c(x, z), z as f32),
                    vec3(x as f32 + 1.0, c(x + 1, z), z as f32),
                    vec3(x as f32 + 1.0, c(x + 1, z + 1), z as f32 + 1.0),
                    vec3(x as f32, c(x, z + 1), z as f32 + 1.0),
                ];
                let avg = (p[0].y + p[1].y + p[2].y + p[3].y) / 4.0;
                if biome_overlay {
                    if avg > 0.62 * HEIGHT_SCALE {
                        let t = ((avg / HEIGHT_SCALE - 0.62) / 0.18).clamp(0.0, 1.0);
                        col = Color::new(
                            col.r + (snow.r - col.r) * t,
                            col.g + (snow.g - col.g) * t,
                            col.b + (snow.b - col.b) * t,
                            1.0,
                        );
                    } else if avg < 0.0 {
                        // Sea floor: sand near the shore, darker in the deep.
                        let t = (-avg / SEA_DEPTH).clamp(0.0, 1.0);
                        col = Color::new(0.76 - 0.5 * t, 0.70 - 0.42 * t, 0.52 - 0.22 * t, 1.0);
                    }
                }
                land.face(&p, Vec3::Y, col);
            }
        }
        // Diorama base: earth walls around the edge, from the ground down.
        let earth = Color::from_rgba(104, 78, 56, 255);
        let mut wall = |a: Vec3, b: Vec3, out: Vec3| {
            land.face(
                &[a, b, vec3(b.x, BASE, b.z), vec3(a.x, BASE, a.z)],
                out,
                earth,
            );
        };
        for x in 0..w {
            let (x0, x1) = (x as f32, x as f32 + 1.0);
            wall(vec3(x0, c(x, 0), 0.0), vec3(x1, c(x + 1, 0), 0.0), -Vec3::Z);
            wall(
                vec3(x0, c(x, h), h as f32),
                vec3(x1, c(x + 1, h), h as f32),
                Vec3::Z,
            );
        }
        for z in 0..h {
            let (z0, z1) = (z as f32, z as f32 + 1.0);
            wall(vec3(0.0, c(0, z), z0), vec3(0.0, c(0, z + 1), z1), -Vec3::X);
            wall(
                vec3(w as f32, c(w, z), z0),
                vec3(w as f32, c(w, z + 1), z1),
                Vec3::X,
            );
        }
        // Trees (thinning as wood is cut) and boulders on stone.
        for z in 0..h {
            for x in 0..w {
                let i = z * w + x;
                let t = &sim.world.tiles[i];
                if !t.is_land() {
                    continue;
                }
                let hs = hash((i as u64) ^ (sim.world.seed << 17));
                if t.wood_capacity > 0.0 {
                    let ratio = (t.wood / t.wood_capacity).clamp(0.0, 1.0);
                    let n = match t.biome {
                        Biome::Forest => (ratio * 3.0).round() as u32,
                        _ if ratio > 0.4 && hs % 5 == 0 => 1,
                        _ => 0,
                    };
                    for k in 0..n {
                        let hk = hash(hs ^ u64::from(k + 1));
                        let px = x as f32 + 0.2 + 0.6 * unit(hk);
                        let pz = z as f32 + 0.2 + 0.6 * unit(hash(hk));
                        let s = 0.8 + 0.5 * unit(hash(hk ^ 7));
                        let g = 0.75 + 0.35 * unit(hash(hk ^ 11));
                        let green = if t.biome == Biome::Hills || t.biome == Biome::Mountains {
                            Color::new(0.13 * g, 0.30 * g, 0.20 * g, 1.0)
                        } else {
                            Color::new(0.16 * g, 0.40 * g, 0.16 * g, 1.0)
                        };
                        let y = self.height_at(px, pz);
                        let foot = vec3(px, y, pz);
                        land.cuboid(
                            foot,
                            vec3(0.08, 0.25 * s, 0.08),
                            Color::from_rgba(92, 64, 40, 255),
                        );
                        land.cone(
                            foot + vec3(0.0, 0.18 * s, 0.0),
                            0.32 * s,
                            0.75 * s,
                            6,
                            unit(hk),
                            green,
                        );
                        if hk % 2 == 0 {
                            land.cone(
                                foot + vec3(0.0, 0.55 * s, 0.0),
                                0.22 * s,
                                0.55 * s,
                                6,
                                unit(hk) + 0.5,
                                green,
                            );
                        }
                    }
                }
                if t.stone > 0.0 && hs % 3 == 0 {
                    let px = x as f32 + 0.3 + 0.4 * unit(hash(hs ^ 3));
                    let pz = z as f32 + 0.3 + 0.4 * unit(hash(hs ^ 5));
                    let s = 0.18 + 0.14 * unit(hash(hs ^ 9));
                    land.cuboid(
                        vec3(px, self.height_at(px, pz) - 0.05, pz),
                        vec3(s * 1.3, s, s),
                        Color::from_rgba(150, 148, 146, 255),
                    );
                }
            }
        }
        // Sea: a translucent sheet at sea level, with glass walls at the edge.
        let mut water = MeshBuilder::default();
        let sea = Color::new(0.16, 0.42, 0.66, 0.62);
        let (fw, fh) = (w as f32, h as f32);
        water.face_lit(
            &[
                vec3(0.0, 0.0, 0.0),
                vec3(fw, 0.0, 0.0),
                vec3(fw, 0.0, fh),
                vec3(0.0, 0.0, fh),
            ],
            sea,
        );
        let side = Color::new(0.10, 0.30, 0.52, 0.55);
        let mut glass = |a: Vec3, b: Vec3| {
            if a.y < 0.0 || b.y < 0.0 {
                water.face_lit(
                    &[
                        vec3(a.x, a.y.min(0.0), a.z),
                        vec3(b.x, b.y.min(0.0), b.z),
                        vec3(b.x, 0.0, b.z),
                        vec3(a.x, 0.0, a.z),
                    ],
                    side,
                );
            }
        };
        for x in 0..w {
            let (x0, x1) = (x as f32, x as f32 + 1.0);
            glass(vec3(x0, c(x, 0), 0.0), vec3(x1, c(x + 1, 0), 0.0));
            glass(vec3(x0, c(x, h), fh), vec3(x1, c(x + 1, h), fh));
        }
        for z in 0..h {
            let (z0, z1) = (z as f32, z as f32 + 1.0);
            glass(vec3(0.0, c(0, z), z0), vec3(0.0, c(0, z + 1), z1));
            glass(vec3(fw, c(w, z), z0), vec3(fw, c(w, z + 1), z1));
        }
        self.land = land;
        self.water = water;
    }

    pub fn camera(&self, area: Rect) -> Camera3D {
        let dir = vec3(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        );
        // The GL viewport origin is the bottom-left of the window.
        let gl_y = screen_height() - (area.y + area.h);
        Camera3D {
            position: self.target + dir * self.dist,
            target: self.target,
            up: Vec3::Y,
            fovy: 0.75,
            aspect: Some(area.w / area.h),
            viewport: Some((area.x as i32, gl_y as i32, area.w as i32, area.h as i32)),
            z_near: 0.5,
            z_far: 4000.0,
            ..Default::default()
        }
    }

    /// Screen position of a world point, or `None` if it is behind the camera.
    pub fn project(cam: &Camera3D, area: Rect, p: Vec3) -> Option<Vec2> {
        let clip = cam.matrix() * p.extend(1.0);
        if clip.w <= 1e-4 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(vec2(
            area.x + (ndc.x + 1.0) * 0.5 * area.w,
            area.y + (1.0 - ndc.y) * 0.5 * area.h,
        ))
    }

    /// Orbit (right/middle drag, Q/E), zoom (wheel), pan (WASD/arrows).
    pub fn input(&mut self, area: Rect) {
        let dt = get_frame_time().min(0.1);
        let mouse = Vec2::from(mouse_position());
        if is_mouse_button_down(MouseButton::Right) || is_mouse_button_down(MouseButton::Middle) {
            if let Some(prev) = self.last_drag {
                let d = mouse - prev;
                self.yaw -= d.x * 0.008;
                self.pitch = (self.pitch + d.y * 0.006).clamp(0.12, 1.5);
            }
            self.last_drag = Some(mouse);
        } else {
            self.last_drag = None;
        }
        if is_key_down(KeyCode::Q) {
            self.yaw += 1.2 * dt;
        }
        if is_key_down(KeyCode::E) {
            self.yaw -= 1.2 * dt;
        }
        if self.auto_orbit {
            self.yaw += 0.12 * dt;
        }
        let (_, wheel) = mouse_wheel();
        if wheel != 0.0 && area.contains(mouse) {
            let f = if wheel > 0.0 { 0.88 } else { 1.0 / 0.88 };
            self.dist = (self.dist * f).clamp(6.0, 400.0);
        }
        let speed = self.dist * 0.9 * dt;
        let fwd = -vec3(self.yaw.sin(), 0.0, self.yaw.cos());
        let right = vec3(-fwd.z, 0.0, fwd.x);
        let mut mv = Vec3::ZERO;
        if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up) {
            mv += fwd;
        }
        if is_key_down(KeyCode::Down) {
            mv -= fwd;
        }
        if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
            mv -= right;
        }
        if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
            mv += right;
        }
        self.target += mv * speed;
        self.target.x = self.target.x.clamp(0.0, self.w as f32);
        self.target.z = self.target.z.clamp(0.0, self.h as f32);
        self.target.y +=
            (self.height_at(self.target.x, self.target.z).max(0.0) - self.target.y) * 0.1;
    }

    pub fn draw_static(&self) {
        self.land.draw();
    }

    pub fn draw_water(&self) {
        self.water.draw();
    }
}
