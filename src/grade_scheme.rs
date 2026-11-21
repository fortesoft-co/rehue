//! The role pipeline: assign (legacy policy or assign) -> cycle
//! -> match-hue -> grade -> legibility guard.  Every stage is
//! deterministic and the wallpaper supplies hues only, so every slot's
//! lightness and chroma targets come from the reference scheme.
//!
//! Arrangement (assign + cycle) is shared with `grade_wal`, which
//! reshapes a palette's slot hues with the same rules before the
//! per-pixel work.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::color::{Lch, circ_dist, circ_lerp, hex_to_oklch, oklch_to_hex, round_value};
use crate::extract::{Cluster, ExtractionParams};
use crate::role::{
    Assign, AssignState, ROLE_NAMES, assign_hues, family_indices, resolve_assign, role_of_slot,
    role_slots, slot_index,
};
use crate::scheme::BASE_SLOTS;

/// Wire form: every field optional so records merge per key, exactly the
/// `defaults // all // role` dictionary merge.  Canonical keys are
/// kebab-case in the config JSON.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default, deny_unknown_fields)]
pub struct SchemeRoleConfig {
    pub assign: Option<Assign>,
    pub cycle: Option<i64>,
    pub match_hue: Option<f64>,
    pub lightness: Option<f64>,
    pub chroma: Option<f64>,
}

/// Resolved per-role settings the pipeline runs with.
#[derive(Debug, Clone)]
pub struct SchemeRoleSettings {
    pub assign: AssignState,
    pub cycle: i64,
    pub match_hue: f64,
    pub lightness: f64,
    pub chroma: f64,
}

/// Map-scheme configuration: extraction options, `reach` (the accent
/// claim gate: how far a family hue may sit from an accent slot's hue and
/// still claim it) and role overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct SchemeConfig {
    #[serde(flatten)]
    pub extraction: ExtractionParams,
    pub reach: f64,
    /// Legibility guard: the least OKLCH lightness gap (0..1) kept
    /// between base05 (text) and base00 (background).  Not a WCAG ratio.
    pub min_text_contrast: f64,
    pub roles: HashMap<String, SchemeRoleConfig>,
}

impl Default for SchemeConfig {
    fn default() -> Self {
        Self {
            extraction: ExtractionParams::default(),
            reach: 45.0,
            min_text_contrast: 0.25,
            roles: HashMap::new(),
        }
    }
}

