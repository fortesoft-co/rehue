//! Seamless loops: a video cut so its end flows back into its start.
//!
//! A loop plays the source from a start frame to a cut, then wraps.  How
//! the wrap is hidden is its method:
//!
//! - `fade`: the last `seam` frames before the cut are crossfaded into
//!   the first `seam` frames from the start, and the loop begins just
//!   after those, so the fade's last frame hands over to the loop's first
//!   like any two neighbouring frames.  The loop is `seam` frames shorter
//!   than start to cut.
//! - `bridge`: ffmpeg's `minterpolate` draws `seam - 1` frames from the
//!   frame before the cut to the start frame.
//! - `trim`: a hard cut, for a clip that already loops.
//!
//! Where to cut is found on a pre-pass at 160x90.  A fade's cost is the
//! mean squared OKLab distance (lightness counted double) between the
//! frames it blends, weighted to the middle of the fade where both show
//! most; the start and the cut are both searched.  A trim's cost is how
//! far the start frame is from the frame that would have come next.
//! Scores are reported against the clip's own motion: 0 is seamless, 1
//! is as big a change as one frame to the next.

use std::fmt;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::color::{OKLAB_M1, OKLAB_M2, srgb_to_linear};

/// The pre-pass frame size.
pub const SCAN_WIDTH: u32 = 160;
pub const SCAN_HEIGHT: u32 = 90;
const PIXELS: usize = (SCAN_WIDTH * SCAN_HEIGHT) as usize;
const DIMS: usize = PIXELS * 3;

/// At most this many frames are scored; a longer clip is scored on every
/// nth frame, so its cut lands within a few frames.
pub const MAX_SCAN_FRAMES: usize = 2000;

/// OKLab is kept as fixed point (lightness doubled, so 0..4096).
const FIXED: f32 = 2048.0;

/// The loop lengths searched, as shares of the clip: the best seam at any
/// length (a quarter of the clip at least), at half and at three-quarters.
const FLOORS: [f64; 3] = [0.25, 0.5, 0.75];

/// A trim is offered when its join scores at most this: the clip
/// already loops there.
const TRIM_MAX: f64 = 0.5;

/// A best seam scoring above this is warned about: it will show.
pub const WARN_ABOVE: f64 = 10.0;

/// How the wrap is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Fade,
    Bridge,
    Trim,
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Method::Fade => "fade",
            Method::Bridge => "bridge",
            Method::Trim => "trim",
        })
    }
}

/// What `--loop` was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The best-scoring candidate (a trim when the clip already loops,
    /// else a fade).
    Auto,
    /// The candidates, numbered, to pick from.
    Ask,
    /// This method, its best cut.
    Only(Method),
}

/// The shortest loop to accept.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Min {
    Seconds(f64),
    /// A share of the clip, 0..1.
    Share(f64),
}

/// `--loop MODE[,key=value...]`; in a config, that string or a table
/// (`mode`, `seam`, `min`, `start`, `end`).
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(try_from = "Value", into = "Value")]
pub struct LoopOptions {
    pub mode: Mode,
    /// Seconds the fade or bridge lasts.
    pub seam: f64,
    pub min: Option<Min>,
    /// Seconds into the source.
    pub start: Option<f64>,
    pub end: Option<f64>,
}

impl Default for LoopOptions {
    fn default() -> Self {
        LoopOptions {
            mode: Mode::Auto,
            seam: 1.0,
            min: None,
            start: None,
            end: None,
        }
    }
}

const KEYS: &str = "seam, min, start, end";

