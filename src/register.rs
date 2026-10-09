//! The register pipeline: assign (legacy policy or distribution) -> rotate
//! -> blend-hue -> grade -> legibility guard.  Every stage is
//! deterministic and the wallpaper supplies hues only, so every slot's
//! lightness and chroma targets come from the reference scheme.
//!
//! Arrangement (distribution + rotate) is shared with `map_wal`, which
//! reshapes a palette's slot hues with the same rules before the
//! per-pixel work.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::color::{Lch, circ_dist, circ_lerp, hex_to_oklch, oklch_to_hex, round_value};
use crate::extract::{Cluster, ExtractionParams};
use crate::scheme::BASE_SLOTS;

pub const REGISTER_NAMES: [&str; 4] = ["bg", "surfaces", "fg", "accents"];

/// Slot -> register ("bg" covers base00; surfaces base01-03; fg base04-07;
/// accents base08-0F).
pub fn register_of_slot(slot: &str) -> &'static str {
    let i = u32::from_str_radix(&slot[4..], 16).expect("base16 slot has a hex index");
    if i == 0 {
        "bg"
    } else if i <= 3 {
        "surfaces"
    } else if i <= 7 {
        "fg"
    } else {
        "accents"
    }
}

pub fn register_slots(name: &str) -> &'static [&'static str] {
    match name {
        "bg" => &[BASE_SLOTS[0]],
        "surfaces" => &[BASE_SLOTS[1], BASE_SLOTS[2], BASE_SLOTS[3]],
        "fg" => &[BASE_SLOTS[4], BASE_SLOTS[5], BASE_SLOTS[6], BASE_SLOTS[7]],
        _ => &BASE_SLOTS[8..16],
    }
}

pub fn slot_index(slot: &str) -> usize {
    BASE_SLOTS
        .iter()
        .position(|s| *s == slot)
        .expect("canonical slot name")
}

/// The `distribution` option: how a register's slots pick up the wallpaper's
/// colour families.  Family indices are positions in the extraction output
/// (weight-ordered, heaviest first) - what `rehue inspect` prints.
///
/// * `true`  - the default ramp: families in extraction order, spreading
///   `min(slots, families)` evenly across the register span, shortest-arc
///   interpolation between them.
/// * integer - pin the register to that one family index (the single-slot
///   `bg` accepts only this form).
/// * array   - explicit stops in user order; at most one per register slot
///   (longer is an error).  A full-length array is a pure per-slot
///   assignment - manual mode - and duplicate indices are legal, giving
///   flat runs.
///
/// Hue only, always: every slot keeps the reference scheme's lightness and
/// chroma.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Distribution {
    /// Explicit switch for the default ramp; `false` counts as unset.
    Auto(bool),
    /// Pin the register to one extracted family.
    Pin(i64),
    /// Explicit stops in user order (family indices).
    Stops(Vec<i64>),
}

/// Resolved distribution intent, after static (slot-count) validation.
/// Family-index validation needs the extracted families and happens where
/// they are known.
#[derive(Debug, Clone, PartialEq)]
pub enum DistributionState {
    /// No distribution set: the register's legacy assignment behaviour.
    Off,
    /// Default ramp.
    Auto,
    /// Pin to one family index.
    Pin(i64),
    /// Explicit stops (family indices) in user order.
    Stops(Vec<i64>),
}

impl DistributionState {
    /// Wire-form summary for logs and reports.
    pub fn describe(&self) -> String {
        match self {
            Self::Off => "off".to_string(),
            Self::Auto => "auto".to_string(),
            Self::Pin(i) => format!("pin {}", i),
            Self::Stops(v) => {
                let joined = v
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                format!("[{}]", joined)
            }
        }
    }
}

/// Wire form: every field optional so records merge per key, exactly the
/// `defaults // all // register` dictionary merge.  Canonical keys are
/// kebab-case in the config JSON.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default, deny_unknown_fields)]
pub struct RegisterConfig {
    pub distribution: Option<Distribution>,
    pub rotate: Option<i64>,
    pub blend_hue: Option<f64>,
    pub light: Option<f64>,
    pub chroma: Option<f64>,
}

