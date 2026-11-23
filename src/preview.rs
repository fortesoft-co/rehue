//! The compiled-in `preview.html`: CSS-only views, GTK and Qt mocks.
//!
//! The HTML preview (`preview.html`): a compiled-in, script-free page
//! that renders a palette as terminal / code / control mocks plus two
//! widget suites mirroring real theme wiring.  GTK view = the
//! adw-gtk3 + `@define-color` css that stylix ships (accent base0D,
//! success base0B, destructive base08, chrome base01); Qt view =
//! stylix's `kvconfig.mustache` colour mapping, exactly (window
//! base01, button base02, highlight base0E with text base00, link
//! base0D, visited base0E).  Views toggle with pure CSS (radio +
//! sibling selectors); derived tints ride `color-mix`; the heading
//! (icons, arrow, per-name ellipsis caps) and the wallpaper card are
//! the template's job — this module fills values only.  Every byte is
//! deterministic.

use std::path::Path;

/// The template, next to this source file in `src/`.
const TEMPLATE: &str = include_str!("preview-template.html");

/// Which side leads the heading — and where the hues flowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewFlow {
    /// scheme: the wallpaper's hues flow into the scheme
    /// (wallpaper first, arrow as drawn).
    SchemeFromWallpaper,
    /// wal: the scheme's palette paints the wallpaper (scheme
    /// first; the template flips the pair).
    WallpaperFromScheme,
    /// inspect --scheme: no wallpaper side at all.
    SchemeAlone,
}

/// Everything the page renders, per flow.  `wallpaper_label` is the
/// (already label-shaped) wallpaper name; `wallpaper_src` is the img
/// source, ready to link — wal links its own repainted
/// `wallpaper.png`, scheme links the source file relative to out,
/// and [`PreviewFlow::SchemeAlone`] carries neither.
pub struct Preview<'a> {
    pub flow: PreviewFlow,
    pub wallpaper_label: Option<&'a str>,
    pub wallpaper_src: Option<&'a str>,
    pub scheme: &'a str,
    pub hexes: &'a [String],
    /// Slot indices the wal `palette` left out (dimmed swatches); empty
    /// when every slot took part.
    pub left_out: &'a [usize],
}

/// A link from the artifact directory to a file: component walk with
/// `../` ascents; falls back to the absolute path when the trees don't
/// share a prefix.
pub fn relative_link(from_dir: &Path, to: &Path) -> String {
    let from = std::path::absolute(from_dir).unwrap_or_else(|_| from_dir.to_path_buf());
    let to = std::path::absolute(to).unwrap_or_else(|_| to.to_path_buf());
    let fcomps: Vec<_> = from.components().collect();
    let tcomps: Vec<_> = to.components().collect();
    let mut shared = 0;
    while shared < fcomps.len() && shared < tcomps.len() && fcomps[shared] == tcomps[shared] {
        shared += 1;
    }
    let mut link = std::path::PathBuf::new();
    for _ in shared..fcomps.len() {
        link.push("..");
    }
    for comp in &tcomps[shared..] {
        link.push(comp.as_os_str());
    }
    if link.as_os_str().is_empty() {
        link.push(".");
    }
    link.to_string_lossy().into_owned()
}

/// Whether a browser can show this picture in `preview.html` (by
/// extension): rehue reads TIFF and QOI too, but browsers don't.
pub fn browser_shows(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
        )
    })
}

/// A wallpaper's human label from its file name: separators (-, _, .,
/// whitespace) split away, each word title-cased (first letter upper,
/// the rest lower), joined back with single spaces; edge and duplicate
/// separators vanish through the split+filter.
pub fn image_label(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    stem.split(|c: char| c.is_whitespace() || matches!(c, '-' | '_' | '.'))
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut rest = word.chars();
            let head = rest.next().unwrap_or_default();
            head.to_uppercase().collect::<String>() + &rest.as_str().to_lowercase()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Escape the variable text (scheme and wallpaper names, src URLs); the
/// palette hexes are already normalized.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render the preview page.  The browser-tab text keeps the full pair
/// (the browser truncates its own tabs); the heading's per-name cap is
/// css.  Unfilled or leftover placeholders fail closed.
pub fn render(input: &Preview) -> Result<String, String> {
    let Preview {
        flow,
        wallpaper_label,
        wallpaper_src,
        scheme,
        hexes,
        left_out,
    } = input;
    if hexes.len() != 16 {
        return Err(format!(
            "the preview renders 16 slots (canonical base order), got {}",
            hexes.len()
        ));
    }
    let paired = *flow != PreviewFlow::SchemeAlone;
    if paired && wallpaper_label.is_none() {
        return Err("the paired flows carry a wallpaper name; inspect alone does not".to_string());
    }
    let (mode, tab) = match (flow, wallpaper_label) {
        (PreviewFlow::SchemeFromWallpaper, Some(wall)) => ("", format!("{wall} → {scheme}")),
        (PreviewFlow::WallpaperFromScheme, Some(wall)) => ("rev", format!("{scheme} → {wall}")),
        (PreviewFlow::SchemeAlone, _) => ("alone", scheme.to_string()),
        (_, None) => unreachable!("checked above"),
    };
    let (wall_mode, wall_src) = match wallpaper_src {
        Some(src) if !src.is_empty() => ("", escape_html(src)),
        _ => (" off", String::new()),
    };
    let mut html = TEMPLATE.to_string();
    html = html.replace("{{tab}}", &escape_html(&tab));
    html = html.replace("{{wall}}", &escape_html(wallpaper_label.unwrap_or("")));
    html = html.replace("{{scheme}}", &escape_html(scheme));
    html = html.replace("{{mode}}", mode);
    html = html.replace("{{wall-mode}}", wall_mode);
    // a video wallpaper plays, silently and on a loop
    let wall_media = if wallpaper_src.is_some_and(|src| crate::video::is_video(Path::new(src))) {
        format!(r#"<video src="{wall_src}" autoplay loop muted playsinline></video>"#)
    } else {
        format!(r#"<img src="{wall_src}" alt="the wallpaper">"#)
    };
    html = html.replace("{{wall-media}}", &wall_media);
    for (index, hex) in hexes.iter().enumerate() {
        html = html.replace(&format!("{{{{base{:02x}}}}}", index), hex);
        let off = if left_out.contains(&index) {
            " out"
        } else {
            ""
        };
        html = html.replace(&format!("{{{{off{:02x}}}}}", index), off);
    }
    if html.contains("{{") {
        return Err("preview template has unfilled placeholders".to_string());
    }
    Ok(html)
}
