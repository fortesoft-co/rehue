//! The dither stage: draw an image with palette colors only, as a pattern.
//!
//! Dithering is the last stage of a wal grade.  The image is drawn in
//! `scale` x `scale` blocks (by default about 640 across, the PC-98's
//! width), each block's color the average of what it covers.
//!
//! The ordered styles (`checker`, `halftone`, `pattern`) draw each block
//! with two palette colors.  How far the block sits between them picks,
//! through a position-dependent threshold, which of the two it takes, so
//! an in-between tone comes out as a pattern of the two.  The threshold
//! is a Bayer matrix (dispersed dots), a rotated spot function
//! (clustered dots) or a drawn tile (lines).
//!
//! `checker` takes the nearest color and the nearest on the far side of
//! the block's difference from it, so two colors only mix in a band
//! between them and most of the picture stays solid: the drawn look of
//! 16-color games.  `halftone` and `pattern` take the two colors whose
//! line passes closest to the block, lighter one as paper and darker as
//! ink, and mix across the whole range between them, so dots and lines
//! grow continuously from paper to solid ink, as in print.  `flat` keeps
//! the ends of that range solid.
//!
//! `diffuse` instead takes the nearest color and pushes the difference
//! on to the blocks not yet drawn (Floyd-Steinberg, or Atkinson, which
//! drops a quarter of it).
//!
//! Colors are compared in OKLab.  The output holds palette colors only.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::color::rgb_to_oklch;

/// How the in-between tones are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Style {
    /// Dispersed dots from a small Bayer matrix: the checkerboards of
    /// 16-color games.
    #[default]
    Checker,
    /// Clustered dots growing from the centers of a rotated grid: print.
    Halftone,
    /// Drawn tiles that fill in line by line: hatching.
    Pattern,
    /// Error diffusion: the difference pushed on to the next pixels.
    Diffuse,
}

impl Style {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Checker => "checker",
            Self::Halftone => "halftone",
            Self::Pattern => "pattern",
            Self::Diffuse => "diffuse",
        }
    }

    /// The settings this style takes, besides `style` itself.
    fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Checker => &["scale", "sample", "flat", "size"],
            Self::Halftone => &["scale", "sample", "flat", "size", "angle"],
            Self::Pattern => &["scale", "sample", "flat", "tiles"],
            Self::Diffuse => &["scale", "kernel"],
        }
    }
}

/// How big the pattern's pixels are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scale {
    /// About 640 pattern pixels across, whatever the image's size.
    #[default]
    Auto,
    /// This many image pixels per pattern pixel.
    Fixed(u32),
}

impl Scale {
    /// The block size for an image this wide.
    pub fn resolve(self, width: u32) -> u32 {
        match self {
            Self::Auto => ((f64::from(width) / AUTO_WIDTH).round() as u32).max(1),
            Self::Fixed(n) => n,
        }
    }
}

/// `scale=auto` aims for this many pattern pixels across.
pub const AUTO_WIDTH: f64 = 640.0;

/// Where a pattern pixel's color comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sample {
    /// The block's average: true big pixels (the period look).
    #[default]
    Block,
    /// Each image pixel's own color under the coarse pattern: outlines
    /// stay sharp, only the shading turns to pattern.
    Pixel,
}

impl Sample {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Pixel => "pixel",
        }
    }
}

/// `diffuse`'s kernel: where the difference goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kernel {
    /// All of the difference, to four neighbors: smooth, faithful tones.
    #[default]
    FloydSteinberg,
    /// Three quarters of it, to six neighbors: brighter highlights,
    /// deeper shadows, the high-contrast early-Mac look.
    Atkinson,
}