/// Resolved per-register settings the pipeline runs with.
#[derive(Debug, Clone)]
pub struct RegisterSettings {
    pub distribution: DistributionState,
    pub rotate: i64,
    pub blend_hue: f64,
    pub light: f64,
    pub chroma: f64,
}

/// Map-scheme configuration: extraction options, `reach-deg` (the accent
/// claim gate: how far a family hue may sit from an accent slot's hue and
/// still claim it) and register overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct MapConfig {
    #[serde(flatten)]
    pub extraction: ExtractionParams,
    pub reach_deg: f64,
    pub registers: HashMap<String, RegisterConfig>,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            extraction: ExtractionParams::default(),
            reach_deg: 45.0,
            registers: HashMap::new(),
        }
    }
}

/// Static validation of one register's `distribution` against its slot
/// count.
pub fn resolve_distribution(
    name: &str,
    given: Option<Distribution>,
) -> Result<DistributionState, String> {
    let k = register_slots(name).len();
    let state = match given {
        None | Some(Distribution::Auto(false)) => DistributionState::Off,
        Some(Distribution::Auto(true)) => DistributionState::Auto,
        Some(Distribution::Pin(i)) => DistributionState::Pin(i),
        Some(Distribution::Stops(v)) => {
            if v.is_empty() {
                return Err(format!(
                    "register '{}': 'distribution' array needs at least one stop",
                    name
                ));
            }
            if v.len() > k {
                return Err(format!(
                    "register '{}': 'distribution' has {} stop(s) but the register has {} slot(s)",
                    name,
                    v.len(),
                    k
                ));
            }
            DistributionState::Stops(v)
        }
    };
    if matches!(state, DistributionState::Auto | DistributionState::Stops(_)) && k == 1 {
        return Err(format!(
            "register '{}' has 1 slot: 'distribution' takes a family index (int), not true or an array",
            name
        ));
    }
    Ok(state)
}

/// The `defaults // all // register` resolution, with option validation.
pub fn resolved_registers(
    config: &MapConfig,
) -> Result<BTreeMap<&'static str, RegisterSettings>, String> {
    let all = config.registers.get("all").cloned().unwrap_or_default();
    let mut out = BTreeMap::new();
    for name in REGISTER_NAMES {
        let given = config.registers.get(name).cloned().unwrap_or_default();
        let distribution = resolve_distribution(
            name,
            given.distribution.or_else(|| all.distribution.clone()),
        )?;
        let rotate = given.rotate.or(all.rotate).unwrap_or(0);
        let blend_hue = given.blend_hue.or(all.blend_hue).unwrap_or(1.0);
        let light = given.light.or(all.light).unwrap_or(0.0);
        let chroma = given.chroma.or(all.chroma).unwrap_or(1.0);
        if !(0.0..=1.0).contains(&blend_hue) {
            return Err(format!(
                "registers.{}.blend-hue must be within [0, 1]",
                name
            ));
        }
        if chroma <= 0.0 {
            return Err(format!("registers.{}.chroma must be positive", name));
        }
        out.insert(
            name,
            RegisterSettings {
                distribution,
                rotate,
                blend_hue,
                light,
                chroma,
            },
        );
    }
    Ok(out)
}

/// Hue for every slot of one register from an explicit stop list.
///
/// Stops spread evenly across the register's slot span; slots between
/// stops take shortest-arc interpolation of the neighbouring stops' hues.
/// `stops.len() == k` degenerates to a pure per-slot assignment (manual
/// mode); duplicate stops give flat runs.
pub fn distribution_hues(k: usize, stops: &[usize], families: &[Cluster]) -> Vec<f64> {
    let m = stops.len();
    debug_assert!(m >= 1 && m <= k);
    (0..k)
        .map(|i| {
            if m == 1 {
                return families[stops[0]].hue;
            }
            let step = (k - 1) as f64 / (m - 1) as f64;
            let x = i as f64 / step;
            let j = (x.floor() as usize).min(m - 1);
            let t = x - j as f64;
            circ_lerp(
                families[stops[j]].hue,
                families[stops[(j + 1).min(m - 1)]].hue,
                t,
            )
        })
        .collect()
}

