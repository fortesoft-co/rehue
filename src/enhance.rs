//! `rehue enhance`: Lanczos upscaling and Vulkan super-resolution.
//!
//! For pictures and videos.  Wallpaper enhancement: resolution scaling on the source, with the
//! palette work left to wal.  Two methods: built-in Lanczos resampling
//! (deterministic, pure) and opt-in AI super-resolution via the
//! ncnn/vulkan tool `realesrgan-ncnn-vulkan` (BSD-3 code and weights;
//! vulkan means no CUDA and no assumptions about the driver).
//!
//! The ordering contract makes the AI stage safe: SR invents detail
//! that inherits the input's hue statistics, so it runs on the SOURCE
//! and the palette transform runs after, exactly as it would at source
//! resolution.  AI touches detail; rehue touches colour.  SR output is
//! fp16 and per-device (byte-identical only on the same gpu/driver);
//! Lanczos is byte-deterministic.
//!
//! Models: `photos` and `anime` are the weights the tool ships.  `soft`
//! (RealESRNet x4plus) and `general` (realesr-general-x4v3 at three
//! denoise levels) are converted from the official Real-ESRGAN weights
//! by `scripts/sr-models.py` and found in a models directory.
//!
//! Tool invocation detail: every model is native to 4x and the tool's
//! own `-s` is unreliable at other factors, so the model always runs at
//! its native scale and the requested factor is reached with the same
//! Lanczos resampler afterwards.  A video's frames go through the tool
//! in batches (one model load per batch, not per frame).

use std::path::{Path, PathBuf};
use std::process::Command;

use image::RgbImage;
use image::imageops::FilterType;

use crate::video;

/// The SR tool this module drives.
pub const SR_TOOL_NAME: &str = "realesrgan-ncnn-vulkan";

/// The `general` model's denoise levels (`--denoise`): 0 keeps the most
/// grain, 1 smooths the most.
pub const DENOISE_LEVELS: [&str; 3] = ["0", "0.5", "1"];

/// `--denoise` when not given.
pub const DEFAULT_DENOISE: f32 = 0.5;

/// Frames per SR tool run on a video.
const BATCH: usize = 24;

/// An `enhance` config file: the same settings as the flags, which win
/// over it.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct EnhanceFile {
    pub upscale: Option<u32>,
    pub sr: bool,
    pub model: Option<SrModel>,
    pub denoise: Option<f32>,
    /// Relative to the config file.
    pub sr_tool: Option<PathBuf>,
    /// Relative to the config file.
    pub sr_models: Option<PathBuf>,
    /// Relative to the config file.
    pub ffmpeg: Option<PathBuf>,
    pub ffmpeg_args: Option<String>,
}

/// The SR weights (`--model`); all upscale 4x natively.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum SrModel {
    /// Photographs (`realesrgan-x4plus`, shipped with the tool).
    Photos,
    /// Drawn and painted pictures with flat areas
    /// (`realesrgan-x4plus-anime`, shipped with the tool).
    Anime,
    /// Photographs without invented texture (RealESRNet x4plus; from
    /// the models directory).
    Soft,
    /// A light general model with a denoise dial (realesr-general-x4v3;
    /// from the models directory).
    General,
}

impl SrModel {
    /// The tool's `-n` value; `denoise` picks `general`'s level.
    pub fn tool_name(&self, denoise: f32) -> String {
        match self {
            Self::Photos => "realesrgan-x4plus".to_string(),
            Self::Anime => "realesrgan-x4plus-anime".to_string(),
            Self::Soft => "realesrnet-x4plus".to_string(),
            Self::General => format!("realesr-general-x4v3-dn{}", denoise_label(denoise)),
        }
    }

    /// Whether the tool ships these weights (else they come from the
    /// models directory).
    pub fn bundled(&self) -> bool {
        matches!(self, Self::Photos | Self::Anime)
    }

    /// The scale the weights are native to.
    pub fn native_scale(&self) -> u32 {
        let _ = self;
        4
    }
}

