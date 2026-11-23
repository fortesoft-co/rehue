//! `rehue icons`: SVG and PNG recoloring, theme layout, folders, the preview.
//!
//! `rehue icons`: recolor an icon theme with a scheme, through the same
//! grade as `wal` ([`ColorMap`], per colour).
//!
//! * SVG: every colour written into the file is rewritten - `#rgb`,
//!   `#rrggbb`, `#rrggbbaa`, `rgb()` / `rgba()`, in attributes, inline
//!   styles and `<style>` blocks (including the `.ColorScheme-*` defaults
//!   KDE swaps at runtime: elsewhere they are what shows), and PNGs
//!   embedded as `data:` URIs.  `currentColor` is left alone.
//! * PNG: every pixel, alpha kept.
//! * Anything else is copied.
//!
//! A theme keeps its layout: symlinks inside it stay symlinks, links that
//! leave it (Papirus-Dark's size folders point into Papirus) are copied
//! in and recoloured, `index.theme` gets the new `Name=`, and the stale
//! `icon-theme.cache` is dropped.
//!
//! Folders can take their own colour (`folders`): a folder is an icon
//! in a `Context=Places` directory named `folder*`, `user-home` or
//! `user-desktop`, or `inode-directory` in `Context=MimeTypes` (the Icon
//! Theme and Icon Naming specs), never a `-symbolic` one; a link with
//! such a name passes it on to the file it points at when that file's
//! name says folder too.
//!
//! Dithering (`dither`, asked for explicitly) is best effort: an SVG in a
//! directory whose size `index.theme` gives is drawn at that size
//! (`Size` x `Scale`), dithered with the scheme's colors and written as
//! a PNG in its place, links to it re-pointed; PNGs are dithered as they
//! are.  Transparency is kept.  Symbolic icons (the toolkit colors those)
//! and icons of unknown size stay recolored SVGs.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dither::{Dither, Ink};
use crate::grade_wal::{ColorMap, WalConfig};

/// The icons config: the wal grade, plus the folder colour.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct IconsConfig {
    #[serde(flatten)]
    pub grade: WalConfig,
    /// The palette folder icons are painted with (the `palette` form);
    /// unset, folders are graded like every other icon.
    #[serde(default)]
    pub folders: Option<HashMap<String, f64>>,
}

// ------------------------------------------------------------- base64

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

// ------------------------------------------------------------- colours

/// One colour literal in an SVG: where it is, its rgb, and how to write
/// it back.
struct Literal {
    at: usize,
    len: usize,
    rgb: [u8; 3],
    /// hex: the alpha digits; rgb(): the `, a` part.
    tail: String,
    function: bool,
}

