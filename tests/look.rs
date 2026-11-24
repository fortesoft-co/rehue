//! Looks: saved grades layered under the config and the flags.

mod common;

use std::path::{Path, PathBuf};

use rehue::grade_scheme::SchemeConfig;
use rehue::grade_wal::WalConfig;
use rehue::look::{self, BUILTIN, Target};
use serde_json::{Value, json};

use common::{fixture, gradient_image};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rehue-look-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, text: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path.display().to_string()
}

fn wal(config: Value, flags: &[&str]) -> Result<WalConfig, String> {
    let flags: Vec<String> = flags.iter().map(|s| s.to_string()).collect();
    look::resolve::<WalConfig>("test", config, &flags, None, Target::Wal).map(|(c, _)| c)
}

#[test]
fn every_builtin_parses_and_says_what_it_is() {
    for (name, _) in BUILTIN {
        let l = look::find(name).unwrap();
        assert!(!l.description.is_empty(), "{name}");
        assert!(l.section(Target::Wal).is_some(), "{name} grades wal");
    }
}

#[test]
fn a_look_is_a_starting_point_the_config_beats() {
    let c = wal(json!({}), &["pop"]).unwrap();
    assert_eq!((c.harmonize, c.chroma), (1.0, 1.2));
    let c = wal(json!({ "chroma": 0.9 }), &["pop"]).unwrap();
    assert_eq!((c.harmonize, c.chroma), (1.0, 0.9));
}

