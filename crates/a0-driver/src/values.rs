//! Values as the debugger shows them (`dap.rs`), read from the program's memory by the debug types
//! lowering records (`a0_ir::DebugKind`), or by the IR type of a variable that has none. The layouts
//! are the runtime's (`a0rt.h`): pointers in objects are 64-bit words on every target.

use a0_ir::{DebugKind, DebugType, Module, Type};

/// How to read a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Typed(DebugType),
    /// A dynamic value (`any`).
    Any,
    /// A reference to an object whose descriptor says its type.
    Object,
    /// A variable without a debug type: its IR type.
    Raw(Type),
}

const QNAN: u64 = 0x7ffc_0000_0000_0000;
const REF_TAG: u64 = 0xfffc_0000_0000_0000;
const INT_TAG: u64 = 0x7ffe_0000_0000_0000;
const WORD_REF: u64 = 1;
const WORD_ANY: u64 = 2;
const KIND_STRING: i32 = 1;
const KIND_ARRAY: i32 = 2;
const KIND_MAP: i32 = 3;
const KIND_INT: i32 = 4;
const KIND_OBJECT: i32 = 5;
/// An array element kind: a byte per element (`A0_ELEM_BYTE`).
const ELEM_BYTE: u64 = 3;
/// Elements and entries shown at most, in summaries and as children.
const SHOWN: usize = 100;
const SUMMARY: usize = 8;

/// The word at `address`.
fn word(address: u64) -> u64 {
    // SAFETY: the debugger reads the live objects of a stopped program, at addresses the program's
    // own values hold.
    unsafe { (address as usize as *const u64).read_unaligned() }
}

fn bytes(address: u64, len: usize) -> Vec<u8> {
    // SAFETY: as for `word`; strings hold `len` bytes.
    unsafe { std::slice::from_raw_parts(address as usize as *const u8, len).to_vec() }
}

fn string(p: u64) -> String {
    let len = word(p) as usize;
    let text = String::from_utf8_lossy(&bytes(p + 16, len.min(1 << 16))).into_owned();
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    if len > 1 << 16 {
        out.push('…');
    }
    out.push('"');
    out
}

/// The descriptor of the object at `p`: its name and kind.
fn descriptor(p: u64) -> (String, i32) {
    // The type is the word before the object (`a0rt.h`).
    let t = word(p - 8);
    let name = word(t);
    let mut text = Vec::new();
    for i in 0..256 {
        let b = bytes(name + i, 1)[0];
        if b == 0 {
            break;
        }
        text.push(b);
    }
    let kind = (word(t + 8) & 0xffff_ffff) as i32;
    (String::from_utf8_lossy(&text).into_owned(), kind)
}

fn float(bits: u64) -> String {
    let f = f64::from_bits(bits);
    if f.is_nan() {
        "nan".to_string()
    } else if f.is_infinite() {
        if f > 0.0 { "inf" } else { "-inf" }.to_string()
    } else {
        format!("{f}")
    }
}

/// The type name to show for a value.
pub fn type_name(m: &Module, w: u64, view: View) -> String {
    match view {
        View::Typed(t) => match m.debug_type_kind(t) {
            DebugKind::Boxed => type_name(m, w, View::Typed(m.debug_type_elems(t)[0])),
            _ => m.debug_type_name(t).to_string(),
        },
        View::Any => "any".to_string(),
        View::Object if w == 0 => "null".to_string(),
        View::Object => descriptor(w).0,
        View::Raw(t) => t.to_string(),
    }
}

/// A one-line summary of the value: `42`, `"hi"`, `[1, 2, 3]`, `Point { x: 1, y: 2 }`.
pub fn show(m: &Module, w: u64, view: View) -> String {
    summary(m, w, view, 0)
}

