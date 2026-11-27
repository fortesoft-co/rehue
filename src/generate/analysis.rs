//! What generation reads from a wallpaper.
//!
//! Its hue families, its colors at two levels of detail, and how light
//! it is.  Everything is measured on a 256 px (long side) Lanczos copy, the
//! size extraction works at.  A video's sampled frames are each scaled
//! that way and pooled, so every frame counts the same.

use image::{RgbImage, imageops};

use crate::color::{oklab_to_rgb, rgb_to_oklab, rgb_to_oklch};
use crate::extract::{Cluster, ExtractionParams, extract_hues_pixels};

/// One color found in the image, with the share of pixels nearest it.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Swatch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
    pub share: f64,
}

/// The image as the generator sees it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Analysis {
    /// rehue's hue families (heaviest first).
    pub families: Vec<Cluster>,
    /// 12 colors (OKLab k-means).
    pub clusters: Vec<Swatch>,
    /// 24 colors (OKLab k-means): the fine colors most methods read.
    pub fine: Vec<Swatch>,
    /// Pixel lightness at the 10th, 50th and 90th percentiles.
    pub p10: f64,
    pub p50: f64,
    pub p90: f64,
    /// The pixels at working size, row by row, for rswal's quantizer.
    #[serde(skip)]
    pub pixels: Vec<[u8; 3]>,
    /// The rows' length.
    #[serde(skip)]
    pub width: usize,
}

/// The working-size copy: the long side scaled to 256 (never up).
pub fn working_size(image: &RgbImage) -> RgbImage {
    let long = image.width().max(image.height());
    if long <= 256 {
        return image.clone();
    }
    let scale = 256.0 / f64::from(long);
    let w = ((f64::from(image.width()) * scale).round() as u32).max(1);
    let h = ((f64::from(image.height()) * scale).round() as u32).max(1);
    imageops::resize(image, w, h, imageops::FilterType::Lanczos3)
}

/// The copy pywal quantizes: a quarter of the size each way.
pub fn quarter_size(image: &RgbImage) -> RgbImage {
    let w = ((f64::from(image.width()) * 0.25).round() as u32).max(1);
    let h = ((f64::from(image.height()) * 0.25).round() as u32).max(1);
    imageops::resize(image, w, h, imageops::FilterType::Lanczos3)
}

impl Analysis {
    /// rswal reads this image's colors instead of the working copy
    /// (a still's quarter-size copy, as pywal reads it).
    pub fn rswal_source(&mut self, image: &RgbImage) {
        self.pixels = image.pixels().map(|p| p.0).collect();
        self.width = image.width() as usize;
    }
}

/// Analyse frames (one for a still), each already at working size.
pub fn analyse(frames: &[RgbImage]) -> Analysis {
    let pixels: Vec<[u8; 3]> = frames
        .iter()
        .flat_map(|f| f.pixels().map(|p| p.0))
        .collect();
    let labs: Vec<[f64; 3]> = pixels.iter().map(rgb_to_oklab).collect();
    let families = extract_hues_pixels(pixels.iter().copied(), &ExtractionParams::default());
    let swatches = |k: usize| {
        let colors = kmeans(&labs, k, 0.325, 30);
        let shares = shares(&labs, &colors);
        colors
            .iter()
            .zip(shares)
            .map(|(rgb, share)| {
                let x = rgb_to_oklch(rgb);
                Swatch {
                    l: x.l,
                    c: x.c,
                    h: x.h,
                    share,
                }
            })
            .collect::<Vec<_>>()
    };
    let mut ls: Vec<f64> = labs.iter().map(|p| p[0]).collect();
    ls.sort_by(f64::total_cmp);
    let q = |p: f64| {
        ls.get(((ls.len().max(1) - 1) as f64 * p) as usize)
            .copied()
            .unwrap_or(0.0)
    };
    Analysis {
        families,
        clusters: swatches(12),
        fine: swatches(24),
        p10: q(0.1),
        p50: q(0.5),
        p90: q(0.9),
        pixels,
        width: frames.first().map_or(1, |f| f.width() as usize),
    }
}

/// Each color's share: the fraction of pixels nearest it (OKLab).
fn shares(pixels: &[[f64; 3]], colors: &[[u8; 3]]) -> Vec<f64> {
    let labs: Vec<[f64; 3]> = colors.iter().map(rgb_to_oklab).collect();
    let mut n = vec![0usize; labs.len()];
    if labs.is_empty() {
        return Vec::new();
    }
    for p in pixels {
        let (i, _) = labs
            .iter()
            .enumerate()
            .map(|(i, l)| (i, (0..3).map(|k| (p[k] - l[k]).powi(2)).sum::<f64>()))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .expect("colors are non-empty");
        n[i] += 1;
    }
    n.iter()
        .map(|&x| x as f64 / pixels.len().max(1) as f64)
        .collect()
}

