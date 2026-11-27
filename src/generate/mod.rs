//! Scheme generation: a base16 scheme from a wallpaper alone.
//!
//! The image sets everything: neutrals tinted with its leading hue and
//! as light or dark as its own ends, accents from its colors by one of
//! the methods, at a chroma level that follows how colorful it is.
//! Every accent keeps 4.5:1 on base00, and base08 and base0B stay apart
//! for deuteranopes.  Deterministic: the same image gives the same
//! scheme.

pub mod analysis;
mod finish;
pub mod math;
mod methods;
mod rswal;

use crate::color::Lch;
use crate::extract::Cluster;
use analysis::Analysis;
use math::{de, gamut, lch, wcag};

pub use finish::hold_contrast;

/// How the accents are made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Method {
    /// Picks the method; found for now.
    Auto,
    /// The image's colors in the slots their hues fit, gaps derived.
    Found,
    /// The set of found and derived colors that fits best while keeping
    /// accents apart.
    Distinct,
    /// The bundled schemes' usual accents.
    Conventional,
    /// Every hue family with its own lightness and chroma.
    ImageHues,
    /// The leading hue in lightness and chroma steps.
    Mono,
    Analogous,
    Complementary,
    SplitComplementary,
    Triadic,
    /// The triad, gaps halved until there are eight hues.
    TriadicFill,
    Square,
    /// The image's own colors, as pywal picks them.
    Rswal,
}

impl Method {
    /// Every method name, for help and errors.
    pub const NAMES: [&'static str; 13] = [
        "auto",
        "found",
        "distinct",
        "conventional",
        "image-hues",
        "mono",
        "analogous",
        "complementary",
        "split-complementary",
        "triadic",
        "triadic-fill",
        "square",
        "rswal",
    ];

    pub fn parse(name: &str) -> Result<Method, String> {
        serde_json::from_value(serde_json::Value::String(name.to_string()))
            .map_err(|_| format!("unknown method '{name}' ({})", Method::NAMES.join(", ")))
    }

    pub fn name(self) -> &'static str {
        Method::NAMES[self as usize]
    }

    /// What `auto` stands for in this version.
    pub fn resolve(self) -> Method {
        if self == Method::Auto {
            Method::Found
        } else {
            self
        }
    }
}

/// Dark or light scheme.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
pub enum Variant {
    /// Light when the image's median lightness is 0.6 or more.
    #[default]
    Auto,
    Dark,
    Light,
}

/// The image's median lightness from which `auto` picks light.
pub const LIGHT_FROM: f64 = 0.6;

/// A generated scheme.
#[derive(Debug, Clone)]
pub struct Generated {
    /// base00..base0F.
    pub slots: [Lch; 16],
    /// The method that ran (`auto` resolved; conventional for an image
    /// with no color).
    pub method: Method,
    pub light: bool,
    /// One-line notes for the run summary.
    pub notes: Vec<String>,
}

/// The most saturated image color under this is no color at all.
const ACHROMATIC: f64 = 0.01;

/// Generate a scheme from an analysed image.
pub fn generate(a: &Analysis, asked: Method, variant: Variant) -> Generated {
    let mut method = asked.resolve();
    let light = match variant {
        Variant::Auto => a.p50 >= LIGHT_FROM,
        Variant::Dark => false,
        Variant::Light => true,
    };
    let mut notes = Vec::new();
    let max_c = a.fine.iter().map(|c| c.c).fold(0.0, f64::max);
    let achromatic = max_c < ACHROMATIC;
    if achromatic && method != Method::Conventional {
        if asked != Method::Auto {
            notes.push("image has no color; using conventional".to_string());
        }
        method = Method::Conventional;
    }
    let seed = methods::seed(&a.families, &a.clusters, &a.fine);
    let (seed_h, tint) = if achromatic {
        (0.0, 0.0)
    } else {
        (seed.h, 1.0)
    };
    let neutrals = if light {
        finish::light_ramp(seed_h, finish::LIGHT_TINT * tint, a.p90)
    } else {
        finish::dark_ramp(seed_h, finish::DARK_TINT * tint, a.p10)
    };
    let stand_in = [Cluster {
        hue: seed.h,
        weight: 1.0,
        chroma: seed.c,
    }];
    let fams: &[Cluster] = if a.families.is_empty() {
        &stand_in
    } else {
        &a.families
    };
    let anchors = methods::anchors(&a.families);
    let raw: [Lch; 8] = match method {
        Method::Found | Method::Auto => methods::found(&a.fine),
        Method::Distinct => methods::distinct(&a.fine),
        Method::Rswal => rswal::place(&rswal::colors(&a.pixels, a.width), light),
        Method::Conventional if achromatic => std::array::from_fn(|i| {
            lch(methods::SLOT_L[i], methods::SLOT_C[i], methods::ANCHOR_H[i])
        }),
        _ => {
            let name = method.name();
            methods::assign(
                &methods::candidates(name, &seed, fams, &a.clusters),
                &anchors,
            )
        }
    };
    let acc: [Lch; 8] = if method == Method::Conventional {
        if light {
            std::array::from_fn(|i| lch(raw[i].l, methods::LIGHT_SLOT_C[i], raw[i].h))
        } else {
            raw
        }
    } else {
        let mean = raw.iter().map(|x| x.c).sum::<f64>() / 8.0;
        let k = finish::chroma_level(&a.fine) / if mean > 0.0 { mean } else { 1.0 };
        raw.map(|x| lch(x.l, x.c * k, x.h))
    };
    let accents = if light {
        finish::finish_light(&acc, &neutrals[0])
    } else {
        finish::finish_dark(&acc, &neutrals[0])
    };
    let mut slots = [lch(0.0, 0.0, 0.0); 16];
    slots[..8].copy_from_slice(&neutrals);
    slots[8..].copy_from_slice(&accents);
    Generated {
        slots,
        method,
        light,
        notes,
    }
}

