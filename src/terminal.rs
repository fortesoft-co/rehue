//! Terminal-native swatch rendering: truecolor ANSI art for `inspect`
//! output.  Always emitted (no TTY detection) — a pipe captures the
//! same bytes, so the render stays deterministic.

use crate::color::{Lch, hex_to_oklch, oklch_to_hex, oklch_to_rgb};
use crate::extract::Cluster;

/// `██` in the colour, followed by the hex — the swatch-plus-label cell.
fn cell(hex: &str) -> String {
    let rgb = oklch_to_rgb(&hex_to_oklch(hex).expect("a schema-normalized hex"));
    let [r, g, b] = rgb;
    format!("\x1b[38;2;{r};{g};{b}m██\x1b[0m {hex}")
}

/// The family row: one cell per family using preview's swatch recipe
/// (l 0.55), positions matching the printed `[i]` indices, tagged with
/// the swatch's hex; the weight share prints in the table below.
pub fn families_strip(clusters: &[Cluster]) -> String {
    let mut line = String::from("  ");
    for cluster in clusters {
        let swatch = oklch_to_hex(&Lch {
            l: 0.55,
            c: cluster.chroma,
            h: cluster.hue,
        });
        line.push_str(&cell(&swatch));
        line.push(' ');
    }
    line
}

/// The canonical scheme view: `██ #hex` per slot, four per line, in
/// canonical order — the registers' tonal structure, readable in any
/// truecolor terminal.
pub fn scheme_strip(slot_hexes: &[String]) -> String {
    let mut out = String::new();
    for chunk in slot_hexes.chunks(4) {
        out.push_str("  ");
        for hex in chunk {
            out.push_str(&cell(hex));
            out.push(' ');
        }
        out.pop(); // trailing space before the newline
        out.push('\n');
    }
    out
}
