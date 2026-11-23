//! Video wallpapers: frames in and out through ffmpeg, graded in between.
//!
//! rehue doesn't decode or encode video itself.  ffmpeg (found like
//! `enhance`'s SR tool: `--ffmpeg`, `$REHUE_FFMPEG`, then `$PATH`)
//! decodes the video to raw RGB frames on a pipe, rehue grades each
//! frame, and a second ffmpeg encodes them, with the source's audio
//! copied across.  The frames rehue hands over are the same bytes every
//! run; the encoded file also depends on the ffmpeg build.
//!
//! Colors: frames are read through the stream's own color matrix and
//! range (untagged HD video is taken as BT.709, the usual convention,
//! and untagged SD as BT.601), and written as BT.709 limited range,
//! tagged so players read them back the same way.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread::JoinHandle;

use image::RgbImage;

/// The tool this module drives, and its probe companion (found beside it).
pub const FFMPEG: &str = "ffmpeg";
const FFPROBE: &str = "ffprobe";

/// The video containers `wal` takes, by extension; the output keeps the
/// input's.
pub const EXTENSIONS: [&str; 4] = ["mp4", "webm", "mkv", "mov"];

/// How many frames, spread over the whole video, the grade is measured
/// on (vivid chroma, and the extraction `assign` uses).
pub const SAMPLE_FRAMES: u32 = 32;

/// The width sampled frames are scaled to.
const SAMPLE_WIDTH: u32 = 480;

/// A scaler that rounds accurately and samples color at full
/// resolution, for both directions.
const SWS_FLAGS: &str = "accurate_rnd+full_chroma_int+bitexact";

/// Whether `path` names a video, by extension.
pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Find ffmpeg: `--ffmpeg`, then `$REHUE_FFMPEG`, then `$PATH`.
pub fn resolve_ffmpeg(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        if !path.is_file() {
            return Err(format!("--ffmpeg {}: not a file", path.display()));
        }
        return Ok(path.to_path_buf());
    }
    if let Some(value) = std::env::var_os("REHUE_FFMPEG") {
        let path = PathBuf::from(value);
        if !path.is_file() {
            return Err(format!("$REHUE_FFMPEG {}: not a file", path.display()));
        }
        return Ok(path);
    }
    on_path(FFMPEG).ok_or_else(|| {
        format!(
            "{FFMPEG} not found on $PATH (or $REHUE_FFMPEG / --ffmpeg); \
             nix: nix profile install nixpkgs#ffmpeg-headless; others: https://ffmpeg.org/download.html"
        )
    })
}

fn on_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// ffprobe: beside ffmpeg (they ship together), else on `$PATH`.
fn resolve_ffprobe(ffmpeg: &Path) -> Result<PathBuf, String> {
    let beside = ffmpeg.with_file_name(FFPROBE);
    if beside.is_file() {
        return Ok(beside);
    }
    on_path(FFPROBE).ok_or_else(|| {
        format!(
            "{FFPROBE} not found beside {} or on $PATH",
            ffmpeg.display()
        )
    })
}

/// What rehue needs to know about a video before reading it.
#[derive(Debug, Clone)]
pub struct Probe {
    /// Frame size as decoded (after the stream's rotation).
    pub width: u32,
    pub height: u32,
    /// Frame rate as ffmpeg spells it (`30000/1001`).
    pub fps: String,
    /// Seconds.
    pub duration: f64,
    /// The color matrix frames are read through (`bt709`, `bt601`, ...).
    pub matrix: String,
    /// `tv` (limited) or `pc` (full).
    pub range: String,
    /// The first audio stream's codec, if there is audio.
    pub audio: Option<String>,
}

impl Probe {
    /// The frame rate as a number.
    pub fn fps_value(&self) -> f64 {
        match self.fps.split_once('/') {
            Some((n, d)) => {
                let (n, d) = (
                    n.parse::<f64>().unwrap_or(0.0),
                    d.parse::<f64>().unwrap_or(1.0),
                );
                if d == 0.0 { 0.0 } else { n / d }
            }
            None => self.fps.parse().unwrap_or(0.0),
        }
    }

    fn frame_len(&self) -> usize {
        self.width as usize * self.height as usize * 3
    }
}

