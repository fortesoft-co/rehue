//! Neutrals, the accents' chroma level, and finishing.
//!
//! Neutrals come from the image; finishing is contrast on base00 and
//! the red/green push.

use super::analysis::Swatch;
use super::math::{deutan_de, gamut, lch, wcag};
use crate::color::Lch;

/// Bundled dark schemes' neutral medians, base00..base07.
const NEUTRAL_L: [f64; 8] = [0.227, 0.285, 0.371, 0.517, 0.679, 0.838, 0.904, 0.962];
const NEUTRAL_C: [f64; 8] = [0.011, 0.015, 0.021, 0.018, 0.017, 0.014, 0.012, 0.006];
/// Bundled light schemes' neutral medians.
const LIGHT_L: [f64; 8] = [0.972, 0.917, 0.858, 0.680, 0.519, 0.405, 0.298, 0.236];
const LIGHT_C: [f64; 8] = [0.006, 0.008, 0.015, 0.016, 0.018, 0.016, 0.011, 0.008];

/// Neutral tint, times the bundled medians' chroma.
pub const DARK_TINT: f64 = 1.5;
pub const LIGHT_TINT: f64 = 3.75;

/// Accents need 4.5:1 on base00.
const CONTRAST: f64 = 4.5;
/// The soft knee above (dark) or below (light) the contrast line.
const KNEE: f64 = 0.06;
/// Light accents start this much darker than the dark ones.
const LIGHT_OFFSET: f64 = 0.14;

/// Dark neutrals: base00 from the image's dark end (0.12 to 0.30), the
/// ramp rescaled so base07 stays.
pub fn dark_ramp(seed_h: f64, tint: f64, p10: f64) -> [Lch; 8] {
    let b00 = 0.12 + 0.18 * (p10 / 0.5).min(1.0);
    let (l0, l7) = (NEUTRAL_L[0], NEUTRAL_L[7]);
    let k = (l7 - b00) / (l7 - l0);
    std::array::from_fn(|i| {
        gamut(lch(
            b00 + (NEUTRAL_L[i] - l0) * k,
            NEUTRAL_C[i] * tint,
            seed_h,
        ))
    })
}

/// Light neutrals: base00 from the image's bright end (0.92 to 0.987).
pub fn light_ramp(seed_h: f64, tint: f64, p90: f64) -> [Lch; 8] {
    let (lo, hi) = (0.92, 0.987);
    let b00 = hi - (hi - lo) * ((1.0 - p90) / 0.5).min(1.0);
    let (l0, l7) = (LIGHT_L[0], LIGHT_L[7]);
    let k = (b00 - l7) / (l0 - l7);
    std::array::from_fn(|i| gamut(lch(l7 + (LIGHT_L[i] - l7) * k, LIGHT_C[i] * tint, seed_h)))
}

/// Pixels count as colored from this chroma.
const COLORED: f64 = 0.04;
/// The colored median counts fully once this share of the image is colored.
const FULL_SHARE: f64 = 0.10;
/// The level never drops below this.
const FLOOR: f64 = 0.04;
/// The more saturated the image (S from S_LO to S_HI), the more the
/// whole-image median counts.
const S_LO: f64 = 0.04;
const S_HI: f64 = 0.09;

/// The share-weighted median chroma.
fn weighted_median(cs: &[Swatch]) -> f64 {
    let mut cs: Vec<&Swatch> = cs.iter().collect();
    cs.sort_by(|a, b| a.c.total_cmp(&b.c));
    let t: f64 = cs.iter().map(|c| c.share).sum();
    let mut a = 0.0;
    for c in &cs {
        a += c.share;
        if a >= t / 2.0 {
            return c.c;
        }
    }
    cs.last().map_or(0.0, |c| c.c)
}

/// The accents' mean chroma: the colored pixels' median, weighted by
/// how much of the image is colored, blended toward the whole image's
/// median as the image gets more saturated.
pub fn chroma_level(fine: &[Swatch]) -> f64 {
    let colored: Vec<Swatch> = fine.iter().copied().filter(|c| c.c >= COLORED).collect();
    let share: f64 = colored.iter().map(|c| c.share).sum();
    let cw = if share == 0.0 {
        FLOOR
    } else {
        let mc = weighted_median(&colored);
        (FLOOR + (mc - FLOOR) * (share / FULL_SHARE).min(1.0)).max(FLOOR)
    };
    let mw = weighted_median(fine);
    let s: f64 = fine.iter().map(|c| c.c * c.share).sum();
    let w = ((s - S_LO) / (S_HI - S_LO)).clamp(0.0, 1.0);
    ((1.0 - w) * cw + w * mw).max(FLOOR)
}

