//! Grain: small, centered, monochrome, the same every run, new each frame.

use rehue::grain::{apply, offset};

#[test]
fn grain_stays_within_its_amount_and_centers_on_zero() {
    let amount = 4.0;
    let (mut sum, mut n) = (0.0, 0.0);
    for y in 0..200 {
        for x in 0..200 {
            let d = offset(x, y, 0, amount);
            assert!(d.abs() <= amount, "{d}");
            sum += d;
            n += 1.0;
        }
    }
    assert!((sum / n).abs() < 0.05, "mean {}", sum / n);
}

#[test]
fn grain_is_the_same_every_run_and_new_each_frame() {
    assert_eq!(offset(10, 20, 3, 4.0), offset(10, 20, 3, 4.0));
    let differs = (0..100)
        .filter(|&x| offset(x, 7, 0, 4.0) != offset(x, 7, 1, 4.0))
        .count();
    assert!(
        differs > 90,
        "{differs} of 100 pixels change between frames"
    );
}

#[test]
fn grain_moves_all_three_channels_together() {
    let mut px = vec![100u8; 64 * 32 * 3];
    apply(&mut px, 64, 0, 6.0);
    for p in px.as_chunks::<3>().0 {
        assert!(p[0] == p[1] && p[1] == p[2]);
    }
    assert!(px.iter().any(|&v| v != 100));
}
