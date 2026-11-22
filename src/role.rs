//! The four roles, their names, and `assign`, shared by both directions.
//!
//! Roles: the four groups a base16 scheme's 16 slots fall into
//! (background, surfaces, foreground, accents), the names they go by,
//! and `assign`, which gives a role's slots hues taken from the
//! wallpaper's families.  Shared by both grading directions.

use serde::{Deserialize, Serialize};

use crate::color::circ_lerp;
use crate::extract::Cluster;
use crate::scheme::BASE_SLOTS;

/// The roles, canonical names, in slot order.
pub const ROLE_NAMES: [&str; 4] = ["background", "surfaces", "foreground", "accents"];

/// The short forms accepted wherever a role is typed (same order).
pub const ROLE_SHORT: [&str; 4] = ["bg", "sf", "fg", "ac"];

/// What each role is, for the docs (same order as `ROLE_NAMES`).
pub const ROLE_DOCS: [&str; 4] = [
    "the background",
    "panels, selections, comments",
    "text",
    "syntax colors, highlights, status colors",
];

/// The canonical name for a typed role (full name or short form), or
/// `all`; `None` for anything else.
pub fn canonical_role(name: &str) -> Option<&'static str> {
    if name == "all" {
        return Some("all");
    }
    ROLE_NAMES
        .iter()
        .zip(ROLE_SHORT)
        .find(|(full, short)| **full == name || *short == name)
        .map(|(full, _)| *full)
}

/// "background (bg), surfaces (sf), ..., all": for error messages.
pub fn role_list() -> String {
    let mut names: Vec<String> = ROLE_NAMES
        .iter()
        .zip(ROLE_SHORT)
        .map(|(full, short)| format!("{full} ({short})"))
        .collect();
    names.push("all".to_string());
    names.join(", ")
}

/// A `roles` config map keyed by canonical names: short forms resolve,
/// unknown names and the same role given twice (`bg` and `background`)
/// are errors.
pub fn canonical_roles<T: Clone>(
    given: &std::collections::HashMap<String, T>,
) -> Result<std::collections::HashMap<&'static str, T>, String> {
    let mut out = std::collections::HashMap::new();
    let mut keys: Vec<&String> = given.keys().collect();
    keys.sort();
    for key in keys {
        let name = canonical_role(key)
            .ok_or_else(|| format!("roles: '{key}' is not a role ({})", role_list()))?;
        if out.insert(name, given[key].clone()).is_some() {
            return Err(format!("roles: '{name}' is given twice"));
        }
    }
    Ok(out)
}

/// Slot -> role (background is base00; surfaces base01-03; foreground
/// base04-07; accents base08-0F).
pub fn role_of_slot(slot: &str) -> &'static str {
    let i = u32::from_str_radix(&slot[4..], 16).expect("base16 slot has a hex index");
    if i == 0 {
        "background"
    } else if i <= 3 {
        "surfaces"
    } else if i <= 7 {
        "foreground"
    } else {
        "accents"
    }
}

/// The slots of a role (canonical name).
pub fn role_slots(name: &str) -> &'static [&'static str] {
    match name {
        "background" => &[BASE_SLOTS[0]],
        "surfaces" => &[BASE_SLOTS[1], BASE_SLOTS[2], BASE_SLOTS[3]],
        "foreground" => &[BASE_SLOTS[4], BASE_SLOTS[5], BASE_SLOTS[6], BASE_SLOTS[7]],
        _ => &BASE_SLOTS[8..16],
    }
}

pub fn slot_index(slot: &str) -> usize {
    BASE_SLOTS
        .iter()
        .position(|s| *s == slot)
        .expect("canonical slot name")
}

