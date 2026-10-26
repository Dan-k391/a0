//! Packages (D81, `crates/a0-driver/src/package.rs`): a program with a path dependency and a git
//! dependency, the lock file that pins the git one, `update`, `--locked`, reproducible builds, and
//! the errors a manifest can cause. The git dependency is a repository made by the test (git must
//! be installed; without it the test says so and checks the rest).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("b0-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Runs `b0` in `dir` with its own cache of git repositories.
fn b0(dir: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_b0"))
        .current_dir(dir)
        .env("A0_HOME", home)
        .args(args)
        .output()
        .unwrap()
}

fn text(o: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&o.stderr).replace("\r\n", "\n"),
    )
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=A0", "-c", "user.email=a0@example.com"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn has_git() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// `util` (a path dependency with a library and a module) and the app; `json` is added by the
/// tests that have git.
fn packages(root: &Path, json: Option<&str>) {
    write(
        &root.join("util/b0.toml"),
        "[package]\nname = \"util\"\nversion = \"0.1.0\"\n",
    );
    write(
        &root.join("util/src/lib.b0"),
        "import strings;\n\npub fn twice(x: int) -> int {\n  return x * 2;\n}\n\n\
         pub fn loud(s: string) -> string {\n  return strings.shout(s);\n}\n",
    );
    write(
        &root.join("util/src/strings.b0"),
        "pub fn shout(s: string) -> string {\n  return s + \"!\";\n}\n",
    );
    let mut manifest =
        "[package]\nname = \"app\"\n\n[dependencies]\nutil = { path = \"../util\" }\n".to_string();
    let mut imports = "import util;\nimport util.strings;\nimport helpers;\n".to_string();
    let mut body = String::new();
    if let Some(url) = json {
        manifest.push_str(&format!(
            "json = {{ git = \"{}\", branch = \"main\" }}\n",
            url.replace('\\', "/")
        ));
        imports.push_str("import json;\n");
        body.push_str("  print(json.version());\n");
    }
    write(&root.join("app/b0.toml"), &manifest);
    write(
        &root.join("app/src/main.b0"),
        &format!(
            "{imports}\nfn main() {{\n  print(util.twice(21));\n  print(strings.shout(\"hi\"));\n  \
             print(util.loud(\"hey\"));\n  print(helpers.three());\n{body}}}\n"
        ),
    );
    write(
        &root.join("app/src/helpers.b0"),
        "pub fn three() -> int {\n  return 3;\n}\n",
    );
}

/// A git repository holding the package `json`, whose `version()` says `version`.
fn json_repo(dir: &Path, version: &str) -> String {
    write(
        &dir.join("b0.toml"),
        "[package]\nname = \"json\"\nversion = \"1.0.0\"\n",
    );
    write(
        &dir.join("src/lib.b0"),
        &format!("pub fn version() -> string {{\n  return \"{version}\";\n}}\n"),
    );
    if !dir.join(".git").exists() {
        git(dir, &["init", "--quiet"]);
        git(dir, &["checkout", "--quiet", "-b", "main"]);
    }
    git(dir, &["add", "."]);
    git(dir, &["commit", "--quiet", "-m", version]);
    git(dir, &["rev-parse", "HEAD"])
}