fn literals(svg: &str) -> Vec<Literal> {
    let bytes = svg.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            let before = svg[..i].trim_end_matches([' ', '\t']);
            // references (`href="#id"`) are ids, not colours
            let reference = before.ends_with("href=\"") || before.ends_with("href='");
            let colour_slot = !reference
                && (before.ends_with(':') || before.ends_with("=\"") || before.ends_with("='"));
            let n = bytes[i + 1..]
                .iter()
                .take_while(|b| b.is_ascii_hexdigit())
                .count();
            if colour_slot && matches!(n, 3 | 6 | 8) {
                let h = &svg[i + 1..i + 1 + n];
                let six: String = if n == 3 {
                    h.chars().flat_map(|c| [c, c]).collect()
                } else {
                    h[..6].to_string()
                };
                let v = u32::from_str_radix(&six, 16).expect("hex digits");
                out.push(Literal {
                    at: i,
                    len: 1 + n,
                    rgb: [(v >> 16) as u8, (v >> 8) as u8, v as u8],
                    tail: if n == 8 {
                        h[6..].to_string()
                    } else {
                        String::new()
                    },
                    function: false,
                });
                i += 1 + n;
                continue;
            }
        }
        if (bytes[i] == b'r')
            && (svg[i..].starts_with("rgb(") || svg[i..].starts_with("rgba("))
            && let (Some(open), Some(close)) = (svg[i..].find('('), svg[i..].find(')'))
        {
            let parts: Vec<&str> = svg[i + open + 1..i + close]
                .split(',')
                .map(str::trim)
                .collect();
            let rgb: Option<Vec<u8>> = parts.iter().take(3).map(|p| p.parse().ok()).collect();
            if let Some(rgb) = rgb.filter(|v| v.len() == 3) {
                out.push(Literal {
                    at: i,
                    len: close + 1,
                    rgb: [rgb[0], rgb[1], rgb[2]],
                    tail: parts[3..].iter().map(|p| format!(", {p}")).collect(),
                    function: true,
                });
                i += close + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

const PNG_URI: &str = "data:image/png;base64,";

/// The colours written into an SVG (not the embedded images'): what the
/// theme's vivid chroma is measured over.
pub fn svg_colours(svg: &str) -> Vec<[u8; 3]> {
    literals(svg).into_iter().map(|l| l.rgb).collect()
}

/// An SVG with every colour literal, and every embedded PNG, recoloured.
pub fn recolor_svg(svg: &str, f: &mut dyn FnMut([u8; 3]) -> [u8; 3]) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut last = 0;
    let lits = literals(svg);
    // embedded PNGs: spans that literal scanning must skip
    let mut images: Vec<(usize, usize)> = Vec::new();
    let mut from = 0;
    while let Some(k) = svg[from..].find(PNG_URI) {
        let start = from + k + PNG_URI.len();
        let len = svg[start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || "+/=\n\r\t ".contains(c)))
            .unwrap_or(svg.len() - start);
        images.push((start, start + len));
        from = start + len;
    }
    let mut events: Vec<(usize, usize, Option<&Literal>)> = lits
        .iter()
        .filter(|l| !images.iter().any(|(a, b)| l.at >= *a && l.at < *b))
        .map(|l| (l.at, l.at + l.len, Some(l)))
        .chain(images.iter().map(|(a, b)| (*a, *b, None)))
        .collect();
    events.sort_by_key(|e| e.0);
    for (at, end, lit) in events {
        out.push_str(&svg[last..at]);
        match lit {
            Some(l) => {
                let [r, g, b] = f(l.rgb);
                if l.function {
                    let name = if l.tail.is_empty() { "rgb" } else { "rgba" };
                    out.push_str(&format!("{name}({r}, {g}, {b}{})", l.tail));
                } else {
                    out.push_str(&format!("#{r:02x}{g:02x}{b:02x}{}", l.tail));
                }
            }
            None => {
                let data = &svg[at..end];
                match base64_decode(data).and_then(|png| recolor_png(&png, f).ok()) {
                    Some(png) => out.push_str(&base64_encode(&png)),
                    None => out.push_str(data),
                }
            }
        }
        last = end;
    }
    out.push_str(&svg[last..]);
    out
}

