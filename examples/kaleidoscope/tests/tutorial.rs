//! `docs/tutorial.md` cannot drift from the code. Every listing names the file and the `ANCHOR`
//! regions it shows and must be exactly that code; the listings together must cover all of the
//! compiler, the grammar and the files around them; and every command in a `console` block is run
//! and must print what the block shows.
//!
//! After changing the code, `A0_BLESS=1 cargo test -p kaleidoscope --test tutorial` rewrites the
//! listings and the commands' output in the tutorial; the prose around them is yours to check.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Files whose every line of code the tutorial must show.
const COVERED: [&str; 5] = [
    "examples/kaleidoscope/src/lib.rs",
    "examples/kaleidoscope/src/main.rs",
    "examples/kaleidoscope/kaleidoscope.a0g",
    "examples/kaleidoscope/runtime/kaleidoscope.c",
    "examples/kaleidoscope/build.rs",
];

/// `ANCHOR: name` or `ANCHOR_END: name` in a line: (is the end, name).
fn marker(line: &str) -> Option<(bool, &str)> {
    let i = line.find("ANCHOR")?;
    let rest = &line[i + "ANCHOR".len()..];
    let (end, rest) = match rest.strip_prefix("_END: ") {
        Some(r) => (true, r),
        None => (false, rest.strip_prefix(": ")?),
    };
    Some((end, rest.split_whitespace().next()?))
}

/// The lines of `file` a listing shows (all of them without anchors), with their line numbers, and
/// the text: marker lines dropped, the common indentation removed.
fn extract(file: &str, anchors: &[&str]) -> (Vec<usize>, String) {
    let text = std::fs::read_to_string(root().join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
    let lines: Vec<&str> = text.lines().collect();
    let mut picked = Vec::new();
    if anchors.is_empty() {
        picked = (0..lines.len()).collect();
    }
    for anchor in anchors {
        let start = lines
            .iter()
            .position(|l| marker(l) == Some((false, anchor)))
            .unwrap_or_else(|| panic!("{file} has no ANCHOR: {anchor}"));
        let end = (start..lines.len())
            .find(|&i| marker(lines[i]) == Some((true, anchor)))
            .unwrap_or_else(|| panic!("{file} has no ANCHOR_END: {anchor}"));
        picked.extend(start + 1..end);
    }
    picked.retain(|&i| marker(lines[i]).is_none());
    let indent = picked
        .iter()
        .filter(|&&i| !lines[i].trim().is_empty())
        .map(|&i| lines[i].len() - lines[i].trim_start_matches(' ').len())
        .min()
        .unwrap_or(0);
    let shown: Vec<&str> = picked
        .iter()
        .map(|&i| lines[i].get(indent..).unwrap_or("").trim_end())
        .collect();
    (picked, shown.join("\n").trim_matches('\n').to_string())
}

struct Block {
    /// The index of the opening fence's line.
    line: usize,
    info: Vec<String>,
    body: String,
}

fn blocks(markdown: &str) -> Vec<Block> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(info) = lines[i].strip_prefix("```") {
            let end = (i + 1..lines.len())
                .find(|&j| lines[j] == "```")
                .unwrap_or_else(|| panic!("tutorial line {}: unclosed code block", i + 1));
            out.push(Block {
                line: i,
                info: info.split_whitespace().map(str::to_string).collect(),
                body: lines[i + 1..end].join("\n"),
            });
            i = end + 1;
        } else {
            i += 1;
        }
    }
    out
}

fn attr<'a>(info: &'a [String], name: &str) -> Option<&'a str> {
    info.iter()
        .find_map(|a| a.strip_prefix(name)?.strip_prefix('='))
}

