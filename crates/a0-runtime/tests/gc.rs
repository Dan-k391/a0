//! The collector and the object support in `a0rt`, called directly. The collector is process-global,
//! so everything runs in one test function, in order.

use std::ffi::c_void;

// Make sure the runtime library is linked.
use a0_runtime as _;

const WORD_RAW: u8 = 0;
const WORD_REF: u8 = 1;
const WORD_ANY: u8 = 2;
const KEY_RAW: i64 = 0;
const KEY_STRING: i64 = 1;
const ANY_UNIT: u64 = 0x7ffc_0000_0000_0001;
const ANY_TRUE: u64 = 0x7ffc_0000_0000_0003;

type Ptr = *mut c_void;

unsafe extern "C" {
    fn a0_type_new(name: *const u8, len: i64, words: i64, layout: *const u8, family: Ptr) -> Ptr;
    fn a0_gc_new(t: Ptr) -> Ptr;
    fn a0_gc_frame_push(slots: i64) -> *mut u64;
    fn a0_gc_frame_pop(slots: i64);
    fn a0_gc_add_root(address: *mut c_void, kind: i64);
    fn a0_gc_collect();
    fn a0_gc_live_objects() -> i64;

    fn a0_any_from_float(v: f64) -> u64;
    fn a0_any_from_int(v: i64) -> u64;
    fn a0_any_from_ref(p: Ptr) -> u64;
    fn a0_any_is_int(v: u64) -> i64;
    fn a0_any_is_string(v: u64) -> i64;
    fn a0_any_is_type(v: u64, t: Ptr) -> i64;
    fn a0_any_to_int(v: u64) -> i64;
    fn a0_any_eq(a: u64, b: u64) -> i64;

    fn a0_string_new(bytes: *const u8, len: i64) -> Ptr;
    fn a0_string_concat(a: Ptr, b: Ptr) -> Ptr;
    fn a0_string_len(s: Ptr) -> i64;
    fn a0_string_eq(a: Ptr, b: Ptr) -> i64;
    fn a0_string_compare(a: Ptr, b: Ptr) -> i64;
    fn a0_string_substr(s: Ptr, start: i64, count: i64) -> Ptr;
    fn a0_string_byte_at(s: Ptr, i: i64) -> i64;
    fn a0_string_from_int(v: i64) -> Ptr;
    fn a0_string_from_float(v: f64) -> Ptr;
    fn a0_string_from_bool(v: i64) -> Ptr;

    fn a0_array_new(kind: i64, cap: i64) -> Ptr;
    fn a0_array_len(a: Ptr) -> i64;
    fn a0_array_get(a: Ptr, i: i64) -> u64;
    fn a0_array_set(a: Ptr, i: i64, v: u64);
    fn a0_array_push(a: Ptr, v: u64);
    fn a0_array_pop(a: Ptr) -> u64;

    fn a0_map_new(key_kind: i64, value_kind: i64) -> Ptr;
    fn a0_map_len(m: Ptr) -> i64;
    fn a0_map_has(m: Ptr, k: u64) -> i64;
    fn a0_map_get(m: Ptr, k: u64) -> u64;
    fn a0_map_set(m: Ptr, k: u64, v: u64);
    fn a0_map_remove(m: Ptr, k: u64) -> i64;
    fn a0_map_keys(m: Ptr) -> Ptr;
}

fn s(text: &str) -> Ptr {
    unsafe { a0_string_new(text.as_ptr(), text.len() as i64) }
}

fn text(p: Ptr) -> String {
    // An A0String is { len, hash, bytes[] }.
    unsafe {
        let len = *(p as *const i64);
        let bytes = (p as *const u8).add(16);
        String::from_utf8(std::slice::from_raw_parts(bytes, len as usize).to_vec()).unwrap()
    }
}

fn any(p: Ptr) -> u64 {
    unsafe { a0_any_from_ref(p) }
}

fn live() -> i64 {
    unsafe {
        a0_gc_collect();
        a0_gc_live_objects()
    }
}

#[test]
fn runtime_objects_and_collection() {
    unsafe {
        strings();
        arrays();
        maps();
        dynamic_values();
        collection();
    }
}

