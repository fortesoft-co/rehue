//! `rehue wal`: the per-color remap (`ColorMap`), its modes, and the dither stage.
//!
//! Per-pixel palette remap: arrange (assign + cycle), then blend
//! toward the palette (hue, light and chroma channel fidelity), then
//! per-role grading, then image-wide grading.  Deterministic and
//! sandbox-pure.
//!
//! The `harmonize` facade is the one-dial surface: it seeds `match-hue`
//! and `match-chroma` wherever they are left unset, while explicit dials
//! win and `match-lightness` stays decoupled (default 0 - photographic
//! lightness, since adopting lightness flattens contrast).  An optional
//! last stage, `dither` ([`crate::dither`]), draws the graded image with
//! the palette's colors only, as a pattern.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::color::{Lch, circ_dist, circ_lerp, oklch_to_rgb, rgb_to_oklch, weighted_circ_mean};
use crate::dither::{Dither, Ink};
use crate::extract::{Cluster, ExtractionParams};
use crate::role::{
    Assign, AssignState, ROLE_NAMES, assign_hues, family_indices, resolve_assign, role_of_slot,
    role_slots, slot_index,
};
use crate::scheme::BASE_SLOTS;

/// The `mode` option: how pixels relate to the palette.
///
/// **snap** (opt-in): every pixel anchors to its nearest slot and adopts
/// that slot's constants - fast, stylized, and exactly the mechanism that
/// bands and hard edges come from.
///
/// **blend** (the default): slot influence falls off with distance instead of cutting off
/// (circular hue distance for chromatic pixels, lightness distance for
/// achromatic ones); every target is the weighted mean over the palette.
/// Boundaries between slots become smooth crossings and the flat-constant
/// posterize largely disappears by construction.  Blend bypasses the
/// snap-mode cutoffs: `reach` acts as the falloff temperature there
/// (and must be positive), and as the influence cutoff in snap mode.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// The default: continuous weighted mixing (palette as influence field).
    Blend,
    /// Opt-in stylized mode: nearest-slot snapping + flat-constant
    /// adoption (the flat look with posterize and hard edges, by design).
    Snap,
}

/// What the scheme's neutral slots (base00-07) do for colorful pixels.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Neutrals {
    /// The default: in blend the neutrals only set the chroma floor;
    /// hue and vivid color come from the accents, and chroma follows how
    /// colorful each pixel is within its image.  In snap, colorful
    /// pixels snap to the accents only.
    #[default]
    Floor,
    /// The neutrals join in for hue and chroma like any slot: the image
    /// takes on the scheme's overall cast, calmer and faded toward its
    /// grays (rehue's blend and snap before 0.3).
    Mix,
}

impl Neutrals {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Floor => "floor",
            Self::Mix => "mix",
        }
    }
}

impl Mode {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Blend => "blend",
            Self::Snap => "snap",
        }
    }
}

/// Per-role override; every field optional (the
/// `globals // all // role` merge fills the gaps).  Canonical keys are
/// kebab-case in the config JSON.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default, deny_unknown_fields)]
pub struct WalRoleConfig {
    pub assign: Option<Assign>,
    pub cycle: Option<i64>,
    pub match_hue: Option<f64>,
    pub match_lightness: Option<f64>,
    pub match_chroma: Option<f64>,
    pub lightness: Option<f64>,
    pub chroma: Option<f64>,
}

