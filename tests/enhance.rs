//! `rehue enhance`: models, parameters, scaling, the SR tool and videos.
//!
//! The model table, parameter checks, Lanczos scaling, the models
//! directory, the SR tool's invocation (through a stand-in tool) and
//! videos (skipped without ffmpeg).

mod common;

use std::path::PathBuf;

use common::gradient_image;
use rehue::enhance::{EnhanceConfig, SrModel};

fn enhance_temp(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rehue-test-enhance-{}-{}", std::process::id(), tag))
}

fn cfg(upscale: u32, sr: bool, model: SrModel) -> EnhanceConfig {
    EnhanceConfig {
        upscale,
        sr,
        model,
        denoise: None,
        sr_tool: None,
        sr_models: None,
    }
}

#[test]
fn enhance_model_table() {
    assert_eq!(SrModel::Photos.tool_name(0.5), "realesrgan-x4plus");
    assert_eq!(SrModel::Anime.tool_name(0.5), "realesrgan-x4plus-anime");
    assert_eq!(SrModel::Soft.tool_name(0.5), "realesrnet-x4plus");
    assert_eq!(SrModel::General.tool_name(0.0), "realesr-general-x4v3-dn0");
    assert_eq!(
        SrModel::General.tool_name(0.5),
        "realesr-general-x4v3-dn0.5"
    );
    assert_eq!(SrModel::General.tool_name(1.0), "realesr-general-x4v3-dn1");
    assert!(SrModel::Photos.bundled() && SrModel::Anime.bundled());
    assert!(!SrModel::Soft.bundled() && !SrModel::General.bundled());
    for m in [
        SrModel::Photos,
        SrModel::Anime,
        SrModel::Soft,
        SrModel::General,
    ] {
        assert_eq!(m.native_scale(), 4);
    }
}

#[test]
fn denoise_goes_with_general_at_three_levels() {
    let mut c = cfg(2, true, SrModel::General);
    for d in [0.0, 0.5, 1.0] {
        c.denoise = Some(d);
        c.validate().expect("a denoise level");
    }
    c.denoise = Some(0.3);
    let err = c.validate().expect_err("between levels");
    assert!(err.contains("0, 0.5 or 1"), "{err}");
    let mut c = cfg(2, true, SrModel::Photos);
    c.denoise = Some(0.0);
    let err = c.validate().expect_err("denoise without general");
    assert!(err.contains("--model general"), "{err}");
}