#[test]
fn a_package_runs_with_a_path_dependency() {
    let root = scratch("pkg-path");
    packages(&root, None);
    let home = root.join("home");
    let app = root.join("app");
    let out = b0(&app, &home, &["run"]);
    assert_eq!(
        text(&out),
        ("42\nhi!\nhey!\n3\n".to_string(), String::new()),
        "{:?}",
        text(&out)
    );
    // From a directory inside the package too, and `hir` names modules after their packages.
    let out = b0(&app.join("src"), &home, &["hir"]);
    let (hir, err) = text(&out);
    assert!(out.status.success(), "{err}");
    assert!(
        hir.contains("fn `util.strings.shout`") && hir.contains("fn `helpers.three`"),
        "{hir}"
    );
    // A path dependency needs no lock file.
    assert!(!app.join("b0.lock").exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn git_dependencies_are_fetched_pinned_and_updated() {
    if !has_git() {
        eprintln!("skipped: git is not installed");
        return;
    }
    let root = scratch("pkg-git");
    let repo = root.join("json");
    std::fs::create_dir_all(&repo).unwrap();
    let first = json_repo(&repo, "v1");
    packages(&root, Some(&repo.to_string_lossy()));
    let home = root.join("home");
    let app = root.join("app");

    let out = b0(&app, &home, &["run"]);
    let (stdout, stderr) = text(&out);
    assert_eq!(stdout, "42\nhi!\nhey!\n3\nv1\n", "{stderr}");
    assert!(stderr.contains("fetching"), "{stderr}");
    let lock = std::fs::read_to_string(app.join("b0.lock")).unwrap();
    assert!(
        lock.contains(&first) && lock.contains("#branch=main"),
        "{lock}"
    );

    // A new commit on the branch changes nothing until `update`.
    let second = json_repo(&repo, "v2");
    let out = b0(&app, &home, &["run"]);
    assert_eq!(text(&out).0, "42\nhi!\nhey!\n3\nv1\n");
    assert!(!text(&out).1.contains("fetching"), "{}", text(&out).1);
    let out = b0(&app, &home, &["update"]);
    assert!(out.status.success(), "{}", text(&out).1);
    let lock = std::fs::read_to_string(app.join("b0.lock")).unwrap();
    assert!(lock.contains(&second) && !lock.contains(&first), "{lock}");
    let out = b0(&app, &home, &["run", "--locked"]);
    assert_eq!(text(&out).0, "42\nhi!\nhey!\n3\nv2\n", "{}", text(&out).1);

    // `--locked` refuses a dependency the lock file does not pin.
    let manifest = std::fs::read_to_string(app.join("b0.toml")).unwrap();
    std::fs::write(
        app.join("b0.toml"),
        format!(
            "{manifest}old = {{ git = \"{}\", rev = \"{first}\" }}\n",
            repo.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    let out = b0(&app, &home, &["build", "--locked"]);
    let (_, stderr) = text(&out);
    assert!(
        !out.status.success() && stderr.contains("E5005") && stderr.contains("`old`"),
        "{stderr}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn package_builds_are_reproducible() {
    let root = scratch("pkg-repro");
    let git = has_git();
    let repo = root.join("json");
    let url = git.then(|| {
        std::fs::create_dir_all(&repo).unwrap();
        json_repo(&repo, "v1");
        repo.to_string_lossy().into_owned()
    });
    packages(&root, url.as_deref());
    let home = root.join("home");
    let app = root.join("app");
    let exe = format!("app{}", std::env::consts::EXE_SUFFIX);
    let mut builds = Vec::new();
    for i in 0..2 {
        // A second apart, so that a time stamp in the executable would show.
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }
        let out = b0(&app, &home, &["build"]);
        assert!(out.status.success(), "{}", text(&out).1);
        builds.push(std::fs::read(app.join("build").join(&exe)).unwrap());
    }
    assert!(builds[0] == builds[1], "two builds differ");
    let run = Command::new(app.join("build").join(&exe)).output().unwrap();
    let expected = if git {
        "42\nhi!\nhey!\n3\nv1\n"
    } else {
        "42\nhi!\nhey!\n3\n"
    };
    assert_eq!(text(&run).0, expected);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn package_errors_say_what_is_wrong() {
    let root = scratch("pkg-errors");
    let home = root.join("home");
    // No package and no file.
    let out = b0(&root, &home, &["build"]);
    let (_, stderr) = text(&out);
    assert!(
        stderr.contains("E5006") && stderr.contains("no b0.toml"),
        "{stderr}"
    );
    // A dependency that is not a package, and one that is no dependency.
    write(
        &root.join("app/b0.toml"),
        "[package]\nname = \"app\"\n\n[dependencies]\nlost = { path = \"../lost\" }\nweird = \"1.0\"\n",
    );
    write(&root.join("app/src/main.b0"), "fn main() {\n}\n");
    let out = b0(&root.join("app"), &home, &["run"]);
    let (_, stderr) = text(&out);
    for wanted in [
        "error[E5002]: the dependency `weird` says neither a path nor a git repository",
        "b0.toml:6:9",
        "there is no registry",
    ] {
        assert!(stderr.contains(wanted), "no {wanted:?} in:\n{stderr}");
    }
    write(
        &root.join("app/b0.toml"),
        "[package]\nname = \"app\"\n\n[dependencies]\nlost = { path = \"../lost\" }\n",
    );
    let out = b0(&root.join("app"), &home, &["run"]);
    let (_, stderr) = text(&out);
    for wanted in [
        "error[E5003]: the dependency `lost` is not a package",
        "no b0.toml in ../lost",
    ] {
        assert!(stderr.contains(wanted), "no {wanted:?} in:\n{stderr}");
    }
    // An import of a module a dependency does not have.
    packages(&root, None);
    write(
        &root.join("app/src/main.b0"),
        "import util.missing;\n\nfn main() {\n}\n",
    );
    let out = b0(&root.join("app"), &home, &["run"]);
    let (_, stderr) = text(&out);
    for wanted in [
        "error[E1010]: cannot find module `util.missing`",
        "looked for ../util/src/missing.b0",
    ] {
        assert!(stderr.contains(wanted), "no {wanted:?} in:\n{stderr}");
    }
    std::fs::remove_dir_all(&root).ok();
}
