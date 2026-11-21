//! Wallpaper hue extraction: chroma^2-weighted hue histogram over a
//! downsized image, then a deterministic circular k-means (greedy
//! histogram seeding, fixed iteration count - no RNG anywhere).
//! Resize/resampling kernels differ across image toolkits by a rounding
//! step on a few pixels; the golden fixtures pin this pipeline's output.

use std::path::Path;

use image::{DynamicImage, ImageReader, imageops};

use crate::color::{circ_dist, floor_mod, rgb_to_oklch, weighted_circ_mean};

/// Extraction options, read from the `scheme` / `wal` / `inspect`
/// config JSON with kebab-case keys.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ExtractionParams {
    pub max_dimension: u32,
    pub min_pixel_chroma: f64,
    pub lightness_range: [f64; 2],
    pub max_families: usize,
    pub seed_spacing: f64,
    pub iterations: u32,
    pub merge_within: f64,
    pub min_family_weight: f64,
    pub min_accent_chroma: f64,
}

impl ExtractionParams {
    /// Every config key with a one-line description, in declaration
    /// order.  The docs tables render from this; a test checks it names
    /// exactly the serialized keys.
    pub const KEYS: [(&'static str, &'static str); 9] = [
        (
            "max-dimension",
            "work at this capped resolution (Lanczos resize)",
        ),
        ("min-pixel-chroma", "ignore pixels below this chroma"),
        (
            "lightness-range",
            "only pixels inside this lightness band vote",
        ),
        ("max-families", "at most this many families survive"),
        (
            "seed-spacing",
            "histogram seeding: minimum separation between seeds",
        ),
        ("iterations", "fixed k-means iteration count"),
        ("merge-within", "merge families closer than this"),
        ("min-family-weight", "drop families lighter than this mass"),
        (
            "min-accent-chroma",
            "a family must be at least this chromatic to claim an accent slot",
        ),
    ];
}

impl Default for ExtractionParams {
    fn default() -> Self {
        Self {
            max_dimension: 256,
            min_pixel_chroma: 0.04,
            lightness_range: [0.10, 0.92],
            max_families: 6,
            seed_spacing: 25.0,
            iterations: 30,
            merge_within: 18.0,
            min_family_weight: 0.02,
            min_accent_chroma: 0.06,
        }
    }
}

/// A surviving hue cluster; `weight` is a fraction of total chroma^2 mass.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Cluster {
    pub hue: f64,
    pub weight: f64,
    pub chroma: f64,
}

/// Raw mass record of one k-means cluster.
#[derive(Debug, Clone, Copy)]
struct Record {
    /// chroma^2-weighted sine/cosine sums (circular mean inputs)
    s: f64,
    cos_sum: f64,
    mass: f64,
    /// sum of chroma * mass, for the mean cluster chroma
    chroma_mass: f64,
    hue: f64,
}

impl Record {
    fn to_cluster(self, total: f64) -> Cluster {
        Cluster {
            hue: self.hue,
            weight: self.mass / total,
            chroma: self.chroma_mass / self.mass,
        }
    }
}

pub fn extract_hues(path: &Path, params: &ExtractionParams) -> Result<Vec<Cluster>, String> {
    let image = ImageReader::open(path)
        .map_err(|e| format!("can't read wallpaper {}: {}", path.display(), e))?
        .decode()
        .map_err(|e| format!("can't decode wallpaper {}: {}", path.display(), e))?;
    Ok(extract_hues_dynamic(&image, params))
}

