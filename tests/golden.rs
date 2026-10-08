//! Golden tests: the mapping engine's output is pinned byte-for-byte
//! against the snapshot fixtures in `tests/fixtures`.  The injected
//! clusters isolate the engine from extraction drift (resize kernels
//! across image libraries are not bit-identical).

use std::path::{Path, PathBuf};

use rehue::extract::Cluster;
use rehue::register::{retint, MapConfig};
use rehue::scheme::Scheme;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn solarized_mapping_matches_snapshot() {
    let scheme = Scheme::parse_file(&fixture("solarized-dark.yaml")).expect("fixture parses");
    let slot_hexes = scheme.slot_hexes().expect("fixture has 16 slots");
    // Extraction snapshot for the solarized wallpaper fixture, at full
    // precision.
    let clusters = vec![
        Cluster {
            hue: 222.10166864883163,
            weight: 0.5851796287736896,
            chroma: 0.046325400310273546,
        },
        Cluster {
            hue: 23.071858500846655,
            weight: 0.4148203712263105,
            chroma: 0.12484408205764291,
        },
    ];
    let mapped = retint(&slot_hexes, &clusters, &MapConfig::default()).expect("engine runs");
    let rendered = scheme.render(&mapped);
    let expected =
        std::fs::read_to_string(fixture("solarized-mapped.yaml")).expect("fixture is readable");
    assert_eq!(rendered, expected);
}

/// A chromatic gradient touching the whole hue circle, in memory.
fn gradient_image(size: (u32, u32)) -> image::DynamicImage {
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

#[test]
fn extraction_is_deterministic() {
    let image = gradient_image((128, 96));
    let params = rehue::extract::ExtractionParams::default();
    let first = rehue::extract::extract_hues_dynamic(&image, &params);
    let second = rehue::extract::extract_hues_dynamic(&image, &params);
    assert_eq!(first, second);
    assert!(!first.is_empty(), "gradient yields clusters");
}

#[test]
fn map_wal_is_deterministic() {
    let scheme = Scheme::parse_file(&fixture("solarized-dark.yaml")).expect("fixture parses");
    let slot_hexes = scheme.slot_hexes().expect("fixture has 16 slots");
    let slots: Vec<rehue::map_wal::SlotPalette> = rehue::scheme::BASE_SLOTS
        .iter()
        .copied()
        .zip(&slot_hexes)
        .map(|(slot, hex)| rehue::map_wal::SlotPalette {
            slot,
            lch: rehue::color::hex_to_oklch(hex).expect("normalized hex"),
        })
        .collect();
    let neutral: Vec<usize> = (0..16).filter(|i| slots[*i].lch.c < 0.08).collect();

    let image = gradient_image((64, 64));
    let rgb = image.to_rgb8();
    let pixels = rgb.as_raw();

    let config = rehue::map_wal::RemapConfig {
        harmonize: 0.8,
        quantize: 0.4,
        ..Default::default()
    };
    let first = rehue::map_wal::apply(pixels, &slots, &neutral, &config);
    let second = rehue::map_wal::apply(pixels, &slots, &neutral, &config);
    assert_eq!(first.pixels, second.pixels);
    assert_eq!(first.coverage, second.coverage);
}
