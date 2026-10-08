//! Per-pixel palette remap: harmonize (hue-only), quantize (adopt the
//! slot's lightness and chroma), then image-wide grading.  Ported 1:1
//! deterministic and sandbox-pure.
//! Full-strength quantize currently bands on gradients (dithering is a
//! planned follow-up).

use serde::{Deserialize, Serialize};

use crate::color::{circ_dist, circ_lerp, oklch_to_rgb, rgb_to_oklch, Lch};

/// Remap knobs; every one is opt-in.  Canonical keys kebab-case, with
/// underscore aliases for the old nix wrapper's parameter names.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct RemapConfig {
    pub harmonize: f64,
    #[serde(alias = "harmonize_threshold_deg")]
    pub harmonize_threshold_deg: f64,
    pub quantize: f64,
    #[serde(alias = "quantize_threshold_deg")]
    pub quantize_threshold_deg: f64,
    #[serde(alias = "gray_chroma_floor")]
    pub gray_chroma_floor: f64,
    pub light: f64,
    pub chroma: f64,
}

impl Default for RemapConfig {
    fn default() -> Self {
        Self {
            harmonize: 0.0,
            harmonize_threshold_deg: 30.0,
            quantize: 0.0,
            quantize_threshold_deg: 30.0,
            gray_chroma_floor: 0.02,
            light: 0.0,
            chroma: 1.0,
        }
    }
}

/// One scheme slot with its resolved colour.
#[derive(Debug, Clone, Copy)]
pub struct SlotPalette {
    pub slot: &'static str,
    pub lch: Lch,
}

/// Result of a remap pass.
#[derive(Debug, Clone)]
pub struct RemapOutput {
    pub pixels: Vec<u8>,
    /// matched-pixel counts per slot, in canonical base order
    pub coverage: Vec<u64>,
}

/// True when no knob is set: the output is then a re-encoded passthrough
/// of the input (matching the documented contract).
pub fn is_passive(config: &RemapConfig) -> bool {
    config.harmonize == 0.0
        && config.quantize == 0.0
        && config.light == 0.0
        && (config.chroma - 1.0).abs() < 1e-9
}

/// The per-pixel pipeline.  `pixels` is a flat rgb8 buffer; `slots` the 16
/// scheme slot colours in canonical order; `neutral_slots` the indices of
/// the scheme's low-chroma slots (achromatic pixels key against those by
/// lightness rather than by hue).
pub fn apply(
    pixels: &[u8],
    slots: &[SlotPalette],
    neutral_slots: &[usize],
    config: &RemapConfig,
) -> RemapOutput {
    let harmonize = config.harmonize.clamp(0.0, 1.0);
    let h_threshold = config.harmonize_threshold_deg;
    let quantize = config.quantize.clamp(0.0, 1.0);
    let q_threshold = config.quantize_threshold_deg;
    let gray_floor = config.gray_chroma_floor;
    let light = config.light;
    let chroma = config.chroma;

    let mut out = vec![0u8; pixels.len()];
    let mut counts = vec![0u64; slots.len()];

    for (chunk_out, chunk_in) in out.chunks_exact_mut(3).zip(pixels.chunks_exact(3)) {
        let lch = rgb_to_oklch(&[chunk_in[0], chunk_in[1], chunk_in[2]]);
        let (mut l2, mut c2, mut h2);
        let best_index;
        if lch.c < gray_floor {
            // Achromatic: keyed to the scheme's low-chroma slots by
            // lightness (first minimum wins).
            let mut best = 0usize;
            let mut best_d = f64::INFINITY;
            for &idx in neutral_slots {
                let d = (slots[idx].lch.l - lch.l).abs();
                if d < best_d {
                    best_d = d;
                    best = idx;
                }
            }
            best_index = best;
            h2 = lch.h;
            if quantize > 0.0 {
                let t = quantize.min(1.0);
                l2 = lch.l + (slots[best].lch.l - lch.l) * t;
                c2 = (lch.c + (slots[best].lch.c - lch.c) * t).max(0.0);
                h2 = circ_lerp(lch.h, slots[best].lch.h, t);
            } else {
                l2 = lch.l;
                c2 = lch.c;
            }
        } else {
            // Chromatic: nearest slot by hue (first minimum), then the
            // opt-in harmonize / quantize moves, threshold-gated.
            let mut best = 0usize;
            let mut best_d = f64::INFINITY;
            for (idx, slot) in slots.iter().enumerate() {
                let d = circ_dist(lch.h, slot.lch.h);
                if d < best_d {
                    best_d = d;
                    best = idx;
                }
            }
            best_index = best;
            let target = slots[best].lch;
            if harmonize > 0.0 && best_d <= h_threshold {
                h2 = circ_lerp(lch.h, target.h, harmonize);
            } else {
                h2 = lch.h;
            }
            if quantize > 0.0 && best_d <= q_threshold {
                let t = quantize.min(1.0);
                l2 = lch.l + (target.l - lch.l) * t;
                c2 = (lch.c + (target.c - lch.c) * t).max(0.0);
                h2 = circ_lerp(h2, target.h, t);
            } else {
                l2 = lch.l;
                c2 = lch.c;
            }
        }
        // Image-wide grade, applied last.
        l2 = (l2 + light).clamp(0.0, 1.0);
        c2 = (c2 * chroma).max(0.0);
        let back = oklch_to_rgb(&Lch {
            l: l2,
            c: c2,
            h: h2,
        });
        chunk_out[0] = back[0];
        chunk_out[1] = back[1];
        chunk_out[2] = back[2];
        counts[best_index] += 1;
    }

    RemapOutput {
        pixels: out,
        coverage: counts,
    }
}
