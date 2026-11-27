//! Builds the docs images from `examples/docs/examples.toml`.
//!
//! Rebuild the docs images from `examples/docs/examples.toml`: run every command,
//! assemble montages / screenshots, then record each image's fingerprint
//! in `examples/docs/examples.lock` (the docs test fails when an example changes
//! without a rebuild).
//!
//!   nix develop .#examples -c cargo run --release --example docs-images
//!   ... --example docs-images -- wal-territory glitter   # only these
//!
//! Needs ImageMagick (`magick`), libwebp's `img2webp` and ffmpeg for the
//! animated examples, and, for the preview screenshots, a Chromium
//! (`chromium`, `google-chrome`, or $CHROME); the examples devshell
//! provides them all, and the pinned Papirus the icon examples
//! recolor (`$REHUE_DOCS_ICONS`).  Scratch output lands in .scratch/docs-images.

#[path = "docs/manifest.rs"]
mod manifest;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use manifest::{
    Cell, Image, Kind, LOCK, Ladder, Manifest, RUNG_CONFIG, ladder_command, rung_label,
};

const BG: &str = "#232327";

/// A rung that changes the picture less than this is flagged: the
/// example would not show what the option does.  The score is the mean
/// OKLab difference x100 over the part that changed most (the top tenth
/// of an image's pixels, the top four slots of a scheme); ~2 is just
/// noticeable over a whole image, so a changed region asks for a bit more.
const MIN_CHANGE: f64 = 3.0;
/// The same for scheme ladders, whose score is already narrowed to the
/// four most-changed slots.
const MIN_SCHEME_CHANGE: f64 = 2.0;
const FG: &str = "#e8e8ef";

/// Animated examples: frames per second (an image can set its own) and
/// lossy WebP quality.  Every frame is a keyframe: with frames drawn
/// over the last, lossy WebP leaves stale blocks wherever the picture
/// changes slowly (a sky darkening), and redrawing whole frames costs
/// only about a fifth more.
const ANIMATION_FPS: u32 = 8;
const ANIMATION_QUALITY: &str = "40";

type Result<T> = std::result::Result<T, String>;

struct Builder {
    root: PathBuf,
    work: PathBuf,
    bin: PathBuf,
    font: String,
    chrome: Option<String>,
    /// `share` dir holding the icon themes icon examples use
    icons: Option<String>,
    manifest: Manifest,
    done: BTreeSet<String>,
}

