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

/// Up to [`SAMPLE_FRAMES`] frames spread evenly over the video (or over
/// `range`, its start and length in seconds), scaled to
/// [`SAMPLE_WIDTH`] across and stacked top to bottom: what the grade is
/// measured on.
pub fn sample(
    ffmpeg: &Path,
    path: &Path,
    probe: &Probe,
    deband: bool,
    range: Option<(f64, f64)>,
) -> Result<RgbImage, String> {
    let (w, h) = sample_size(probe);
    let length = range.map_or(probe.duration, |(_, len)| len);
    let rate = if length > 0.0 {
        format!("fps={}/{:.6},", SAMPLE_FRAMES, length)
    } else {
        String::new()
    };
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", "error", "-nostdin"]);
    if let Some((start, len)) = range {
        cmd.args(["-ss", &format!("{start:.6}"), "-t", &format!("{len:.6}")]);
    }
    let raw = output(
        cmd.arg("-i")
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

/// The loop pre-pass: every `stride`th frame at the scan size, in order
/// (with a stride over 1, each followed by the frame after it, which
/// measures one frame's motion), handed to `each`.
pub fn scan(
    ffmpeg: &Path,
    path: &Path,
    probe: &Probe,
    stride: usize,
    mut each: impl FnMut(&[u8]),
) -> Result<(), String> {
    let (w, h) = (crate::looping::SCAN_WIDTH, crate::looping::SCAN_HEIGHT);
    let select = if stride > 1 {
        format!("select=not(mod(n\\,{stride}))+not(mod(n-1\\,{stride})),")
    } else {
        String::new()
    };
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-vf"])
        .arg(format!(
            "{select}{}",
            read_filter(probe, Some((w, h)), false)
        ))
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]);
    let mut decoder = Decoder::spawn(cmd, w as usize * h as usize * 3, "scanning for a loop")?;
    let mut frame = Vec::new();
    while decoder.next(&mut frame)? {
        each(&frame);
    }
    Ok(())
}

/// Where to seek for source frame `frame`: half a frame early, so the
/// frame itself is the first kept whatever the rounding.
fn seek_to(frame: usize, probe: &Probe) -> Option<String> {
    (frame > 0).then(|| format!("{:.6}", (frame as f64 - 0.5) / probe.fps_value()))
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
    /// What it's for, in its errors.
    what: &'static str,
}

impl Decoder {
    /// `deband`: run ffmpeg's deband filter on the frames first.
    /// `range`: only `count` frames from source frame `start`.
    pub fn open(
        ffmpeg: &Path,
        path: &Path,
        probe: &Probe,
        deband: bool,
        range: Option<(usize, usize)>,
    ) -> Result<Self, String> {
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-v", "error", "-nostdin"]);
        if let Some(t) = range.and_then(|(start, _)| seek_to(start, probe)) {
            cmd.args(["-ss", &t]);
        }
        cmd.arg("-i")
            .arg(path)
            .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-vf"])
            .arg(read_filter(probe, None, deband));
        if let Some((_, count)) = range {
            cmd.args(["-frames:v", &count.to_string()]);
        }
        cmd.args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]);
        Self::spawn(cmd, probe.frame_len(), "decoding")
    }

    /// A loop's bridge: `frames` frames ffmpeg's motion interpolation
    /// draws from source frame `from` to source frame `to` (neither
    /// included).
    pub fn bridge(
        ffmpeg: &Path,
        path: &Path,
        probe: &Probe,
        deband: bool,
        from: usize,
        to: usize,
        frames: usize,
    ) -> Result<Self, String> {
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-v", "error", "-nostdin"]);
        for frame in [from, to] {
            if let Some(t) = seek_to(frame, probe) {
                cmd.args(["-ss", &t]);
            }
            cmd.arg("-i").arg(path);
        }
        // the two frames one second apart; at `frames + 1` fps the
        // interpolation fills the second between them
        let steps = frames + 1;
        cmd.arg("-filter_complex")
            .arg(format!(
                "[0:v]trim=end_frame=1,setpts=PTS-STARTPTS[p];[1:v]trim=end_frame=1,setpts=PTS-STARTPTS[q];\
                 [p][q]concat=n=2:v=1,settb=1,setpts=N,tpad=stop=2:stop_mode=clone,\
                 minterpolate=fps={steps}:mi_mode=mci:mc_mode=aobmc:me_mode=bidir:vsbmc=1,\
                 trim=start_frame=1:end_frame={steps},{}[v]",
                read_filter(probe, None, deband)
            ))
            .args(["-map", "[v]", "-fps_mode", "passthrough"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]);
        Self::spawn(cmd, probe.frame_len(), "bridging the loop")
    }

    fn spawn(mut cmd: Command, frame_len: usize, what: &'static str) -> Result<Self, String> {
        let mut child = cmd
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
            frame_len,
            what,
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
                "ffmpeg ({}) failed: {}",
                self.what,
                stderr.unwrap_or_default().trim()
            ));
        }
        Ok(())
    }
}