impl Kernel {
    pub fn describe(self) -> &'static str {
        match self {
            Self::FloydSteinberg => "floyd-steinberg",
            Self::Atkinson => "atkinson",
        }
    }

    /// (dx, dy, share) on a left-to-right row.
    fn taps(self) -> &'static [(i64, i64, f64)] {
        match self {
            Self::FloydSteinberg => &[
                (1, 0, 7.0 / 16.0),
                (-1, 1, 3.0 / 16.0),
                (0, 1, 5.0 / 16.0),
                (1, 1, 1.0 / 16.0),
            ],
            Self::Atkinson => &[
                (1, 0, 1.0 / 8.0),
                (2, 0, 1.0 / 8.0),
                (-1, 1, 1.0 / 8.0),
                (0, 1, 1.0 / 8.0),
                (1, 1, 1.0 / 8.0),
                (0, 2, 1.0 / 8.0),
            ],
        }
    }
}

/// `pattern`'s drawn tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tiles {
    /// Diagonal lines, then crossing ones: crosshatch.
    #[default]
    Hatch,
    /// Horizontal rules, then the rules between them.
    Lines,
}

impl Tiles {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Hatch => "hatch",
            Self::Lines => "lines",
        }
    }
}

/// A dither: a style and its settings.  Written as `checker`, or
/// `halftone,size=8,angle=15` (the `--palette` form); in a config, the
/// same string or a table with `style` and the settings.  A setting the
/// style doesn't take is an error.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Value", into = "Value")]
pub struct Dither {
    pub style: Style,
    /// Pattern pixel size.
    pub scale: Scale,
    /// Where each pattern pixel's color comes from (ordered styles).
    pub sample: Sample,
    /// How much stays solid, 0..1 (ordered styles): 0 patterns wherever
    /// two colors meet, 1 is plain posterized color.  `checker`: two
    /// colors only mix where a block sits a quarter to half of the way
    /// from its nearest color toward the second; `flat` is the share of
    /// that band, from the nearest color's side, drawn solid (default
    /// 0.5: keeps the checkerboards, drops the sparse dots).  `halftone`
    /// and `pattern`: the share of the paper-to-ink range kept solid,
    /// half at each end (default 0.2).
    pub flat: f64,
    /// `checker`: the Bayer matrix, 2, 4 or 8.  `halftone`: the dot
    /// cell, in pattern pixels (default 8).
    pub size: u32,
    /// `halftone`: the screen angle, in degrees.
    pub angle: f64,
    /// `diffuse`: the kernel.
    pub kernel: Kernel,
    /// `pattern`: the tiles.
    pub tiles: Tiles,
}

impl Default for Dither {
    fn default() -> Self {
        Self::for_style(Style::Checker)
    }
}

impl Dither {
    /// A style at its defaults.
    pub fn for_style(style: Style) -> Self {
        // halftone is print: fine dots at full resolution
        let halftone = style == Style::Halftone;
        Self {
            style,
            scale: if halftone {
                Scale::Fixed(1)
            } else {
                Scale::Auto
            },
            sample: Sample::Block,
            flat: if style == Style::Checker { 0.5 } else { 0.2 },
            size: if halftone { 8 } else { 2 },
            angle: 45.0,
            kernel: Kernel::FloydSteinberg,
            tiles: Tiles::Hatch,
        }
    }

    /// Parse the flag form: `STYLE[,key=val...]`, a bare value being the
    /// style.  An empty string is `checker` with its defaults.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut table = Map::new();
        for (i, part) in text.split(',').map(str::trim).enumerate() {
            if part.is_empty() {
                continue;
            }
            match part.split_once('=') {
                None if i == 0 => {
                    table.insert("style".into(), Value::String(part.into()));
                }
                None => return Err(format!("'{part}': settings are key=value")),
                Some((k, v)) => {
                    let v = v.trim();
                    let value = v
                        .parse::<f64>()
                        .ok()
                        .and_then(serde_json::Number::from_f64)
                        .map(Value::Number)
                        .unwrap_or_else(|| Value::String(v.into()));
                    table.insert(k.trim().into(), value);
                }
            }
        }
        Self::try_from(Value::Object(table))
    }

    /// The flag form back, with the settings this style takes.
    pub fn describe(&self) -> String {
        let mut out = self.style.describe().to_string();
        for key in self.style.keys() {
            let value = match *key {
                "scale" => match self.scale {
                    Scale::Auto => "auto".to_string(),
                    Scale::Fixed(n) => n.to_string(),
                },
                "sample" => self.sample.describe().into(),
                "flat" => self.flat.to_string(),
                "size" => self.size.to_string(),
                "angle" => self.angle.to_string(),
                "kernel" => self.kernel.describe().into(),
                "tiles" => self.tiles.describe().into(),
                _ => unreachable!("every style key is described"),
            };
            out.push_str(&format!(",{key}={value}"));
        }
        out
    }
}