/// Extraction from an image already in memory (used by tests).
pub fn extract_hues_dynamic(image: &DynamicImage, params: &ExtractionParams) -> Vec<Cluster> {
    let rgb = image.to_rgb8();
    let scale =
        1.0f64.min(f64::from(params.max_dimension) / f64::from(rgb.width().max(rgb.height())));
    let resized: image::RgbImage = if scale < 1.0 {
        let w = 1u32.max((f64::from(rgb.width()) * scale).round_ties_even() as u32);
        let h = 1u32.max((f64::from(rgb.height()) * scale).round_ties_even() as u32);
        imageops::resize(&rgb, w, h, imageops::FilterType::Lanczos3)
    } else {
        rgb
    };

    let mut hist_mass = [0.0f64; 360];
    let mut hist_chroma = [0.0f64; 360];
    let lo = params.lightness_range[0];
    let hi = params.lightness_range[1];
    for pixel in resized.pixels() {
        let lch = rgb_to_oklch(&[pixel[0], pixel[1], pixel[2]]);
        if lch.c < params.min_pixel_chroma || !(lo <= lch.l && lch.l <= hi) {
            continue;
        }
        let bin = floor_mod(lch.h, 360.0) as usize % 360;
        hist_mass[bin] += lch.c * lch.c;
        hist_chroma[bin] += lch.c * lch.c * lch.c;
    }

    if hist_mass.iter().all(|m| *m <= 0.0) {
        return Vec::new(); // greyscale wallpaper
    }

    let records = circular_kmeans(&hist_mass, &hist_chroma, params);
    let total: f64 = records.iter().map(|r| r.mass).sum();
    let mut survivors: Vec<Cluster> = records
        .iter()
        .filter(|r| r.mass / total >= params.min_family_weight)
        .map(|r| r.to_cluster(total))
        .collect();
    if survivors.is_empty() {
        // Keep the heaviest cluster, promoted to the whole palette.
        let heaviest = records
            .iter()
            .max_by(|a, b| {
                a.mass
                    .partial_cmp(&b.mass)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("records are non-empty at this point");
        survivors.push(Cluster {
            hue: heaviest.hue,
            weight: 1.0,
            chroma: heaviest.chroma_mass / heaviest.mass,
        });
    }
    survivors.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.hue
                    .partial_cmp(&b.hue)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    survivors.truncate(params.max_families);
    survivors
}

/// Deterministic circular k-means over a 360-bin hue histogram.
///
/// Seeding greedily picks the heaviest bins sitting at least
/// `seed_spacing` from every already-picked seed - reproducible,
/// unlike random k-means++ initialisations.
fn circular_kmeans(
    hist_mass: &[f64; 360],
    hist_chroma: &[f64; 360],
    params: &ExtractionParams,
) -> Vec<Record> {
    let bins: Vec<(usize, f64, f64)> = (0..360)
        .filter(|i| hist_mass[*i] > 0.0)
        .map(|i| (i, hist_mass[i], hist_chroma[i]))
        .collect();
    if bins.is_empty() {
        return Vec::new();
    }

    let mut order: Vec<usize> = (0..360).collect();
    order.sort_by(|i, j| {
        hist_mass[*j]
            .partial_cmp(&hist_mass[*i])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(i.cmp(j))
    });
    let mut seeds: Vec<f64> = Vec::new();
    for i in order {
        if hist_mass[i] <= 0.0 {
            break; // sorted descending: the rest are zero
        }
        if seeds
            .iter()
            .all(|s| circ_dist(i as f64, *s) >= params.seed_spacing)
        {
            seeds.push(i as f64);
        }
        if seeds.len() >= params.max_families {
            break;
        }
    }

    let mut centers: Vec<f64> = if !seeds.is_empty() {
        seeds
    } else {
        let hues: Vec<f64> = bins.iter().map(|(i, ..)| *i as f64).collect();
        let masses: Vec<f64> = bins.iter().map(|(_, m, _)| *m).collect();
        vec![weighted_circ_mean(&hues, &masses)]
    };

    let assign = |round_centers: &[f64]| -> Vec<Record> {
        let mut acc = vec![[0.0f64; 4]; round_centers.len()]; // s, cos, mass, chroma_mass
        for (bin, mass, chroma) in &bins {
            let mut best = 0usize;
            let mut best_d = f64::INFINITY;
            for (ci, center) in round_centers.iter().enumerate() {
                let d = circ_dist(*bin as f64, *center);
                if d < best_d {
                    best_d = d;
                    best = ci;
                }
            }
            let rad = (*bin as f64).to_radians();
            acc[best][0] += mass * rad.sin();
            acc[best][1] += mass * rad.cos();
            acc[best][2] += mass;
            acc[best][3] += chroma;
        }
        acc.iter()
            .filter(|a| a[2] > 0.0)
            .map(|a| Record {
                s: a[0],
                cos_sum: a[1],
                mass: a[2],
                chroma_mass: a[3],
                hue: floor_mod(a[0].atan2(a[1]).to_degrees(), 360.0),
            })
            .collect()
    };

    // Fixed iteration count keeps seed -> output pure.
    // Fixed iteration count keeps seed -> output pure.  The sequence is
    // initial assignment, iterations - 1 refinement passes, then
    // one final pass so each record's mass/chroma matches its settled
    // centre.
    let mut records = assign(&centers);
    for _ in 1..params.iterations {
        centers = records.iter().map(|r| r.hue).collect();
        if centers.is_empty() {
            return Vec::new();
        }
        records = assign(&centers);
    }
    // One final pass so each record's mass/chroma matches its settled centre.
    records = assign(&centers);
    records.sort_by(|a, b| {
        b.mass
            .partial_cmp(&a.mass)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Merge sweep: heavier clusters absorb nearer ones (mass and chroma
    // accumulators are additive, so the combined hue/chroma stay exact).
    let mut merged: Vec<Record> = Vec::new();
    for rec in records {
        let mut absorbed = false;
        for m in merged.iter_mut() {
            if circ_dist(m.hue, rec.hue) < params.merge_within {
                m.s += rec.s;
                m.cos_sum += rec.cos_sum;
                m.mass += rec.mass;
                m.chroma_mass += rec.chroma_mass;
                absorbed = true;
                break;
            }
        }
        if !absorbed {
            merged.push(rec);
        }
    }
    merged
}
