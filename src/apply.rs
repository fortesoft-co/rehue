//! `rehue apply`: run a pipeline file and hand the result to the desktop.
//!
//! The pipeline is a TOML file.  `[rehue.scheme]` and `[rehue.wal]` are the
//! same records the JSON `--config` files take (same keys, same merge
//! rules), so a dial means the same thing everywhere; a table's presence
//! is what turns its step on.  `look = "pop"` under `[rehue]` starts
//! both steps from a look; a step's own `look` key replaces it.  `[apply]`
//! says where the result goes:
//! tinty (templates for every app it knows) and a wallpaper command.
//!
//! rehue owns the colours and the wallpaper; tinty owns the per-app
//! templates and hooks.  The handoff is tinty's documented custom-scheme
//! path: `<data-dir>/custom-schemes/base16/<name>.yaml`, then
//! `tinty apply base16-<name>`.  Nothing is patched in place.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::grade_scheme::SchemeConfig;
use crate::grade_wal::WalConfig;

/// A pipeline file.  Paths are resolved against the file's directory;
/// a leading `~/` expands to `$HOME`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Pipeline {
    /// The source wallpaper.
    pub wallpaper: PathBuf,
    /// The reference scheme: a bundled name or a path to a yaml file.
    pub scheme: String,
    /// Optional directory searched for scheme names before the bundled set.
    pub scheme_dir: Option<PathBuf>,
    /// Where the outputs land (absent: `$XDG_DATA_HOME/rehue/current`).
    pub out: Option<PathBuf>,
    /// The pipeline file's directory: relative paths resolve against it
    /// and commands run in it.
    #[serde(skip)]
    pub base_dir: PathBuf,
    /// The `[rehue]` table as written: looks and the two step tables.
    #[serde(default, rename = "rehue")]
    raw_steps: RawSteps,
    /// The grading steps, `[rehue.scheme]` and `[rehue.wal]`, resolved
    /// (looks layered under each table).
    #[serde(skip)]
    pub rehue: Steps,
    /// Where the result goes.
    #[serde(default)]
    pub apply: ApplyTarget,
}

/// The `[rehue]` table as written.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSteps {
    look: Option<LookNames>,
    scheme: Option<toml::Table>,
    wal: Option<toml::Table>,
}

/// `look = "pop"`, `look = "snap,pop"` or `look = ["snap", "pop"]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum LookNames {
    One(String),
    Many(Vec<String>),
}

impl LookNames {
    fn names(&self) -> Vec<String> {
        match self {
            Self::One(s) => crate::look::names(std::slice::from_ref(s)),
            Self::Many(v) => crate::look::names(v),
        }
    }
}

/// The grading steps: a table's presence turns its step on (an empty
/// table runs at defaults).
#[derive(Debug, Default)]
pub struct Steps {
    /// Rehue the scheme with the wallpaper; the result is what gets
    /// applied.
    pub scheme: Option<SchemeConfig>,
    /// Repaint the wallpaper (with the rehued scheme when
    /// `[rehue.scheme]` ran, else the reference); the result is what gets
    /// set.
    pub wal: Option<WalConfig>,
}

/// The `[apply]` table.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields, default)]
pub struct ApplyTarget {
    /// Hand the scheme to tinty (default true).
    pub tinty: bool,
    /// The custom-scheme name tinty sees: `base16-<name>` (default `rehue`).
    pub tinty_name: String,
    /// Passed to tinty as `--config` (absent: tinty's own default).
    pub tinty_config: Option<PathBuf>,
    /// Passed to tinty as `--data-dir` (absent: tinty's own default).
    pub tinty_data_dir: Option<PathBuf>,
    /// Shell command that sets the wallpaper; `{wallpaper}` becomes the
    /// (quoted) image path, e.g. `swww img {wallpaper}`.
    pub wallpaper_command: Option<String>,
}

impl Default for ApplyTarget {
    fn default() -> Self {
        Self {
            tinty: true,
            tinty_name: "rehue".to_string(),
            tinty_config: None,
            tinty_data_dir: None,
            wallpaper_command: None,
        }
    }
}