/// `0`, `0.5` or `1`.
fn denoise_label(denoise: f32) -> &'static str {
    if denoise == 0.0 {
        "0"
    } else if denoise == 1.0 {
        "1"
    } else {
        "0.5"
    }
}

/// Enhancement parameters.
#[derive(Debug, Clone)]
pub struct EnhanceConfig {
    /// Target scale vs the source: 2..=8 with `sr` off (Lanczos);
    /// 1..=native with `sr` on (1 = same-size restoration).
    pub upscale: u32,
    /// Use the AI SR path (needs [`SR_TOOL_NAME`]).
    pub sr: bool,
    /// SR weights (inert without `sr`).
    pub model: SrModel,
    /// `general`'s denoise level (0, 0.5 or 1); only with `general`.
    pub denoise: Option<f32>,
    /// Explicit tool path; else `$REHUE_SR_TOOL`, else $PATH.
    pub sr_tool: Option<PathBuf>,
    /// Explicit models directory (for `soft` and `general`); else
    /// `$REHUE_SR_MODELS`, else `$XDG_DATA_HOME/rehue/sr-models`.
    pub sr_models: Option<PathBuf>,
}

impl EnhanceConfig {
    /// Check the settings before any work.
    pub fn validate(&self) -> Result<(), String> {
        let upscale = self.upscale;
        if upscale == 0 {
            return Err("--upscale must be at least 1".to_string());
        }
        let native = self.model.native_scale();
        if self.sr && upscale > native {
            return Err(format!(
                "the SR weights upscale {}x natively; --upscale {} exceeds it \
                 (lower the target, or run enhance twice)",
                native, upscale
            ));
        }
        if !self.sr && upscale > 8 {
            return Err(format!(
                "--upscale {} is beyond the plain-Lanczos sanity cap (8x; \
                 use --sr for AI resolution)",
                upscale
            ));
        }
        if !self.sr && upscale == 1 {
            return Err("nothing to do: --upscale 1 without --sr".to_string());
        }
        if let Some(d) = self.denoise {
            if self.model != SrModel::General {
                return Err("--denoise goes with --model general".to_string());
            }
            if ![0.0, 0.5, 1.0].contains(&d) {
                return Err(format!("--denoise takes 0, 0.5 or 1 (got {d})"));
            }
        }
        Ok(())
    }

    fn denoise(&self) -> f32 {
        self.denoise.unwrap_or(DEFAULT_DENOISE)
    }
}

/// Find the SR tool: `--sr-tool`, then `$REHUE_SR_TOOL`, then $PATH.
pub fn resolve_sr_tool(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        if !path.is_file() {
            return Err(format!("--sr-tool {}: not a file", path.display()));
        }
        return Ok(path.to_path_buf());
    }
    if let Some(value) = std::env::var_os("REHUE_SR_TOOL") {
        let path = PathBuf::from(value);
        if !path.is_file() {
            return Err(format!("$REHUE_SR_TOOL {}: not a file", path.display()));
        }
        return Ok(path);
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(SR_TOOL_NAME);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(format!(
        "{SR_TOOL_NAME} not found on $PATH (or $REHUE_SR_TOOL / --sr-tool); \
         nix: nix profile install nixpkgs#realesrgan-ncnn-vulkan; upstream releases: \
         https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan/releases"
    ))
}

