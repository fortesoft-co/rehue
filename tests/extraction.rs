//! The extraction stage: determinism on the synthetic gradient.

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
