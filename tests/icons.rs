//! `rehue icons`: colour rewriting, theme layout, folders.

mod common;

use std::path::{Path, PathBuf};

use rehue::icons::{
    base64_decode, base64_encode, is_folder, parse_index, recolor_png, recolor_svg, rename_index,
};

use common::fixture;

fn invert(rgb: [u8; 3]) -> [u8; 3] {
    [255 - rgb[0], 255 - rgb[1], 255 - rgb[2]]
}

#[test]
fn every_colour_form_is_rewritten_and_nothing_else() {
    let svg = r##"<svg><style>.ColorScheme-Highlight { color:#3daee9; }</style>
<linearGradient id="g"><stop stop-color="#FFF" offset="0"/></linearGradient>
<path fill="#000000" stroke='#11223380' style="fill:#ff0000;opacity:.5"/>
<path fill="rgb(0, 0, 255)"/><path fill="rgba(255,255,0,0.5)"/>
<path fill="url(#g)"/><path fill="currentColor"/><use href="#bead12"/>
<text>#c0ffee in prose</text></svg>"##;
    let out = recolor_svg(svg, &mut invert);
    for want in [
        "color:#c25116;",
        r##"stop-color="#000000""##,
        r##"fill="#ffffff""##,
        "stroke='#eeddcc80'",
        "fill:#00ffff;opacity:.5",
        r#"fill="rgb(255, 255, 0)""#,
        r#"fill="rgba(0, 0, 255, 0.5)""#,
        r#"fill="url(#g)""#,
        r#"fill="currentColor""#,
        r##"href="#bead12""##,
        "#c0ffee in prose",
    ] {
        assert!(out.contains(want), "missing {want}:\n{out}");
    }
}

