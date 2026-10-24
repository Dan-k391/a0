//! Explicit target descriptions.
//!
//! Every decision that depends on the machine being compiled for (instruction set, object format,
//! calling convention, how to link) is made from a [`Target`], never from the host the compiler runs
//! on. [`Target::host`] exists for tools that really do mean "this machine".

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Arch {
    X86_64,
    Aarch64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ObjectFormat {
    Elf,
    MachO,
    Coff,
}

/// How executables are linked for a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LinkerFlavor {
    /// The system C compiler driver (`cc`) with GNU-style arguments.
    Gnu,
    /// `cc` on macOS (clang + ld64).
    Darwin,
    /// MSVC `link.exe`.
    Msvc,
}

/// The C calling convention used for every IR function on a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CallConv {
    SystemV,
    WindowsFastcall,
    AppleAarch64,
}

/// A supported target: an architecture and an operating system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    arch: Arch,
    os: Os,
}

impl Target {
    pub const X86_64_LINUX: Target = Target::new(Arch::X86_64, Os::Linux);
    pub const AARCH64_LINUX: Target = Target::new(Arch::Aarch64, Os::Linux);
    pub const X86_64_MACOS: Target = Target::new(Arch::X86_64, Os::MacOs);
    pub const AARCH64_MACOS: Target = Target::new(Arch::Aarch64, Os::MacOs);
    pub const X86_64_WINDOWS: Target = Target::new(Arch::X86_64, Os::Windows);
    pub const AARCH64_WINDOWS: Target = Target::new(Arch::Aarch64, Os::Windows);

    /// Every supported target.
    pub const ALL: [Target; 6] = [
        Target::X86_64_LINUX,
        Target::AARCH64_LINUX,
        Target::X86_64_MACOS,
        Target::AARCH64_MACOS,
        Target::X86_64_WINDOWS,
        Target::AARCH64_WINDOWS,
    ];

    pub const fn new(arch: Arch, os: Os) -> Target {
        Target { arch, os }
    }

    /// The machine this program was compiled for, if it is a supported target.
    pub fn host() -> Option<Target> {
        let arch = if cfg!(target_arch = "x86_64") {
            Arch::X86_64
        } else if cfg!(target_arch = "aarch64") {
            Arch::Aarch64
        } else {
            return None;
        };
        let os = if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            return None;
        };
        Some(Target::new(arch, os))
    }

    /// Parses a target triple (`x86_64-unknown-linux-gnu`) or a short name (`aarch64-macos`).
    pub fn parse(name: &str) -> Result<Target, String> {
        let lower = name.to_ascii_lowercase();
        let parts: Vec<&str> = lower.split('-').collect();
        let arch = match parts.first().copied() {
            Some("x86_64" | "amd64" | "x64") => Arch::X86_64,
            Some("aarch64" | "arm64") => Arch::Aarch64,
            _ => return Err(format!("unsupported architecture in target '{name}'")),
        };
        let os = if parts.contains(&"linux") {
            Os::Linux
        } else if parts
            .iter()
            .any(|p| matches!(*p, "darwin" | "macos" | "apple"))
        {
            Os::MacOs
        } else if parts.contains(&"windows") {
            Os::Windows
        } else {
            return Err(format!("unsupported operating system in target '{name}'"));
        };
        Ok(Target::new(arch, os))
    }

    pub fn arch(self) -> Arch {
        self.arch
    }

    pub fn os(self) -> Os {
        self.os
    }

    /// The canonical target triple, as understood by Cranelift and C toolchains.
    pub fn triple(self) -> &'static str {
        match (self.arch, self.os) {
            (Arch::X86_64, Os::Linux) => "x86_64-unknown-linux-gnu",
            (Arch::Aarch64, Os::Linux) => "aarch64-unknown-linux-gnu",
            (Arch::X86_64, Os::MacOs) => "x86_64-apple-darwin",
            (Arch::Aarch64, Os::MacOs) => "aarch64-apple-darwin",
            (Arch::X86_64, Os::Windows) => "x86_64-pc-windows-msvc",
            (Arch::Aarch64, Os::Windows) => "aarch64-pc-windows-msvc",
        }
    }

    /// Size of `ptr` in bytes.
    pub fn pointer_bytes(self) -> u32 {
        8
    }

    pub fn object_format(self) -> ObjectFormat {
        match self.os {
            Os::Linux => ObjectFormat::Elf,
            Os::MacOs => ObjectFormat::MachO,
            Os::Windows => ObjectFormat::Coff,
        }
    }

    pub fn linker_flavor(self) -> LinkerFlavor {
        match self.os {
            Os::Linux => LinkerFlavor::Gnu,
            Os::MacOs => LinkerFlavor::Darwin,
            Os::Windows => LinkerFlavor::Msvc,
        }
    }

    pub fn call_conv(self) -> CallConv {
        match (self.arch, self.os) {
            (Arch::X86_64, Os::Windows) => CallConv::WindowsFastcall,
            (Arch::Aarch64, Os::MacOs) => CallConv::AppleAarch64,
            _ => CallConv::SystemV,
        }
    }

    /// Whether code must be position independent (PIE executables on Linux, always on macOS).
    pub fn requires_pic(self) -> bool {
        matches!(self.os, Os::Linux | Os::MacOs)
    }

    pub fn exe_suffix(self) -> &'static str {
        match self.os {
            Os::Windows => ".exe",
            _ => "",
        }
    }

    pub fn object_suffix(self) -> &'static str {
        match self.os {
            Os::Windows => ".obj",
            _ => ".o",
        }
    }

    pub fn static_lib_suffix(self) -> &'static str {
        match self.os {
            Os::Windows => ".lib",
            _ => ".a",
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.triple())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triples_round_trip() {
        for t in Target::ALL {
            assert_eq!(Target::parse(t.triple()), Ok(t));
        }
    }

    #[test]
    fn short_names() {
        assert_eq!(Target::parse("aarch64-macos"), Ok(Target::AARCH64_MACOS));
        assert_eq!(Target::parse("x86_64-windows"), Ok(Target::X86_64_WINDOWS));
        assert_eq!(
            Target::parse("arm64-apple-darwin"),
            Ok(Target::AARCH64_MACOS)
        );
        assert!(Target::parse("riscv64-linux").is_err());
        assert!(Target::parse("x86_64-freebsd").is_err());
    }

    #[test]
    fn host_is_supported_here() {
        let host = Target::host().expect("tests run on a supported host");
        assert!(Target::ALL.contains(&host));
    }

    #[test]
    fn properties_follow_the_os() {
        assert_eq!(Target::X86_64_WINDOWS.object_format(), ObjectFormat::Coff);
        assert_eq!(
            Target::X86_64_WINDOWS.call_conv(),
            CallConv::WindowsFastcall
        );
        assert_eq!(Target::AARCH64_MACOS.call_conv(), CallConv::AppleAarch64);
        assert_eq!(Target::AARCH64_LINUX.linker_flavor(), LinkerFlavor::Gnu);
        assert_eq!(Target::X86_64_LINUX.exe_suffix(), "");
    }
}
