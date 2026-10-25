/* Core of the A0 runtime: lifecycle, traps, output, memory and time. */
#include "internal.h"

#include <float.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif

static int32_t g_argc;
static char **g_argv;

void a0_rt_init(int32_t argc, char **argv) {
    g_argc = argc;
    g_argv = argv;
#ifdef _WIN32
    /* Write "\n", not "\r\n": output must be byte-identical on every platform. */
    _setmode(_fileno(stdout), _O_BINARY);
    _setmode(_fileno(stderr), _O_BINARY);
#endif
}

void a0_rt_flush(void) {
    fflush(stdout);
    fflush(stderr);
}

/* A debugger's hooks (a0rt.h, "Debuggers"); NULL when the program runs on its own. */
static A0WriteFn g_write;
static A0FailFn g_fail;
static A0ExitFn g_exit;

void a0_rt_set_output(A0WriteFn write) { g_write = write; }
void a0_rt_set_fail_hook(A0FailFn fail) { g_fail = fail; }
void a0_rt_set_exit_hook(A0ExitFn exit) { g_exit = exit; }
int32_t a0_rt_has_debugger(void) { return g_write != NULL; }

void a0_rt_write(int32_t stream, const uint8_t *bytes, size_t len) {
    if (g_write != NULL) {
        g_write(stream, bytes, len);
        return;
    }
    if (len == 0) return;
    if (stream == 2) {
        /* What was printed before an error comes before it. */
        fflush(stdout);
        fwrite(bytes, 1, len, stderr);
    } else {
        fwrite(bytes, 1, len, stdout);
    }
}

/* For the failure path (trace.c): the debugger first, while the failing call is active. */
void a0_rt_tell_failure(const uint8_t *message, size_t len) {
    if (g_fail != NULL) g_fail(message, len);
}

void a0_rt_exit(int32_t code) {
    a0_rt_flush();
    if (g_exit != NULL) g_exit(code);
    exit(code);
}

int32_t a0_rt_argc(void) { return g_argc; }

const char *a0_rt_argv(int32_t i) {
    if (i < 0 || i >= g_argc || g_argv == NULL) return NULL;
    return g_argv[i];
}

static const char *trap_reason(int32_t code) {
    switch (code) {
    case A0_TRAP_UNREACHABLE: return "unreachable code was reached";
    case A0_TRAP_DIVIDE_BY_ZERO: return "integer division by zero";
    case A0_TRAP_INTEGER_OVERFLOW: return "integer overflow";
    case A0_TRAP_STACK_OVERFLOW: return "stack overflow";
    default: return "unknown trap";
    }
}

void a0_rt_trap(int32_t code, uint64_t loc, void *frame) {
    /* A stack overflow's trace would be the same call thousands of times, and how deep the stack
     * got differs between the interpreter and native code. */
    if (code == A0_TRAP_STACK_OVERFLOW) a0_rt_fail_frames(trap_reason(code), NULL, 0);
    a0_rt_fail(trap_reason(code), loc, frame);
}

void a0_panic(const char *message) { a0_rt_fail_frames(message, NULL, 0); }

void a0_panic_bytes(const uint8_t *message, int64_t len, uint64_t loc, void *frame) {
    a0_rt_fail_bytes(message, len, loc, frame);
}

/* ---- Output ------------------------------------------------------------------------------- */

void a0_print_i64(int64_t value) {
    char buf[24];
    int32_t n = a0_format_i64(value, buf);
    a0_rt_write(1, (const uint8_t *)buf, (size_t)n);
}

int32_t a0_format_i64(int64_t value, char *buf) {
    char digits[24];
    int n = 0;
    /* Through unsigned, so that the most negative value has a magnitude. */
    uint64_t v = value < 0 ? 0 - (uint64_t)value : (uint64_t)value;
    do {
        digits[n++] = (char)('0' + v % 10);
        v /= 10;
    } while (v != 0);
    int32_t len = 0;
    if (value < 0) buf[len++] = '-';
    while (n > 0) buf[len++] = digits[--n];
    buf[len] = '\0';
    return len;
}