/// Wu-style variance splitting over weighted points: start with one box
/// holding everything; repeatedly take the box with the largest summed
/// squared error and cut it where (on any axis, between sorted points)
/// the two halves' summed error is smallest.  Box means seed k-means.
fn wu_seeds(pts: &[([f64; 3], f64)], k: usize) -> Vec<[f64; 3]> {
    let sse = |b: &[usize]| -> f64 {
        let (mut n, mut s, mut q) = (0.0, [0.0; 3], 0.0);
        for &i in b {
            let (p, w) = pts[i];
            n += w;
            for c in 0..3 {
                s[c] += p[c] * w;
                q += p[c] * p[c] * w;
            }
        }
        if n > 0.0 {
            q - (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]) / n
        } else {
            0.0
        }
    };
    let all: Vec<usize> = (0..pts.len()).collect();
    let e = sse(&all);
    let mut boxes: Vec<(Vec<usize>, f64)> = vec![(all, e)];
    while boxes.len() < k {
        let Some(bi) = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.0.len() > 1)
            .max_by(|a, b| a.1.1.total_cmp(&b.1.1))
            .map(|(i, _)| i)
        else {
            break;
        };
        let members = boxes[bi].0.clone();
        let mut best: Option<(f64, Vec<usize>, Vec<usize>)> = None;
        for axis in 0..3 {
            let mut idx = members.clone();
            idx.sort_by(|&a, &b| pts[a].0[axis].total_cmp(&pts[b].0[axis]).then(a.cmp(&b)));
            let m = idx.len();
            let mut pn = vec![0.0; m + 1];
            let mut ps = vec![[0.0; 3]; m + 1];
            let mut pq = vec![0.0; m + 1];
            for (j, &i) in idx.iter().enumerate() {
                let (p, w) = pts[i];
                pn[j + 1] = pn[j] + w;
                pq[j + 1] = pq[j] + w * (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]);
                for c in 0..3 {
                    ps[j + 1][c] = ps[j][c] + p[c] * w;
                }
            }
            let err = |a: usize, b: usize| {
                let n = pn[b] - pn[a];
                let q = pq[b] - pq[a];
                let s = [0, 1, 2].map(|c| ps[b][c] - ps[a][c]);
                q - (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]) / n
            };
            for cut in 1..m {
                if pts[idx[cut - 1]].0[axis] == pts[idx[cut]].0[axis] {
                    continue;
                }
                let e = err(0, cut) + err(cut, m);
                if best.as_ref().is_none_or(|b| e < b.0) {
                    best = Some((e, idx[..cut].to_vec(), idx[cut..].to_vec()));
                }
            }
        }
        let Some((_, a, b)) = best else {
            boxes[bi].1 = 0.0;
            continue;
        };
        let (ea, eb) = (sse(&a), sse(&b));
        boxes[bi] = (a, ea);
        boxes.push((b, eb));
    }
    boxes
        .iter()
        .map(|(b, _)| {
            let n: f64 = b.iter().map(|&i| pts[i].1).sum();
            [0, 1, 2].map(|c| b.iter().map(|&i| pts[i].0[c] * pts[i].1).sum::<f64>() / n)
        })
        .collect()
}

/// Deterministic OKLab k-means: pixels binned (64 L steps, a/b in steps
/// of 0.008), L scaled by `lw`, Wu-seeded, then weighted Lloyd passes.
fn kmeans(pixels: &[[f64; 3]], k: usize, lw: f64, iters: usize) -> Vec<[u8; 3]> {
    use std::collections::BTreeMap;
    let mut bins: BTreeMap<(i32, i32, i32), ([f64; 3], f64)> = BTreeMap::new();
    for p in pixels {
        let key = (
            (p[0] * 64.0) as i32,
            (p[1] / 0.008) as i32,
            (p[2] / 0.008) as i32,
        );
        let e = bins.entry(key).or_insert(([0.0; 3], 0.0));
        for (sum, v) in e.0.iter_mut().zip(p) {
            *sum += v;
        }
        e.1 += 1.0;
    }
    if bins.is_empty() {
        return Vec::new();
    }
    let pts: Vec<([f64; 3], f64)> = bins
        .values()
        .map(|(s, n)| ([s[0] / n * lw, s[1] / n, s[2] / n], *n))
        .collect();
    let d2 = |a: &[f64; 3], b: &[f64; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>();
    let mut centers = wu_seeds(&pts, k);
    for _ in 0..iters {
        let mut sum = vec![([0.0; 3], 0.0); centers.len()];
        for (p, n) in &pts {
            let i = (0..centers.len())
                .min_by(|&a, &b| d2(p, &centers[a]).total_cmp(&d2(p, &centers[b])))
                .expect("centers are non-empty");
            for (s, v) in sum[i].0.iter_mut().zip(p) {
                *s += v * n;
            }
            sum[i].1 += n;
        }
        for (c, (s, n)) in centers.iter_mut().zip(&sum) {
            if *n > 0.0 {
                *c = [s[0] / n, s[1] / n, s[2] / n];
            }
        }
    }
    centers
        .iter()
        .map(|c| oklab_to_rgb(&[c[0] / lw, c[1], c[2]]))
        .collect()
}
