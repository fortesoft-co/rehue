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

fn rehue(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_rehue"))
        .args(args)
        .output()
        .expect("rehue binary runs")
}

/// Scheme names in an `inspect --schemes` listing, in printed order.
fn listed(stdout: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .skip(1)
        .map(|l| l[2..].split_whitespace().next().unwrap().to_string())
        .collect()
}

#[test]
fn inspect_schemes_lists_every_scheme_sorted_with_swatches() {
    let out = rehue(&["inspect", "--schemes"]);
    assert!(out.status.success());
    let names = listed(&out.stdout);
    let embedded: Vec<String> = rehue::scheme::collection_names()
        .map(String::from)
        .collect();
    assert_eq!(names, embedded, "every embedded scheme, sorted");
    assert_eq!(names[0], "0x96f", "the lexicographic first name");
    let text = String::from_utf8_lossy(&out.stdout);
    let row = text.lines().find(|l| l.contains("gruvbox-light ")).unwrap();
    // gruvbox-light base00 (#fbf1c7) as a truecolor swatch, 16 per row
    assert!(row.contains("\x1b[38;2;251;241;199m██"), "{row}");
    assert_eq!(row.matches("██").count(), 16);
}

#[test]
fn inspect_schemes_filters_case_insensitively() {
    let out = rehue(&["inspect", "--schemes", "GRUVBOX-LIGHT"]);
    assert!(out.status.success());
    let names = listed(&out.stdout);
    assert!(names.contains(&"gruvbox-light".to_string()));
    assert!(
        names.iter().all(|n| n.contains("gruvbox-light")),
        "{names:?}"
    );

    let none = rehue(&["inspect", "--schemes", "no-such-scheme-anywhere"]);
    assert!(!none.status.success());
    assert!(String::from_utf8_lossy(&none.stderr).contains("no scheme name contains"));
}

#[test]
fn inspect_schemes_includes_and_marks_scheme_dir() {
    let dir = std::env::temp_dir().join(format!("rehue-schemes-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let text = std::fs::read_to_string(fixture("solarized-dark.yaml")).unwrap();
    std::fs::write(dir.join("zz-mine.yaml"), &text).unwrap();
    std::fs::write(dir.join("nord.yaml"), &text).unwrap(); // shadows the embedded nord
    std::fs::write(dir.join("notes.txt"), "not a scheme").unwrap();

    let out = rehue(&[
        "inspect",
        "--schemes",
        "--scheme-dir",
        dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout
            .lines()
            .next()
            .unwrap()
            .contains("2 from --scheme-dir"),
        "{stdout}"
    );
    let marked: Vec<&str> = stdout.lines().filter(|l| l.starts_with('*')).collect();
    assert_eq!(marked.len(), 2, "{stdout}");
    assert_eq!(
        stdout
            .lines()
            .filter(|l| l[2..].starts_with("nord "))
            .count(),
        1,
        "the custom nord replaces the embedded one"
    );
    assert!(!stdout.contains("notes"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inspect_schemes_conflicts_with_other_modes() {
    let out = rehue(&["inspect", "--schemes", "--scheme", "nord"]);
    assert!(!out.status.success());
    let none = rehue(&["inspect"]);
    assert!(!none.status.success());
    assert!(String::from_utf8_lossy(&none.stderr).contains("--schemes"));
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

#[test]
fn every_embedded_scheme_parses_with_16_slots() {
    // The snapshot gate: a malformed upstream scheme fails here, at
    // update time (scripts/update-schemes.sh runs this suite), instead of
    // in a user's run.
    let mut count = 0;
    for name in rehue::scheme::collection_names() {
        let text = rehue::scheme::collection_text(name).expect("listed name resolves");
        let scheme = Scheme::parse_text(text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let hexes = scheme
            .slot_hexes()
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(hexes.len(), 16, "{name}");
        count += 1;
    }
    assert!(count > 300, "collection looks truncated: {count} schemes");
}
