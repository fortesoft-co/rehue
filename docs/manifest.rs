//! The examples manifest (`docs/examples.toml`): every documented
//! command and every docs image, defined once.  Shared by the docs test
//! (`tests/docs.rs`, which renders the commands into README / EXAMPLES
//! and checks the images are current) and the image builder
//! (`examples/docs-images.rs`, which runs the commands and assembles
//! the images).  Included by path from both, so it is plain Rust with
//! no crate of its own.
#![allow(dead_code)] // each includer uses a different half

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const MANIFEST: &str = "docs/examples.toml";
pub const LOCK: &str = "docs/examples.lock";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// `{name}` placeholders (asset paths), expanded the same way in the
    /// docs and when running.
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    /// Single commands shown in the docs (`<!-- gen:run:ID -->`).
    #[serde(default)]
    pub run: Vec<Run>,
    /// Images assembled from runs and assets.
    #[serde(default)]
    pub image: Vec<Image>,
    /// Option ladders: one image of several runs plus their recipe
    /// (`<!-- gen:ladder:ID -->`).
    #[serde(default)]
    pub ladder: Vec<Ladder>,
}

/// One rehue invocation.  `args` must contain `-o {out}`; in the docs
/// `{out}` shows as `out`, when running it is a scratch directory.
/// `{out:ID}` names another run's output the same way.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub id: String,
    pub args: String,
    pub out: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// `magick montage` of labelled cells.
    Montage,
    /// The single cell's file, copied.
    Copy,
    /// The single cell's file, enlarged without smoothing.
    Scale,
    /// Headless-browser screenshots of a preview.html, one per view,
    /// side by side.
    Shot,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub file: String,
    pub kind: Kind,
    /// montage tile, e.g. "2x1"
    #[serde(default)]
    pub tile: Option<String>,
    /// montage cell width in px
    #[serde(default)]
    pub width: Option<u32>,
    /// scale percentage
    #[serde(default)]
    pub scale: Option<u32>,
    /// shot: preview.html views (palette | gtk | qt)
    #[serde(default)]
    pub views: Vec<String>,
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    #[serde(default)]
    pub label: String,
    pub src: String,
}

/// A ladder: the same command with one option stepped.  `base` (inputs)
/// is listed once in the docs; `common` and each rung's `args` appear on
/// every recipe line; each image cell is labelled with its rung's
/// `args`, so the picture always says what ran.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ladder {
    pub id: String,
    pub file: String,
    pub base: String,
    #[serde(default)]
    pub common: String,
    /// which file of each run's output goes into the image
    pub output: String,
    /// optional first cell, labelled "original"
    #[serde(default)]
    pub original: Option<String>,
    #[serde(default = "default_width")]
    pub width: u32,
    pub rungs: Vec<Rung>,
}