/// Run a command to completion; its stdout, or an error with its stderr.
fn output(cmd: &mut Command, what: &str) -> Result<Vec<u8>, String> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("can't run {what}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

/// Probe `path`'s first video stream (and whether it has audio).
pub fn probe(ffmpeg: &Path, path: &Path) -> Result<Probe, String> {
    let ffprobe = resolve_ffprobe(ffmpeg)?;
    let text = output(
        Command::new(&ffprobe)
            .args(["-v", "error", "-of", "json"])
            .args(["-show_entries", "stream=index,codec_type,codec_name,width,height,r_frame_rate,color_space,color_range:stream_side_data=rotation:format=duration"])
            .arg(path),
        "ffprobe",
    )?;
    let json: serde_json::Value = serde_json::from_slice(&text)
        .map_err(|e| format!("ffprobe gave unreadable output: {e}"))?;
    let streams = json["streams"].as_array().cloned().unwrap_or_default();
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or_else(|| format!("{} has no video stream", path.display()))?;
    let audio = streams
        .iter()
        .find(|s| s["codec_type"] == "audio")
        .and_then(|s| s["codec_name"].as_str())
        .map(str::to_string);
    let (mut width, mut height) = (
        video["width"].as_u64().unwrap_or(0) as u32,
        video["height"].as_u64().unwrap_or(0) as u32,
    );
    if width == 0 || height == 0 {
        return Err(format!("{}: can't read the frame size", path.display()));
    }
    let rotation = video["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|d| d["rotation"].as_i64())
        .unwrap_or(0);
    if rotation.rem_euclid(180) == 90 {
        std::mem::swap(&mut width, &mut height);
    }
    let tag = |key: &str| {
        video[key]
            .as_str()
            .filter(|v| !v.is_empty() && *v != "unknown")
            .map(str::to_string)
    };
    let matrix = tag("color_space")
        .map(|m| match m.as_str() {
            // the scale filter's names for the common matrices
            "bt470bg" | "smpte170m" => "bt601".to_string(),
            "bt2020nc" | "bt2020c" => "bt2020".to_string(),
            _ => m,
        })
        .unwrap_or_else(|| if height >= 720 { "bt709" } else { "bt601" }.to_string());
    let range = tag("color_range").unwrap_or_else(|| "tv".to_string());
    let duration = json["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);
    Ok(Probe {
        width,
        height,
        fps: video["r_frame_rate"].as_str().unwrap_or("0/1").to_string(),
        duration,
        matrix,
        range,
        audio,
    })
}

/// ffmpeg's deband filter as `--deband` runs it: smooths the blocky
/// steps compression leaves in gradients, before the grade makes them
/// show.
const DEBAND: &str = "deband=1thr=0.015:2thr=0.015:3thr=0.015:range=16:blur=1";

/// The decoding filter: optionally deband, then the stream's matrix and
/// range in, RGB out.
fn read_filter(probe: &Probe, size: Option<(u32, u32)>, deband: bool) -> String {
    let scale = match size {
        Some((w, h)) => format!("w={w}:h={h}:"),
        None => String::new(),
    };
    let deband = if deband {
        format!("{DEBAND},")
    } else {
        String::new()
    };
    format!(
        "{deband}scale={scale}in_color_matrix={}:in_range={}:flags={SWS_FLAGS},format=rgb24",
        probe.matrix, probe.range
    )
}

/// The size each sampled frame is scaled to.
pub fn sample_size(probe: &Probe) -> (u32, u32) {
    let w = SAMPLE_WIDTH.min(probe.width);
    let h = ((u64::from(w) * u64::from(probe.height) / u64::from(probe.width)) as u32).max(1);
    (w, h)
}

/// Up to [`SAMPLE_FRAMES`] frames spread evenly over the video, scaled
/// to [`SAMPLE_WIDTH`] across and stacked top to bottom: what the grade
/// is measured on.
pub fn sample(ffmpeg: &Path, path: &Path, probe: &Probe, deband: bool) -> Result<RgbImage, String> {
    let (w, h) = sample_size(probe);
    let rate = if probe.duration > 0.0 {
        format!("fps={}/{:.6},", SAMPLE_FRAMES, probe.duration)
    } else {
        String::new()
    };
    let raw = output(
        Command::new(ffmpeg)
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args(["-map", "0:v:0", "-vf"])
            .arg(format!(
                "{rate}{}",
                read_filter(probe, Some((w, h)), deband)
            ))
            .args(["-frames:v", &SAMPLE_FRAMES.to_string()])
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]),
        "ffmpeg (sampling frames)",
    )?;
    let frame = w as usize * h as usize * 3;
    let frames = raw.len() / frame;
    if frames == 0 {
        return Err(format!("{}: no frames to read", path.display()));
    }
    let mut raw = raw;
    raw.truncate(frames * frame);
    Ok(RgbImage::from_raw(w, h * frames as u32, raw).expect("whole frames"))
}

/// A child's stderr collected on a thread, so a full pipe never stalls it.
fn collect_stderr(child: &mut Child) -> JoinHandle<String> {
    let mut stderr = child.stderr.take().expect("stderr is piped");
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    })
}

