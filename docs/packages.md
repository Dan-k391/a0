# Packages

A program of several files can be a *package*: a directory with a manifest, which names the
program's main file and the packages it depends on. Every language with modules gets packages from
the driver (`crates/a0-driver/src/package.rs`, decision D81); this page uses Minnow.

```
app/
  minnow.toml        the manifest
  minnow.lock        written by builds: the commits git dependencies are pinned to
  src/
    main.mn          the program
    helpers.mn       `import helpers`
    net/http.mn      `import net.http`
  build/app          what `minnow build` makes
```

```toml
[package]
name = "app"
version = "0.1.0"

[dependencies]
util = { path = "../util" }
json = { git = "https://example.com/json.git", tag = "v1.2" }
```

In the package's directory, or any directory under it, the commands take no file:

```
minnow run                  # runs src/main.mn
minnow run -- a b           # with arguments
minnow build                # build/app (build/app.exe on Windows)
minnow build --locked       # fails rather than change minnow.lock (for CI)
minnow hir                  # prints the program's HIR
minnow update               # takes the newest commits of the git dependencies
```

Given a file, the commands also find its package (the nearest manifest in its directory or above)
and resolve its imports through it. So do the language server and the debugger.

## The manifest

`[package]`:

| Key | Meaning |
|---|---|
| `name` | The package's name (required): the executable's, and the name other packages import it by unless they rename it. |
| `version` | A version, for people; nothing reads it yet. |
| `main` | The program, relative to the manifest (default `src/main.<ext>`, then `main.<ext>`). |
| `lib` | What `import <package>` gives other packages (default `src/lib.<ext>`, then `lib.<ext>`). |

`[dependencies]`: each key is the name the package imports the dependency by, and its value says
where the dependency is:

| Value | Meaning |
|---|---|
| `{ path = "../util" }` | A directory, relative to this manifest, with a package in it. |
| `{ git = "<url>" }` | A git repository's default branch. |
| `{ git = "<url>", branch = "dev" }`, `tag = "v1"`, `rev = "<commit>"` | A branch, a tag or a commit of it. |

There is no registry: a dependency is a directory or a repository.

## Modules

A package's modules are its files, named by their paths from the directory of its main file
(`src/net/http.mn` is `net.http`), whichever file imports them. A dependency's are named after the
dependency: `import json` gives its library file, `import json.parser` the module `parser` of it
(`src/parser.mn` in the dependency). Inside the dependency, its own files import each other by
their own names (`import parser`), and its dependencies by theirs.

A dependency's name wins over a module of the same name in the package. Two packages that depend on
the same directory or commit share it; two different packages imported under one name are both
loaded, the second under a numbered name.

## Git dependencies and the lock file

Git dependencies are fetched with the `git` command (so credentials, proxies and SSH keys work as
for git itself) into a cache shared by every language built on A0: `A0_HOME`, or `.a0` in the home
directory (`git/db` holds the repositories, `git/checkouts` one directory per commit).

The first build resolves each dependency's branch, tag or default branch to a commit and writes it
to the lock file, `minnow.lock`, beside the manifest. Later builds use that commit, without the
network if the cache has it, until `minnow update` takes the newest commits. Commit the lock file:
with it, the same sources build the same program anywhere.

```toml
# Written by `minnow build`: the commits git dependencies are pinned to. Commit it.

[git]
"https://example.com/json.git#tag=v1.2" = "3f1c0de0b8e5c8a9d2f6a1b7c4e2d9f0a1b2c3d4"
```

`--locked` (`build --locked`, `run --locked`) fails with `E5005` if the lock file does not pin
every git dependency, instead of changing it; a CI job uses it to check that the committed lock
file is complete.

## Reproducible builds

The same sources and lock file give the same executable, byte for byte: A0's compilation is
deterministic, git dependencies are pinned, file names in traces are the package-relative names
diagnostics show (`json@3f1c0de0b8e5/src/parser.mn` for a git dependency, not the cache's path), and
linking adds no time stamps (`/Brepro` on Windows). A test builds a package with a path and a git
dependency twice, a second apart, and compares the executables (`examples/minnow/tests/packages.rs`).
Building in another directory changes the paths debug information (`-g`) records.

## Errors

Problems with packages have codes of their own (`minnow explain E5003`):

| Code | |
|---|---|
| E5001 | invalid manifest (not the TOML manifests use) |
| E5002 | invalid manifest entry (an unknown table or key, a missing name, a dependency without a path or a repository) |
| E5003 | dependency not found (no manifest where a path points) |
| E5004 | dependency cannot be fetched (with git's message) |
| E5005 | lock file out of date (`--locked`) |
| E5006 | no package here (no manifest, and no file given) |

They render against the manifest like any other diagnostic:

```
error[E5002]: the dependency `weird` says neither a path nor a git repository
 --> minnow.toml:6:9
  |
6 | weird = "1.0"
  |         ^^^^^ expected `{ path = "..." }` or `{ git = "..." }`
  |
  = note: dependencies come from a directory or a git repository; there is no registry
```

## For a language

A language gets packages by having modules (`Language::modules`, `docs/grammar.md`'s `@import`):
the manifest is `<language>.toml`, the lock file `<language>.lock`, and source files have the
language's extension. Nothing else is needed. `Language::packages_of` loads the package a file is
in, for tools of the language's own.