fn summary(m: &Module, w: u64, view: View, depth: u32) -> String {
    let t = match view {
        View::Raw(ty) => {
            return match ty {
                Type::F64 => float(w),
                Type::F32 => float((f32::from_bits(w as u32) as f64).to_bits()),
                Type::I1 => (w & 1 != 0).to_string(),
                Type::Ptr => format!("0x{w:x}"),
                _ => (w as i64).to_string(),
            };
        }
        View::Any => return any(m, w, depth),
        View::Object => return object(m, w, depth),
        View::Typed(t) => t,
    };
    let nested = depth > 1;
    match m.debug_type_kind(t) {
        DebugKind::Int => (w as i64).to_string(),
        DebugKind::UInt => w.to_string(),
        DebugKind::Float => float(w),
        DebugKind::Float32 => float((f32::from_bits(w as u32) as f64).to_bits()),
        DebugKind::Bool => (w & 1 != 0).to_string(),
        DebugKind::String if w == 0 => "null".to_string(),
        DebugKind::String => string(w),
        DebugKind::Nullable if w == 0 => "null".to_string(),
        DebugKind::Nullable => summary(m, w, View::Typed(m.debug_type_elems(t)[0]), depth),
        DebugKind::Boxed if w == 0 => "<not yet set>".to_string(),
        DebugKind::Boxed => summary(m, word(w), View::Typed(m.debug_type_elems(t)[0]), depth),
        DebugKind::Any => any(m, w, depth),
        DebugKind::Object => object(m, w, depth),
        DebugKind::Opaque => format!("<{}>", m.debug_type_name(t)),
        _ if w == 0 => "null".to_string(),
        DebugKind::Array if nested => format!("[…; {}]", word(w)),
        DebugKind::Map if nested => format!("{{…; {}}}", word(w)),
        DebugKind::Struct | DebugKind::Inline if nested => {
            format!("{} {{…}}", m.debug_type_name(t))
        }
        DebugKind::Array
        | DebugKind::Map
        | DebugKind::Struct
        | DebugKind::Inline
        | DebugKind::Enum => {
            let items = parts(m, w, View::Typed(t));
            let shown: Vec<String> = items
                .iter()
                .take(SUMMARY)
                .map(|(name, iw, iv)| {
                    let v = summary(m, *iw, *iv, depth + 1);
                    match m.debug_type_kind(t) {
                        DebugKind::Struct | DebugKind::Inline => format!("{name}: {v}"),
                        DebugKind::Map => format!("{name}: {v}"),
                        _ => v,
                    }
                })
                .collect();
            let more = if items.len() > SUMMARY { ", …" } else { "" };
            match m.debug_type_kind(t) {
                DebugKind::Array => format!("[{}{more}]", shown.join(", ")),
                DebugKind::Map => format!("{{{}{more}}}", shown.join(", ")),
                DebugKind::Struct | DebugKind::Inline => {
                    format!("{} {{ {} }}", m.debug_type_name(t), shown.join(", "))
                }
                _ => {
                    let variant = variant_name(m, w, t);
                    if shown.is_empty() {
                        variant
                    } else {
                        format!("{variant}({})", shown.join(", "))
                    }
                }
            }
        }
    }
}

fn variant_name(m: &Module, w: u64, t: DebugType) -> String {
    let tag = word(w) as u32;
    if tag < m.debug_type_variant_count(t) {
        m.debug_type_variant(t, tag).0.to_string()
    } else {
        format!("<variant {tag}>")
    }
}

/// A dynamic value.
fn any(m: &Module, w: u64, depth: u32) -> String {
    match w {
        _ if w == QNAN | 1 => "()".to_string(),
        _ if w == QNAN | 2 => "false".to_string(),
        _ if w == QNAN | 3 => "true".to_string(),
        _ if w & REF_TAG == REF_TAG => object(m, w & 0xffff_ffff_ffff, depth),
        _ if w >> 48 == INT_TAG >> 48 => (((w << 16) as i64) >> 16).to_string(),
        _ if w & QNAN != QNAN => float(w),
        _ => format!("<any 0x{w:x}>"),
    }
}

/// A reference to an object of the type its descriptor says.
fn object(m: &Module, p: u64, depth: u32) -> String {
    if p == 0 {
        return "null".to_string();
    }
    match dynamic(m, p) {
        Some(view) => summary(m, p, view, depth),
        None => match descriptor(p) {
            (_, KIND_STRING) => string(p),
            (_, KIND_INT) => (word(p) as i64).to_string(),
            (_, KIND_ARRAY) => format!("[…; {}]", word(p)),
            (_, KIND_MAP) => format!("{{…; {}}}", word(p)),
            (_, KIND_OBJECT) if depth > 1 => "{…}".to_string(),
            (_, KIND_OBJECT) => {
                let fields: Vec<String> = fields(p)
                    .into_iter()
                    .take(SUMMARY)
                    .map(|(name, w, _)| format!("{name}: {}", any(m, w, depth + 1)))
                    .collect();
                format!("{{{}}}", fields.join(", "))
            }
            (name, _) => format!("<{name}>"),
        },
    }
}

/// The debug type of an object, from its descriptor's name (`Point`, or `Shape::Circle`).
fn dynamic(m: &Module, p: u64) -> Option<View> {
    let (name, kind) = descriptor(p);
    if kind != 0 {
        return None;
    }
    let family = name.split("::").next().unwrap_or(&name);
    m.debug_type_named(family).map(View::Typed)
}

/// Whether the value has parts to show (elements, fields, entries).
pub fn has_parts(m: &Module, w: u64, view: View) -> bool {
    !parts(m, w, view).is_empty()
}