/// A video's frames in playing order: all of them, or a loop's.
pub struct Frames<'a> {
    ffmpeg: &'a Path,
    path: &'a Path,
    probe: &'a Probe,
    deband: bool,
    plan: Option<crate::looping::Plan>,
    main: Decoder,
    /// Frames `main` has given.
    read: usize,
    /// A fade's head, opened when its tail starts.
    head: Option<Decoder>,
    head_frame: Vec<u8>,
    /// A bridge, once `main` ends.
    bridge: Option<Decoder>,
}

impl<'a> Frames<'a> {
    pub fn open(
        ffmpeg: &'a Path,
        path: &'a Path,
        probe: &'a Probe,
        deband: bool,
        plan: Option<crate::looping::Plan>,
    ) -> Result<Self, String> {
        use crate::looping::Method;
        let range = plan.map(|p| match p.method {
            // the body, then the tail the head fades in over
            Method::Fade => (p.start + p.seam, p.end - p.start - p.seam),
            Method::Bridge | Method::Trim => (p.start, p.end - p.start),
        });
        Ok(Frames {
            ffmpeg,
            path,
            probe,
            deband,
            plan,
            main: Decoder::open(ffmpeg, path, probe, deband, range)?,
            read: 0,
            head: None,
            head_frame: Vec::new(),
            bridge: None,
        })
    }

    /// Fill `frame` with the next frame; `false` at the end.
    pub fn next(&mut self, frame: &mut Vec<u8>) -> Result<bool, String> {
        use crate::looping::Method;
        if let Some(bridge) = &mut self.bridge {
            return bridge.next(frame);
        }
        if !self.main.next(frame)? {
            return match self.plan {
                Some(p) if p.method == Method::Bridge => {
                    let mut bridge = Decoder::bridge(
                        self.ffmpeg,
                        self.path,
                        self.probe,
                        self.deband,
                        p.end - 1,
                        p.start,
                        p.seam - 1,
                    )?;
                    let more = bridge.next(frame)?;
                    self.bridge = Some(bridge);
                    Ok(more)
                }
                Some(p) if self.read < p.frames().min(p.end - p.start) => Err(format!(
                    "the clip ended {} frames before the loop's cut",
                    p.end - p.start - self.read - if p.method == Method::Fade { p.seam } else { 0 }
                )),
                _ => Ok(false),
            };
        }
        self.read += 1;
        if let Some(p) = self.plan.filter(|p| p.method == Method::Fade) {
            let body = p.end - p.start - 2 * p.seam;
            if self.read > body {
                // tail frame k of the fade, over head frame k: the last
                // is the head's alone, so the loop's first frame follows
                let k = self.read - body - 1;
                if self.head.is_none() {
                    self.head = Some(Decoder::open(
                        self.ffmpeg,
                        self.path,
                        self.probe,
                        self.deband,
                        Some((p.start, p.seam)),
                    )?);
                }
                let head = self.head.as_mut().expect("opened above");
                if !head.next(&mut self.head_frame)? {
                    return Err("ffmpeg ran out of frames for the loop's fade".into());
                }
                let (wh, n) = (k as u32 + 1, p.seam as u32);
                for (t, h) in frame.iter_mut().zip(&self.head_frame) {
                    *t = ((u32::from(*t) * (n - wh) + u32::from(*h) * wh + n / 2) / n) as u8;
                }
            }
        }
        Ok(true)
    }
}