/// The lowest lightness at which a color keeps 4.5:1 on a dark base00.
fn floor_l(c: f64, h: f64, base00: &Lch) -> f64 {
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..25 {
        let mid = (lo + hi) / 2.0;
        if wcag(&gamut(lch(mid, c, h)), base00) >= CONTRAST {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// The highest lightness at which a color keeps 4.5:1 on a light base00.
fn ceiling_l(c: f64, h: f64, base00: &Lch) -> f64 {
    let (mut lo, mut hi) = (0.0, base00.l);
    for _ in 0..25 {
        let mid = (lo + hi) / 2.0;
        if wcag(&gamut(lch(mid, c, h)), base00) >= CONTRAST {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Below KNEE past the line, the distance shrinks smoothly toward it.
fn knee(x: f64) -> f64 {
    if x < KNEE {
        KNEE * ((x - KNEE) / KNEE).exp()
    } else {
        x
    }
}

/// Dark accents: a soft knee above the contrast floor, then 4.5:1 held,
/// then base08 and base0B pushed apart for deuteranopes.
pub fn finish_dark(acc: &[Lch; 8], base00: &Lch) -> [Lch; 8] {
    let mut out = acc.map(|a| {
        let f = floor_l(a.c, a.h, base00);
        gamut(lch(f + knee(a.l - f), a.c, a.h))
    });
    for a in out.iter_mut() {
        let mut x = gamut(lch(a.l.min(0.95), a.c, a.h));
        while wcag(&x, base00) < CONTRAST && x.l < 0.95 {
            x = gamut(lch(x.l + 0.01, x.c, x.h));
        }
        *a = x;
    }
    let (mut r, mut g) = (out[0], out[3]);
    let mut n = 0;
    while deutan_de(&r, &g) < 0.08 && n < 20 {
        let g_up = g.l >= r.l;
        let (mut up, mut dn) = if g_up { (g, r) } else { (r, g) };
        up = gamut(lch((up.l + 0.01).min(0.95), up.c, up.h));
        let lower = gamut(lch(dn.l - 0.01, dn.c, dn.h));
        if wcag(&lower, base00) >= CONTRAST {
            dn = lower;
        } else {
            up = gamut(lch((up.l + 0.01).min(0.95), up.c, up.h));
        }
        (r, g) = if g_up { (dn, up) } else { (up, dn) };
        n += 1;
    }
    out[0] = r;
    out[3] = g;
    out
}

/// Light accents: 0.14 darker than dark ones, a soft knee below the
/// contrast ceiling, then the red/green push.
pub fn finish_light(acc: &[Lch; 8], base00: &Lch) -> [Lch; 8] {
    let mut out = acc.map(|a| {
        let l = a.l - LIGHT_OFFSET;
        let c = ceiling_l(a.c, a.h, base00);
        gamut(lch(c - knee(c - l), a.c, a.h))
    });
    let (mut r, mut g) = (out[0], out[3]);
    let mut n = 0;
    while deutan_de(&r, &g) < 0.08 && n < 30 {
        let g_up = g.l >= r.l;
        let (mut up, mut dn) = if g_up { (g, r) } else { (r, g) };
        dn = gamut(lch(dn.l - 0.01, dn.c, dn.h));
        let raised = gamut(lch(up.l + 0.01, up.c, up.h));
        if wcag(&raised, base00) >= CONTRAST {
            up = raised;
        }
        (r, g) = if g_up { (dn, up) } else { (up, dn) };
        n += 1;
    }
    out[0] = r;
    out[3] = g;
    out
}

/// 4.5:1 on base00 held after a later change: dark accents rise, light
/// ones darken, until they meet it.
pub fn hold_contrast(a: Lch, base00: &Lch, light: bool) -> Lch {
    let mut x = gamut(a);
    let mut n = 0;
    while wcag(&x, base00) < CONTRAST && n < 200 {
        let l = if light { x.l - 0.005 } else { x.l + 0.005 };
        if !(0.0..=1.0).contains(&l) {
            break;
        }
        x = gamut(lch(l, a.c, a.h));
        n += 1;
    }
    x
}