impl LoopOptions {
    /// Parse the flag form: `MODE[,key=value...]`, a bare value being the
    /// mode.  An empty string is `auto`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut table = Map::new();
        for (i, part) in text.split(',').map(str::trim).enumerate() {
            if part.is_empty() {
                continue;
            }
            match part.split_once('=') {
                None if i == 0 => {
                    table.insert("mode".into(), Value::String(part.into()));
                }
                None => return Err(format!("loop '{part}': settings are key=value")),
                Some((k, v)) => {
                    table.insert(k.trim().into(), Value::String(v.trim().into()));
                }
            }
        }
        let mut options = LoopOptions::default();
        for (key, value) in &table {
            let text = value.as_str().unwrap_or_default();
            let seconds = |what: &str| {
                text.parse::<f64>()
                    .ok()
                    .filter(|s| s.is_finite() && *s >= 0.0)
                    .ok_or_else(|| format!("loop {what} is seconds, not '{text}'"))
            };
            match key.as_str() {
                "mode" => {
                    options.mode = match text {
                        "auto" => Mode::Auto,
                        "ask" => Mode::Ask,
                        "fade" => Mode::Only(Method::Fade),
                        "bridge" => Mode::Only(Method::Bridge),
                        "trim" => Mode::Only(Method::Trim),
                        _ => {
                            return Err(format!(
                                "no loop mode '{text}' (modes: auto, ask, fade, bridge, trim)"
                            ));
                        }
                    }
                }
                "seam" => {
                    let s = seconds("seam")?;
                    if s <= 0.0 {
                        return Err("loop seam is more than 0 seconds".into());
                    }
                    options.seam = s;
                }
                "min" => {
                    options.min = Some(match text.strip_suffix('%') {
                        Some(pct) => {
                            let p = pct
                                .trim()
                                .parse::<f64>()
                                .ok()
                                .filter(|p| *p > 0.0 && *p <= 100.0);
                            Min::Share(
                                p.ok_or_else(|| {
                                    format!("loop min is seconds or 1..100%, not '{text}'")
                                })? / 100.0,
                            )
                        }
                        None => Min::Seconds(seconds("min")?),
                    })
                }
                "start" => options.start = Some(seconds("start")?),
                "end" => options.end = Some(seconds("end")?),
                _ => return Err(format!("loop has no setting '{key}' (it takes {KEYS})")),
            }
        }
        if let (Some(start), Some(end)) = (options.start, options.end)
            && end <= start
        {
            return Err(format!("loop end ({end}) comes after start ({start})"));
        }
        Ok(options)
    }

    /// The mode's name.
    pub fn mode_name(&self) -> String {
        match self.mode {
            Mode::Auto => "auto".into(),
            Mode::Ask => "ask".into(),
            Mode::Only(m) => m.to_string(),
        }
    }

    /// Whether everything is given, so there's nothing to search.
    pub fn pinned(&self) -> bool {
        matches!(self.mode, Mode::Only(_)) && self.start.is_some() && self.end.is_some()
    }
}

impl TryFrom<Value> for LoopOptions {
    type Error = String;

    /// The flag's string, `true` (auto), or a table of the same settings.
    fn try_from(value: Value) -> Result<Self, String> {
        match value {
            Value::String(s) => Self::parse(&s),
            Value::Bool(true) => Ok(Self::default()),
            Value::Object(table) => {
                let mut parts = vec![
                    table
                        .get("mode")
                        .map(|m| m.as_str().map(str::to_string).ok_or("loop mode is a name"))
                        .transpose()?
                        .unwrap_or_else(|| "auto".into()),
                ];
                for (key, v) in &table {
                    if key == "mode" {
                        continue;
                    }
                    let text = match v {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        _ => return Err(format!("loop {key} is a number or text, not {v}")),
                    };
                    parts.push(format!("{key}={text}"));
                }
                Self::parse(&parts.join(","))
            }
            _ => Err("loop is a mode (\"fade,min=50%\"), true, or a table of settings".into()),
        }
    }
}

impl From<LoopOptions> for Value {
    fn from(o: LoopOptions) -> Value {
        let number = |x: f64| serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number);
        serde_json::json!({
            "mode": o.mode_name(),
            "seam": number(o.seam),
            "min": match o.min {
                None => Value::Null,
                Some(Min::Seconds(s)) => number(s),
                Some(Min::Share(p)) => Value::String(format!("{}%", p * 100.0)),
            },
            "start": o.start.map_or(Value::Null, number),
            "end": o.end.map_or(Value::Null, number),
        })
    }
}

