//! Reading grading configs.  A `--config` file, a pipeline table and a
//! look's section all become one JSON value; values layer key by key
//! ([`merge`]), and the result is checked for unknown keys before it
//! becomes a typed config ([`from_value`]), so a typo is an error instead
//! of a silently ignored setting.

use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// Layer `over` onto `base`: objects merge key by key (recursively, so a
/// `roles.accents.chroma` leaves `roles.accents.assign` alone); anything
/// else replaces.
pub fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (key, value) in o {
                match b.get_mut(&key) {
                    Some(existing) => merge(existing, value),
                    None => {
                        b.insert(key, value);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

/// Layer one config over another: [`merge`], except that a `palette`
/// replaces the one below it whole (merging two subsets would blur
/// which slots are in).
pub fn layer(base: &mut Value, mut over: Value) {
    canonical_role_keys(base);
    canonical_role_keys(&mut over);
    if let (Some(b), Some(o)) = (base.as_object_mut(), over.as_object())
        && o.contains_key("palette")
    {
        b.remove("palette");
    }
    merge(base, over);
}

/// `roles` keys by canonical name (`ac` -> `accents`), so a short form
/// in one layer and the full name in another meet as the same role.
/// Unknown names stay as written, for validation to report.
fn canonical_role_keys(config: &mut Value) {
    if let Some(Value::Object(roles)) = config.get_mut("roles") {
        let given = std::mem::take(roles);
        for (key, value) in given {
            let name = crate::role::canonical_role(&key).map_or(key.clone(), String::from);
            // the same role twice in one layer (`bg` and `background`)
            // keeps both keys, so validation reports it
            let name = if roles.contains_key(&name) { key } else { name };
            roles.insert(name, value);
        }
    }
}

/// Every key in `given` must survive into `parsed` (the typed config,
/// serialized back): a key that vanished was not one the config knows.
fn check_keys(given: &Value, parsed: &Value, path: &str) -> Result<(), String> {
    if let (Value::Object(g), Value::Object(p)) = (given, parsed) {
        let mut keys: Vec<&String> = g.keys().collect();
        keys.sort();
        for key in keys {
            let at = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            match p.get(key) {
                None => {
                    let near = p
                        .keys()
                        .filter(|k| !g.contains_key(*k))
                        .map(|k| (distance(key, k), k))
                        .filter(|(d, _)| *d <= 2)
                        .min();
                    return Err(match near {
                        Some((_, k)) => format!("unknown key '{at}' (did you mean '{k}'?)"),
                        None => format!("unknown key '{at}'"),
                    });
                }
                Some(inner) => check_keys(&g[key], inner, &at)?,
            }
        }
    }
    Ok(())
}

/// Edit distance, for the "did you mean" hint.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(prev + usize::from(ca != *cb));
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// A typed config from a JSON value, rejecting unknown keys.
pub fn from_value<T: DeserializeOwned + Serialize>(value: Value) -> Result<T, String> {
    let typed: T = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    let back = serde_json::to_value(&typed).map_err(|e| e.to_string())?;
    check_keys(&value, &back, "")?;
    Ok(typed)
}

/// A JSON config file as a value (an object); no file is the empty object.
pub fn read_file(path: Option<&Path>) -> Result<Value, String> {
    let Some(p) = path else {
        return Ok(Value::Object(Map::new()));
    };
    let text = std::fs::read_to_string(p)
        .map_err(|e| format!("can't read config {}: {}", p.display(), e))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("bad config {}: {}", p.display(), e))?;
    if !value.is_object() {
        return Err(format!("bad config {}: expected an object", p.display()));
    }
    Ok(value)
}

/// A TOML table as a JSON value.
pub fn from_toml(table: toml::Table) -> Value {
    serde_json::to_value(table).expect("a TOML table is valid JSON")
}
