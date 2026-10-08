//! Per-pixel palette remap: arrange (distribution + rotate), then
//! harmonize (hue-only), quantize (adopt the slot's lightness/chroma),
//! per-register grading, then image-wide grading.  Deterministic and
//! sandbox-pure.
//!
//! Quantize splits into independent hue (`quantize`), lightness
//! (`quantize-light`) and chroma (`quantize-chroma`) adoption dials, all
//! defaulting to the scalar; the adopted share can be dithered with an
//! 8x8 Bayer matrix (`dithering`) so full-strength adoption doesn't band.
//! Dithering rides only on the adopted share, so it is inert at zero
//! quantize.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::color::{Lch, circ_dist, circ_lerp, oklch_to_rgb, rgb_to_oklch};
use crate::extract::{Cluster, ExtractionParams};
use crate::register::{
    Distribution, DistributionState, REGISTER_NAMES, distribution_hues, family_indices,
    register_of_slot, register_slots, resolve_distribution, slot_index,
};
use crate::scheme::BASE_SLOTS;

/// Classic 8x8 ordered-dithering threshold matrix (Bayer); values are
/// jitter scalars in [0, 1), read at (x mod 8, y mod 8).
const BAYER8: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

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
    pub quantize_light: Option<f64>,
    pub quantize_chroma: Option<f64>,
    pub light: Option<f64>,
    pub chroma: Option<f64>,
}

/// Remap knobs; every one is opt-in.  Canonical keys kebab-case, with
/// underscore aliases where the old nix wrapper had parameter names.  The
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
    /// Adoption dials defaulting to `quantize` when unset.
    pub quantize_light: Option<f64>,
    pub quantize_chroma: Option<f64>,
    /// Ordered-dithering strength for the adopted L/C share; inert at
    /// zero adoption.
    #[serde(alias = "dithering")]
    pub dithering: f64,
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
            quantize_light: None,
            quantize_chroma: None,
            dithering: 0.0,
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
    pub quantize_light: f64,
    pub quantize_chroma: f64,
    pub light: f64,
    pub chroma: f64,
}

/// The `globals // all // register` resolution, with validation.
pub fn resolved_registers(
    config: &RemapConfig,
) -> Result<BTreeMap<&'static str, WalRegisterSettings>, String> {
    if !(0.0..=1.0).contains(&config.dithering) {
        return Err("dithering must be within [0, 1]".to_string());
    }
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
        let quantize_light = given
            .quantize_light
            .or(all.quantize_light)
            .or(config.quantize_light)
            .unwrap_or(config.quantize);
        let quantize_chroma = given
            .quantize_chroma
            .or(all.quantize_chroma)
            .or(config.quantize_chroma)
            .unwrap_or(config.quantize);
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
        if !(0.0..=1.0).contains(&quantize_light) {
            return Err(format!(
                "registers.{}.quantize-light must be within [0, 1]",
                name
            ));
        }
        if !(0.0..=1.0).contains(&quantize_chroma) {
            return Err(format!(
                "registers.{}.quantize-chroma must be within [0, 1]",
                name
            ));
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
                quantize_light,
                quantize_chroma,
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
    pub quantize_light: f64,
    pub quantize_chroma: f64,
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
                quantize_light: s.quantize_light,
                quantize_chroma: s.quantize_chroma,
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
/// Dithering alone is inert (it only alters the adopted quantize share),
/// so it does not decide passivity.
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

/// The per-pixel pipeline.  `pixels` is a flat rgb8 buffer of a
/// `width x height` image; `slots` the 16 palette slot colours in
/// canonical order (already arranged by `arrange_palette` when
/// distribution knobs are in play); `knobs` the per-slot mix/grade values;
/// `neutral_slots` the indices of the scheme's low-chroma slots
/// (achromatic pixels key against those by lightness rather than by hue).
/// Thresholds, dithering strength and the final image-wide grade come from
/// `config`.
pub fn apply(
    pixels: &[u8],
    width: u32,
    height: u32,
    slots: &[SlotPalette],
    neutral_slots: &[usize],
    knobs: &[SlotKnobs],
    config: &RemapConfig,
) -> RemapOutput {
    debug_assert_eq!(slots.len(), knobs.len());
    debug_assert_eq!(
        pixels.len(),
        width as usize * height as usize * 3,
        "flat buffer must match dimensions"
    );
    let h_threshold = config.harmonize_threshold_deg;
    let q_threshold = config.quantize_threshold_deg;
    let gray_floor = config.gray_chroma_floor;
    let dither = config.dithering.clamp(0.0, 1.0);
    let image_light = config.light;
    let image_chroma = config.chroma;

    let mut out = vec![0u8; pixels.len()];
    let mut counts = vec![0u64; slots.len()];

    for y in 0..height as usize {
        for x in 0..width as usize {
            let i = (y * width as usize + x) * 3;
            let chunk_in = &pixels[i..i + 3];
            let chunk_out = &mut out[i..i + 3];
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
                let slots_index = slots[best];
                let n = knobs[best];
                let jitter = 1.0 + ((f64::from(BAYER8[y % 8][x % 8]) + 0.5) / 64.0 - 0.5) * dither;
                h2 = lch.h;
                if n.quantize_light > 0.0 {
                    l2 = lch.l + (slots_index.lch.l - lch.l) * n.quantize_light * jitter;
                } else {
                    l2 = lch.l;
                }
                if n.quantize_chroma > 0.0 {
                    c2 =
                        (lch.c + (slots_index.lch.c - lch.c) * n.quantize_chroma * jitter).max(0.0);
                } else {
                    c2 = lch.c;
                }
                if n.quantize > 0.0 {
                    h2 = circ_lerp(lch.h, slots_index.lch.h, n.quantize.min(1.0));
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
                let n = knobs[best];
                if n.harmonize > 0.0 && best_d <= h_threshold {
                    h2 = circ_lerp(lch.h, target.h, n.harmonize);
                } else {
                    h2 = lch.h;
                }
                if (n.quantize > 0.0 || n.quantize_light > 0.0 || n.quantize_chroma > 0.0)
                    && best_d <= q_threshold
                {
                    let jitter =
                        1.0 + ((f64::from(BAYER8[y % 8][x % 8]) + 0.5) / 64.0 - 0.5) * dither;
                    h2 = circ_lerp(h2, target.h, n.quantize.min(1.0));
                    l2 = lch.l + (target.l - lch.l) * n.quantize_light * jitter;
                    c2 = (lch.c + (target.c - lch.c) * n.quantize_chroma * jitter).max(0.0);
                } else {
                    l2 = lch.l;
                    c2 = lch.c;
                }
            }
            // Per-slot (register) grade, then the image-wide grade last.
            let n = knobs[best_index];
            l2 = (l2 + n.light).clamp(0.0, 1.0);
            c2 = (c2 * n.chroma).max(0.0);
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
    }

    RemapOutput {
        pixels: out,
        coverage: counts,
    }
}