/// One way to loop a clip, in source frames.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Plan {
    pub method: Method,
    /// The first source frame used.
    pub start: usize,
    /// The cut: the source frame after the last one used.
    pub end: usize,
    /// Frames the fade or bridge lasts.
    pub seam: usize,
}

impl Plan {
    /// Frames the loop runs.
    pub fn frames(&self) -> usize {
        match self.method {
            Method::Fade => self.end - self.start - self.seam,
            Method::Bridge => self.end - self.start + self.seam - 1,
            Method::Trim => self.end - self.start,
        }
    }

    /// The `--loop` flag that makes this plan again.
    pub fn flag(&self, fps: f64, seam_seconds: f64) -> String {
        let mut flag = format!(
            "{},start={},end={}",
            self.method,
            seconds(self.start, fps),
            seconds(self.end, fps)
        );
        if self.method != Method::Trim && seam_seconds != 1.0 {
            flag += &format!(",seam={seam_seconds}");
        }
        flag
    }
}

/// A frame index as seconds, precise enough to round back to it.
pub fn seconds(frame: usize, fps: f64) -> String {
    let s = format!("{:.3}", frame as f64 / fps);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".into() } else { s.into() }
}

/// A plan and how well its seam scored (none for a bridge: the score
/// can't tell whether interpolation will smear).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Candidate {
    #[serde(flatten)]
    pub plan: Plan,
    pub score: Option<f64>,
}

/// The clip, small, for scoring.
pub struct Scan {
    /// Fixed-point OKLab per scanned frame, `DIMS` each.
    frames: Vec<i16>,
    /// Source frames per scanned frame.
    stride: usize,
    /// The median squared distance between neighbouring source frames.
    step: f64,
}

/// A scan being read: frames in decode order.  With a stride over 1,
/// ffmpeg sends pairs (frame n and n+1 for every nth): the first is kept,
/// the second only measures one frame's motion.
pub struct ScanBuilder {
    stride: usize,
    frames: Vec<i16>,
    pending: Option<Vec<i16>>,
    steps: Vec<u64>,
    seen: usize,
}

impl ScanBuilder {
    pub fn new(stride: usize) -> Self {
        ScanBuilder {
            stride: stride.max(1),
            frames: Vec::new(),
            pending: None,
            steps: Vec::new(),
            seen: 0,
        }
    }

    /// The next `SCAN_WIDTH`x`SCAN_HEIGHT` RGB frame.
    pub fn push(&mut self, rgb: &[u8]) {
        let lab = to_lab(rgb);
        let second = self.stride > 1 && self.seen % 2 == 1;
        self.seen += 1;
        if let Some(prev) = &self.pending {
            self.steps.push(distance(prev, &lab));
        }
        if second {
            self.pending = None;
            return;
        }
        self.frames.extend_from_slice(&lab);
        self.pending = Some(lab);
    }

    pub fn finish(mut self) -> Scan {
        self.steps.sort_unstable();
        let step = self
            .steps
            .get(self.steps.len() / 2)
            .copied()
            .unwrap_or(1)
            .max(1);
        Scan {
            frames: self.frames,
            stride: self.stride,
            step: step as f64,
        }
    }
}

/// How many source frames per scanned one, for a clip of `frames`.
pub fn stride_for(frames: usize) -> usize {
    frames.div_ceil(MAX_SCAN_FRAMES).max(1)
}

fn to_lab(rgb: &[u8]) -> Vec<i16> {
    let m1 = OKLAB_M1.map(|r| r.map(|v| v as f32));
    let m2 = OKLAB_M2.map(|r| r.map(|v| v as f32));
    let mut out = Vec::with_capacity(DIMS);
    for px in rgb.as_chunks::<3>().0 {
        let lin = [0, 1, 2].map(|c| srgb_to_linear(px[c]) as f32);
        let lms = m1.map(|r| (r[0] * lin[0] + r[1] * lin[1] + r[2] * lin[2]).cbrt());
        let lab = m2.map(|r| r[0] * lms[0] + r[1] * lms[1] + r[2] * lms[2]);
        out.push((lab[0] * 2.0 * FIXED).round() as i16);
        out.push((lab[1] * FIXED).round() as i16);
        out.push((lab[2] * FIXED).round() as i16);
    }
    out
}

