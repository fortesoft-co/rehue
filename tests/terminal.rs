//! The ANSI strip renderers (the terminal previews).

mod common;

use common::fixture;
use rehue::extract::Cluster;
use rehue::scheme::Scheme;

#[test]
fn terminal_strips_render_and_stay_deterministic() {
    let scheme = Scheme::parse_file(&fixture("solarized-dark.yaml")).expect("fixture parses");
    let hexes = scheme.slot_hexes().expect("slots");
    let strip = rehue::terminal::scheme_strip(&hexes);
    assert_eq!(strip.lines().count(), 4, "four slots per line\n{strip}");
    assert!(
        strip.contains("002b36") && strip.contains("d33682"),
        "canonical order + hex labels\n{strip}"
    );
    assert_eq!(
        strip,
        rehue::terminal::scheme_strip(&hexes),
        "deterministic"
    );

    // Families mode: one cell per family, positions = the [i] indices.
    let families = vec![
        Cluster {
            hue: 222.0,
            weight: 0.6,
            chroma: 0.05,
        },
        Cluster {
            hue: 23.1,
            weight: 0.4,
            chroma: 0.125,
        },
    ];
    let line = rehue::terminal::families_strip(&families);
    assert_eq!(line.matches("██").count(), 2, "{line}");
    assert_eq!(
        line,
        rehue::terminal::families_strip(&families),
        "deterministic"
    );
}
