//! Linking objects into executables: with the system linker for the host, or with lld and a
//! target pack for any target (cross-linking, D79).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use a0_diag::{Diagnostic, Diagnostics};
use a0_target::{LinkerFlavor, Target};

/// A static library to link into an executable, for example a language's own C runtime.
#[derive(Clone, Debug)]
pub struct Library {
    /// A file-name-friendly name (no extension).
    pub name: String,
    /// The library file's contents, built for the target.
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct LinkOptions {
    /// Extra static libraries. They may use the A0 runtime, which is always linked after them.
    pub libraries: Vec<Library>,
    /// Stack size of the main thread in bytes, where the target lets the executable choose it.
    pub stack_size: u64,
    /// The object has debug information: on macOS it is then gathered into `<output>.dSYM` with
    /// `dsymutil`, since debuggers read it from object files, and this one is temporary; on
    /// Windows the linker writes it into `<output>.pdb`.
    pub debug_info: bool,
    /// With `debug_info` on Windows: a natvis file for the PDB, which tells Visual Studio and
    /// WinDbg how to show the program's values (empty for none).
    pub natvis: String,
}

impl Default for LinkOptions {
    fn default() -> Self {
        LinkOptions {
            libraries: Vec::new(),
            stack_size: 8 << 20,
            debug_info: false,
            natvis: String::new(),
        }
    }
}

fn link_error(message: &str) -> Diagnostics {
    Diagnostic::error("native.link", message).into()
}

/// A scratch directory that is removed when dropped.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> std::io::Result<ScratchDir> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "a0-link-{}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir)?;
        Ok(ScratchDir(dir))
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Links an object file (as produced by [`crate::compile_object`] with an entry point) together
/// with the A0 runtime and `opts.libraries` into the executable `output`.
///
/// For the host, the system's linker links against the runtime built with A0. For another target
/// (or with `A0_LINK=pack`), lld links against the target's pack: the runtime, the languages' C
/// runtimes and a C library built for it (`targets/build.py`), found by [`find_pack`].
pub fn link_executable(
    object: &[u8],
    target: Target,
    output: &Path,
    opts: &LinkOptions,
) -> Result<(), Diagnostics> {
    let with_pack =
        Target::host() != Some(target) || std::env::var("A0_LINK").is_ok_and(|v| v == "pack");
    if with_pack {
        return link_with_pack(object, target, output, opts);
    }
    let scratch = ScratchDir::new()
        .map_err(|e| link_error(&format!("cannot create a temporary directory: {e}")))?;
    let write = |name: String, bytes: &[u8]| -> Result<PathBuf, Diagnostics> {
        let path = scratch.0.join(name);
        std::fs::write(&path, bytes)
            .map_err(|e| link_error(&format!("cannot write {}: {e}", path.display())))?;
        Ok(path)
    };
    let object_path = write(format!("program{}", target.object_suffix()), object)?;
    let mut libs = Vec::new();
    for lib in &opts.libraries {
        libs.push(write(
            format!("{}{}", lib.name, target.static_lib_suffix()),
            &lib.bytes,
        )?);
    }
    libs.push(write(
        format!("a0rt{}", target.static_lib_suffix()),
        a0_runtime::static_library(),
    )?);

    let mut cmd = match target.linker_flavor() {
        LinkerFlavor::Msvc => {
            let mut cmd = cc::windows_registry::find(target.triple(), "link.exe").ok_or_else(|| {
                link_error("cannot find the MSVC linker (link.exe); install the Visual Studio C++ build tools")
            })?;
            // `/Brepro`: no time stamps, so that the same program links to the same bytes (D81).
            cmd.arg("/NOLOGO")
                .arg("/Brepro")
                .arg(format!("/OUT:{}", output.display()))
                .arg("/SUBSYSTEM:CONSOLE")
                .arg(format!("/STACK:{}", opts.stack_size))
                .arg(&object_path)
                .args(&libs)
                .args(windows_debug_args(opts, output, &scratch.0)?);
            cmd
        }
        LinkerFlavor::Gnu => {
            let mut cmd = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".to_string()));
            cmd.arg("-o")
                .arg(output)
                .arg(&object_path)
                .args(&libs)
                .arg("-lm")
                .arg("-lpthread");
            cmd
        }
        LinkerFlavor::Darwin => {
            let mut cmd = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".to_string()));
            cmd.arg("-o")
                .arg(output)
                .arg(&object_path)
                .args(&libs)
                .arg(format!("-Wl,-stack_size,{:#x}", opts.stack_size));
            cmd
        }
    };
    let result = cmd.output().map_err(|e| {
        link_error(&format!(
            "cannot run the linker {:?}: {e}",
            cmd.get_program()
        ))
    })?;
    if !result.status.success() {
        let mut text = String::from_utf8_lossy(&result.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&result.stderr));
        return Err(Diagnostic::error("native.link", "the linker failed")
            .with_note(text.trim())
            .into());
    }
    if opts.debug_info && target.linker_flavor() == LinkerFlavor::Darwin {
        // Missing dsymutil only costs the debug information.
        let _ = Command::new("dsymutil").arg(output).output();
    }
    Ok(())
}