/// The `defaults // all // role` resolution, with option validation.
pub fn resolved_roles(
    config: &SchemeConfig,
) -> Result<BTreeMap<&'static str, SchemeRoleSettings>, String> {
    if !(0.0..=1.0).contains(&config.min_text_contrast) {
        return Err(format!(
            "min-text-contrast is a lightness gap within [0, 1] (got {}); it is not a WCAG ratio like 4.5",
            config.min_text_contrast
        ));
    }
    let roles = crate::role::canonical_roles(&config.roles)?;
    let all = roles.get("all").cloned().unwrap_or_default();
    let mut out = BTreeMap::new();
    for name in ROLE_NAMES {
        let given = roles.get(name).cloned().unwrap_or_default();
        let assign = resolve_assign(name, given.assign.or_else(|| all.assign.clone()))?;
        let cycle = given.cycle.or(all.cycle).unwrap_or(0);
        let match_hue = given.match_hue.or(all.match_hue).unwrap_or(1.0);
        let light = given.lightness.or(all.lightness).unwrap_or(0.0);
        let chroma = given.chroma.or(all.chroma).unwrap_or(1.0);
        if !(0.0..=1.0).contains(&match_hue) {
            return Err(format!("roles.{}.match-hue must be within [0, 1]", name));
        }
        if chroma <= 0.0 {
            return Err(format!("roles.{}.chroma must be positive", name));
        }
        out.insert(
            name,
            SchemeRoleSettings {
                assign,
                cycle,
                match_hue,
                lightness: light,
                chroma,
            },
        );
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
/// Per role: either the assign ramp (when set) or the legacy
/// policy - neutrals adopt the heaviest family; accents anchor-match the
/// nearest chroma-eligible clusters, near misses keeping the scheme
/// colour.  Each slot's lightness and chroma come from the reference
/// scheme and only ever change via the grade options and the legibility
/// guard.
pub fn retint(
    slot_hexes: &[String],
    clusters: &[Cluster],
    config: &SchemeConfig,
) -> Result<BTreeMap<String, String>, String> {
    if slot_hexes.len() != BASE_SLOTS.len() {
        return Err(format!(
            "expected {} base slots, got {}",
            BASE_SLOTS.len(),
            slot_hexes.len()
        ));
    }
    let regs = resolved_roles(config)?;

    let lch_of = |i: usize| -> Lch {
        hex_to_oklch(&slot_hexes[i]).expect("slot hexes are normalized at parse time")
    };

    // Pass-1 keys hash on the value's bits (f64 itself is not hashable).
    let mut used: HashSet<u64> = HashSet::new();
    let eligible: Vec<Cluster> = clusters
        .iter()
        .copied()
        .filter(|c| c.chroma >= config.extraction.min_accent_chroma)
        .collect();

    // -- assignment: per role, the assign ramp (arrangement) or
    //    the legacy policy (accents anchor-match; neutrals adopt the
    //    heaviest family).  A greyscale wallpaper leaves every scheme hue.
    let mut assigned: HashMap<usize, f64> = HashMap::new();
    for name in ROLE_NAMES {
        let slots = role_slots(name);
        let settings = &regs[name];
        let ramp: Option<Vec<f64>> = if clusters.is_empty() || settings.assign == AssignState::Off {
            None
        } else {
            let k = slots.len();
            let stops = match &settings.assign {
                AssignState::Auto => (0..k.min(clusters.len())).collect::<Vec<usize>>(),
                AssignState::Pin(i) => family_indices(&[*i], clusters, name)?,
                AssignState::Stops(v) => family_indices(v, clusters, name)?,
                AssignState::Off => unreachable!("checked above"),
            };
            Some(assign_hues(k, &stops, clusters))
        };
        for (pos, slot) in slots.iter().copied().enumerate() {
            let i = slot_index(slot);
            let anchor = lch_of(i);
            let hue = if let Some(hues) = &ramp {
                hues[pos]
            } else if name == "accents" && !eligible.is_empty() {
                // A matched cluster wins; a near miss keeps the scheme colour.
                choose_cluster(&eligible, anchor.h, &mut used, config.reach).unwrap_or(anchor.h)
            } else {
                // Neutrals (and accents with no eligible vivid family) adopt
                // the heaviest family.
                clusters.first().map(|c| c.hue).unwrap_or(anchor.h)
            };
            assigned.insert(i, hue);
        }
    }

    // -- rotation: shift each role's assigned hues across its slots --
    // "1 = move right one": slot i receives what slot i - 1 was assigned.
    for name in ROLE_NAMES {
        let slots = role_slots(name);
        let rotated_by = regs[name].cycle.rem_euclid(slots.len() as i64);
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

    // -- match-hue: circular hue interpolation toward the scheme's own hue --
    for i in 0..BASE_SLOTS.len() {
        let settings = &regs[role_of_slot(BASE_SLOTS[i])];
        if settings.match_hue < 1.0 {
            let blended = circ_lerp(lch_of(i).h, assigned[&i], settings.match_hue);
            assigned.insert(i, blended);
        }
    }

    // -- grade: additive L shift, multiplicative chroma, per role --
    let mut graded: BTreeMap<String, Lch> = BTreeMap::new();
    for i in 0..BASE_SLOTS.len() {
        let settings = &regs[role_of_slot(BASE_SLOTS[i])];
        let lch = lch_of(i);
        graded.insert(
            BASE_SLOTS[i].to_string(),
            Lch {
                l: (lch.l + settings.lightness).clamp(0.0, 1.0),
                c: (lch.c * settings.chroma).max(0.0),
                h: assigned[&i],
            },
        );
    }

    // -- legibility guard: base05 (default fg) vs base00 (default bg)
    // keeps the reference scheme's direction and at least
    // min-text-contrast of delta.  Uniform per-role L shifts move the
    // fg ramp together, so protecting base05 protects the ramp.
    let floor = config.min_text_contrast;
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