/// The default models directory: `$XDG_DATA_HOME/rehue/sr-models`, else
/// `~/.local/share/rehue/sr-models`.
pub fn default_sr_models() -> Option<PathBuf> {
    let env = |k| std::env::var_os(k).filter(|v| !v.is_empty());
    env("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|d| d.join("rehue/sr-models"))
}

/// Find the models directory holding `name`: `--sr-models`, then
/// `$REHUE_SR_MODELS`, then [`default_sr_models`].
pub fn resolve_sr_models(explicit: Option<&Path>, name: &str) -> Result<PathBuf, String> {
    let has = |dir: &Path| {
        dir.join(format!("{name}.param")).is_file() && dir.join(format!("{name}.bin")).is_file()
    };
    let check = |dir: PathBuf, label: &str| {
        if has(&dir) {
            Ok(dir)
        } else {
            Err(format!(
                "{label} {}: no {name}.param / {name}.bin there",
                dir.display()
            ))
        }
    };
    if let Some(dir) = explicit {
        return check(dir.to_path_buf(), "--sr-models");
    }
    if let Some(value) = std::env::var_os("REHUE_SR_MODELS").filter(|v| !v.is_empty()) {
        return check(PathBuf::from(value), "$REHUE_SR_MODELS");
    }
    if let Some(dir) = default_sr_models().filter(|d| has(d)) {
        return Ok(dir);
    }
    Err(format!(
        "the {name} model isn't installed: rehue looks in --sr-models, $REHUE_SR_MODELS, then {}; \
         get rehue-sr-models.tar.gz from https://github.com/fortesoft-co/rehue/releases \
         and unpack it there, or nix build github:fortesoft-co/rehue#sr-models",
        default_sr_models()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|| "$XDG_DATA_HOME/rehue/sr-models".to_string())
    ))
}

/// A ready SR run: the tool, the model's `-n` name, and where its files
/// are (`None`: the tool's own).
#[derive(Debug, Clone)]
pub struct Sr {
    pub tool: PathBuf,
    pub name: String,
    pub models: Option<PathBuf>,
}

impl Sr {
    /// Resolve the tool and the model's files for `cfg`.
    pub fn resolve(cfg: &EnhanceConfig) -> Result<Self, String> {
        let name = cfg.model.tool_name(cfg.denoise());
        let models = if cfg.model.bundled() {
            None
        } else {
            Some(resolve_sr_models(cfg.sr_models.as_deref(), &name)?)
        };
        Ok(Sr {
            tool: resolve_sr_tool(cfg.sr_tool.as_deref())?,
            name,
            models,
        })
    }

