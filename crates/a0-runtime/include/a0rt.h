/* The A0 runtime: the C support library linked into every A0 program.
 *
 * Both backends use it: the interpreter calls these functions directly and the native backend links
 * the same static library into executables, so the two behave identically.
 */
#ifndef A0RT_H
#define A0RT_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#if defined(_MSC_VER)
#define A0_NORETURN __declspec(noreturn)
#else
#define A0_NORETURN __attribute__((noreturn))
#endif

/* A pointer field that generated code reads or writes as a 64-bit word (in A0Type, A0Array and
 * the collector's header), so that the layout is the same for every pointer size: on 32-bit
 * targets (wasm32, little-endian) the pointer is the low half of a word whose high half stays
 * zero, as these structures are zeroed when they are allocated. */
#if UINTPTR_MAX > 0xffffffffu
#define A0_WORD_PTR(type, name) type name
#else
#define A0_WORD_PTR(type, name)     union {                             type name;                      uint64_t name##_word;       }
#endif

/* ---- Lifecycle ---------------------------------------------------------------------------- */

/* Called once before the program's main function. */
void a0_rt_init(int32_t argc, char **argv);
/* Flushes output and exits the process with `code`. */
A0_NORETURN void a0_rt_exit(int32_t code);
/* Flushes standard output and standard error. */
void a0_rt_flush(void);
int32_t a0_rt_argc(void);
/* The `i`th command-line argument, or NULL. */
const char *a0_rt_argv(int32_t i);

/* ---- Traps and panics --------------------------------------------------------------------- */

enum {
    A0_TRAP_UNREACHABLE = 1,
    A0_TRAP_DIVIDE_BY_ZERO = 2,
    A0_TRAP_INTEGER_OVERFLOW = 3,
    A0_TRAP_STACK_OVERFLOW = 4,
};

/* Exit status used by traps and panics. */
#define A0_PANIC_EXIT_CODE 101

/* A failure prints "panic: <message>" to stderr, then the stack of source-level functions that led
 * to it, and exits with A0_PANIC_EXIT_CODE:
 *
 *     panic: index out of bounds: the length is 2 but the index is 5
 *         at get (prog.mn:4:9)
 *         at main (prog.mn:9:3)
 *
 * Runtime functions that can fail take two arguments more than the IR declares, which generated
 * code passes: `loc`, the location of the failing operation in the program's location table (0
 * when unknown), and `frame`, the frame pointer of the generated function that called (NULL from
 * the interpreter, which reports its frames through a0_rt_set_frames). */

/* One entry of the location table: a function as traces name it and a place in the source (`func`
 * and `file` are offsets of NUL-terminated strings; `file` names an empty one when the place is not
 * known), and for inlined code the id of the location of the call it was inlined at (0 for none). */
typedef struct A0Location {
    uint32_t func, file, line, col, parent;
} A0Location;
/* `count` A0Locations follow the table, with ids 1 to count; the strings start `strings` bytes
 * after its start. */
typedef struct A0LocationTable {
    uint32_t count, strings;
} A0LocationTable;
/* Native code: each generated function, and the code ranges of the calls in it with their
 * locations (offsets from the function's start), so that return addresses find their calls. */
typedef struct A0CodeFunction {
    const uint8_t *start;
    uint32_t size, first, count, unused;
} A0CodeFunction;
typedef struct A0CodeRange {
    uint32_t start, end, location;
} A0CodeRange;
typedef struct A0CodeTable {
    uint64_t count;
    const A0CodeFunction *functions;
    const A0CodeRange *ranges;
} A0CodeTable;

/* Gives the runtime the program's tables (`code` is NULL in the interpreter). */
void a0_rt_set_debug(const A0LocationTable *locations, const A0CodeTable *code);
/* The interpreter's frames: fills `out` with the location of the active call in each frame,
 * innermost first, and returns how many. */
typedef size_t (*A0FramesFn)(uint32_t *out, size_t max);
void a0_rt_set_frames(A0FramesFn frames);
/* Fails with `message` at `loc`. */
A0_NORETURN void a0_rt_fail(const char *message, uint64_t loc, void *frame);
A0_NORETURN void a0_rt_fail_bytes(const uint8_t *message, int64_t len, uint64_t loc, void *frame);
/* Fails with `message` and the frames `ids` (the interpreter's own failures). */
A0_NORETURN void a0_rt_fail_frames(const char *message, const uint32_t *ids, size_t count);
/* Writes the trace of the frames `ids` into `buf` (NUL-terminated, cut to `cap`); returns its
 * full length. */