/// A PNG with every pixel recoloured; alpha (and fully transparent
/// pixels) kept.
pub fn recolor_png(bytes: &[u8], f: &mut dyn FnMut([u8; 3]) -> [u8; 3]) -> Result<Vec<u8>, String> {
    let mut img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    for px in img.pixels_mut() {
        if px.0[3] > 0 {
            let [r, g, b] = f([px.0[0], px.0[1], px.0[2]]);
            px.0 = [r, g, b, px.0[3]];
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

// ------------------------------------------------------------- themes

/// What `-i` names.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Theme(PathBuf),
    File(PathBuf),
}

/// The icon directories a theme name is looked up in, in order:
/// `$XDG_DATA_HOME/icons` (`~/.local/share/icons`), `~/.icons`, then each
/// `$XDG_DATA_DIRS` entry's `icons` (default `/usr/local/share`,
/// `/usr/share`).
pub fn search_dirs() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
    let home = env("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    match (env("XDG_DATA_HOME"), &home) {
        (Some(d), _) => dirs.push(PathBuf::from(d).join("icons")),
        (None, Some(h)) => dirs.push(h.join(".local/share/icons")),
        _ => {}
    }
    if let Some(h) = &home {
        dirs.push(h.join(".icons"));
    }
    let data = env("XDG_DATA_DIRS")
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(
        data.split(':')
            .filter(|d| !d.is_empty())
            .map(|d| PathBuf::from(d).join("icons")),
    );
    dirs
}

/// Resolve `-i`: a path (a '/' in it, or an existing file) is a theme
/// directory or one icon; anything else is a theme name.
pub fn find_input(input: &str) -> Result<Input, String> {
    let path = PathBuf::from(input);
    if input.contains('/') || path.is_file() {
        if path.is_dir() {
            return Ok(Input::Theme(path));
        }
        if path.is_file() {
            return Ok(Input::File(path));
        }
        return Err(format!("no icon theme or icon at {input}"));
    }
    let dirs = search_dirs();
    for dir in &dirs {
        let theme = dir.join(input);
        if theme.join("index.theme").is_file() {
            return Ok(Input::Theme(theme));
        }
    }
    Err(format!(
        "no icon theme named '{input}' in {}",
        dirs.iter()
            .map(|d| d.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// The parts of `index.theme` rehue needs.
#[derive(Debug, Clone, Default)]
pub struct ThemeIndex {
    pub name: Option<String>,
    /// the themes it falls back to (`Inherits=`)
    pub inherits: Vec<String>,
    /// directory (as listed in `Directories=`) -> its `Context=`
    pub contexts: HashMap<String, String>,
    /// directory -> the pixel size its icons are drawn at (`Size` x
    /// `Scale`)
    pub sizes: HashMap<String, u32>,
}

pub fn parse_index(text: &str) -> ThemeIndex {
    let mut index = ThemeIndex::default();
    let mut section = String::new();
    let mut size: HashMap<String, (u32, u32)> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.to_string();
        } else if let Some((k, v)) = line.split_once('=') {
            let (k, v) = (k.trim(), v.trim());
            if section == "Icon Theme" && k == "Name" {
                index.name = Some(v.to_string());
            } else if section == "Icon Theme" && k == "Inherits" {
                index.inherits = v.split(',').map(|t| t.trim().to_string()).collect();
            } else if section != "Icon Theme" && k == "Context" {
                index.contexts.insert(section.clone(), v.to_string());
            } else if section != "Icon Theme"
                && (k == "Size" || k == "Scale")
                && let Ok(n) = v.parse::<u32>()
            {
                let e = size.entry(section.clone()).or_insert((0, 1));
                if k == "Size" {
                    e.0 = n;
                } else {
                    e.1 = n.max(1);
                }
            }
        }
    }
    index.sizes = size
        .into_iter()
        .filter(|(_, (s, _))| *s > 0)
        .map(|(dir, (s, scale))| (dir, s * scale))
        .collect();
    index
}

/// `index.theme` with the new `Name=` (the rest kept as written).
pub fn rename_index(text: &str, name: &str) -> String {
    let mut section = String::new();
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let t = line.trim();
        if let Some(s) = t.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.to_string();
        }
        if section == "Icon Theme" && t.split_once('=').is_some_and(|(k, _)| k.trim() == "Name") {
            out.push_str(&format!("Name={name}"));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

fn folder_name(stem: &str) -> bool {
    !stem.ends_with("-symbolic")
        && (stem.starts_with("folder") || stem == "user-home" || stem == "user-desktop")
}

/// A file a folder-named link points at is painted as a folder only when
/// its own name says folder too (Fluent's `default-folder-music`,
/// Papirus's `user-blue-home`): packs also link folder names to other
/// images (Breeze points some at action icons), which must keep the
/// normal grade.
fn folder_like(stem: &str) -> bool {
    !stem.ends_with("-symbolic")
        && (stem.contains("folder")
            || (stem.contains("user") && (stem.contains("home") || stem.contains("desktop"))))
}

/// Is the icon at `rel` (relative to the theme root) a folder?
pub fn is_folder(rel: &Path, index: &ThemeIndex) -> bool {
    let Some(stem) = rel.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let dir = rel
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    match index.contexts.get(&dir).map(|c| c.to_ascii_lowercase()) {
        Some(c) if c == "places" => folder_name(stem),
        Some(c) if c == "mimetypes" => stem == "inode-directory",
        _ => false,
    }
}

enum Entry {
    Dir,
    /// a file to process, from wherever it really lives
    File(PathBuf),
    /// a symlink kept as one: its target relative to the theme root
    Link(PathBuf),
    /// a symlink kept verbatim (dangling)
    Verbatim(PathBuf),
}

/// The theme laid out: every output path (relative to the theme root)
/// and what goes there.
struct Plan {
    entries: Vec<(PathBuf, Entry)>,
    /// source directories walked as part of the theme: canonical path ->
    /// where it lands
    mounts: Vec<(PathBuf, PathBuf)>,
}

fn mapped(mounts: &[(PathBuf, PathBuf)], abs: &Path) -> Option<PathBuf> {
    mounts
        .iter()
        .find_map(|(src, rel)| abs.strip_prefix(src).ok().map(|rest| rel.join(rest)))
}

fn scan(abs: &Path, rel: &Path, plan: &mut Plan) -> Result<(), String> {
    let mut children: Vec<_> = std::fs::read_dir(abs)
        .map_err(|e| format!("can't read {}: {e}", abs.display()))?
        .flatten()
        .collect();
    children.sort_by_key(|c| c.file_name());
    for child in children {
        let path = child.path();
        let rel_child = rel.join(child.file_name());
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            match std::fs::canonicalize(&path) {
                Err(_) => {
                    let text = std::fs::read_link(&path).map_err(|e| e.to_string())?;
                    plan.entries.push((rel_child, Entry::Verbatim(text)));
                }
                Ok(target) => match mapped(&plan.mounts, &target) {
                    Some(target_rel) => plan.entries.push((rel_child, Entry::Link(target_rel))),
                    None if target.is_dir() => {
                        plan.mounts.push((target.clone(), rel_child.clone()));
                        plan.entries.push((rel_child.clone(), Entry::Dir));
                        scan(&target, &rel_child, plan)?;
                    }
                    None => plan.entries.push((rel_child, Entry::File(target))),
                },
            }
        } else if meta.is_dir() {
            plan.entries.push((rel_child.clone(), Entry::Dir));
            scan(&path, &rel_child, plan)?;
        } else {
            plan.entries.push((rel_child, Entry::File(path)));
        }
    }
    Ok(())
}

/// `target` relative to the directory `from` (both relative to one root).
fn relative(from: &Path, target: &Path) -> PathBuf {
    let f: Vec<Component> = from.components().collect();
    let t: Vec<Component> = target.components().collect();
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..f.len() {
        out.push("..");
    }
    for c in &t[common..] {
        out.push(c);
    }
    out
}

/// The two grades a theme is painted with, and the dither, if asked for.
pub struct Painter<'a> {
    pub icons: ColorMap<'a>,
    pub folders: ColorMap<'a>,
    pub dither: Option<IconDither>,
}

/// Dithering icons: the dither and the colors each grade draws with.
pub struct IconDither {
    pub dither: Dither,
    pub icon_inks: Vec<Ink>,
    pub folder_inks: Vec<Ink>,
}

/// The size a single icon file is dithered at (no theme to say).
pub const SINGLE_ICON_SIZE: u32 = 48;

/// What a run did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Stats {
    pub svg: usize,
    pub png: usize,
    pub copied: usize,
    pub links: usize,
    /// files copied in from outside the theme (links that left it)
    pub brought_in: usize,
    pub folders: usize,
    pub colours: usize,
    /// icons dithered (SVGs drawn to PNG, and PNGs)
    pub dithered: usize,
}

/// A theme's colours, for its vivid chroma: every literal in its SVGs.
pub fn theme_colours(root: &Path) -> Result<Vec<u8>, String> {
    let root = canonical(root)?;
    let mut plan = Plan {
        entries: Vec::new(),
        mounts: vec![(root.clone(), PathBuf::new())],
    };
    scan(&root, Path::new(""), &mut plan)?;
    let mut out = Vec::new();
    for (_, e) in &plan.entries {
        if let Entry::File(src) = e
            && src.extension().is_some_and(|x| x == "svg")
            && let Ok(text) = std::fs::read_to_string(src)
        {
            out.extend(svg_colours(&text).into_iter().flatten());
        }
    }
    Ok(out)
}

fn canonical(p: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(p).map_err(|e| format!("can't read {}: {e}", p.display()))
}

/// The theme's index (contexts, name).
pub fn read_index(root: &Path) -> Result<ThemeIndex, String> {
    let text = std::fs::read_to_string(root.join("index.theme"))
        .map_err(|e| format!("{} has no readable index.theme: {e}", root.display()))?;
    Ok(parse_index(&text))
}

/// Recolour the theme at `src` into `out` (`None`: a dry run, nothing
/// written) under the theme name `name`.  Files are painted in parallel;
/// each distinct colour is mapped once per worker.
pub fn recolor_theme(
    src: &Path,
    out: Option<&Path>,
    name: &str,
    painter: &Painter,
) -> Result<Stats, String> {
    let index = read_index(src)?;
    let root = canonical(src)?;
    let mut plan = Plan {
        entries: Vec::new(),
        mounts: vec![(root.clone(), PathBuf::new())],
    };
    scan(&root, Path::new(""), &mut plan)?;

    // folders: by context and name, plus what folder-named links point at
    // (Papirus: folder.svg -> folder-blue.svg, user-home -> user-blue-home)
    let mut folder_files: HashSet<PathBuf> = HashSet::new();
    let link_targets: HashMap<&Path, &Path> = plan
        .entries
        .iter()
        .filter_map(|(rel, e)| match e {
            Entry::Link(t) => Some((rel.as_path(), t.as_path())),
            _ => None,
        })
        .collect();
    for (rel, e) in &plan.entries {
        if is_folder(rel, &index) {
            match e {
                Entry::File(_) => {
                    folder_files.insert(rel.clone());
                }
                Entry::Link(_) => {
                    let mut t = rel.as_path();
                    let mut hops = 0;
                    while let Some(next) = link_targets.get(t) {
                        t = next;
                        hops += 1;
                        if hops > 16 {
                            break;
                        }
                    }
                    if t.file_stem()
                        .and_then(|x| x.to_str())
                        .is_some_and(folder_like)
                    {
                        folder_files.insert(t.to_path_buf());
                    }
                }
                _ => {}
            }
        }
    }

    // dithering: which SVGs become PNGs (a known size, not symbolic), and
    // which links follow them to the new name
    let mut converted: HashSet<PathBuf> = HashSet::new();
    if painter.dither.is_some() {
        for (rel, e) in &plan.entries {
            if matches!(e, Entry::File(_)) && dither_size(rel, &index).is_some() {
                converted.insert(rel.clone());
            }
        }
        loop {
            let before = converted.len();
            for (rel, e) in &plan.entries {
                if let Entry::Link(t) = e
                    && is_svg(rel)
                    && converted.contains(t)
                {
                    converted.insert(rel.clone());
                }
            }
            if converted.len() == before {
                break;
            }
        }
    }
    let renamed = |rel: &Path| -> PathBuf {
        if converted.contains(rel) {
            rel.with_extension("png")
        } else {
            rel.to_path_buf()
        }
    };

    let mut stats = Stats::default();
    if let Some(out) = out {
        std::fs::create_dir_all(out).map_err(|e| format!("can't create {}: {e}", out.display()))?;
    }
    let mut jobs: Vec<Job> = Vec::new();
    for (rel, e) in &plan.entries {
        match e {
            Entry::Dir => {
                if let Some(out) = out {
                    std::fs::create_dir_all(out.join(rel)).map_err(|e| e.to_string())?;
                }
            }
            Entry::Link(target) | Entry::Verbatim(target) => {
                stats.links += 1;
                if let Some(out) = out {
                    let text = match e {
                        Entry::Link(_) => {
                            relative(rel.parent().unwrap_or(Path::new("")), &renamed(target))
                        }
                        _ => target.clone(),
                    };
                    let dest = out.join(renamed(rel));
                    let _ = std::fs::remove_file(&dest);
                    symlink(&text, &dest)?;
                }
            }
            Entry::File(from) => {
                if !from.starts_with(&root) {
                    stats.brought_in += 1;
                }
                jobs.push(Job {
                    rel: rel.clone(),
                    from: from.clone(),
                    folder: folder_files.contains(rel),
                    dither_at: if converted.contains(rel)
                        || (is_png(rel) && painter.dither.is_some())
                    {
                        dither_size(rel, &index)
                    } else {
                        None
                    },
                });
            }
        }
    }

    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = jobs.len().div_ceil(workers).max(1);
    let results: Vec<Result<WorkerResult, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(chunk)
            .map(|part| {
                let index_name = name;
                scope.spawn(move || paint_files(part, out, index_name, painter))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("worker"))
            .collect()
    });
    let mut colours = HashSet::new();
    for r in results {
        let (s, c) = r?;
        stats.svg += s.svg;
        stats.png += s.png;
        stats.copied += s.copied;
        stats.folders += s.folders;
        stats.dithered += s.dithered;
        colours.extend(c);
    }
    stats.colours = colours.len();
    Ok(stats)
}