#[test]
fn the_models_directory_must_hold_the_model() {
    let temp = enhance_temp("models");
    std::fs::create_dir_all(&temp).expect("temp dir");
    let name = SrModel::Soft.tool_name(0.5);
    let err = rehue::enhance::resolve_sr_models(Some(&temp), &name).expect_err("empty dir");
    assert!(
        err.contains("--sr-models") && err.contains("realesrnet-x4plus.param"),
        "{err}"
    );
    std::fs::write(temp.join(format!("{name}.param")), "").unwrap();
    std::fs::write(temp.join(format!("{name}.bin")), "").unwrap();
    let found = rehue::enhance::resolve_sr_models(Some(&temp), &name).expect("found");
    assert_eq!(found, temp);
    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn enhance_validates_its_params() {
    let temp = enhance_temp("params");
    std::fs::create_dir_all(&temp).expect("temp dir");
    let src = temp.join("src.png");
    gradient_image((48, 32)).save(&src).expect("fixture writes");

    let cfg = |upscale: u32, sr: bool, sr_tool: Option<PathBuf>| EnhanceConfig {
        sr_tool,
        ..cfg(upscale, sr, SrModel::Photos)
    };
    let err =
        rehue::enhance::run(&src, &temp, &cfg(0, false, None), None).expect_err("scale 0 rejected");
    assert!(err.contains("at least 1"), "{err}");
    let err = rehue::enhance::run(&src, &temp, &cfg(1, false, None), None)
        .expect_err("identity lanczos rejected");
    assert!(err.contains("nothing to do"), "{err}");
    let err =
        rehue::enhance::run(&src, &temp, &cfg(9, false, None), None).expect_err("lanczos cap");
    assert!(err.contains("sanity cap"), "{err}");
    let err = rehue::enhance::run(&src, &temp, &cfg(8, true, None), None)
        .expect_err("beyond native rejected before any tool runs");
    assert!(err.contains("natively"), "{err}");
    let err = rehue::enhance::run(&src, &temp, &cfg(2, true, Some(temp.join("absent"))), None)
        .expect_err("missing tool");
    assert!(err.contains("not a file"), "{err}");

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn enhance_lanczos_scales_deterministically() {
    let temp = enhance_temp("lanczos");
    std::fs::create_dir_all(&temp).expect("temp dir");
    let src = temp.join("src.png");
    gradient_image((48, 32)).save(&src).expect("fixture writes");
    let cfg = cfg(2, false, SrModel::Photos);
    let first = rehue::enhance::run(&src, &temp.join("a"), &cfg, None).expect("enhance runs");
    assert_eq!(first, temp.join("a").join("src.png"), "stem-named artifact");
    let second = rehue::enhance::run(&src, &temp.join("b"), &cfg, None).expect("enhance runs");
    let decoded = image::ImageReader::open(&first)
        .expect("artifact reads")
        .decode()
        .expect("artifact decodes");
    assert_eq!((decoded.width(), decoded.height()), (96, 64));
    let a = std::fs::read(&first).expect("artifact bytes");
    let b = std::fs::read(&second).expect("artifact bytes");
    assert_eq!(a, b, "lanczos enhance is byte-deterministic");

    let _ = std::fs::remove_dir_all(&temp);
}

/// A stand-in for realesrgan-ncnn-vulkan: logs its arguments, fails
/// unless `-m` (when given) is a directory whose path says "models" and
/// holds the model, and fills `-o` with copies of `$FAKE_SR_OUTPUT`, one
/// per input frame.
#[cfg(unix)]
fn fake_sr_tool(dir: &std::path::Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let tool = dir.join("fake-sr");
    std::fs::write(
        &tool,
        r#"#!/bin/sh
echo "$@" >> "$FAKE_SR_LOG"
while [ $# -gt 0 ]; do
  case "$1" in
    -i) in="$2"; shift ;;
    -o) out="$2"; shift ;;
    -m) models="$2"; shift ;;
    -n) name="$2"; shift ;;
  esac
  shift
done
if [ -n "$models" ]; then
  case "$models" in *models*) ;; *) echo "unknown model dir type" >&2; exit 1 ;; esac
  [ -f "$models/$name.param" ] && [ -f "$models/$name.bin" ] || { echo "no model" >&2; exit 1; }