size_t a0_rt_trace_text(const uint32_t *ids, size_t count, char *buf, size_t cap);
/* "index out of bounds: the length is <len> but the index is <index>". */
A0_NORETURN void a0_rt_out_of_bounds(int64_t len, int64_t index, uint64_t loc, void *frame);
/* Exceptions (crates/a0-hir/src/lower.rs): generated code keeps the exception in flight itself and
 * calls a0_rt_throw where one is thrown, which remembers the stack there; a0_rt_uncaught reports an
 * exception that reached `main` (`panic: uncaught exception: <value>` and that stack). */
void a0_rt_throw(uint64_t loc, void *frame);
A0_NORETURN void a0_rt_uncaught(uint64_t value);

A0_NORETURN void a0_rt_trap(int32_t code, uint64_t loc, void *frame);
/* A failure of the runtime itself (out of memory), with no location. */
A0_NORETURN void a0_panic(const char *message);
A0_NORETURN void a0_panic_bytes(const uint8_t *message, int64_t len, uint64_t loc, void *frame);

/* ---- Output ------------------------------------------------------------------------------- */

void a0_print_i64(int64_t value);
/* Prints the shortest decimal that reads back as the same double. Magnitudes from 1e-5 up to 1e17
 * use plain notation without a trailing ".0" ("100", "0.1", "-0"); others use scientific notation
 * ("1e+300"). Non-finite values print as "nan", "inf" and "-inf". */
void a0_print_f64(double value);
void a0_print_bytes(const uint8_t *bytes, int64_t len);
void a0_print_cstr(const char *s);
void a0_print_newline(void);
int32_t a0_putchar(int32_t c);
void a0_eprint_bytes(const uint8_t *bytes, int64_t len);
void a0_eprint_cstr(const char *s);
/* Writes the shortest round-trip form of `value` into `buf` (at least 32 bytes); returns length. */
/* `value` in decimal into `buf` (at least 24 bytes), NUL-terminated; returns the length. */
int32_t a0_format_i64(int64_t value, char *buf);
int32_t a0_format_f64(double value, char *buf);

/* ---- Memory ------------------------------------------------------------------------------- */

/* Zero-initialized allocation; panics when out of memory. */
void *a0_alloc(int64_t size);
void *a0_realloc(void *ptr, int64_t size);
void a0_free(void *ptr);

/* ---- Time --------------------------------------------------------------------------------- */

/* Seconds of processor time used by the program, as a double. */
double a0_clock(void);

/* ---- Symbol tables ------------------------------------------------------------------------ */

/* A NULL-terminated table mapping C symbol names to addresses. Runtime libraries export one so that
 * the interpreter can resolve `extern func` declarations without dynamic loading. */
typedef struct A0Symbol {
    const char *name;
    void *address;
} A0Symbol;

/* The runtime's own functions plus common C library functions (math, memory, character I/O). */
extern const A0Symbol a0_rt_symbols[];
/* The address of a C function exported by the executable or a library loaded into the process, or
 * NULL. For the interpreter, after the tables. */
void *a0_rt_find_symbol(const char *name);

/* ---- FFI ---------------------------------------------------------------------------------- */

enum { A0_FFI_VOID = 0, A0_FFI_INT = 1, A0_FFI_F64 = 2 };

/* Calls `fn` with `nargs` (at most 8) arguments. Argument i is a double (its bits in args[i]) when
 * bit i of `float_mask` is set, otherwise an integer or pointer passed in a 64-bit slot. The result,
 * if any, is written to `*ret`. Returns 0, or -1 if the shape is not supported. */
int32_t a0_ffi_call(void *fn, int32_t nargs, const uint64_t *args, uint32_t float_mask,
                    int32_t ret_kind, uint64_t *ret);

/* ---- Garbage collector -------------------------------------------------------------------- *
 *
 * A precise mark-sweep collector. Objects are allocated with a type descriptor (A0Type) and live in
 * the GC heap; a pointer to an object points at its payload. The collector finds roots in:
 *   - the shadow stack: frames of 64-bit words pushed by generated code (a0_gc_frame_push/pop), each
 *     word in the `any` encoding below;
 *   - registered roots (a0_gc_add_root), such as global variables;
 *   - its own temporaries while a runtime function allocates more than once.
 * Only object allocation can start a collection, so a runtime function that allocates once needs no
 * rooting as long as its arguments are rooted by the caller. A0_GC_STRESS=1 in the environment makes
 * every allocation collect. */

/* How one 64-bit word is traced. */
enum { A0_WORD_RAW = 0, A0_WORD_REF = 1, A0_WORD_ANY = 2 };

/* Kinds of objects. FIXED objects (structs, enum variants, closures, boxes) are a fixed number of
 * words described by the type's layout; the others have built-in descriptors. */
enum { A0_KIND_FIXED = 0, A0_KIND_STRING = 1, A0_KIND_ARRAY = 2, A0_KIND_MAP = 3, A0_KIND_INT = 4 };