/// Validate family indices against the extraction output.
pub fn family_indices(
    indices: &[i64],
    families: &[Cluster],
    name: &str,
) -> Result<Vec<usize>, String> {
    indices
        .iter()
        .map(|i| {
            if *i < 0 || (*i as usize) >= families.len() {
                Err(format!(
                    "register '{}': distribution stop {} out of range ({} families extracted)",
                    name,
                    i,
                    families.len()
                ))
            } else {
                Ok(*i as usize)
            }
        })
        .collect()
}

/// Pick a wallpaper-cluster hue for one accent slot's anchor hue, or None
/// to keep the scheme colour.  First pass rewards distinctness: only
/// claim clusters no other slot has used, so separate accents stay
/// separate while hues remain available.  Second pass allows reuse, so a
/// two-hue wallpaper still recolours all eight accent slots coherently.
fn choose_cluster(
    eligible: &[Cluster],
    anchor: f64,
    used: &mut HashSet<u64>,
    threshold: f64,
) -> Option<f64> {
    let mut ranked: Vec<&Cluster> = eligible.iter().collect();
    ranked.sort_by(|a, b| {
        circ_dist(anchor, a.hue)
            .partial_cmp(&circ_dist(anchor, b.hue))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.hue
                    .partial_cmp(&b.hue)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });

    // Pass 1: claim clusters nobody has taken yet.
    for cluster in &ranked {
        if circ_dist(anchor, cluster.hue) > threshold {
            break; // ranked by distance; nothing closer remains
        }
        let key = round_value(cluster.hue, 2).to_bits();
        if !used.contains(&key) {
            used.insert(key);
            return Some(cluster.hue);
        }
    }

    // Pass 2: allow reuse (a two-hue wallpaper still recolours every
    // accent while staying on-theme).
    for cluster in &ranked {
        if circ_dist(anchor, cluster.hue) <= threshold {
            return Some(cluster.hue);
        }
    }
    None
}