impl TryFrom<Value> for Dither {
    type Error = String;

    fn try_from(value: Value) -> Result<Self, String> {
        let table = match value {
            Value::String(s) => return Self::parse(&s),
            Value::Bool(true) => return Ok(Self::default()),
            Value::Object(t) => t,
            _ => return Err("dither is a style name or a table of settings".into()),
        };
        let style = match table.get("style") {
            None => Style::Checker,
            Some(v) => serde_json::from_value(v.clone()).map_err(|_| {
                format!("no dither style {v} (styles: checker, halftone, pattern, diffuse)")
            })?,
        };
        let mut d = Self::for_style(style);
        let number = |k: &str, v: &Value| {
            v.as_f64()
                .ok_or_else(|| format!("dither {k} is a number, not {v}"))
        };
        for (k, v) in &table {
            if k == "style" {
                continue;
            }
            if !style.keys().contains(&k.as_str()) {
                return Err(format!(
                    "dither has no setting '{k}' ({} takes {})",
                    style.describe(),
                    style.keys().join(", ")
                ));
            }
            match k.as_str() {
                "scale" if v.as_str() == Some("auto") => d.scale = Scale::Auto,
                "scale" => {
                    let s = v.as_f64().ok_or_else(|| {
                        format!("dither scale is auto or a whole number 1..16, not {v}")
                    })?;
                    if s.fract() != 0.0 || !(1.0..=16.0).contains(&s) {
                        return Err(format!(
                            "dither scale is auto or a whole number 1..16, not {s}"
                        ));
                    }
                    d.scale = Scale::Fixed(s as u32);
                }
                "sample" => {
                    d.sample = serde_json::from_value(v.clone())
                        .map_err(|_| format!("dither sample is block or pixel, not {v}"))?;
                }
                "flat" => {
                    let f = number(k, v)?;
                    if !(0.0..=1.0).contains(&f) {
                        return Err(format!("dither flat is within 0..1, not {f}"));
                    }
                    d.flat = f;
                }
                "size" if style == Style::Checker => {
                    let s = number(k, v)?;
                    if ![2.0, 4.0, 8.0].contains(&s) {
                        return Err(format!("checker size is 2, 4 or 8, not {s}"));
                    }
                    d.size = s as u32;
                }
                "size" => {
                    let s = number(k, v)?;
                    if s.fract() != 0.0 || !(3.0..=32.0).contains(&s) {
                        return Err(format!(
                            "halftone size is a whole number of pattern pixels 3..32, not {s}"
                        ));
                    }
                    d.size = s as u32;
                }
                "angle" => {
                    let a = number(k, v)?;
                    if !(0.0..=90.0).contains(&a) {
                        return Err(format!("halftone angle is 0..90 degrees, not {a}"));
                    }
                    d.angle = a;
                }
                "kernel" => {
                    d.kernel = serde_json::from_value(v.clone()).map_err(|_| {
                        format!("diffuse kernel is floyd-steinberg or atkinson, not {v}")
                    })?;
                }
                "tiles" => {
                    d.tiles = serde_json::from_value(v.clone())
                        .map_err(|_| format!("pattern tiles are hatch or lines, not {v}"))?;
                }
                _ => unreachable!("keys are checked against the style"),
            }
        }
        Ok(d)
    }
}