impl Pipeline {
    /// Read and validate a pipeline file, resolving its paths.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("can't read pipeline {}: {}", path.display(), e))?;
        let mut pipeline: Pipeline =
            toml::from_str(&text).map_err(|e| format!("bad pipeline {}: {}", path.display(), e))?;
        if pipeline.raw_steps.scheme.is_none() && pipeline.raw_steps.wal.is_none() {
            return Err(format!(
                "pipeline {} has no steps: add [rehue.scheme] and/or [rehue.wal]",
                path.display()
            ));
        }
        let name = &pipeline.apply.tinty_name;
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!(
                "tinty-name must be letters, digits, '-' or '_' (got {name:?})"
            ));
        }
        let base = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let base = base.as_path();
        pipeline.base_dir = base.to_path_buf();
        pipeline.rehue = resolve_steps(&pipeline.raw_steps, base, path)?;
        pipeline.wallpaper = resolve(base, &pipeline.wallpaper);
        pipeline.scheme_dir = pipeline.scheme_dir.map(|p| resolve(base, &p));
        pipeline.out = Some(match pipeline.out {
            Some(p) => resolve(base, &p),
            None => default_out()?,
        });
        // A scheme with a '/' is a path (same rule as --scheme); bare names
        // stay names.
        if pipeline.scheme.contains('/') {
            pipeline.scheme = resolve(base, Path::new(&pipeline.scheme))
                .to_string_lossy()
                .into_owned();
        }
        let target = &mut pipeline.apply;
        target.tinty_config = target.tinty_config.take().map(|p| resolve(base, &p));
        target.tinty_data_dir = target.tinty_data_dir.take().map(|p| resolve(base, &p));
        Ok(pipeline)
    }
}

/// The step tables with their looks layered under them.  Look paths
/// (names with a '/') resolve against the pipeline's directory.
fn resolve_steps(raw: &RawSteps, base: &Path, path: &Path) -> Result<Steps, String> {
    use crate::look::{Target, resolve as resolve_look};
    let rebase = |names: Vec<String>| -> Vec<String> {
        names
            .into_iter()
            .map(|n| {
                if n.contains('/') {
                    resolve(base, Path::new(&n)).to_string_lossy().into_owned()
                } else {
                    n
                }
            })
            .collect()
    };
    let shared = raw.look.as_ref().map(|l| rebase(l.names()));
    let step = |table: &toml::Table| -> Result<serde_json::Value, String> {
        let mut value = crate::config::from_toml(table.clone());
        if let Some(own) = crate::look::take_look(&mut value)? {
            value["look"] = serde_json::json!(rebase(own));
        }
        Ok(value)
    };
    let what = |target: Target| format!("{} [rehue.{}]", path.display(), target.name());
    Ok(Steps {
        scheme: match &raw.scheme {
            Some(t) => Some(
                resolve_look(
                    &what(Target::Scheme),
                    step(t)?,
                    &[],
                    shared.clone(),
                    Target::Scheme,
                )?
                .0,
            ),
            None => None,
        },
        wal: match &raw.wal {
            Some(t) => Some(
                resolve_look(
                    &what(Target::Wal),
                    step(t)?,
                    &[],
                    shared.clone(),
                    Target::Wal,
                )?
                .0,
            ),
            None => None,
        },
    })
}

impl Pipeline {
    /// The resolved output directory.
    pub fn out_dir(&self) -> &Path {
        self.out.as_deref().expect("resolved by load")
    }
}

/// `~/x` against `$HOME`, relative paths against `base`, absolute as-is.
pub fn resolve(base: &Path, path: &Path) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// `$XDG_DATA_HOME/rehue/current`, falling back to `~/.local/share`.
fn default_out() -> Result<PathBuf, String> {
    if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(data).join("rehue/current"));
    }
    let home = std::env::var_os("HOME")
        .ok_or("neither XDG_DATA_HOME nor HOME is set; give the pipeline an `out`")?;
    Ok(PathBuf::from(home).join(".local/share/rehue/current"))
}

/// Single-quote a string for `sh`.
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The wallpaper command with `{wallpaper}` filled in (quoted).
pub fn wallpaper_command_line(template: &str, wallpaper: &Path) -> String {
    template.replace("{wallpaper}", &shell_quote(&wallpaper.to_string_lossy()))
}

/// Run a shell command line from `cwd` (the pipeline's directory, so
/// relative paths mean the same as in the file), with the outputs exposed
/// as environment variables (`REHUE_WALLPAPER`, `REHUE_SCHEME`).
pub fn run_shell(line: &str, cwd: &Path, wallpaper: &Path, scheme: &Path) -> Result<(), String> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(line)
        .current_dir(cwd)
        .env("REHUE_WALLPAPER", wallpaper)
        .env("REHUE_SCHEME", scheme)
        .status()
        .map_err(|e| format!("can't run sh: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("command failed ({status}): {line}"))
    }
}

