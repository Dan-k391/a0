//! Linking objects into executables with the system linker.

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
}

impl Default for LinkOptions {
    fn default() -> Self {
        LinkOptions {
            libraries: Vec::new(),
            stack_size: 8 << 20,
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
/// Linking needs the target's system linker and a runtime built for the target, so `target` must be
/// the host: cross-linking is out of scope.
pub fn link_executable(object: &[u8], target: Target, output: &Path, opts: &LinkOptions) -> Result<(), Diagnostics> {
    if Target::host() != Some(target) {
        return Err(link_error(&format!(
            "cannot link for {target}: linking is only supported for the host target ({})",
            Target::host().map(|t| t.triple()).unwrap_or("unknown")
        )));
    }
    let scratch = ScratchDir::new().map_err(|e| link_error(&format!("cannot create a temporary directory: {e}")))?;
    let write = |name: String, bytes: &[u8]| -> Result<PathBuf, Diagnostics> {
        let path = scratch.0.join(name);
        std::fs::write(&path, bytes)
            .map_err(|e| link_error(&format!("cannot write {}: {e}", path.display())))?;
        Ok(path)
    };
    let object_path = write(format!("program{}", target.object_suffix()), object)?;
    let mut libs = Vec::new();
    for lib in &opts.libraries {
        libs.push(write(format!("{}{}", lib.name, target.static_lib_suffix()), &lib.bytes)?);
    }
    libs.push(write(format!("a0rt{}", target.static_lib_suffix()), a0_runtime::static_library())?);

    let mut cmd = match target.linker_flavor() {
        LinkerFlavor::Msvc => {
            let mut cmd = cc::windows_registry::find(target.triple(), "link.exe").ok_or_else(|| {
                link_error("cannot find the MSVC linker (link.exe); install the Visual Studio C++ build tools")
            })?;
            cmd.arg("/NOLOGO")
                .arg(format!("/OUT:{}", output.display()))
                .arg("/SUBSYSTEM:CONSOLE")
                .arg(format!("/STACK:{}", opts.stack_size))
                .arg(&object_path)
                .args(&libs);
            cmd
        }
        LinkerFlavor::Gnu => {
            let mut cmd = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".to_string()));
            cmd.arg("-o").arg(output).arg(&object_path).args(&libs).arg("-lm");
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
    let result = cmd
        .output()
        .map_err(|e| link_error(&format!("cannot run the linker {:?}: {e}", cmd.get_program())))?;
    if !result.status.success() {
        let mut text = String::from_utf8_lossy(&result.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&result.stderr));
        return Err(Diagnostic::error("native.link", "the linker failed")
            .with_note(text.trim())
            .into());
    }
    Ok(())
}