/// Sum of squared differences: exact, so the same on every machine.
fn distance(a: &[i16], b: &[i16]) -> u64 {
    a.chunks(64)
        .zip(b.chunks(64))
        .map(|(a, b)| {
            // 64 squares of at most 4096^2 fit an i32
            a.iter()
                .zip(b)
                .map(|(x, y)| {
                    let d = i32::from(*x) - i32::from(*y);
                    d * d
                })
                .sum::<i32>() as u64
        })
        .sum()
}

impl Scan {
    /// Frames scanned.
    pub fn len(&self) -> usize {
        self.frames.len() / DIMS
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn stride(&self) -> usize {
        self.stride
    }

    fn frame(&self, i: usize) -> &[i16] {
        &self.frames[i * DIMS..(i + 1) * DIMS]
    }
}

/// Distances between scanned frames `i` and `j`, for `i - j >= gap`.
struct Pairs {
    n: usize,
    gap: usize,
    rows: Vec<Vec<u64>>,
}

impl Pairs {
    fn new(scan: &Scan, gap: usize) -> Self {
        let n = scan.len();
        let rows: Vec<usize> = (0..n).collect();
        let rows = crate::par::chunks(&rows, 1, |_, run| {
            run.iter()
                .map(|&i| {
                    (0..(i + 1).saturating_sub(gap))
                        .map(|j| distance(scan.frame(i), scan.frame(j)))
                        .collect::<Vec<u64>>()
                })
                .collect::<Vec<_>>()
        })
        .into_iter()
        .flatten()
        .collect();
        Pairs { n, gap, rows }
    }

