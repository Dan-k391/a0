# Packages

A *package* is a directory that holds a program of several files and a *manifest*: a small file
that names the program and the other packages it uses. This page shows packages in Minnow, one of
A0's example languages. Every language built on A0 that has imports gets packages in the same way,
with its own name in place of `minnow`.

To follow along, you need the `minnow` command. It comes in the
[release archive](https://github.com/Dan-k391/a0/releases/latest), or you can build it in a clone
of A0 with `cargo build -p minnow` (it is then `target/debug/minnow`).

## A package of two files

Make a directory `hello` with three files in it:

```
hello/
  minnow.toml
  src/
    main.mn
    greet.mn
```

`minnow.toml` is the manifest. It gives the package a name:

```toml
[package]
name = "hello"
version = "0.1.0"
```

`src/main.mn` is the program. It imports the module `greet`, which is the file `src/greet.mn`:

```
import greet;

fn main() {
  print(greet.hello("world"));
}
```

`src/greet.mn`:

```
pub fn hello(name: string) -> string {
  return "Hello, " + name + "!";
}
```

In the `hello` directory, or any directory under it, run the program. You do not name a file:
the command finds the manifest and runs the package's main file, `src/main.mn`.

```
$ minnow run
Hello, world!
```

Build an executable. It goes into `build/`, named after the package (`build/hello.exe` on
Windows):

```
$ minnow build
$ ./build/hello
Hello, world!
```

## Commands in a package

Without a file, these commands work on the package of the current directory:

| Command | What it does |
|---|---|
| `minnow run` | runs the main file |
| `minnow run -- a b` | runs it with the arguments `a` and `b` |
| `minnow build` | builds `build/<name>` (`build/<name>.exe` on Windows) |
| `minnow check` | compiles the program and reports errors, without running or building it |
| `minnow hir`, `minnow ir` | print the program in A0's [high-level IR](hir.md) or [core IR](ir.md) |
| `minnow update` | takes the newest commits of the git dependencies ([below](#git-dependencies-and-the-lock-file)) |

`build --locked` and `run --locked` refuse to change the lock file
([below](#git-dependencies-and-the-lock-file)).

Given a file, as in `minnow run src/main.mn`, a command still finds the file's package. It looks
for the nearest `minnow.toml` in the file's directory or a directory above it, and finds the file's
imports through that package. The language server and the debugger do the same.

## Dependencies

A package can use other packages, its *dependencies*. Here `hello` uses a package `util` from the
directory next to it:

```
util/
  minnow.toml
  src/
    lib.mn
    strings.mn
hello/
  minnow.toml
  src/
    main.mn
    greet.mn
```

`util/minnow.toml`:

```toml
[package]
name = "util"
version = "0.1.0"
```

`util/src/lib.mn` is what other packages get when they write `import util`. It uses the module
`strings` of its own package:

```
import strings;

pub fn twice(x: int) -> int {
  return x * 2;
}

pub fn loud(s: string) -> string {
  return strings.shout(s);
}
```

`util/src/strings.mn`:

```
pub fn shout(s: string) -> string {
  return s + "!";
}
```

`hello/minnow.toml` names the dependency under `[dependencies]`. The path is relative to the
manifest:

```toml
[package]
name = "hello"
version = "0.1.0"

[dependencies]
util = { path = "../util" }
```

`hello/src/main.mn` imports the library as `util`, and the dependency's other module as
`util.strings`:

```
import greet;
import util;
import util.strings;

fn main() {
  print(greet.hello("world"));
  print(util.twice(21));
  print(util.loud("hey"));
  print(strings.shout("hi"));
}
```

```
$ minnow run
Hello, world!
42
hey!
hi!
```

## Git dependencies and the lock file

A dependency can also come from a git repository. Add a line to `[dependencies]` that names the
repository and, optionally, a tag, a branch or a commit. In this example the repository is a local
directory, but any URL that `git clone` accepts works:

```toml
[dependencies]
util = { path = "../util" }
json = { git = "/home/daniel/docs-build/json", tag = "v1.0" }
```

The first build or run fetches the repository:

```
$ minnow run
minnow: fetching /home/daniel/docs-build/json
Hello, world!
42
hey!
hi!
v1
```

It also writes the *lock file*, `minnow.lock`, next to the manifest. The lock file records the
exact commit that the tag, branch or default branch pointed to:

```toml
# Written by `minnow build`: the commits git dependencies are pinned to. Commit it.

[git]
"/home/daniel/docs-build/json#tag=v1.0" = "1a16a6c7e16128c49a5e1561164041cc13a55351"
```

Later builds use that commit, even if the branch or tag moves. They do not use the network when
the commit is already fetched. Commit the lock file with your code: with it, the same sources build
the same program on every machine.

To move to the newest commits, run `minnow update`. It fetches every git dependency again and
rewrites the lock file:

```
$ minnow update
minnow: fetching /home/daniel/docs-build/json
```

`--locked` (`minnow build --locked`, `minnow run --locked`) uses the lock file as it is and never
changes it. If the lock file does not pin every git dependency, or pins one the manifests no longer
name, the build fails. A CI job uses it to check that the committed lock file is complete:

```
$ minnow build --locked
error[E5005]: the lock file does not pin `json`
 --> minnow.toml:7:1
  |
7 | json = { git = "/home/daniel/docs-build/json", tag = "v1.0" }
  | ^^^^ not in the lock file
  |
  = note: `--locked` keeps minnow.lock as it is; build without it once
```

A0 fetches with the `git` command, so credentials, proxies and SSH keys work as they do for git
itself. Fetched repositories are kept in a cache that every language built on A0 shares. The cache
is the directory `A0_HOME` names, or `.a0` in your home directory. In it, `git/db` holds the
repositories, and `git/checkouts` holds one directory for each commit used.

Only `build`, `run` and the other commands fetch. The language server and the debugger use what is
already in the cache. If a git dependency has not been fetched yet, they report that `minnow build`
fetches it.

## The manifest

The manifest is a TOML file named after the language (`minnow.toml`), with two tables.

`[package]`:

| Key | Meaning |
|---|---|
| `name` | The package's name. Required. The executable is named after it, and other packages import the package by this name unless they give it another. |
| `version` | A version string, for people. Nothing reads it yet. |
| `main` | The program's main file, relative to the manifest. The default is `src/main.<ext>`, then `main.<ext>`, where `<ext>` is the language's file extension. |
| `lib` | The file other packages get with `import <package>`. The default is `src/lib.<ext>`, then `lib.<ext>`. |

`[dependencies]`: each key is the name this package imports the dependency by. Its value says
where the dependency is:

| Value | Meaning |
|---|---|
| `{ path = "../util" }` | A directory with a package in it, relative to this manifest. |
| `{ git = "<url>" }` | The default branch of a git repository. |
| `{ git = "<url>", branch = "dev" }` | A branch of it. |
| `{ git = "<url>", tag = "v1" }` | A tag of it. |
| `{ git = "<url>", rev = "<commit>" }` | A commit of it. |

A git dependency takes at most one of `branch`, `tag` and `rev`. There is no registry: a
dependency is always a directory or a repository.

The lock file is named after the language too (`minnow.lock`). Builds write it; you do not edit it.

## Module names

A package's modules are its files. A module's name is the file's path from the directory of the
main file, without the extension and with `.` between the parts. For example, `src/net/http.mn` is
`net.http`. The name is the same whichever file imports it.

A dependency's modules are named after the dependency:

- `import util` gives its library file (`lib`).
- `import util.strings` gives the module `strings` of it: `strings.mn` in the directory of its
  library file, so `util/src/strings.mn` above.

Inside the dependency, its own files import each other by their own names (`import strings` in
`util/src/lib.mn` above), and import its dependencies by the names its manifest gives them.

`minnow hir` shows the full names. In the example above, the functions are named:

```
fn `greet.hello`(name: string) -> string {
fn `util.strings.shout`(s: string) -> string {
fn `util.twice`(x: int) -> int {
fn `util.loud`(s: string) -> string {
fn `json.version`() -> string {
fn main() {
```

When names meet:

- If a dependency and a module of the package have the same name, the dependency wins.
- Two packages that depend on the same directory or the same commit share one copy of it.
- If two different packages are imported under one name, both are loaded, and the second one's
  modules get a number added to their name.

## Reproducible builds

The same sources and the same lock file give the same executable, byte for byte. Four things make
this so:

- A0's compilation is deterministic.
- The lock file pins git dependencies to commits.
- File names in stack traces are the names diagnostics show, relative to the package, not the
  places files happen to be on disk. A file of a git dependency is named like
  `json@1a16a6c7e161/src/lib.mn`, not by its path in the cache.
- Linking adds no time stamps. On Windows, A0 passes `/Brepro` to the linker. On macOS, it sets
  `ZERO_AR_DATE=1`, and without `-g` it passes `-S`, which leaves out the linker's record of the
  temporary object files it read.

A test builds a package with a path dependency and a git dependency twice, a second apart, and
compares the two executables (`examples/minnow/tests/packages.rs`).

One exception: an executable built with debug information (`build -g`) records the paths of the
source files, so building in another directory changes it.

## Errors

Problems with packages have error codes of their own. `minnow explain <code>` says more about each.

| Code | Meaning |
|---|---|
| E5001 | The manifest is not valid TOML of the kind manifests use. |
| E5002 | The manifest has an unknown table or key, a package without a name, or a dependency without a path or a repository. |
| E5003 | A dependency's path has no manifest in it, or the main file named in the manifest does not exist. |
| E5004 | A git dependency cannot be fetched. The error includes git's message. |
| E5005 | The lock file is out of date, with `--locked`. |
| E5006 | There is no manifest here, and no file was given. |

They point at the manifest like any other error. Here the manifest has a dependency
`weird = "1.0"`, which names neither a path nor a repository:

```
$ minnow run
error[E5002]: the dependency `weird` says neither a path nor a git repository
 --> minnow.toml:8:9
  |
8 | weird = "1.0"
  |         ^^^^^ expected `{ path = "..." }` or `{ git = "..." }`
  |
  = note: dependencies come from a directory or a git repository; there is no registry
```

And outside any package, without a file:

```
$ minnow build
error[E5006]: no minnow.toml in this directory or its parents, and no file to build
  = help: give a file: `minnow build <file>`
```

## Packages in your own language

A language gets packages when it supports imports: it declares them in its driver with
`Language::modules`, and its grammar marks import statements with `@import`
([grammars](grammar.md#imports-import)). Its manifest is then `<language>.toml`, its lock file
`<language>.lock`, and its source files have the language's extension. Nothing else is needed.

For a tool of the language's own, `Language::packages_of` loads the package a file is in, without
fetching anything.
