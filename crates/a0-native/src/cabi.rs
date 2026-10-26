//! C structs passed to and returned from C functions by value, as each target's C ABI says
//! (`docs/ir.md`, "C structs"). The IR gives a struct parameter as a pointer to the struct's bytes
//! with its layout; here those bytes become what the ABI wants: registers, a copy on the stack, or
//! a pointer to a copy. Where a struct goes can depend on the arguments before it (how many
//! registers they left), so a call is planned as a whole.

use a0_ir::{CStruct, FuncId, Module, StructPass, Type};
use a0_target::{Arch, Os, Target};
use cranelift_codegen::ir::{self, types};

/// How one IR parameter of a C function is passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pass {
    /// A scalar, as it is.
    Scalar,
    /// A struct in registers: each part a scalar of the type at the offset, read from a copy of the
    /// struct padded to a multiple of 8 bytes.
    Registers(Vec<(ir::Type, u32)>),
    /// A struct copied onto the stack by the call itself (Cranelift's struct argument; x86-64),
    /// from a copy padded to the size given, a multiple of 8 bytes.
    Stack(u32),
    /// A struct passed as a pointer to a copy the caller makes.
    Indirect,
    /// A struct on the stack where Cranelift has no struct arguments (AArch64): `pad` arguments
    /// that use up the registers of the struct's class, as the ABI says a struct that does not fit
    /// does, then the struct's bytes as `words`, which then find no register and go on the stack.
    Spilled {
        pad: Vec<ir::Type>,
        words: Vec<(ir::Type, u32)>,
    },
    /// A struct result returned in registers, parts as for `Registers`.
    Returned(Vec<(ir::Type, u32)>),
    /// A struct result written through a pointer the caller passes (Cranelift's struct return).
    ResultPointer,
}

/// Registers left for arguments, by class.
struct Left {
    int: u32,
    float: u32,
}

impl Left {
    fn take(&mut self, parts: &[(ir::Type, u32)]) -> bool {
        let floats = parts.iter().filter(|(t, _)| t.is_float()).count() as u32;
        let ints = parts.len() as u32 - floats;
        if ints <= self.int && floats <= self.float {
            self.int -= ints;
            self.float -= floats;
            true
        } else {
            false
        }
    }
}

/// How each parameter of the C function `f` is passed on `target`.
pub(crate) fn plan(target: Target, m: &Module, f: FuncId, ptr: ir::Type) -> Vec<Pass> {
    let (int, float) = match (target.arch(), target.os()) {
        (Arch::X86_64, Os::Windows) => (4, 4),
        (Arch::X86_64, _) => (6, 8),
        (Arch::Aarch64, _) => (8, 8),
    };
    let mut left = Left { int, float };
    let windows = target.os() == Os::Windows;
    let aarch64 = target.arch() == Arch::Aarch64;
    let apple = aarch64 && target.os() == Os::MacOs;
    let mut out = Vec::new();
    for (i, &t) in m.func_params(f).iter().enumerate() {
        let pass = match m.func_struct_param(f, i as u32) {
            None => {
                let class = if matches!(t, Type::F32 | Type::F64) {
                    types::F64
                } else {
                    types::I64
                };
                left.take(&[(class, 0)]);
                Pass::Scalar
            }
            Some((StructPass::Result, s)) => match classify(target, s, true) {
                Some(parts) => Pass::Returned(parts),
                None => {
                    // The result's address takes an argument register except on AArch64 (x8).
                    if !aarch64 {
                        left.take(&[(ptr, 0)]);
                    }
                    Pass::ResultPointer
                }
            },
            Some((StructPass::ByValue, s)) => match classify(target, s, false) {
                // Windows passes every argument in its own slot, whatever came before.
                Some(parts) if windows => Pass::Registers(parts),
                Some(parts) if left.take(&parts) => Pass::Registers(parts),
                Some(parts) if aarch64 => {
                    let float = parts[0].0.is_float();
                    let (n, pad) = if float {
                        (std::mem::take(&mut left.float), types::F64)
                    } else {
                        (std::mem::take(&mut left.int), types::I64)
                    };
                    // On the stack, Apple packs a struct by its alignment, others by 8 bytes.
                    let (word, width) = match (apple, float, s.align) {
                        (true, true, 4) => (types::F32, 4),
                        _ => (pad, 8),
                    };
                    let words = (0..s.size.div_ceil(width))
                        .map(|k| (word, k * width))
                        .collect();
                    Pass::Spilled {
                        pad: vec![pad; n as usize],
                        words,
                    }
                }
                // System V: a struct that does not fit the registers left goes to memory, and
                // leaves them to the arguments after it.
                Some(_) => Pass::Stack(s.size.div_ceil(8) * 8),
                None if aarch64 || windows => {
                    left.take(&[(ptr, 0)]);
                    Pass::Indirect
                }
                None => Pass::Stack(s.size.div_ceil(8) * 8),
            },
        };
        out.push(pass);
    }
    out
}

/// The registers `s` travels in on `target` (`result`: returned from a function), or `None` when
/// it does not travel in registers: it is passed in memory (by pointer, or on the stack).
fn classify(target: Target, s: &CStruct, result: bool) -> Option<Vec<(ir::Type, u32)>> {
    match (target.arch(), target.os()) {
        // Windows x64: structs of 1, 2, 4 or 8 bytes travel as integers; others by pointer.
        (Arch::X86_64, Os::Windows) => {
            matches!(s.size, 1 | 2 | 4 | 8).then(|| vec![(types::I64, 0)])
        }
        // System V x86-64: up to 16 bytes in registers, each eightbyte integer unless all of its
        // fields are floats; larger structs in memory.
        (Arch::X86_64, _) => (s.size <= 16).then(|| {
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
                .collect()
        }),
        // AArch64 (AAPCS64, Apple's too): a homogeneous aggregate of up to four floats of one
        // type in floating-point registers; up to 16 bytes in general registers; larger by
        // pointer.
        (Arch::Aarch64, _) => {
            let _ = result;
            homogeneous_floats(s).or_else(|| {
                (s.size <= 16).then(|| {
                    (0..s.size.div_ceil(8))
                        .map(|k| (types::I64, 8 * k))
                        .collect()
                })
            })
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