/// The `assign` option: gives a role's slots hues taken from the
/// wallpaper's families.  Family indices are positions in the extraction
/// output (weight-ordered, heaviest first), what `rehue inspect` prints.
///
/// * `"auto"` - families in extraction order, spread across the role's
///   slots (`min(slots, families)` stops, shortest-arc interpolation
///   between them).
/// * integer - pin the role to that one family (the single-slot
///   background accepts only this form).
/// * list    - explicit stops in user order; at most one per slot.  A
///   full-length list is a pure per-slot assignment, and duplicate
///   indices are legal, giving flat runs.
///
/// Hue only, always: every slot keeps the scheme's lightness and chroma.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Assign {
    /// `"auto"`: spread the heaviest families across the slots.
    Auto(AutoWord),
    /// Pin the role to one extracted family.
    Pin(i64),
    /// Explicit stops in user order (family indices).
    Stops(Vec<i64>),
}

/// The literal `"auto"` (the only string `assign` takes).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoWord {
    Auto,
}

impl Assign {
    /// `"auto"`.
    pub const AUTO: Assign = Assign::Auto(AutoWord::Auto);

    /// The command-line form: `auto`, a family index (`3`), or a
    /// comma list of stops (`0,2,5`; brackets optional).
    pub fn parse_cli(token: &str) -> Result<Assign, String> {
        let t = token.trim();
        if t == "auto" {
            return Ok(Assign::AUTO);
        }
        let bare = t.strip_prefix('[').and_then(|r| r.strip_suffix(']'));
        let listed = bare.is_some() || t.contains(',');
        let parts: Result<Vec<i64>, _> = bare
            .unwrap_or(t)
            .split(',')
            .map(|p| p.trim().parse::<i64>())
            .collect();
        match parts {
            Ok(v) if listed => Ok(Assign::Stops(v)),
            Ok(v) if v.len() == 1 => Ok(Assign::Pin(v[0])),
            _ => Err(format!(
                "expected auto, a family index (3) or a list (0,2,5), got {token}"
            )),
        }
    }
}

/// Resolved assign intent, after static (slot-count) validation.
/// Family-index validation needs the extracted families and happens where
/// they are known.
#[derive(Debug, Clone, PartialEq)]
pub enum AssignState {
    /// No assign set: the role's legacy assignment behaviour.
    Off,
    /// Default ramp.
    Auto,
    /// Pin to one family index.
    Pin(i64),
    /// Explicit stops (family indices) in user order.
    Stops(Vec<i64>),
}

impl AssignState {
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

/// Static validation of one role's `assign` against its slot
/// count.
pub fn resolve_assign(name: &str, given: Option<Assign>) -> Result<AssignState, String> {
    let k = role_slots(name).len();
    let state = match given {
        None => AssignState::Off,
        Some(Assign::Auto(_)) => AssignState::Auto,
        Some(Assign::Pin(i)) => AssignState::Pin(i),
        Some(Assign::Stops(v)) => {
            if v.is_empty() {
                return Err(format!(
                    "role '{}': 'assign' list needs at least one stop",
                    name
                ));
            }
            if v.len() > k {
                return Err(format!(
                    "role '{}': 'assign' has {} stop(s) but the role has {} slot(s)",
                    name,
                    v.len(),
                    k
                ));
            }
            AssignState::Stops(v)
        }
    };
    if matches!(state, AssignState::Auto | AssignState::Stops(_)) && k == 1 {
        return Err(format!(
            "role '{}' has 1 slot: 'assign' takes a family index, not auto or a list",
            name
        ));
    }
    Ok(state)
}

/// Hue for every slot of one role from an explicit stop list.
///
/// Stops spread evenly across the role's slot span; slots between
/// stops take shortest-arc interpolation of the neighbouring stops' hues.
/// `stops.len() == k` degenerates to a pure per-slot assignment (manual
/// mode); duplicate stops give flat runs.
pub fn assign_hues(k: usize, stops: &[usize], families: &[Cluster]) -> Vec<f64> {
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
                    "role '{}': assign stop {} out of range ({} families extracted)",
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