/// Frames read from a video, one RGB buffer at a time.
pub struct Decoder {
    child: Child,
    stdout: ChildStdout,
    stderr: Option<JoinHandle<String>>,
    frame_len: usize,
}

impl Decoder {
    /// `deband`: run ffmpeg's deband filter on the frames first.
    pub fn open(ffmpeg: &Path, path: &Path, probe: &Probe, deband: bool) -> Result<Self, String> {
        let mut child = Command::new(ffmpeg)
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-vf"])
            .arg(read_filter(probe, None, deband))
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("can't run ffmpeg: {e}"))?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = Some(collect_stderr(&mut child));
        Ok(Decoder {
            child,
            stdout,
            stderr,
            frame_len: probe.frame_len(),
        })
    }

    /// Fill `frame` with the next frame; `false` at the end.
    pub fn next(&mut self, frame: &mut Vec<u8>) -> Result<bool, String> {
        frame.resize(self.frame_len, 0);
        let mut filled = 0;
        while filled < self.frame_len {
            match self.stdout.read(&mut frame[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("reading frames from ffmpeg: {e}")),
            }
        }
        if filled == 0 {
            self.finish()?;
            return Ok(false);
        }
        if filled < self.frame_len {
            return Err(format!(
                "ffmpeg sent a partial frame ({filled} of {} bytes)",
                self.frame_len
            ));
        }
        Ok(true)
    }

    fn finish(&mut self) -> Result<(), String> {
        let status = self
            .child
            .wait()
            .map_err(|e| format!("waiting for ffmpeg: {e}"))?;
        let stderr = self.stderr.take().map(|h| h.join().unwrap_or_default());
        if !status.success() {
            return Err(format!(
                "ffmpeg (decoding) failed: {}",
                stderr.unwrap_or_default().trim()
            ));
        }
        Ok(())
    }
}

/// Frames written into a new video, with the source's audio.
pub struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: Option<JoinHandle<String>>,
}

impl Encoder {
    /// Encode `probe`-sized frames into `out`; the container (and so the
    /// codec) follows `out`'s extension, and the audio comes from
    /// `source`.
    pub fn open(ffmpeg: &Path, source: &Path, out: &Path, probe: &Probe) -> Result<Self, String> {
        let ext = out
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-v", "error", "-nostdin", "-y"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24"])
            .args(["-video_size", &format!("{}x{}", probe.width, probe.height)])
            .args(["-framerate", &probe.fps, "-i", "-", "-i"])
            .arg(source)
            .args(["-map", "0:v:0", "-map", "1:a:0?", "-map_metadata", "-1"])
            .arg("-vf")
            .arg(format!(
                "scale=out_color_matrix=bt709:out_range=tv:flags={SWS_FLAGS},format=yuv420p"
            ))
            .args(["-colorspace", "bt709", "-color_primaries", "bt709"])
            .args(["-color_trc", "bt709", "-color_range", "tv"]);
        if ext == "webm" {
            cmd.args([
                "-c:v",
                "libvpx-vp9",
                "-crf",
                "20",
                "-b:v",
                "0",
                "-row-mt",
                "1",
            ]);
            match probe.audio.as_deref() {
                Some("opus" | "vorbis") => cmd.args(["-c:a", "copy"]),
                _ => cmd.args(["-c:a", "libopus", "-b:a", "160k"]),
            };
        } else {
            cmd.args(["-c:v", "libx264", "-preset", "slow", "-crf", "16"])
                .args(["-c:a", "copy"]);
            if ext == "mp4" || ext == "mov" {
                cmd.args(["-movflags", "+faststart"]);
            }
        }
        let mut child = cmd
            .arg(out)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("can't run ffmpeg: {e}"))?;
        let stdin = child.stdin.take();
        let stderr = Some(collect_stderr(&mut child));
        Ok(Encoder {
            child,
            stdin,
            stderr,
        })
    }

    pub fn write(&mut self, frame: &[u8]) -> Result<(), String> {
        let stdin = self.stdin.as_mut().expect("open until finish");
        if stdin.write_all(frame).is_err() {
            // ffmpeg stopped reading: its own error says why
            return Err(self.failure());
        }
        Ok(())
    }

    /// Close the input and wait for the file to be written.
    pub fn finish(mut self) -> Result<(), String> {
        drop(self.stdin.take());
        let status = self
            .child
            .wait()
            .map_err(|e| format!("waiting for ffmpeg: {e}"))?;
        if !status.success() {
            return Err(self.failure());
        }
        Ok(())
    }

    fn failure(&mut self) -> String {
        drop(self.stdin.take());
        let _ = self.child.wait();
        let stderr = self.stderr.take().map(|h| h.join().unwrap_or_default());
        format!(
            "ffmpeg (encoding) failed: {}",
            stderr.unwrap_or_default().trim()
        )
    }
}
