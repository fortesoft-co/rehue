//! Base16 scheme loading and rendering.
//!
//! The accepted dialect:
//! flat `key: value` or `palette:`-nested files, `#`-prefixed hex values,
//! quoted values, inline comments after a ` #` (gruvbox et al carry those),
//! and keys matched case-insensitively (stored lowercased, so the lowercase
//! slot names look up directly even though the collection spells `base0A`).

use std::collections::BTreeMap;
use std::path::Path;

use crate::color;

/// Lowercase base16 slot names, in the canonical order.
pub const BASE_SLOTS: [&str; 16] = [
    "base00", "base01", "base02", "base03", "base04", "base05", "base06", "base07",
    "base08", "base09", "base0a", "base0b", "base0c", "base0d", "base0e", "base0f",
];

#[derive(Debug, Default, Clone)]
pub struct Scheme {
    /// Every key/value from the file (keys lowercased, values unquoted and
    /// comment-stripped).
    pub attrs: BTreeMap<String, String>,
}

impl Scheme {
    pub fn parse_file(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("can't read scheme {}: {}", path.display(), e))?;
        let mut attrs = BTreeMap::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || !line.contains(':') {
                continue;
            }
            let (key, value) = line.split_once(':').expect("line contains a colon");
            // Trailing inline comments: `'#1d2021' # ----` - anything after
            // a space-prefixed # is comment, not value.
            let value = value.split(" #").next().unwrap_or("");
            attrs.insert(
                key.trim().to_ascii_lowercase(),
                value.trim().trim_matches(['"', '\'']).to_string(),
            );
        }
        Ok(Scheme { attrs })
    }

    /// Python-style truthiness lookup: first non-empty among the candidates,
    /// else the fallback.  (`or()` alone would not skip empty strings.)
    pub fn pick(&self, first: &str, second: &str, fallback: &str) -> String {
        let get = |k: &str| {
            self.attrs
                .get(k)
                .map(|s| s.to_string())
                .unwrap_or_default()
        };
        let (a, b) = (get(first), get(second));
        if !a.is_empty() {
            a
        } else if !b.is_empty() {
            b
        } else {
            fallback.to_string()
        }
    }

    /// The scheme's display name (name, else scheme, else "unnamed").
    pub fn meta_name(&self) -> String {
        self.pick("name", "scheme", "unnamed")
    }

    /// The scheme's author ("unknown" when absent).
    pub fn meta_author(&self) -> String {
        self.pick("author", "author", "unknown")
    }

    /// The 16 base slots as normalized hex, in canonical order; fails closed
    /// when any slot is missing.
    pub fn slot_hexes(&self) -> Result<Vec<String>, String> {
        let mut out = Vec::with_capacity(16);
        for slot in BASE_SLOTS {
            let value = self
                .attrs
                .get(slot)
                .ok_or_else(|| format!("reference scheme lacks: {}", slot))?;
            out.push(color::normalize_hex(value)?);
        }
        Ok(out)
    }

    /// The canonical render: the tinted-schemes palette layout with
    /// uppercase slot keys and no ANSI aliases (base16.nix synthesizes
    /// mnemonics downstream; extra keys would pollute consumers such as
    /// `colors.toList`).
    pub fn render(&self, mapped: &BTreeMap<String, String>) -> String {
        let quoted = |s: &str| s.replace('"', "'");
        let name = self.pick("name", "scheme", "unnamed");
        let author = self.pick("author", "author", "unknown");
        let mut lines = vec![
            "system: \"base16\"".to_string(),
            format!("name: \"{} (hue-mapped)\"", quoted(&name)),
            format!("author: \"{} (hue-mapped by rehue)\"", quoted(&author)),
        ];
        if let Some(variant) = self.attrs.get("variant") {
            if !variant.is_empty() {
                lines.push(format!("variant: \"{}\"", quoted(variant)));
            }
        }
        lines.push("palette:".to_string());
        for slot in BASE_SLOTS {
            // Uppercase hex beyond base09 is the spelling base16.nix
            // resolves slots by; the collection files use it.
            let key = format!("base{}", slot[4..].to_ascii_uppercase());
            lines.push(format!("  {}: \"{}\"", key, mapped[slot]));
        }
        lines.join("\n") + "\n"
    }
}