/// The directory of target packs: `A0_TARGETS`, then `targets` beside the running executable (or
/// one directory up, as release archives put it), then the repository's `target/packs`, where
/// `targets/build.py` writes them.
fn pack_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = std::env::var_os("A0_TARGETS") {
        roots.push(std::path::PathBuf::from(dir));
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        roots.push(dir.join("targets"));
        roots.push(dir.join("..").join("targets"));
    }
    roots.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/packs"));
    roots
}

/// The name of `target`'s pack (its C library's triple).
pub fn pack_name(target: Target) -> Option<&'static str> {
    use a0_target::{Arch, Os};
    match (target.arch(), target.os()) {
        (Arch::X86_64, Os::Linux) => Some("x86_64-unknown-linux-musl"),
        (Arch::Aarch64, Os::Linux) => Some("aarch64-unknown-linux-musl"),
        (Arch::X86_64, Os::Windows) => Some("x86_64-pc-windows-msvc"),
        // Apple's SDK may not be redistributed, and Windows on ARM64 has no import libraries built
        // yet.
        _ => None,
    }
}

/// The pack for `target`, if one is installed.
pub fn find_pack(target: Target) -> Option<std::path::PathBuf> {
    let name = pack_name(target)?;
    pack_roots()
        .into_iter()
        .map(|r| r.join(name))
        .find(|p| p.join("PACK").exists())
}