#[test]
fn looks_stack_left_to_right() {
    let dir = scratch("stack");
    let a = write(&dir, "a.toml", "[wal]\nreach = 10\nharmonize = 0.2\n");
    let b = write(&dir, "b.toml", "[wal]\nreach = 20\n");
    let c = wal(json!({}), &[&format!("{a},{b}")]).unwrap();
    assert_eq!((c.reach, c.harmonize), (20.0, 0.2));
    let c = wal(json!({}), &[&b, &a]).unwrap();
    assert_eq!(c.reach, 10.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn roles_merge_per_role_and_key_across_short_and_full_names() {
    let (c, _) = look::resolve::<SchemeConfig>(
        "test",
        json!({ "roles": { "ac": { "cycle": 1 } } }),
        &["pop".to_string()],
        None,
        Target::Scheme,
    )
    .unwrap();
    let accents = &c.roles["accents"];
    assert_eq!(accents.chroma, Some(1.3), "from the look");
    assert_eq!(accents.cycle, Some(1), "from the config");
}

#[test]
fn a_palette_replaces_the_looks_palette_whole() {
    let c = wal(json!({ "palette": { "base0D": 1 } }), &["vivid"]).unwrap();
    let palette = c.palette.unwrap();
    assert_eq!(palette.len(), 1);
    assert_eq!(palette["base0D"], 1.0);
}

#[test]
fn the_flag_replaces_the_configs_look_and_the_config_beats_an_inherited_one() {
    let c = wal(json!({ "look": "soft" }), &["pop"]).unwrap();
    assert_eq!(c.chroma, 1.2, "-l pop, not the config's soft");
    let c = wal(json!({ "look": ["soft"] }), &[]).unwrap();
    assert_eq!(c.harmonize, 0.7);
    let inherited = Some(vec!["pop".to_string()]);
    let (c, _) =
        look::resolve::<WalConfig>("test", json!({}), &[], inherited.clone(), Target::Wal).unwrap();
    assert_eq!(c.chroma, 1.2);
    let (c, _) = look::resolve::<WalConfig>(
        "test",
        json!({ "look": "soft" }),
        &[],
        inherited,
        Target::Wal,
    )
    .unwrap();
    assert_eq!((c.harmonize, c.chroma), (0.7, 1.0));
}

#[test]
fn bad_looks_and_bad_keys_are_errors() {
    let dir = scratch("bad");
    let typo = write(&dir, "typo.toml", "[wal]\nharmonise = 1\n");
    let empty = write(&dir, "empty.toml", "description = \"nothing\"\n");
    let stray = write(&dir, "stray.toml", "colour = 1\n[wal]\n");
    for (flags, config, needle) in [
        (vec!["snap"], json!({}), "nothing for scheme"),
        (vec!["nope"], json!({}), "no look named 'nope'"),
        (vec!["../x"], json!({}), "can't read look"),
        (vec!["a b"], json!({}), "not a look name"),
        (vec![typo.as_str()], json!({}), "did you mean 'harmonize'"),
        (
            vec![empty.as_str()],
            json!({}),
            "none of the [wal], [scheme] and [icons]",
        ),
        (vec![stray.as_str()], json!({}), "colour"),
        (
            vec![],
            json!({ "reach": 30, "rech": 1 }),
            "unknown key 'rech'",
        ),
        (
            vec![],
            json!({ "roles": { "accents": { "chroma": 1 } }, "rols": {} }),
            "rols",
        ),
    ] {
        let flags: Vec<String> = flags.iter().map(|s| s.to_string()).collect();
        let err = look::resolve::<SchemeConfig>("test", config, &flags, None, Target::Scheme)
            .map(|_| ())
            .unwrap_err();
        assert!(err.contains(needle), "{flags:?}: {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn user_looks_live_in_the_config_dir_and_can_replace_builtins() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let home = scratch("home");
    let looks = home.join("rehue/looks");
    std::fs::create_dir_all(&looks).unwrap();
    write(
        &looks,
        "mine.toml",
        "description = \"my own\"\n[wal]\nchroma = 0.5\n",
    );
    write(
        &looks,
        "pop.toml",
        "description = \"my pop\"\n[wal]\nchroma = 1.5\n",
    );
    let listing = std::process::Command::new(bin)
        .args(["inspect", "--looks"])
        .env("XDG_CONFIG_HOME", &home)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&listing.stdout);
    assert!(listing.status.success(), "{text}");
    assert!(text.contains("* mine"), "{text}");
    assert!(text.contains("+ pop"), "{text}");
    assert!(text.contains("my pop"), "{text}");

    // a real run: the look under the flags, the summary naming both
    let wall = home.join("wall.png");
    gradient_image((24, 24)).save(&wall).unwrap();
    let run = |extra: &[&str]| {
        let out = std::process::Command::new(bin)
            .args(["wal", "-l", "mine"])
            .args(extra)
            .arg("-i")
            .arg(&wall)
            .arg("-s")
            .arg(fixture("solarized-dark.yaml"))
            .arg("-o")
            .arg(home.join("out"))
            .env("XDG_CONFIG_HOME", &home)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let text = run(&[]);
    assert!(text.contains("rehue wal: look mine"), "{text}");
    assert!(text.contains("chroma 0.50"), "{text}");
    assert!(
        run(&["--chroma", "0.8"]).contains("chroma 0.80"),
        "flags win"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_pipeline_takes_a_shared_look_and_per_step_looks() {
    let dir = scratch("pipeline");
    let pipeline = write(
        &dir,
        "rehue.toml",
        "wallpaper = \"wall.png\"\nscheme = \"nord\"\n\n[rehue]\nlook = \"pop\"\n\n\
         [rehue.scheme]\n\n[rehue.wal]\nlook = \"muted\"\nharmonize = 0.4\n",
    );
    let p = rehue::apply::Pipeline::load(Path::new(&pipeline)).unwrap();
    let scheme = p.rehue.scheme.unwrap();
    assert_eq!(scheme.roles["accents"].chroma, Some(1.3), "shared pop");
    let wal = p.rehue.wal.unwrap();
    assert_eq!(
        (wal.grade.harmonize, wal.grade.chroma),
        (0.4, 0.75),
        "own muted, table wins"
    );

    let typo = write(
        &dir,
        "typo.toml",
        "wallpaper = \"wall.png\"\nscheme = \"nord\"\n[rehue.wal]\nhramonize = 1\n",
    );
    let err = rehue::apply::Pipeline::load(Path::new(&typo)).unwrap_err();
    assert!(
        err.contains("[rehue.wal]") && err.contains("hramonize"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn looks_are_written_in_any_config_format() {
    let dir = scratch("formats");
    let toml = write(
        &dir,
        "dusk.toml",
        "description = \"evening\"\n[wal]\nharmonize = 1\nchroma = 0.8\n\n[scheme]\nroles.all.chroma = 0.85\n",
    );
    let yaml = write(
        &dir,
        "dusk.yaml",
        "description: evening\nwal:\n  harmonize: 1\n  chroma: 0.8\nscheme:\n  roles:\n    all:\n      chroma: 0.85\n",
    );
    let json = write(
        &dir,
        "dusk.json",
        r#"{ "description": "evening", "wal": { "harmonize": 1, "chroma": 0.8 },
             "scheme": { "roles": { "all": { "chroma": 0.85 } } } }"#,
    );
    let want = wal(json!({}), &[&toml]).unwrap();
    assert_eq!(want.chroma, 0.8);
    for other in [&yaml, &json] {
        assert_eq!(wal(json!({}), &[other]).unwrap(), want, "{other}");
        let look = look::find(other).unwrap();
        assert_eq!(
            (look.description.as_str(), look.targets().as_str()),
            ("evening", "wal, scheme, icons")
        );
    }
    for (file, text, needle) in [
        ("stray.yaml", "colour: 1\nwal: {}\n", "unknown key 'colour'"),
        ("flat.yaml", "wal: 1\n", "[wal] is a table"),
        ("look.ini", "[wal]\n", ".toml, .yaml or .json"),
    ] {
        let path = write(&dir, file, text);
        let err = look::find(&path).map(|_| ()).unwrap_err();
        assert!(err.contains(needle), "{file}: {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn user_looks_by_name_in_any_format_but_only_one_file_each() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let home = scratch("named");
    let looks = home.join("rehue/looks");
    std::fs::create_dir_all(&looks).unwrap();
    write(
        &looks,
        "calm.yaml",
        "description: calm\nwal:\n  chroma: 0.5\n",
    );
    write(
        &looks,
        "loud.json",
        r#"{ "description": "loud", "wal": { "chroma": 1.5 } }"#,
    );
    let inspect = |args: &[&str]| {
        std::process::Command::new(bin)
            .arg("inspect")
            .args(args)
            .env("XDG_CONFIG_HOME", &home)
            .output()
            .unwrap()
    };
    let out = inspect(&["--looks"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && text.contains("* calm") && text.contains("* loud"),
        "{text}"
    );
    let out = inspect(&["--looks", "calm"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("chroma: 0.5"));

    write(&looks, "calm.toml", "[wal]\nchroma = 0.6\n");
    let out = inspect(&["--looks", "calm"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && err.contains("more than one file"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&home);
}