/// Flags `--ffmpeg-args` can't take: rehue owns the input, the stream
/// mapping, the filters (the grade comes before any), the length and
/// the container.
const REFUSED: [&str; 14] = [
    "-i",
    "-map",
    "-vf",
    "-filter",
    "-filter_complex",
    "-lavfi",
    "-ss",
    "-t",
    "-to",
    "-frames",
    "-vframes",
    "-f",
    "-y",
    "-n",
];

/// Encoding flags for ffmpeg (`--ffmpeg-args`), in place of rehue's
/// codec defaults.  A `-pix_fmt` is taken out and used by rehue's own
/// conversion, so `yuv444p` (or 10-bit) is reached in one step.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct EncodeArgs {
    pub pix_fmt: Option<String>,
    pub args: Vec<String>,
}

impl EncodeArgs {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut words = split_args(text)?.into_iter();
        let mut parsed = EncodeArgs::default();
        while let Some(word) = words.next() {
            let name = word.split(':').next().unwrap_or_default();
            if REFUSED.contains(&name) {
                return Err(format!(
                    "--ffmpeg-args can't take {word}: rehue sets the input, mapping, filters, length and container"
                ));
            }
            if name == "-pix_fmt" {
                let value = words
                    .next()
                    .ok_or_else(|| format!("--ffmpeg-args: {word} needs a value"))?;
                parsed.pix_fmt = Some(value);
                continue;
            }
            parsed.args.push(word);
        }
        Ok(parsed)
    }
}

/// How a dithered video is encoded when `--ffmpeg-args` doesn't say:
/// 4:4:4, so the dots keep their colors (4:2:0 halves the color
/// resolution and smears them), with the first encoder this ffmpeg
/// has that the container takes.  H.264 first: it plays back on far
/// less CPU than AV1 (graphics hardware doesn't decode 4:4:4, so the
/// player's CPU does) and encodes about five times faster, at nearly
/// the same dots.  `(encoder, settings)`.
const DITHER_LADDER: [(&str, &str); 3] = [
    ("libx264", "-c:v libx264 -preset slow -crf 16"),
    ("libvpx-vp9", "-c:v libvpx-vp9 -crf 20 -b:v 0 -row-mt 1"),
    (
        "libaom-av1",
        "-c:v libaom-av1 -cpu-used 6 -row-mt 1 -crf 20 -b:v 0",
    ),
];

/// The encoders `ffmpeg` was built with.
pub fn encoders(ffmpeg: &Path) -> Result<Vec<String>, String> {
    let out = output(
        Command::new(ffmpeg).args(["-hide_banner", "-encoders"]),
        "ffmpeg (listing encoders)",
    )?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let flags = words.next()?;
            // " V....D libx264 ..." rows; the legend's rows have "="
            (flags.starts_with('V') && !line.contains('=')).then(|| words.next())?
        })
        .map(str::to_string)
        .collect())
}

