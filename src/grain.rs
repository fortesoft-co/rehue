//! Fine grain: a small, deterministic jitter in lightness, after the grade.
//!
//! Each pixel moves up or down by a few levels, the same amount on all
//! three channels (monochrome grain, like film), drawn from a
//! triangular distribution so most pixels move a little and few move
//! the full amount.  The jitter comes from a hash of the pixel's
//! position and the frame number: the same every run and on any thread
//! count, and on a video it changes from frame to frame like film grain
//! instead of sitting still like a screen.  It hides the steps of
//! banding that a grade makes more visible.

/// The amount a bare `--grain` uses, in 8-bit levels.
pub const DEFAULT: f64 = 4.0;

/// The largest amount `--grain` takes.
pub const MAX: f64 = 32.0;

/// SplitMix64's finalizer: a well-mixed 64-bit hash of `x`.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// The grain at one pixel of one frame, in levels (-amount..=amount).
pub fn offset(x: u32, y: u32, frame: u64, amount: f64) -> f64 {
    let h = mix((u64::from(x) | u64::from(y) << 32) ^ mix(frame));
    let a = (h >> 40) as f64 / (1u64 << 24) as f64;
    let b = (h & 0xff_ffff) as f64 / (1u64 << 24) as f64;
    (a + b - 1.0) * amount
}

/// Add grain to a `width`-wide rgb8 buffer (frame 0 for a still).
pub fn apply(pixels: &mut [u8], width: u32, frame: u64, amount: f64) {
    if amount <= 0.0 || width == 0 {
        return;
    }
    let row = width as usize * 3;
    crate::par::chunks_mut(pixels, row, |start, run| {
        let first = (start / row) as u32;
        for (r, line) in run.chunks_mut(row).enumerate() {
            let y = first + r as u32;
            for (x, px) in line.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                let d = offset(x as u32, y, frame, amount).round();
                for c in px.iter_mut() {
                    *c = (f64::from(*c) + d).clamp(0.0, 255.0) as u8;
                }
            }
        }
    });
}
