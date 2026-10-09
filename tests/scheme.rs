//! The base16 scheme surface: parse/render, the embedded collection,
//! and name resolution.

mod common;

use common::fixture;
use rehue::scheme::Scheme;

#[test]
fn collection_fixture_pins_the_embedded_text() {
    // The vendored fixture is the audit pin: an upstream scheme bump
    // changes this test, not the binary's behaviour silently.
    let text =
        rehue::scheme::collection_text("gruvbox-light").expect("collection carries gruvbox-light");
    let pinned =
        std::fs::read_to_string(fixture("gruvbox-light-upstream.yaml")).expect("fixture readable");
    assert_eq!(text, pinned);
}

#[test]
fn schemes_subcommand_lists_sorted() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let out = std::process::Command::new(bin)
        .args(["schemes"])
        .output()
        .expect("rehue binary runs");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("gruvbox-light"), "{text}");
    assert!(
        text.contains("0x96f"),
        "the lexicographic first name\n{text}"
    );
}

#[test]
fn scheme_name_resolution() {
    // Bare name and .yaml-suffixed name resolve identically, and match a
    // file parse of the same text.
    let bare = Scheme::resolve("gruvbox-light", None).expect("bare name resolves");
    let suffixed = Scheme::resolve("gruvbox-light.yaml", None).expect("suffixed name resolves");
    assert_eq!(
        bare.slot_hexes().unwrap(),
        suffixed.slot_hexes().unwrap(),
        "name resolution is suffix-symmetric"
    );
    let from_file =
        Scheme::parse_file(&fixture("gruvbox-light-upstream.yaml")).expect("file parses");
    assert_eq!(bare.slot_hexes().unwrap(), from_file.slot_hexes().unwrap());

    // A dir-provided name wins over the collection.
    let dir = std::env::temp_dir().join("rehue-scheme-dir-probe");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(
        dir.join("gruvbox-light.yaml"),
        "name: \"GRUVBOX-LOCAL\"\nbase00: \"123456\"\n",
    )
    .unwrap();
    let from_dir = Scheme::resolve("gruvbox-light", Some(&dir)).expect("dir resolves");
    assert_eq!(from_dir.meta_name(), "GRUVBOX-LOCAL", "scheme-dir wins");
    let _ = std::fs::remove_dir_all(&dir);

    // Unknown names error loudly and suggest the nearest names.
    let err = Scheme::resolve("gruvbox-lite", None).expect_err("unknown name");
    assert!(err.contains("unknown scheme name"), "{err}");
    assert!(err.contains("gruvbox-light"), "{err}");

    // Paths stay paths.
    let by_path =
        Scheme::resolve("tests/fixtures/solarized-dark.yaml", None).expect("path resolves");
    let by_name = Scheme::resolve("solarized-dark", None).expect("name resolves");
    assert_eq!(
        by_path.slot_hexes().unwrap(),
        by_name.slot_hexes().unwrap(),
        "path and collection name give the same slots"
    );
}
