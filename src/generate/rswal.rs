//! rswal: the image's own colors as accents, as pywal picks them.
//!
//! Each slot gets the color nearest its usual one.  pywal asks
//! ImageMagick for 16 colors and uses the upper eight of its listing;
//! this is ImageMagick's octree quantizer done the same way (an sRGB
//! color cube, the least-error branches pruned until few enough colors
//! are left, listed in the cube's order), so the colors are pywal's.

use super::math::lch;
use super::methods::{ANCHOR_H, LIGHT_SLOT_C, SLOT_C};
use crate::color::{Lch, rgb_to_oklch};

/// The order slots pick in: red, yellow, orange, brown, green, then the
/// cool slots (cyan, blue, purple).
const ORDER: [usize; 8] = [0, 2, 1, 7, 3, 4, 5, 6];

// The quantizer follows ImageMagick's color reduction step for step
// (MagickCore/quantize.c: classify, prune to depth, reduce, colormap),
// so rswal's colors are the ones pywal gets from `magick -colors 16`.
// ImageMagick is (c) ImageMagick Studio LLC, under the ImageMagick
// License; pywal is (c) Dylan Araps, MIT.

/// A node of the color cube (an arena index tree).
#[derive(Default, Clone)]
struct Node {
    children: [Option<usize>; 8],
    parent: usize,
    id: usize,
    level: usize,
    /// Pixels classified here (at the leaves, or pruned up into it).
    unique: f64,
    /// Summed color, on a 0..1 scale.
    total: [f64; 3],
    /// Summed distance of every pixel through here to its center.
    error: f64,
}

/// ImageMagick's color cube, as `-colors` builds it: the first rows at
/// full depth, the rest at the cube's depth once there are too many
/// colors, then the least-error branches pruned until few enough are
/// left.  Values on a 16-bit scale, as ImageMagick's Q16 build works.
struct Cube {
    nodes: Vec<Node>,
    /// Live nodes (ImageMagick's count, which pruning is capped by).
    live: usize,
    depth: usize,
    k: usize,
    colors: usize,
    pruning: f64,
    next: f64,
}

const MAX_DEPTH: usize = 8;
const RANGE: f64 = 65535.0;

impl Cube {
    fn new(k: usize) -> Cube {
        // log4(colors) + 2, one less with dithering on
        let mut depth = 1;
        let mut c = k;
        while c != 0 {
            c >>= 2;
            depth += 1;
        }
        if depth > 2 {
            depth -= 1;
        }
        Cube {
            nodes: vec![Node::default()],
            live: 1,
            depth: depth.clamp(2, MAX_DEPTH),
            k,
            colors: 0,
            pruning: 0.0,
            next: 0.0,
        }
    }

    fn classify(&mut self, p: [u8; 3], levels: usize) {
        let pixel = p.map(|v| f64::from(v) * 257.0);
        let mut mid = [RANGE / 2.0; 3];
        let mut bisect = (RANGE + 1.0) / 2.0;
        let mut node = 0;
        for level in 1..=levels {
            bisect *= 0.5;
            let shift = MAX_DEPTH - level;
            let bit = |v: u8| usize::from((v >> shift) & 1);
            let id = bit(p[0]) | bit(p[1]) << 1 | bit(p[2]) << 2;
            for (i, m) in mid.iter_mut().enumerate() {
                *m += if (id >> i) & 1 == 1 { bisect } else { -bisect };
            }
            let child = match self.nodes[node].children[id] {
                Some(c) => c,
                None => {
                    self.nodes.push(Node {
                        parent: node,
                        id,
                        level,
                        ..Node::default()
                    });
                    let c = self.nodes.len() - 1;
                    self.nodes[node].children[id] = Some(c);
                    self.live += 1;
                    if level == levels {
                        self.colors += 1;
                    }
                    c
                }
            };
            node = child;
            let d: f64 = (0..3).map(|i| ((pixel[i] - mid[i]) / RANGE).powi(2)).sum();
            self.nodes[node].error += d.sqrt();
            let e = self.nodes[node].error;
            self.nodes[0].error += e;
        }
        self.nodes[node].unique += 1.0;
        for (t, v) in self.nodes[node].total.iter_mut().zip(pixel) {
            *t += v / RANGE;
        }
    }

    /// Merge a node into its parent, its children first, while there are
    /// more nodes than colors wanted.
    fn prune_child(&mut self, n: usize) {
        for i in 0..8 {
            if let Some(c) = self.nodes[n].children[i] {
                self.prune_child(c);
            }
        }
        if self.live > self.k {
            let node = self.nodes[n].clone();
            let parent = &mut self.nodes[node.parent];
            parent.unique += node.unique;
            for i in 0..3 {
                parent.total[i] += node.total[i];
            }
            parent.children[node.id] = None;
            self.live -= 1;
        }
    }

    fn prune_to_depth(&mut self, n: usize) {
        for i in 0..8 {
            if let Some(c) = self.nodes[n].children[i] {
                self.prune_to_depth(c);
            }
        }
        if self.nodes[n].level > self.depth {
            self.prune_child(n);
        }
    }

    fn reduce(&mut self, n: usize) {
        for i in 0..8 {
            if let Some(c) = self.nodes[n].children[i] {
                self.reduce(c);
            }
        }
        let node = &self.nodes[n];
        if node.error <= self.pruning {
            self.prune_child(n);
        } else {
            if node.unique > 0.0 {
                self.colors += 1;
            }
            if node.error < self.next {
                self.next = node.error;
            }
        }
    }

