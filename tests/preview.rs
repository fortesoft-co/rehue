//! The compiled-in HTML preview (`preview.html`).

mod common;

use std::path::Path;

use common::solarized;
use rehue::preview::{Preview, PreviewFlow, browser_shows, relative_link, render};
use rehue::scheme::BASE_SLOTS;

fn alone(scheme: &str) -> Preview<'_> {
    Preview {
        flow: PreviewFlow::SchemeAlone,
        wallpaper_label: None,
        wallpaper_src: None,
        scheme,
        // filled by the caller per test
        hexes: &[],
        left_out: &[],
    }
}

#[test]
fn preview_html_renders_the_full_page_deterministically() {
    let (slot_hexes, _) = solarized();
    let mut page = alone("Solarized Dark");
    page.hexes = &slot_hexes;
    let html = render(&page).expect("render runs");
    for (slot, hex) in BASE_SLOTS.iter().zip(&slot_hexes) {
        assert!(
            html.contains(&format!("--{slot}: #{hex};")),
            "{slot} lands as a css variable"
        );
    }
    for token in [
        "view-palette",
        "view-gtk",
        "view-qt",
        "adwswitch",
        "qprog",
        "cursor",
        // the heading + wallpaper-card machinery is the template's
        "ic-image",
        "ic-brush",
        "class=\"alone\"",
        "text-overflow: ellipsis",
        "wallbox off",
    ] {
        assert!(html.contains(token), "page carries {token}");
    }
    assert!(!html.contains("{{"), "no unfilled placeholders");
    let again = render(&page).expect("render runs");
    assert_eq!(html, again, "deterministic");

    let short: Vec<String> = slot_hexes[..15].to_vec();
    page.hexes = &short;
    let err = render(&page).expect_err("wrong slot count");
    assert!(err.contains("16 slots"), "{err}");
}

#[test]
fn relative_link_walks_to_the_file() {
    assert_eq!(
        relative_link(Path::new("mapped"), Path::new("assets/x.webp")),
        "../assets/x.webp"
    );
    assert_eq!(
        relative_link(Path::new("mapped/deep"), Path::new("assets/x.webp")),
        "../../assets/x.webp"
    );
    // a sibling artifact links bare
    assert_eq!(
        relative_link(Path::new("mapped"), Path::new("mapped/wallpaper.png")),
        "wallpaper.png"
    );
    // absolute trees still compare
    assert_eq!(
        relative_link(Path::new("/a/b/out"), Path::new("/a/b/wall/x.webp")),
        "../wall/x.webp"
    );
}

#[test]
fn image_label_split_titlecases_and_trims() {
    use rehue::preview::image_label as label;
    let case = |name: &str| label(Path::new(name));
    assert_eq!(
        case("kelly-ishmael-butterfly-closeup.webp"),
        "Kelly Ishmael Butterfly Closeup"
    );
    // edge separators and duplicates vanish; runs collapse to one space
    assert_eq!(case("_my  theme_.webp"), "My Theme");
    assert_eq!(case("bango--renders__3d.webp"), "Bango Renders 3d");
    // dots in the stem split too (release tags read as words)
    assert_eq!(case("4k wallpaper.FINAL.webp"), "4k Wallpaper Final");
    // per-word title case lowercases the rest
    assert_eq!(case("HDR-wall.jpg"), "Hdr Wall");
    // separator-only names label empty; bare stems behave the same
    assert_eq!(case("___"), "");
    assert_eq!(case("rosé-pine.webp"), "Rosé Pine");
}

#[test]
fn the_heading_flows_match_the_commands() {
    let (slot_hexes, _) = solarized();
    let tab_of = |html: &str| {
        html.split("<title>")
            .nth(1)
            .and_then(|tab| tab.split("</title>").next())
            .expect("title tag")
            .to_string()
    };

    // scheme: wallpaper first, arrow as drawn, the source file linked
    let page = Preview {
        flow: PreviewFlow::SchemeFromWallpaper,
        wallpaper_label: Some("Bango"),
        wallpaper_src: Some("../assets/bango.webp"),
        scheme: "Gruvbox Light",
        hexes: &slot_hexes,
        left_out: &[],
    };
    let html = render(&page).expect("scheme page");
    assert_eq!(tab_of(&html), "Bango → Gruvbox Light - rehue preview");
    assert!(html.contains("<h1 class=\"\">"));
    assert!(html.contains("src=\"../assets/bango.webp\""));
    assert!(!html.contains("wallbox off"), "the card shows");

    // wal: scheme first, the pair flips in the template, the output
    // wallpaper.png is a sibling
    let page = Preview {
        flow: PreviewFlow::WallpaperFromScheme,
        wallpaper_label: Some("Bango"),
        wallpaper_src: Some("wallpaper.png"),
        scheme: "Gruvbox Light",
        hexes: &slot_hexes,
        left_out: &[],
    };
    let html = render(&page).expect("wal page");
    assert_eq!(tab_of(&html), "Gruvbox Light → Bango - rehue preview");
    assert!(html.contains("<h1 class=\"rev\">"));
    assert!(html.contains("h1.rev .par { transform: rotate(180deg); }"));
    assert!(html.contains("src=\"wallpaper.png\""));

    // inspect --scheme: no wallpaper side at all
    let mut page = alone("Solarized Dark");
    page.hexes = &slot_hexes;
    let html = render(&page).expect("inspect page");
    assert_eq!(tab_of(&html), "Solarized Dark - rehue preview");
    assert!(html.contains("<h1 class=\"alone\">"));

    // the paired flows refuse to run without the wallpaper name
    let bad = Preview {
        flow: PreviewFlow::SchemeFromWallpaper,
        wallpaper_label: None,
        wallpaper_src: Some("assets/x.webp"),
        scheme: "s",
        hexes: &slot_hexes,
        left_out: &[],
    };
    let err = render(&bad).expect_err("paired flows need a wallpaper name");
    assert!(err.contains("wallpaper name"), "{err}");
}

#[test]
fn the_wallpaper_panel_links_only_formats_a_browser_shows() {
    for name in ["a.png", "a.JPG", "a.jpeg", "a.webp", "a.gif", "a.bmp"] {
        assert!(browser_shows(Path::new(name)), "{name}");
    }
    for name in ["a.tiff", "a.tif", "a.qoi", "a"] {
        assert!(!browser_shows(Path::new(name)), "{name}");
    }
}