#[cfg(unix)]
fn symlink(target: &Path, at: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(target, at).map_err(|e| format!("can't link {}: {e}", at.display()))
}

#[cfg(not(unix))]
fn symlink(_target: &Path, at: &Path) -> Result<(), String> {
    Err(format!(
        "can't link {}: symlinks need a unix system",
        at.display()
    ))
}

/// One worker's tally and the colours it mapped.
type WorkerResult = (Stats, HashSet<[u8; 3]>);

/// A file to paint: where it goes, where it's read from, whether it's a
/// folder, and the size to dither it at (when dithering it).
struct Job {
    rel: PathBuf,
    from: PathBuf,
    folder: bool,
    dither_at: Option<u32>,
}

fn is_svg(rel: &Path) -> bool {
    rel.extension().is_some_and(|x| x == "svg")
}

fn is_png(rel: &Path) -> bool {
    rel.extension().is_some_and(|x| x == "png")
}

/// The size an icon is dithered at: its directory's size from the index,
/// for an SVG or PNG that isn't symbolic.
fn dither_size(rel: &Path, index: &ThemeIndex) -> Option<u32> {
    if !(is_svg(rel) || is_png(rel)) || rel.to_string_lossy().contains("symbolic") {
        return None;
    }
    let dir = rel
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    index.sizes.get(&dir).copied()
}

