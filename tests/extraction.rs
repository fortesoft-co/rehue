//! Extraction: determinism on a synthetic gradient.

mod common;

use common::gradient_image;

#[test]
fn extraction_is_deterministic() {
    let image = gradient_image((128, 96));
    let params = rehue::extract::ExtractionParams::default();
    let first = rehue::extract::extract_hues_dynamic(&image, &params);
    let second = rehue::extract::extract_hues_dynamic(&image, &params);
    assert_eq!(first, second);
    assert!(!first.is_empty(), "gradient yields clusters");
}

/// A video's frames, stacked as sampled: each is measured at its own
/// size, so a small bright light in every frame still counts (squeezing
/// the whole stack to the working size averaged it into the dark).
#[test]
fn video_frames_are_measured_one_by_one() {
    let (w, h, frames) = (480u32, 270u32, 32u32);
    let stack = image::RgbImage::from_fn(w, h * frames, |x, y| {
        let (fx, fy) = (x, y % h);
        if (200..232).contains(&fx) && (120..152).contains(&fy) {
            image::Rgb([255, 140, 20]) // a city light
        } else {
            image::Rgb([20, 30, 60]) // the night sky
        }
    });
    let params = rehue::extract::ExtractionParams::default();
    let warm = |clusters: &[rehue::extract::Cluster]| {
        clusters
            .iter()
            .find(|c| rehue::color::circ_dist(c.hue, 55.0) < 25.0)
            .map_or(0.0, |c| c.chroma)
    };
    let pooled = warm(&rehue::extract::extract_hues_frames(&stack, h, &params));
    let squeezed = warm(&rehue::extract::extract_hues_dynamic(
        &image::DynamicImage::ImageRgb8(stack),
        &params,
    ));
    assert!(
        pooled > 0.12 && pooled > squeezed * 1.5,
        "the light keeps its chroma: {pooled:.3} one by one, {squeezed:.3} squeezed"
    );
}
