# Releases

A release of A0 is a version tag, archives of the tools for every platform, and packages ready to
publish. The workflow `.github/workflows/release.yml` makes all of them when a tag `v*` is pushed
and puts them in a **draft** GitHub release; a maintainer checks it and publishes the release and
the packages by hand. Nothing is published automatically.

## The archives

`a0-<version>-<platform>.tar.gz` (`.zip` on Windows), for `x86_64-linux`, `aarch64-linux`,
`aarch64-macos`, `x86_64-macos`, `x86_64-windows` and `aarch64-windows`, each with a `.sha256`
file. Unpacked anywhere, it needs nothing installed beyond a C compiler's linker for native
executables of the host (`cc`, or MSVC on Windows):

| Directory | What |
|---|---|
| `bin/` | `a0`, `b0`, `minnow`, `lox`, `kaleidoscope`, `tin`, and `rust-lld` |
| `targets/` | the target packs (`docs/cross.md`): with `rust-lld`, `build --target` makes Linux and Windows executables from any host with nothing else installed |
| `lib/`, `include/` | the C API (`a0.h`, static and shared libraries) and `a0rt.h` for languages' C runtimes; on Linux and macOS, also the LLVM library `rust-lld` loads |
| `python/` | the Python package `a0`, with the shared library inside it (`pip install ./python`) |
| `editors/vscode/` | the VS Code extension's source |
| `docs/` | the book and the reference, as Markdown |

`release/package.py` makes the archive for the host (`--packs` for the target packs that
`targets/build.py` made) and the packages below; `release/check.py` unpacks an archive, runs a B0
program with it and cross-compiles one, as the workflow does on every platform.

## The packages

Made by the same script, published by a maintainer:

- **Crates** (`.crate` files, `cargo package`). Publish them in this order, each after the ones it
  depends on: `a0-diag`, `a0-target`, `a0-runtime`, `a0-ir`, `a0-syntax`, `a0-sema`, `a0-hir`,
  `a0-interp`, `a0-native`, `a0-wasm`, `a0-build`, `a0-driver`, `a0-capi`, `a0-cli`, `b0`
  (`cargo publish -p <crate>`). The A0 crates depend on each other at exactly the same version.
  The example languages and the playground's engine are not published (`publish = false`).
  The command-line driver is the crate `a0-cli`, since `a0` on crates.io is another project's
  (D88); `cargo install a0-cli` installs the command `a0`.
  One limit: the copy of `cranelift-object` in `vendor/` (D77), which aarch64-windows objects need,
  cannot go to crates.io with them, so `a0-native` from crates.io cannot write aarch64-windows
  objects until upstream Cranelift has the change; the archives, built from the repository, can.
- **Python wheels** (`a0_lang-<version>-py3-none-<platform>.whl`, when the `build` module is
  installed): the package and the shared library for one platform; publish with `twine upload`.
- **The VS Code extension** (`.vsix`, when `vsce` is installed): publish with `vsce publish`
  under the `publisher` in `editors/vscode/package.json`.

## Making a release

1. Finish the milestone: `docs/milestones.md`, the changelog's section for the version (the
   release's notes are taken from it by `release/notes.py`), the benchmark (below).
2. Set the version everywhere it is written: `Cargo.toml` (`version` and the A0 crates' `=x.y.z`
   in `[workspace.dependencies]`), `A0_VERSION_*` in `a0.h`, `__version__` in
   `bindings/python/a0/__init__.py` and `version` in `bindings/python/pyproject.toml`,
   `editors/vscode/package.json`, and `README.md`'s status. Tests check that the C header, the
   library and the Python package agree.
3. Commit, tag `v<version>` and push the tag; the workflow drafts the release. A tag can also be
   made on GitHub's page for a new release (a draft makes no tag: publish it, as a pre-release
   until it is checked); the workflow then adds the archives and packages to that release.
   A tag's run uses the workflow as it was at the tag; when that fails for a reason of the workflow
   alone, fix it on `main` and run it by hand (Actions, Release, Run workflow) with the tag as
   `tag`: it builds the tag's code with the fixed workflow and `release/` scripts, and adds the
   files to the tag's release.
4. Check the draft's archives, then publish it, and publish the packages.

## Performance from release to release

`python3 bench/run.py --record <version>` runs the benchmark (`docs/benchmarks.md`) and appends its
headline numbers to `bench/history.csv`, with the date and the machine. Numbers from different
machines are not comparable, so each release is measured on the machine of the release before it
too: check out the previous tag, run `bench/run.py --record <previous>` with the new release's
`bench/` copied in, then the new release. `python3 bench/history.py` prints the table that
`docs/benchmarks.md` shows ("From release to release").
