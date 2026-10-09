//! Per-pixel palette remap: arrange (distribution + rotate), then blend
//! toward the palette (hue, light and chroma channel fidelity), then
//! per-register grading, then image-wide grading.  Deterministic and
//! sandbox-pure.
//!
//! The `harmonize` facade is the one-dial surface: it seeds `blend-hue`
//! and `blend-chroma` wherever they are left unset, while explicit dials
//! win and `blend-light` stays decoupled (default 0 - photographic
//! lightness, since adopting lightness flattens contrast).  The adopted
//! lightness/chroma share can be dithered so full-strength adoption
//! doesn't band: `dithering` scales an ordered-dither threshold
//! (blue-noise or Bayer mask) in ordered modes, and the fraction of
//! residual actually diffused in the error-diffusion modes
//! (Floyd-Steinberg, Atkinson; serpentine scan, fixed order - no RNG).
//! Anchoring always happens on the raw pixel, so slot membership - the
//! territories - is independent of the dither; diffusion only feeds the
//! mix, not the anchor.  Dithering is inert at zero adoption.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::bluenoise::BLUE_NOISE64;
use crate::color::{Lch, circ_dist, circ_lerp, oklch_to_rgb, rgb_to_oklch, weighted_circ_mean};
use crate::extract::{Cluster, ExtractionParams};
use crate::register::{
    Distribution, DistributionState, REGISTER_NAMES, distribution_hues, family_indices,
    register_of_slot, register_slots, resolve_distribution, slot_index,
};
use crate::scheme::BASE_SLOTS;

/// Classic 8x8 ordered-dithering threshold matrix (Bayer); threshold
/// scalar `(v + 0.5) / 64` in [0, 1), read at (x mod 8, y mod 8).
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

/// The `dithering-mode` option.  Ordered modes threshold the adopted share
/// with a pixel mask; diffusion modes propagate the mix residual to
/// neighbouring pixels (serpentine scan order, fully deterministic).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum DitherMode {
    /// 64x64 tileable void-and-cluster mask (the default; pattern is
    /// visually invisible).
    BlueNoise,
    /// 8x8 Bayer matrix (classic crosshatch look).
    Bayer,
    /// Classic serpentine error diffusion; best tonal fidelity, can
    /// "worm" in flat regions.
    FloydSteinberg,
    /// Gentler diffusion (6/8 of the error propagated, 2/8 dropped).
    Atkinson,
    /// Explicitly off (same as dithering strength 0).
    None,
}

impl DitherMode {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::BlueNoise => "blue-noise",
            Self::Bayer => "bayer",
            Self::FloydSteinberg => "floyd-steinberg",
            Self::Atkinson => "atkinson",
            Self::None => "none",
        }
    }
}

/// The `territory` option: how pixels relate to the palette.
///
/// **hard** (opt-in): every pixel anchors to its nearest slot and adopts
/// that slot's constants - fast, stylized, and exactly the mechanism that
/// bands and hard edges come from.
///
/// **soft**: slot influence falls off with distance instead of cutting off
/// (circular hue distance for chromatic pixels, lightness distance for
/// achromatic ones); every target is the weighted mean over the palette.
/// Boundaries between slots become smooth crossings and the flat-constant
/// posterize largely disappears by construction.  Soft bypasses the
/// hard-mode cutoffs: `reach-deg` acts as the falloff temperature there
/// (and must be positive), and as the influence cutoff in hard mode.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Territory {
    /// The default: continuous weighted mixing (palette as influence field).
    Soft,
    /// Opt-in stylized mode: nearest-slot snapping + flat-constant
    /// adoption (the flat-look with posterize and hard edges, by design).
    Hard,
}

impl Territory {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Soft => "soft",
            Self::Hard => "hard",
        }
    }
}

/// Per-register override; every field optional (the
/// `globals // all // register` merge fills the gaps).  Canonical keys are
/// kebab-case in the config JSON.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default, deny_unknown_fields)]
pub struct WalRegisterConfig {
    pub distribution: Option<Distribution>,
    pub rotate: Option<i64>,
    pub blend_hue: Option<f64>,
    pub blend_light: Option<f64>,
    pub blend_chroma: Option<f64>,
    pub light: Option<f64>,
    pub chroma: Option<f64>,
}

