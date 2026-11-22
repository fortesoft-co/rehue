//! Config layering, file reading and the unknown-key check.
//!
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

/// The file formats a config or look can be written in, by extension.
pub const EXTENSIONS: [&str; 4] = ["toml", "yaml", "yml", "json"];

/// Parse config text in the format its file extension names (`toml`,
/// `yaml` / `yml`, `json`; any case).  The result must be a table.
pub fn parse_text(text: &str, extension: &str) -> Result<Value, String> {
    let value = match extension.to_lowercase().as_str() {
        "json" => serde_json::from_str(text).map_err(|e| e.to_string())?,
        "toml" => from_toml(toml::from_str(text).map_err(|e| e.to_string())?),
        "yaml" | "yml" => from_yaml(text)?,
        _ => {
            return Err(
                "name it .toml, .yaml or .json (the extension picks the format)".to_string(),
            );
        }
    };
    if !value.is_object() {
        return Err("expected a table of settings".into());
    }
    Ok(value)
}

/// A config file as a value (an object), read by its extension:
/// `.json`, `.toml`, or `.yaml` / `.yml`.  No file is the empty object.
pub fn read_file(path: Option<&Path>) -> Result<Value, String> {
    let Some(p) = path else {
        return Ok(Value::Object(Map::new()));
    };
    let text = std::fs::read_to_string(p)
        .map_err(|e| format!("can't read config {}: {}", p.display(), e))?;
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    parse_text(&text, &ext).map_err(|e| format!("bad config {}: {e}", p.display()))
}

/// A YAML document as a JSON value (YAML 1.2: `no` and `on` stay
/// strings).  An empty file is the empty object.
pub fn from_yaml(text: &str) -> Result<Value, String> {
    use yaml_rust2::{Yaml, YamlLoader};
    let docs = YamlLoader::load_from_str(text).map_err(|e| e.to_string())?;
    if docs.len() > 1 {
        return Err("one YAML document only (found a `---` separator)".into());
    }
    fn convert(y: &Yaml) -> Result<Value, String> {
        Ok(match y {
            Yaml::Null => Value::Null,
            Yaml::Boolean(b) => Value::Bool(*b),
            Yaml::Integer(i) => Value::from(*i),
            Yaml::Real(r) => {
                let f: f64 = r.parse().map_err(|_| format!("not a number: {r}"))?;
                serde_json::Number::from_f64(f)
                    .map(Value::Number)
                    .ok_or_else(|| format!("not a finite number: {r}"))?
            }
            Yaml::String(s) => Value::String(s.clone()),
            Yaml::Array(a) => Value::Array(a.iter().map(convert).collect::<Result<_, _>>()?),
            Yaml::Hash(h) => {
                let mut map = Map::new();
                for (k, v) in h {
                    let key = match k {
                        Yaml::String(s) => s.clone(),
                        Yaml::Integer(i) => i.to_string(),
                        other => return Err(format!("keys are names, not {other:?}")),
                    };
                    map.insert(key, convert(v)?);
                }
                Value::Object(map)
            }
            Yaml::Alias(_) | Yaml::BadValue => return Err("unreadable value".into()),
        })
    }
    match docs.first() {
        None => Ok(Value::Object(Map::new())),
        Some(doc) => convert(doc),
    }
}

/// A TOML table as a JSON value.
pub fn from_toml(table: toml::Table) -> Value {
    serde_json::to_value(table).expect("a TOML table is valid JSON")
}
