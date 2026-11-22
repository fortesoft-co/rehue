//! Looks: saved grades, selectable by name (`-l pop`).
//!
//! A look is a TOML file with a `description` and a `[wal]` and/or
//! `[scheme]` section; the sections take the same keys as the
//! commands' configs.  A look is a starting point: it lays under the
//! `--config` file and the flags, key by key, so anything stated
//! explicitly wins.  Looks stack left to right (`-l snap,pop`).
//!
//! Built-in looks are compiled in; your own live in
//! `$XDG_CONFIG_HOME/rehue/looks/NAME.toml` (default
//! `~/.config/rehue/looks`), where a file named like a built-in
//! replaces it.  A name with a `/` is a path to a look file.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::config::{from_toml, from_value, layer as layer_config};
use crate::grade_scheme::SchemeConfig;
use crate::grade_wal::WalConfig;

/// The built-in looks: (name, file text).
pub const BUILTIN: [(&str, &str); 7] = [
    ("soft", include_str!("looks/soft.toml")),
    ("tight", include_str!("looks/tight.toml")),
    ("pop", include_str!("looks/pop.toml")),
    ("muted", include_str!("looks/muted.toml")),
    ("wash", include_str!("looks/wash.toml")),
    ("snap", include_str!("looks/snap.toml")),
    ("vivid", include_str!("looks/vivid.toml")),
];

/// The command a look section grades for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Wal,
    Scheme,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Self::Wal => "wal",
            Self::Scheme => "scheme",
        }
    }
}

/// Where a look came from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    BuiltIn,
    /// A user file (or a path given directly).
    File(PathBuf),
    /// A user file that replaces the built-in of the same name.
    Override(PathBuf),
}

/// A loaded, validated look.
#[derive(Debug, Clone)]
pub struct Look {
    pub name: String,
    pub description: String,
    pub source: Source,
    /// The file text, as written.
    pub text: String,
    wal: Option<Value>,
    scheme: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LookFile {
    #[serde(default)]
    description: String,
    wal: Option<toml::Table>,
    scheme: Option<toml::Table>,
}

impl Look {
    /// Parse and validate a look: every section must be a valid config
    /// for its command.
    pub fn parse(name: &str, text: &str, source: Source) -> Result<Look, String> {
        let file: LookFile = toml::from_str(text).map_err(|e| format!("look '{name}': {e}"))?;
        if file.wal.is_none() && file.scheme.is_none() {
            return Err(format!(
                "look '{name}' has neither a [wal] nor a [scheme] section"
            ));
        }
        let wal = file.wal.map(from_toml);
        let scheme = file.scheme.map(from_toml);
        if let Some(v) = &wal {
            from_value::<WalConfig>(v.clone()).map_err(|e| format!("look '{name}' [wal]: {e}"))?;
        }
        if let Some(v) = &scheme {
            from_value::<SchemeConfig>(v.clone())
                .map_err(|e| format!("look '{name}' [scheme]: {e}"))?;
        }
        Ok(Look {
            name: name.to_string(),
            description: file.description,
            source,
            text: text.to_string(),
            wal,
            scheme,
        })
    }

    /// The section for `target`, if the look has one.
    pub fn section(&self, target: Target) -> Option<&Value> {
        match target {
            Target::Wal => self.wal.as_ref(),
            Target::Scheme => self.scheme.as_ref(),
        }
    }

    /// The commands this look grades for: "wal, scheme", "wal", ...
    pub fn targets(&self) -> String {
        [Target::Wal, Target::Scheme]
            .into_iter()
            .filter(|t| self.section(*t).is_some())
            .map(Target::name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Where user looks live: `$XDG_CONFIG_HOME/rehue/looks`, else
/// `~/.config/rehue/looks`.
pub fn user_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("rehue").join("looks"))
}

fn builtin(name: &str) -> Option<&'static str> {
    BUILTIN.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("can't read look {}: {e}", path.display()))
}