fi
for f in "$in"/*.png; do cp "$FAKE_SR_OUTPUT" "$out/$(basename "$f")"; done
"#,
    )
    .unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    tool
}

/// --model general: the tool gets the staged models, the denoise
/// level's name, its thread settings and the native scale; the result
/// is brought to the target with Lanczos.
#[cfg(unix)]
#[test]
fn sr_runs_a_models_directory_model_through_the_tool() {
    let temp = enhance_temp("sr-general");
    std::fs::create_dir_all(&temp).expect("temp dir");
    let src = temp.join("src.png");
    gradient_image((12, 8)).save(&src).unwrap();
    let native = temp.join("native.png");
    gradient_image((48, 32)).save(&native).unwrap();
    // a directory not named "models", as a user's may be
    let weights = temp.join("weights");
    std::fs::create_dir_all(&weights).unwrap();
    for ext in ["param", "bin"] {
        std::fs::write(weights.join(format!("realesr-general-x4v3-dn0.{ext}")), "").unwrap();
    }
    let log = temp.join("log");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rehue"))
        .args(["enhance", "-i", src.to_str().unwrap(), "-o"])
        .arg(temp.join("out"))
        .args([
            "--upscale",
            "2",
            "--sr",
            "--model",
            "general",
            "--denoise",
            "0",
        ])
        .arg("--sr-models")
        .arg(&weights)
        .arg("--sr-tool")
        .arg(fake_sr_tool(&temp))
        .env("FAKE_SR_LOG", &log)
        .env("FAKE_SR_OUTPUT", &native)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let args = std::fs::read_to_string(&log).unwrap();
    assert!(args.contains("-n realesr-general-x4v3-dn0 -s 4"), "{args}");
    assert!(args.contains(" -j "), "{args}");
    let result = image::open(temp.join("out/src.png")).unwrap();
    assert_eq!((result.width(), result.height()), (24, 16));
    let _ = std::fs::remove_dir_all(&temp);
}

fn ffmpeg() -> Option<PathBuf> {
    let ffmpeg = rehue::video::resolve_ffmpeg(None).ok()?;
    ffmpeg.with_file_name("ffprobe").is_file().then_some(ffmpeg)
}

/// A 96x54 clip of 30 frames with a tone.
fn clip(ffmpeg: &std::path::Path, path: &std::path::Path) {
    let ok = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=96x54:rate=12:duration=2.5")
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2.5"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
        .arg(path)
        .status()
        .unwrap()
        .success();
    assert!(ok, "making the test clip");
}

fn probe(ffmpeg: &std::path::Path, path: &std::path::Path, entries: &str) -> String {
    let out = std::process::Command::new(ffmpeg.with_file_name("ffprobe"))
        .args(["-v", "error", "-count_frames", "-show_entries", entries])
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn a_video_is_scaled_frame_by_frame_with_its_audio() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("ffmpeg / ffprobe not found: skipping");
        return;
    };
    let temp = enhance_temp("video");
    std::fs::create_dir_all(&temp).expect("temp dir");
    let src = temp.join("clip.mp4");
    clip(&ffmpeg, &src);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rehue"))
        .args(["enhance", "-i", src.to_str().unwrap(), "-o"])
        .arg(temp.join("out"))
        .args(["--upscale", "2"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result = temp.join("out/clip.mp4");
    let video = probe(&ffmpeg, &result, "stream=width,height,nb_read_frames");
    assert_eq!(video.lines().next(), Some("192,108,30"), "{video}");
    assert!(
        probe(&ffmpeg, &result, "stream=codec_type").contains("audio"),
        "audio kept"
    );
    let _ = std::fs::remove_dir_all(&temp);
}

/// A video through SR: frames go to the tool in batches (30 frames: two
/// runs), come back at the native scale, and are brought to the target.
#[cfg(unix)]
#[test]
fn a_video_goes_through_the_sr_tool_in_batches() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("ffmpeg / ffprobe not found: skipping");
        return;
    };
    let temp = enhance_temp("video-sr");
    std::fs::create_dir_all(&temp).expect("temp dir");
    let src = temp.join("clip.mp4");
    clip(&ffmpeg, &src);
    let native = temp.join("native.png");
    gradient_image((384, 216)).save(&native).unwrap();
    let log = temp.join("log");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rehue"))
        .args(["enhance", "-i", src.to_str().unwrap(), "-o"])
        .arg(temp.join("out"))
        .args(["--upscale", "2", "--sr", "--sr-tool"])
        .arg(fake_sr_tool(&temp))
        .env("FAKE_SR_LOG", &log)
        .env("FAKE_SR_OUTPUT", &native)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let runs = std::fs::read_to_string(&log).unwrap();
    assert_eq!(runs.lines().count(), 2, "{runs}");
    assert!(
        runs.contains("-n realesrgan-x4plus -s 4") && !runs.contains(" -m "),
        "{runs}"
    );
    let result = temp.join("out/clip.mp4");
    let video = probe(&ffmpeg, &result, "stream=width,height,nb_read_frames");
    assert_eq!(video.lines().next(), Some("192,108,30"), "{video}");
    let _ = std::fs::remove_dir_all(&temp);
}
