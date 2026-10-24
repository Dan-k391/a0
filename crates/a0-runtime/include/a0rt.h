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

/* Prints "a0: trap: <reason>" to stderr and exits with A0_PANIC_EXIT_CODE. */
A0_NORETURN void a0_rt_trap(int32_t code);
/* Prints "a0: panic: <message>" to stderr and exits with A0_PANIC_EXIT_CODE. */
A0_NORETURN void a0_panic(const char *message);
A0_NORETURN void a0_panic_bytes(const uint8_t *message, int64_t len);

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

/* ---- FFI ---------------------------------------------------------------------------------- */

enum { A0_FFI_VOID = 0, A0_FFI_INT = 1, A0_FFI_F64 = 2 };

/* Calls `fn` with `nargs` (at most 8) arguments. Argument i is a double (its bits in args[i]) when
 * bit i of `float_mask` is set, otherwise an integer or pointer passed in a 64-bit slot. The result,
 * if any, is written to `*ret`. Returns 0, or -1 if the shape is not supported. */
int32_t a0_ffi_call(void *fn, int32_t nargs, const uint64_t *args, uint32_t float_mask,
                    int32_t ret_kind, uint64_t *ret);

#ifdef __cplusplus
}
#endif

#endif
