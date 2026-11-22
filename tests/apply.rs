//! `rehue apply`: pipeline files, the tinty handoff, the wallpaper command.
//!
//! `rehue apply`: pipeline files, the tinty handoff and the wallpaper
//! command.  tinty is faked by a shell script on PATH that logs its
//! arguments, so these run without network or a real tinty.
#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::gradient_image;
use rehue::apply::{Pipeline, with_slug};

const FAKE_TINTY: &str = r#"#!/bin/sh
echo "$@" >> "$FAKE_TINTY_LOG"
case "$*" in
  *--version*) echo "tinty ${FAKE_TINTY_VERSION:-0.35.0}" ;;
  *"config --data-dir-path"*) echo "$FAKE_TINTY_DATA" ;;
  *apply*) [ -n "$FAKE_TINTY_MISSING" ] && echo "Theme does not exists for alacritty in somewhere" ;;
esac
exit 0
"#;

/// A scratch directory with a wallpaper, a fake tinty and a log path.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("rehue-apply-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).unwrap();
        gradient_image((48, 32))
            .save(root.join("wall.png"))
            .unwrap();
        let tinty = root.join("bin/tinty");
        std::fs::write(&tinty, FAKE_TINTY).unwrap();
        std::fs::set_permissions(&tinty, std::fs::Permissions::from_mode(0o755)).unwrap();
        Sandbox { root }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn write_pipeline(&self, body: &str) -> PathBuf {
        let path = self.path("pipeline.toml");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn command(&self, pipeline: &Path, extra: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rehue"));
        command
            .arg("apply")
            .arg("--pipeline")
            .arg(pipeline)
            .args(extra);
        let path = format!(
            "{}:{}",
            self.path("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        command
            .env("PATH", path)
            .env("FAKE_TINTY_LOG", self.path("tinty.log"))
            .env("FAKE_TINTY_DATA", self.path("tinty-data"));
        command
    }

    fn tinty_log(&self) -> String {
        std::fs::read_to_string(self.path("tinty.log")).unwrap_or_default()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const FULL: &str = r#"
wallpaper = "wall.png"
scheme = "gruvbox-light"
out = "out"

[rehue.scheme]

[rehue.wal]
harmonize = 1.0

[apply]
tinty-config = "tinty.toml"
wallpaper-command = "printf '%s' {wallpaper} > set-wallpaper.txt"
"#;

#[test]
fn full_pipeline_maps_installs_applies_and_sets_the_wallpaper() {
    let sb = Sandbox::new("full");
    let pipeline = sb.write_pipeline(FULL);
    let output = sb.command(&pipeline, &[]).output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));

    // both steps wrote their outputs under `out`
    let scheme = sb.path("out/scheme/scheme.yaml");
    let wallpaper = sb.path("out/wallpaper/wallpaper.png");
    assert!(scheme.is_file() && wallpaper.is_file());

    // tinty got the mapped scheme, slug pinned to the custom-scheme name
    let installed =
        std::fs::read_to_string(sb.path("tinty-data/custom-schemes/base16/rehue.yaml")).unwrap();
    let mapped = std::fs::read_to_string(&scheme).unwrap();
    assert!(installed.contains("slug: \"rehue\""), "{installed}");
    assert_eq!(installed.replace("slug: \"rehue\"\n", ""), mapped);

    // --config passes through, and apply targets base16-<name>
    let log = sb.tinty_log();
    let config = sb.path("tinty.toml");
    assert!(
        log.contains(&format!("--config {} apply base16-rehue", config.display())),
        "{log}"
    );

    // the wallpaper command saw the repainted image
    let set = std::fs::read_to_string(sb.path("set-wallpaper.txt")).unwrap();
    assert_eq!(Path::new(&set), wallpaper);
}

#[test]
fn dry_run_writes_nothing_and_runs_nothing() {
    let sb = Sandbox::new("dry");
    let pipeline = sb.write_pipeline(FULL);
    let output = sb.command(&pipeline, &["--dry-run"]).output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("would run: tinty --config"), "{stdout}");
    assert!(!sb.path("out").exists(), "dry run must not create out");
    assert!(sb.tinty_log().is_empty(), "dry run must not call tinty");
    assert!(!sb.path("set-wallpaper.txt").exists());
}

#[test]
fn wal_only_applies_the_reference_scheme() {
    let sb = Sandbox::new("walonly");
    let pipeline = sb.write_pipeline(
        r#"
wallpaper = "wall.png"
scheme = "gruvbox-light"
out = "out"

[rehue.wal]
harmonize = 1.0

[apply]
tinty = false
"#,
    );
    let output = sb.command(&pipeline, &[]).output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let written = std::fs::read_to_string(sb.path("out/scheme/scheme.yaml")).unwrap();
    assert_eq!(
        written,
        rehue::scheme::collection_text("gruvbox-light").unwrap()
    );
    assert!(sb.path("out/wallpaper/wallpaper.png").is_file());
    assert!(sb.tinty_log().is_empty(), "tinty = false skips tinty");
}

#[test]
fn old_tinty_is_refused() {
    let sb = Sandbox::new("old");
    let pipeline = sb.write_pipeline(FULL);
    let output = sb
        .command(&pipeline, &[])
        .env("FAKE_TINTY_VERSION", "0.34.1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("needs tinty 0.35"),
        "{}",
        stderr(&output)
    );
    assert!(!sb.path("set-wallpaper.txt").exists());
}

#[test]
fn a_missing_theme_fails_even_though_tinty_exits_zero() {
    let sb = Sandbox::new("missing");
    let pipeline = sb.write_pipeline(FULL);
    let output = sb
        .command(&pipeline, &[])
        .env("FAKE_TINTY_MISSING", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("got no theme"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn tinty_missing_from_path_is_explained() {
    let sb = Sandbox::new("nopath");
    let pipeline = sb.write_pipeline(FULL);
    let output = sb
        .command(&pipeline, &[])
        .env("PATH", sb.path("empty"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("tinty not found"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn pipeline_load_resolves_paths_and_rejects_bad_files() {
    let sb = Sandbox::new("load");
    let pipeline = Pipeline::load(&sb.write_pipeline(FULL)).unwrap();
    assert_eq!(pipeline.wallpaper, sb.path("wall.png"));
    assert_eq!(pipeline.out_dir(), sb.path("out"));
    assert_eq!(pipeline.apply.tinty_name, "rehue");
    assert_eq!(pipeline.rehue.wal.as_ref().unwrap().harmonize, 1.0);

    let no_steps = sb.write_pipeline("wallpaper = \"wall.png\"\nscheme = \"nord\"\n");
    let err = Pipeline::load(&no_steps).unwrap_err();
    assert!(err.contains("no steps"), "{err}");

    let typo = sb.write_pipeline(
        "wallpaper = \"wall.png\"\nscheme = \"nord\"\nwalpaper = 1\n[rehue.scheme]\n",
    );
    assert!(Pipeline::load(&typo).is_err(), "unknown keys are rejected");

    let bad_name = sb.write_pipeline(
        "wallpaper = \"wall.png\"\nscheme = \"nord\"\n[rehue.scheme]\n[apply]\ntinty-name = \"../x\"\n",
    );
    let err = Pipeline::load(&bad_name).unwrap_err();
    assert!(err.contains("tinty-name"), "{err}");
}

#[test]
fn with_slug_pins_and_replaces() {
    let pinned = with_slug("system: \"base16\"\nname: \"X\"\npalette:\n", "rehue");
    assert_eq!(
        pinned,
        "system: \"base16\"\nname: \"X\"\nslug: \"rehue\"\npalette:\n"
    );
    let replaced = with_slug("name: \"X\"\nslug: \"old\"\n", "rehue");
    assert_eq!(replaced, "name: \"X\"\nslug: \"rehue\"\n");
}