/// Runs the commands of a `console` block in `work` and returns the block as it should read.
fn transcript(body: &str, work: &Path) -> String {
    let kaleidoscope = PathBuf::from(env!("CARGO_BIN_EXE_kaleidoscope"));
    let a0 = kaleidoscope.with_file_name(format!("a0{}", std::env::consts::EXE_SUFFIX));
    assert!(
        a0.exists(),
        "{} is missing: the tutorial's `a0` commands need it (cargo build -p a0-cli)",
        a0.display()
    );
    let mut out = Vec::new();
    for line in body.lines() {
        let Some(command) = line.strip_prefix("$ ") else {
            continue;
        };
        out.push(line.to_string());
        let mut words = command.split_whitespace();
        let program = match words.next().unwrap() {
            "a0" => a0.clone(),
            "kaleidoscope" => kaleidoscope.clone(),
            p => match p.strip_prefix("./") {
                Some(exe) => work.join(format!("{exe}{}", std::env::consts::EXE_SUFFIX)),
                None => panic!("the tutorial runs an unknown command: {command}"),
            },
        };
        let result = Command::new(&program)
            .args(words)
            .current_dir(work)
            .output()
            .unwrap_or_else(|e| panic!("{command}: {e}"));
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        )
        .replace("\r\n", "\n");
        let text = text.trim_end_matches('\n');
        if !text.is_empty() {
            out.push(text.to_string());
        }
    }
    out.join("\n")
}

#[test]
fn every_listing_is_the_code_and_every_command_prints_what_it_shows() {
    let tutorial = std::fs::read_to_string(root().join("docs/tutorial.md")).unwrap();
    let work = std::env::temp_dir().join(format!("kaleidoscope-tutorial-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::copy(
        crate_dir.join("kaleidoscope.a0g"),
        work.join("kaleidoscope.a0g"),
    )
    .unwrap();
    for dir in ["tests/programs", "tests/tutorial"] {
        for entry in std::fs::read_dir(crate_dir.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            std::fs::copy(&path, work.join(path.file_name().unwrap())).unwrap();
        }
    }

    let mut failures = Vec::new();
    let mut shown: BTreeSet<(String, usize)> = BTreeSet::new();
    // What each wrong block should read: (fence line, lines in the block, text).
    let mut corrected = Vec::new();
    for block in blocks(&tutorial) {
        let at = format!("tutorial line {}", block.line + 1);
        if let Some(file) = attr(&block.info, "file") {
            let anchors: Vec<&str> = attr(&block.info, "anchor")
                .map(|a| a.split(',').collect())
                .unwrap_or_default();
            let (lines, code) = extract(file, &anchors);
            shown.extend(lines.into_iter().map(|l| (file.to_string(), l)));
            if block.body != code {
                failures.push(format!(
                    "{at}: the listing of {file} {anchors:?} is not the code; it should read:\n{code}"
                ));
                corrected.push((block.line, block.body.lines().count(), code));
            }
        } else if block.info.first().map(String::as_str) == Some("console") {
            let expected = transcript(&block.body, &work);
            if block.body != expected {
                failures.push(format!(
                    "{at}: the commands print something else; the block should read:\n{expected}"
                ));
                corrected.push((block.line, block.body.lines().count(), expected));
            }
        } else {
            failures.push(format!(
                "{at}: a listing must name its file (file=...) or be a console block"
            ));
        }
    }
    std::fs::remove_dir_all(&work).ok();
    if std::env::var("A0_BLESS").is_ok_and(|v| v == "1") && !corrected.is_empty() {
        // From the last block, so the earlier ones keep their line numbers.
        let mut lines: Vec<String> = tutorial.lines().map(str::to_string).collect();
        for (fence, old_len, new) in corrected.into_iter().rev() {
            lines.splice(
                fence + 1..fence + 1 + old_len,
                new.lines().map(str::to_string),
            );
        }
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(root().join("docs/tutorial.md"), text).unwrap();
        failures.retain(|f| !f.starts_with("tutorial line"));
    }

    // Every line of code is in some listing; blank lines, comments and markers may be left out.
    for file in COVERED {
        let text = std::fs::read_to_string(root().join(file)).unwrap();
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            let skippable =
                trimmed.is_empty() || trimmed.starts_with("//") || marker(line).is_some();
            if !skippable && !shown.contains(&(file.to_string(), i)) {
                failures.push(format!(
                    "{file}:{}: not shown in the tutorial: {trimmed}",
                    i + 1
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}