/// Draw SVG text at `size` x `size`, straight RGBA.
pub fn rasterize_svg(text: &[u8], resources: Option<&Path>, size: u32) -> Option<image::RgbaImage> {
    let options = resvg::usvg::Options {
        resources_dir: resources.map(Path::to_path_buf),
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_data(text, &options).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    let s = tree.size();
    let scale = size as f32 / s.width().max(s.height());
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut img = image::RgbaImage::new(size, size);
    for (px, c) in img.pixels_mut().zip(pixmap.pixels()) {
        let c = c.demultiply();
        px.0 = [c.red(), c.green(), c.blue(), c.alpha()];
    }
    Some(img)
}

/// Dither an icon's colors, keeping its transparency.  Fully transparent
/// pixels take the icon's average color first, so they don't pull the
/// pattern at the edges toward black.
pub fn dither_icon(img: &image::RgbaImage, d: &Dither, inks: &[Ink]) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    let mut sum = [0u64; 3];
    let mut n = 0u64;
    for p in img.pixels().filter(|p| p.0[3] > 0) {
        for (total, channel) in sum.iter_mut().zip(p.0) {
            *total += u64::from(channel);
        }
        n += 1;
    }
    let mean = sum.map(|total| total.checked_div(n).unwrap_or(0) as u8);
    let rgb: Vec<u8> = img
        .pixels()
        .flat_map(|p| {
            if p.0[3] == 0 {
                mean
            } else {
                [p.0[0], p.0[1], p.0[2]]
            }
        })
        .collect();
    let (out, _) = crate::dither::dither(&rgb, w, h, inks, d);
    let mut back = image::RgbaImage::new(w, h);
    for (i, px) in back.pixels_mut().enumerate() {
        px.0 = [
            out[i * 3],
            out[i * 3 + 1],
            out[i * 3 + 2],
            img.as_raw()[i * 4 + 3],
        ];
    }
    back
}