impl From<Dither> for Value {
    fn from(d: Dither) -> Value {
        let mut out = Map::new();
        out.insert("style".into(), d.style.describe().into());
        for key in d.style.keys() {
            let value = match *key {
                "scale" => match d.scale {
                    Scale::Auto => Value::String("auto".into()),
                    Scale::Fixed(n) => Value::from(n),
                },
                "sample" => d.sample.describe().into(),
                "flat" => d.flat.into(),
                "size" => d.size.into(),
                "angle" => d.angle.into(),
                "kernel" => d.kernel.describe().into(),
                "tiles" => d.tiles.describe().into(),
                _ => unreachable!("every style key serializes"),
            };
            out.insert((*key).into(), value);
        }
        Value::Object(out)
    }
}

/// The Bayer threshold at (x, y) for an `n` x `n` matrix (n a power of
/// two), in (0, 1).  Built from the bits of x and y, no table: 2x2 is
/// 0 2 / 3 1.
pub fn bayer(x: u32, y: u32, n: u32) -> f64 {
    let bits = n.trailing_zeros();
    let mut v = 0u32;
    for b in 0..bits {
        let xb = (x >> b) & 1;
        let yb = (y >> b) & 1;
        v |= (((xb ^ yb) << 1) | yb) << (2 * (bits - 1 - b));
    }
    (f64::from(v) + 0.5) / f64::from(n * n)
}

/// The halftone threshold at (x, y): a round dot per `size` x `size`
/// cell of a grid turned `angle` degrees, lowest at the cell's center,
/// so dots grow outward as the tone deepens.  In [0, 1].
pub fn halftone(x: u32, y: u32, size: u32, angle: f64) -> f64 {
    let (sin, cos) = angle.to_radians().sin_cos();
    let (x, y) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
    let u = (x * cos + y * sin) / f64::from(size);
    let v = (y * cos - x * sin) / f64::from(size);
    let (du, dv) = (u - u.floor() - 0.5, v - v.floor() - 0.5);
    let spot = ((std::f64::consts::TAU * du).cos() + (std::f64::consts::TAU * dv).cos()) / 2.0;
    (1.0 - spot) / 2.0
}

/// The stage a tile cell fills in at: the lines first.
fn tile_stage(tiles: Tiles, x: i32, y: i32) -> u8 {
    match tiles {
        Tiles::Hatch if (x + y) % 4 == 0 => 0,
        Tiles::Hatch if (x - y).rem_euclid(4) == 0 => 1,
        Tiles::Hatch if (x + y) % 4 == 2 => 2,
        Tiles::Hatch => 3,
        Tiles::Lines if y % 4 == 0 => 0,
        Tiles::Lines if y % 4 == 2 => 1,
        Tiles::Lines => 2,
    }
}

/// The pattern threshold at (x, y): the order an 8x8 tile's cells fill
/// in (by stage, then the Bayer order within a stage), so the lines
/// come in first.  In (0, 1).
pub fn tile(x: u32, y: u32, tiles: Tiles) -> f64 {
    let key = |cx: i32, cy: i32| (tile_stage(tiles, cx, cy), bayer(cx as u32, cy as u32, 8));
    let me = key((x % 8) as i32, (y % 8) as i32);
    let before = (0..64)
        .map(|i| key(i % 8, i / 8))
        .filter(|other| other.0 < me.0 || (other.0 == me.0 && other.1 < me.1))
        .count();
    (before as f64 + 0.5) / 64.0
}

/// A palette color: its slot, its rgb and its OKLab.
#[derive(Debug, Clone, Copy)]
pub struct Ink {
    pub slot: usize,
    pub rgb: [u8; 3],
    lab: [f64; 3],
}

impl Ink {
    pub fn new(slot: usize, rgb: [u8; 3]) -> Self {
        Self {
            slot,
            rgb,
            lab: lab(rgb),
        }
    }
}

