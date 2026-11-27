//! The generation methods' accent candidates.
//!
//! Eight per method, before the chroma level, contrast and the
//! red/green push (`finish`).

use super::analysis::Swatch;
use super::math::{de, hd, lch, mean_hue};
use crate::color::{Lch, floor_mod};
use crate::extract::Cluster;

/// The accent slots' usual hues, base08..base0F.
pub const ANCHOR_H: [f64; 8] = [20.0, 54.0, 86.0, 140.0, 197.0, 247.0, 320.0, 26.0];
/// Bundled dark schemes' accent medians (lightness, chroma).
pub const SLOT_L: [f64; 8] = [0.665, 0.740, 0.811, 0.756, 0.750, 0.697, 0.701, 0.618];
pub const SLOT_C: [f64; 8] = [0.159, 0.131, 0.126, 0.127, 0.097, 0.110, 0.128, 0.104];
/// Bundled light schemes' accent chroma medians.
pub const LIGHT_SLOT_C: [f64; 8] = [0.165, 0.139, 0.128, 0.121, 0.097, 0.123, 0.146, 0.110];

const STEP_L: [f64; 3] = [0.0, 0.09, -0.09];
const STEP_C: [f64; 3] = [1.0, 0.7, 1.2];
/// Found colors need this much chroma.
const MIN_C: f64 = 0.04;
/// Found colors within this many degrees merge into one slot.
const MERGE: f64 = 15.0;
/// Slot windows reach this far past halfway to the neighbors.
const OVERLAP: f64 = 10.0;
/// base0F's chroma cap (bundled schemes' brown: median 0.104 dark, 0.110 light).
const BROWN_CAP: f64 = 0.12;
/// Accents closer than this are penalized as too alike (distinct).
const TARGET: f64 = 0.04;

const TECHNIQUES: [(&str, &[f64]); 4] = [
    ("analogous", &[-30.0, 30.0]),
    ("complementary", &[180.0]),
    ("triadic", &[-120.0, 120.0]),
    ("square", &[90.0, 180.0, 270.0]),
];

fn tech_pref(tech: &str) -> f64 {
    match tech {
        "analogous" => 0.45,
        "complementary" => 0.4,
        "triadic" => 0.35,
        _ => 0.3,
    }
}

/// The seed: the heaviest family with the lightness and chroma of the
/// colors near it; with no families, the most saturated color.
#[derive(Debug, Clone, Copy)]
pub struct Seed {
    pub h: f64,
    pub l: f64,
    pub c: f64,
}

pub fn seed(fams: &[Cluster], clusters: &[Swatch], fine: &[Swatch]) -> Seed {
    let Some(top) = fams.first() else {
        let s = fine
            .iter()
            .copied()
            .reduce(|a, b| if b.c > a.c { b } else { a })
            .unwrap_or(Swatch {
                l: 0.7,
                c: 0.0,
                h: 0.0,
                share: 1.0,
            });
        return Seed {
            h: s.h,
            l: s.l,
            c: s.c,
        };
    };
    let near: Vec<&Swatch> = clusters
        .iter()
        .filter(|c| c.c >= 0.04 && hd(c.h, top.hue).abs() <= 30.0)
        .collect();
    if near.is_empty() {
        return Seed {
            h: top.hue,
            l: 0.7,
            c: top.chroma,
        };
    }
    let w: f64 = near.iter().map(|c| c.share).sum();
    Seed {
        h: top.hue,
        l: near.iter().map(|c| c.l * c.share).sum::<f64>() / w,
        c: near.iter().map(|c| c.c * c.share).sum::<f64>() / w,
    }
}

fn clampac(l: f64, c: f64) -> (f64, f64) {
    (l.clamp(0.6, 0.82), c.clamp(0.07, 0.18))
}

/// Slots 08..0E: each window reaches halfway to the neighboring anchors,
/// plus OVERLAP, as (lo, hi) offsets from the anchor.
fn windows() -> [(f64, f64); 7] {
    let hs = &ANCHOR_H[..7];
    std::array::from_fn(|i| {
        let prev = hs[(i + 6) % 7];
        let next = hs[(i + 1) % 7];
        (
            -hd(prev, hs[i]).abs() / 2.0 - OVERLAP,
            hd(hs[i], next).abs() / 2.0 + OVERLAP,
        )
    })
}

