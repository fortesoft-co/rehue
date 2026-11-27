//! Color math the generator works in.
//!
//! OKLCH colors as `Lch`, the sRGB gamut, WCAG contrast and the
//! deuteranopia check, with the constants and order of operations the
//! generator was tuned with, so its schemes come out exactly as tuned.

use crate::color::{Lch, OKLAB_M1, OKLAB_M1_INV, OKLAB_M2, OKLAB_M2_INV, floor_mod};

/// Signed hue difference `b - a`, in [-180, 180).
pub fn hd(a: f64, b: f64) -> f64 {
    floor_mod(b - a + 180.0, 360.0) - 180.0
}

pub fn lch(l: f64, c: f64, h: f64) -> Lch {
    Lch { l, c, h }
}

pub fn to_lab(x: &Lch) -> [f64; 3] {
    let r = x.h.to_radians();
    [x.l, x.c * r.cos(), x.c * r.sin()]
}

pub fn from_lab(lab: [f64; 3]) -> Lch {
    Lch {
        l: lab[0],
        c: lab[1].hypot(lab[2]),
        h: floor_mod(lab[2].atan2(lab[1]).to_degrees(), 360.0),
    }
}

fn dot(m: &[[f64; 3]; 3], v: &[f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// OKLab to linear sRGB, unclamped.
pub fn lab_to_lrgb(lab: [f64; 3]) -> [f64; 3] {
    let lms = dot(&OKLAB_M2_INV, &lab).map(|x| x * x * x);
    dot(&OKLAB_M1_INV, &lms)
}

/// Linear sRGB to OKLab (signed cube root, so out-of-gamut values work).
pub fn lrgb_to_lab(rgb: [f64; 3]) -> [f64; 3] {
    let lms = dot(&OKLAB_M1, &rgb).map(|x| x.abs().powf(1.0 / 3.0).copysign(x));
    dot(&OKLAB_M2, &lms)
}

fn in_gamut(rgb: [f64; 3]) -> bool {
    rgb.iter().all(|x| (-1e-4..=1.0 + 1e-4).contains(x))
}

/// The same color with its chroma reduced until it fits sRGB.
pub fn gamut(x: Lch) -> Lch {
    if in_gamut(lab_to_lrgb(to_lab(&x))) {
        return x;
    }
    let (mut lo, mut hi) = (0.0, x.c);
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        if in_gamut(lab_to_lrgb(to_lab(&lch(x.l, mid, x.h)))) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lch(x.l, lo, x.h)
}

/// Linear sRGB, clamped to [0, 1].
pub fn lrgb(x: &Lch) -> [f64; 3] {
    lab_to_lrgb(to_lab(x)).map(|v| v.clamp(0.0, 1.0))
}

fn luminance(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// WCAG contrast ratio, 1 to 21.
pub fn wcag(a: &Lch, b: &Lch) -> f64 {
    let (x, y) = (luminance(lrgb(a)), luminance(lrgb(b)));
    let (hi, lo) = if x >= y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// Euclidean OKLab distance.
pub fn de(a: &Lch, b: &Lch) -> f64 {
    let (p, q) = (to_lab(a), to_lab(b));
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
}

/// Deuteranopia simulation (Machado et al., severity 1) on linear sRGB.
const DEUTAN: [[f64; 3]; 3] = [
    [0.367322, 0.860646, -0.227968],
    [0.280085, 0.672501, 0.047413],
    [-0.01182, 0.04294, 0.968881],
];

/// OKLab distance between two colors as a deuteranope sees them.
pub fn deutan_de(a: &Lch, b: &Lch) -> f64 {
    let sim = |c: &Lch| lrgb_to_lab(dot(&DEUTAN, &lrgb(c)).map(|v| v.max(0.0)));
    let (p, q) = (sim(a), sim(b));
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
}

/// Circular mean of hues weighted by `w`.
pub fn mean_hue(hues: impl Iterator<Item = (f64, f64)>) -> f64 {
    let (mut x, mut y) = (0.0, 0.0);
    for (h, w) in hues {
        x += h.to_radians().cos() * w;
        y += h.to_radians().sin() * w;
    }
    floor_mod(y.atan2(x).to_degrees(), 360.0)
}
