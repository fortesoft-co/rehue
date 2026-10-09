//! The enhance stage: the model table, param validation, and the
//! deterministic Lanczos pipeline.

mod common;

use std::path::{Path, PathBuf};

use common::gradient_image;
use rehue::enhance::{EnhanceConfig, SrModel};

fn enhance_temp(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rehue-test-enhance-{}-{}", std::process::id(), tag))
}

#[test]
fn enhance_model_table() {
    assert_eq!(SrModel::Photos.tool_name(), "realesrgan-x4plus");
    assert_eq!(SrModel::Anime.tool_name(), "realesrgan-x4plus-anime");
    assert_eq!(SrModel::Photos.native_scale(), 4);
    assert_eq!(SrModel::Anime.native_scale(), 4);
}

#[test]
fn enhance_validates_its_params() {
    let temp = enhance_temp("params");
    std::fs::create_dir_all(&temp).expect("temp dir");
    let src = temp.join("src.png");
    gradient_image((48, 32)).save(&src).expect("fixture writes");

    let cfg = |upscale: u32, sr: bool, sr_tool: Option<PathBuf>| EnhanceConfig {
        upscale,
        sr,
        model: SrModel::Photos,
        sr_tool,
    };
    let err =
        rehue::enhance::run(&src, &temp, &cfg(0, false, None), None).expect_err("scale 0 rejected");
    assert!(err.contains("at least 1"), "{err}");
    let err = rehue::enhance::run(&src, &temp, &cfg(1, false, None), None)
        .expect_err("identity lanczos rejected");
    assert!(err.contains("nothing to do"), "{err}");
    let err =
        rehue::enhance::run(&src, &temp, &cfg(9, false, None), None).expect_err("lanczos cap");
    assert!(err.contains("sanity cap"), "{err}");
    let err = rehue::enhance::run(&src, &temp, &cfg(8, true, None), None)
        .expect_err("beyond native rejected before any tool runs");
    assert!(err.contains("natively"), "{err}");
    let err = rehue::enhance::run(&src, &temp, &cfg(2, true, Some(temp.join("absent"))), None)
        .expect_err("missing tool");
    assert!(err.contains("not a file"), "{err}");

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn enhance_lanczos_scales_deterministically() {
    let temp = enhance_temp("lanczos");
    std::fs::create_dir_all(&temp).expect("temp dir");
    let src = temp.join("src.png");
    gradient_image((48, 32)).save(&src).expect("fixture writes");
    let cfg = EnhanceConfig {
        upscale: 2,
        sr: false,
        model: SrModel::Photos,
        sr_tool: None,
    };
    let first = rehue::enhance::run(&src, &temp.join("a"), &cfg, None).expect("enhance runs");
    assert_eq!(first, temp.join("a").join("src.png"), "stem-named artifact");
    let second = rehue::enhance::run(&src, &temp.join("b"), &cfg, None).expect("enhance runs");
    let decoded = image::ImageReader::open(&first)
        .expect("artifact reads")
        .decode()
        .expect("artifact decodes");
    assert_eq!((decoded.width(), decoded.height()), (96, 64));
    let a = std::fs::read(&first).expect("artifact bytes");
    let b = std::fs::read(&second).expect("artifact bytes");
    assert_eq!(a, b, "lanczos enhance is byte-deterministic");

    let _ = std::fs::remove_dir_all(&temp);
}
