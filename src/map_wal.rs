//! Per-pixel palette remap: arrange (distribution + rotate), then
//! harmonize (hue-only), quantize (adopt the slot's lightness and chroma),
//! per-register grading, then image-wide grading.  Deterministic and
//! sandbox-pure.  Full-strength quantize currently bands on gradients
//! (dithering is a planned follow-up).

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::color::{Lch, circ_dist, circ_lerp, oklch_to_rgb, rgb_to_oklch};
use crate::extract::{Cluster, ExtractionParams};
use crate::register::{
    Distribution, DistributionState, REGISTER_NAMES, distribution_hues, family_indices,
    register_of_slot, register_slots, resolve_distribution, slot_index,
};
use crate::scheme::BASE_SLOTS;

/// Per-register override; every field optional (the
/// `globals // all // register` merge fills the gaps).  Canonical keys are
/// kebab-case in the config JSON.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default, deny_unknown_fields)]
pub struct WalRegisterConfig {
    pub distribution: Option<Distribution>,
    pub rotate: Option<i64>,
    pub harmonize: Option<f64>,
    pub quantize: Option<f64>,
    pub light: Option<f64>,
    pub chroma: Option<f64>,
}

/// Remap knobs; every one is opt-in.  Canonical keys kebab-case, with
/// underscore aliases for the old nix wrapper's parameter names.  The
/// flattened extraction section feeds the arrangement stage's family
/// resolution (only needed when a register sets `distribution`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct RemapConfig {
    #[serde(flatten)]
    pub extraction: ExtractionParams,
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
    pub registers: HashMap<String, WalRegisterConfig>,
}

impl Default for RemapConfig {
    fn default() -> Self {
        Self {
            extraction: ExtractionParams::default(),
            harmonize: 0.0,
            harmonize_threshold_deg: 30.0,
            quantize: 0.0,
            quantize_threshold_deg: 30.0,
            gray_chroma_floor: 0.02,
            light: 0.0,
            chroma: 1.0,
            registers: HashMap::new(),
        }
    }
}

/// Resolved per-register settings the wall pipeline runs with.
#[derive(Debug, Clone)]
pub struct WalRegisterSettings {
    pub distribution: DistributionState,
    pub rotate: i64,
    pub harmonize: f64,
    pub quantize: f64,
    pub light: f64,
    pub chroma: f64,
}

/// The `globals // all // register` resolution, with validation.
pub fn resolved_registers(
    config: &RemapConfig,
) -> Result<BTreeMap<&'static str, WalRegisterSettings>, String> {
    let all = config.registers.get("all").cloned().unwrap_or_default();
    let mut out = BTreeMap::new();
    for name in REGISTER_NAMES {
        let given = config.registers.get(name).cloned().unwrap_or_default();
        let distribution = resolve_distribution(
            name,
            given.distribution.or_else(|| all.distribution.clone()),
        )?;
        let rotate = given.rotate.or(all.rotate).unwrap_or(0);
        let harmonize = given
            .harmonize
            .or(all.harmonize)
            .unwrap_or(config.harmonize);
        let quantize = given.quantize.or(all.quantize).unwrap_or(config.quantize);
        let light = given.light.or(all.light).unwrap_or(config.light);
        let chroma = given.chroma.or(all.chroma).unwrap_or(config.chroma);
        if !(0.0..=1.0).contains(&harmonize) {
            return Err(format!(
                "registers.{}.harmonize must be within [0, 1]",
                name
            ));
        }
        if !(0.0..=1.0).contains(&quantize) {
            return Err(format!("registers.{}.quantize must be within [0, 1]", name));
        }
        if chroma <= 0.0 {
            return Err(format!("registers.{}.chroma must be positive", name));
        }
        out.insert(
            name,
            WalRegisterSettings {
                distribution,
                rotate,
                harmonize,
                quantize,
                light,
                chroma,
            },
        );
    }
    Ok(out)
}

/// One scheme slot with its resolved colour.
#[derive(Debug, Clone, Copy)]
pub struct SlotPalette {
    pub slot: &'static str,
    pub lch: Lch,
}

/// Per-slot mix/grade knobs: the slot's register's resolved settings.
#[derive(Debug, Clone, Copy)]
pub struct SlotKnobs {
    pub harmonize: f64,
    pub quantize: f64,
    pub light: f64,
    pub chroma: f64,
}

/// Per-slot knobs for the 16 canonical slots.
pub fn slot_knobs(regs: &BTreeMap<&'static str, WalRegisterSettings>) -> Vec<SlotKnobs> {
    BASE_SLOTS
        .iter()
        .map(|slot| {
            let s = &regs[register_of_slot(slot)];
            SlotKnobs {
                harmonize: s.harmonize,
                quantize: s.quantize,
                light: s.light,
                chroma: s.chroma,
            }
        })
        .collect()
}

