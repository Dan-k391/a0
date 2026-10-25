//! `a0 new <name>`: a new language project, ready to build, run, test and edit (`templates/starter`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The project's files: where each goes, and its template. `{{name}}` is the language's name,
/// `{{Name}}` the same capitalized, `{{crate}}` its name as a Rust path, `{{ext}}` its files'
/// extension, and `{{a0:<crate>}}` how its `Cargo.toml` depends on an A0 crate.
const FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        include_str!("../templates/starter/Cargo.toml.tmpl"),
    ),
    (
        "{{name}}.a0g",
        include_str!("../templates/starter/grammar.a0g"),
    ),
    (
        "src/lib.rs",
        include_str!("../templates/starter/src/lib.rs"),
    ),
    (
        "src/main.rs",
        include_str!("../templates/starter/src/main.rs"),
    ),
    (
        "tests/programs.rs",
        include_str!("../templates/starter/tests/programs.rs"),
    ),
    (
        "tests/lsp.rs",
        include_str!("../templates/starter/tests/lsp.rs"),
    ),
    (
        "tests/programs/basics.{{ext}}",
        include_str!("../templates/starter/tests/programs/basics.ext"),
    ),
    (
        "tests/programs/division.{{ext}}",
        include_str!("../templates/starter/tests/programs/division.ext"),
    ),
    (
        "examples/hello.{{ext}}",
        include_str!("../templates/starter/hello.ext"),
    ),
    (
        ".vscode/settings.json",
        include_str!("../templates/starter/settings.json"),
    ),
    ("README.md", include_str!("../templates/starter/README.md")),
    (".gitignore", "/target\n"),
];

const USAGE: &str =
    "usage: a0 new <name> [--dir <directory>] [--ext <extension>] [--a0 <path to A0>]

Creates a language project: its grammar, compiler, driver, tests, language server and VS Code
settings. The name is lower case letters, digits, '-' and '_', starting with a letter. The project
goes in <directory> (by default ./<name>), its files have the extension <extension> (by default
the name), and it depends on A0 from git, or on the A0 checkout at <path to A0>.";

/// What the templates' placeholders become.
struct Names {
    name: String,
    capitalized: String,
    krate: String,
    ext: String,
    a0: Option<PathBuf>,
}

impl Names {
    fn fill(&self, template: &str) -> String {
        let mut out = template
            .replace("{{name}}", &self.name)
            .replace("{{Name}}", &self.capitalized)
            .replace("{{crate}}", &self.krate)
            .replace("{{ext}}", &self.ext);
        while let Some(start) = out.find("{{a0:") {
            let end = start + out[start..].find("}}").expect("an unclosed placeholder") + 2;
            let krate = &out[start + 5..end - 2];
            let dependency = match &self.a0 {
                Some(root) => {
                    let path = root.join("crates").join(krate);
                    format!(
                        "{{ path = {:?} }}",
                        path.display().to_string().replace('\\', "/")
                    )
                }
                None => format!(
                    "{{ git = \"https://github.com/Dan-k391/a0\", tag = \"v{}\" }}",
                    env!("CARGO_PKG_VERSION")
                ),
            };
            out.replace_range(start..end, &dependency);
        }
        out
    }
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn new(args: &[String]) -> ExitCode {
    let mut name = None;
    let (mut dir, mut ext, mut a0) = (None, None, None);
    let mut i = 0;
    while i < args.len() {
        let value = args.get(i + 1).cloned();
        match (args[i].as_str(), value) {
            ("--dir", Some(v)) => dir = Some(PathBuf::from(v)),
            ("--ext", Some(v)) => ext = Some(v.trim_start_matches('.').to_string()),
            ("--a0", Some(v)) => a0 = Some(PathBuf::from(v)),
            (n, _) if name.is_none() && !n.starts_with('-') => {
                name = Some(n.to_string());
                i += 1;
                continue;
            }
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
        i += 2;
    }
    let Some(name) = name else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if !valid_name(&name) {
        eprintln!(
            "a0: `{name}` cannot name a language: use lower case letters, digits, '-' and '_', starting with a letter"
        );
        return ExitCode::from(2);
    }
    let dir = dir.unwrap_or_else(|| PathBuf::from(&name));
    if dir.exists() && std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_some()) {
        eprintln!("a0: {} exists and is not empty", dir.display());
        return ExitCode::from(1);
    }
    let a0 = match a0 {
        Some(p) => match std::path::absolute(&p) {
            Ok(p) if p.join("crates/a0-hir").is_dir() => Some(p),
            _ => {
                eprintln!(
                    "a0: {} is not an A0 checkout (no crates/a0-hir)",
                    p.display()
                );
                return ExitCode::from(1);
            }
        },
        None => None,
    };
    let names = Names {
        capitalized: name[..1].to_uppercase() + &name[1..],
        krate: name.replace('-', "_"),
        ext: ext.unwrap_or_else(|| name.clone()),
        name,
        a0,
    };
    match write(&dir, &names) {
        Ok(()) => {
            println!(
                "Created {} in {}.\n\n    cd {}\n    cargo run -- run examples/hello.{}\n    cargo test",
                names.capitalized,
                dir.display(),
                dir.display(),
                names.ext
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("a0: cannot write {}: {e}", dir.display());
            ExitCode::from(1)
        }
    }
}

fn write(dir: &Path, names: &Names) -> std::io::Result<()> {
    for (path, template) in FILES {
        let path = dir.join(names.fill(path));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, names.fill(template))?;
    }
    Ok(())
}
