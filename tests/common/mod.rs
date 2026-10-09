//! Shared test inputs: repo-fixture paths and synthetic palettes/images.

use std::path::{Path, PathBuf};

use rehue::color::{Lch, hex_to_oklch};
use rehue::extract::Cluster;
use rehue::map_wal::SlotPalette;
use rehue::scheme::{BASE_SLOTS, Scheme};

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

pub fn solarized() -> (Vec<String>, Vec<SlotPalette>) {
    let scheme = Scheme::parse_file(&fixture("solarized-dark.yaml")).expect("fixture parses");
    let slot_hexes = scheme.slot_hexes().expect("fixture has 16 slots");
    let slots: Vec<SlotPalette> = BASE_SLOTS
        .iter()
        .copied()
        .zip(&slot_hexes)
        .map(|(slot, hex)| SlotPalette {
            slot,
            lch: hex_to_oklch(hex).expect("normalized hex"),
        })
        .collect();
    (slot_hexes, slots)
}

/// Four vivid families in extraction (weight) order; chroma is high
/// enough that the 8-bit render keeps hue within ~2 degrees.
pub fn four_families() -> Vec<Cluster> {
    [40.0, 130.0, 250.0, 330.0]
        .iter()
        .enumerate()
        .map(|(i, h)| Cluster {
            hue: *h,
            weight: 0.4 - i as f64 * 0.1,
            chroma: 0.25,
        })
        .collect()
}

/// A chromatic gradient touching the whole hue circle, in memory.
pub fn gradient_image(size: (u32, u32)) -> image::DynamicImage {
    let (w, h) = size;
    let mut img = image::RgbImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let u = f64::from(x) / f64::from(w) * 360.0;
            let rad = u.to_radians();
            img.put_pixel(
                x,
                y,
                image::Rgb(
                    [
                        f64::from(y) / f64::from(h),
                        0.5 + 0.4 * rad.cos(),
                        0.5 + 0.4 * rad.sin(),
                    ]
                    .map(|v: f64| (v.clamp(0.0, 1.0) * 255.0) as u8),
                ),
            );
        }
    }
    image::DynamicImage::ImageRgb8(img)
}