/// Remap options; every one is opt-in.  Canonical keys kebab-case.  The
/// flattened extraction section feeds the arrangement stage's family
/// resolution (only needed when a role sets `assign`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct WalConfig {
    #[serde(flatten)]
    pub extraction: ExtractionParams,
    /// Facade: seeds `match-hue` and `match-chroma` where they are unset
    /// (see [`facade_defaults`]).
    pub harmonize: f64,
    /// Falloff width in `blend` mode (degrees on the hue wheel; must
    /// be positive there), influence cutoff for the blend and adoption
    /// gates in `snap` mode.
    pub reach: f64,
    /// Channel-fidelity dials; defaults filled from the facade.
    pub match_hue: Option<f64>,
    pub match_chroma: Option<f64>,
    /// Lightness adoption is decoupled from the facade and defaults to 0
    /// (photographic lightness).
    pub match_lightness: f64,
    /// The last stage: draw the graded image with palette colors only,
    /// as a pattern (unset = no dithering).
    pub dither: Option<Dither>,
    /// How pixels find their palette color: `blend` (weighted mixing,
    /// the default) or `snap` (nearest slot).
    pub mode: Option<Mode>,
    /// What the scheme's neutral slots do in `blend` mode: `floor`
    /// (the default) or `mix`.
    pub neutrals: Neutrals,
    pub max_gray_chroma: f64,
    pub lightness: f64,
    pub chroma: f64,
    /// Which scheme colors pixels may be pulled toward, and how hard:
    /// keys are slots (`base0D`), roles (`accents`, `ac`) or `all`,
    /// values weights.  Unset = every slot at weight 1; once set,
    /// unlisted slots are left out (see [`palette_weights`]).
    pub palette: Option<HashMap<String, f64>>,
    pub roles: HashMap<String, WalRoleConfig>,
}

impl Default for WalConfig {
    fn default() -> Self {
        Self {
            extraction: ExtractionParams::default(),
            harmonize: 0.0,
            reach: 45.0,
            match_hue: None,
            match_chroma: None,
            match_lightness: 0.0,
            dither: None,
            mode: None,
            neutrals: Neutrals::Floor,
            max_gray_chroma: 0.02,
            lightness: 0.0,
            chroma: 1.0,
            palette: None,
            roles: HashMap::new(),
        }
    }
}

/// Resolved per-role settings the wall pipeline runs with.
#[derive(Debug, Clone)]
pub struct WalRoleSettings {
    pub assign: AssignState,
    pub cycle: i64,
    pub match_hue: f64,
    pub match_lightness: f64,
    pub match_chroma: f64,
    pub lightness: f64,
    pub chroma: f64,
}

/// The facade's seed rule: where the config leaves `match-hue` /
/// `match-chroma` unset they adopt `harmonize`; explicit dials win and
/// `match-lightness` never follows the facade.  Returns (hue, light, chroma).
pub fn facade_defaults(config: &WalConfig) -> (f64, f64, f64) {
    (
        config.match_hue.unwrap_or(config.harmonize),
        config.match_lightness,
        config.match_chroma.unwrap_or(config.harmonize),
    )
}

/// The canonical slot for a typed slot name (`base0D`, any case).
fn canonical_slot(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    BASE_SLOTS.iter().copied().find(|s| *s == lower)
}

/// The per-slot weights of the `palette` option, in canonical slot order.
///
/// No palette: every slot at 1.  Otherwise unlisted slots are 0 (left
/// out) and the most specific key wins: `all`, then a role, then a slot.
/// Weights are finite and >= 0, and at least one slot must stay in.
pub fn palette_weights(config: &WalConfig) -> Result<[f64; 16], String> {
    let Some(palette) = &config.palette else {
        return Ok([1.0; 16]);
    };
    let mut all = None;
    let mut roles: HashMap<&'static str, f64> = HashMap::new();
    let mut slots: HashMap<&'static str, f64> = HashMap::new();
    let mut keys: Vec<&String> = palette.keys().collect();
    keys.sort();
    for key in keys {
        let w = palette[key];
        if !w.is_finite() || w < 0.0 {
            return Err(format!(
                "palette: '{key}' needs a weight of 0 or more (got {w})"
            ));
        }
        let twice = || format!("palette: '{key}' is given twice");
        if let Some(slot) = canonical_slot(key) {
            if slots.insert(slot, w).is_some() {
                return Err(twice());
            }
        } else {
            match crate::role::canonical_role(key) {
                Some("all") => {
                    if all.replace(w).is_some() {
                        return Err(twice());
                    }
                }
                Some(role) => {
                    if roles.insert(role, w).is_some() {
                        return Err(twice());
                    }
                }
                None => {
                    return Err(format!(
                        "palette: '{key}' is not a slot (base00..base0F) or a role ({})",
                        crate::role::role_list()
                    ));
                }
            }
        }
    }
    let mut weights = [0.0; 16];
    for (i, slot) in BASE_SLOTS.iter().enumerate() {
        weights[i] = slots
            .get(slot)
            .or_else(|| roles.get(role_of_slot(slot)))
            .copied()
            .or(all)
            .unwrap_or(0.0);
    }
    if weights.iter().all(|w| *w == 0.0) {
        return Err(
            "palette: every slot is left out; give at least one a weight above 0".to_string(),
        );
    }
    Ok(weights)
}