/* Whether `digits` significant digits of `value` read back as `value`; leaves them in `sci`. */
static int round_trips(double value, int digits, char *sci, size_t cap) {
    snprintf(sci, cap, "%.*e", digits - 1, value);
    return strtod(sci, NULL) == value;
}

int32_t a0_format_f64(double value, char *buf) {
    if (isnan(value)) {
        strcpy(buf, "nan");
        return 3;
    }
    if (isinf(value)) {
        strcpy(buf, value > 0 ? "inf" : "-inf");
        return value > 0 ? 3 : 4;
    }
    if (value == 0) {
        strcpy(buf, signbit(value) ? "-0" : "0");
        return signbit(value) ? 2 : 1;
    }
    /* Integers below 1e17 print as themselves (what the search below finds for them, faster). */
    if (value == floor(value) && fabs(value) < 1e17) return a0_format_i64((int64_t)value, buf);
    /* The fewest significant digits that read back exactly: one at a time for short numbers, then
     * by bisection, since more digits never read back worse. 17 always do. */
    char sci[40];
    int digits = 0;
    for (int d = 1; d <= 6 && digits == 0; d++) {
        if (round_trips(value, d, sci, sizeof sci)) digits = d;
    }
    if (digits == 0) {
        int lo = 7, hi = 17;
        while (lo < hi) {
            int mid = (lo + hi) / 2;
            if (round_trips(value, mid, sci, sizeof sci)) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        digits = lo;
    }
    snprintf(sci, sizeof sci, "%.*e", digits - 1, value);
    int exponent = atoi(strchr(sci, 'e') + 1);
    /* Plain notation for "reasonable" magnitudes, scientific otherwise. */
    if (exponent >= -5 && exponent < 17) {
        int decimals = digits - 1 - exponent;
        if (decimals < 0) decimals = 0;
        return snprintf(buf, 32, "%.*f", decimals, value);
    }
    strcpy(buf, sci);
    return (int32_t)strlen(buf);
}

void a0_print_f64(double value) {
    char buf[32];
    int32_t n = a0_format_f64(value, buf);
    a0_rt_write(1, (const uint8_t *)buf, (size_t)n);
}

void a0_print_bytes(const uint8_t *bytes, int64_t len) {
    if (len > 0) a0_rt_write(1, bytes, (size_t)len);
}

void a0_print_cstr(const char *s) { a0_rt_write(1, (const uint8_t *)s, strlen(s)); }

void a0_print_newline(void) { a0_rt_write(1, (const uint8_t *)"\n", 1); }

int32_t a0_putchar(int32_t c) {
    uint8_t b = (uint8_t)c;
    a0_rt_write(1, &b, 1);
    return c;
}

void a0_eprint_bytes(const uint8_t *bytes, int64_t len) {
    if (len > 0) a0_rt_write(2, bytes, (size_t)len);
}

void a0_eprint_cstr(const char *s) { a0_rt_write(2, (const uint8_t *)s, strlen(s)); }

/* ---- Memory ------------------------------------------------------------------------------- */

void *a0_alloc(int64_t size) {
    if (size < 0) a0_panic("negative allocation size");
    void *p = calloc(1, size == 0 ? 1 : (size_t)size);
    if (p == NULL) a0_panic("out of memory");
    return p;
}

void *a0_realloc(void *ptr, int64_t size) {
    if (size < 0) a0_panic("negative allocation size");
    void *p = realloc(ptr, size == 0 ? 1 : (size_t)size);
    if (p == NULL) a0_panic("out of memory");
    return p;
}

void a0_free(void *ptr) { free(ptr); }

/* ---- Time --------------------------------------------------------------------------------- */

#ifdef __wasi__
/* WASI has no processor-time clock; the time since the program started stands in for it. */
double a0_clock(void) {
    static double start = -1;
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    double now = (double)ts.tv_sec + (double)ts.tv_nsec * 1e-9;
    if (start < 0) start = now;
    return now - start;
}
#else
double a0_clock(void) { return (double)clock() / (double)CLOCKS_PER_SEC; }
#endif
