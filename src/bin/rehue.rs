//! `rehue` CLI: the two mapping flows as subcommands.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use rehue::color::{hex_to_oklch, hex_to_rgb, self_test};
use rehue::extract;
use rehue::register;
use rehue::map_wal;
use rehue::scheme::{Scheme, BASE_SLOTS};

#[derive(Parser)]
#[command(
    name = "rehue",
    version,
    about = "Map base16 colour schemes and wallpapers onto each other"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Map a reference scheme's structure onto a wallpaper's hues.
    MapScheme {
        #[arg(long)]
        wallpaper: PathBuf,
        #[arg(long)]
        scheme: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Map config JSON (extraction knobs + register overrides);
        /// absent means defaults, which reproduce the plain hue-mapping.
        #[arg(long, value_name = "JSON")]
        config: Option<PathBuf>,
    },
    /// Map a scheme's palette onto a wallpaper's colours (all knobs opt-in).
    MapWal {
        #[arg(long)]
        wallpaper: PathBuf,
        #[arg(long)]
        scheme: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Remap config JSON; absent means a re-encoded passthrough.
        #[arg(long, value_name = "JSON")]
        config: Option<PathBuf>,
    },
}

fn read_json<T: serde::de::DeserializeOwned + Default>(
    path: Option<&Path>,
) -> Result<T, String> {
    match path {
        None => Ok(T::default()),
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .map_err(|e| format!("can't read config {}: {}", p.display(), e))?;
            serde_json::from_str(&text)
                .map_err(|e| format!("bad config {}: {}", p.display(), e))
        }
    }
}

fn fill_rect(
    img: &mut image::RgbImage,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    rgb: [u8; 3],
) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            img.put_pixel(x, y, image::Rgb(rgb));
        }
    }
}

/// The 16-cell before/after swatch image (reference top, mapped bottom).
fn render_preview(
    original: &BTreeMap<String, String>,
    mapped: &BTreeMap<String, String>,
) -> Result<image::RgbImage, String> {
    let cell: u32 = 24;
    let gap: u32 = 4;
    let width = cell * BASE_SLOTS.len() as u32;
    let height = cell * 2 + gap;
    let mut img = image::ImageBuffer::from_pixel(width, height, image::Rgb([34u8, 34, 39]));
    for (idx, slot) in BASE_SLOTS.iter().copied().enumerate() {
        let x0 = idx as u32 * cell + 2;
        let original_hex = original
            .get(slot)
            .ok_or_else(|| format!("missing colour {}", slot))?;
        let mapped_hex = mapped
            .get(slot)
            .ok_or_else(|| format!("missing colour {}", slot))?;
        fill_rect(
            &mut img,
            x0,
            1,
            x0 + cell - 4,
            cell - 1,
            hex_to_rgb(original_hex)?,
        );
        fill_rect(
            &mut img,
            x0,
            cell + gap,
            x0 + cell - 4,
            cell * 2 + gap - 1,
            hex_to_rgb(mapped_hex)?,
        );
    }
    Ok(img)
}

fn thumb(src: &image::RgbImage) -> image::RgbImage {
    let scale = 384.0f64 / f64::from(src.width());
    let height = 1u32.max((f64::from(src.height()) * scale) as u32);
    image::imageops::resize(src, 384, height, image::imageops::FilterType::Lanczos3)
}

/// Original vs remapped, side by side.
fn write_compare(
    out: &Path,
    original: &image::RgbImage,
    remapped: &image::RgbImage,
) -> Result<(), String> {
    let left = thumb(original);
    let right = thumb(remapped);
    let height = left.height().max(right.height()) + 8;
    let mut img =
        image::ImageBuffer::from_pixel(384 * 2 + 12, height, image::Rgb([22u8, 22, 26]));
    for (x, y, p) in left.enumerate_pixels() {
        img.put_pixel(x, y + 4, *p);
    }
    for (x, y, p) in right.enumerate_pixels() {
        img.put_pixel(396 + x, y + 4, *p);
    }
    img.save(out.join("compare.png"))
        .map_err(|e| format!("can't write compare.png: {}", e))
}

fn write_json_file(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let text =
        serde_json::to_string_pretty(value).expect("report value always serializes");
    std::fs::write(path, text + "\n")
        .map_err(|e| format!("can't write {}: {}", path.display(), e))
}

