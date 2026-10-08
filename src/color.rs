//! Colour math.  Three details are load-bearing for byte-level parity of
//! outputs across rebuilds:
//!
//! * hue arithmetic uses FLOOR-modulo: Rust's `%` truncates toward zero
//!   while Python's `%` floors; every hue helper below depends on the
//!   always-positive result
//! * the cube root is `powf(1.0 / 3.0)`, mirroring CPython's
//!   `x ** (1.0 / 3.0)` - NOT `cbrt()`, which can differ in the last ulp
//!   from libm's `pow`
//! * final 8-bit rounding is ties-to-even, matching Python's `round()`

/// sRGB linearization: linear `rgb -> lms'` matrix.
pub const OKLAB_M1: [[f64; 3]; 3] = [
    [0.4122214708, 0.5363325363, 0.0514459929],
    [0.2119034982, 0.6806995451, 0.1073969566],
    [0.0883024619, 0.2817188376, 0.6299787005],
];

/// `lms' -> OKLab` matrix (Ottosson's published constants).
pub const OKLAB_M2: [[f64; 3]; 3] = [
    [0.2104542553, 0.7936177850, -0.0040720468],
    [1.9779984951, -2.4285922050, 0.4505937099],
    [0.0259040371, 0.7827717662, -0.8086757660],
];

/// `OKLab -> lms'` matrix.
pub const OKLAB_M2_INV: [[f64; 3]; 3] = [
    [1.0, 0.3963377774, 0.2158037573],
    [1.0, -0.1055613458, -0.0638541728],
    [1.0, -0.0894841775, -1.2914855480],
];

/// `lms' -> linear rgb` matrix.
pub const OKLAB_M1_INV: [[f64; 3]; 3] = [
    [4.0767416621, -3.3077115913, 0.2309699292],
    [-1.2684380046, 2.6097574011, -0.3413193965],
    [-0.0041960863, -0.7034186147, 1.7076147010],
];

/// A color in cylindrical OKLCH form: lightness 0..1, chroma 0..~0.37,
/// hue in degrees (always kept in 0..360 via floor-modulo).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

fn dot(row: &[f64; 3], vec: &[f64; 3]) -> f64 {
    row[0] * vec[0] + row[1] * vec[1] + row[2] * vec[2]
}

fn srgb_to_linear(v: u8) -> f64 {
    let v = f64::from(v) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Inverse transfer curve with clamping; ties-to-even rounding matches
/// Python's `round()` on the 0..255 scale.
pub fn linear_to_srgb(v: f64) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let out = if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (out * 255.0).round_ties_even().clamp(0.0, 255.0) as u8
}

pub fn rgb_to_oklab(rgb: &[u8; 3]) -> [f64; 3] {
    let lin = [
        srgb_to_linear(rgb[0]),
        srgb_to_linear(rgb[1]),
        srgb_to_linear(rgb[2]),
    ];
    let l = dot(&OKLAB_M1[0], &lin);
    let m = dot(&OKLAB_M1[1], &lin);
    let s = dot(&OKLAB_M1[2], &lin);
    let lms = [
        l.powf(1.0 / 3.0),
        m.powf(1.0 / 3.0),
        s.powf(1.0 / 3.0),
    ];
    [
        dot(&OKLAB_M2[0], &lms),
        dot(&OKLAB_M2[1], &lms),
        dot(&OKLAB_M2[2], &lms),
    ]
}

pub fn oklab_to_rgb(lab: &[f64; 3]) -> [u8; 3] {
    let mul = |m: &[[f64; 3]; 3], v: &[f64; 3]| -> [f64; 3] {
        [dot(&m[0], v), dot(&m[1], v), dot(&m[2], v)]
    };
    let lms_dot = mul(&OKLAB_M2_INV, lab);
    let lms = [lms_dot[0].powi(3), lms_dot[1].powi(3), lms_dot[2].powi(3)];
    mul(&OKLAB_M1_INV, &lms).map(linear_to_srgb)
}