fn encode_png(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(buf.into_inner())
}

fn paint_files(
    jobs: &[Job],
    out: Option<&Path>,
    name: &str,
    painter: &Painter,
) -> Result<WorkerResult, String> {
    let mut stats = Stats::default();
    let mut icon_cache: HashMap<[u8; 3], [u8; 3]> = HashMap::new();
    let mut folder_cache: HashMap<[u8; 3], [u8; 3]> = HashMap::new();
    for job in jobs {
        let (rel, from, folder) = (&job.rel, &job.from, &job.folder);
        let file = rel.file_name().and_then(|f| f.to_str()).unwrap_or("");
        if file == "icon-theme.cache" && rel.parent() == Some(Path::new("")) {
            continue;
        }
        let (map, cache) = if *folder {
            stats.folders += 1;
            (&painter.folders, &mut folder_cache)
        } else {
            (&painter.icons, &mut icon_cache)
        };
        let mut f = |rgb: [u8; 3]| *cache.entry(rgb).or_insert_with(|| map.map_rgb(&rgb));
        let read =
            || std::fs::read(from).map_err(|e| format!("can't read {}: {e}", from.display()));
        if let (Some(size), Some(dither)) = (job.dither_at, &painter.dither) {
            let inks = if *folder {
                &dither.folder_inks
            } else {
                &dither.icon_inks
            };
            let bytes = read()?;
            let drawn = if is_svg(rel) {
                std::str::from_utf8(&bytes).ok().and_then(|text| {
                    let painted = recolor_svg(text, &mut f);
                    rasterize_svg(painted.as_bytes(), from.parent(), size)
                })
            } else {
                recolor_png(&bytes, &mut f)
                    .ok()
                    .and_then(|png| image::load_from_memory(&png).ok())
                    .map(|img| img.to_rgba8())
            };
            if let Some(img) = drawn {
                stats.dithered += 1;
                if let Some(out) = out {
                    let dest = out.join(rel).with_extension("png");
                    std::fs::write(&dest, encode_png(&dither_icon(&img, &dither.dither, inks))?)
                        .map_err(|e| format!("can't write {}: {e}", dest.display()))?;
                }
                continue;
            }
            // couldn't be drawn: falls through to a plain recolor
        }
        let body: Vec<u8> = if rel == Path::new("index.theme") {
            let text = String::from_utf8_lossy(&read()?).into_owned();
            stats.copied += 1;
            rename_index(&text, name).into_bytes()
        } else if file.ends_with(".svg") {
            let bytes = read()?;
            match std::str::from_utf8(&bytes) {
                Ok(text) => {
                    stats.svg += 1;
                    recolor_svg(text, &mut f).into_bytes()
                }
                Err(_) => {
                    stats.copied += 1;
                    bytes
                }
            }
        } else if file.ends_with(".png") {
            let bytes = read()?;
            match recolor_png(&bytes, &mut f) {
                Ok(png) => {
                    stats.png += 1;
                    png
                }
                Err(_) => {
                    stats.copied += 1;
                    bytes
                }
            }
        } else {
            stats.copied += 1;
            read()?
        };
        if let Some(out) = out {
            let dest = out.join(rel);
            std::fs::write(&dest, body)
                .map_err(|e| format!("can't write {}: {e}", dest.display()))?;
        }
    }
    let colours = icon_cache
        .into_keys()
        .chain(folder_cache.into_keys())
        .collect();
    Ok((stats, colours))
}