/// tinty, with the pipeline's `--config` / `--data-dir` (when set).
fn tinty(target: &ApplyTarget) -> Command {
    let mut command = Command::new("tinty");
    if let Some(config) = &target.tinty_config {
        command.arg("--config").arg(config);
    }
    if let Some(data) = &target.tinty_data_dir {
        command.arg("--data-dir").arg(data);
    }
    command
}

/// The tinty invocation that applies the scheme, for display.
pub fn tinty_apply_line(target: &ApplyTarget) -> String {
    let mut parts = vec!["tinty".to_string()];
    if let Some(config) = &target.tinty_config {
        parts.push("--config".to_string());
        parts.push(shell_quote(&config.to_string_lossy()));
    }
    if let Some(data) = &target.tinty_data_dir {
        parts.push("--data-dir".to_string());
        parts.push(shell_quote(&data.to_string_lossy()));
    }
    parts.push("apply".to_string());
    parts.push(format!("base16-{}", target.tinty_name));
    parts.join(" ")
}

/// Ask tinty where its data directory is.
pub fn tinty_data_dir(target: &ApplyTarget) -> Result<PathBuf, String> {
    let output = tinty(target)
        .args(["config", "--data-dir-path"])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "tinty not found on PATH (install it, or set `tinty = false` under [apply])"
                    .to_string()
            } else {
                format!("can't run tinty: {e}")
            }
        })?;
    if !output.status.success() {
        return Err(format!(
            "tinty config --data-dir-path failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if dir.is_empty() {
        return Err("tinty config --data-dir-path printed nothing".to_string());
    }
    Ok(PathBuf::from(dir))
}

/// Where the custom scheme goes inside tinty's data directory.
pub fn tinty_scheme_path(data_dir: &Path, name: &str) -> PathBuf {
    data_dir
        .join("custom-schemes/base16")
        .join(format!("{name}.yaml"))
}

/// The scheme as tinty should see it: `slug` pinned to the custom-scheme
/// name.  tinty finds the theme files by the scheme's id
/// (`base16-<file name>`), but its builder names them after the slug,
/// which otherwise derives from `name` ("Tokyo Night Dark (hue-mapped)").
pub fn with_slug(scheme_yaml: &str, slug: &str) -> String {
    let mut out = Vec::new();
    let mut placed = false;
    for line in scheme_yaml.lines() {
        if line.starts_with("slug:") {
            continue;
        }
        out.push(line.to_string());
        if !placed && line.starts_with("name:") {
            out.push(format!("slug: \"{slug}\""));
            placed = true;
        }
    }
    if !placed {
        out.insert(0, format!("slug: \"{slug}\""));
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// tinty's version as (major, minor, patch), from `tinty --version`.
fn tinty_version(target: &ApplyTarget) -> Option<(u32, u32, u32)> {
    let output = tinty(target).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.split_whitespace().nth(1)?;
    let mut parts = version.split('.').map(|p| p.parse::<u32>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

/// Install the scheme as a tinty custom scheme and apply it.  Returns
/// where the scheme was installed.
pub fn tinty_apply(target: &ApplyTarget, scheme_yaml: &str) -> Result<PathBuf, String> {
    let data_dir = tinty_data_dir(target)?;
    // Custom schemes only get their themes built on `apply` from 0.35.
    if let Some(version) = tinty_version(target)
        && version < (0, 35, 0)
    {
        return Err(format!(
            "tinty {}.{}.{} can't build themes for custom schemes; rehue apply needs tinty 0.35 or newer",
            version.0, version.1, version.2
        ));
    }
    let dest = tinty_scheme_path(&data_dir, &target.tinty_name);
    let parent = dest.parent().expect("scheme path has a parent");
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("can't create {}: {}", parent.display(), e))?;
    std::fs::write(&dest, with_slug(scheme_yaml, &target.tinty_name))
        .map_err(|e| format!("can't write {}: {}", dest.display(), e))?;
    let output = tinty(target)
        .arg("apply")
        .arg(format!("base16-{}", target.tinty_name))
        .output()
        .map_err(|e| format!("can't run tinty: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    print!("{stdout}");
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        return Err(format!("tinty apply failed ({})", output.status));
    }
    // tinty reports a missing theme on stdout and still exits 0.
    let missing: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains("Theme does not exist"))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "tinty applied, but {} template item(s) got no theme (see above)",
            missing.len()
        ));
    }
    Ok(dest)
}