fn default_width() -> u32 {
    480
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rung {
    #[serde(default)]
    pub args: String,
    /// prose comment above the recipe line (the image label is `args`)
    #[serde(default)]
    pub note: String,
    /// extraction-style options written to a JSON file and passed as
    /// `--config` (for options that have no flag)
    #[serde(default)]
    pub config: Option<toml::Table>,
}

impl Manifest {
    pub fn load(root: &std::path::Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(root.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
        let m: Manifest = toml::from_str(&text).map_err(|e| format!("{MANIFEST}: {e}"))?;
        m.validate()?;
        Ok(m)
    }

    fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        for r in &self.run {
            if !ids.insert(r.id.as_str()) {
                return Err(format!("run id '{}' twice", r.id));
            }
            if !r.args.contains("{out}") {
                return Err(format!("run '{}': args need `-o {{out}}`", r.id));
            }
        }
        for l in &self.ladder {
            if !ids.insert(l.id.as_str()) {
                return Err(format!("ladder id '{}' clashes", l.id));
            }
            if !l.base.contains("{out}") {
                return Err(format!("ladder '{}': base needs `-o {{out}}`", l.id));
            }
        }
        Ok(())
    }

    pub fn run(&self, id: &str) -> Result<&Run, String> {
        self.run
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("no run '{id}'"))
    }

    /// Expand `{var}`, `{out}` and `{out:ID}` in one token; `outs` maps
    /// a run id ("" = the current run) to its directory.
    pub fn expand(
        &self,
        token: &str,
        outs: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<String, String> {
        let mut out = String::new();
        let mut rest = token;
        while let Some(open) = rest.find('{') {
            let Some(len) = rest[open..].find('}') else {
                break;
            };
            let key = &rest[open + 1..open + len];
            let value = if key == "out" {
                outs("")?
            } else if let Some(id) = key.strip_prefix("out:") {
                outs(id)?
            } else if let Some(v) = self.vars.get(key) {
                v.clone()
            } else if key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '-' || c == '_')
                && !key.is_empty()
            {
                return Err(format!("unknown placeholder {{{key}}}"));
            } else {
                // not a placeholder (e.g. JSON); keep verbatim
                rest[open..open + len + 1].to_string()
            };
            out.push_str(&rest[..open]);
            out.push_str(&value);
            rest = &rest[open + len + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// The run ids an image's cells read from (`{out:ID}`).
    pub fn image_runs(&self, image: &Image) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        for c in &image.cells {
            if let Some(i) = c.src.find("{out:") {
                let id = &c.src[i + 5..];
                let id = &id[..id.find('}').unwrap_or(id.len())];
                if !ids.iter().any(|x| x == id) {
                    ids.push(id.to_string());
                }
            }
        }
        ids
    }

    /// Everything that determines an image's pixels, for staleness: the
    /// entry itself, the runs it reads (and the runs those read).
    pub fn fingerprint_image(&self, image: &Image) -> Result<u64, String> {
        let mut text = toml::to_string(image).map_err(|e| e.to_string())?;
        let mut pending = self.image_runs(image);
        let mut seen = Vec::new();
        while let Some(id) = pending.pop() {
            if seen.contains(&id) {
                continue;
            }
            let run = self.run(&id)?;
            text.push_str(&toml::to_string(run).map_err(|e| e.to_string())?);
            for t in run.args.split_whitespace() {
                if let Some(i) = t.find("{out:") {
                    let dep = &t[i + 5..];
                    pending.push(dep[..dep.find('}').unwrap_or(dep.len())].to_string());
                }
            }
            seen.push(id);
        }
        Ok(fnv1a(&self.expand_vars(&text)))
    }

    pub fn fingerprint_ladder(&self, ladder: &Ladder) -> Result<u64, String> {
        let text = toml::to_string(ladder).map_err(|e| e.to_string())?;
        Ok(fnv1a(&self.expand_vars(&text)))
    }

    /// Vars inlined, so changing an asset path also changes the hash.
    fn expand_vars(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (k, v) in &self.vars {
            out = out.replace(&format!("{{{k}}}"), v);
        }
        out
    }

    /// `file fingerprint` lines for every image and ladder, sorted.
    pub fn lock(&self) -> Result<String, String> {
        let mut lines = Vec::new();
        for i in &self.image {
            lines.push(format!("{} {:016x}", i.file, self.fingerprint_image(i)?));
        }
        for l in &self.ladder {
            lines.push(format!("{} {:016x}", l.file, self.fingerprint_ladder(l)?));
        }
        lines.sort();
        Ok(lines.join("\n") + "\n")
    }
}

/// FNV-1a, 64-bit: stable across Rust versions (unlike `DefaultHasher`).
pub fn fnv1a(text: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// A token as a shell user would type it: quoted when the shell would
/// otherwise glob or split it.
pub fn shell_word(token: &str) -> String {
    if token.is_empty() || token.chars().any(|c| " []{}*?()$;&|<>'\"\\!#~".contains(c)) {
        format!("'{}'", token.replace('\'', "'\\''"))
    } else {
        token.to_string()
    }
}

/// `-i`, `--out`: a flag, not a value (`-0.1` is a value).
pub fn is_flag(token: &str) -> bool {
    let mut chars = token.chars();
    chars.next() == Some('-')
        && chars
            .next()
            .is_some_and(|c| c == '-' || c.is_ascii_alphabetic())
}

/// The docs form of a run: one flag (and its values) per line.
pub fn display_run(m: &Manifest, run: &Run) -> Result<String, String> {
    let outs = |id: &str| -> Result<String, String> {
        if id.is_empty() {
            Ok(run.out.clone())
        } else {
            Ok(m.run(id)?.out.clone())
        }
    };
    let mut lines = vec!["rehue".to_string()];
    for token in run.args.split_whitespace() {
        let word = shell_word(&m.expand(token, &outs)?);
        if is_flag(token) || lines.len() == 1 {
            lines.push(word);
        } else {
            let last = lines.last_mut().unwrap();
            last.push(' ');
            last.push_str(&word);
        }
    }
    // "rehue" and the command share the first line
    let first = format!("{} {}", lines[0], lines[1]);
    let mut out = vec![first];
    out.extend(lines[2..].iter().map(|l| format!("  {l}")));
    Ok(format!("```sh\n{}\n```", out.join(" \\\n")))
}

/// The command word of a ladder (`map-wal`).
pub fn ladder_command(ladder: &Ladder) -> &str {
    ladder.base.split_whitespace().next().unwrap_or("")
}

/// Config file name shown for a rung's `config`.
pub const RUNG_CONFIG: &str = "config.json";

/// The docs recipe of a ladder: one line per rung, `base` left out.
pub fn display_ladder(m: &Manifest, ladder: &Ladder) -> Result<String, String> {
    let none = |_: &str| -> Result<String, String> { Err("ladders can't use {out}".into()) };
    let mut lines = Vec::new();
    for rung in &ladder.rungs {
        if !rung.note.is_empty() {
            lines.push(format!("# {}", rung.note));
        }
        let mut cmd = vec!["rehue".to_string(), ladder_command(ladder).to_string()];
        for part in [&ladder.common, &rung.args] {
            for t in part.split_whitespace() {
                cmd.push(shell_word(&m.expand(t, &none)?));
            }
        }
        if let Some(cfg) = &rung.config {
            let json = serde_json::to_string(cfg).map_err(|e| e.to_string())?;
            lines.push(format!("# {RUNG_CONFIG}: {json}"));
            cmd.push("--config".into());
            cmd.push(RUNG_CONFIG.into());
        }
        lines.push(cmd.join(" "));
    }
    Ok(format!("```sh\n{}\n```", lines.join("\n")))
}

/// What a ladder rung is labelled with in its image.
pub fn rung_label(rung: &Rung) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !rung.args.trim().is_empty() {
        parts.push(rung.args.trim().to_string());
    }
    if let Some(cfg) = &rung.config {
        for (k, v) in cfg {
            parts.push(format!("{k} {v}"));
        }
    }
    if parts.is_empty() {
        "defaults".into()
    } else {
        parts.join(" ")
    }
}

/// The docs block listing each ladder command's inputs once.
pub fn display_bases(m: &Manifest) -> Result<String, String> {
    let none = |_: &str| -> Result<String, String> { Ok("out".into()) };
    let mut seen: Vec<String> = Vec::new();
    let mut lines = Vec::new();
    for l in &m.ladder {
        let mut words = vec!["rehue".to_string()];
        for t in l.base.split_whitespace() {
            words.push(shell_word(&m.expand(t, &none)?));
        }
        let line = words.join(" ");
        if !seen.contains(&line) {
            lines.push(format!("# {} recipes run as:", ladder_command(l)));
            lines.push(line.clone());
            seen.push(line);
        }
    }
    Ok(format!("```sh\n{}\n```", lines.join("\n")))
}