/// The arrangement stage - shared semantics with the scheme side: each
/// register's `distribution` recolours its slots' hues from the wallpaper's
/// families (lightness and chroma stay slot-local), then `rotate` permutes
/// the register's hues across its slots.  A palette with no hues extracted
/// (e.g. greyscale wallpaper, or no register distributes) passes through.
pub fn arrange_palette(
    slots: &[SlotPalette],
    clusters: &[Cluster],
    regs: &BTreeMap<&'static str, WalRegisterSettings>,
) -> Result<Vec<SlotPalette>, String> {
    let mut out = slots.to_vec();
    for name in REGISTER_NAMES {
        let settings = &regs[name];
        let reg_slots = register_slots(name);
        let k = reg_slots.len();
        if !clusters.is_empty() && settings.distribution != DistributionState::Off {
            let stops = match &settings.distribution {
                DistributionState::Auto => (0..k.min(clusters.len())).collect::<Vec<usize>>(),
                DistributionState::Pin(i) => family_indices(&[*i], clusters, name)?,
                DistributionState::Stops(v) => family_indices(v, clusters, name)?,
                DistributionState::Off => unreachable!("checked above"),
            };
            let hues = distribution_hues(k, &stops, clusters);
            for (pos, slot) in reg_slots.iter().copied().enumerate() {
                out[slot_index(slot)].lch.h = hues[pos];
            }
        }
        let rotated_by = settings.rotate.rem_euclid(k as i64) as usize;
        if rotated_by != 0 {
            let hues: Vec<f64> = (0..k)
                .map(|pos| out[slot_index(reg_slots[pos])].lch.h)
                .collect();
            for pos in 0..k {
                let from = (pos as i64 - rotated_by as i64).rem_euclid(k as i64) as usize;
                out[slot_index(reg_slots[pos])].lch.h = hues[from];
            }
        }
    }
    Ok(out)
}

/// Result of a remap pass.
#[derive(Debug, Clone)]
pub struct RemapOutput {
    pub pixels: Vec<u8>,
    /// matched-pixel counts per slot, in canonical base order
    pub coverage: Vec<u64>,
}

/// True when no knob is active anywhere: the output is then a re-encoded
/// passthrough of the input (matching the documented contract).
pub fn is_passive(
    config: &RemapConfig,
    regs: &BTreeMap<&'static str, WalRegisterSettings>,
) -> bool {
    let globals_idle = config.harmonize == 0.0
        && config.quantize == 0.0
        && config.light == 0.0
        && (config.chroma - 1.0).abs() < 1e-9;
    globals_idle
        && regs.values().all(|r| {
            matches!(r.distribution, DistributionState::Off)
                && r.rotate == 0
                && r.harmonize == 0.0
                && r.quantize == 0.0
                && r.light == 0.0
                && (r.chroma - 1.0).abs() < 1e-9
        })
}

/// The per-pixel pipeline.  `pixels` is a flat rgb8 buffer; `slots` the 16
/// palette slot colours in canonical order (already arranged by
/// `arrange_palette` when distribution knobs are in play); `knobs` the
/// per-slot mix/grade values; `neutral_slots` the indices of the scheme's
/// low-chroma slots (achromatic pixels key against those by lightness
/// rather than by hue).  Thresholds and the final image-wide grade come
/// from `config`.
pub fn apply(
    pixels: &[u8],
    slots: &[SlotPalette],
    neutral_slots: &[usize],
    knobs: &[SlotKnobs],
    config: &RemapConfig,
) -> RemapOutput {
    debug_assert_eq!(slots.len(), knobs.len());
    let h_threshold = config.harmonize_threshold_deg;
    let q_threshold = config.quantize_threshold_deg;
    let gray_floor = config.gray_chroma_floor;
    let image_light = config.light;
    let image_chroma = config.chroma;

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
            let knobs = knobs[best];
            h2 = lch.h;
            if knobs.quantize > 0.0 {
                let t = knobs.quantize.min(1.0);
                l2 = lch.l + (slots[best].lch.l - lch.l) * t;
                c2 = (lch.c + (slots[best].lch.c - lch.c) * t).max(0.0);
                h2 = circ_lerp(lch.h, slots[best].lch.h, t);
            } else {
                l2 = lch.l;
                c2 = lch.c;
            }
            l2 = (l2 + knobs.light).clamp(0.0, 1.0);
            c2 = (c2 * knobs.chroma).max(0.0);
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
            let knobs = knobs[best];
            let target = slots[best].lch;
            if knobs.harmonize > 0.0 && best_d <= h_threshold {
                h2 = circ_lerp(lch.h, target.h, knobs.harmonize);
            } else {
                h2 = lch.h;
            }
            if knobs.quantize > 0.0 && best_d <= q_threshold {
                let t = knobs.quantize.min(1.0);
                l2 = lch.l + (target.l - lch.l) * t;
                c2 = (lch.c + (target.c - lch.c) * t).max(0.0);
                h2 = circ_lerp(h2, target.h, t);
            } else {
                l2 = lch.l;
                c2 = lch.c;
            }
            l2 = (l2 + knobs.light).clamp(0.0, 1.0);
            c2 = (c2 * knobs.chroma).max(0.0);
        }
        // Image-wide grade, applied last.
        l2 = (l2 + image_light).clamp(0.0, 1.0);
        c2 = (c2 * image_chroma).max(0.0);
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
