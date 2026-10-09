//! The compiled-in HTML preview (`preview.html`).

mod common;

use common::solarized;
use rehue::scheme::BASE_SLOTS;

#[test]
fn preview_html_renders_the_full_page_deterministically() {
    let (slot_hexes, _) = solarized();
    let html = rehue::preview::render("Solarized Dark", &slot_hexes).expect("render runs");
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
    ] {
        assert!(html.contains(token), "views present: {token}");
    }
    assert!(!html.contains("{{"), "no unfilled placeholders");
    let again = rehue::preview::render("Solarized Dark", &slot_hexes).expect("render runs");
    assert_eq!(html, again, "deterministic");

    let short: Vec<String> = slot_hexes[..15].to_vec();
    let err = rehue::preview::render("x", &short).expect_err("wrong slot count");
    assert!(err.contains("16 slots"), "{err}");
}