fn lab(rgb: [u8; 3]) -> [f64; 3] {
    let c = rgb_to_oklch(&rgb);
    let h = c.h.to_radians();
    [c.l, c.c * h.cos(), c.c * h.sin()]
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

fn nearest(inks: &[Ink], p: [f64; 3]) -> usize {
    let mut best = 0;
    let mut best_d = f64::INFINITY;
    for (i, ink) in inks.iter().enumerate() {
        let d = dist2(ink.lab, p);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

/// The two inks around `p` and how far `p` sits from the first toward
/// the second (0..1).
fn pair(inks: &[Ink], p: [f64; 3]) -> (usize, usize, f64) {
    let a = nearest(inks, p);
    let c1 = inks[a].lab;
    let reflected = [2.0 * p[0] - c1[0], 2.0 * p[1] - c1[1], 2.0 * p[2] - c1[2]];
    let b = nearest(inks, reflected);
    if a == b {
        return (a, a, 0.0);
    }
    let c2 = inks[b].lab;
    let d = [c2[0] - c1[0], c2[1] - c1[1], c2[2] - c1[2]];
    let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    let f = ((p[0] - c1[0]) * d[0] + (p[1] - c1[1]) * d[1] + (p[2] - c1[2]) * d[2]) / dd;
    (a, b, f.clamp(0.0, 1.0))
}

/// The two inks whose line passes closest to `p`, lighter first, and how
/// far `p` sits from the lighter toward the darker (0..1): paper and ink
/// for the continuous styles.
fn paper_ink(inks: &[Ink], p: [f64; 3]) -> (usize, usize, f64) {
    let a = nearest(inks, p);
    let along = |i: usize, j: usize| {
        let (c1, c2) = (inks[i].lab, inks[j].lab);
        let d = [c2[0] - c1[0], c2[1] - c1[1], c2[2] - c1[2]];
        let dd = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if dd < 1e-12 {
            return 0.0;
        }
        (((p[0] - c1[0]) * d[0] + (p[1] - c1[1]) * d[1] + (p[2] - c1[2]) * d[2]) / dd)
            .clamp(0.0, 1.0)
    };
    let mut best = (a, f64::INFINITY);
    for j in 0..inks.len() {
        if j == a {
            continue;
        }
        let t = along(a, j);
        let (c1, c2) = (inks[a].lab, inks[j].lab);
        let q = [
            c1[0] + (c2[0] - c1[0]) * t,
            c1[1] + (c2[1] - c1[1]) * t,
            c1[2] + (c2[2] - c1[2]) * t,
        ];
        let d = dist2(p, q);
        if d < best.1 {
            best = (j, d);
        }
    }
    let b = best.0;
    if b == a {
        return (a, a, 0.0);
    }
    let (paper, ink) = if inks[a].lab[0] >= inks[b].lab[0] {
        (a, b)
    } else {
        (b, a)
    };
    (paper, ink, along(paper, ink))
}

/// Dither an rgb8 image with `inks`: returns the new pixels and, per
/// ink, how many pixels took it.
pub fn dither(
    pixels: &[u8],
    width: u32,
    height: u32,
    inks: &[Ink],
    d: &Dither,
) -> (Vec<u8>, Vec<u64>) {
    assert!(
        !inks.is_empty(),
        "dithering needs at least one palette color"
    );
    let (w, h) = (width as usize, height as usize);
    let s = d.scale.resolve(width) as usize;
    let (gw, gh) = (w.div_ceil(s), h.div_ceil(s));
    let at = |x: usize, y: usize| {
        let i = (y * w + x) * 3;
        lab([pixels[i], pixels[i + 1], pixels[i + 2]])
    };
    let span = |b: usize, n: usize| (b * s, ((b + 1) * s).min(n));
    let average = |bx: usize, by: usize| {
        let ((x0, x1), (y0, y1)) = (span(bx, w), span(by, h));
        let mut sum = [0.0f64; 3];
        for y in y0..y1 {
            for x in x0..x1 {
                let p = at(x, y);
                sum = [sum[0] + p[0], sum[1] + p[1], sum[2] + p[2]];
            }
        }
        let n = ((x1 - x0) * (y1 - y0)) as f64;
        [sum[0] / n, sum[1] / n, sum[2] / n]
    };

    // which ink each block (or, sampling per pixel, each pixel) takes
    let mut out = vec![0u8; pixels.len()];
    let mut counts = vec![0u64; inks.len()];
    let mut paint = |x: usize, y: usize, ink: usize| {
        let i = (y * w + x) * 3;
        out[i..i + 3].copy_from_slice(&inks[ink].rgb);
        counts[ink] += 1;
    };

    if d.style == Style::Diffuse {
        let blocks: Vec<[f64; 3]> = (0..gh)
            .flat_map(|by| (0..gw).map(move |bx| (bx, by)))
            .map(|(bx, by)| average(bx, by))
            .collect();
        let mut err = vec![[0.0f64; 3]; gw * gh];
        let mut chosen = vec![0usize; gw * gh];
        for by in 0..gh {
            // serpentine: every other row right to left, kernel mirrored
            let dir: i64 = if by % 2 == 0 { 1 } else { -1 };
            for step in 0..gw {
                let bx = if dir == 1 { step } else { gw - 1 - step };
                let k = by * gw + bx;
                let c = blocks[k];
                let e = err[k];
                let p = [
                    (c[0] + e[0]).clamp(0.0, 1.0),
                    (c[1] + e[1]).clamp(-0.5, 0.5),
                    (c[2] + e[2]).clamp(-0.5, 0.5),
                ];
                let ink = nearest(inks, p);
                chosen[k] = ink;
                let q = inks[ink].lab;
                let diff = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
                for &(dx, dy, share) in d.kernel.taps() {
                    let tx = bx as i64 + dx * dir;
                    let ty = by + dy as usize;
                    if tx < 0 || tx >= gw as i64 || ty >= gh {
                        continue;
                    }
                    let t = &mut err[ty * gw + tx as usize];
                    for c in 0..3 {
                        t[c] += diff[c] * share;
                    }
                }
            }
        }
        for by in 0..gh {
            for bx in 0..gw {
                let ((x0, x1), (y0, y1)) = (span(bx, w), span(by, h));
                for y in y0..y1 {
                    for x in x0..x1 {
                        paint(x, y, chosen[by * gw + bx]);
                    }
                }
            }
        }
        return (out, counts);
    }

    let threshold = |bx: usize, by: usize| {
        let (bx, by) = (bx as u32, by as u32);
        match d.style {
            Style::Checker => bayer(bx, by, d.size),
            Style::Halftone => halftone(bx, by, d.size, d.angle),
            Style::Pattern => tile(bx, by, d.tiles),
            Style::Diffuse => unreachable!("diffuse is drawn above"),
        }
    };
    let pick = |p: [f64; 3], bx: usize, by: usize| {
        if d.style == Style::Checker {
            let (a, b, f) = pair(inks, p);
            // where in the mixing band (a quarter to half way) the block sits
            let band = ((f - 0.25) / 0.25).clamp(0.0, 1.0);
            if a == b || band < d.flat || (d.flat >= 1.0 && band < 1.0) {
                return a;
            }
            return if threshold(bx, by) < f { b } else { a };
        }
        let (paper, ink, f) = paper_ink(inks, p);
        let edge = d.flat / 2.0;
        if paper == ink || f <= edge {
            return paper;
        }
        if f >= 1.0 - edge {
            return ink;
        }
        if threshold(bx, by) < f { ink } else { paper }
    };
    for by in 0..gh {
        for bx in 0..gw {
            let ((x0, x1), (y0, y1)) = (span(bx, w), span(by, h));
            let block = (d.sample == Sample::Block).then(|| pick(average(bx, by), bx, by));
            for y in y0..y1 {
                for x in x0..x1 {
                    let ink = block.unwrap_or_else(|| pick(at(x, y), bx, by));
                    paint(x, y, ink);
                }
            }
        }
    }
    (out, counts)
}
