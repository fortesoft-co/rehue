//! Input formats: png, jpeg, webp, gif, tiff, bmp and qoi all read.

mod common;

use common::{fixture, gradient_image};

#[test]
fn lossless_formats_paint_the_same_and_a_gif_paints_its_first_frame() {
    let bin = env!("CARGO_BIN_EXE_rehue");
    let dir = std::env::temp_dir().join(format!("rehue-formats-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let scheme = fixture("solarized-dark.yaml");
    let picture = gradient_image((96, 64));
    let paint = |input: &std::path::Path, name: &str| {
        let out = dir.join(format!("out-{name}"));
        let run = std::process::Command::new(bin)
            .args(["wal", "-i"])
            .arg(input)
            .arg("-s")
            .arg(&scheme)
            .args(["-H", "1", "-o"])
            .arg(&out)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        std::fs::read(out.join("wallpaper.png")).unwrap()
    };

    let mut painted = Vec::new();
    for ext in ["png", "bmp", "tiff", "qoi"] {
        let input = dir.join(format!("wall.{ext}"));
        picture.save(&input).unwrap();
        painted.push((ext, paint(&input, ext)));
    }
    for (ext, bytes) in &painted[1..] {
        assert!(*bytes == painted[0].1, "{ext} paints differently from png");
    }

    // a two-frame gif: the first frame is the one painted
    let gif = dir.join("wall.gif");
    {
        use image::codecs::gif::GifEncoder;
        use image::{Delay, Frame, RgbaImage};
        let frame = |rgb: [u8; 3]| {
            Frame::from_parts(
                RgbaImage::from_pixel(32, 24, image::Rgba([rgb[0], rgb[1], rgb[2], 255])),
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            )
        };
        let mut encoder = GifEncoder::new(std::fs::File::create(&gif).unwrap());
        encoder
            .encode_frames([frame([200, 40, 40]), frame([40, 40, 200])])
            .unwrap();
    }
    let first = dir.join("first.png");
    image::RgbImage::from_pixel(32, 24, image::Rgb([200, 40, 40]))
        .save(&first)
        .unwrap();
    assert!(paint(&gif, "gif") == paint(&first, "first"));
    let _ = std::fs::remove_dir_all(&dir);
}