fn in_window(h: f64, i: usize, win: &[(f64, f64); 7]) -> bool {
    let d = hd(ANCHOR_H[i], h);
    win[i].0 <= d && d <= win[i].1
}

fn score(c: &Swatch) -> f64 {
    c.share * c.c
}

/// Found colors, strongest (share x chroma) first.
fn found_colors(fine: &[Swatch]) -> Vec<Swatch> {
    let mut found: Vec<Swatch> = fine.iter().copied().filter(|c| c.c >= MIN_C).collect();
    found.sort_by(|a, b| score(b).total_cmp(&score(a)));
    found
}

/// A share-weighted merge of found colors into one slot color.
fn merged(found: &[Swatch], group: &[usize]) -> Lch {
    let w: f64 = group.iter().map(|&k| found[k].share).sum();
    let l = group
        .iter()
        .map(|&k| found[k].l * found[k].share)
        .sum::<f64>()
        / w;
    let c = group
        .iter()
        .map(|&k| found[k].c * found[k].share)
        .sum::<f64>()
        / w;
    let h = mean_hue(group.iter().map(|&k| (found[k].h, found[k].share)));
    let (l, c) = clampac(l, c);
    lch(l, c, h)
}

/// What a slot was filled from: base0F's fallback looks at 08 and 09.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Found,
    Derived,
}

/// base0F: a real brown (warm, darker) if one is left, else a darker
/// 08 (or 09, when only 09 is a found color).
fn brown(found: &[Swatch], used: &[bool], slots: &mut [Lch; 8], kinds: &[Kind; 8]) {
    let browns = (0..found.len())
        .filter(|&k| !used[k] && (10.0..=75.0).contains(&found[k].h) && found[k].l < 0.6)
        .reduce(|a, b| {
            if score(&found[b]) > score(&found[a]) {
                b
            } else {
                a
            }
        });
    slots[7] = match browns {
        Some(k) => {
            let b = found[k];
            lch(b.l.max(0.5), b.c.clamp(0.05, BROWN_CAP), b.h)
        }
        None => {
            let k = if kinds[0] == Kind::Found || kinds[1] != Kind::Found {
                0
            } else {
                1
            };
            lch(slots[k].l - 0.12, slots[k].c * 0.75, slots[k].h)
        }
    };
}