/// Remap options; every one is opt-in.  Canonical keys kebab-case.  The
/// flattened extraction section feeds the arrangement stage's family
/// resolution (only needed when a register sets `distribution`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct RemapConfig {
    #[serde(flatten)]
    pub extraction: ExtractionParams,
    /// Facade: seeds `blend-hue` and `blend-chroma` where they are unset
    /// (see [`facade_defaults`]).
    pub harmonize: f64,
    /// Falloff width in `soft` territory (degrees on the hue wheel; must
    /// be positive there), influence cutoff for the blend and adoption
    /// gates in `hard` territory.
    pub reach_deg: f64,
    /// Channel-fidelity dials; defaults filled from the facade.
    pub blend_hue: Option<f64>,
    pub blend_chroma: Option<f64>,
    /// Lightness adoption is decoupled from the facade and defaults to 0
    /// (photographic lightness).
    pub blend_light: f64,
    /// Dithering strength (0 off; scales the mask offsets, resp. the
    /// diffusion).
    pub dithering: f64,
    /// Which ordered mask / diffusion kernel the strength applies to.
    pub dithering_mode: Option<DitherMode>,
    /// Territory mode: soft weighted mixing (default) or hard stylized
    /// snapping.
    pub territory: Option<Territory>,
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
            reach_deg: 45.0,
            blend_hue: None,
            blend_chroma: None,
            blend_light: 0.0,
            dithering: 0.0,
            dithering_mode: None,
            territory: None,
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
    pub blend_hue: f64,
    pub blend_light: f64,
    pub blend_chroma: f64,
    pub light: f64,
    pub chroma: f64,
}

/// The facade's seed rule: where the config leaves `blend-hue` /
/// `blend-chroma` unset they adopt `harmonize`; explicit dials win and
/// `blend-light` never follows the facade.  Returns (hue, light, chroma).
pub fn facade_defaults(config: &RemapConfig) -> (f64, f64, f64) {
    (
        config.blend_hue.unwrap_or(config.harmonize),
        config.blend_light,
        config.blend_chroma.unwrap_or(config.harmonize),
    )
}

