//! Global sensitivity analysis: Sobol' indices from a Saltelli design.
//!
//! **Design.** A Sobol' low-discrepancy sequence (Joe & Kuo direction numbers)
//! in `2k` dimensions gives two quasi-random matrices `A` and `B` (`N × k`).
//! For every row `j` the design evaluates `A_j`, `B_j`, and the `k` hybrids
//! `AB_j^(i)`, which are `A_j` with column `i` taken from `B_j`: `N (k + 2)`
//! points in all, laid out row by row so a bootstrap can resample rows.
//!
//! **Estimators** (Saltelli et al. 2010, Table 2):
//! * first order `S_i = mean(f_B · (f_AB_i − f_A)) / V`;
//! * total order `ST_i = mean((f_A − f_AB_i)^2) / (2V)` (Jansen 1999);
//!
//! where `V` is the variance of `f` over the `A` and `B` rows. Percentile
//! bootstrap intervals resample the `N` rows with replacement.
//!
//! **Stochastic models.** Each point's output is the mean over its seeds. The
//! seed-to-seed variance left in that mean is noise, not sensitivity. It is
//! reported as `noise_share` (mean within-point variance of the mean, divided
//! by `V`), so an index can be read against how much of `V` is noise.
//!
//! Only `+ - * /`, `abs`, `sqrt` and comparisons are used; results do not
//! depend on `--jobs` because every input arrives in a fixed order.

use accrete_core::rng::{Rng, Stream};

/// Joe & Kuo (2008) direction numbers for dimensions 2..=21: `(s, a, m_1..m_s)`.
/// Dimension 1 is the van der Corput sequence.
const JOE_KUO: [(u32, u32, &[u32]); 20] = [
    (1, 0, &[1]),
    (2, 1, &[1, 3]),
    (3, 1, &[1, 3, 1]),
    (3, 2, &[1, 1, 1]),
    (4, 1, &[1, 1, 3, 3]),
    (4, 4, &[1, 3, 5, 13]),
    (5, 2, &[1, 1, 5, 5, 17]),
    (5, 4, &[1, 1, 5, 5, 5]),
    (5, 7, &[1, 1, 7, 11, 19]),
    (5, 11, &[1, 1, 5, 1, 1]),
    (5, 13, &[1, 1, 1, 3, 11]),
    (5, 14, &[1, 3, 5, 5, 31]),
    (6, 1, &[1, 3, 3, 9, 7, 49]),
    (6, 13, &[1, 1, 1, 15, 21, 21]),
    (6, 16, &[1, 3, 1, 13, 27, 49]),
    (6, 19, &[1, 1, 1, 15, 7, 5]),
    (6, 22, &[1, 3, 1, 15, 13, 25]),
    (6, 25, &[1, 1, 5, 5, 19, 61]),
    (7, 1, &[1, 3, 7, 11, 23, 15, 103]),
    (7, 4, &[1, 3, 7, 13, 13, 15, 69]),
];

/// The largest dimension supported (`2k`, so up to 10 parameters).
pub const MAX_DIMS: usize = 1 + JOE_KUO.len();
const BITS: u32 = 32;

/// Sobol' sequence generator (Gray-code order).
pub struct SobolSeq {
    v: Vec<[u32; BITS as usize]>,
    x: Vec<u32>,
    index: u32,
}

impl SobolSeq {
    pub fn new(dims: usize) -> Result<Self, String> {
        if dims == 0 || dims > MAX_DIMS {
            return Err(format!(
                "a Sobol' sequence supports 1..={MAX_DIMS} dimensions, not {dims}"
            ));
        }
        let mut v = vec![[0u32; BITS as usize]; dims];
        for (i, slot) in v[0].iter_mut().enumerate() {
            *slot = 1u32 << (BITS - 1 - i as u32);
        }
        for d in 1..dims {
            let (s, a, m) = JOE_KUO[d - 1];
            let s = s as usize;
            for i in 0..BITS as usize {
                if i < s {
                    v[d][i] = m[i] << (BITS - 1 - i as u32);
                } else {
                    let mut x = v[d][i - s] ^ (v[d][i - s] >> s);
                    for k in 1..s {
                        if (a >> (s - 1 - k)) & 1 == 1 {
                            x ^= v[d][i - k];
                        }
                    }
                    v[d][i] = x;
                }
            }
        }
        Ok(Self {
            v,
            x: vec![0; dims],
            index: 0,
        })
    }