/// Find a look by name (user file first, then built-in) or by path (a
/// name containing `/`).
pub fn find(name: &str) -> Result<Look, String> {
    if name.contains('/') {
        let path = PathBuf::from(name);
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.to_string());
        return Look::parse(&stem, &read(&path)?, Source::File(path));
    }
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "'{name}' is not a look name (letters, digits, '-', '_')"
        ));
    }
    if let Some(dir) = user_dir() {
        let path = dir.join(format!("{name}.toml"));
        if path.is_file() {
            let source = if builtin(name).is_some() {
                Source::Override(path.clone())
            } else {
                Source::File(path.clone())
            };
            return Look::parse(name, &read(&path)?, source);
        }
    }
    match builtin(name) {
        Some(text) => Look::parse(name, text, Source::BuiltIn),
        None => {
            let known: Vec<String> = all()
                .map(|looks| looks.into_iter().map(|l| l.name).collect())
                .unwrap_or_default();
            Err(format!(
                "no look named '{name}' (looks: {}; rehue inspect --looks)",
                known.join(", ")
            ))
        }
    }
}

/// Every look: the built-ins (or their user overrides), then the user's
/// own, sorted by name within each group.
pub fn all() -> Result<Vec<Look>, String> {
    let mut user: Vec<String> = Vec::new();
    if let Some(dir) = user_dir()
        && let Ok(listing) = std::fs::read_dir(&dir)
    {
        for entry in listing.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml")
                && path.is_file()
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                && builtin(stem).is_none()
            {
                user.push(stem.to_string());
            }
        }
    }
    user.sort();
    BUILTIN
        .iter()
        .map(|(n, _)| n.to_string())
        .chain(user)
        .map(|n| find(&n))
        .collect()
}

/// `-l` values: repeated flags and comma lists, in order.
pub fn names(given: &[String]) -> Vec<String> {
    given
        .iter()
        .flat_map(|g| g.split(','))
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(String::from)
        .collect()
}

/// The `look` key of a config value (a name, a comma list, or an array
/// of names), removed from it.
pub fn take_look(config: &mut Value) -> Result<Option<Vec<String>>, String> {
    let Some(map) = config.as_object_mut() else {
        return Ok(None);
    };
    match map.remove("look") {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(names(&[s]))),
        Some(Value::Array(items)) => items
            .into_iter()
            .map(|v| match v {
                Value::String(s) => Ok(s),
                other => Err(format!("look: expected names, got {other}")),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|v| Some(names(&v))),
        Some(other) => Err(format!(
            "look: expected a name or a list of names, got {other}"
        )),
    }
}

/// The looks named, layered for `target` (left to right, later wins).
/// A look without a section for `target` is an error.
pub fn layer(looks: &[Look], target: Target) -> Result<Value, String> {
    let mut out = Value::Object(Map::new());
    for look in looks {
        let section = look.section(target).ok_or_else(|| {
            format!(
                "look '{}' has nothing for {} (it grades: {})",
                look.name,
                target.name(),
                look.targets()
            )
        })?;
        layer_config(&mut out, section.clone());
    }
    Ok(out)
}

/// The full resolution for one command: the looks (`flag_looks` when
/// given, else the config's own `look` key, else `inherited`) lay under
/// the config, key by key; the result is checked and typed.  `what`
/// names the config in errors (the file, or the pipeline table).  Returns
/// the config and the looks that were used.
pub fn resolve<T: DeserializeOwned + Serialize>(
    what: &str,
    mut config: Value,
    flag_looks: &[String],
    inherited: Option<Vec<String>>,
    target: Target,
) -> Result<(T, Vec<Look>), String> {
    let own = take_look(&mut config)?;
    let wanted = if flag_looks.is_empty() {
        own.or(inherited).unwrap_or_default()
    } else {
        names(flag_looks)
    };
    let looks: Vec<Look> = wanted.iter().map(|n| find(n)).collect::<Result<_, _>>()?;
    let mut value = layer(&looks, target)?;
    layer_config(&mut value, config);
    let typed = from_value(value).map_err(|e| format!("{what}: {e}"))?;
    Ok((typed, looks))
}