fn sh(cmd: &mut Command) -> Result<String> {
    let shown = format!("{cmd:?}");
    let out = cmd.output().map_err(|e| format!("{shown}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{shown} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn found(cmd: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {cmd}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

impl Builder {
    fn run_dir(&self, id: &str) -> PathBuf {
        self.work.join("runs").join(id)
    }

    /// Run `id` (and the runs it reads) once.
    fn ensure_run(&mut self, id: &str) -> Result<PathBuf> {
        let dir = self.run_dir(id);
        if self.done.contains(id) {
            return Ok(dir);
        }
        let run = self.manifest.run(id)?.clone();
        for t in run.args.split_whitespace() {
            if let Some(i) = t.find("{out:") {
                let dep = &t[i + 5..];
                self.ensure_run(&dep[..dep.find('}').unwrap_or(dep.len())])?;
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        let args = self.expand_args(&run.args, &dir)?;
        println!("  rehue {}", run.args);
        self.rehue(&args)?;
        self.done.insert(id.to_string());
        Ok(dir)
    }

    fn expand_args(&self, args: &str, own: &Path) -> Result<Vec<String>> {
        let outs = |id: &str| -> Result<String> {
            Ok(if id.is_empty() {
                own.display().to_string()
            } else {
                self.run_dir(id).display().to_string()
            })
        };
        args.split_whitespace()
            .map(|t| self.manifest.expand(t, &outs))
            .collect()
    }

    fn rehue(&self, args: &[String]) -> Result<()> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args).current_dir(&self.root);
        if args.first().is_some_and(|a| a == "icons") {
            // the pinned themes only: nothing the machine has installed
            let share = self
                .icons
                .as_ref()
                .ok_or("missing: REHUE_DOCS_ICONS (try: nix develop .#examples)")?;
            let none = self.work.join("no-data");
            cmd.env("XDG_DATA_DIRS", share)
                .env("XDG_DATA_HOME", &none)
                .env("HOME", &none);
        }
        sh(&mut cmd).map(|_| ())
    }

    fn src(&mut self, cell: &Cell) -> Result<PathBuf> {
        if let Some(i) = cell.src.find("{out:") {
            let id = &cell.src[i + 5..];
            let id = id[..id.find('}').unwrap_or(id.len())].to_string();
            self.ensure_run(&id)?;
        }
        let none = |id: &str| -> Result<String> { Ok(self.run_dir(id).display().to_string()) };
        Ok(self.root.join(self.manifest.expand(&cell.src, &none)?))
    }

    fn montage(
        &self,
        cells: &[(String, PathBuf)],
        tile: &str,
        geometry: &str,
        out: &Path,
    ) -> Result<()> {
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut cmd = Command::new("magick");
        cmd.args([
            "montage",
            "-background",
            BG,
            "-fill",
            FG,
            "-font",
            &self.font,
        ]);
        for (label, path) in cells {
            cmd.arg("-label").arg(label).arg(path);
        }
        cmd.args(["-tile", tile, "-geometry", geometry]);
        sh(cmd.arg(out)).map(|_| ())
    }

    /// `video` as an animated WebP `width` px wide, at `fps`.
    fn animation(&self, video: &Path, width: u32, fps: u32, out: &Path) -> Result<()> {
        let stem = out.file_stem().unwrap_or_default().to_string_lossy();
        let frames = self.work.join("animation").join(&*stem);
        let _ = std::fs::remove_dir_all(&frames);
        std::fs::create_dir_all(&frames).map_err(|e| e.to_string())?;
        let ffmpeg = rehue::video::resolve_ffmpeg(None)?;
        sh(Command::new(ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(video)
            .arg("-vf")
            .arg(format!(
                "fps={fps},scale={width}:-2:flags=lanczos"
            ))
            .arg(frames.join("%04d.png")))?;
        let mut pngs: Vec<PathBuf> = std::fs::read_dir(&frames)
            .map_err(|e| e.to_string())?
            .map(|e| e.map(|e| e.path()).map_err(|e| e.to_string()))
            .collect::<Result<_>>()?;
        pngs.sort();
        sh(Command::new("img2webp")
            .args(["-loop", "0", "-kmin", "0", "-kmax", "1", "-lossy"])
            .args(["-q", ANIMATION_QUALITY, "-m", "6", "-d"])
            .arg((1000 / fps).to_string())
            .args(&pngs)
            .arg("-o")
            .arg(out))
        .map(|_| ())
    }

    fn corner(&self, png: &Path) -> Result<String> {
        sh(Command::new("magick")
            .arg(png)
            .args(["-format", "%[pixel:p{1,1}]", "info:"]))
    }

    /// One trimmed screenshot of `page` with `view` selected (the views
    /// are css-only radio tabs, so the copy just moves `checked`; it
    /// sits beside the original so relative paths still resolve).
    fn shot(&self, page: &Path, view: &str) -> Result<PathBuf> {
        let chrome = self
            .chrome
            .as_ref()
            .ok_or("no chromium found (set CHROME=...)")?;
        let html = std::fs::read_to_string(page).map_err(|e| e.to_string())?;
        let html = html
            .replace("id=\"v-palette\" checked>", "id=\"v-palette\">")
            .replace(
                &format!("id=\"v-{view}\">"),
                &format!("id=\"v-{view}\" checked>"),
            );
        let copy = page.with_file_name(format!(".shot-{view}.html"));
        std::fs::write(&copy, html).map_err(|e| e.to_string())?;
        let raw = self.work.join(format!("shot-{view}-raw.png"));
        let shot = sh(Command::new(chrome)
            .args([
                "--headless=new",
                "--disable-gpu",
                "--no-sandbox",
                "--hide-scrollbars",
                "--force-device-scale-factor=1",
                "--window-size=1280,1000",
            ])
            .arg(format!("--screenshot={}", raw.display()))
            .arg(format!("file://{}", copy.display())));
        let _ = std::fs::remove_file(&copy);
        shot?;
        let trimmed = self.work.join(format!("shot-{view}.png"));
        let border = self.corner(&raw)?;
        sh(Command::new("magick")
            .arg(&raw)
            .args([
                "-fuzz",
                "2%",
                "-trim",
                "+repage",
                "-bordercolor",
                &border,
                "-border",
                "24",
            ])
            .arg(&trimmed))?;
        Ok(trimmed)
    }

    fn image(&mut self, image: &Image) -> Result<()> {
        let out = self.root.join(&image.file);
        println!("{}", image.file);
        let mut cells = Vec::new();
        for c in &image.cells {
            cells.push((c.label.clone(), self.src(c)?));
        }
        match image.kind {
            Kind::Montage => {
                // a video in a montage shows as its first frame
                for (i, (_, path)) in cells.iter_mut().enumerate() {
                    if rehue::video::is_video(path) {
                        std::fs::create_dir_all(&self.work).map_err(|e| e.to_string())?;
                        let stem = out.file_stem().unwrap_or_default().to_string_lossy();
                        let frame = self.work.join(format!("{stem}-frame-{i}.png"));
                        sh(Command::new("ffmpeg")
                            .args(["-y", "-loglevel", "error", "-i"])
                            .arg(&*path)
                            .args(["-frames:v", "1"])
                            .arg(&frame))?;
                        *path = frame;
                    }
                }
                let tile = image.tile.clone().unwrap_or(format!("1x{}", cells.len()));
                let geometry = format!("{}x+6+10", image.width.unwrap_or(480));
                self.montage(&cells, &tile, &geometry, &out)
            }
            Kind::Copy => std::fs::copy(&cells[0].1, &out)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Kind::Scale => sh(Command::new("magick")
                .arg(&cells[0].1)
                .args(["-filter", "point", "-resize"])
                .arg(format!("{}%", image.scale.unwrap_or(200)))
                .arg(&out))
            .map(|_| ()),
            Kind::Animation => self.animation(
                &cells[0].1,
                image.width.unwrap_or(480),
                image.fps.unwrap_or(ANIMATION_FPS),
                &out,
            ),
            Kind::Shot => {
                let shots: Vec<PathBuf> = image
                    .views
                    .iter()
                    .map(|v| self.shot(&cells[0].1, v))
                    .collect::<Result<_>>()?;
                let background = self.corner(&shots[0])?;
                sh(Command::new("magick")
                    .args(&shots)
                    .args([
                        "-background",
                        &background,
                        "-gravity",
                        "north",
                        "+append",
                        "-quality",
                        "90",
                    ])
                    .arg(&out))
                .map(|_| ())
            }
        }
    }

    fn ladder(&mut self, ladder: &Ladder) -> Result<()> {
        println!("{}", ladder.file);
        let mut cells = Vec::new();
        if let Some(original) = &ladder.original {
            let label = ladder.original_label.as_deref().unwrap_or("original");
            let cell = Cell {
                label: label.to_string(),
                src: original.clone(),
            };
            cells.push((cell.label.clone(), self.src(&cell)?));
        }
        for (i, rung) in ladder.rungs.iter().enumerate() {
            let dir = self
                .work
                .join("ladders")
                .join(&ladder.id)
                .join(i.to_string());
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let line = format!("{} {} {}", ladder.base, ladder.common, rung.args);
            let mut args = self.expand_args(&line, &dir)?;
            if let Some(cfg) = &rung.config {
                let path = dir.join(RUNG_CONFIG);
                std::fs::write(&path, serde_json::to_string(cfg).unwrap())
                    .map_err(|e| e.to_string())?;
                args.push("--config".into());
                args.push(path.display().to_string());
            }
            println!("  rehue {}", args.join(" "));
            self.rehue(&args)?;
            if ladder_command(ladder) == "icons" {
                // keep the preview, not a whole recolored theme per rung
                for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
                    let path = entry.map_err(|e| e.to_string())?.path();
                    if path.is_dir() {
                        std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
                    }
                }
            }
            cells.push((rung_label(rung), dir.join(&ladder.output)));
        }
        if let Some(geometry) = &ladder.crop {
            let dir = self.work.join("ladders").join(&ladder.id);
            for (i, (_, path)) in cells.iter_mut().enumerate() {
                let cropped = dir.join(format!("crop-{i}.png"));
                sh(Command::new("magick")
                    .arg(&*path)
                    .args(["-crop", geometry, "+repage", "-filter", "point", "-resize"])
                    .arg(format!("{}x", ladder.width))
                    .arg(&cropped))?;
                *path = cropped;
            }
        }
        self.check_change(ladder, &cells)?;
        let tile = format!("1x{}", cells.len());
        let geometry = format!("{}x+3+10", ladder.width);
        self.montage(&cells, &tile, &geometry, &self.root.join(&ladder.file))
    }
}

impl Builder {
    /// Print how much each rung changes from the cell before it, and
    /// flag the ones too subtle to see.  Scheme ladders compare the
    /// 16 slots of each run's scheme.yaml; the rest compare pixels.
    fn check_change(&self, ladder: &Ladder, cells: &[(String, PathBuf)]) -> Result<()> {
        let colors = |path: &Path| -> Result<Vec<[f64; 3]>> {
            let scheme = path.with_file_name("scheme.yaml");
            let rgbs: Vec<[u8; 3]> = if ladder.output == "preview.png" && scheme.exists() {
                rehue::scheme::Scheme::parse_file(&scheme)?
                    .slot_hexes()?
                    .iter()
                    .map(|h| {
                        let v = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap();
                        [(v >> 16) as u8, (v >> 8) as u8, v as u8]
                    })
                    .collect()
            } else {
                let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
                img.resize_exact(256, 256, image::imageops::FilterType::Triangle)
                    .to_rgb8()
                    .pixels()
                    .map(|p| p.0)
                    .collect()
            };
            Ok(rgbs
                .iter()
                .map(|rgb| {
                    let c = rehue::color::rgb_to_oklch(rgb);
                    let h = c.h.to_radians();
                    [c.l, c.c * h.cos(), c.c * h.sin()]
                })
                .collect())
        };
        // Scheme ladders score the four most-changed slots: an option
        // that targets one role leaves the other slots alone by design.
        let schemes = ladder.output == "preview.png";
        let change = |a: &[[f64; 3]], b: &[[f64; 3]]| {
            let mut d: Vec<f64> = a
                .iter()
                .zip(b)
                .map(|(a, b)| {
                    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
                })
                .collect();
            // score the part that changed: the four most-changed slots of
            // a scheme, the most-changed tenth of an image's pixels (a
            // grade can rightly leave whatever already matches alone)
            d.sort_by(|x, y| y.total_cmp(x));
            let keep = if schemes { 4 } else { (d.len() / 10).max(1) };
            d.truncate(keep);
            d.iter().sum::<f64>() / d.len() as f64 * 100.0
        };
        // Each rung against the cell before it (steps) and against the
        // first rung (alternatives to a baseline); both must show.
        let skip = usize::from(ladder.original.is_some());
        let all: Vec<Vec<[f64; 3]>> = cells
            .iter()
            .map(|(_, p)| colors(p))
            .collect::<Result<_>>()?;
        for (i, (label, _)) in cells.iter().enumerate().skip(1) {
            let prev = change(&all[i - 1], &all[i]);
            let mut line = format!("    change {prev:5.1}");
            let mut least = prev;
            if i > skip + 1 {
                let first = change(&all[skip], &all[i]);
                line += &format!(" (vs first rung {first:5.1})");
                least = least.min(first);
            }
            let min = if schemes {
                MIN_SCHEME_CHANGE
            } else {
                MIN_CHANGE
            };
            let flag = if least < min {
                "  <- barely visible"
            } else {
                ""
            };
            println!("{line}  {label}{flag}");
        }
        Ok(())
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("docs-images: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = Manifest::load(&root)?;
    let only: BTreeSet<String> = std::env::args().skip(1).collect();
    let wanted = |id: &str, file: &str| {
        only.is_empty() || only.contains(id) || only.iter().any(|o| file.contains(o.as_str()))
    };
    let images: Vec<Image> = manifest
        .image
        .iter()
        .filter(|i| {
            let stem = Path::new(&i.file)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            wanted(&stem, &i.file)
        })
        .cloned()
        .collect();
    let ladders: Vec<Ladder> = manifest
        .ladder
        .iter()
        .filter(|l| wanted(&l.id, &l.file))
        .cloned()
        .collect();
    if images.is_empty() && ladders.is_empty() {
        return Err(format!("nothing matches {only:?}"));
    }

    if !found("magick") {
        return Err("missing: magick (try: nix develop .#examples)".into());
    }
    let chrome = std::env::var("CHROME").ok().or_else(|| {
        [
            "chromium",
            "chromium-browser",
            "google-chrome-stable",
            "google-chrome",
        ]
        .into_iter()
        .find(|c| found(c))
        .map(String::from)
    });
    if images.iter().any(|i| i.kind == Kind::Animation) && !found("img2webp") {
        return Err("missing: img2webp (try: nix develop .#examples)".into());
    }
    if chrome.is_none() && images.iter().any(|i| i.kind == Kind::Shot) {
        return Err("missing: chromium (or set CHROME=...)".into());
    }

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    sh(Command::new(cargo)
        .args(["build", "--release", "--quiet", "--bin", "rehue"])
        .current_dir(&root))?;

    let work = root.join(".scratch/docs-images");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let mut b = Builder {
        bin: root.join("target/release/rehue"),
        font: std::env::var("FONT").unwrap_or_else(|_| "DejaVu-Sans-Mono".into()),
        root: root.clone(),
        work,
        chrome,
        icons: std::env::var("REHUE_DOCS_ICONS").ok(),
        manifest: manifest.clone(),
        done: BTreeSet::new(),
    };
    for i in &images {
        b.image(i)?;
    }
    for l in &ladders {
        b.ladder(l)?;
    }

    // record what was built; entries for images not rebuilt keep their
    // previous fingerprint
    let built: BTreeSet<&str> = images
        .iter()
        .map(|i| i.file.as_str())
        .chain(ladders.iter().map(|l| l.file.as_str()))
        .collect();
    let old = std::fs::read_to_string(root.join(LOCK)).unwrap_or_default();
    let mut lines: BTreeMap<String, String> = old
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(f, h)| (f.to_string(), h.to_string()))
        .collect();
    for line in manifest.lock()?.lines() {
        let (file, hash) = line.split_once(' ').unwrap();
        if built.contains(file) {
            lines.insert(file.to_string(), hash.to_string());
        }
    }
    let files: BTreeSet<String> = manifest
        .lock()?
        .lines()
        .map(|l| l.split(' ').next().unwrap().to_string())
        .collect();
    lines.retain(|f, _| files.contains(f));
    let text: String = lines.iter().map(|(f, h)| format!("{f} {h}\n")).collect();
    std::fs::write(root.join(LOCK), text).map_err(|e| e.to_string())?;
    println!("done: {} image(s); {LOCK} updated", built.len());
    Ok(())
}
