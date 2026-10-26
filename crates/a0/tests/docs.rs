//! The documentation's links: every relative link in the repository's Markdown names a file that
//! exists, and the book's table of contents names every chapter.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The Markdown files a reader follows links in.
fn markdown() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![root().join("docs")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "md") {
                out.push(path);
            }
        }
    }
    for f in ["README.md", "CHANGELOG.md", "AGENTS.md", "b0/README.md"] {
        out.push(root().join(f));
    }
    for entry in std::fs::read_dir(root().join("examples")).unwrap() {
        let readme = entry.unwrap().path().join("README.md");
        if readme.is_file() {
            out.push(readme);
        }
    }
    out.sort();
    out
}

/// The targets of the Markdown links `[text](target)` in `text`.
fn links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("](") {
        rest = &rest[i + 2..];
        if let Some(end) = rest.find(')') {
            out.push(rest[..end].to_string());
        }
    }
    out
}

#[test]
fn relative_links_name_files_that_exist() {
    let mut broken = Vec::new();
    for file in markdown() {
        let text = std::fs::read_to_string(&file).unwrap();
        for link in links(&text) {
            if link.contains("://") || link.starts_with('#') || link.starts_with("mailto:") {
                continue;
            }
            let path = link.split('#').next().unwrap();
            if path.is_empty() || path.contains(' ') {
                continue;
            }
            if !file.parent().unwrap().join(path).exists() {
                broken.push(format!("{}: {link}", file.display()));
            }
        }
    }
    assert!(broken.is_empty(), "broken links:\n{}", broken.join("\n"));
}

#[test]
fn the_book_lists_every_chapter() {
    let summary = std::fs::read_to_string(root().join("docs/SUMMARY.md")).unwrap();
    for entry in std::fs::read_dir(root().join("docs/book")).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        if name.ends_with(".md") {
            assert!(
                summary.contains(&format!("book/{name}")),
                "docs/SUMMARY.md does not list book/{name}"
            );
        }
    }
}