// ------------------------------------------------------------- preview

/// The icons the preview sheet looks for (first name found wins).
pub const PREVIEW_ICONS: &[&str] = &[
    "firefox",
    "chromium|google-chrome",
    "thunderbird|org.mozilla.Thunderbird",
    "gimp",
    "inkscape",
    "blender",
    "steam",
    "vlc",
    "libreoffice-writer",
    "code|vscode|visual-studio-code|com.visualstudio.code",
    "discord",
    "spotify|spotify-client",
    "krita",
    "obs|com.obsproject.Studio",
    "utilities-terminal|org.gnome.Terminal|kitty|konsole",
    "system-file-manager|org.gnome.Nautilus|nautilus|dolphin",
    "telegram|org.telegram.desktop",
    "preferences-system|org.gnome.Settings|systemsettings",
    "accessories-text-editor|org.gnome.TextEditor|kate",
    "accessories-calculator|org.gnome.Calculator",
    "folder",
    "folder-documents",
    "folder-download",
    "folder-music",
    "folder-pictures",
    "user-home",
    "user-trash",
    "application-pdf",
    "text-x-python",
    "image-x-generic",
];

/// The theme's file for each preview icon, relative to the root: the
/// first name found, preferring 48 px, then scalable, then 64 px.
pub fn preview_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let root = canonical(root)?;
    let mut plan = Plan {
        entries: Vec::new(),
        mounts: vec![(root.clone(), PathBuf::new())],
    };
    scan(&root, Path::new(""), &mut plan)?;
    let mut by_stem: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for (rel, e) in &plan.entries {
        if matches!(e, Entry::Dir) {
            continue;
        }
        let s = rel.to_string_lossy();
        let usable = (s.ends_with(".svg") || s.ends_with(".png"))
            && !s.contains("symbolic")
            && !s.contains('@');
        if usable && let Some(stem) = rel.file_stem().and_then(|x| x.to_str()) {
            by_stem
                .entry(stem.to_string())
                .or_default()
                .push(rel.clone());
        }
    }
    let score = |p: &Path| {
        let s = format!("/{}", p.to_string_lossy());
        ["/48x48/", "/48/", "/scalable/", "/64x64/", "/64/"]
            .iter()
            .position(|k| s.contains(k))
            .map_or(0, |i| 10 - i)
    };
    Ok(PREVIEW_ICONS
        .iter()
        .filter_map(|alts| {
            alts.split('|').find_map(|n| {
                by_stem
                    .get(n)
                    .and_then(|v| v.iter().max_by_key(|p| score(p)).cloned())
            })
        })
        .collect())
}