/* Generated code reads three things directly (crates/a0-hir/src/lower.rs): an object's type is the
 * pointer 24 bytes before the object (in the collector's header), a type's family is the field 24
 * bytes into its descriptor, and its interface table the field 32 bytes in. Keep them in sync. */
typedef struct A0Type {
    A0_WORD_PTR(char *, name);
    int32_t kind;
    int32_t word_count;
    A0_WORD_PTR(uint8_t *, layout);
    /* The enum a variant belongs to; a type's own descriptor otherwise. */
    A0_WORD_PTR(const struct A0Type *, family);
    /* For each interface of the program, the table of this type's methods for it (64-bit words
     * holding pointers to functions that take the object first), or NULL; NULL if it implements
     * none. Set by generated code at startup. */
    A0_WORD_PTR(void *, itable);
} A0Type;

/* Registers a FIXED type: `word_count` payload words traced as `layout[i]` says. `family` is the
 * descriptor that `a0_any_is_type` compares against (NULL means the type itself). The descriptor
 * lives for the rest of the program. */
A0Type *a0_type_new(const uint8_t *name, int64_t name_len, int64_t word_count, const uint8_t *layout,
                    A0Type *family);
/* Allocates a zeroed FIXED object. */
void *a0_gc_new(A0Type *type);
const A0Type *a0_gc_type_of(const void *object);

/* Pushes a frame of `slots` zeroed words onto the shadow stack and returns it. Before popping it, the
 * caller sets every word back to zero; that keeps the stack above its top zero, so pushing is cheap.
 * Under A0_GC_STRESS, popping a frame that was not cleared panics. */
uint64_t *a0_gc_frame_push(int64_t slots);
void a0_gc_frame_pop(int64_t slots);
/* Registers the word at `address` as a root holding a pointer (A0_WORD_REF) or an `any`
 * (A0_WORD_ANY). */
void a0_gc_add_root(void *address, int64_t word_kind);
void a0_gc_collect(void);
/* Number of collections so far (for tests and statistics). */
int64_t a0_gc_collections(void);
/* Number of live objects after the last collection. */
int64_t a0_gc_live_objects(void);

/* ---- Dynamic values (`any`) ----------------------------------------------------------------- *
 * NaN-boxed 64-bit words: doubles as themselves (NaNs canonicalized), unit and booleans as special
 * NaN patterns, references as A0_ANY_REF_TAG | pointer (a null pointer boxes to unit), integers that
 * fit in 48 bits as A0_ANY_INT_TAG | their low 48 bits, and other integers as boxed heap objects.
 * Every integer has exactly one encoding, so equal inline integers are equal words.
 *
 * References keep only the low 48 bits of the pointer. The collector checks that every heap address
 * it hands out fits (a0_gc_alloc panics otherwise), which holds for user space on every supported
 * target. */
#define A0_ANY_QNAN 0x7ffc000000000000ull
#define A0_ANY_REF_TAG 0xfffc000000000000ull
#define A0_ANY_INT_TAG 0x7ffe000000000000ull
#define A0_ANY_UNIT (A0_ANY_QNAN | 1)
#define A0_ANY_FALSE (A0_ANY_QNAN | 2)
#define A0_ANY_TRUE (A0_ANY_QNAN | 3)

uint64_t a0_any_from_float(double value);
uint64_t a0_any_from_int(int64_t value); /* allocates if the value does not fit in 48 bits */
uint64_t a0_any_from_ref(void *object);
int64_t a0_any_is_int(uint64_t value);
int64_t a0_any_is_string(uint64_t value);
/* Whether `value` holds an object whose type (or type family) is `type`. */
int64_t a0_any_is_type(uint64_t value, const A0Type *type);
int64_t a0_any_to_int(uint64_t value, uint64_t loc, void *frame);
/* Equality of two dynamic values: floats by IEEE, ints by value, strings by content, others by
 * identity. */
int64_t a0_any_eq(uint64_t a, uint64_t b);
/* Fails with "expected <expected>, found <type of value>". */
A0_NORETURN void a0_any_cast_failed(uint64_t value, const uint8_t *expected, int64_t len,
                                    uint64_t loc, void *frame);

/* ---- Strings ----------------------------------------------------------------------------------- */

typedef struct A0String {
    int64_t len;
    uint64_t hash; /* 0 until computed */
    uint8_t bytes[];
} A0String;