/// Accent pairs closer than this get pushed apart, to TARGET.
const TRIGGER: f64 = 0.02;
const TARGET: f64 = 0.04;

/// Near-identical accent pairs pushed apart to just distinct: the
/// lighter rises (keeping its chroma to lightness ratio), the darker
/// drops while it keeps 4.5:1; if neither can move in lightness, the
/// darker gains chroma and the lighter loses some.
pub fn nudge(slots: &mut [Lch; 16]) -> bool {
    let base00 = slots[0];
    let acc = &mut slots[8..];
    let step = 0.004;
    let mut moved = false;
    for _ in 0..4 {
        for i in 0..8 {
            for j in i + 1..8 {
                if de(&acc[i], &acc[j]) >= TRIGGER {
                    continue;
                }
                let (up, dn) = if acc[i].l >= acc[j].l { (i, j) } else { (j, i) };
                let can_l = acc[up].l + step <= 0.92
                    || wcag(&lch(acc[dn].l - step, acc[dn].c, acc[dn].h), &base00) >= 4.5;
                for _ in 0..80 {
                    if de(&acc[i], &acc[j]) >= TARGET {
                        break;
                    }
                    let before = de(&acc[i], &acc[j]);
                    if can_l {
                        let lower = gamut(lch(acc[dn].l - step, acc[dn].c, acc[dn].h));
                        if wcag(&lower, &base00) >= 4.5 {
                            acc[dn] = lower;
                        }
                        if acc[up].l + step <= 0.92 {
                            let x = acc[up];
                            acc[up] = gamut(lch(x.l + step, x.c * x.l / (x.l + step), x.h));
                        }
                    } else {
                        acc[dn] = gamut(lch(acc[dn].l, acc[dn].c + step, acc[dn].h));
                        acc[up] = lch(acc[up].l, (acc[up].c - step).max(0.03), acc[up].h);
                    }
                    if de(&acc[i], &acc[j]) <= before + 1e-6 {
                        break;
                    }
                }
                moved = true;
            }
        }
    }
    moved
}

/// Pull a role's chroma (`lightness: false`) or lightness toward its
/// mean, the mean kept: 0 leaves it, 1 makes it even.  Accents keep
/// 4.5:1 on base00; where sRGB or contrast stops a color short, 1 is
/// not quite even.
pub fn even(slots: &mut [Lch; 16], range: std::ops::Range<usize>, amount: f64, lightness: bool) {
    if amount <= 0.0 || range.len() < 2 {
        return;
    }
    let accents = range.start >= 8;
    let base00 = slots[0];
    let light = slots[0].l > slots[5].l;
    let n = range.len() as f64;
    let mean = slots[range.clone()]
        .iter()
        .map(|x| if lightness { x.l } else { x.c })
        .sum::<f64>()
        / n;
    for x in &mut slots[range] {
        let moved = if lightness {
            lch(mean + (x.l - mean) * (1.0 - amount), x.c, x.h)
        } else {
            lch(x.l, (mean + (x.c - mean) * (1.0 - amount)).max(0.0), x.h)
        };
        *x = if accents {
            hold_contrast(moved, &base00, light)
        } else {
            gamut(moved)
        };
    }
}
