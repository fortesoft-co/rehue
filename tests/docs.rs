//! The docs: generated regions, the drift check, links, example images.
//!
//! The docs are templates of themselves: README.md and docs/*.md carry
//! marked regions (`<!-- gen:NAME -->` ... `<!-- /gen:NAME -->`) whose
//! contents render from the code, so reference tables can't drift from
//! the CLI and config they describe.  Everything outside a region is
//! hand-written and never touched.
//!
//!   cargo test --test docs                    # fails if a region is stale
//!   REHUE_BLESS=1 cargo test --test docs      # rewrites stale regions
//!
//! A region whose open marker sits alone on its line is a block (its
//! content is whole lines); otherwise it is inline (content between the
//! markers on one line).  Markers inside fenced code blocks are ignored,
//! so docs can show the syntax.  A drift check then scans the
//! hand-written text for flags and config keys that don't exist.

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::CommandFactory;
use rehue::cli::Cli;

#[path = "../examples/docs/manifest.rs"]
mod manifest;

fn examples() -> manifest::Manifest {
    manifest::Manifest::load(env!("CARGO_MANIFEST_DIR").as_ref()).unwrap()
}

/// Every page checked: the README and each page under docs/.
fn docs() -> Vec<String> {
    let mut pages: Vec<String> = std::fs::read_dir(path("docs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .map(|p| format!("docs/{}", p.file_name().unwrap().to_string_lossy()))
        .collect();
    pages.sort();
    pages.insert(0, "README.md".into());
    pages
}

// ---------------------------------------------------------------- parser

#[derive(Debug, Clone, PartialEq)]
struct Region {
    name: String,
    inline: bool,
    /// byte range of the content between the markers
    start: usize,
    end: usize,
    line: usize,
}

const OPEN: &str = "<!-- gen:";
const CLOSE: &str = "<!-- /gen:";
const TAIL: &str = " -->";

fn line_of(text: &str, pos: usize) -> usize {
    text[..pos].matches('\n').count() + 1
}

/// Byte ranges of fenced code blocks (``` or ~~~, up to three spaces of
/// indent), fence lines included; an unclosed fence runs to the end.
fn fences(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut open: Option<(usize, char, usize)> = None;
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let body = line.trim_start_matches(' ');
        let fence = |c: char| body.chars().take_while(|&x| x == c).count();
        match open {
            None if indent <= 3 => {
                for c in ['`', '~'] {
                    if fence(c) >= 3 {
                        open = Some((pos, c, fence(c)));
                        break;
                    }
                }
            }
            Some((start, c, n))
                if indent <= 3 && fence(c) >= n && body.trim_start_matches(c).trim().is_empty() =>
            {
                out.push((start, pos + line.len()));
                open = None;
            }
            _ => {}
        }
        pos += line.len();
    }
    if let Some((start, _, _)) = open {
        out.push((start, text.len()));
    }
    out
}

/// One marker: (position, is_close, name, end of marker).
fn markers(text: &str) -> Result<Vec<(usize, bool, String, usize)>, String> {
    let fenced = fences(text);
    let in_fence = |p: usize| fenced.iter().any(|&(a, b)| p >= a && p < b);
    let mut out = Vec::new();
    for (prefix, is_close) in [(OPEN, false), (CLOSE, true)] {
        let mut from = 0;
        while let Some(i) = text[from..].find(prefix) {
            let at = from + i;
            from = at + prefix.len();
            // inside a fenced block, or an inline `code span` (an odd
            // number of backticks earlier on the line)
            let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
            if in_fence(at) || text[line_start..at].matches('`').count() % 2 == 1 {
                continue;
            }
            let rest = &text[at + prefix.len()..];
            let Some(close) = rest.find(TAIL) else {
                return Err(format!("line {}: unterminated marker", line_of(text, at)));
            };
            let name = &rest[..close];
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-:_".contains(c))
            {
                return Err(format!(
                    "line {}: bad region name {name:?}",
                    line_of(text, at)
                ));
            }
            out.push((
                at,
                is_close,
                name.to_string(),
                at + prefix.len() + close + TAIL.len(),
            ));
        }
    }
    out.sort_by_key(|m| m.0);
    Ok(out)
}

/// Is `[a, b)` the whole of its line (ignoring surrounding spaces)?
fn alone_on_line(text: &str, a: usize, b: usize) -> bool {
    let line_start = text[..a].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[b..].find('\n').map_or(text.len(), |i| b + i);
    text[line_start..a].trim().is_empty() && text[b..line_end].trim().is_empty()
}

fn parse(text: &str) -> Result<Vec<Region>, String> {
    let mut regions = Vec::new();
    let mut open: Option<(usize, String, usize)> = None;
    for (at, is_close, name, end) in markers(text)? {
        let line = line_of(text, at);
        match (&open, is_close) {
            (Some((_, outer, oline)), false) => {
                return Err(format!(
                    "line {line}: region '{name}' opens inside '{outer}' (line {oline}); regions don't nest"
                ));
            }
            (None, false) => open = Some((at, name, end)),
            (None, true) => return Err(format!("line {line}: '/gen:{name}' closes nothing")),
            (Some((oat, oname, oend)), true) => {
                if *oname != name {
                    return Err(format!(
                        "line {line}: '/gen:{name}' closes '{oname}' (line {})",
                        line_of(text, *oat)
                    ));
                }
                let block = alone_on_line(text, *oat, *oend);
                if block != alone_on_line(text, at, end) {
                    return Err(format!(
                        "line {line}: region '{name}' mixes block and inline markers"
                    ));
                }
                let region = if block {
                    let start = text[*oend..]
                        .find('\n')
                        .map_or(text.len(), |i| oend + i + 1);
                    let end = text[..at].rfind('\n').map_or(0, |i| i + 1);
                    Region {
                        name,
                        inline: false,
                        start,
                        end: end.max(start),
                        line: line_of(text, *oat),
                    }
                } else {
                    if text[*oend..at].contains('\n') {
                        return Err(format!("line {line}: inline region '{name}' spans lines"));
                    }
                    Region {
                        name,
                        inline: true,
                        start: *oend,
                        end: at,
                        line: line_of(text, *oat),
                    }
                };
                regions.push(region);
                open = None;
            }
        }
    }
    if let Some((at, name, _)) = open {
        return Err(format!(
            "line {}: region '{name}' is never closed",
            line_of(text, at)
        ));
    }
    let mut seen = BTreeSet::new();
    for r in regions.iter().filter(|r| !r.inline) {
        if !seen.insert(&r.name) {
            return Err(format!(
                "line {}: block region '{}' appears twice",
                r.line, r.name
            ));
        }
    }
    Ok(regions)
}

/// Every region re-rendered by `render`; text outside regions is kept
/// byte for byte.
fn render(text: &str, render: &dyn Fn(&str) -> Result<String, String>) -> Result<String, String> {
    let regions = parse(text)?;
    let mut out = text.to_string();
    for r in regions.iter().rev() {
        let body = render(&r.name).map_err(|e| format!("line {}: {e}", r.line))?;
        let body = if r.inline {
            if body.contains('\n') {
                return Err(format!(
                    "line {}: inline region '{}' rendered several lines",
                    r.line, r.name
                ));
            }
            body
        } else {
            format!("{}\n", body.trim_end())
        };
        out.replace_range(r.start..r.end, &body);
    }
    Ok(out)
}

// --------------------------------------------------------------- regions

fn escape(cell: &str) -> String {
    let cell = cell.replace('|', "\\|").replace('\n', " ");
    cell.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sentence(s: &str) -> String {
    s.trim().trim_end_matches('.').to_string()
}

fn subcommand(name: &str) -> Result<clap::Command, String> {
    Cli::command()
        .find_subcommand(name)
        .cloned()
        .ok_or_else(|| format!("no command '{name}'"))
}

/// `| command | what it does |` for every visible command.
fn commands_table() -> String {
    let mut t = String::from("| command | what it does |\n| --- | --- |\n");
    for c in Cli::command()
        .get_subcommands()
        .filter(|c| !c.is_hide_set())
    {
        let about = c.get_about().map(|a| a.to_string()).unwrap_or_default();
        t.push_str(&format!(
            "| `rehue {}` | {} |\n",
            c.get_name(),
            escape(&sentence(&about))
        ));
    }
    t
}

/// The value a flag takes, as shown in a table.
fn flag_value(arg: &clap::Arg) -> String {
    // a switch (--sr, --dry-run) takes no value
    if !arg.get_action().takes_values() {
        return String::new();
    }
    let possible: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|v| !v.is_hide_set())
        .map(|v| v.get_name().to_string())
        .collect();
    if !possible.is_empty() {
        return format!("`{}`", possible.join(" \\| "));
    }
    match arg.get_value_names() {
        Some(names) if arg.get_action().takes_values() => {
            let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
            format!("`{}`", names.join(" "))
        }
        _ => String::new(),
    }
}

/// `| flag | value | what it does |` for one command's flags under a
/// `--help` heading.
fn flags_table(command: &str, heading: &str) -> Result<String, String> {
    let c = subcommand(command)?;
    let mut t = String::from("| flag | value | what it does |\n| --- | --- | --- |\n");
    let mut any = false;
    for arg in c
        .get_arguments()
        .filter(|a| a.get_help_heading() == Some(heading))
    {
        let Some(long) = arg.get_long() else { continue };
        any = true;
        let help = arg.get_help().map(|h| h.to_string()).unwrap_or_default();
        let mut help = sentence(&help);
        let defaults: Vec<String> = arg
            .get_default_values()
            .iter()
            .map(|d| d.to_string_lossy().into_owned())
            .collect();
        if !defaults.is_empty() && !help.contains("default") {
            help.push_str(&format!(" (default {})", defaults.join(" ")));
        }
        t.push_str(&format!(
            "| `--{long}` | {} | {} |\n",
            flag_value(arg),
            escape(&help)
        ));
    }
    if !any {
        return Err(format!(
            "'{command}' has no flags under heading '{heading}'"
        ));
    }
    Ok(t)
}

/// `| role | short | slots | what it is |`.
fn roles_table() -> String {
    use rehue::role::{ROLE_DOCS, ROLE_NAMES, ROLE_SHORT, role_slots};
    let mut t = String::from("| role | short | slots | what it is |\n| --- | --- | --- | --- |\n");
    for ((name, short), role) in ROLE_NAMES.iter().zip(ROLE_SHORT).zip(ROLE_DOCS) {
        let slots = role_slots(name);
        let first = format!("base{}", slots[0][4..].to_uppercase());
        let span = if slots.len() == 1 {
            first
        } else {
            format!("{first}-{}", slots[slots.len() - 1][4..].to_uppercase())
        };
        t.push_str(&format!("| `{name}` | `{short}` | {span} | {role} |\n"));
    }
    t
}

fn looks_table() -> String {
    let mut t = String::from("| look | grades | what it does |\n| --- | --- | --- |\n");
    for (name, text) in rehue::look::BUILTIN {
        let l = rehue::look::Look::parse(name, text, "toml", rehue::look::Source::BuiltIn)
            .expect("built-in looks parse");
        t.push_str(&format!(
            "| `{name}` | {} | {} |\n",
            l.targets(),
            escape(&l.description)
        ));
    }
    t
}

/// A config value as written in the docs (`0.04`, `[0.1, 0.92]`).
fn json_cell(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Array(xs) => format!(
            "[{}]",
            xs.iter().map(json_cell).collect::<Vec<_>>().join(", ")
        ),
        serde_json::Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

fn extraction_defaults() -> serde_json::Map<String, serde_json::Value> {
    match serde_json::to_value(rehue::extract::ExtractionParams::default()).unwrap() {
        serde_json::Value::Object(m) => m,
        _ => unreachable!("a struct serializes to an object"),
    }
}

/// `| option | default | semantics |` for the extraction keys.
fn extraction_table() -> String {
    let defaults = extraction_defaults();
    let mut t = String::from("| option | default | semantics |\n| --- | --- | --- |\n");
    for (key, doc) in rehue::extract::ExtractionParams::KEYS {
        t.push_str(&format!(
            "| `{key}` | {} | {} |\n",
            json_cell(&defaults[key]),
            escape(doc)
        ));
    }
    t
}

fn render_region(name: &str) -> Result<String, String> {
    if let Some(rest) = name.strip_prefix("flags:") {
        let (command, heading) = rest
            .split_once(':')
            .ok_or_else(|| format!("'{name}': want flags:COMMAND:HEADING"))?;
        // headings are written kebab-case in the marker (`lut-export`)
        let c = subcommand(command)?;
        let heading = c
            .get_arguments()
            .filter_map(|a| a.get_help_heading())
            .find(|h| h.to_lowercase().replace(' ', "-") == heading)
            .ok_or_else(|| format!("'{command}' has no heading '{heading}'"))?
            .to_string();
        return flags_table(command, &heading);
    }
    if let Some(id) = name.strip_prefix("run:") {
        let m = examples();
        return manifest::display_run(&m, m.run(id)?);
    }
    if let Some(id) = name.strip_prefix("ladder:") {
        let m = examples();
        let ladder = m
            .ladder
            .iter()
            .find(|l| l.id == id)
            .ok_or_else(|| format!("no ladder '{id}' in {}", manifest::MANIFEST))?;
        return manifest::display_ladder(&m, ladder);
    }
    match name {
        "commands" => Ok(commands_table()),
        "roles" => Ok(roles_table()),
        "looks" => Ok(looks_table()),
        "source-files" => source_files(),
        "look-names" => {
            let names: Vec<String> = rehue::look::BUILTIN
                .iter()
                .map(|(n, _)| format!("`{n}`"))
                .collect();
            let (last, rest) = names.split_last().unwrap();
            Ok(format!("{} and {last}", rest.join(", ")))
        }
        "extraction-table" => Ok(extraction_table()),
        "extraction-keys" => Ok(rehue::extract::ExtractionParams::KEYS
            .iter()
            .map(|(k, _)| format!("`{k}`"))
            .collect::<Vec<_>>()
            .join(", ")),
        "scheme-count" => Ok(rehue::scheme::collection_names().count().to_string()),
        _ => Err(format!("unknown region '{name}'")),
    }
}

/// The source files and what each is: the first line of its `//!`
/// module doc, which must be a paragraph of its own (rustdoc's summary).
fn source_files() -> Result<String, String> {
    let rs = |dir: &str| -> Vec<String> {
        let mut files: Vec<String> = std::fs::read_dir(path(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "rs"))
            .map(|p| format!("{dir}/{}", p.file_name().unwrap().to_string_lossy()))
            .collect();
        files.sort();
        files
    };
    let files: Vec<String> = [
        rs("src"),
        rs("src/bin"),
        vec!["build.rs".into()],
        rs("examples"),
    ]
    .into_iter()
    .chain([rs("tests"), vec!["tests/common/mod.rs".into()]])
    .flatten()
    .collect();
    let mut rows = vec![
        "| file | what it is |".to_string(),
        "| --- | --- |".to_string(),
    ];
    for file in files {
        let text = std::fs::read_to_string(path(&file)).map_err(|e| format!("{file}: {e}"))?;
        let mut doc = text
            .lines()
            .skip_while(|l| !l.starts_with("//!"))
            .take_while(|l| l.starts_with("//!"))
            .map(|l| l.trim_start_matches("//!").trim());
        let summary = doc
            .next()
            .filter(|l| !l.is_empty())
            .ok_or_else(|| format!("{file}: no //! summary line"))?;
        if doc.next().is_some_and(|l| !l.is_empty()) {
            return Err(format!(
                "{file}: the first //! line is the summary: give it a paragraph of its own"
            ));
        }
        rows.push(format!("| `{file}` | {} |", escape(summary)));
    }
    Ok(rows.join("\n"))
}

fn path(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(file)
}

#[test]
fn docs_are_current() {
    let bless = std::env::var_os("REHUE_BLESS").is_some();
    let mut stale = Vec::new();
    let pages = docs();
    for file in &pages {
        let text = std::fs::read_to_string(path(file)).unwrap();
        let rendered = render(&text, &render_region).unwrap_or_else(|e| panic!("{file}: {e}"));
        if rendered != text {
            if bless {
                std::fs::write(path(file), &rendered).unwrap();
            } else {
                stale.push(file);
            }
        }
    }
    assert!(
        stale.is_empty(),
        "stale generated regions in {stale:?}: run REHUE_BLESS=1 cargo test --test docs"
    );
}

#[test]
fn extraction_keys_are_documented_exactly() {
    let documented: Vec<&str> = rehue::extract::ExtractionParams::KEYS
        .iter()
        .map(|(k, _)| *k)
        .collect();
    let mut serialized: Vec<String> = extraction_defaults().keys().cloned().collect();
    let mut sorted = documented.clone();
    sorted.sort();
    serialized.sort();
    assert_eq!(sorted, serialized);
}

// -------------------------------------------------------------- examples

#[test]
fn example_images_are_current() {
    let m = examples();
    let want = m.lock().unwrap();
    let have = std::fs::read_to_string(path(manifest::LOCK)).unwrap_or_default();
    let have: BTreeSet<&str> = have.lines().collect();
    let stale: Vec<&str> = want
        .lines()
        .filter(|l| !have.contains(l))
        .map(|l| l.split(' ').next().unwrap())
        .collect();
    assert!(
        stale.is_empty(),
        "example images out of date with {}:\n  {}\nrebuild: nix develop .#examples -c cargo run --release --example docs-images",
        manifest::MANIFEST,
        stale.join("\n  ")
    );
}

#[test]
fn every_docs_image_is_built_from_the_manifest() {
    let m = examples();
    let built: BTreeSet<&str> = m
        .image
        .iter()
        .map(|i| i.file.as_str())
        .chain(m.ladder.iter().map(|l| l.file.as_str()))
        .collect();
    for file in &built {
        assert!(
            path(file).is_file(),
            "{file} is in the manifest but missing"
        );
    }
    let mut orphans = Vec::new();
    for doc in &docs() {
        let text = std::fs::read_to_string(path(doc)).unwrap();
        let mut rest = text.as_str();
        while let Some(i) = rest.find("assets/examples/") {
            let tail = &rest[i..];
            let end = tail
                .find(|c: char| c == ')' || c == '"' || c.is_whitespace())
                .unwrap_or(tail.len());
            let file = &tail[..end];
            if !built.contains(file) {
                orphans.push(format!("{doc}: {file}"));
            }
            rest = &tail[end..];
        }
    }
    assert!(
        orphans.is_empty(),
        "docs images with no recipe in {}:\n  {}",
        manifest::MANIFEST,
        orphans.join("\n  ")
    );
}

/// GitHub's anchor for a heading: lowercase, spaces to dashes, every
/// other character but letters, digits, `-` and `_` dropped; repeats get
/// `-1`, `-2`, ...
fn anchors(text: &str) -> BTreeSet<String> {
    let fenced = fences(text);
    let mut seen = BTreeSet::new();
    let mut pos = 0;
    for line in text.lines() {
        let at = pos;
        pos += line.len() + 1;
        if fenced.iter().any(|&(a, b)| at >= a && at < b) {
            continue;
        }
        let Some(heading) = line.strip_prefix('#') else {
            continue;
        };
        let heading = heading.trim_start_matches('#');
        if !heading.starts_with(' ') {
            continue;
        }
        let slug: String = heading
            .trim()
            .to_lowercase()
            .chars()
            .filter_map(|c| match c {
                ' ' => Some('-'),
                c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
                _ => None,
            })
            .collect();
        let mut unique = slug.clone();
        let mut n = 0;
        while seen.contains(&unique) {
            n += 1;
            unique = format!("{slug}-{n}");
        }
        seen.insert(unique);
    }
    seen
}

/// Every link target in a page's prose: `](target)` and `src="target"`.
fn links(text: &str) -> Vec<(usize, String)> {
    let fenced = fences(text);
    let mut out = Vec::new();
    for (open, close) in [("](", ")"), ("src=\"", "\"")] {
        let mut from = 0;
        while let Some(i) = text[from..].find(open) {
            let start = from + i + open.len();
            let Some(len) = text[start..].find(close) else {
                break;
            };
            from = start + len;
            if fenced.iter().any(|&(a, b)| start >= a && start < b) {
                continue;
            }
            out.push((line_of(text, start), text[start..start + len].to_string()));
        }
    }
    out
}

#[test]
fn links_between_pages_resolve() {
    let mut broken = Vec::new();
    let pages: Vec<String> = docs()
        .into_iter()
        .chain(["CHANGELOG.md".into(), "ROADMAP.md".into()])
        .collect();
    for page in &pages {
        let text = std::fs::read_to_string(path(page)).unwrap();
        let dir = std::path::Path::new(page).parent().unwrap();
        for (line, target) in links(&text) {
            if target.contains("://") || target.starts_with("mailto:") {
                continue;
            }
            let (file, anchor) = target.split_once('#').unwrap_or((&target, ""));
            let file = if file.is_empty() {
                path(page)
            } else {
                path(&dir.join(file).to_string_lossy())
            };
            let shown = format!("{page}:{line}: {target}");
            if !file.exists() {
                broken.push(format!("{shown} (no such file)"));
            } else if !anchor.is_empty() {
                let body = std::fs::read_to_string(&file).unwrap_or_default();
                if !anchors(&body).contains(anchor) {
                    broken.push(format!("{shown} (no such heading)"));
                }
            }
        }
    }
    assert!(
        broken.is_empty(),
        "broken links:\n  {}",
        broken.join("\n  ")
    );
}

#[test]
fn manifest_commands_use_real_flags() {
    let m = examples();
    let cli = Cli::command();
    let mut bad = Vec::new();
    let mut check = |what: &str, line: &str| {
        let mut words = line.split_whitespace();
        let Some(command) = words.next() else { return };
        let Some(sub) = cli.find_subcommand(command) else {
            bad.push(format!("{what}: no command '{command}'"));
            return;
        };
        let longs: BTreeSet<&str> = sub.get_arguments().filter_map(|a| a.get_long()).collect();
        let shorts: BTreeSet<char> = sub.get_arguments().filter_map(|a| a.get_short()).collect();
        for w in words.filter(|w| manifest::is_flag(w)) {
            let known = match w.strip_prefix("--") {
                Some(long) => longs.contains(long),
                None => w.len() == 2 && shorts.contains(&w.chars().nth(1).unwrap()),
            };
            if !known {
                bad.push(format!("{what}: {command} has no {w}"));
            }
        }
    };
    for r in &m.run {
        check(&format!("run {}", r.id), &r.args);
    }
    for l in &m.ladder {
        for rung in &l.rungs {
            check(
                &format!("ladder {}", l.id),
                &format!("{} {} {}", l.base, l.common, rung.args),
            );
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

// ----------------------------------------------------------- drift check

/// Every long flag of every command, without the dashes.
fn known_flags() -> BTreeSet<String> {
    fn walk(c: &clap::Command, out: &mut BTreeSet<String>) {
        for a in c.get_arguments() {
            if let Some(l) = a.get_long() {
                out.insert(l.to_string());
            }
        }
        for s in c.get_subcommands() {
            walk(s, out);
        }
    }
    let mut out = BTreeSet::from(["help".to_string(), "version".to_string()]);
    walk(&Cli::command(), &mut out);
    out
}

/// Words a backticked kebab-case name may be: flags, commands, enum
/// values, config keys, plus the non-rehue names the docs mention.
fn known_names() -> BTreeSet<String> {
    let mut out = known_flags();
    let cli = Cli::command();
    for c in cli.get_subcommands() {
        out.insert(c.get_name().to_string());
        for a in c.get_arguments() {
            for v in a.get_possible_values() {
                out.insert(v.get_name().to_string());
            }
        }
    }
    let mut keys = |v: serde_json::Value| {
        if let serde_json::Value::Object(m) = v {
            out.extend(m.keys().cloned());
        }
    };
    keys(serde_json::to_value(rehue::grade_wal::WalConfig::default()).unwrap());
    keys(serde_json::to_value(rehue::grade_scheme::SchemeConfig::default()).unwrap());
    keys(serde_json::to_value(rehue::grade_wal::WalRoleConfig::default()).unwrap());
    keys(serde_json::to_value(rehue::grade_scheme::SchemeRoleConfig::default()).unwrap());
    for (k, _) in rehue::extract::ExtractionParams::KEYS {
        out.insert(k.to_string());
    }
    // dither styles, their settings and values, from the code
    {
        use rehue::dither::{Dither, Kernel, Sample, Style, Tiles};
        let mut names = |v: serde_json::Value| match v {
            serde_json::Value::String(s) => {
                out.insert(s);
            }
            serde_json::Value::Object(m) => {
                for (k, v) in m {
                    out.insert(k);
                    if let serde_json::Value::String(s) = v {
                        out.insert(s);
                    }
                }
            }
            _ => {}
        };
        for style in [
            Style::Checker,
            Style::Halftone,
            Style::Pattern,
            Style::Diffuse,
        ] {
            names(serde_json::to_value(Dither::for_style(style)).unwrap());
        }
        for v in [
            serde_json::to_value(Kernel::FloydSteinberg).unwrap(),
            serde_json::to_value(Kernel::Atkinson).unwrap(),
            serde_json::to_value(Tiles::Hatch).unwrap(),
            serde_json::to_value(Tiles::Lines).unwrap(),
            serde_json::to_value(Sample::Block).unwrap(),
            serde_json::to_value(Sample::Pixel).unwrap(),
        ] {
            names(v);
        }
    }
    // pipeline keys (src/apply.rs; deserialize-only structs)
    for k in [
        "scheme-dir",
        "tinty-name",
        "tinty-config",
        "tinty-data-dir",
        "wallpaper-command",
    ] {
        out.insert(k.to_string());
    }
    // names outside rehue's own surface
    for k in [
        "rehue-out",
        "base16-schemes",
        "tinted-builder",
        "custom-schemes",
        "hald-clut",
    ] {
        out.insert(k.to_string());
    }
    out.extend(rehue::scheme::collection_names().map(String::from));
    out
}

/// Hand-written text only: generated regions (and their markers) blanked.
fn handwritten(text: &str) -> String {
    let mut out = text.to_string();
    for r in parse(text).unwrap().iter().rev() {
        let blank: String = text[r.start..r.end]
            .chars()
            .map(|c| if c == '\n' { '\n' } else { ' ' })
            .collect();
        out.replace_range(r.start..r.end, &blank);
    }
    out
}

#[test]
fn handwritten_docs_name_only_real_flags_and_keys() {
    let flags = known_flags();
    let names = known_names();
    let mut unknown = Vec::new();
    for file in &docs() {
        let text = handwritten(&std::fs::read_to_string(path(file)).unwrap());
        let fenced = fences(&text);
        let mut section = "";
        let mut in_rehue_command = false;
        let mut pos = 0;
        for (n, line) in text.lines().enumerate() {
            let in_fence = fenced.iter().any(|&(a, b)| pos >= a && pos < b);
            pos += line.len() + 1;
            if let Some(h) = line.strip_prefix("## ") {
                section = h.trim();
            }
            // the roadmap names things that don't exist yet, on purpose
            if section == "Roadmap" {
                continue;
            }
            // in code, only rehue's own command lines (and their `\`
            // continuations) are checked; cargo/nix/magick flags are not ours
            let check_flags = if in_fence {
                let starts = line.trim_start().starts_with("rehue ");
                let checked = starts || in_rehue_command;
                in_rehue_command = checked && line.trim_end().ends_with('\\');
                checked
            } else {
                true
            };
            let mut rest = if check_flags { line } else { "" };
            while let Some(i) = rest.find("--") {
                let tail = &rest[i + 2..];
                let word: String = tail
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
                    .collect();
                let before = rest[..i].chars().last();
                let starts_word = before.is_none_or(|c| c.is_whitespace() || "`([\"'".contains(c));
                if starts_word && word.starts_with(|c: char| c.is_ascii_lowercase()) {
                    let word = word.trim_end_matches('-');
                    if !flags.contains(word) {
                        unknown.push(format!("{file}:{}: --{word}", n + 1));
                    }
                }
                rest = &tail[word.len()..];
            }
            // `kebab-case` names in inline code
            for (k, span) in line.split('`').enumerate() {
                let is_name = k % 2 == 1
                    && span.contains('-')
                    && span.split('-').all(|p| {
                        !p.is_empty()
                            && p.chars()
                                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                    });
                if is_name && !names.contains(span) {
                    unknown.push(format!("{file}:{}: `{span}`", n + 1));
                }
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "docs mention names rehue doesn't have:\n  {}",
        unknown.join("\n  ")
    );
}

// --------------------------------------------------------- parser tests

fn fake(name: &str) -> Result<String, String> {
    match name {
        "n" => Ok("42".into()),
        "table" => Ok("| a |\n| - |\n".into()),
        other => Err(format!("unknown region '{other}'")),
    }
}

#[test]
fn renders_block_and_inline_regions_and_keeps_the_rest() {
    let doc = "Intro <!-- gen:n -->7<!-- /gen:n --> things.\n\n<!-- gen:table -->\nold\nrows\n<!-- /gen:table -->\nOutro\n";
    let out = render(doc, &fake).unwrap();
    assert_eq!(
        out,
        "Intro <!-- gen:n -->42<!-- /gen:n --> things.\n\n<!-- gen:table -->\n| a |\n| - |\n<!-- /gen:table -->\nOutro\n"
    );
    assert_eq!(render(&out, &fake).unwrap(), out, "rendering is idempotent");
    // an empty block fills in
    let empty = "<!-- gen:table -->\n<!-- /gen:table -->\n";
    assert_eq!(
        render(empty, &fake).unwrap(),
        "<!-- gen:table -->\n| a |\n| - |\n<!-- /gen:table -->\n"
    );
}

#[test]
fn anchors_follow_github() {
    let a = anchors(
        "# Top\n## match-chroma — `0..1`, default `0` (set by harmonize)\n### cycle\n### cycle\n```\n# not a heading\n```\n",
    );
    for want in [
        "top",
        "match-chroma--01-default-0-set-by-harmonize",
        "cycle",
        "cycle-1",
    ] {
        assert!(a.contains(want), "{want}: {a:?}");
    }
    assert!(!a.contains("not-a-heading"));
}

#[test]
fn markers_in_code_are_ignored() {
    let doc = "```markdown\n<!-- gen:table -->\nexample\n<!-- /gen:table -->\n```\n~~~\n<!-- gen:nope -->\n~~~\nprose `<!-- gen:NAME -->` too\n";
    assert_eq!(render(doc, &fake).unwrap(), doc);
}

#[test]
fn malformed_regions_are_errors_with_lines() {
    let err = |doc: &str| render(doc, &fake).unwrap_err();
    assert!(err("a\n<!-- gen:table -->\nx\n").contains("line 2: region 'table' is never closed"));
    assert!(err("<!-- /gen:n -->").contains("closes nothing"));
    assert!(err("<!-- gen:n -->1<!-- /gen:table -->").contains("closes 'n'"));
    assert!(err("<!-- gen:table -->\n<!-- gen:n -->\n").contains("don't nest"));
    assert!(err("<!-- gen:zzz -->x<!-- /gen:zzz -->").contains("unknown region 'zzz'"));
    assert!(err("<!-- gen:table -->\nx <!-- /gen:table -->").contains("mixes block and inline"));
    assert!(
        err("<!-- gen:table -->\n<!-- /gen:table -->\n<!-- gen:table -->\n<!-- /gen:table -->\n")
            .contains("appears twice")
    );
    assert!(err("<!-- gen:Bad Name -->").contains("bad region name"));
    // inline duplicates are fine
    let twice = "<!-- gen:n -->1<!-- /gen:n --> and <!-- gen:n -->2<!-- /gen:n -->";
    assert_eq!(
        render(twice, &fake).unwrap(),
        "<!-- gen:n -->42<!-- /gen:n --> and <!-- gen:n -->42<!-- /gen:n -->"
    );
}