/// found: the image's colors into the slots whose hue windows they fall
/// in, strongest first, close neighbors merged; empty slots get hues
/// derived from the found colors.
pub fn found(fine: &[Swatch]) -> [Lch; 8] {
    let win = windows();
    let found = found_colors(fine);
    let mut slots: [Option<Lch>; 8] = [None; 8];
    let mut kinds = [Kind::Derived; 8];
    let mut used = vec![false; found.len()];
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (j, c) in found.iter().enumerate() {
        for i in 0..7 {
            if in_window(c.h, i, &win) {
                let half = if hd(ANCHOR_H[i], c.h) >= 0.0 {
                    win[i].1
                } else {
                    -win[i].0
                };
                pairs.push((
                    score(c) * (1.0 - 0.5 * hd(ANCHOR_H[i], c.h).abs() / half),
                    i,
                    j,
                ));
            }
        }
    }
    // Python's sorted(reverse=True) on (score, slot, color) tuples
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
    for &(_, i, j) in &pairs {
        if slots[i].is_some() || used[j] {
            continue;
        }
        let pick = found[j];
        let group: Vec<usize> = (0..found.len())
            .filter(|&k| {
                !used[k] && in_window(found[k].h, i, &win) && hd(pick.h, found[k].h).abs() <= MERGE
            })
            .collect();
        for &k in &group {
            used[k] = true;
        }
        slots[i] = Some(merged(&found, &group));
        kinds[i] = Kind::Found;
    }
    let mut sources: Vec<usize> = (0..found.len()).filter(|&j| used[j]).collect();
    sources.sort_by(|&a, &b| {
        score(&found[b])
            .total_cmp(&score(&found[a]))
            .then(a.cmp(&b))
    });
    // empty slots: expand from found colors, technique by technique
    for (_, offsets) in TECHNIQUES {
        for i in 0..7 {
            if slots[i].is_some() {
                continue;
            }
            for &s in &sources {
                let src = found[s];
                let hits: Vec<f64> = offsets
                    .iter()
                    .map(|o| floor_mod(src.h + o, 360.0))
                    .filter(|&h| in_window(h, i, &win))
                    .filter(|&h| {
                        slots
                            .iter()
                            .all(|x| x.is_none_or(|x| hd(x.h, h).abs() > 8.0))
                    })
                    .collect();
                if let Some(h) = hits.iter().copied().reduce(|a, b| {
                    if hd(ANCHOR_H[i], b).abs() < hd(ANCHOR_H[i], a).abs() {
                        b
                    } else {
                        a
                    }
                }) {
                    let (l, c) = clampac(src.l, src.c);
                    slots[i] = Some(lch(l, c, h));
                    break;
                }
            }
        }
    }
    // still empty: the closest found or derived hue
    let pool: Vec<usize> = if sources.is_empty() {
        (0..found.len()).collect()
    } else {
        sources.clone()
    };
    for i in 0..7 {
        if slots[i].is_some() {
            continue;
        }
        let mut best: Option<(f64, f64, usize)> = None;
        for &s in &pool {
            let src = found[s];
            let offsets =
                std::iter::once(0.0).chain(TECHNIQUES.iter().flat_map(|(_, o)| o.iter().copied()));
            for o in offsets {
                let h = floor_mod(src.h + o, 360.0);
                let d = hd(ANCHOR_H[i], h).abs();
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, h, s));
                }
            }
        }
        slots[i] = Some(match best {
            None => lch(SLOT_L[i], SLOT_C[i], ANCHOR_H[i]),
            Some((_, h, s)) => {
                let (l, c) = clampac(found[s].l, found[s].c);
                lch(l, c, h)
            }
        });
    }
    let mut out: [Lch; 8] = std::array::from_fn(|i| slots[i].unwrap_or(lch(0.0, 0.0, 0.0)));
    brown(&found, &used, &mut out, &kinds);
    out
}

/// One option for a slot in distinct: preference, color, found index.
#[derive(Clone, Copy)]
struct Opt {
    pref: f64,
    color: Lch,
    found: Option<usize>,
}