    fn get(&self, i: usize, j: usize) -> u64 {
        debug_assert!(i >= j + self.gap && i < self.n);
        self.rows[i][j]
    }
}

/// The candidates for a clip of `total` source frames at `fps`.  With
/// `scan`, cuts are searched and scored; without, `options` must give
/// the start and end.
pub fn candidates(
    scan: Option<&Scan>,
    options: &LoopOptions,
    fps: f64,
    total: usize,
) -> Result<Vec<Candidate>, String> {
    let seam = (options.seam * fps).round() as usize;
    if seam < 2 {
        return Err(format!(
            "loop seam={} is under two frames at {fps:.3} fps",
            options.seam
        ));
    }
    let to_frame = |s: f64| ((s * fps).round() as usize).min(total);
    let start = options.start.map(to_frame);
    let end = options.end.map(to_frame);
    let min_frames = options.min.map(|m| match m {
        Min::Seconds(s) => (s * fps).ceil() as usize,
        Min::Share(p) => (p * total as f64).ceil() as usize,
    });
    if let Some(min) = min_frames
        && min > total
    {
        return Err(format!(
            "loop min is longer than the clip ({})",
            seconds(total, fps)
        ));
    }
    let methods: Vec<Method> = match options.mode {
        Mode::Only(m) => vec![m],
        Mode::Auto | Mode::Ask => vec![Method::Trim, Method::Fade, Method::Bridge],
    };
    let Some(scan) = scan else {
        let (Some(s), Some(e)) = (start, end) else {
            unreachable!("only pinned plans skip the scan")
        };
        let plan = Plan {
            method: methods[0],
            start: s,
            end: e,
            seam,
        };
        check(&plan, fps)?;
        return Ok(vec![Candidate { plan, score: None }]);
    };

    let n = scan.len();
    let stride = scan.stride();
    let sf = (seam as f64 / stride as f64).round().max(1.0) as usize;
    let to_scan = |f: usize| (f as f64 / stride as f64).round() as usize;
    let (fix_s, fix_e) = (start.map(to_scan), end.map(|e| to_scan(e).min(n)));
    // the loop lengths (in scanned frames) to find a best seam at
    let floors: Vec<usize> = if fix_s.is_some() && fix_e.is_some() {
        vec![0]
    } else {
        let min = min_frames.map(to_scan);
        let mut floors: Vec<usize> = FLOORS
            .iter()
            .map(|f| (f * n as f64).ceil() as usize)
            .filter(|f| min.is_none_or(|m| *f > m))
            .collect();
        if let Some(m) = min {
            floors.insert(0, m);
        }
        floors.dedup();
        floors
    };
    let gap = floors[0].max(1);
    if n <= sf + gap {
        return Err(format!(
            "this clip is too short to loop with a {}s seam",
            options.seam
        ));
    }
    let pairs = Pairs::new(scan, gap.saturating_sub(1).min(gap));
    let weights: Vec<f64> = (0..sf)
        .map(|k| {
            let a = (k as f64 + 0.5) / sf as f64;
            a * (1.0 - a)
        })
        .collect();
    let wsum: f64 = weights.iter().sum();
    let starts = |len: usize| -> Vec<usize> {
        match fix_s {
            Some(s) => vec![s],
            None => (0..n.saturating_sub(len).min(n / 2 + 1)).collect(),
        }
    };
    let cuts = |s: usize, len: usize, extra: usize| -> Vec<usize> {
        match fix_e {
            Some(e) => vec![e],
            None => (s + len + extra..=n).collect(),
        }
    };
    let to_source = |s: usize, e: usize| (s * stride, (e * stride).min(total));
    let mut out: Vec<Candidate> = Vec::new();
    let mut push = |c: Candidate| {
        if !out.iter().any(|o| o.plan == c.plan) {
            out.push(c);
        }
    };

    // a trim: the start should be the frame that would have come next
    // (the frame at the cut); at the clip's very end there is none, so
    // the jump from its last frame should be like any other step
    if methods.contains(&Method::Trim) && stride == 1 {
        let explicit = options.mode == Mode::Only(Method::Trim);
        for &len in &floors {
            let mut best: Option<(f64, usize, usize)> = None;
            for s in starts(len) {
                for e in cuts(s, len.max(1), 0) {
                    if e < s + 2 || e - 1 < s + pairs.gap {
                        continue;
                    }
                    let r = if e < n {
                        (pairs.get(e, s) as f64 / scan.step).sqrt()
                    } else {
                        ((pairs.get(e - 1, s) as f64 / scan.step).sqrt() - 1.0).abs()
                    };
                    // ties go to the longer loop
                    let better = best.is_none_or(|(b, bs, be)| {
                        r < b - 1e-9 || (r <= b + 1e-9 && e - s > be - bs)
                    });
                    if better {
                        best = Some((r, s, e));
                    }
                }
            }
            if let Some((r, s, e)) = best
                && (explicit || r <= TRIM_MAX)
            {
                let (s, e) = to_source(s, e);
                push(Candidate {
                    plan: Plan {
                        method: Method::Trim,
                        start: s,
                        end: e,
                        seam,
                    },
                    score: Some(r),
                });
            }
            if !explicit {
                break;
            }
        }
    }
    if methods.contains(&Method::Fade) || methods.contains(&Method::Bridge) {
        for &len in &floors {
            let mut best: Option<(f64, usize, usize)> = None;
            for s in starts(len) {
                for e in cuts(s, len, sf) {
                    if e < s + sf + pairs.gap.max(1) {
                        continue;
                    }
                    let cost: f64 = weights
                        .iter()
                        .enumerate()
                        .map(|(k, w)| w * pairs.get(e - sf + k, s + k) as f64)
                        .sum::<f64>()
                        / wsum;
                    if best.is_none_or(|(b, ..)| cost < b) {
                        best = Some((cost, s, e));
                    }
                }
            }
            let Some((cost, s, e)) = best else { continue };
            let (s, e) = to_source(s, e);
            for method in [Method::Fade, Method::Bridge] {
                if methods.contains(&method) {
                    push(Candidate {
                        plan: Plan {
                            method,
                            start: s,
                            end: e,
                            seam,
                        },
                        score: (method == Method::Fade).then(|| (cost / scan.step).sqrt()),
                    });
                }
            }
        }
    }
    if out.is_empty() {
        return Err(match min_frames {
            Some(_) => "no loop that long fits in this clip; try a lower --loop min".into(),
            None => "no loop fits in this clip".into(),
        });
    }
    for c in &out {
        check(&c.plan, fps)?;
    }
    Ok(out)
}

fn check(plan: &Plan, fps: f64) -> Result<(), String> {
    let room = match plan.method {
        Method::Fade => 2 * plan.seam,
        _ => 2,
    };
    if plan.end < plan.start + room {
        return Err(format!(
            "loop start={} end={} leaves no room for a {}-frame seam",
            seconds(plan.start, fps),
            seconds(plan.end, fps),
            plan.seam
        ));
    }
    Ok(())
}

/// The candidate `auto` takes: a trim when there is one, else the
/// lowest-scoring fade (the first listed), else the first.
pub fn auto_pick(candidates: &[Candidate]) -> usize {
    candidates
        .iter()
        .position(|c| c.plan.method == Method::Trim)
        .or_else(|| {
            candidates
                .iter()
                .enumerate()
                .filter(|(_, c)| c.plan.method == Method::Fade)
                .min_by(|a, b| {
                    a.1.score
                        .partial_cmp(&b.1.score)
                        .expect("scores are numbers")
                })
                .map(|(i, _)| i)
        })
        .unwrap_or(0)
}

/// The audio for a loop, as an ffmpeg filter graph from `[1:a]` to `[a]`,
/// and a note when it can't match the picture exactly.  The audio is
/// crossfaded the same way: a fade or a bridge takes a stretch of sound
/// as long as the loop plus the seam and crossfades its end into its
/// start (equal power, as two unrelated sounds add); a trim gets a
/// moment's fade at each end, so the wrap doesn't click.
pub fn audio_graph(plan: &Plan, fps: f64, duration: f64) -> (String, Option<String>) {
    let t = |f: usize| f as f64 / fps;
    let length = plan.frames() as f64 / fps;
    let head = "[1:a]asetpts=PTS-STARTPTS";
    if plan.method == Method::Trim {
        let edge = 0.01_f64.min(length / 4.0);
        return (
            format!(
                "{head},atrim=start={:.6}:end={:.6},asetpts=PTS-STARTPTS,afade=t=in:d={edge:.6},afade=t=out:st={:.6}:d={edge:.6}[a]",
                t(plan.start),
                t(plan.end),
                length - edge
            ),
            None,
        );
    }
    let fade = |from: f64, d: f64, to: f64| {
        format!(
            "{head},asplit[x][y];[x]atrim=start={:.6}:end={:.6},asetpts=PTS-STARTPTS[p];\
             [y]atrim=start={from:.6}:end={:.6},asetpts=PTS-STARTPTS[q];\
             [p][q]acrossfade=d={d:.6}:c1=qsin:c2=qsin",
            from + d,
            to,
            from + d
        )
    };
    let d = t(plan.seam);
    if plan.method == Method::Fade {
        return (format!("{}[a]", fade(t(plan.start), d, t(plan.end))), None);
    }
    // a bridge: sound can't be interpolated, so a stretch of the real
    // thing, starting with the picture where it fits
    let from = t(plan.start).min(duration - length - d).max(0.0);
    if from + length + d <= duration + 1e-6 {
        return (
            format!("{}[a]", fade(from, d, from + length + d)),
            (from < t(plan.start) - 1e-6).then(|| {
                format!(
                    "the loop's sound starts {:.2}s before its picture, to fit the bridge",
                    t(plan.start) - from
                )
            }),
        );
    }
    let d = d.min(duration / 4.0);
    (
        format!("{},apad=whole_dur={length:.6}[a]", fade(0.0, d, duration)),
        Some(format!(
            "the clip's sound is {:.2}s short of the bridged loop; the rest is silence",
            length - (duration - d)
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white bar 16 columns wide at 4 * `p` across, on black: one
    /// step of `p` changes the same number of pixels anywhere.
    fn bar(p: usize) -> Vec<u8> {
        let mut f = vec![0u8; DIMS];
        for y in 0..SCAN_HEIGHT as usize {
            for x in 4 * p..(4 * p + 16).min(SCAN_WIDTH as usize) {
                let i = (y * SCAN_WIDTH as usize + x) * 3;
                f[i..i + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        f
    }

    fn scan(positions: impl Iterator<Item = usize>) -> Scan {
        let mut b = ScanBuilder::new(1);
        for p in positions {
            b.push(&bar(p));
        }
        b.finish()
    }

    #[test]
    fn options_parse_like_the_other_flags() {
        assert_eq!(LoopOptions::parse("").unwrap(), LoopOptions::default());
        let o = LoopOptions::parse("bridge,seam=1.5,min=50%,start=2,end=9.5").unwrap();
        assert_eq!(o.mode, Mode::Only(Method::Bridge));
        assert_eq!(
            (o.seam, o.min, o.start, o.end),
            (1.5, Some(Min::Share(0.5)), Some(2.0), Some(9.5))
        );
        assert!(o.pinned());
        assert_eq!(
            LoopOptions::parse("fade,min=20").unwrap().min,
            Some(Min::Seconds(20.0))
        );
        for bad in [
            "spin",
            "fade,seam=0",
            "fade,min=150%",
            "fade,x=1",
            "fade,2",
            "fade,start=5,end=3",
        ] {
            assert!(LoopOptions::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_clip_that_returns_to_its_start_is_trimmed() {
        // the bar goes right and back: frame 40 is frame 0 again
        let scan = scan((0..41).map(|i| if i <= 20 { i } else { 40 - i }));
        let c = candidates(Some(&scan), &LoopOptions::default(), 10.0, 41).unwrap();
        let pick = c[auto_pick(&c)];
        assert_eq!(pick.plan.method, Method::Trim);
        assert_eq!((pick.plan.start, pick.plan.end), (0, 40));
        assert_eq!(pick.score, Some(0.0));
    }

    #[test]
    fn a_fade_blends_frames_that_match() {
        // the bar sweeps right every 40 frames, jumping back to the left
        let scan = scan((0..80).map(|i| i % 40));
        let options = LoopOptions {
            mode: Mode::Only(Method::Fade),
            ..LoopOptions::default()
        };
        let c = candidates(Some(&scan), &options, 10.0, 80).unwrap();
        assert!(c.iter().all(|c| c.plan.method == Method::Fade));
        let p = c[0].plan;
        // the fade's frames [end-10, end) are the same as [start, start+10)
        assert_eq!(p.seam, 10);
        assert_eq!(p.end - p.seam - p.start, 40, "{p:?}");
        assert_eq!(c[0].score, Some(0.0));
        assert_eq!(p.frames(), 40);
        // no loop of three-quarters fits a perfect seam
        assert!(
            c.iter()
                .any(|c| c.plan.frames() >= 60 && c.score > Some(0.0))
        );
    }

    #[test]
    fn pinned_plans_need_no_scan_and_print_back_the_same() {
        let o = LoopOptions::parse("fade,start=1.2,end=4,seam=0.5").unwrap();
        let c = candidates(None, &o, 30.0, 300).unwrap();
        assert_eq!(c.len(), 1);
        let p = c[0].plan;
        assert_eq!((p.start, p.end, p.seam), (36, 120, 15));
        assert_eq!(p.flag(30.0, 0.5), "fade,start=1.2,end=4,seam=0.5");
        assert_eq!(LoopOptions::parse(&p.flag(30.0, 0.5)).unwrap(), o);
    }

    #[test]
    fn the_audio_follows_the_loop() {
        let fade = Plan {
            method: Method::Fade,
            start: 30,
            end: 300,
            seam: 30,
        };
        let (g, note) = audio_graph(&fade, 30.0, 10.0);
        assert!(
            g.contains("atrim=start=2.000000:end=10.000000")
                && g.contains("atrim=start=1.000000:end=2.000000"),
            "{g}"
        );
        assert!(note.is_none());
        // a bridge as long as the clip has no spare sound
        let bridge = Plan {
            method: Method::Bridge,
            start: 0,
            end: 300,
            seam: 30,
        };
        let (g, note) = audio_graph(&bridge, 30.0, 10.0);
        assert!(g.contains("apad") && note.is_some(), "{g}");
    }
}