A0String *a0_string_new(const uint8_t *bytes, int64_t len);
A0String *a0_string_concat(A0String *a, A0String *b);
int64_t a0_string_len(A0String *s);
int64_t a0_string_eq(A0String *a, A0String *b);
/* -1, 0 or 1, comparing bytes. */
int64_t a0_string_compare(A0String *a, A0String *b);
A0String *a0_string_substr(A0String *s, int64_t start, int64_t count, uint64_t loc, void *frame);
int64_t a0_string_byte_at(A0String *s, int64_t index, uint64_t loc, void *frame);
A0String *a0_string_from_int(int64_t value);
A0String *a0_string_from_float(double value);
A0String *a0_string_from_bool(int64_t value);
void a0_string_print(A0String *s);
void a0_string_eprint(A0String *s);
A0_NORETURN void a0_string_panic(A0String *message, uint64_t loc, void *frame);
/* A copy of the NUL-terminated string a C function returned. NULL is null, or fails if `required`. */
A0String *a0_string_from_c(const char *s, int64_t required, uint64_t loc, void *frame);

/* ---- Arrays ------------------------------------------------------------------------------------ *
 * Elements are 64-bit words traced as `elem_kind` (A0_WORD_*). */

typedef struct A0Array {
    int64_t len;
    int64_t cap;
    A0_WORD_PTR(uint64_t *, data); /* read by generated code (element access) */
    int64_t elem_kind;
} A0Array;

A0Array *a0_array_new(int64_t elem_kind, int64_t capacity);
int64_t a0_array_len(A0Array *a);
uint64_t a0_array_get(A0Array *a, int64_t index, uint64_t loc, void *frame);
void a0_array_set(A0Array *a, int64_t index, uint64_t value, uint64_t loc, void *frame);
void a0_array_push(A0Array *a, uint64_t value);
uint64_t a0_array_pop(A0Array *a, uint64_t loc, void *frame);

/* ---- Maps -------------------------------------------------------------------------------------- *
 * Hash maps that remember insertion order. Keys are raw words (ints, booleans) or strings compared by
 * content; values are words traced as `value_kind`. */

enum { A0_KEY_RAW = 0, A0_KEY_STRING = 1 };

typedef struct A0Map A0Map;

A0Map *a0_map_new(int64_t key_kind, int64_t value_kind);
int64_t a0_map_len(A0Map *m);
int64_t a0_map_has(A0Map *m, uint64_t key);
uint64_t a0_map_get(A0Map *m, uint64_t key, uint64_t loc, void *frame);
void a0_map_set(A0Map *m, uint64_t key, uint64_t value);
int64_t a0_map_remove(A0Map *m, uint64_t key);
/* The keys in insertion order, as an array with elements of the key's word kind. */
A0Array *a0_map_keys(A0Map *m);

/* A null reference where the runtime needed an object. */
A0_NORETURN void a0_panic_null(void);

/* ---- The standard library's C side (std.c), declared by crates/a0-hir/std ---------------------- *
 * Functions that can fail return NULL or 0 and leave a message for a0_std_error; the parsers also
 * set the flag a0_std_failed reads. Booleans are 0 or 1. */
A0String *a0_std_error(void);
int64_t a0_std_failed(void);
A0String *a0_std_read_line(void);
A0String *a0_std_read_all(void);
A0String *a0_std_read_file(A0String *path);
int64_t a0_std_write_file(A0String *path, A0String *text, int64_t append);
int64_t a0_std_path_kind(A0String *path);
int64_t a0_std_remove(A0String *path);
A0Array *a0_std_list_dir(A0String *path);
A0Array *a0_std_args(void);
double a0_std_now(void);
double a0_std_monotonic(void);
void a0_std_sleep(double seconds);
A0String *a0_std_format_time(double seconds);
void a0_std_seed(int64_t seed);
double a0_std_random(void);
int64_t a0_std_random_int(int64_t low, int64_t high);
int64_t a0_std_find(A0String *s, A0String *part, int64_t from);
int64_t a0_std_find_last(A0String *s, A0String *part);
int64_t a0_std_starts_with(A0String *s, A0String *prefix);
int64_t a0_std_ends_with(A0String *s, A0String *suffix);
A0String *a0_std_replace(A0String *s, A0String *from, A0String *to);
A0Array *a0_std_split(A0String *s, A0String *separator);
A0Array *a0_std_lines(A0String *s);
A0String *a0_std_join(A0Array *parts, A0String *separator);
A0String *a0_std_trim(A0String *s, int64_t sides);
A0String *a0_std_change_case(A0String *s, int64_t upper);
A0String *a0_std_repeat(A0String *s, int64_t count);
int64_t a0_std_parse_int(A0String *s);
double a0_std_parse_float(A0String *s);
A0String *a0_std_fixed(double x, int64_t decimals);
A0String *a0_std_from_byte(int64_t byte);
A0Array *a0_std_bytes(A0String *s);
A0String *a0_std_from_bytes(A0Array *bytes);
A0String *a0_std_from_code_point(int64_t c);

#ifdef __cplusplus
}
#endif

#endif