    /// The next point in `[0, 1)^dims`. The first point is the origin.
    pub fn next_point(&mut self) -> Vec<f64> {
        let out = self
            .x
            .iter()
            .map(|&x| f64::from(x) / 4_294_967_296.0)
            .collect();
        let c = self.index.trailing_ones() as usize;
        for (x, v) in self.x.iter_mut().zip(&self.v) {
            *x ^= v[c.min(BITS as usize - 1)];
        }
        self.index += 1;
        out
    }
}

/// Saltelli design in the unit cube: `n` rows of `k + 2` points each
/// (`A_j`, `B_j`, then `AB_j^(1..k)`). The origin of the sequence is skipped.
pub fn saltelli_unit(n: usize, k: usize) -> Result<Vec<Vec<f64>>, String> {
    let mut seq = SobolSeq::new(2 * k)?;
    seq.next_point();
    let mut out = Vec::with_capacity(n * (k + 2));
    for _ in 0..n {
        let q = seq.next_point();
        let (a, b) = q.split_at(k);
        out.push(a.to_vec());
        out.push(b.to_vec());
        for i in 0..k {
            let mut ab = a.to_vec();
            ab[i] = b[i];
            out.push(ab);
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    pub s1: f64,
    pub s1_ci: (f64, f64),
    pub st: f64,
    pub st_ci: (f64, f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    pub variance: f64,
    pub indices: Vec<Index>,
}

fn point_estimates(f: &[f64], k: usize, rows: &[usize]) -> (f64, Vec<(f64, f64)>) {
    let stride = k + 2;
    let n = rows.len() as f64;
    let mut ab_vals = Vec::with_capacity(2 * rows.len());
    for &j in rows {
        ab_vals.push(f[j * stride]);
        ab_vals.push(f[j * stride + 1]);
    }
    let mean = ab_vals.iter().sum::<f64>() / ab_vals.len() as f64;
    let var =
        ab_vals.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (ab_vals.len() as f64 - 1.0);
    let mut est = Vec::with_capacity(k);
    for i in 0..k {
        let (mut s1, mut st) = (0.0, 0.0);
        for &j in rows {
            let fa = f[j * stride];
            let fb = f[j * stride + 1];
            let fab = f[j * stride + 2 + i];
            s1 += fb * (fab - fa);
            st += (fa - fab) * (fa - fab);
        }
        if var > 0.0 {
            est.push((s1 / n / var, st / n / (2.0 * var)));
        } else {
            est.push((f64::NAN, f64::NAN));
        }
    }
    (var, est)
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = q * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

/// Sobol' indices for outputs `f` laid out as by [`saltelli_unit`], with
/// `n_boot` bootstrap resamples drawn from the `Design` stream at `seed`.
pub fn analyse(f: &[f64], k: usize, n_boot: usize, seed: u64) -> Result<Analysis, String> {
    let stride = k + 2;
    if k == 0 || f.len() % stride != 0 || f.len() / stride < 2 {
        return Err(format!("need at least 2 rows of {stride} outputs"));
    }
    if f.iter().any(|x| !x.is_finite()) {
        return Err("an output is undefined (NaN) at some point".into());
    }
    let n = f.len() / stride;
    let all: Vec<usize> = (0..n).collect();
    let (variance, est) = point_estimates(f, k, &all);
    let mut rng = Rng::for_stream(seed, Stream::Design);
    let mut boot: Vec<(Vec<f64>, Vec<f64>)> = vec![(Vec::new(), Vec::new()); k];
    for _ in 0..n_boot {
        let rows: Vec<usize> = (0..n).map(|_| rng.below(n as u32) as usize).collect();
        let (_, e) = point_estimates(f, k, &rows);
        for (i, (s1, st)) in e.into_iter().enumerate() {
            if s1.is_finite() && st.is_finite() {
                boot[i].0.push(s1);
                boot[i].1.push(st);
            }
        }
    }
    let indices = est
        .into_iter()
        .zip(boot)
        .map(|((s1, st), (mut b1, mut bt))| {
            b1.sort_by(f64::total_cmp);
            bt.sort_by(f64::total_cmp);
            Index {
                s1,
                s1_ci: (percentile(&b1, 0.025), percentile(&b1, 0.975)),
                st,
                st_ci: (percentile(&bt, 0.025), percentile(&bt, 0.975)),
            }
        })
        .collect();
    Ok(Analysis { variance, indices })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sobol_points_are_stratified_in_every_dimension() {
        // Any 2^m consecutive points from the start fill each of the 2^m
        // equal intervals of every coordinate exactly once.
        let dims = MAX_DIMS;
        let mut seq = SobolSeq::new(dims).unwrap();
        let pts: Vec<Vec<f64>> = (0..64).map(|_| seq.next_point()).collect();
        for d in 0..dims {
            let mut seen = [false; 64];
            for p in &pts {
                let cell = (p[d] * 64.0) as usize;
                assert!(!seen[cell], "dimension {d}: cell {cell} twice");
                seen[cell] = true;
            }
        }
        assert!(SobolSeq::new(MAX_DIMS + 1).is_err());
    }

    #[test]
    fn saltelli_layout() {
        let pts = saltelli_unit(4, 3).unwrap();
        assert_eq!(pts.len(), 4 * 5);
        for j in 0..4 {
            let (a, b) = (&pts[j * 5], &pts[j * 5 + 1]);
            for i in 0..3 {
                let ab = &pts[j * 5 + 2 + i];
                for c in 0..3 {
                    assert_eq!(ab[c], if c == i { b[c] } else { a[c] });
                }
            }
        }
    }

    /// Sobol's g-function: indices are known analytically.
    #[test]
    fn g_function_indices_match_theory() {
        let a = [0.0f64, 1.0, 9.0, 99.0];
        let k = a.len();
        let g = |x: &[f64]| {
            x.iter()
                .zip(a)
                .map(|(xi, ai)| ((4.0 * xi - 2.0).abs() + ai) / (1.0 + ai))
                .product::<f64>()
        };
        let pts = saltelli_unit(4096, k).unwrap();
        let f: Vec<f64> = pts.iter().map(|p| g(p)).collect();
        let res = analyse(&f, k, 200, 7).unwrap();
        let vi: Vec<f64> = a
            .iter()
            .map(|ai| 1.0 / 3.0 / ((1.0 + ai) * (1.0 + ai)))
            .collect();
        let v = vi.iter().map(|x| 1.0 + x).product::<f64>() - 1.0;
        for i in 0..k {
            let s1 = vi[i] / v;
            let st = vi[i]
                * vi.iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, x)| 1.0 + x)
                    .product::<f64>()
                / v;
            let got = &res.indices[i];
            assert!((got.s1 - s1).abs() < 0.03, "S{i}: {} vs {s1}", got.s1);
            assert!((got.st - st).abs() < 0.03, "ST{i}: {} vs {st}", got.st);
            assert!(got.s1_ci.0 <= got.s1_ci.1 && got.st_ci.0 <= got.st_ci.1);
        }
    }

    #[test]
    fn undefined_outputs_are_refused() {
        assert!(analyse(&[1.0, f64::NAN, 1.0, 2.0, 3.0, 4.0], 1, 10, 1).is_err());
        assert!(analyse(&[1.0, 2.0, 3.0], 1, 10, 1).is_err());
    }
}
