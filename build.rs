//! Embeds the vendored tinted-theming scheme snapshot
//! (`vendor/tinted-schemes`, refreshed by `scripts/update-schemes.sh`):
//! a sorted `(spec, name, yaml text)` table of `include_str!` entries.

use std::{env, fs, path::PathBuf};

const SPECS: [&str; 2] = ["base16", "base24"];

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("vendor/tinted-schemes");
    println!("cargo:rerun-if-changed=vendor/tinted-schemes");

    let mut entries = Vec::new();
    for spec in SPECS {
        let dir = root.join(spec);
        println!("cargo:rerun-if-changed={}", dir.display());
        let read = fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("can't read {} ({e}); run scripts/update-schemes.sh", dir.display()));
        for entry in read.flatten() {
            let path = entry.path();
            if !matches!(path.extension().and_then(|e| e.to_str()), Some("yaml" | "yml")) {
                continue;
            }
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            entries.push((spec, name, path));
        }
    }
    entries.sort();

    let rows: Vec<String> = entries
        .iter()
        .map(|(spec, name, path)| format!("    ({spec:?}, {name:?}, include_str!({path:?})),"))
        .collect();
    let text = format!(
        "/// (spec, name, yaml text), sorted by spec then name.\n\
         pub const SCHEMES: &[(&str, &str, &str)] = &[\n{}\n];\n",
        rows.join("\n")
    );
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("schemes_table.rs");
    fs::write(out, text).unwrap();
}
