//! Deterministic data parallelism inside one tick (A.32).
//!
//! The only parallel step is scoring proposals, which reads a frozen world.
//! Work is split into contiguous index ranges and the results are
//! concatenated in index order, so the output is identical to a sequential
//! map for any number of threads. Nothing here draws random numbers or
//! touches shared state.

use std::sync::atomic::{AtomicUsize, Ordering};

static DEFAULT_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Set the thread count used by simulations created from now on (0 = auto).
/// Front ends that already run several simulations at once set this to 1.
pub fn set_default_threads(n: usize) {
    DEFAULT_THREADS.store(n, Ordering::Relaxed);
}

/// Threads for a new simulation: the value set by [`set_default_threads`],
/// else `ACCRETE_THREADS`, else the machine's parallelism capped at 8.
pub fn default_threads() -> usize {
    match DEFAULT_THREADS.load(Ordering::Relaxed) {
        0 => std::env::var("ACCRETE_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get().min(8)))
            .max(1),
        n => n,
    }
}

/// Below this many items, the cost of spawning threads outweighs the work.
const MIN_PER_THREAD: usize = 24;

/// `(0..n).map(f).collect()`, computed on up to `threads` threads.
pub fn map_indexed<T: Send>(n: usize, threads: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = threads.min(n / MIN_PER_THREAD).max(1);
    if threads == 1 {
        return (0..n).map(f).collect();
    }
    let chunk = n.div_ceil(threads);
    let f = &f;
    std::thread::scope(|s| {
        let handles: Vec<_> = (1..threads)
            .map(|t| {
                s.spawn(move || {
                    (t * chunk..((t + 1) * chunk).min(n))
                        .map(f)
                        .collect::<Vec<T>>()
                })
            })
            .collect();
        // The calling thread takes the first chunk.
        let mut out: Vec<T> = (0..chunk.min(n)).map(f).collect();
        for h in handles {
            out.extend(h.join().expect("proposal worker panicked"));
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_sequential_for_any_thread_count() {
        let seq: Vec<u64> = (0..1000).map(|i| (i as u64) * 7 + 1).collect();
        for t in [1, 2, 3, 7, 16, 64] {
            assert_eq!(map_indexed(1000, t, |i| (i as u64) * 7 + 1), seq);
        }
        assert!(map_indexed(0, 8, |i| i).is_empty());
    }
}