unsafe fn strings() {
    unsafe {
        let hello = s("hello, ");
        let world = s("world");
        let both = a0_string_concat(hello, world);
        assert_eq!(text(both), "hello, world");
        assert_eq!(a0_string_len(both), 12);
        assert_eq!(a0_string_eq(both, s("hello, world")), 1);
        assert_eq!(a0_string_eq(both, hello), 0);
        assert_eq!(a0_string_compare(s("abc"), s("abd")), -1);
        assert_eq!(a0_string_compare(s("ab"), s("a")), 1);
        assert_eq!(a0_string_compare(s("x"), s("x")), 0);
        assert_eq!(text(a0_string_substr(both, 7, 5)), "world");
        assert_eq!(a0_string_byte_at(both, 0), b'h' as i64);
        assert_eq!(text(a0_string_from_int(-42)), "-42");
        assert_eq!(text(a0_string_from_float(2.5)), "2.5");
        assert_eq!(text(a0_string_from_float(3.0)), "3");
        assert_eq!(text(a0_string_from_bool(1)), "true");
    }
}

unsafe fn arrays() {
    unsafe {
        let a = a0_array_new(WORD_RAW as i64, 0);
        for i in 0..100 {
            a0_array_push(a, i);
        }
        assert_eq!(a0_array_len(a), 100);
        assert_eq!(a0_array_get(a, 42), 42);
        a0_array_set(a, 42, 7);
        assert_eq!(a0_array_get(a, 42), 7);
        assert_eq!(a0_array_pop(a), 99);
        assert_eq!(a0_array_len(a), 99);
    }
}

unsafe fn maps() {
    unsafe {
        let m = a0_map_new(KEY_STRING, WORD_RAW as i64);
        let names = ["one", "two", "three", "four", "five"];
        for (i, n) in names.iter().enumerate() {
            a0_map_set(m, s(n) as u64, i as u64 + 1);
        }
        // Lookups use content, not identity.
        assert_eq!(a0_map_get(m, s("three") as u64), 3);
        assert_eq!(a0_map_has(m, s("six") as u64), 0);
        a0_map_set(m, s("two") as u64, 22);
        assert_eq!(a0_map_get(m, s("two") as u64), 22);
        assert_eq!(a0_map_remove(m, s("one") as u64), 1);
        assert_eq!(a0_map_remove(m, s("one") as u64), 0);
        a0_map_set(m, s("one") as u64, 11);
        assert_eq!(a0_map_len(m), 5);
        // Keys come back in insertion order; a removed and re-added key goes last.
        let keys = a0_map_keys(m);
        let order: Vec<String> = (0..a0_array_len(keys))
            .map(|i| text(a0_array_get(keys, i) as Ptr))
            .collect();
        assert_eq!(order, vec!["two", "three", "four", "five", "one"]);

        // Integer keys, through several rebuilds and removals.
        let m = a0_map_new(KEY_RAW, WORD_RAW as i64);
        for i in 0..1000u64 {
            a0_map_set(m, i * 7, i);
        }
        for i in (0..1000u64).step_by(2) {
            assert_eq!(a0_map_remove(m, i * 7), 1);
        }
        assert_eq!(a0_map_len(m), 500);
        for i in 0..1000u64 {
            assert_eq!(a0_map_has(m, i * 7), (i % 2) as i64);
        }
        assert_eq!(a0_map_get(m, 7 * 999), 999);
    }
}

