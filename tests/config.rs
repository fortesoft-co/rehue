//! Config files: JSON, TOML and YAML, picked by extension.

mod common;

use std::path::{Path, PathBuf};

use rehue::config::{from_yaml, read_file};
use rehue::grade_wal::WalConfig;
use rehue::look::{self, Target};
use serde_json::json;

use common::{fixture, gradient_image};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rehue-config-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn wal(path: &Path) -> WalConfig {
    let value = read_file(Some(path)).unwrap();
    look::resolve::<WalConfig>("test", value, &[], None, Target::Wal)
        .unwrap()
        .0
}

#[test]
fn the_same_settings_read_the_same_in_every_format() {
    let dir = scratch("formats");
    let json = write(
        &dir,
        "wal.json",
        r#"{ "harmonize": 1, "neutrals": "mix", "palette": { "ac": 1, "base0D": 2 },
             "roles": { "accents": { "chroma": 1.3, "assign": [0, 1, 2, 3] } } }"#,
    );
    let toml = write(
        &dir,
        "wal.toml",
        "harmonize = 1\nneutrals = \"mix\"\npalette = { ac = 1, base0D = 2 }\n\n\
         [roles.accents]\nchroma = 1.3\nassign = [0, 1, 2, 3]\n",
    );
    let yaml = write(
        &dir,
        "wal.yaml",
        "harmonize: 1\nneutrals: mix\npalette: { ac: 1, base0D: 2 }\nroles:\n  accents:\n    chroma: 1.3\n    assign: [0, 1, 2, 3]\n",
    );
    let yml = write(&dir, "wal.YML", &std::fs::read_to_string(&yaml).unwrap());
    let want = wal(&json);
    assert_eq!(want.harmonize, 1.0);
    assert_eq!(want.roles["accents"].chroma, Some(1.3));
    for other in [&toml, &yaml, &yml] {
        assert_eq!(wal(other), want, "{}", other.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn yaml_follows_1_2_and_stays_one_document() {
    // YAML 1.2: `no` and `on` are strings, not booleans
    assert_eq!(
        from_yaml("a: no\nb: on\nc: true\nd: 0.5\ne: ~\n").unwrap(),
        json!({ "a": "no", "b": "on", "c": true, "d": 0.5, "e": null })
    );
    assert_eq!(from_yaml("").unwrap(), json!({}));
    assert!(
        from_yaml("a: 1\n---\nb: 2\n")
            .unwrap_err()
            .contains("one YAML document")
    );
    assert!(from_yaml("a: [1\n").is_err());
}

#[test]
fn bad_files_say_what_is_wrong() {
    let dir = scratch("bad");
    let ini = write(&dir, "wal.ini", "harmonize = 1\n");
    let err = read_file(Some(&ini)).unwrap_err();
    assert!(err.contains(".toml, .yaml or .json"), "{err}");
    let list = write(&dir, "wal.yaml", "- 1\n- 2\n");
    let err = read_file(Some(&list)).unwrap_err();
    assert!(err.contains("expected a table"), "{err}");
    let broken = write(&dir, "wal.toml", "harmonize = \n");
    let err = read_file(Some(&broken)).unwrap_err();
    assert!(
        err.contains("bad config") && err.contains("wal.toml"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_command_reads_yaml_and_toml_and_catches_typos() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let dir = scratch("cli");
    let wall = dir.join("wall.png");
    gradient_image((24, 24)).save(&wall).unwrap();
    let scheme = fixture("solarized-dark.yaml");
    let run = |args: &[&str], config: &Path| {
        std::process::Command::new(bin)
            .args(args)
            .arg("-c")
            .arg(config)
            .arg("-o")
            .arg(dir.join("out"))
            .output()
            .unwrap()
    };
    let wal_args = [
        "wal",
        "-i",
        wall.to_str().unwrap(),
        "-s",
        scheme.to_str().unwrap(),
    ];
    let scheme_args = [
        "scheme",
        "-i",
        scheme.to_str().unwrap(),
        "-w",
        wall.to_str().unwrap(),
    ];
    let inspect_args = ["inspect", "-w", wall.to_str().unwrap()];

    let good = write(&dir, "wal.yaml", "harmonize: 0.5\nchroma: 0.8\n");
    let out = run(&wal_args, &good);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("chroma 0.80"));

    let roles = write(&dir, "scheme.toml", "[roles.accents]\nchroma = 1.2\n");
    let out = run(&scheme_args, &roles);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let extraction = write(&dir, "extract.toml", "max-families = 2\n");
    let out = run(&inspect_args, &extraction);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let typo = write(&dir, "typo.yaml", "harmonise: 1\n");
    let out = run(&wal_args, &typo);
    assert!(String::from_utf8_lossy(&out.stderr).contains("did you mean 'harmonize'"));
    let typo = write(&dir, "typo.toml", "max-familes = 2\n");
    let out = run(&inspect_args, &typo);
    assert!(
        !out.status.success() && String::from_utf8_lossy(&out.stderr).contains("max-familes"),
        "inspect checks its keys too"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_run_setting_has_a_config_key_and_flags_win() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let dir = scratch("run");
    let wall = dir.join("wall.png");
    gradient_image((24, 24)).save(&wall).unwrap();
    std::fs::create_dir_all(dir.join("schemes")).unwrap();
    std::fs::copy(
        fixture("solarized-dark.yaml"),
        dir.join("schemes/mine.yaml"),
    )
    .unwrap();
    // a scheme-dir relative to the config, a LUT and grain
    let config = write(
        &dir,
        "wal.toml",
        "harmonize = 1\nscheme-dir = \"schemes\"\nlut = \"cube\"\nlut-level = 2\ngrain = 2\n",
    );
    let wal = |extra: &[&str], out: &str| {
        let run = std::process::Command::new(bin)
            .args(["wal", "-i", wall.to_str().unwrap(), "-s", "mine", "-c"])
            .arg(&config)
            .args(extra)
            .arg("-o")
            .arg(dir.join(out))
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(out).join("report.json")).unwrap())
                .unwrap();
        report
    };
    let report = wal(&[], "a");
    assert_eq!(report["grain"], 2.0);
    assert!(dir.join("a/lut.cube").is_file());
    // the flags win over the config, key by key
    let report = wal(&["--grain", "3", "--lut", "hald"], "b");
    assert_eq!(report["grain"], 3.0);
    assert!(dir.join("b/lut.png").is_file() && !dir.join("b/lut.cube").exists());

    // the video settings are keys too (checked here on their own: a
    // still refuses them)
    let video = rehue::config::from_value::<rehue::grade_wal::WalCommandConfig>(json!({
        "deband": true,
        "grain": true,
        "ffmpeg": "bin/ffmpeg",
        "ffmpeg-args": "-crf 20",
        "loop": { "mode": "fade", "min": "50%", "seam": 0.5 },
    }))
    .unwrap();
    assert!(video.deband);
    assert_eq!(video.grain, Some(rehue::grade_wal::DEFAULT_GRAIN));
    assert_eq!(video.ffmpeg_args.as_deref(), Some("-crf 20"));
    let looping = video.looping.unwrap();
    assert_eq!(
        looping,
        rehue::looping::LoopOptions::parse("fade,min=50%,seam=0.5").unwrap()
    );
    for bad in [json!({ "loop": { "mod": "fade" } }), json!({ "loop": 3 })] {
        assert!(rehue::config::from_value::<rehue::grade_wal::WalCommandConfig>(bad).is_err());
    }
    assert_eq!(
        rehue::config::from_value::<rehue::grade_wal::WalCommandConfig>(json!({ "loop": "ask" }))
            .unwrap()
            .looping
            .unwrap()
            .mode,
        rehue::looping::Mode::Ask
    );

    // enhance reads upscale from its config
    let up = write(&dir, "enhance.toml", "upscale = 2\n");
    let run = std::process::Command::new(bin)
        .args(["enhance", "-i", wall.to_str().unwrap(), "-c"])
        .arg(&up)
        .arg("-o")
        .arg(dir.join("up"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let scaled = image::open(dir.join("up/wall.png")).unwrap();
    assert_eq!((scaled.width(), scaled.height()), (48, 48));
    let run = std::process::Command::new(bin)
        .args(["enhance", "-i", wall.to_str().unwrap(), "-o"])
        .arg(dir.join("up2"))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&run.stderr).contains("needs --upscale"));
    let _ = std::fs::remove_dir_all(&dir);
}
