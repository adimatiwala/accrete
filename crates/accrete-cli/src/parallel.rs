//! Running independent simulations in parallel (one per seed).
//!
//! Each simulation is single-threaded and deterministic; running *separate*
//! simulations on separate threads is allowed by the determinism rules
//! (CLAUDE.md). Results come back in input order, so every output file is
//! byte-identical whatever the number of jobs.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// `items.iter().map(f).collect()`, using up to `jobs` threads.
///
/// If `f` panics for an item, no further items are started, the threads
/// already running finish their current item, and the panic is re-raised
/// with the index of the item that failed (the first one, by index, if
/// several did).
pub fn map_ordered<T: Sync, R: Send>(
    items: &[T],
    jobs: usize,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    if jobs <= 1 || items.len() <= 1 {
        return items.iter().map(&f).collect();
    }
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let failure: Mutex<Option<(usize, String)>> = Mutex::new(None);
    let slots: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..jobs.min(items.len()) {
            s.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= items.len() {
                        break;
                    }
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&items[i]))) {
                        Ok(r) => *slots[i].lock().expect("result slot") = Some(r),
                        Err(payload) => {
                            stop.store(true, Ordering::Relaxed);
                            let msg = payload
                                .downcast_ref::<String>()
                                .cloned()
                                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                                .unwrap_or_else(|| "panic".into());
                            let mut f = failure.lock().expect("failure slot");
                            if f.as_ref().is_none_or(|(j, _)| i < *j) {
                                *f = Some((i, msg));
                            }
                        }
                    }
                }
            });
        }
    });
    if let Some((i, msg)) = failure.into_inner().expect("failure slot") {
        panic!("item {i} failed: {msg}");
    }
    slots
        .into_iter()
        .map(|m| {
            m.into_inner()
                .expect("result slot")
                .expect("every item ran")
        })
        .collect()
}

/// Two-sided 95 % critical value of Student's t with `df` degrees of freedom.
pub fn t95(df: usize) -> f64 {
    const T: [f64; 30] = [
        12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160,
        2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056,
        2.052, 2.048, 2.045, 2.042,
    ];
    match df {
        0 => f64::NAN,
        1..=30 => T[df - 1],
        // Cornish-Fisher expansion around the normal quantile; within 0.001
        // of the exact value for df > 30 (e.g. 2.040 at df 31).
        _ => {
            let z = 1.959_963_984_540_054_f64;
            let (z3, d) = (z * z * z, df as f64);
            let z5 = z3 * z * z;
            z + (z3 + z) / (4.0 * d) + (5.0 * z5 + 16.0 * z3 + 3.0 * z) / (96.0 * d * d)
        }
    }
}

/// Mean, sample standard deviation, and the half-width of the 95 %
/// confidence interval of the mean (NaN where undefined, e.g. n = 1).
pub fn mean_sd_ci(xs: &[f64]) -> (f64, f64, f64) {
    let n = xs.len();
    if n == 0 {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    if n == 1 {
        return (mean, f64::NAN, f64::NAN);
    }
    let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1) as f64;
    let sd = var.sqrt();
    (mean, sd, t95(n - 1) * sd / (n as f64).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_ordered_keeps_order_for_any_job_count() {
        let items: Vec<u64> = (0..37).collect();
        let want: Vec<u64> = items.iter().map(|x| x * x).collect();
        for jobs in [1, 2, 4, 16, 64] {
            assert_eq!(map_ordered(&items, jobs, |x| x * x), want, "jobs {jobs}");
        }
        assert!(map_ordered(&Vec::<u64>::new(), 4, |x| *x).is_empty());
    }

    #[test]
    fn confidence_intervals() {
        let (m, sd, ci) = mean_sd_ci(&[1.0, 2.0, 3.0, 4.0]);
        assert!((m - 2.5).abs() < 1e-12);
        assert!((sd - 1.2909944487).abs() < 1e-9);
        assert!((ci - 3.182 * sd / 2.0).abs() < 1e-9);
        let (m, sd, ci) = mean_sd_ci(&[5.0]);
        assert_eq!(m, 5.0);
        assert!(sd.is_nan() && ci.is_nan());
        assert!(t95(0).is_nan());
        // Reference values of Student's t (two-sided 95 %).
        for (df, exact) in [
            (31, 2.0395),
            (40, 2.0211),
            (60, 2.0003),
            (120, 1.9799),
            (1000, 1.9623),
        ] {
            assert!(
                (t95(df) - exact).abs() < 1e-3,
                "df {df}: {} vs {exact}",
                t95(df)
            );
        }
    }

    #[test]
    fn a_failing_item_stops_the_rest_and_names_the_item() {
        let started = AtomicUsize::new(0);
        let items: Vec<u32> = (0..200).collect();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            map_ordered(&items, 4, |&x| {
                started.fetch_add(1, Ordering::Relaxed);
                if x == 3 {
                    panic!("boom at {x}");
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
                x
            })
        }));
        let err = result.expect_err("the panic propagates");
        let msg = err.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(
            msg.contains("item 3 failed") && msg.contains("boom at 3"),
            "{msg}"
        );
        assert!(started.load(Ordering::Relaxed) < 50, "stopped early");
    }
}