unsafe fn dynamic_values() {
    unsafe {
        let x = a0_any_from_float(1.5);
        assert_eq!(f64::from_bits(x), 1.5);
        let nan = a0_any_from_float(f64::NAN);
        assert_eq!(a0_any_eq(nan, nan), 0, "NaN is not equal to itself");
        assert_eq!(
            a0_any_eq(a0_any_from_float(0.0), a0_any_from_float(-0.0)),
            1
        );
        let i = a0_any_from_int(-7);
        assert_eq!(a0_any_is_int(i), 1);
        assert_eq!(a0_any_to_int(i), -7);
        assert_eq!(a0_any_eq(i, a0_any_from_int(-7)), 1);
        // 48-bit integers are inline, the rest boxed; the boundaries round-trip either way.
        for v in [
            (1i64 << 47) - 1,
            -(1 << 47),
            1 << 47,
            -(1 << 47) - 1,
            i64::MAX,
            i64::MIN,
            0,
        ] {
            let w = a0_any_from_int(v);
            assert_eq!((a0_any_is_int(w), a0_any_to_int(w)), (1, v));
            assert_eq!(a0_any_eq(w, a0_any_from_int(v)), 1);
            assert_eq!(a0_any_eq(w, a0_any_from_int(v.wrapping_add(1))), 0);
        }
        assert_eq!(
            a0_any_from_int(5) >> 48,
            0x7ffe,
            "small ints carry the int tag"
        );
        assert_eq!(
            a0_any_eq(i, a0_any_from_float(-7.0)),
            0,
            "ints and floats differ"
        );
        let a = any(s("text"));
        assert_eq!(a0_any_is_string(a), 1);
        assert_eq!(a0_any_eq(a, any(s("text"))), 1);
        assert_eq!(a0_any_from_ref(std::ptr::null_mut()), ANY_UNIT);
        assert_eq!(a0_any_eq(ANY_UNIT, ANY_UNIT), 1);
        assert_eq!(a0_any_eq(ANY_TRUE, ANY_UNIT), 0);

        let point = a0_type_new(
            b"Point".as_ptr(),
            5,
            2,
            [WORD_RAW, WORD_RAW].as_ptr(),
            std::ptr::null_mut(),
        );
        let p = a0_gc_new(point);
        assert_eq!(a0_any_is_type(any(p), point), 1);
        assert_eq!(a0_any_is_type(a, point), 0);
        // Variants of one enum share a family.
        let shape = a0_type_new(b"Shape".as_ptr(), 5, 0, [].as_ptr(), std::ptr::null_mut());
        let circle = a0_type_new(
            b"Circle".as_ptr(),
            6,
            2,
            [WORD_RAW, WORD_RAW].as_ptr(),
            shape,
        );
        assert_eq!(a0_any_is_type(any(a0_gc_new(circle)), shape), 1);
    }
}

unsafe fn collection() {
    unsafe {
        // Garbage from the earlier steps goes away; nothing is rooted yet.
        assert_eq!(live(), 0);

        // A struct with a reference field and an `any` field, kept alive by a frame slot.
        let node = a0_type_new(
            b"Node".as_ptr(),
            4,
            3,
            [WORD_REF, WORD_ANY, WORD_RAW].as_ptr(),
            std::ptr::null_mut(),
        );
        let frame = a0_gc_frame_push(2);
        let n = a0_gc_new(node);
        *frame = any(n);
        let fields = n as *mut u64;
        *fields = s("name") as u64;
        *fields.add(1) = any(s("payload"));
        *fields.add(2) = 12345;
        // A raw word that happens to look like nothing in particular is not traced.
        assert_eq!(live(), 3, "node, its string and its any-string");

        // An array of `any` and a map with string keys, reachable from the second slot.
        let arr = a0_array_new(WORD_ANY as i64, 0);
        *frame.add(1) = any(arr);
        a0_array_push(arr, any(s("in array")));
        a0_array_push(arr, a0_any_from_int(9));
        assert_eq!(live(), 3 + 2, "small ints are inline, not objects");
        a0_array_push(arr, a0_any_from_int(1 << 50));
        a0_array_push(arr, a0_any_from_float(2.0));
        assert_eq!(live(), 3 + 3, "array, a string and a boxed int");

        let map = a0_map_new(KEY_STRING, WORD_REF as i64);
        a0_array_push(arr, any(map));
        a0_map_set(map, s("key") as u64, s("value") as u64);
        assert_eq!(live(), 6 + 3, "map, key and value");
        a0_map_remove(map, s("key") as u64);
        assert_eq!(live(), 6 + 1, "removed entries are no longer traced");

        // Popping the frame drops everything; a registered root keeps its object.
        a0_gc_frame_pop(2);
        assert_eq!(live(), 0);
        static mut GLOBAL: u64 = 0;
        GLOBAL = s("global") as u64;
        a0_gc_add_root(&raw mut GLOBAL as *mut c_void, WORD_REF as i64);
        assert_eq!(live(), 1);
        assert_eq!(text(GLOBAL as Ptr), "global");

        // Lots of garbage with a little live data: the heap stays small.
        let frame = a0_gc_frame_push(1);
        let keep = a0_array_new(WORD_REF as i64, 0);
        *frame = any(keep);
        for i in 0..200_000 {
            let t = a0_string_from_int(i);
            if i % 1000 == 0 {
                a0_array_push(keep, t as u64);
            }
        }
        assert_eq!(live(), 1 + 1 + 200);
        assert_eq!(text(a0_array_get(keep, 199) as Ptr), "199000");
        a0_gc_frame_pop(1);
    }
}