/// The `globals // all // register` resolution, with validation.
pub fn resolved_registers(
    config: &RemapConfig,
) -> Result<BTreeMap<&'static str, WalRegisterSettings>, String> {
    if !(0.0..=1.0).contains(&config.dithering) {
        return Err("dithering must be within [0, 1]".to_string());
    }
    let territory = config.territory.unwrap_or(Territory::Soft);
    if territory == Territory::Soft && config.reach_deg <= 0.0 {
        return Err("territory 'soft' needs a positive reach (reach-deg)".to_string());
    }
    let all = config.registers.get("all").cloned().unwrap_or_default();
    let (seed_hue, seed_light, seed_chroma) = facade_defaults(config);
    let mut out = BTreeMap::new();
    for name in REGISTER_NAMES {
        let given = config.registers.get(name).cloned().unwrap_or_default();
        let distribution = resolve_distribution(
            name,
            given.distribution.or_else(|| all.distribution.clone()),
        )?;
        let rotate = given.rotate.or(all.rotate).unwrap_or(0);
        let blend_hue = given.blend_hue.or(all.blend_hue).unwrap_or(seed_hue);
        let blend_light = given.blend_light.or(all.blend_light).unwrap_or(seed_light);
        let blend_chroma = given
            .blend_chroma
            .or(all.blend_chroma)
            .unwrap_or(seed_chroma);
        let light = given.light.or(all.light).unwrap_or(config.light);
        let chroma = given.chroma.or(all.chroma).unwrap_or(config.chroma);
        if !(0.0..=1.0).contains(&blend_hue) {
            return Err(format!(
                "registers.{}.blend-hue must be within [0, 1]",
                name
            ));
        }
        if !(0.0..=1.0).contains(&blend_light) {
            return Err(format!(
                "registers.{}.blend-light must be within [0, 1]",
                name
            ));
        }
        if !(0.0..=1.0).contains(&blend_chroma) {
            return Err(format!(
                "registers.{}.blend-chroma must be within [0, 1]",
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
                blend_hue,
                blend_light,
                blend_chroma,
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

/// Per-slot mix/grade options: the slot's register's resolved settings.
#[derive(Debug, Clone, Copy)]
pub struct SlotKnobs {
    pub blend_hue: f64,
    pub blend_light: f64,
    pub blend_chroma: f64,
    pub light: f64,
    pub chroma: f64,
}

/// Per-slot options for the 16 canonical slots.
pub fn slot_knobs(regs: &BTreeMap<&'static str, WalRegisterSettings>) -> Vec<SlotKnobs> {
    BASE_SLOTS
        .iter()
        .map(|slot| {
            let s = &regs[register_of_slot(slot)];
            SlotKnobs {
                blend_hue: s.blend_hue,
                blend_light: s.blend_light,
                blend_chroma: s.blend_chroma,
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

/// True when no option is active anywhere: the output is then a re-encoded
/// passthrough of the input (matching the documented contract).  Decided
/// on the resolved values only - dithering is inert at zero adoption and
/// does not decide passivity.
pub fn is_passive(
    regs: &BTreeMap<&'static str, WalRegisterSettings>,
    image_light: f64,
    image_chroma: f64,
) -> bool {
    image_light == 0.0
        && (image_chroma - 1.0).abs() < 1e-9
        && regs.values().all(|r| {
            matches!(r.distribution, DistributionState::Off)
                && r.rotate == 0
                && r.blend_hue == 0.0
                && r.blend_light == 0.0
                && r.blend_chroma == 0.0
                && r.light == 0.0
                && (r.chroma - 1.0).abs() < 1e-9
        })
}

/// Shared per-pixel core.  Anchoring happens on the raw pixel, so slot
/// membership is independent of the dither; the mix runs on the effective
/// input (`l_in`/`c_in` = raw + accumulated dither), `jitter` scales the
/// adopted lightness/chroma share (the ordered-dither threshold; 1.0 in
/// diffusion mode, where error diffusion plays the dither).
struct PixelCore<'a> {
    slots: &'a [SlotPalette],
    neutral_slots: &'a [usize],
    knobs: &'a [SlotKnobs],
    config: &'a RemapConfig,
    territory: Territory,
}

impl PixelCore<'_> {
    /// Returns the final (l, c, h) before the 8-bit render, plus the
    /// anchor slot index.
    fn run(&self, raw: Lch, l_in: f64, c_in: f64, jitter: f64) -> (f64, f64, f64, usize) {
        let reach = self.config.reach_deg;
        let gray_floor = self.config.gray_chroma_floor;
        let (mut l2, mut c2, h2);
        let best_index;
        if raw.c < gray_floor {
            // Achromatic: keyed to the scheme's low-chroma slots by raw
            // lightness (first minimum wins).
            let mut best = 0usize;
            let mut best_d = f64::INFINITY;
            for &idx in self.neutral_slots {
                let d = (self.slots[idx].lch.l - raw.l).abs();
                if d < best_d {
                    best_d = d;
                    best = idx;
                }
            }
            best_index = best;
            let slots_target = self.slots[best].lch;
            let n = self.knobs[best];
            // In soft mode the target is the palette's weighted mean over
            // the neutral slots (by lightness closeness), not one slot's
            // the constant - the lightness ramp blends smoothly.
            let (target_l, target_c) = if self.territory == Territory::Soft {
                let t = reach.max(1e-3);
                let mut ls = Vec::new();
                let mut cs = Vec::new();
                let mut ws = Vec::new();
                let mut total = 0.0f64;
                for &idx in self.neutral_slots {
                    let s = self.slots[idx].lch;
                    let d = (s.l - raw.l).abs();
                    let v = (-(d * d) / (t * t)).exp();
                    ls.push(s.l);
                    cs.push(s.c);
                    ws.push(v);
                    total += v;
                }
                let mix = |xs: &Vec<f64>| {
                    xs.iter().zip(ws.iter()).map(|(x, w)| x * w).sum::<f64>() / total
                };
                (mix(&ls), mix(&cs))
            } else {
                (slots_target.l, slots_target.c)
            };
            l2 = l_in;
            if n.blend_light > 0.0 {
                l2 = l_in + (target_l - l_in) * n.blend_light * jitter;
            }
            if n.blend_chroma > 0.0 {
                c2 = (c_in + (target_c - c_in) * n.blend_chroma * jitter).max(0.0);
            } else {
                c2 = c_in;
            }
            // Achromatic pixels keep their raw hue: blending hue on a
            // colour with no chroma would be noise.
            h2 = raw.h;
        } else {
            // Chromatic: nearest slot by raw hue (first minimum), then
            // the opt-in blend moves, reach-gated in hard territory.
            let mut best = 0usize;
            let mut best_d = f64::INFINITY;
            for (idx, slots_target) in self.slots.iter().enumerate() {
                let d = circ_dist(raw.h, slots_target.lch.h);
                if d < best_d {
                    best_d = d;
                    best = idx;
                }
            }
            best_index = best;
            let target = self.slots[best].lch;
            let n = self.knobs[best];
            // In soft mode the targets are the palette's weighted means
            // over all slots by circular hue distance; influence falls off
            // with distance instead of cutting off at the reach, so the
            // hard-mode cutoff is bypassed.
            let (hue_t, l_t, c_t) = if self.territory == Territory::Soft {
                let t = reach.max(1e-3);
                let mut hues = Vec::new();
                let mut ls = Vec::new();
                let mut cs = Vec::new();
                let mut ws = Vec::new();
                let mut total = 0.0f64;
                for s in self.slots.iter() {
                    let d = circ_dist(raw.h, s.lch.h);
                    let v = (-(d * d) / (t * t)).exp();
                    hues.push(s.lch.h);
                    ls.push(s.lch.l);
                    cs.push(s.lch.c);
                    ws.push(v);
                    total += v;
                }
                if total > 1e-12 {
                    let mix = |xs: &Vec<f64>| {
                        xs.iter().zip(ws.iter()).map(|(x, w)| x * w).sum::<f64>() / total
                    };
                    (weighted_circ_mean(&hues, &ws), mix(&ls), mix(&cs))
                } else {
                    (target.h, target.l, target.c)
                }
            } else {
                (target.h, target.l, target.c)
            };
            if n.blend_hue > 0.0 && (self.territory == Territory::Soft || best_d <= reach) {
                h2 = circ_lerp(raw.h, hue_t, n.blend_hue);
            } else {
                h2 = raw.h;
            }
            if (n.blend_light > 0.0 || n.blend_chroma > 0.0)
                && (self.territory == Territory::Soft || best_d <= reach)
            {
                l2 = l_in + (l_t - l_in) * n.blend_light * jitter;
                c2 = (c_in + (c_t - c_in) * n.blend_chroma * jitter).max(0.0);
            } else {
                l2 = l_in;
                c2 = c_in;
            }
        }
        // Per-slot (register) grade, then the image-wide grade last.
        let n = self.knobs[best_index];
        l2 = (l2 + n.light).clamp(0.0, 1.0);
        c2 = (c2 * n.chroma).max(0.0);
        l2 = (l2 + self.config.light).clamp(0.0, 1.0);
        c2 = (c2 * self.config.chroma).max(0.0);
        (l2, c2, h2, best_index)
    }
}

/// Ordered-dither threshold scalar for one pixel.
fn ordered_jitter(mode: DitherMode, x: usize, y: usize, dither: f64) -> f64 {
    let mask = match mode {
        DitherMode::BlueNoise => (f64::from(BLUE_NOISE64[y % 64][x % 64]) + 0.5) / 256.0,
        DitherMode::Bayer => (f64::from(BAYER8[y % 8][x % 8]) + 0.5) / 64.0,
        _ => 0.5,
    };
    1.0 + (mask - 0.5) * dither
}

/// Error-diffusion kernel: (dx, dy, weight); weights sum to 1 for
/// Floyd-Steinberg and 3/4 for Atkinson (the dropped 2/8 is the point).
const FS_KERNEL: [(i64, i64, f64); 4] = [
    (1, 0, 7.0 / 16.0),
    (-1, 1, 3.0 / 16.0),
    (0, 1, 5.0 / 16.0),
    (1, 1, 1.0 / 16.0),
];
const ATKINSON_KERNEL: [(i64, i64, f64); 6] = [
    (1, 0, 1.0 / 8.0),
    (2, 0, 1.0 / 8.0),
    (-1, 1, 1.0 / 8.0),
    (0, 1, 1.0 / 8.0),
    (1, 1, 1.0 / 8.0),
    (0, 2, 1.0 / 8.0),
];

/// The full pipeline.  `pixels` is a flat rgb8 buffer of a
/// `width x height` image; `slots` the 16 palette slot colours in
/// canonical order (already arranged by `arrange_palette` when
/// distribution options are in play); `knobs` the per-slot mix/grade
/// values; `neutral_slots` the indices of the scheme's low-chroma slots
/// (achromatic pixels key against those by lightness rather than by hue).
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
    let core = PixelCore {
        slots,
        neutral_slots,
        knobs,
        config,
        territory: config.territory.unwrap_or(Territory::Soft),
    };
    let dither = config.dithering.clamp(0.0, 1.0);
    let mode = config.dithering_mode.unwrap_or(DitherMode::BlueNoise);

    let mut out = vec![0u8; pixels.len()];
    let mut counts = vec![0u64; slots.len()];

    match mode {
        DitherMode::BlueNoise | DitherMode::Bayer | DitherMode::None => {
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let i = (y * width as usize + x) * 3;
                    let raw = rgb_to_oklch(&[pixels[i], pixels[i + 1], pixels[i + 2]]);
                    let jitter = if mode == DitherMode::None || dither == 0.0 {
                        1.0
                    } else {
                        ordered_jitter(mode, x, y, dither)
                    };
                    let (l2, c2, h2, slot) = core.run(raw, raw.l, raw.c, jitter);
                    let back = oklch_to_rgb(&Lch {
                        l: l2,
                        c: c2,
                        h: h2,
                    });
                    out[i] = back[0];
                    out[i + 1] = back[1];
                    out[i + 2] = back[2];
                    counts[slot] += 1;
                }
            }
        }
        DitherMode::FloydSteinberg | DitherMode::Atkinson => {
            let kernel: &[(i64, i64, f64)] = match mode {
                DitherMode::FloydSteinberg => &FS_KERNEL,
                _ => &ATKINSON_KERNEL,
            };
            let w = width as usize;
            // Pending error for the current row (arrives from the row
            // above) and for the next one; serpentine scan flips the
            // kernel horizontally on left-to-right -> right-to-left rows.
            let mut row_err_l = vec![0.0f64; w];
            let mut row_err_c = vec![0.0f64; w];
            let mut next_err_l = vec![0.0f64; w];
            let mut next_err_c = vec![0.0f64; w];
            for y in 0..height as usize {
                let dir: i64 = if y % 2 == 0 { 1 } else { -1 };
                let mut x_it: i64 = if dir == 1 { 0 } else { w as i64 - 1 };
                for _ in 0..w {
                    let x = x_it as usize;
                    let i = (y * w + x) * 3;
                    let raw = rgb_to_oklch(&[pixels[i], pixels[i + 1], pixels[i + 2]]);
                    let l_in = (raw.l + row_err_l[x]).clamp(0.0, 1.0);
                    let c_in = (raw.c + row_err_c[x]).clamp(0.0, f64::MAX);
                    let (l2, c2, h2, slot) = core.run(raw, l_in, c_in, 1.0);
                    let res_l = l2 - l_in;
                    let res_c = c2 - c_in;
                    for &(dx, dy, weight) in kernel {
                        let tx_i = x_it + dx * dir;
                        if tx_i < 0 || tx_i >= w as i64 {
                            continue;
                        }
                        let ty = y + dy as usize;
                        if ty >= height as usize {
                            continue;
                        }
                        let tx = tx_i as usize;
                        if dy == 0 {
                            row_err_l[tx] += res_l * weight * dither;
                            row_err_c[tx] += res_c * weight * dither;
                        } else {
                            next_err_l[tx] += res_l * weight * dither;
                            next_err_c[tx] += res_c * weight * dither;
                        }
                    }
                    let back = oklch_to_rgb(&Lch {
                        l: l2,
                        c: c2,
                        h: h2,
                    });
                    out[i] = back[0];
                    out[i + 1] = back[1];
                    out[i + 2] = back[2];
                    counts[slot] += 1;
                    x_it += dir;
                }
                std::mem::swap(&mut row_err_l, &mut next_err_l);
                std::mem::swap(&mut row_err_c, &mut next_err_c);
                next_err_l.iter_mut().for_each(|v| *v = 0.0);
                next_err_c.iter_mut().for_each(|v| *v = 0.0);
            }
        }
    }

    RemapOutput {
        pixels: out,
        coverage: counts,
    }
}