    fn flatten(&self, n: usize, out: &mut Vec<f64>) {
        out.push(self.nodes[n].error);
        for c in self.nodes[n].children.iter().flatten() {
            self.flatten(*c, out);
        }
    }

    fn reduce_colors(&mut self) {
        self.next = 0.0;
        if self.colors > self.k {
            // a fast first cut: keep about 10% more branches than colors
            let mut errors = Vec::new();
            self.flatten(0, &mut errors);
            errors.sort_by(f64::total_cmp);
            let keep = 110 * (self.k + 1) / 100;
            if errors.len() > keep {
                self.next = errors[errors.len() - keep];
            }
        }
        while self.colors > self.k {
            self.pruning = self.next;
            self.next = self.nodes[0].error - 1.0;
            self.colors = 0;
            self.reduce(0);
        }
    }

    /// Every color left: each node's mean.
    fn colormap(&self, n: usize, out: &mut Vec<[u8; 3]>) {
        for c in self.nodes[n].children.iter().flatten() {
            self.colormap(*c, out);
        }
        let node = &self.nodes[n];
        if node.unique > 0.0 {
            out.push(node.total.map(|t| {
                // no rounding in between, as ImageMagick's HDRI build
                let q = (t / node.unique * RANGE).clamp(0.0, RANGE);
                (q / 257.0 + 0.5).floor() as u8
            }));
        }
    }
}

/// ImageMagick's listing order: bits interleaved most significant first,
/// blue over green over red.
fn cube_key(c: &[u8; 3]) -> u32 {
    let mut key = 0;
    for shift in (0..8).rev() {
        let bit = |v: u8| u32::from((v >> shift) & 1);
        key = key << 3 | bit(c[2]) << 2 | bit(c[1]) << 1 | bit(c[0]);
    }
    key
}

/// Up to `k` colors for the pixels (rows `width` long), in listing order.
fn quantize(pixels: &[[u8; 3]], width: usize, k: usize) -> Vec<[u8; 3]> {
    let mut cube = Cube::new(k);
    let rows: Vec<&[[u8; 3]]> = pixels.chunks(width.max(1)).collect();
    let mut y = 0;
    while y < rows.len() {
        for p in rows[y] {
            cube.classify(*p, MAX_DEPTH);
        }
        y += 1;
        if cube.colors > k {
            cube.prune_to_depth(0);
            break;
        }
    }
    let depth = cube.depth;
    for row in &rows[y..] {
        for p in *row {
            cube.classify(*p, depth);
        }
    }
    if cube.colors > k {
        cube.reduce_colors();
    }
    let mut out = Vec::new();
    cube.colormap(0, &mut out);
    out.sort_by_key(cube_key);
    out.dedup();
    out
}

/// pywal's eight accent colors: the upper eight of 16 (more are asked
/// for when fewer come back, as pywal does).
pub fn colors(pixels: &[[u8; 3]], width: usize) -> Vec<Lch> {
    let mut got = Vec::new();
    for k in 16..36 {
        got = quantize(pixels, width, k);
        if got.len() >= 16 {
            break;
        }
    }
    let upper: Vec<[u8; 3]> = got.iter().skip(8).take(8).copied().collect();
    let mut out: Vec<Lch> = upper.iter().map(rgb_to_oklch).collect();
    // an image too plain for 16 colors: repeat what there is
    let base = out.clone();
    let fallback: Vec<Lch> = if base.is_empty() {
        got.iter().map(rgb_to_oklch).collect()
    } else {
        base
    };
    let mut i = 0;
    while out.len() < 8 && !fallback.is_empty() {
        out.push(fallback[i % fallback.len()]);
        i += 1;
    }
    while out.len() < 8 {
        out.push(lch(0.5, 0.0, 0.0));
    }
    out
}

/// Each slot in turn (red, yellow, orange, brown, green, then cool)
/// takes the remaining color nearest its typical color, by hue and
/// chroma together; the image's chroma is scaled so its mean matches
/// the typical colors' mean first.
pub fn place(cands: &[Lch], light: bool) -> [Lch; 8] {
    let sc = if light { LIGHT_SLOT_C } else { SLOT_C };
    let mean = cands.iter().map(|c| c.c).sum::<f64>() / 8.0;
    let k = (sc.iter().sum::<f64>() / 8.0) / if mean > 0.0 { mean } else { 1.0 };
    let dist = |c: &Lch, slot: usize| {
        let (a, b) = (
            c.c * k * c.h.to_radians().cos(),
            c.c * k * c.h.to_radians().sin(),
        );
        let r = ANCHOR_H[slot].to_radians();
        let (ta, tb) = (sc[slot] * r.cos(), sc[slot] * r.sin());
        ((a - ta).powi(2) + (b - tb).powi(2)).sqrt()
    };
    let mut left: Vec<Lch> = cands.to_vec();
    let mut out = [lch(0.0, 0.0, 0.0); 8];
    for slot in ORDER {
        let i = (0..left.len())
            .reduce(|a, b| {
                let (da, db) = (dist(&left[a], slot), dist(&left[b], slot));
                if db < da || (db == da && left[b].h < left[a].h) {
                    b
                } else {
                    a
                }
            })
            .expect("eight candidates");
        out[slot] = left.remove(i);
    }
    out
}
