//! Rebuild the docs images from `docs/examples.toml`: run every command,
//! assemble montages / screenshots, then record each image's fingerprint
//! in `docs/examples.lock` (the docs test fails when an example changes
//! without a rebuild).
//!
//!   nix develop .#examples -c cargo run --release --example docs-images
//!   ... --example docs-images -- wal-territory glitter   # only these
//!
//! Needs ImageMagick (`magick`) and, for the preview screenshots, a
//! Chromium (`chromium`, `google-chrome`, or $CHROME); the examples
//! devshell provides both.  Scratch output lands in .scratch/docs-images.

#[path = "../docs/manifest.rs"]
mod manifest;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use manifest::{Cell, Image, Kind, LOCK, Ladder, Manifest, RUNG_CONFIG, rung_label};

const BG: &str = "#232327";
const FG: &str = "#e8e8ef";

type Result<T> = std::result::Result<T, String>;

struct Builder {
    root: PathBuf,
    work: PathBuf,
    bin: PathBuf,
    font: String,
    chrome: Option<String>,
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
        sh(Command::new(&self.bin).args(args).current_dir(&self.root)).map(|_| ())
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
            let none = |_: &str| -> Result<String> { Err("no {out} in original".into()) };
            cells.push((
                "original".to_string(),
                self.root.join(self.manifest.expand(original, &none)?),
            ));
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
            cells.push((rung_label(rung), dir.join(&ladder.output)));
        }
        let tile = format!("1x{}", cells.len());
        let geometry = format!("{}x+3+10", ladder.width);
        self.montage(&cells, &tile, &geometry, &self.root.join(&ladder.file))
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