/// The encoding for a dithered video written to `out`: the first rung
/// of [`DITHER_LADDER`] that `available` has and `out`'s container
/// takes (x264 doesn't go in .webm, VP9 only goes there, AV1 not in
/// .mov), and the rungs passed over for lack of an encoder.
pub fn dither_encoding(
    out: &Path,
    available: &[String],
) -> (EncodeArgs, String, Vec<&'static str>) {
    let ext = out
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let fits = |encoder: &str| match encoder {
        "libaom-av1" => ext != "mov",
        "libx264" => ext != "webm",
        _ => ext == "webm",
    };
    let mut missing = Vec::new();
    let mut fallback = None;
    for (encoder, settings) in DITHER_LADDER.iter().filter(|(e, _)| fits(e)) {
        fallback.get_or_insert((encoder, settings));
        if available.iter().any(|a| a == encoder) {
            return (dither_args(settings), encoder.to_string(), missing);
        }
        missing.push(*encoder);
    }
    // none found: the best one, so ffmpeg says what's missing
    let (encoder, settings) = fallback.expect("every container has a rung");
    (dither_args(settings), encoder.to_string(), missing)
}

fn dither_args(settings: &str) -> EncodeArgs {
    EncodeArgs {
        pix_fmt: Some("yuv444p".to_string()),
        args: split_args(settings).expect("ladder settings split"),
    }
}

/// Split `text` into words the way a shell would: on whitespace, with
/// 'single' quotes taken literally, "double" quotes and backslashes
/// escaping.
pub fn split_args(text: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => words.extend(word.take()),
            '\'' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => w.push(c),
                        None => return Err(format!("unclosed ' in: {text}")),
                    }
                }
            }
            '"' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c) => w.push(c),
                            None => return Err(format!("unclosed \" in: {text}")),
                        },
                        Some(c) => w.push(c),
                        None => return Err(format!("unclosed \" in: {text}")),
                    }
                }
            }
            '\\' => {
                if let Some(c) = chars.next() {
                    word.get_or_insert_with(String::new).push(c);
                }
            }
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    Ok(words)
}

/// Frames written into a new video, with the source's audio.
pub struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: Option<JoinHandle<String>>,
}

impl Encoder {
    /// Encode `probe`-sized frames into `out`; the container (and so the
    /// default codec) follows `out`'s extension, the audio comes from
    /// `source` (through `audio`, a filter graph from `[1:a]` to `[a]`,
    /// when given), and `encode` replaces the video codec settings.
    pub fn open(
        ffmpeg: &Path,
        source: &Path,
        out: &Path,
        probe: &Probe,
        encode: &EncodeArgs,
        audio: Option<&str>,
    ) -> Result<Self, String> {
        let custom = !encode.args.is_empty();
        let pix_fmt = encode.pix_fmt.as_deref().unwrap_or("yuv420p");
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
            .args(["-map", "0:v:0"]);
        match audio {
            Some(graph) => cmd.args(["-filter_complex", graph, "-map", "[a]"]),
            None => cmd.args(["-map", "1:a:0?"]),
        };
        cmd.args(["-map_metadata", "-1"])
            .arg("-vf")
            .arg(format!(
                "scale=out_color_matrix=bt709:out_range=tv:flags={SWS_FLAGS},format={pix_fmt}"
            ))
            .args(["-colorspace", "bt709", "-color_primaries", "bt709"])
            .args(["-color_trc", "bt709", "-color_range", "tv"]);
        if ext == "webm" {
            if !custom {
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
            }
            match probe.audio.as_deref() {
                Some("opus" | "vorbis") if audio.is_none() => cmd.args(["-c:a", "copy"]),
                _ => cmd.args(["-c:a", "libopus", "-b:a", "160k"]),
            };
        } else {
            if !custom {
                cmd.args(["-c:v", "libx264", "-preset", "slow", "-crf", "16"]);
            }
            // filtered sound can't be copied
            match audio {
                Some(_) => cmd.args(["-c:a", "aac", "-b:a", "192k"]),
                None => cmd.args(["-c:a", "copy"]),
            };
            if ext == "mp4" || ext == "mov" {
                cmd.args(["-movflags", "+faststart"]);
            }
        }
        // after rehue's own, so an audio flag here wins too
        cmd.args(&encode.args);
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