/// Floor-modulo: Python's `%`, not Rust's truncating remainder.
pub fn floor_mod(a: f64, m: f64) -> f64 {
    let r = a % m;
    if r < 0.0 {
        r + m
    } else {
        r
    }
}

/// Circular angular distance in degrees (0..180).
pub fn circ_dist(a: f64, b: f64) -> f64 {
    let d = floor_mod(a - b, 360.0);
    d.min(360.0 - d)
}

/// Shortest-arc interpolation between two hues; `t` in [0, 1].
pub fn circ_lerp(a: f64, b: f64, t: f64) -> f64 {
    let d = floor_mod(b - a + 180.0, 360.0) - 180.0;
    floor_mod(a + d * t, 360.0)
}

/// Prominence-weighted circular mean of hues.
pub fn weighted_circ_mean(angles: &[f64], weights: &[f64]) -> f64 {
    if angles.is_empty() {
        return 0.0;
    }
    let mut sin_sum = 0.0;
    let mut cos_sum = 0.0;
    for (angle, weight) in angles.iter().zip(weights.iter()) {
        let rad = angle.to_radians();
        sin_sum += weight * rad.sin();
        cos_sum += weight * rad.cos();
    }
    floor_mod(sin_sum.atan2(cos_sum).to_degrees(), 360.0)
}

pub fn rgb_to_oklch(rgb: &[u8; 3]) -> Lch {
    let lab = rgb_to_oklab(rgb);
    Lch {
        l: lab[0],
        c: (lab[1] * lab[1] + lab[2] * lab[2]).sqrt(),
        h: floor_mod(lab[2].atan2(lab[1]).to_degrees(), 360.0),
    }
}

pub fn oklch_to_rgb(lch: &Lch) -> [u8; 3] {
    let rad = lch.h.to_radians();
    oklab_to_rgb(&[lch.l, lch.c * rad.cos(), lch.c * rad.sin()])
}

pub fn oklch_to_hex(lch: &Lch) -> String {
    let rgb = oklch_to_rgb(lch);
    format!("{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

pub fn hex_to_rgb(hex: &str) -> Result<[u8; 3], String> {
    let part = |i: usize| {
        u8::from_str_radix(&hex[i..i + 2], 16)
            .map_err(|_| format!("bad hex colour {}", hex))
    };
    Ok([part(0)?, part(2)?, part(4)?])
}

pub fn hex_to_oklch(hex: &str) -> Result<Lch, String> {
    Ok(rgb_to_oklch(&hex_to_rgb(hex)?))
}

/// Python `round(x, digits)`-equivalent: decimal rounding, ties to even.
pub fn round_value(x: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    (x * factor).round_ties_even() / factor
}

/// Normalize hex: strips surrounding `#`, expands
/// three-digit shorthand, validates charset, lowercases.
pub fn normalize_hex(value: &str) -> Result<String, String> {
    let v = value.trim().trim_matches('#').to_ascii_lowercase();
    let v = if v.len() == 3 {
        v.chars()
            .map(|ch| {
                let s = ch.to_string();
                s.clone() + &s
            })
            .collect::<String>()
    } else {
        v
    };
    let valid = v.len() == 6
        && v.chars()
            .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch));
    if !valid {
        return Err(format!(
            "scheme value {:?} is not a six-digit hex colour",
            value
        ));
    }
    Ok(v)
}

/// Round-trip check of the OKLab constants.  Runs before any colour work.
pub fn self_test() {
    for rgb in [[255u8, 0, 0], [255, 255, 255], [11, 108, 255], [30, 30, 34]] {
        let back = oklab_to_rgb(&rgb_to_oklab(&rgb));
        let ok = (0..3).all(|i| ((back[i] as i32) - (rgb[i] as i32)).abs() <= 2);
        assert!(ok, "OKLab round-trip failed: {:?} -> {:?}", rgb, back);
    }
}
