//! Video wallpapers through ffmpeg (skipped when ffmpeg isn't installed).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::fixture;
use rehue::video::is_video;

/// ffmpeg and ffprobe, or `None` (the test then has nothing to drive).
fn tools() -> Option<(PathBuf, PathBuf)> {
    let ffmpeg = rehue::video::resolve_ffmpeg(None).ok()?;
    let ffprobe = ffmpeg.with_file_name("ffprobe");
    if !ffprobe.is_file() {
        return None;
    }
    Some((ffmpeg, ffprobe))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rehue-video-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A small test clip: moving color bars, a tone, and 12 frames.
fn clip(ffmpeg: &Path, path: &Path) {
    let ok = Command::new(ffmpeg)
        .args(["-v", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=96x54:rate=12:duration=1",
        ])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
        .arg(path)
        .status()
        .unwrap()
        .success();
    assert!(ok, "making the test clip");
}

fn probe(ffprobe: &Path, path: &Path, entries: &str) -> String {
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-count_frames", "-show_entries", entries])
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn wal(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rehue"))
        .arg("wal")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn video_is_picked_by_extension() {
    for name in ["a.mp4", "a.WEBM", "a.mkv", "a.mov"] {
        assert!(is_video(Path::new(name)), "{name}");
    }
    for name in ["a.png", "a.gif", "a.webp", "a"] {
        assert!(!is_video(Path::new(name)), "{name}");
    }
}

#[test]
fn a_video_is_graded_frame_by_frame_with_its_audio() {
    let Some((ffmpeg, ffprobe)) = tools() else {
        eprintln!("ffmpeg / ffprobe not found: skipping");
        return;
    };
    let dir = scratch("grade");
    let input = dir.join("clip.mp4");
    clip(&ffmpeg, &input);
    let scheme = fixture("solarized-dark.yaml");
    let run = |out: &str| {
        let out = dir.join(out);
        let run = wal(&[
            "-i",
            input.to_str().unwrap(),
            "-s",
            scheme.to_str().unwrap(),
            "-H",
            "1",
            "-o",
            out.to_str().unwrap(),
        ]);
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        out
    };
    let out = run("out");
    let video = out.join("wallpaper.mp4");
    for file in ["compare.png", "preview.html", "report.json"] {
        assert!(out.join(file).is_file(), "{file}");
    }
    assert!(!out.join("wallpaper.png").exists());
    // every frame, the same size, with the audio
    let streams = probe(
        &ffprobe,
        &video,
        "stream=codec_type,width,height,nb_read_frames",
    );
    let mut streams = streams.lines();
    assert_eq!(streams.next(), Some("video,96,54,12"));
    assert!(streams.next().is_some_and(|a| a.starts_with("audio")));
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["video"]["frames"], 12);
    let covered: u64 = report["slot_coverage"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_u64().unwrap())
        .sum();
    assert!(covered > 0 && covered <= 12 * 96 * 54);
    let html = std::fs::read_to_string(out.join("preview.html")).unwrap();
    assert!(html.contains(r#"<video src="wallpaper.mp4""#));

    // the frames rehue hands ffmpeg are the same every run
    let frames = |path: &Path| {
        Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .unwrap()
            .stdout
    };
    let again = run("again");
    assert!(frames(&video) == frames(&again.join("wallpaper.mp4")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inspect_and_scheme_take_a_video() {
    let Some((ffmpeg, _)) = tools() else {
        eprintln!("ffmpeg / ffprobe not found: skipping");
        return;
    };
    let dir = scratch("families");
    let input = dir.join("clip.mp4");
    clip(&ffmpeg, &input);
    let inspect = Command::new(env!("CARGO_BIN_EXE_rehue"))
        .args(["inspect", "-w"])
        .arg(&input)
        .arg("-o")
        .arg(dir.join("inspect"))
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    assert!(String::from_utf8_lossy(&inspect.stdout).contains("hue family"));
    assert!(dir.join("inspect/inspect.png").is_file());
    let scheme = Command::new(env!("CARGO_BIN_EXE_rehue"))
        .args(["scheme", "-i"])
        .arg(fixture("solarized-dark.yaml"))
        .arg("-w")
        .arg(&input)
        .arg("-o")
        .arg(dir.join("scheme"))
        .output()
        .unwrap();
    assert!(
        scheme.status.success(),
        "{}",
        String::from_utf8_lossy(&scheme.stderr)
    );
    assert!(dir.join("scheme/scheme.yaml").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_video_takes_no_dither_yet() {
    let Some((ffmpeg, _)) = tools() else {
        eprintln!("ffmpeg / ffprobe not found: skipping");
        return;
    };
    let dir = scratch("dither");
    let input = dir.join("clip.mp4");
    clip(&ffmpeg, &input);
    let scheme = fixture("solarized-dark.yaml");
    let run = wal(&[
        "-i",
        input.to_str().unwrap(),
        "-s",
        scheme.to_str().unwrap(),
        "--dither",
        "-o",
        dir.join("out").to_str().unwrap(),
    ]);
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("--dither doesn't take a video"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deband_and_grain_go_through_a_video() {
    let Some((ffmpeg, ffprobe)) = tools() else {
        eprintln!("ffmpeg / ffprobe not found: skipping");
        return;
    };
    let dir = scratch("deband");
    let input = dir.join("clip.mp4");
    clip(&ffmpeg, &input);
    let scheme = fixture("solarized-dark.yaml");
    let out = dir.join("out");
    let run = wal(&[
        "-i",
        input.to_str().unwrap(),
        "-s",
        scheme.to_str().unwrap(),
        "-H",
        "1",
        "--deband",
        "--grain",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let video = out.join("wallpaper.mp4");
    let streams = probe(
        &ffprobe,
        &video,
        "stream=codec_type,width,height,nb_read_frames",
    );
    assert_eq!(streams.lines().next(), Some("video,96,54,12"));
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["deband"], true);
    assert_eq!(report["grain"], 4.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ffmpeg_args_split_like_a_shell_and_refuse_what_rehue_owns() {
    use rehue::video::{EncodeArgs, split_args};
    assert_eq!(
        split_args(r#"-c:v libx264 -x264-params 'a=1:b=2' -metadata title="a \"b\"" x\ y"#)
            .unwrap(),
        [
            "-c:v",
            "libx264",
            "-x264-params",
            "a=1:b=2",
            "-metadata",
            r#"title=a "b""#,
            "x y"
        ]
    );
    assert!(split_args("-x 'open").is_err());
    let parsed = EncodeArgs::parse("-c:v libx265 -pix_fmt:v yuv444p -crf 20").unwrap();
    assert_eq!(parsed.pix_fmt.as_deref(), Some("yuv444p"));
    assert_eq!(parsed.args, ["-c:v", "libx265", "-crf", "20"]);
    for refused in [
        "-vf scale=1:1",
        "-filter:v null",
        "-map 0",
        "-t 2",
        "-i x.mp4",
        "-f matroska",
    ] {
        let err = EncodeArgs::parse(refused).unwrap_err();
        assert!(err.contains("can't take"), "{refused}: {err}");
    }
    assert!(EncodeArgs::parse("-pix_fmt").is_err());
}

#[test]
fn ffmpeg_args_set_how_the_video_is_encoded() {
    let Some((ffmpeg, ffprobe)) = tools() else {
        eprintln!("ffmpeg / ffprobe not found: skipping");
        return;
    };
    let dir = scratch("ffmpeg-args");
    let input = dir.join("clip.mp4");
    clip(&ffmpeg, &input);
    let scheme = fixture("solarized-dark.yaml");
    let out = dir.join("out");
    let args = "-c:v libx264 -preset ultrafast -crf 30 -pix_fmt yuv444p";
    let run = wal(&[
        "-i",
        input.to_str().unwrap(),
        "-s",
        scheme.to_str().unwrap(),
        "-H",
        "1",
        "--ffmpeg-args",
        args,
        "-o",
        out.to_str().unwrap(),
    ]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let streams = probe(
        &ffprobe,
        &out.join("wallpaper.mp4"),
        "stream=codec_type,codec_name,pix_fmt,nb_read_frames",
    );
    let mut streams = streams.lines();
    assert_eq!(streams.next(), Some("h264,video,yuv444p,12"));
    assert!(streams.next().is_some_and(|a| a.contains("audio")));
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["video"]["ffmpeg_args"], args);

    let refused = wal(&[
        "-i",
        input.to_str().unwrap(),
        "-s",
        scheme.to_str().unwrap(),
        "--ffmpeg-args",
        "-vf scale=8:8",
        "-o",
        dir.join("refused").to_str().unwrap(),
    ]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("can't take -vf"));
    let _ = std::fs::remove_dir_all(&dir);
}