/// The `palette` option's command-line form: comma-separated keys, each
/// bare (weight 1) or `key=weight`: `ac`, `base0D,base0E`,
/// `all,base0D=6`.
pub fn parse_palette(text: &str) -> Result<HashMap<String, f64>, String> {
    let mut out = HashMap::new();
    for part in text.split(',').map(str::trim) {
        if part.is_empty() {
            return Err(format!("expected keys like ac or base0D=6, got '{text}'"));
        }
        let (key, weight) = match part.split_once('=') {
            Some((k, w)) => {
                let w: f64 = w
                    .trim()
                    .parse()
                    .map_err(|_| format!("'{part}': the weight is not a number"))?;
                (k.trim(), w)
            }
            None => (part, 1.0),
        };
        if out.insert(key.to_string(), weight).is_some() {
            return Err(format!("'{key}' is given twice"));
        }
    }
    Ok(out)
}

/// The `globals // all // role` resolution, with validation.
pub fn resolved_roles(
    config: &WalConfig,
) -> Result<BTreeMap<&'static str, WalRoleSettings>, String> {
    let mode = config.mode.unwrap_or(Mode::Blend);
    if mode == Mode::Blend && config.reach <= 0.0 {
        return Err("mode 'blend' needs a positive reach".to_string());
    }
    palette_weights(config)?;
    let roles = crate::role::canonical_roles(&config.roles)?;
    let all = roles.get("all").cloned().unwrap_or_default();
    let (seed_hue, seed_light, seed_chroma) = facade_defaults(config);
    let mut out = BTreeMap::new();
    for name in ROLE_NAMES {
        let given = roles.get(name).cloned().unwrap_or_default();
        let assign = resolve_assign(name, given.assign.or_else(|| all.assign.clone()))?;
        let cycle = given.cycle.or(all.cycle).unwrap_or(0);
        let match_hue = given.match_hue.or(all.match_hue).unwrap_or(seed_hue);
        let match_lightness = given
            .match_lightness
            .or(all.match_lightness)
            .unwrap_or(seed_light);
        let match_chroma = given
            .match_chroma
            .or(all.match_chroma)
            .unwrap_or(seed_chroma);
        let light = given
            .lightness
            .or(all.lightness)
            .unwrap_or(config.lightness);
        let chroma = given.chroma.or(all.chroma).unwrap_or(config.chroma);
        if !(0.0..=1.0).contains(&match_hue) {
            return Err(format!("roles.{}.match-hue must be within [0, 1]", name));
        }
        if !(0.0..=1.0).contains(&match_lightness) {
            return Err(format!(
                "roles.{}.match-lightness must be within [0, 1]",
                name
            ));
        }
        if !(0.0..=1.0).contains(&match_chroma) {
            return Err(format!("roles.{}.match-chroma must be within [0, 1]", name));
        }
        if chroma <= 0.0 {
            return Err(format!("roles.{}.chroma must be positive", name));
        }
        out.insert(
            name,
            WalRoleSettings {
                assign,
                cycle,
                match_hue,
                match_lightness,
                match_chroma,
                lightness: light,
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

/// Per-slot mix/grade options: the slot's role's resolved settings.
#[derive(Debug, Clone, Copy)]
pub struct SlotOptions {
    pub match_hue: f64,
    pub match_lightness: f64,
    pub match_chroma: f64,
    pub lightness: f64,
    pub chroma: f64,
    /// The slot's `palette` weight: its pull in the match (0 = left out).
    pub weight: f64,
}

/// Per-slot options for the 16 canonical slots: the slot's role's
/// settings and its `palette` weight.
pub fn slot_options(
    regs: &BTreeMap<&'static str, WalRoleSettings>,
    config: &WalConfig,
) -> Result<Vec<SlotOptions>, String> {
    let weights = palette_weights(config)?;
    Ok(BASE_SLOTS
        .iter()
        .zip(weights)
        .map(|(slot, weight)| {
            let s = &regs[role_of_slot(slot)];
            SlotOptions {
                match_hue: s.match_hue,
                match_lightness: s.match_lightness,
                match_chroma: s.match_chroma,
                lightness: s.lightness,
                chroma: s.chroma,
                weight,
            }
        })
        .collect())
}

/// The arrangement stage - shared semantics with the scheme side: each
/// role's `assign` recolours its slots' hues from the wallpaper's
/// families (lightness and chroma stay slot-local), then `cycle` permutes
/// the role's hues across its slots.  A palette with no hues extracted
/// (e.g. greyscale wallpaper, or no role distributes) passes through.
pub fn arrange_palette(
    slots: &[SlotPalette],
    clusters: &[Cluster],
    regs: &BTreeMap<&'static str, WalRoleSettings>,
) -> Result<Vec<SlotPalette>, String> {
    let mut out = slots.to_vec();
    for name in ROLE_NAMES {
        let settings = &regs[name];
        let reg_slots = role_slots(name);
        let k = reg_slots.len();
        if !clusters.is_empty() && settings.assign != AssignState::Off {
            let stops = match &settings.assign {
                AssignState::Auto => (0..k.min(clusters.len())).collect::<Vec<usize>>(),
                AssignState::Pin(i) => family_indices(&[*i], clusters, name)?,
                AssignState::Stops(v) => family_indices(v, clusters, name)?,
                AssignState::Off => unreachable!("checked above"),
            };
            let hues = assign_hues(k, &stops, clusters);
            for (pos, slot) in reg_slots.iter().copied().enumerate() {
                out[slot_index(slot)].lch.h = hues[pos];
            }
        }
        let rotated_by = settings.cycle.rem_euclid(k as i64) as usize;
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
pub struct WalOutput {
    pub pixels: Vec<u8>,
    /// matched-pixel counts per slot, in canonical base order
    pub coverage: Vec<u64>,
}

/// True when no option is active anywhere: the output is then a re-encoded
/// passthrough of the input (matching the documented contract).  Decided
/// on the grading values only; a `dither` stage still runs on a passive
/// grade (the caller checks it).
pub fn is_passive(
    regs: &BTreeMap<&'static str, WalRoleSettings>,
    image_light: f64,
    image_chroma: f64,
) -> bool {
    image_light == 0.0
        && (image_chroma - 1.0).abs() < 1e-9
        && regs.values().all(|r| {
            matches!(r.assign, AssignState::Off)
                && r.cycle == 0
                && r.match_hue == 0.0
                && r.match_lightness == 0.0
                && r.match_chroma == 0.0
                && r.lightness == 0.0
                && (r.chroma - 1.0).abs() < 1e-9
        })
}

/// The per-colour remap: one palette, one set of dials.  The result
/// depends only on the input colour (`map_rgb`), which is what lets the
/// same grade be baked into a LUT or applied to anything else made of
/// colours.  `apply` drives it per pixel, then runs the optional dither
/// stage, which depends on position.
pub struct ColorMap<'a> {
    slots: &'a [SlotPalette],
    neutral_slots: &'a [usize],
    options: &'a [SlotOptions],
    config: &'a WalConfig,
    mode: Mode,
    vivid: f64,
}

/// The chroma a "fully vivid" pixel has when no image says otherwise
/// (a saturated photo's 95th percentile is around here).
pub const DEFAULT_VIVID: f64 = 0.2;

/// An image's vivid chroma: the 95th percentile of its pixels' chroma
/// (at least 0.02), from a histogram, so it costs one pass and no
/// memory per pixel.  Blend mode measures each pixel's colorfulness
/// against it.
pub fn vivid_chroma(pixels: &[u8]) -> f64 {
    const BINS: usize = 1024;
    const MAX: f64 = 0.4;
    let mut hist = [0u64; BINS];
    let mut n = 0u64;
    for px in pixels.as_chunks::<3>().0 {
        let c = rgb_to_oklch(&[px[0], px[1], px[2]]).c;
        hist[((c / MAX * BINS as f64) as usize).min(BINS - 1)] += 1;
        n += 1;
    }
    if n == 0 {
        return DEFAULT_VIVID;
    }
    let want = n * 95 / 100;
    let mut seen = 0u64;
    for (i, count) in hist.iter().enumerate() {
        seen += count;
        if seen > want {
            return ((i as f64 + 0.5) / BINS as f64 * MAX).max(0.02);
        }
    }
    MAX
}

impl<'a> ColorMap<'a> {
    /// `slots` is the (arranged) palette, `options` its per-slot settings
    /// (`slot_options`), `neutral_slots` the low-chroma slots grays key to.
    pub fn new(
        slots: &'a [SlotPalette],
        neutral_slots: &'a [usize],
        options: &'a [SlotOptions],
        config: &'a WalConfig,
    ) -> Self {
        debug_assert_eq!(slots.len(), options.len());
        ColorMap {
            slots,
            neutral_slots,
            options,
            config,
            mode: config.mode.unwrap_or(Mode::Blend),
            vivid: DEFAULT_VIVID,
        }
    }

    /// Measure colorfulness against this chroma (the image's
    /// [`vivid_chroma`]); `apply` sets it from the image, and a LUT baked
    /// for an image should use the same value.
    pub fn with_vivid(mut self, vivid: f64) -> Self {
        self.vivid = vivid.max(1e-3);
        self
    }

    /// One colour through the remap: exactly what `apply` produces for a
    /// pixel of this colour without a dither stage.
    pub fn map_rgb(&self, rgb: &[u8; 3]) -> [u8; 3] {
        let raw = rgb_to_oklch(rgb);
        let (l, c, h, _) = self.run(raw);
        oklch_to_rgb(&Lch { l, c, h })
    }

    /// Returns the final (l, c, h) before the 8-bit render, plus the
    /// anchor slot index (`None` for a gray pixel when the palette
    /// leaves every neutral slot out: it passes through ungraded).
    ///
    /// Palette weights divide a slot's distance, in both modes: a
    /// weight-2 slot pulls from twice as far in blend and claims an arc
    /// twice as wide when picking the nearest slot.  The snap-mode reach
    /// gate still measures the real distance.  Weight 0 slots take no
    /// part at all.
    fn run(&self, raw: Lch) -> (f64, f64, f64, Option<usize>) {
        let (l_in, c_in) = (raw.l, raw.c);
        let reach = self.config.reach;
        let gray_floor = self.config.max_gray_chroma;
        let weight = |idx: usize| self.options[idx].weight;
        let (mut l2, mut c2, h2);
        let best_index;
        if raw.c < gray_floor {
            // Achromatic: keyed to the scheme's low-chroma slots by raw
            // lightness (first minimum wins).
            let neutrals: Vec<usize> = self
                .neutral_slots
                .iter()
                .copied()
                .filter(|&idx| weight(idx) > 0.0)
                .collect();
            if neutrals.is_empty() {
                // The palette left the neutrals out: grays stay gray.
                let (l, c) = image_grade(self.config, l_in, c_in);
                return (l, c, raw.h, None);
            }
            let mut best = neutrals[0];
            let mut best_d = f64::INFINITY;
            for &idx in &neutrals {
                let d = (self.slots[idx].lch.l - raw.l).abs() / weight(idx);
                if d < best_d {
                    best_d = d;
                    best = idx;
                }
            }
            best_index = best;
            let slots_target = self.slots[best].lch;
            let n = self.options[best];
            // In blend mode the target is the palette's weighted mean over
            // the neutral slots (by lightness closeness), not one slot's
            // the constant - the lightness ramp blends smoothly.
            let (target_l, target_c) = if self.mode == Mode::Blend {
                let t = reach.max(1e-3);
                let mut ls = Vec::new();
                let mut cs = Vec::new();
                let mut ws = Vec::new();
                let mut total = 0.0f64;
                for &idx in &neutrals {
                    let s = self.slots[idx].lch;
                    let d = (s.l - raw.l).abs() / weight(idx);
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
            if n.match_lightness > 0.0 {
                l2 = l_in + (target_l - l_in) * n.match_lightness;
            }
            if n.match_chroma > 0.0 {
                c2 = (c_in + (target_c - c_in) * n.match_chroma).max(0.0);
            } else {
                c2 = c_in;
            }
            // Achromatic pixels keep their raw hue: blending hue on a
            // colour with no chroma would be noise.
            h2 = raw.h;
        } else {
            // Chromatic: nearest slot by raw hue (first minimum), then
            // the opt-in blend moves, reach-gated in snap mode.
            //
            // Snap picks among the palette's accents (every palette slot
            // when it has none): a gray's hue is near-arbitrary, and a
            // warm or cool gray would otherwise catch the colors beside
            // its hue and drain them.  `neutrals = mix` keeps every slot
            // in the running.  Blend's pick only chooses which role's
            // options apply; its hue already comes from the accents.
            let accent = |i: usize| role_of_slot(self.slots[i].slot) == "accents";
            let accents_only = self.mode == Mode::Snap
                && self.config.neutrals != Neutrals::Mix
                && (0..self.slots.len()).any(|i| accent(i) && weight(i) > 0.0);
            let mut best = 0usize;
            let mut best_score = f64::INFINITY;
            let mut best_d = f64::INFINITY;
            for (idx, slots_target) in self.slots.iter().enumerate() {
                let w = weight(idx);
                if w <= 0.0 || (accents_only && !accent(idx)) {
                    continue;
                }
                let d = circ_dist(raw.h, slots_target.lch.h);
                if d / w < best_score {
                    best_score = d / w;
                    best_d = d;
                    best = idx;
                }
            }
            best_index = best;
            let target = self.slots[best].lch;
            let n = self.options[best];
            // Blend mode: weighted means by circular hue distance, so
            // influence falls off with distance instead of cutting off at
            // the reach (the snap-mode cutoff is bypassed).
            //
            // * hue: from the accents (base08-0F) in the palette, every
            //   palette slot when it has none, so the near-arbitrary hues
            //   of the scheme's grays never steer a color;
            // * lightness: from every palette slot;
            // * chroma: by how colorful the pixel is within the image (its
            //   chroma against the image's vivid chroma), between the
            //   palette's neutral chroma and the accents': muted areas
            //   settle at the scheme's neutral level, as the surfaces of
            //   a themed desktop do, and the vivid details reach accent
            //   strength.  A palette with no neutrals gives every pixel
            //   accent strength.
            //
            // `neutrals = mix` counts every slot as an accent here: hue,
            // lightness and chroma are then plain means over the whole
            // palette (the faded cast toward the scheme's grays).
            let (hue_t, l_t, c_t) = if self.mode == Mode::Blend {
                let t = reach.max(1e-3);
                let mix_neutrals = self.config.neutrals == Neutrals::Mix;
                let accent =
                    |i: usize| mix_neutrals || role_of_slot(self.slots[i].slot) == "accents";
                let has_accents = (0..self.slots.len()).any(|i| accent(i) && weight(i) > 0.0);
                let (mut ls, mut lw) = (Vec::new(), Vec::new());
                let (mut hues, mut cs, mut hw) = (Vec::new(), Vec::new(), Vec::new());
                let (mut neutral_c, mut neutral_w) = (0.0f64, 0.0f64);
                for (idx, s) in self.slots.iter().enumerate() {
                    let w = weight(idx);
                    if w <= 0.0 {
                        continue;
                    }
                    let d = circ_dist(raw.h, s.lch.h) / w;
                    let v = (-(d * d) / (t * t)).exp();
                    ls.push(s.lch.l);
                    lw.push(v);
                    if !accent(idx) {
                        neutral_c += s.lch.c * w;
                        neutral_w += w;
                    }
                    if !has_accents || accent(idx) {
                        hues.push(s.lch.h);
                        cs.push(s.lch.c);
                        hw.push(v);
                    }
                }
                let mean = |xs: &[f64], ws: &[f64]| {
                    let total: f64 = ws.iter().sum();
                    (total > 1e-12)
                        .then(|| xs.iter().zip(ws).map(|(x, w)| x * w).sum::<f64>() / total)
                };
                match (mean(&ls, &lw), mean(&cs, &hw)) {
                    (Some(l), Some(accent_c)) => {
                        let c = if neutral_w > 0.0 {
                            let floor = neutral_c / neutral_w;
                            let r = (raw.c / self.vivid).clamp(0.0, 1.0);
                            floor + r * (accent_c - floor)
                        } else {
                            accent_c
                        };
                        (weighted_circ_mean(&hues, &hw), l, c)
                    }
                    _ => (target.h, target.l, target.c),
                }
            } else {
                (target.h, target.l, target.c)
            };
            if n.match_hue > 0.0 && (self.mode == Mode::Blend || best_d <= reach) {
                h2 = circ_lerp(raw.h, hue_t, n.match_hue);
            } else {
                h2 = raw.h;
            }
            if (n.match_lightness > 0.0 || n.match_chroma > 0.0)
                && (self.mode == Mode::Blend || best_d <= reach)
            {
                l2 = l_in + (l_t - l_in) * n.match_lightness;
                c2 = (c_in + (c_t - c_in) * n.match_chroma).max(0.0);
            } else {
                l2 = l_in;
                c2 = c_in;
            }
        }
        // Per-slot (role) grade, then the image-wide grade last.
        let n = self.options[best_index];
        l2 = (l2 + n.lightness).clamp(0.0, 1.0);
        c2 = (c2 * n.chroma).max(0.0);
        let (l, c) = image_grade(self.config, l2, c2);
        (l, c, h2, Some(best_index))
    }
}

/// The image-wide grade, applied last to every pixel.
fn image_grade(config: &WalConfig, l: f64, c: f64) -> (f64, f64) {
    (
        (l + config.lightness).clamp(0.0, 1.0),
        (c * config.chroma).max(0.0),
    )
}

/// The full pipeline.  `pixels` is a flat rgb8 buffer of a
/// `width x height` image; `slots` the 16 palette slot colours in
/// canonical order (already arranged by `arrange_palette` when
/// assign options are in play); `options` the per-slot mix/grade
/// values; `neutral_slots` the indices of the scheme's low-chroma slots
/// (achromatic pixels key against those by lightness rather than by hue).
pub fn apply(
    pixels: &[u8],
    width: u32,
    height: u32,
    slots: &[SlotPalette],
    neutral_slots: &[usize],
    options: &[SlotOptions],
    config: &WalConfig,
) -> WalOutput {
    debug_assert_eq!(slots.len(), options.len());
    debug_assert_eq!(
        pixels.len(),
        width as usize * height as usize * 3,
        "flat buffer must match dimensions"
    );
    let core =
        ColorMap::new(slots, neutral_slots, options, config).with_vivid(vivid_chroma(pixels));
    let mut out = vec![0u8; pixels.len()];
    let mut counts = vec![0u64; slots.len()];
    for (src, dst) in pixels
        .as_chunks::<3>()
        .0
        .iter()
        .zip(out.as_chunks_mut::<3>().0.iter_mut())
    {
        let raw = rgb_to_oklch(src);
        let (l, c, h, slot) = core.run(raw);
        *dst = oklch_to_rgb(&Lch { l, c, h });
        if let Some(slot) = slot {
            counts[slot] += 1;
        }
    }
    if let Some(d) = &config.dither {
        (out, counts) = dither_stage(&out, width, height, slots, options, d);
    }
    WalOutput {
        pixels: out,
        coverage: counts,
    }
}

/// The dither stage over a graded image: the palette's colors (the slots
/// the palette keeps) drawn as a pattern.  Coverage counts each slot's
/// pixels exactly, since every pixel is one of them.
pub fn dither_stage(
    pixels: &[u8],
    width: u32,
    height: u32,
    slots: &[SlotPalette],
    options: &[SlotOptions],
    d: &Dither,
) -> (Vec<u8>, Vec<u64>) {
    let mut inks: Vec<Ink> = slots
        .iter()
        .enumerate()
        .filter(|(i, _)| options[*i].weight > 0.0)
        .map(|(i, s)| Ink::new(i, oklch_to_rgb(&s.lch)))
        .collect();
    if inks.is_empty() {
        inks = slots
            .iter()
            .enumerate()
            .map(|(i, s)| Ink::new(i, oklch_to_rgb(&s.lch)))
            .collect();
    }
    let (out, per_ink) = crate::dither::dither(pixels, width, height, &inks, d);
    let mut counts = vec![0u64; slots.len()];
    for (ink, n) in inks.iter().zip(per_ink) {
        counts[ink.slot] += n;
    }
    (out, counts)
}