/// The parts of a value, with their names: elements (`[0]`), fields, a variant's fields, a map's
/// entries (named by their keys), or what a box or a nullable holds.
pub fn parts(m: &Module, w: u64, view: View) -> Vec<(String, u64, View)> {
    let t = match view {
        View::Raw(_) => return Vec::new(),
        View::Any => {
            return if w & REF_TAG == REF_TAG && w != QNAN | 1 {
                parts(m, w & 0xffff_ffff_ffff, View::Object)
            } else {
                Vec::new()
            };
        }
        View::Object => {
            if w == 0 {
                return Vec::new();
            }
            return match dynamic(m, w) {
                Some(view) => parts(m, w, view),
                None => match descriptor(w).1 {
                    KIND_ARRAY => {
                        let kind = word(w + 24);
                        let elem = match kind {
                            WORD_REF => View::Object,
                            WORD_ANY => View::Any,
                            ELEM_BYTE => View::Raw(Type::I1),
                            _ => View::Raw(Type::I64),
                        };
                        elements(w, elem)
                    }
                    KIND_OBJECT => fields(w),
                    _ => Vec::new(),
                },
            };
        }
        View::Typed(t) => t,
    };
    if w == 0 {
        return Vec::new();
    }
    match m.debug_type_kind(t) {
        DebugKind::Array => elements(w, View::Typed(m.debug_type_elems(t)[0])),
        DebugKind::Map => {
            let (key, value) = (m.debug_type_elems(t)[0], m.debug_type_elems(t)[1]);
            let used = word(w + 8) as usize;
            let entries = word(w + 24);
            (0..used)
                .map(|i| entries + 32 * i as u64)
                .filter(|&e| word(e + 24) != 0)
                .take(SHOWN)
                .map(|e| {
                    let name = summary(m, word(e), View::Typed(key), 2);
                    (name, word(e + 8), View::Typed(value))
                })
                .collect()
        }
        DebugKind::Struct | DebugKind::Inline => (0..m.debug_type_field_count(t))
            .map(|i| {
                let (name, ft) = m.debug_type_field(t, i);
                let (offset, size) = m.debug_type_field_place(t, i);
                let at = w + offset as u64;
                // An inline struct in a field is its bytes there; anything else is read, and a
                // narrow signed integer extended by its sign.
                let v = match m.debug_type_kind(ft) {
                    DebugKind::Inline => at,
                    kind if size < 8 => {
                        let mut le = [0u8; 8];
                        le[..size as usize].copy_from_slice(&bytes(at, size as usize));
                        let raw = u64::from_le_bytes(le);
                        let shift = 64 - 8 * size;
                        if kind == DebugKind::Int {
                            (((raw << shift) as i64) >> shift) as u64
                        } else {
                            raw
                        }
                    }
                    _ => word(at),
                };
                (name.to_string(), v, View::Typed(ft))
            })
            .collect(),
        DebugKind::Enum => {
            let tag = word(w) as u32;
            if tag >= m.debug_type_variant_count(t) {
                return Vec::new();
            }
            let (_, fields) = m.debug_type_variant(t, tag);
            fields
                .iter()
                .enumerate()
                .map(|(i, &ft)| (i.to_string(), word(w + 8 * (i as u64 + 1)), View::Typed(ft)))
                .collect()
        }
        DebugKind::Nullable => parts(m, w, View::Typed(m.debug_type_elems(t)[0])),
        DebugKind::Boxed => parts(m, word(w), View::Typed(m.debug_type_elems(t)[0])),
        DebugKind::Any => parts(m, w, View::Any),
        DebugKind::Object => parts(m, w, View::Object),
        _ => Vec::new(),
    }
}

/// The fields of a dynamic object (`A0Object`: its shape, capacity and slots), in the order they
/// were added: each shape adds a name (`A0Shape`: parent, name, count).
fn fields(o: u64) -> Vec<(String, u64, View)> {
    let slots = word(o + 16);
    let mut out = Vec::new();
    let mut shape = word(o);
    while shape != 0 && word(shape) != 0 {
        let index = word(shape + 16) - 1;
        let name = string(word(shape + 8));
        out.push((
            name.trim_matches('"').to_string(),
            word(slots + 8 * index),
            View::Any,
        ));
        shape = word(shape);
    }
    out.reverse();
    out.truncate(SHOWN);
    out
}

fn elements(a: u64, elem: View) -> Vec<(String, u64, View)> {
    let len = word(a) as usize;
    let data = word(a + 16);
    // Elements of 1, 2 or 4 bytes (`A0_ELEM_BYTE`, `A0_ELEM_16`, `A0_ELEM_32`) or words.
    let kind = word(a + 24);
    let size: u64 = match kind {
        ELEM_BYTE => 1,
        4 => 2,
        5 => 4,
        // Inline structs (`A0_ELEM_BYTES` and their size): each element is its address.
        k if k >= 16 => k - 16,
        _ => 8,
    };
    (0..len.min(SHOWN))
        .map(|i| {
            let at = data + size * i as u64;
            let w = if kind >= 16 {
                at
            } else if size < 8 {
                let mut le = [0u8; 8];
                le[..size as usize].copy_from_slice(&self::bytes(at, size as usize));
                u64::from_le_bytes(le)
            } else {
                word(at)
            };
            (format!("[{i}]"), w, elem)
        })
        .collect()
}
