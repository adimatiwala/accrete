//! Minimal RGB canvas for headless rendering to PNG.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use accrete_core::{Config, Pos, World};
use accrete_viz::{Overlay, Rgb, render_tiles};

pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub pixels: Vec<u8>,
}

impl Canvas {
    pub fn from_world(world: &World, config: &Config, overlay: Overlay, scale: u32) -> Self {
        let scale = scale.max(1);
        let (tw, th) = (world.dims.width, world.dims.height);
        let (width, height) = (tw * scale, th * scale);
        let colors = render_tiles(world, config, overlay);
        let mut pixels = vec![0u8; (width * height * 3) as usize];
        for y in 0..height {
            for x in 0..width {
                let c = colors[((y / scale) * tw + x / scale) as usize];
                let o = ((y * width + x) * 3) as usize;
                pixels[o..o + 3].copy_from_slice(&c);
            }
        }
        Self {
            width,
            height,
            scale,
            pixels,
        }
    }

    pub fn put(&mut self, x: i32, y: i32, c: Rgb) {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return;
        }
        let o = ((y as u32 * self.width + x as u32) * 3) as usize;
        self.pixels[o..o + 3].copy_from_slice(&c);
    }

    /// Filled square of side `size` pixels centred on a tile.
    pub fn dot(&mut self, p: Pos, c: Rgb, size: u32) {
        let s = self.scale as i32;
        let cx = p.x * s + s / 2;
        let cy = p.y * s + s / 2;
        let r = size as i32 / 2;
        for dy in -r..=r {
            for dx in -r..=r {
                self.put(cx + dx, cy + dy, c);
            }
        }
    }

    /// Hollow square outline around a tile.
    pub fn ring(&mut self, p: Pos, c: Rgb) {
        let s = self.scale as i32;
        for k in 0..s {
            self.put(p.x * s + k, p.y * s, c);
            self.put(p.x * s + k, p.y * s + s - 1, c);
            self.put(p.x * s, p.y * s + k, c);
            self.put(p.x * s + s - 1, p.y * s + k, c);
        }
    }

    /// Circle outline of radius `r` tiles around a tile centre. Rendering only:
    /// output never feeds back into the simulation, so trigonometry is fine.
    #[allow(clippy::disallowed_methods)]
    pub fn circle(&mut self, p: Pos, r: f32, c: Rgb) {
        let s = self.scale as f32;
        let cx = p.x as f32 * s + s / 2.0;
        let cy = p.y as f32 * s + s / 2.0;
        let rad = r * s;
        let steps = (rad * 8.0).max(16.0) as i32;
        for k in 0..steps {
            let a = k as f32 / steps as f32 * std::f32::consts::TAU;
            self.put((cx + rad * a.cos()) as i32, (cy + rad * a.sin()) as i32, c);
        }
    }

    /// Lay canvases of equal size side by side with 4 px gutters.
    pub fn strip(parts: &[Canvas]) -> Canvas {
        let (pw, ph, gap) = (parts[0].width, parts[0].height, 4u32);
        let n = parts.len() as u32;
        let width = n * pw + (n - 1) * gap;
        let mut out = Canvas {
            width,
            height: ph,
            scale: parts[0].scale,
            pixels: vec![255u8; (width * ph * 3) as usize],
        };
        for (k, part) in parts.iter().enumerate() {
            let ox = k as u32 * (pw + gap);
            for y in 0..ph {
                let src = (y * pw * 3) as usize;
                let dst = ((y * width + ox) * 3) as usize;
                out.pixels[dst..dst + (pw * 3) as usize]
                    .copy_from_slice(&part.pixels[src..src + (pw * 3) as usize]);
            }
        }
        out
    }

    /// Tile canvases of equal size into a near-square grid with 4 px gutters.
    pub fn montage(parts: &[Canvas]) -> Canvas {
        let cols = (1..).find(|c| c * c >= parts.len()).unwrap_or(1) as u32;
        let rows = (parts.len() as u32).div_ceil(cols);
        let (pw, ph, gap) = (parts[0].width, parts[0].height, 4u32);
        let width = cols * pw + (cols - 1) * gap;
        let height = rows * ph + (rows - 1) * gap;
        let mut out = Canvas {
            width,
            height,
            scale: parts[0].scale,
            pixels: vec![255u8; (width * height * 3) as usize],
        };
        for (k, part) in parts.iter().enumerate() {
            let ox = (k as u32 % cols) * (pw + gap);
            let oy = (k as u32 / cols) * (ph + gap);
            for y in 0..ph {
                let src = (y * pw * 3) as usize;
                let dst = (((oy + y) * width + ox) * 3) as usize;
                out.pixels[dst..dst + (pw * 3) as usize]
                    .copy_from_slice(&part.pixels[src..src + (pw * 3) as usize]);
            }
        }
        out
    }

    pub fn save(&self, path: &Path) {
        let file = File::create(path).unwrap_or_else(|e| panic!("create {}: {e}", path.display()));
        let mut enc = png::Encoder::new(BufWriter::new(file), self.width, self.height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(&self.pixels).expect("png data");
    }
}
