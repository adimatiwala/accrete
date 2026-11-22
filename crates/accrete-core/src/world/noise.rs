//! Seeded 2D gradient noise and fractal sums, implemented with only
//! exactly-specified float operations (`+ - * /`, `floor`, comparisons, `as`).
//!
//! Lattice gradients are chosen by hashing the integer lattice coordinates with
//! the seed, so there is no permutation table and no hidden state.

use crate::hash::mix64;

const INV_SQRT2: f32 = 0.707_106_77;

/// Eight unit gradient directions (no trigonometry needed).
const GRADS: [(f32, f32); 8] = [
    (1.0, 0.0),
    (INV_SQRT2, INV_SQRT2),
    (0.0, 1.0),
    (-INV_SQRT2, INV_SQRT2),
    (-1.0, 0.0),
    (-INV_SQRT2, -INV_SQRT2),
    (0.0, -1.0),
    (INV_SQRT2, -INV_SQRT2),
];

#[inline]
fn lattice_hash(seed: u64, ix: i32, iy: i32) -> u64 {
    mix64(seed ^ mix64((ix as u32 as u64) | ((iy as u32 as u64) << 32)))
}

#[inline]
fn grad_dot(seed: u64, ix: i32, iy: i32, dx: f32, dy: f32) -> f32 {
    let (gx, gy) = GRADS[(lattice_hash(seed, ix, iy) & 7) as usize];
    gx * dx + gy * dy
}

/// Quintic fade 6t^5 - 15t^4 + 10t^3 (C2-continuous).
#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Gradient noise in approximately `[-0.71, 0.71]`, zero at lattice points.
pub fn gradient(seed: u64, x: f32, y: f32) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let ix = x0 as i32;
    let iy = y0 as i32;
    let fx = x - x0;
    let fy = y - y0;
    let n00 = grad_dot(seed, ix, iy, fx, fy);
    let n10 = grad_dot(seed, ix + 1, iy, fx - 1.0, fy);
    let n01 = grad_dot(seed, ix, iy + 1, fx, fy - 1.0);
    let n11 = grad_dot(seed, ix + 1, iy + 1, fx - 1.0, fy - 1.0);
    let u = fade(fx);
    let v = fade(fy);
    lerp(lerp(n00, n10, u), lerp(n01, n11, u), v)
}

/// Fractal Brownian motion: `octaves` layers of gradient noise, each at twice
/// the frequency and half the amplitude. Output is normalized by the total
/// amplitude, so it stays roughly within `[-0.71, 0.71]`.
pub fn fbm(seed: u64, x: f32, y: f32, octaves: u32) -> f32 {
    let mut sum = 0.0f32;
    let mut amp = 1.0f32;
    let mut freq = 1.0f32;
    let mut norm = 0.0f32;
    for o in 0..octaves.max(1) {
        let s = mix64(seed.wrapping_add(u64::from(o).wrapping_mul(0x9e37_79b9)));
        sum += amp * gradient(s, x * freq, y * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

/// Rescale `values` in place to `[0, 1]` by their min and max (fixed order).
pub fn normalize(values: &mut [f32]) {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for &v in values.iter() {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let span = hi - lo;
    if span <= 0.0 {
        values.iter_mut().for_each(|v| *v = 0.5);
        return;
    }
    for v in values.iter_mut() {
        *v = ((*v - lo) / span).clamp(0.0, 1.0);
    }
}

/// The value below which `fraction` of `values` lie (sorted with `total_cmp`).
pub fn quantile(values: &[f32], fraction: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f32> = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let k = ((sorted.len() as f32 * fraction) as usize).min(sorted.len() - 1);
    sorted[k]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_bounded() {
        for i in 0..2000 {
            let x = i as f32 * 0.37;
            let y = i as f32 * 0.11;
            let a = fbm(7, x, y, 5);
            assert_eq!(a.to_bits(), fbm(7, x, y, 5).to_bits());
            assert!(a.is_finite() && a.abs() <= 0.75, "{a}");
        }
    }

    #[test]
    fn zero_at_lattice_points() {
        assert_eq!(gradient(3, 5.0, -2.0), 0.0);
    }

    #[test]
    fn continuous() {
        let a = gradient(11, 3.5, 4.25);
        let b = gradient(11, 3.5001, 4.25);
        assert!((a - b).abs() < 1e-3);
    }

    #[test]
    fn seeds_differ() {
        let a: Vec<u32> = (0..50)
            .map(|i| fbm(1, i as f32 * 0.3, 0.7, 4).to_bits())
            .collect();
        let b: Vec<u32> = (0..50)
            .map(|i| fbm(2, i as f32 * 0.3, 0.7, 4).to_bits())
            .collect();
        assert_ne!(a, b);
    }
}
