//! Wallpaper enhancement: resolution scaling on the source image, with
//! the palette work left to map-wal.  Two methods: built-in Lanczos
//! resampling (deterministic, pure) and opt-in AI super-resolution via
//! the ncnn/vulkan tool `realesrgan-ncnn-vulkan` (BSD-3 code and
//! weights; vulkan means no CUDA and no assumptions about the driver).
//!
//! The ordering contract makes the AI stage safe: SR invents detail
//! that inherits the input's hue statistics, so it runs on the SOURCE
//! image and the palette transform runs after, exactly as it would at
//! source resolution.  AI touches detail; rehue touches colour.  SR
//! output is fp16 and per-device (byte-identical only on the same
//! gpu/driver); Lanczos is byte-deterministic.
//!
//! Tool invocation detail: the bundled weights are native to one scale
//! (4x) and the tool's own `-s` is unreliable at other factors, so the
//! model always runs at its native scale and the requested factor is
//! reached with the same Lanczos resampler afterwards.

use std::path::{Path, PathBuf};
use std::process::Command;

use image::imageops::FilterType;

/// The SR tool this module drives.
pub const SR_TOOL_NAME: &str = "realesrgan-ncnn-vulkan";

/// The bundled Real-ESRGAN weights (`--model`); both upscale 4x
/// natively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SrModel {
    /// General photographic weights (`realesrgan-x4plus`).
    Photos,
    /// Stylized weights (`realesrgan-x4plus-anime`); often better on
    /// painted/flat content.
    Anime,
}

impl SrModel {
    /// The tool's `-n` value.
    pub fn tool_name(&self) -> &'static str {
        match self {
            Self::Photos => "realesrgan-x4plus",
            Self::Anime => "realesrgan-x4plus-anime",
        }
    }

    /// The scale the weights are native to.
    pub fn native_scale(&self) -> u32 {
        let _ = self;
        4
    }
}

/// Enhancement parameters (CLI-shaped; no config-JSON layer in v1).
#[derive(Debug, Clone)]
pub struct EnhanceConfig {
    /// Target scale vs the source: 2..=8 with `sr` off (Lanczos);
    /// 1..=native with `sr` on (1 = same-size restoration).
    pub upscale: u32,
    /// Use the AI SR path (needs [`SR_TOOL_NAME`]).
    pub sr: bool,
    /// SR weights (inert without `sr`).
    pub model: SrModel,
    /// Explicit tool path; else `$REHUE_SR_TOOL`, else $PATH.
    pub sr_tool: Option<PathBuf>,
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

/// Enhance the wallpaper into `out_dir`, writing the source file's stem
/// as a PNG there.  Returns the artifact path.
pub fn run(
    source: &Path,
    out_dir: &Path,
    cfg: &EnhanceConfig,
    sr_tool: Option<&Path>,
) -> Result<PathBuf, String> {
    let upscale = cfg.upscale;
    if upscale == 0 {
        return Err("--upscale must be at least 1".to_string());
    }
    let native = cfg.model.native_scale();
    if cfg.sr && upscale > native {
        return Err(format!(
            "the SR weights upscale {}x natively; --upscale {} exceeds it \
             (lower the target, or run enhance twice)",
            native, upscale
        ));
    }
    if !cfg.sr && upscale > 8 {
        return Err(format!(
            "--upscale {} is beyond the plain-Lanczos sanity cap (8x; \
             use --sr for AI resolution)",
            upscale
        ));
    }
    if !cfg.sr && upscale == 1 {
        return Err("nothing to do: --upscale 1 without --sr".to_string());
    }

    let dynamic = image::ImageReader::open(source)
        .map_err(|e| format!("can't read wallpaper {}: {}", source.display(), e))?
        .decode()
        .map_err(|e| format!("can't decode wallpaper {}: {}", source.display(), e))?;
    let source_rgb = dynamic.to_rgb8();
    let (w, h) = (source_rgb.width(), source_rgb.height());

    let scaled = if cfg.sr {
        let tool = match sr_tool {
            Some(tool) => tool.to_path_buf(),
            None => resolve_sr_tool(cfg.sr_tool.as_deref())?,
        };
        let native_rgb = sr_invoke(&tool, &source_rgb, &cfg.model, native)?;
        if upscale == native {
            native_rgb
        } else {
            image::imageops::resize(&native_rgb, w * upscale, h * upscale, FilterType::Lanczos3)
        }
    } else {
        image::imageops::resize(&source_rgb, w * upscale, h * upscale, FilterType::Lanczos3)
    };

    let stem = source
        .file_stem()
        .ok_or_else(|| format!("cannot derive an artifact name from {}", source.display()))?;
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("can't create {}: {}", out_dir.display(), e))?;
    let artifact = out_dir.join(stem).with_extension("png");
    image::RgbImage::from_raw(scaled.width(), scaled.height(), scaled.into_raw())
        .expect("resize preserves the pixel-buffer size")
        .save(&artifact)
        .map_err(|e| format!("can't write {}: {}", artifact.display(), e))?;
    Ok(artifact)
}

/// Drive the tool: stage a PNG, run the model at its native scale,
/// validate the returned geometry.  Private temp dir; cleaned up on
/// every outcome path.
fn sr_invoke(
    tool: &Path,
    rgb: &image::RgbImage,
    model: &SrModel,
    native: u32,
) -> Result<image::RgbImage, String> {
    let temp = std::env::temp_dir().join(format!("rehue-enhance-{}", std::process::id()));
    std::fs::create_dir_all(&temp)
        .map_err(|e| format!("can't create {}: {}", temp.display(), e))?;
    let result = sr_invoke_inner(tool, rgb, model, native, &temp);
    let _ = std::fs::remove_dir_all(&temp);
    result
}

fn sr_invoke_inner(
    tool: &Path,
    rgb: &image::RgbImage,
    model: &SrModel,
    native: u32,
    temp: &Path,
) -> Result<image::RgbImage, String> {
    let input = temp.join("input.png");
    rgb.save_with_format(&input, image::ImageFormat::Png)
        .map_err(|e| format!("can't stage {}: {}", input.display(), e))?;
    let output = temp.join("native.png");
    let invocation = Command::new(tool)
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("-n")
        .arg(model.tool_name())
        .arg("-s")
        .arg(native.to_string())
        .arg("-f")
        .arg("png")
        .output()
        .map_err(|e| format!("can't run {}: {}", tool.display(), e))?;
    if !invocation.status.success() {
        return Err(format!(
            "{} failed ({}): {}",
            tool.display(),
            invocation.status,
            tool_tail(&invocation)
        ));
    }
    let scaled = image::ImageReader::open(&output)
        .map_err(|e| {
            format!(
                "SR tool wrote nothing usable at {}: {}",
                output.display(),
                e
            )
        })?
        .decode()
        .map_err(|e| format!("can't decode SR output {}: {}", output.display(), e))?
        .to_rgb8();
    let want = (rgb.width() * native, rgb.height() * native);
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
    Ok(scaled)
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