    /// Upscale `frames` in one tool run at the model's `native` scale,
    /// then with Lanczos to `upscale` x.  Private temp dir; cleaned up on
    /// every outcome path.
    pub fn upscale(
        &self,
        frames: &[RgbImage],
        native: u32,
        upscale: u32,
    ) -> Result<Vec<RgbImage>, String> {
        let temp = std::env::temp_dir().join(format!(
            "rehue-enhance-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        let result = self.upscale_in(frames, native, upscale, &temp);
        let _ = std::fs::remove_dir_all(&temp);
        result
    }

    fn upscale_in(
        &self,
        frames: &[RgbImage],
        native: u32,
        upscale: u32,
        temp: &Path,
    ) -> Result<Vec<RgbImage>, String> {
        let (input, output) = (temp.join("in"), temp.join("out"));
        for dir in [&input, &output] {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("can't create {}: {}", dir.display(), e))?;
        }
        let file = |i: usize| format!("{i:06}.png");
        crate::par::chunks(frames, 1, |start, run| {
            for (k, frame) in run.iter().enumerate() {
                let path = input.join(file(start + k));
                frame
                    .save_with_format(&path, image::ImageFormat::Png)
                    .map_err(|e| format!("can't stage {}: {}", path.display(), e))?;
            }
            Ok::<_, String>(())
        })
        .into_iter()
        .collect::<Result<(), String>>()?;
        // the tool loads and saves on 1 and 2 threads by default, and
        // saving a 4x PNG is most of its time
        let threads = crate::par::threads();
        let jobs = format!("{}:2:{}", threads.clamp(1, 4), threads.clamp(2, 16));
        let mut cmd = Command::new(&self.tool);
        cmd.arg("-i").arg(&input).arg("-o").arg(&output);
        cmd.args(["-n", &self.name, "-s", &native.to_string(), "-f", "png"]);
        cmd.args(["-j", &jobs]);
        if let Some(dir) = &self.models {
            // the tool takes only a directory whose path says "models"
            let staged = temp.join("models");
            std::fs::create_dir_all(&staged)
                .map_err(|e| format!("can't create {}: {}", staged.display(), e))?;
            for ext in ["param", "bin"] {
                let name = format!("{}.{ext}", self.name);
                let from = std::path::absolute(dir.join(&name))
                    .map_err(|e| format!("{}: {e}", dir.display()))?;
                link(&from, &staged.join(&name))?;
            }
            cmd.arg("-m").arg(&staged);
        }
        let invocation = cmd
            .output()
            .map_err(|e| format!("can't run {}: {}", self.tool.display(), e))?;
        if !invocation.status.success() {
            return Err(format!(
                "{} failed ({}): {}",
                self.tool.display(),
                invocation.status,
                tool_tail(&invocation)
            ));
        }
        let indices: Vec<usize> = (0..frames.len()).collect();
        crate::par::chunks(&indices, 1, |_, run| {
            run.iter()
                .map(|&i| {
                    let frame = &frames[i];
                    let path = output.join(file(i));
                    let scaled = image::ImageReader::open(&path)
                        .map_err(|e| {
                            format!(
                                "SR tool wrote nothing usable at {}: {}; tool output: {}",
                                path.display(),
                                e,
                                tool_tail(&invocation)
                            )
                        })?
                        .decode()
                        .map_err(|e| format!("can't decode SR output {}: {}", path.display(), e))?
                        .to_rgb8();
                    let want = (frame.width() * native, frame.height() * native);
                    if (scaled.width(), scaled.height()) != want {
                        return Err(format!(
                            "SR output is {}x{}, expected {}x{}; tool output: {}",
                            scaled.width(),
                            scaled.height(),
                            want.0,
                            want.1,
                            tool_tail(&invocation)
                        ));
                    }
                    Ok(if upscale == native {
                        scaled
                    } else {
                        let (w, h) = (frame.width() * upscale, frame.height() * upscale);
                        image::imageops::resize(&scaled, w, h, FilterType::Lanczos3)
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .into_iter()
        .collect::<Result<Vec<Vec<_>>, String>>()
        .map(|runs| runs.into_iter().flatten().collect())
    }
}

#[cfg(unix)]
fn link(from: &Path, to: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(from, to).map_err(|e| format!("can't link {}: {}", to.display(), e))
}

#[cfg(not(unix))]
fn link(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("can't copy {}: {}", to.display(), e))
}

/// Bring frames to `upscale` x the source: through SR (at its native
/// scale, then Lanczos to the target) or Lanczos alone.
fn scale_frames(
    frames: Vec<RgbImage>,
    cfg: &EnhanceConfig,
    sr: Option<&Sr>,
) -> Result<Vec<RgbImage>, String> {
    let upscale = cfg.upscale;
    match sr {
        Some(sr) => sr.upscale(&frames, cfg.model.native_scale(), upscale),
        None => Ok(crate::par::chunks(&frames, 1, |_, run| {
            run.iter()
                .map(|f| {
                    let (w, h) = (f.width() * upscale, f.height() * upscale);
                    image::imageops::resize(f, w, h, FilterType::Lanczos3)
                })
                .collect::<Vec<_>>()
        })
        .into_iter()
        .flatten()
        .collect()),
    }
}

/// The artifact for `source` in `out_dir`: the source's stem, as a PNG
/// for a picture and in the source's container for a video.
pub fn artifact_path(source: &Path, out_dir: &Path) -> Result<PathBuf, String> {
    let stem = source
        .file_stem()
        .ok_or_else(|| format!("cannot derive an artifact name from {}", source.display()))?;
    let ext = if video::is_video(source) {
        source
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| "mp4".to_string())
    } else {
        "png".to_string()
    };
    Ok(out_dir.join(stem).with_extension(ext))
}

/// Enhance the picture into `out_dir`, writing the source file's stem
/// as a PNG there.  `sr`: a resolved SR run (else resolved from `cfg`
/// when `cfg.sr`).  Returns the artifact path.
pub fn run(
    source: &Path,
    out_dir: &Path,
    cfg: &EnhanceConfig,
    sr: Option<&Sr>,
) -> Result<PathBuf, String> {
    cfg.validate()?;
    let dynamic = image::ImageReader::open(source)
        .map_err(|e| format!("can't read wallpaper {}: {}", source.display(), e))?
        .decode()
        .map_err(|e| format!("can't decode wallpaper {}: {}", source.display(), e))?;
    let resolved;
    let sr = match (cfg.sr, sr) {
        (false, _) => None,
        (true, Some(sr)) => Some(sr),
        (true, None) => {
            resolved = Sr::resolve(cfg)?;
            Some(&resolved)
        }
    };
    let scaled = scale_frames(vec![dynamic.to_rgb8()], cfg, sr)?
        .pop()
        .expect("one frame in, one out");
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("can't create {}: {}", out_dir.display(), e))?;
    let artifact = artifact_path(source, out_dir)?;
    scaled
        .save(&artifact)
        .map_err(|e| format!("can't write {}: {}", artifact.display(), e))?;
    Ok(artifact)
}

/// Enhance a video into `out_dir` (the source's stem and container),
/// with its audio.  `tick` hears the frames done so far.
pub fn run_video(
    source: &Path,
    out_dir: &Path,
    cfg: &EnhanceConfig,
    sr: Option<&Sr>,
    ffmpeg: &Path,
    encode: &video::EncodeArgs,
    tick: &mut dyn FnMut(u64),
) -> Result<PathBuf, String> {
    cfg.validate()?;
    let probe = video::probe(ffmpeg, source)?;
    let resolved;
    let sr = match (cfg.sr, sr) {
        (false, _) => None,
        (true, Some(sr)) => Some(sr),
        (true, None) => {
            resolved = Sr::resolve(cfg)?;
            Some(&resolved)
        }
    };
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("can't create {}: {}", out_dir.display(), e))?;
    let artifact = artifact_path(source, out_dir)?;
    let mut out_probe = probe.clone();
    out_probe.width *= cfg.upscale;
    out_probe.height *= cfg.upscale;
    let mut decoder = video::Decoder::open(ffmpeg, source, &probe, false, None)?;
    let mut encoder = video::Encoder::open(ffmpeg, source, &artifact, &out_probe, encode, None)?;
    let batch = BATCH;
    let mut pending = Vec::with_capacity(batch);
    let mut frame = Vec::new();
    let mut done = 0u64;
    let mut flush = |pending: &mut Vec<RgbImage>, encoder: &mut video::Encoder| {
        for scaled in scale_frames(std::mem::take(pending), cfg, sr)? {
            encoder.write(scaled.as_raw())?;
            done += 1;
            tick(done);
        }
        Ok::<_, String>(())
    };
    while decoder.next(&mut frame)? {
        pending.push(
            RgbImage::from_raw(probe.width, probe.height, std::mem::take(&mut frame))
                .expect("frame is the probed size"),
        );
        if pending.len() == batch {
            flush(&mut pending, &mut encoder)?;
        }
    }
    flush(&mut pending, &mut encoder)?;
    encoder.finish()?;
    if done == 0 {
        return Err(format!("{}: no frames", source.display()));
    }
    Ok(artifact)
}

/// Diagnostic text on a tool failure: stderr first, stdout fallback.
fn tool_tail(invocation: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&invocation.stderr);
    let text = if stderr.trim().is_empty() {
        String::from_utf8_lossy(&invocation.stdout)
    } else {
        stderr
    };
    let text = text.trim_end();
    let len = text.chars().count();
    if len > 2000 {
        format!("…{}", text.chars().skip(len - 2000).collect::<String>())
    } else {
        text.to_string()
    }
}
