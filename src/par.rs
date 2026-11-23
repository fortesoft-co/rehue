//! Per-pixel work across threads, with results that don't depend on the thread count.
//!
//! Work is cut into contiguous runs of whole units (a pixel, a row, a
//! block row), one run per thread, and the results come back in order.
//! Each unit's output depends only on its own input, so the bytes are
//! the same however many threads run; anything summed across runs (pixel
//! counts, histograms) is an integer.  `REHUE_THREADS` pins the thread
//! count (1 runs everything on the calling thread).  Work started from
//! inside a run stays on that run's thread, so nested calls (an icon's
//! dither inside the icon pool) don't multiply the threads.

use std::cell::Cell;

thread_local! {
    /// Set on the threads `chunks` and `chunks_mut` start.
    static IN_RUN: Cell<bool> = const { Cell::new(false) };
}

/// `f` on a run thread: marks it, so nested work stays on it.
fn in_run<R>(f: impl FnOnce() -> R) -> R {
    IN_RUN.with(|r| r.set(true));
    f()
}

/// How many threads to cut work into: `REHUE_THREADS` when set to a
/// positive number, else the machine's available parallelism.
pub fn threads() -> usize {
    std::env::var("REHUE_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

/// How many items each run takes: whole `unit`s, spread over the threads
/// (everything in one run inside another run).
fn run_len(len: usize, unit: usize) -> usize {
    if IN_RUN.with(Cell::get) {
        return len;
    }
    let unit = unit.max(1);
    let units = len.div_ceil(unit).max(1);
    units.div_ceil(threads()) * unit
}

/// `f(start, run)` over `data` cut into runs of whole `unit`s (the last
/// may be short), in parallel; `start` is the run's offset into `data`.
/// Results are in run order.  `unit` is also the finest split: a cheap
/// per-item job should pass a unit of a few thousand items.
pub fn chunks<T, R, F>(data: &[T], unit: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(usize, &[T]) -> R + Sync,
{
    if data.is_empty() {
        return Vec::new();
    }
    let len = run_len(data.len(), unit);
    if len >= data.len() {
        return vec![f(0, data)];
    }
    std::thread::scope(|scope| {
        let f = &f;
        let handles: Vec<_> = data
            .chunks(len)
            .enumerate()
            .map(|(k, run)| scope.spawn(move || in_run(|| f(k * len, run))))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a worker thread panicked"))
            .collect()
    })
}

/// [`chunks`] over a buffer each run writes into.
pub fn chunks_mut<T, R, F>(data: &mut [T], unit: usize, f: F) -> Vec<R>
where
    T: Send,
    R: Send,
    F: Fn(usize, &mut [T]) -> R + Sync,
{
    if data.is_empty() {
        return Vec::new();
    }
    let len = run_len(data.len(), unit);
    if len >= data.len() {
        return vec![f(0, data)];
    }
    std::thread::scope(|scope| {
        let f = &f;
        let handles: Vec<_> = data
            .chunks_mut(len)
            .enumerate()
            .map(|(k, run)| scope.spawn(move || in_run(|| f(k * len, run))))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a worker thread panicked"))
            .collect()
    })
}

/// Element-wise sum of equal-length count vectors (the per-run counts
/// `chunks` and `chunks_mut` hand back).
pub fn sum_counts(parts: Vec<Vec<u64>>, len: usize) -> Vec<u64> {
    let mut total = vec![0u64; len];
    for part in parts {
        for (t, n) in total.iter_mut().zip(part) {
            *t += n;
        }
    }
    total
}
