//! C structs passed to and returned from C functions by value, as each target's C ABI says
//! (`docs/ir.md`, "C structs"). The IR gives a struct parameter as a pointer to the struct's bytes
//! with its layout; here those bytes become what the ABI wants: registers, a copy on the stack, or
//! a pointer to a copy.

use a0_ir::{CStruct, Type};
use a0_target::{Arch, Os, Target};
use cranelift_codegen::ir::{self, types};

/// How a C struct is passed, or returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pass {
    /// In registers: each part a scalar of the type at the offset, read from (or written to) a copy
    /// of the struct padded to a multiple of 8 bytes.
    Registers(Vec<(ir::Type, u32)>),
    /// Copied onto the stack by the call itself (Cranelift's struct argument): SysV's MEMORY class.
    Stack(u32),
    /// Through a pointer: to a copy the caller makes, or, for a result, to where the callee
    /// writes it (Cranelift's struct return).
    Indirect,
}

/// How `s` is passed to a C function on `target` (`result`: returned from one).
pub(crate) fn classify(target: Target, s: &CStruct, result: bool) -> Pass {
    let large = if result {
        Pass::Indirect
    } else {
        Pass::Stack(s.size)
    };
    match (target.arch(), target.os()) {
        // Windows x64: structs of 1, 2, 4 or 8 bytes travel as integers; others by pointer.
        (Arch::X86_64, Os::Windows) => match s.size {
            1 | 2 | 4 | 8 => Pass::Registers(vec![(types::I64, 0)]),
            _ => Pass::Indirect,
        },
        // System V x86-64: up to 16 bytes in registers, each eightbyte integer unless all of its
        // fields are floats; larger structs in memory.
        (Arch::X86_64, _) => {
            if s.size > 16 {
                return large;
            }
            Pass::Registers(
                (0..s.size.div_ceil(8))
                    .map(|k| {
                        let mut floats = s
                            .fields
                            .iter()
                            .filter(|&&(_, at)| at / 8 == k)
                            .map(|&(t, _)| matches!(t, Type::F32 | Type::F64))
                            .peekable();
                        let float = floats.peek().is_some() && floats.all(|f| f);
                        (if float { types::F64 } else { types::I64 }, 8 * k)
                    })
                    .collect(),
            )
        }
        // AArch64 (AAPCS64, Apple's too): a homogeneous aggregate of up to four floats of one
        // type in floating-point registers; up to 16 bytes in general registers; larger by
        // pointer.
        (Arch::Aarch64, _) => {
            if let Some(parts) = homogeneous_floats(s) {
                return Pass::Registers(parts);
            }
            if s.size > 16 {
                return Pass::Indirect;
            }
            Pass::Registers(
                (0..s.size.div_ceil(8))
                    .map(|k| (types::I64, 8 * k))
                    .collect(),
            )
        }
    }
}

/// The parts of an AAPCS64 homogeneous floating-point aggregate: one to four fields of the same
/// float type, one after another with nothing else.
fn homogeneous_floats(s: &CStruct) -> Option<Vec<(ir::Type, u32)>> {
    let &(first, _) = s.fields.first()?;
    let (ty, width) = match first {
        Type::F32 => (types::F32, 4),
        Type::F64 => (types::F64, 8),
        _ => return None,
    };
    let n = s.fields.len() as u32;
    let contiguous = s
        .fields
        .iter()
        .enumerate()
        .all(|(i, &(t, at))| t == first && at == width * i as u32);
    (n <= 4 && contiguous && s.size == width * n)
        .then(|| s.fields.iter().map(|&(_, at)| (ty, at)).collect())
}
