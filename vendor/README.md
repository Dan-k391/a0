# Vendored crates

Crates A0 builds from a copy here instead of crates.io, each with the reason (D77). Remove a copy
once the release it waits for is out, and update `[patch.crates-io]` in the root `Cargo.toml`.

| Crate | Version | Why | Change |
|---|---|---|---|
| `cranelift-object` | 0.137.0 | It writes no COFF relocations for AArch64's `adrp`/`add` pairs, so aarch64-windows objects could not be produced | `Aarch64AdrPrelPgHi21` and `Aarch64AddAbsLo12Nc` map to `IMAGE_REL_ARM64_PAGEBASE_REL21` and `IMAGE_REL_ARM64_PAGEOFFSET_12A` in `src/backend.rs` (marked `A0:`) |

The copies keep their licenses (Apache-2.0 WITH LLVM-exception).