#[test]
fn embedded_pngs_are_recoloured_and_base64_round_trips() {
    for bytes in [&b""[..], b"a", b"ab", b"abc", b"any carnal pleasure."] {
        assert_eq!(base64_decode(&base64_encode(bytes)).unwrap(), bytes);
    }
    let mut img = image::RgbaImage::new(2, 1);
    img.put_pixel(0, 0, image::Rgba([10, 20, 30, 255]));
    img.put_pixel(1, 0, image::Rgba([10, 20, 30, 0]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let png = png.into_inner();

    let out = recolor_png(&png, &mut invert).unwrap();
    let back = image::load_from_memory(&out).unwrap().to_rgba8();
    assert_eq!(back.get_pixel(0, 0).0, [245, 235, 225, 255]);
    assert_eq!(
        back.get_pixel(1, 0).0[3],
        0,
        "transparent stays transparent"
    );

    let svg = format!(
        r##"<svg><image href="data:image/png;base64,{}"/><path fill="#000"/></svg>"##,
        base64_encode(&png)
    );
    let out = recolor_svg(&svg, &mut invert);
    let start = out.find("base64,").unwrap() + 7;
    let end = start + out[start..].find('"').unwrap();
    let inner = image::load_from_memory(&base64_decode(&out[start..end]).unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(inner.get_pixel(0, 0).0, [245, 235, 225, 255]);
    assert!(out.contains(r##"fill="#ffffff""##));
}

#[test]
fn the_index_names_folders_by_context() {
    let index = parse_index(
        "[Icon Theme]\nName=Test\nInherits=Base,hicolor\nDirectories=48/places,48/apps,48/mimetypes\n\n\
         [48/places]\nContext=Places\nSize=48\n\n[48/apps]\nContext=Applications\n\n\
         [48/mimetypes]\nContext=MimeTypes\n",
    );
    assert_eq!(index.name.as_deref(), Some("Test"));
    assert_eq!(index.inherits, ["Base", "hicolor"]);
    let folder = |p: &str| is_folder(Path::new(p), &index);
    assert!(folder("48/places/folder.svg"));
    assert!(folder("48/places/folder-music.svg"));
    assert!(folder("48/places/user-home.svg"));
    assert!(folder("48/mimetypes/inode-directory.svg"));
    assert!(!folder("48/places/folder-symbolic.svg"));
    assert!(!folder("48/places/user-trash.svg"));
    assert!(!folder("48/places/start-here.svg"));
    assert!(
        !folder("48/apps/folder.svg"),
        "an app named folder is an app"
    );
    assert!(
        !folder("16/places/folder.svg"),
        "a directory the index doesn't list"
    );

    let renamed = rename_index(
        "[Icon Theme]\nName=Test\nComment=x\n[48/places]\nName=no\n",
        "New",
    );
    assert_eq!(
        renamed,
        "[Icon Theme]\nName=New\nComment=x\n[48/places]\nName=no\n"
    );
}

fn icon(colour: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><rect width="4" height="4" fill="{colour}"/></svg>"#
    )
}

/// A small theme: real folders, a folder name linked to a folder-named
/// file, a folder name linked to an action icon, an app, an internal dir
/// link, a link leaving the theme, and a stale cache.
fn theme(dir: &Path) -> PathBuf {
    let root = dir.join("src/Test");
    let outside = dir.join("outside/48");
    for d in ["48/places", "48/apps", "48/actions"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::create_dir_all(outside.join("devices")).unwrap();
    std::fs::write(
        root.join("index.theme"),
        "[Icon Theme]\nName=Test\nInherits=hicolor\nDirectories=48/places,48/apps,48/actions,64/devices\n\n\
         [48/places]\nContext=Places\n[48/apps]\nContext=Applications\n[48/actions]\nContext=Actions\n\
         [64/devices]\nContext=Devices\n",
    )
    .unwrap();
    std::fs::write(root.join("48/places/folder.svg"), icon("#5294e2")).unwrap();
    std::fs::write(
        root.join("48/places/default-folder-music.svg"),
        icon("#5294e2"),
    )
    .unwrap();
    std::fs::write(root.join("48/actions/document-print.svg"), icon("#5294e2")).unwrap();
    std::fs::write(root.join("48/apps/firefox.svg"), icon("#e66000")).unwrap();
    std::fs::write(outside.join("devices/drive.svg"), icon("#5294e2")).unwrap();
    std::fs::write(root.join("icon-theme.cache"), b"stale").unwrap();
    let link = |target: &str, at: &str| std::os::unix::fs::symlink(target, root.join(at)).unwrap();
    link("default-folder-music.svg", "48/places/folder-music.svg");
    link(
        "../actions/document-print.svg",
        "48/places/folder-print.svg",
    );
    link("48", "48@2");
    std::os::unix::fs::symlink(&outside, root.join("64")).unwrap();
    root
}

fn run(args: &[&str], env: &[(&str, &Path)]) -> String {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_rehue"));
    cmd.arg("icons").args(args).env("PATH", "/nonexistent");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_theme_keeps_its_layout_and_only_folders_take_the_folder_colour() {
    let dir = std::env::temp_dir().join(format!("rehue-icons-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = theme(&dir);
    let scheme = fixture("solarized-dark.yaml");
    let scheme = scheme.to_str().unwrap();
    let plain = dir.join("plain");
    let purple = dir.join("purple");
    run(
        &[
            "-i",
            root.to_str().unwrap(),
            "-s",
            scheme,
            "-o",
            plain.to_str().unwrap(),
        ],
        &[],
    );
    let log = run(
        &[
            "-i",
            root.to_str().unwrap(),
            "-s",
            scheme,
            "-o",
            purple.to_str().unwrap(),
            "--name",
            "Test Purple",
            "--folders",
            "base0E",
        ],
        &[],
    );
    assert!(log.contains("2 folder icons"), "{log}");

    let a = plain.join("Test-rehue");
    let b = purple.join("Test-Purple");
    let read = |root: &Path, rel: &str| std::fs::read_to_string(root.join(rel)).unwrap();
    // folders (and the folder-named file a folder link points at) differ
    for rel in ["48/places/folder.svg", "48/places/default-folder-music.svg"] {
        assert_ne!(read(&a, rel), read(&b, rel), "{rel}");
    }
    // the action icon a folder name links to, apps and devices don't
    for rel in [
        "48/actions/document-print.svg",
        "48/apps/firefox.svg",
        "64/devices/drive.svg",
    ] {
        assert_eq!(read(&a, rel), read(&b, rel), "{rel}");
    }
    // everything was recoloured
    assert!(!read(&a, "48/apps/firefox.svg").contains("#e66000"));
    // links inside the theme stay links; the one leaving it was copied in
    let link = |rel: &str| std::fs::read_link(b.join(rel)).ok();
    assert_eq!(
        link("48/places/folder-music.svg"),
        Some("default-folder-music.svg".into())
    );
    assert_eq!(link("48@2"), Some("48".into()));
    assert!(link("64").is_none() && b.join("64/devices/drive.svg").is_file());
    // the new name, the stale cache gone, the preview beside the theme
    assert!(read(&b, "index.theme").contains("Name=Test Purple\n"));
    assert!(!b.join("icon-theme.cache").exists());
    assert!(purple.join("preview.png").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn themes_are_found_by_name_files_work_alone_and_dry_runs_write_nothing() {
    let dir = std::env::temp_dir().join(format!("rehue-icons-find-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = theme(&dir);
    let scheme = fixture("solarized-dark.yaml");
    let scheme = scheme.to_str().unwrap();
    // a data dir whose icons/ holds the theme
    let share = dir.join("share");
    std::fs::create_dir_all(&share).unwrap();
    std::os::unix::fs::symlink(root.parent().unwrap(), share.join("icons")).unwrap();
    let out = dir.join("out");
    let log = run(
        &[
            "-i",
            "Test",
            "-s",
            scheme,
            "-o",
            out.to_str().unwrap(),
            "-n",
        ],
        &[
            ("XDG_DATA_DIRS", &share),
            ("XDG_DATA_HOME", &dir.join("none")),
            ("HOME", &dir),
        ],
    );
    assert!(log.contains("dry run"), "{log}");
    assert!(!out.exists(), "a dry run writes nothing");

    let one = dir.join("one");
    let svg = dir.join("firefox.svg");
    std::fs::write(&svg, icon("#e66000")).unwrap();
    run(
        &[
            "-i",
            svg.to_str().unwrap(),
            "-s",
            scheme,
            "-o",
            one.to_str().unwrap(),
        ],
        &[],
    );
    let out = std::fs::read_to_string(one.join("firefox.svg")).unwrap();
    assert!(!out.contains("#e66000"));
    assert!(one.join("preview.png").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn looks_lend_icons_their_wal_section_and_icons_start_harmonized() {
    use rehue::icons::IconsConfig;
    use rehue::look::{self, Target};
    let (c, _) =
        look::resolve::<IconsConfig>("t", serde_json::json!({}), &[], None, Target::Icons).unwrap();
    assert_eq!(c.grade.harmonize, 1.0, "icons default to harmonize 1");
    let (c, _) = look::resolve::<IconsConfig>(
        "t",
        serde_json::json!({ "folders": { "base0E": 1 } }),
        &["snap".to_string()],
        None,
        Target::Icons,
    )
    .unwrap();
    assert_eq!(
        c.grade.mode,
        Some(rehue::grade_wal::Mode::Snap),
        "snap's [wal] section"
    );
    assert_eq!(c.folders.unwrap()["base0E"], 1.0);
}

#[test]
fn dithering_draws_sized_icons_to_png_and_follows_the_links() {
    let dir = std::env::temp_dir().join(format!("rehue-icons-dither-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = dir.join("src/Sized");
    for d in ["48/apps", "48/places", "symbolic/apps", "scalable/apps"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(
        root.join("index.theme"),
        "[Icon Theme]\nName=Sized\nDirectories=48/apps,48/places,symbolic/apps,scalable/apps\n\n\
         [48/apps]\nContext=Applications\nSize=48\n[48/places]\nContext=Places\nSize=24\nScale=2\n\
         [symbolic/apps]\nContext=Applications\nSize=16\n[scalable/apps]\nContext=Applications\n",
    )
    .unwrap();
    let gradient = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><defs><linearGradient id="g"><stop offset="0" stop-color="#e66000"/><stop offset="1" stop-color="#2060c0"/></linearGradient></defs><circle cx="2" cy="2" r="1.8" fill="url(#g)"/></svg>"##;
    std::fs::write(root.join("48/apps/firefox.svg"), gradient).unwrap();
    std::fs::write(
        root.join("48/places/default-folder-music.svg"),
        icon("#5294e2"),
    )
    .unwrap();
    std::fs::write(
        root.join("symbolic/apps/firefox-symbolic.svg"),
        icon("#bebebe"),
    )
    .unwrap();
    std::fs::write(root.join("scalable/apps/firefox.svg"), gradient).unwrap();
    std::os::unix::fs::symlink(
        "default-folder-music.svg",
        root.join("48/places/folder-music.svg"),
    )
    .unwrap();
    std::os::unix::fs::symlink("firefox.svg", root.join("48/apps/web-browser.svg")).unwrap();

    let out = dir.join("out");
    let scheme = fixture("solarized-dark.yaml");
    let log = run(
        &[
            "-i",
            root.to_str().unwrap(),
            "-s",
            scheme.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--dither",
        ],
        &[],
    );
    assert!(
        log.contains("best effort") && log.contains("2 icons dithered"),
        "{log}"
    );
    let t = out.join("Sized-rehue");

    // sized icons become pngs at their directory's size, scheme colors only
    let png = image::open(t.join("48/apps/firefox.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(png.dimensions(), (48, 48));
    let folder = image::open(t.join("48/places/default-folder-music.png")).unwrap();
    assert_eq!(folder.width(), 48, "Size 24 x Scale 2");
    assert!(!t.join("48/apps/firefox.svg").exists());
    let scheme_colors: std::collections::BTreeSet<[u8; 3]> =
        rehue::scheme::Scheme::parse_file(&scheme)
            .unwrap()
            .slot_hexes()
            .unwrap()
            .iter()
            .map(|h| {
                let v = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap();
                [(v >> 16) as u8, (v >> 8) as u8, v as u8]
            })
            .collect();
    let used: std::collections::BTreeSet<[u8; 3]> = png
        .pixels()
        .filter(|p| p.0[3] > 0)
        .map(|p| [p.0[0], p.0[1], p.0[2]])
        .collect();
    assert!(used.is_subset(&scheme_colors), "{used:?}");
    assert!(used.len() > 1, "the gradient is drawn with several colors");
    assert!(
        png.pixels().any(|p| p.0[3] == 0),
        "transparency is kept around the circle"
    );

    // links follow their targets to the new name
    let link = |rel: &str| std::fs::read_link(t.join(rel)).ok();
    assert_eq!(
        link("48/places/folder-music.png"),
        Some("default-folder-music.png".into())
    );
    assert_eq!(link("48/apps/web-browser.png"), Some("firefox.png".into()));
    assert!(
        t.join("48/apps/web-browser.png").is_file(),
        "the link resolves"
    );

    // symbolic and unknown-size icons stay recolored svgs
    assert!(t.join("symbolic/apps/firefox-symbolic.svg").is_file());
    assert!(t.join("scalable/apps/firefox.svg").is_file());
    assert!(!t.join("scalable/apps/firefox.png").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