fn run_map_scheme(
    wallpaper: &Path,
    scheme_path: &Path,
    out: &Path,
    config_path: Option<&Path>,
) -> Result<(), String> {
    let config = read_json::<register::MapConfig>(config_path)?;
    let parsed = Scheme::parse_file(scheme_path)?;
    println!(
        "rehue map-scheme: reference '{}' by {}",
        parsed.meta_name(),
        parsed.meta_author()
    );

    let slot_hexes = parsed.slot_hexes()?;
    let clusters = extract::extract_hues(wallpaper, &config.extraction)?;
    match clusters.first() {
        Some(dominant) => {
            println!(
                "rehue map-scheme: {} hue cluster(s), dominant {:.1} deg",
                clusters.len(),
                dominant.hue
            );
            for cluster in &clusters {
                println!(
                    "  hue {:>6.1}  chroma {:.3}  weight {:>5.1}%",
                    cluster.hue,
                    cluster.chroma,
                    cluster.weight * 100.0
                );
            }
        }
        None => println!("rehue map-scheme: no chromatic signal - scheme hues kept"),
    }

    let mapped = register::retint(&slot_hexes, &clusters, &config)?;
    let regs = register::resolved_registers(&config)?;
    for name in register::REGISTER_NAMES {
        let settings = regs[name];
        println!(
            "  {:<8} hue-blend {:.2}  rotate {}  family-offset {}  light {:+.3}  chroma {:.2}",
            name,
            settings.hue_blend,
            settings.rotate,
            settings.family_offset,
            settings.light,
            settings.chroma
        );
    }

    std::fs::create_dir_all(out)
        .map_err(|e| format!("can't create {}: {}", out.display(), e))?;
    std::fs::write(out.join("scheme.yaml"), parsed.render(&mapped))
        .map_err(|e| format!("can't write scheme.yaml: {}", e))?;

    let original: BTreeMap<String, String> = BASE_SLOTS
        .iter()
        .copied()
        .zip(&slot_hexes)
        .map(|(slot, hex)| (slot.to_string(), hex.clone()))
        .collect();
    render_preview(&original, &mapped)?
        .save(out.join("preview.png"))
        .map_err(|e| format!("can't write preview.png: {}", e))?;

    let report = serde_json::json!({
        "params": config,
        "grayscale_wallpaper": clusters.is_empty(),
        "clusters": clusters,
        "dominant_hue": clusters.first().map(|c| c.hue),
    });
    write_json_file(&out.join("clusters.json"), &report)?;

    println!("rehue map-scheme: wrote scheme.yaml, preview.png, clusters.json");
    Ok(())
}

fn run_map_wal(
    wallpaper: &Path,
    scheme_path: &Path,
    out: &Path,
    config_path: Option<&Path>,
) -> Result<(), String> {
    let config = read_json::<map_wal::RemapConfig>(config_path)?;
    let parsed = Scheme::parse_file(scheme_path)?;
    let slot_hexes = parsed.slot_hexes()?;
    let slots: Vec<map_wal::SlotPalette> = BASE_SLOTS
        .iter()
        .copied()
        .zip(&slot_hexes)
        .map(|(slot, hex)| {
            Ok(map_wal::SlotPalette {
                slot,
                lch: hex_to_oklch(hex)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut neutral_slots: Vec<usize> =
        (0..16).filter(|i| slots[*i].lch.c < 0.08).collect();
    if neutral_slots.is_empty() {
        neutral_slots = (0..16).collect();
    }

    let source = image::ImageReader::open(wallpaper)
        .map_err(|e| format!("can't read wallpaper {}: {}", wallpaper.display(), e))?
        .decode()
        .map_err(|e| format!("can't decode wallpaper {}: {}", wallpaper.display(), e))?
        .to_rgb8();

    std::fs::create_dir_all(out)
        .map_err(|e| format!("can't create {}: {}", out.display(), e))?;

    let passive = map_wal::is_passive(&config);
    let (remapped_rgb, coverage) = if passive {
        // Contract: output is always wallpaper.png (here = input, re-encoded).
        (source.clone(), vec![0u64; 16])
    } else {
        let result = map_wal::apply(&source, &slots, &neutral_slots, &config);
        let pixels = image::RgbImage::from_raw(
            source.width(),
            source.height(),
            result.pixels,
        )
        .expect("remap preserves the pixel-buffer size");
        (pixels, result.coverage)
    };

    remapped_rgb
        .save(out.join("wallpaper.png"))
        .map_err(|e| format!("can't write wallpaper.png: {}", e))?;
    write_compare(out, &source, &remapped_rgb)?;

    let coverage_map: BTreeMap<String, u64> = BASE_SLOTS
        .iter()
        .copied()
        .zip(&coverage)
        .map(|(slot, count)| (slot.to_string(), *count))
        .collect();
    let report = serde_json::json!({
        "params": config,
        "passthrough": passive,
        "slot_coverage": coverage_map,
    });
    write_json_file(&out.join("report.json"), &report)?;

    if passive {
        println!("rehue map-wal: passthrough (no knobs active)");
    } else {
        let total: u64 = coverage.iter().sum();
        println!("rehue map-wal: remapped {} px", total);
        let mut ranked: Vec<(&str, u64)> = BASE_SLOTS
            .iter()
            .copied()
            .zip(&coverage)
            .map(|(slot, count)| (slot, *count))
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        for (slot, count) in ranked.iter().take(6) {
            println!(
                "   pixel coverage {}: {:>5.1}%",
                slot,
                100.0 * f64::from(*count as u32) / f64::from(total as u32)
            );
        }
    }
    println!("rehue map-wal: wrote wallpaper.png, compare.png, report.json");
    Ok(())
}

fn run(command: &Commands) -> Result<(), String> {
    self_test();
    match command {
        Commands::MapScheme {
            wallpaper,
            scheme,
            out,
            config,
        } => run_map_scheme(wallpaper, scheme, out, config.as_deref()),
        Commands::MapWal {
            wallpaper,
            scheme,
            out,
            config,
        } => run_map_wal(wallpaper, scheme, out, config.as_deref()),
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli.command) {
        eprintln!("rehue: {}", e);
        std::process::exit(1);
    }
}
