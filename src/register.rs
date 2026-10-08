//! The register pipeline: assignment -> rotate -> hue-blend -> grade ->
//! legibility guard.  Every stage is
//! is deterministic and the wallpaper supplies hues only, so every slot's
//! lightness and chroma targets come from the reference scheme.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::color::{circ_dist, circ_lerp, hex_to_oklch, oklch_to_hex, round_value, Lch};
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

fn slot_index(slot: &str) -> usize {
    BASE_SLOTS
        .iter()
        .position(|s| *s == slot)
        .expect("canonical slot name")
}

/// Wire form: every field optional so records merge per key, exactly the
/// `defaults // all // register` dictionary merge.  Canonical keys are
/// kebab-case in the config JSON.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct RegisterConfig {
    #[serde(alias = "hue_blend")]
    pub hue_blend: Option<f64>,
    pub rotate: Option<i64>,
    #[serde(alias = "family_offset")]
    pub family_offset: Option<i64>,
    pub light: Option<f64>,
    pub chroma: Option<f64>,
}

/// Resolved per-register settings the pipeline runs with.
#[derive(Debug, Clone, Copy)]
pub struct RegisterSettings {
    pub hue_blend: f64,
    pub rotate: i64,
    pub family_offset: i64,
    pub light: f64,
    pub chroma: f64,
}

/// Map-scheme configuration: extraction knobs + register overrides.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MapConfig {
    #[serde(flatten)]
    pub extraction: ExtractionParams,
    pub registers: HashMap<String, RegisterConfig>,
}

pub fn resolved_registers(
    config: &MapConfig,
) -> Result<BTreeMap<&'static str, RegisterSettings>, String> {
    let all = config.registers.get("all").copied().unwrap_or_default();
    let mut out = BTreeMap::new();
    for name in REGISTER_NAMES {
        let given = config.registers.get(name).copied().unwrap_or_default();
        let settings = RegisterSettings {
            hue_blend: given.hue_blend.or(all.hue_blend).unwrap_or(1.0),
            rotate: given.rotate.or(all.rotate).unwrap_or(0),
            family_offset: given.family_offset.or(all.family_offset).unwrap_or(0),
            light: given.light.or(all.light).unwrap_or(0.0),
            chroma: given.chroma.or(all.chroma).unwrap_or(1.0),
        };
        if !(0.0..=1.0).contains(&settings.hue_blend) {
            return Err(format!(
                "registers.{}.hue-blend must be within [0, 1]",
                name
            ));
        }
        if settings.chroma <= 0.0 {
            return Err(format!("registers.{}.chroma must be positive", name));
        }
        out.insert(name, settings);
    }
    Ok(out)
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
            .then(a.hue.partial_cmp(&b.hue).unwrap_or(std::cmp::Ordering::Equal))
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

/// The register's selected wallpaper family's hue: the family-offset-th
/// heaviest cluster, wrapping.  None when the wallpaper is greyscale, in
/// which case slots keep the scheme colour.
fn family_hue(clusters: &[Cluster], settings: &RegisterSettings) -> Option<f64> {
    if clusters.is_empty() {
        return None;
    }
    let n = clusters.len() as i64;
    Some(clusters[settings.family_offset.rem_euclid(n) as usize].hue)
}

/// The full mapped colours for all 16 slots.
///
/// With default register settings this is the plain hue-mapping given
/// the same clusters: neutrals adopt the
/// heaviest cluster's hue (never a full-wheel average - antipodal cluster
/// means land on hues the wallpaper does not contain); accent slots
/// anchor-match nearest eligible clusters, near misses keeping the scheme
/// colour.  Each slot's lightness and chroma come from the reference
/// scheme and only ever change via the grade stage.
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
        hex_to_oklch(&slot_hexes[i])
            .expect("slot hexes are normalized at parse time")
    };

    // Pass-1 keys hash on the value's bits (f64 itself is not hashable).
    let mut used: HashSet<u64> = HashSet::new();
    let eligible: Vec<Cluster> = clusters
        .iter()
        .copied()
        .filter(|c| c.chroma >= config.extraction.accent_chroma_floor)
        .collect();

    // -- assignment: one wallpaper hue (or the scheme's own) per slot --
    let mut assigned: HashMap<usize, f64> = HashMap::new();
    for i in 0..BASE_SLOTS.len() {
        let slot = BASE_SLOTS[i];
        let name_settings = regs[register_of_slot(slot)];
        let anchor = lch_of(i);
        let hue = if register_of_slot(slot) == "accents" && !eligible.is_empty() {
            // A matched cluster wins; a near miss keeps the scheme colour.
            choose_cluster(
                &eligible,
                anchor.h,
                &mut used,
                config.extraction.hue_match_threshold_deg,
            )
            .unwrap_or(anchor.h)
        } else {
            // Neutrals (and accents with no eligible vivid family) adopt
            // the register's selected wallpaper family.
            family_hue(clusters, &name_settings).unwrap_or(anchor.h)
        };
        assigned.insert(i, hue);
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
            let from =
                (pos as i64 - rotated_by).rem_euclid(indices.len() as i64) as usize;
            assigned.insert(i, hues[from]);
        }
    }

    // -- blend: circular hue interpolation toward the scheme's own hue --
    for i in 0..BASE_SLOTS.len() {
        let settings = regs[register_of_slot(BASE_SLOTS[i])];
        if settings.hue_blend < 1.0 {
            let blended = circ_lerp(lch_of(i).h, assigned[&i], settings.hue_blend);
            assigned.insert(i, blended);
        }
    }

    // -- grade: additive L shift, multiplicative chroma, per register --
    let mut graded: BTreeMap<String, Lch> = BTreeMap::new();
    for i in 0..BASE_SLOTS.len() {
        let settings = regs[register_of_slot(BASE_SLOTS[i])];
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