/// lld: `A0_LLD`, a copy beside the running executable, the Rust toolchain's `rust-lld`, or one on
/// the `PATH`. Returns the program and the arguments that choose `flavor` (`gnu` for ELF, `link`
/// for COFF).
fn find_lld(flavor: &str) -> Option<(std::path::PathBuf, Vec<String>)> {
    let exe = std::env::consts::EXE_SUFFIX;
    let with_flavor = |p: std::path::PathBuf| (p, vec!["-flavor".to_string(), flavor.to_string()]);
    if let Some(p) = std::env::var_os("A0_LLD") {
        return Some(with_flavor(p.into()));
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        for name in ["lld", "rust-lld"] {
            let p = dir.join(format!("{name}{exe}"));
            if p.exists() {
                return Some(with_flavor(p));
            }
        }
    }
    if let Ok(out) = Command::new("rustc").args(["--print", "sysroot"]).output() {
        let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if let Ok(host) = Command::new("rustc")
            .args(["--print", "host-tuple"])
            .output()
        {
            let host = String::from_utf8_lossy(&host.stdout).trim().to_string();
            let p = Path::new(&sysroot)
                .join("lib/rustlib")
                .join(host)
                .join("bin")
                .join(format!("rust-lld{exe}"));
            if p.exists() {
                return Some(with_flavor(p));
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let only = if flavor == "gnu" {
            "ld.lld"
        } else {
            "lld-link"
        };
        let p = dir.join(format!("{only}{exe}"));
        if p.exists() {
            return Some((p, Vec::new()));
        }
        let p = dir.join(format!("lld{exe}"));
        if p.exists() {
            return Some(with_flavor(p));
        }
    }
    None
}

/// The linker's options for a PDB (`/DEBUG`) and its natvis file, with debug information.
fn windows_debug_args(
    opts: &LinkOptions,
    output: &Path,
    scratch: &Path,
) -> Result<Vec<String>, Diagnostics> {
    if !opts.debug_info {
        return Ok(Vec::new());
    }
    // The linker updates a PDB that is there, keeping what an earlier build put in it.
    let _ = std::fs::remove_file(output.with_extension("pdb"));
    let mut args = vec!["/DEBUG".to_string(), "/INCREMENTAL:NO".to_string()];
    if !opts.natvis.is_empty() {
        let path = scratch.join("a0.natvis");
        std::fs::write(&path, &opts.natvis)
            .map_err(|e| link_error(&format!("cannot write {}: {e}", path.display())))?;
        args.push(format!("/NATVIS:{}", path.display()));
    }
    Ok(args)
}

/// Links with lld against `target`'s pack (D79).
fn link_with_pack(
    object: &[u8],
    target: Target,
    output: &Path,
    opts: &LinkOptions,
) -> Result<(), Diagnostics> {
    use a0_target::Os;
    let Some(pack) = find_pack(target) else {
        let why = match pack_name(target) {
            Some(name) => format!(
                "no target pack {name} is installed (targets/build.py builds the packs into \
                 target/packs; A0_TARGETS names another directory)"
            ),
            None if target.os() == Os::MacOs => {
                "executables for macOS need Apple's SDK, which may not be redistributed: build on \
                 a Mac"
                    .to_string()
            }
            None => "there is no target pack for it yet".to_string(),
        };
        return Err(link_error(&format!("cannot link for {target}: {why}")));
    };
    let windows = target.os() == Os::Windows;
    let flavor = if windows { "link" } else { "gnu" };
    let Some((lld, flavor_args)) = find_lld(flavor) else {
        return Err(link_error(&format!(
            "cannot link for {target}: lld was not found (it comes with Rust as rust-lld; A0_LLD \
             names it)"
        )));
    };
    let scratch = ScratchDir::new()
        .map_err(|e| link_error(&format!("cannot create a temporary directory: {e}")))?;
    let object_path = scratch.0.join(format!("program{}", target.object_suffix()));
    std::fs::write(&object_path, object)
        .map_err(|e| link_error(&format!("cannot write {}: {e}", object_path.display())))?;
    // A language's C runtime, built for the target in the pack under the library's name.
    let mut libs = Vec::new();
    for lib in &opts.libraries {
        let file = if windows {
            format!("{}.lib", lib.name)
        } else {
            format!("lib{}.a", lib.name)
        };
        let path = pack.join(&file);
        if !path.exists() {
            return Err(link_error(&format!(
                "cannot link for {target}: the target pack has no {file} (the C runtime {} needs \
                 building for the target)",
                lib.name
            )));
        }
        libs.push(path);
    }
    let mut cmd = Command::new(&lld);
    cmd.args(&flavor_args);
    if windows {
        cmd.arg("/nologo")
            .arg("/Brepro")
            .arg(format!("/out:{}", output.display()))
            .arg("/subsystem:console")
            .arg("/entry:mainCRTStartup")
            .arg(format!("/stack:{}", opts.stack_size))
            .arg("/nodefaultlib")
            .arg(&object_path)
            .args(&libs)
            .arg(pack.join("a0rt.lib"))
            .arg(pack.join("start_windows.o"))
            .arg(pack.join("libucrt.a"))
            .arg(pack.join("libucrt_extra.a"))
            .arg(pack.join("libkernel32.a"))
            .arg(pack.join("libpsapi.a"))
            .arg(pack.join("libclang_rt.builtins.a"))
            .args(windows_debug_args(opts, output, &scratch.0)?);
    } else {
        // Static: the executable runs on any Linux with the architecture.
        cmd.arg("-static")
            .arg("-z")
            .arg("noexecstack")
            .arg("-o")
            .arg(output)
            .arg(pack.join("crt1.o"))
            .arg(pack.join("crti.o"))
            .arg(&object_path)
            .arg("--start-group")
            .args(&libs)
            .arg(pack.join("liba0rt.a"))
            .arg(pack.join("libc.a"))
            .arg(pack.join("libclang_rt.builtins.a"))
            .arg("--end-group")
            .arg(pack.join("crtn.o"));
    }
    let result = cmd
        .output()
        .map_err(|e| link_error(&format!("cannot run the linker {}: {e}", lld.display())))?;
    if !result.status.success() {
        let mut text = String::from_utf8_lossy(&result.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&result.stderr));
        return Err(Diagnostic::error("native.link", "the linker failed")
            .with_note(text.trim())
            .into());
    }
    Ok(())
}
