/* Core of the A0 runtime: lifecycle, traps, output, memory and time. */
#include "a0rt.h"

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

void a0_rt_exit(int32_t code) {
    a0_rt_flush();
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

void a0_rt_trap(int32_t code) {
    fflush(stdout);
    fprintf(stderr, "a0: trap: %s\n", trap_reason(code));
    a0_rt_exit(A0_PANIC_EXIT_CODE);
}

void a0_panic(const char *message) {
    fflush(stdout);
    fprintf(stderr, "a0: panic: %s\n", message);
    a0_rt_exit(A0_PANIC_EXIT_CODE);
}

void a0_panic_bytes(const uint8_t *message, int64_t len) {
    fflush(stdout);
    fputs("a0: panic: ", stderr);
    fwrite(message, 1, (size_t)len, stderr);
    fputc('\n', stderr);
    a0_rt_exit(A0_PANIC_EXIT_CODE);
}

/* ---- Output ------------------------------------------------------------------------------- */

void a0_print_i64(int64_t value) { printf("%lld", (long long)value); }

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
    /* Find the fewest significant digits that read back exactly. */
    char sci[40];
    int digits;
    for (digits = 1; digits <= 17; digits++) {
        snprintf(sci, sizeof sci, "%.*e", digits - 1, value);
        if (strtod(sci, NULL) == value) break;
    }
    if (digits > 17) digits = 17;
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
    fwrite(buf, 1, (size_t)n, stdout);
}

void a0_print_bytes(const uint8_t *bytes, int64_t len) {
    if (len > 0) fwrite(bytes, 1, (size_t)len, stdout);
}

void a0_print_cstr(const char *s) { fputs(s, stdout); }

void a0_print_newline(void) { putchar('\n'); }

int32_t a0_putchar(int32_t c) { return putchar(c); }

void a0_eprint_bytes(const uint8_t *bytes, int64_t len) {
    fflush(stdout);
    if (len > 0) fwrite(bytes, 1, (size_t)len, stderr);
}

void a0_eprint_cstr(const char *s) {
    fflush(stdout);
    fputs(s, stderr);
}

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

double a0_clock(void) { return (double)clock() / (double)CLOCKS_PER_SEC; }