/// The full mapped colours for all 16 slots.
///
/// Per register: either the distribution ramp (when set) or the legacy
/// policy - neutrals adopt the heaviest family; accents anchor-match the
/// nearest chroma-eligible clusters, near misses keeping the scheme
/// colour.  Each slot's lightness and chroma come from the reference
/// scheme and only ever change via the grade options and the legibility
/// guard.
pub fn retint(
    slot_hexes: &[String],
    clusters: &[Cluster],
    config: &MapConfig,
) -> Result<BTreeMap<String, String>, String> {
    if slot_hexes.len() != BASE_SLOTS.len() {
        return Err(format!(
            "expected {} base slots, got {}",
            BASE_SLOTS.len(),
            slot_hexes.len()
        ));
    }
    let regs = resolved_registers(config)?;

    let lch_of = |i: usize| -> Lch {
        hex_to_oklch(&slot_hexes[i]).expect("slot hexes are normalized at parse time")
    };

    // Pass-1 keys hash on the value's bits (f64 itself is not hashable).
    let mut used: HashSet<u64> = HashSet::new();
    let eligible: Vec<Cluster> = clusters
        .iter()
        .copied()
        .filter(|c| c.chroma >= config.extraction.accent_chroma_floor)
        .collect();

    // -- assignment: per register, the distribution ramp (arrangement) or
    //    the legacy policy (accents anchor-match; neutrals adopt the
    //    heaviest family).  A greyscale wallpaper leaves every scheme hue.
    let mut assigned: HashMap<usize, f64> = HashMap::new();
    for name in REGISTER_NAMES {
        let slots = register_slots(name);
        let settings = &regs[name];
        let ramp: Option<Vec<f64>> =
            if clusters.is_empty() || settings.distribution == DistributionState::Off {
                None
            } else {
                let k = slots.len();
                let stops = match &settings.distribution {
                    DistributionState::Auto => (0..k.min(clusters.len())).collect::<Vec<usize>>(),
                    DistributionState::Pin(i) => family_indices(&[*i], clusters, name)?,
                    DistributionState::Stops(v) => family_indices(v, clusters, name)?,
                    DistributionState::Off => unreachable!("checked above"),
                };
                Some(distribution_hues(k, &stops, clusters))
            };
        for (pos, slot) in slots.iter().copied().enumerate() {
            let i = slot_index(slot);
            let anchor = lch_of(i);
            let hue = if let Some(hues) = &ramp {
                hues[pos]
            } else if name == "accents" && !eligible.is_empty() {
                // A matched cluster wins; a near miss keeps the scheme colour.
                choose_cluster(&eligible, anchor.h, &mut used, config.reach_deg).unwrap_or(anchor.h)
            } else {
                // Neutrals (and accents with no eligible vivid family) adopt
                // the heaviest family.
                clusters.first().map(|c| c.hue).unwrap_or(anchor.h)
            };
            assigned.insert(i, hue);
        }
    }

    // -- rotation: shift each register's assigned hues across its slots --
    // "1 = move right one": slot i receives what slot i - 1 was assigned.
    for name in REGISTER_NAMES {
        let slots = register_slots(name);
        let rotated_by = regs[name].rotate.rem_euclid(slots.len() as i64) as i64;
        if rotated_by == 0 {
            continue;
        }
        let indices: Vec<usize> = slots.iter().map(|s| slot_index(s)).collect();
        let hues: Vec<f64> = indices.iter().map(|i| assigned[i]).collect();
        for (pos, i) in indices.iter().copied().enumerate() {
            let from = (pos as i64 - rotated_by).rem_euclid(indices.len() as i64) as usize;
            assigned.insert(i, hues[from]);
        }
    }

    // -- blend-hue: circular hue interpolation toward the scheme's own hue --
    for i in 0..BASE_SLOTS.len() {
        let settings = &regs[register_of_slot(BASE_SLOTS[i])];
        if settings.blend_hue < 1.0 {
            let blended = circ_lerp(lch_of(i).h, assigned[&i], settings.blend_hue);
            assigned.insert(i, blended);
        }
    }

    // -- grade: additive L shift, multiplicative chroma, per register --
    let mut graded: BTreeMap<String, Lch> = BTreeMap::new();
    for i in 0..BASE_SLOTS.len() {
        let settings = &regs[register_of_slot(BASE_SLOTS[i])];
        let lch = lch_of(i);
        graded.insert(
            BASE_SLOTS[i].to_string(),
            Lch {
                l: (lch.l + settings.light).clamp(0.0, 1.0),
                c: (lch.c * settings.chroma).max(0.0),
                h: assigned[&i],
            },
        );
    }

    // -- legibility guard: base05 (default fg) vs base00 (default bg)
    // keeps the reference scheme's direction and at least
    // fg-contrast-floor of delta.  Uniform per-register L shifts move the
    // fg ramp together, so protecting base05 protects the ramp.
    let floor = config.extraction.fg_contrast_floor;
    let reference_delta = lch_of(5).l - lch_of(0).l;
    let sign = if reference_delta >= 0.0 { 1.0 } else { -1.0 };
    let delta = graded["base05"].l - graded["base00"].l;
    if delta * sign < floor {
        let fg = graded["base05"];
        graded.insert(
            "base05".to_string(),
            Lch {
                l: graded["base00"].l + sign * floor,
                c: fg.c,
                h: fg.h,
            },
        );
    }

    Ok(graded
        .iter()
        .map(|(slot, lch)| (slot.clone(), oklch_to_hex(lch)))
        .collect())
}