/// distinct: every option per slot (found colors in the window, hues
/// derived from the strongest found colors), then the set that fits
/// best while penalizing accents too alike.
pub fn distinct(fine: &[Swatch]) -> [Lch; 8] {
    let win = windows();
    let found = found_colors(fine);
    let top = found.first().map_or(1.0, score);
    let mut opts: Vec<Vec<Opt>> = vec![Vec::new(); 7];
    for (j, c) in found.iter().enumerate() {
        for (i, slot) in opts.iter_mut().enumerate() {
            if in_window(c.h, i, &win) {
                let d = hd(ANCHOR_H[i], c.h);
                let half = if d >= 0.0 { win[i].1 } else { -win[i].0 };
                let (l, cc) = clampac(c.l, c.c);
                slot.push(Opt {
                    pref: 0.5 + 0.5 * (score(c) / top).sqrt() * (1.0 - 0.3 * d.abs() / half),
                    color: lch(l, cc, c.h),
                    found: Some(j),
                });
            }
        }
    }
    for (i, slot) in opts.iter_mut().enumerate() {
        for (tech, offsets) in TECHNIQUES {
            for src in found.iter().take(8) {
                for o in offsets {
                    let h = floor_mod(src.h + o, 360.0);
                    if in_window(h, i, &win) {
                        let (l, c) = clampac(src.l, src.c);
                        slot.push(Opt {
                            pref: tech_pref(tech) * (0.5 + 0.5 * (score(src) / top).sqrt()),
                            color: lch(l, c, h),
                            found: None,
                        });
                    }
                }
            }
        }
        if slot.is_empty() {
            slot.push(Opt {
                pref: 0.1,
                color: lch(SLOT_L[i], SLOT_C[i], ANCHOR_H[i]),
                found: None,
            });
        }
    }
    let total = |ch: &[usize]| {
        let used: Vec<usize> = (0..7).filter_map(|i| opts[i][ch[i]].found).collect();
        let mut uniq = used.clone();
        uniq.sort_unstable();
        uniq.dedup();
        let mut t: f64 = (0..7).map(|i| opts[i][ch[i]].pref).sum::<f64>()
            - 5.0 * (used.len() - uniq.len()) as f64;
        for a in 0..7 {
            for b in a + 1..7 {
                let d = de(&opts[a][ch[a]].color, &opts[b][ch[b]].color);
                if d < TARGET {
                    t -= 2.0 * (TARGET - d) / TARGET;
                }
            }
        }
        t
    };
    // Python's max() keeps the first of equal values
    let argmax = |n: usize, f: &dyn Fn(usize) -> f64| {
        let mut best = 0;
        for k in 1..n {
            if f(k) > f(best) {
                best = k;
            }
        }
        best
    };
    let mut ch: Vec<usize> = (0..7)
        .map(|i| argmax(opts[i].len(), &|k| opts[i][k].pref))
        .collect();
    for _ in 0..10 {
        let mut changed = false;
        for i in 0..7 {
            let best = argmax(opts[i].len(), &|k| {
                let mut trial = ch.clone();
                trial[i] = k;
                total(&trial)
            });
            if best != ch[i] {
                ch[i] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut used = vec![false; found.len()];
    for i in 0..7 {
        if let Some(j) = opts[i][ch[i]].found {
            used[j] = true;
        }
    }
    let mut out = [lch(0.0, 0.0, 0.0); 8];
    let mut kinds = [Kind::Derived; 8];
    for i in 0..7 {
        let pick = opts[i][ch[i]];
        out[i] = pick.color;
        if let Some(j) = pick.found {
            let mut group = vec![j];
            for o in &opts[i] {
                if let Some(k) = o.found
                    && !used[k]
                    && hd(found[j].h, found[k].h).abs() <= MERGE
                    && de(&pick.color, &o.color) <= 0.03
                {
                    group.push(k);
                }
            }
            for &k in &group {
                used[k] = true;
            }
            out[i] = merged(&found, &group);
            kinds[i] = Kind::Found;
        }
    }
    brown(&found, &used, &mut out, &kinds);
    out
}

/// The step and wheel methods' candidates (before slot assignment).
pub fn candidates(name: &str, seed: &Seed, fams: &[Cluster], clusters: &[Swatch]) -> Vec<Lch> {
    let h = seed.h;
    let l = seed.l.clamp(0.62, 0.80);
    let c = seed.c.clamp(0.08, 0.18);
    match name {
        "conventional" => (0..8)
            .map(|i| lch(SLOT_L[i], SLOT_C[i], ANCHOR_H[i]))
            .collect(),
        "image-hues" => image_hues(fams, clusters, l),
        "mono" => {
            let mut v = Vec::new();
            for dl in [0.0, 0.1, -0.1] {
                for dc in [1.0, 0.6, 1.3] {
                    v.push(lch(l + dl, c * dc, h));
                }
            }
            v.truncate(8);
            v
        }
        "analogous" => steps(&[h, h - 30.0, h + 30.0], l, c),
        "complementary" => steps(&[h, h + 180.0], l, c),
        "split-complementary" => steps(&[h, h + 150.0, h + 210.0], l, c),
        "triadic" => steps(&[h, h + 120.0, h + 240.0], l, c),
        "triadic-fill" => interp(&[h, h + 120.0, h + 240.0], l, c),
        _ => steps(&[h, h + 90.0, h + 180.0, h + 270.0], l, c),
    }
}

fn image_hues(fams: &[Cluster], clusters: &[Swatch], l: f64) -> Vec<Lch> {
    let mut cands: Vec<Lch> = Vec::new();
    for f in fams.iter().take(6) {
        let near: Vec<&Swatch> = clusters
            .iter()
            .filter(|x| x.c >= 0.04 && hd(x.h, f.hue).abs() <= 30.0)
            .collect();
        let (fl, fc) = if near.is_empty() {
            (l, f.chroma)
        } else {
            let w: f64 = near.iter().map(|x| x.share).sum();
            (
                near.iter().map(|x| x.l * x.share).sum::<f64>() / w,
                near.iter().map(|x| x.c).fold(f64::MIN, f64::max),
            )
        };
        cands.push(lch(fl.clamp(0.6, 0.82), fc.clamp(0.07, 0.18), f.hue));
    }
    while cands.len() < 8 {
        let mut hs = cands.clone();
        hs.sort_by(|a, b| a.h.total_cmp(&b.h));
        let n = hs.len();
        let (g, i) = (0..n)
            .map(|i| {
                let g = floor_mod(hs[(i + 1) % n].h - hs[i].h, 360.0);
                (if g == 0.0 { 360.0 } else { g }, i)
            })
            .reduce(|a, b| if (b.0, b.1) > (a.0, a.1) { b } else { a })
            .expect("at least one family");
        let (a, b) = (hs[i], hs[(i + 1) % n]);
        cands.push(lch(
            (a.l + b.l) / 2.0,
            (a.c + b.c) / 2.0,
            floor_mod(a.h + g / 2.0, 360.0),
        ));
    }
    cands
}

fn steps(hues: &[f64], l: f64, c: f64) -> Vec<Lch> {
    (0..8)
        .map(|i| {
            let r = i / hues.len();
            lch(
                l + STEP_L[r % 3],
                c * STEP_C[r % 3],
                floor_mod(hues[i % hues.len()], 360.0),
            )
        })
        .collect()
}

fn interp(hues: &[f64], l: f64, c: f64) -> Vec<Lch> {
    let mut hs: Vec<f64> = hues.iter().map(|h| floor_mod(*h, 360.0)).collect();
    hs.sort_by(f64::total_cmp);
    while hs.len() < 8 {
        let n = hs.len();
        let (g, i) = (0..n)
            .map(|i| {
                let g = floor_mod(hs[(i + 1) % n] - hs[i], 360.0);
                (if g == 0.0 { 360.0 } else { g }, i)
            })
            .reduce(|a, b| if (b.0, b.1) > (a.0, a.1) { b } else { a })
            .expect("three hues");
        hs.insert(i + 1, floor_mod(hs[i] + g / 2.0, 360.0));
        hs.sort_by(f64::total_cmp);
    }
    hs.iter().map(|&h| lch(l, c, h)).collect()
}

/// The slot anchors leaned toward the image: each moves halfway to the
/// nearest family, at most 15 degrees.
pub fn anchors(fams: &[Cluster]) -> [f64; 8] {
    if fams.is_empty() {
        return ANCHOR_H;
    }
    ANCHOR_H.map(|a| {
        let f = fams
            .iter()
            .reduce(|x, y| {
                if hd(a, y.hue).abs() < hd(a, x.hue).abs() {
                    y
                } else {
                    x
                }
            })
            .expect("families are non-empty");
        let d = hd(a, f.hue);
        floor_mod(a + (d.abs() / 2.0).min(15.0).copysign(d), 360.0)
    })
}

/// The candidates' best slot order: hue near each anchor, red kept apart
/// from green and yellow.
pub fn assign(cands: &[Lch], anchors: &[f64; 8]) -> [Lch; 8] {
    let cost: Vec<[f64; 8]> = cands
        .iter()
        .map(|c| anchors.map(|a| (hd(c.h, a) / 180.0).powi(2)))
        .collect();
    let pen = |x: usize, y: usize| (0.08 - de(&cands[x], &cands[y])).max(0.0) * 4.0;
    let pen03: Vec<Vec<f64>> = (0..8)
        .map(|x| (0..8).map(|y| pen(x, y)).collect())
        .collect();
    let mut p: [usize; 8] = std::array::from_fn(|i| i);
    let mut best = f64::INFINITY;
    let mut best_p = p;
    // lexicographic permutations, as itertools.permutations yields them
    loop {
        let s = (0..8).map(|i| cost[p[i]][i]).sum::<f64>() + pen03[p[0]][p[3]] + pen03[p[0]][p[2]];
        if s < best {
            best = s;
            best_p = p;
        }
        let Some(i) = (0..7).rev().find(|&i| p[i] < p[i + 1]) else {
            break;
        };
        let j = (i + 1..8)
            .rev()
            .find(|&j| p[j] > p[i])
            .expect("a larger element");
        p.swap(i, j);
        p[i + 1..].reverse();
    }
    best_p.map(|i| cands[i])
}