/// One icon as straight RGBA at `size`.
pub fn render_icon(path: &Path, size: u32) -> Option<image::RgbaImage> {
    if path.extension().is_some_and(|x| x == "png") {
        let img = image::open(path).ok()?;
        return Some(
            img.resize_exact(size, size, image::imageops::FilterType::Lanczos3)
                .to_rgba8(),
        );
    }
    let data = std::fs::read(path).ok()?;
    rasterize_svg(&data, path.parent(), size)
}

/// The preview sheet: the scheme's accents on top, then the icons in
/// bands of up to 15, each band one row before and one row after, on the
/// scheme's background.
pub fn preview_sheet(
    before: &[Option<image::RgbaImage>],
    after: &[Option<image::RgbaImage>],
    background: [u8; 3],
    accents: &[[u8; 3]],
) -> image::RgbImage {
    const ICON: u32 = 48;
    const GAP: u32 = 8;
    const PER_BAND: u32 = 15;
    let n = before.len().max(1) as u32;
    let columns = n.min(PER_BAND);
    let bands = n.div_ceil(PER_BAND);
    let band = 2 * (ICON + GAP) + 2 * GAP;
    let width = GAP + columns * (ICON + GAP);
    let strip = 14;
    let height = GAP + strip + 2 * GAP + bands * band - 2 * GAP;
    let mut sheet = image::RgbImage::from_pixel(width, height, image::Rgb(background));
    let sw = (width - 2 * GAP) / accents.len().max(1) as u32;
    for (i, a) in accents.iter().enumerate() {
        for y in GAP..GAP + strip {
            for x in GAP + i as u32 * sw..GAP + (i as u32 + 1) * sw {
                sheet.put_pixel(x, y, image::Rgb(*a));
            }
        }
    }
    let mut paste = |img: &image::RgbaImage, x0: u32, y0: u32| {
        for (x, y, p) in img.enumerate_pixels() {
            let a = f32::from(p.0[3]) / 255.0;
            let bg = sheet.get_pixel(x0 + x, y0 + y).0;
            let mix = |c: u8, b: u8| (f32::from(c) * a + f32::from(b) * (1.0 - a)).round() as u8;
            sheet.put_pixel(
                x0 + x,
                y0 + y,
                image::Rgb([mix(p.0[0], bg[0]), mix(p.0[1], bg[1]), mix(p.0[2], bg[2])]),
            );
        }
    };
    for (i, (b, a)) in before.iter().zip(after).enumerate() {
        let (row, col) = (i as u32 / PER_BAND, i as u32 % PER_BAND);
        let x = GAP + col * (ICON + GAP);
        let top = GAP + strip + 2 * GAP + row * band;
        if let Some(b) = b {
            paste(b, x, top);
        }
        if let Some(a) = a {
            paste(a, x, top + ICON + GAP);
        }
    }
    sheet
}
