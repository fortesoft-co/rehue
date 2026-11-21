//! Cross-command CLI behaviour (the shared flags).

mod common;

use std::path::Path;

use common::{fixture, gradient_image};

#[test]
fn dry_run_writes_nothing() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let wall = std::env::temp_dir().join(format!("rehue-dry-wall-{}.png", std::process::id()));
    gradient_image((32, 32)).save(&wall).expect("wall writes");
    let scheme = fixture("solarized-dark.yaml");

    // the short flags: -i is the command's subject, -s / -w the other
    // input, -o / -n out and dry run
    let run = |command: &[&str], out: &Path| {
        let _ = std::fs::remove_dir_all(out);
        let (subject, other) = if command[0] == "wal" {
            (&wall, scheme.as_os_str())
        } else {
            (&scheme, wall.as_os_str())
        };
        let other_flag = if command[0] == "wal" { "-s" } else { "-w" };
        let status = std::process::Command::new(bin)
            .args(command)
            .arg("-i")
            .arg(subject)
            .arg(other_flag)
            .arg(other)
            .arg("-o")
            .arg(out)
            .arg("-n")
            .status()
            .expect("rehue binary runs");
        assert!(status.success(), "dry run {command:?} failed");
        assert!(!out.exists(), "dry run must not create --out");
    };
    run(
        &["wal"],
        &std::env::temp_dir().join(format!("rehue-dry-wal-{}", std::process::id())),
    );
    run(
        &["scheme"],
        &std::env::temp_dir().join(format!("rehue-dry-scheme-{}", std::process::id())),
    );

    let _ = std::fs::remove_file(&wall);
}

#[test]
fn dry_run_writes_nothing_even_without_out() {
    // --dry-run with no --out at all: neither the default `rehue-out`
    // nor anything else may appear in the working directory.
    let bin = env!("CARGO_BIN_EXE_rehue");
    let workdir = std::env::temp_dir().join(format!("rehue-cwd-{}", std::process::id()));
    std::fs::create_dir_all(&workdir).expect("cwd");
    let wall = workdir.join("wall.png");
    gradient_image((32, 32)).save(&wall).expect("wall writes");
    let scheme = fixture("solarized-dark.yaml");

    let status = std::process::Command::new(bin)
        .args(["wal", "--input"])
        .arg(&wall)
        .args(["--scheme"])
        .arg(&scheme)
        .arg("--dry-run")
        .current_dir(&workdir)
        .status()
        .expect("rehue binary runs");
    assert!(status.success());
    assert!(
        !workdir.join("rehue-out").exists(),
        "dry run must not fall back to the default dir either"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn absent_out_defaults_to_rehue_out() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let workdir = std::env::temp_dir().join(format!("rehue-cwd2-{}", std::process::id()));
    std::fs::create_dir_all(&workdir).expect("cwd");
    let wall = workdir.join("wall.png");
    gradient_image((32, 32)).save(&wall).expect("wall writes");
    let scheme = fixture("solarized-dark.yaml");

    let status = std::process::Command::new(bin)
        .args(["wallpaper", "--input"])
        .arg(&wall)
        .args(["--scheme"])
        .arg(&scheme)
        .current_dir(&workdir)
        .status()
        .expect("rehue binary runs");
    assert!(status.success());
    assert!(workdir.join("rehue-out/wallpaper.png").exists());

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn role_dials_accept_negative_values() {
    // The help text's own examples (`--lightness surfaces -0.02`,
    // `--lightness bg -0.05`) must parse: a negative value is a value, not a flag.
    let bin = env!("CARGO_BIN_EXE_rehue");
    let wall = std::env::temp_dir().join(format!("rehue-neg-wall-{}.png", std::process::id()));
    gradient_image((32, 32)).save(&wall).expect("wall writes");
    let scheme = fixture("solarized-dark.yaml");

    for command in ["scheme", "wal"] {
        for dial in [
            &["--lightness", "-0.05"][..],
            &["--lightness", "sf", "-0.02"][..],
            &["--cycle", "accents", "-1"][..],
        ] {
            let (subject, other_flag, other) = if command == "wal" {
                (wall.as_os_str(), "-s", scheme.as_os_str())
            } else {
                (scheme.as_os_str(), "-w", wall.as_os_str())
            };
            let output = std::process::Command::new(bin)
                .arg(command)
                .arg("-i")
                .arg(subject)
                .arg(other_flag)
                .arg(other)
                .args(dial)
                .arg("--dry-run")
                .output()
                .expect("rehue binary runs");
            assert!(
                output.status.success(),
                "{command} {dial:?} rejected: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    let _ = std::fs::remove_file(&wall);
}
