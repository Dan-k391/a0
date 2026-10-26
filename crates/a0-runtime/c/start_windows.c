/* The entry point of Windows executables A0 links without Microsoft's toolchain, from a target
 * pack (targets/build.py, D79): what a C library's startup code does, for a program that needs only
 * its arguments, thread-local variables, exit handlers and the Universal C Runtime, which comes
 * with Windows. Not part of the runtime library the build links natively (build.rs does not compile
 * it). */

#include <stdint.h>

extern int main(int argc, char **argv);

/* From the Universal C Runtime (ucrtbase.dll), declared here so that no C library's headers are
 * needed. */
int _configure_narrow_argv(int mode);
int _initialize_narrow_environment(void);
int *__p___argc(void);
char ***__p___argv(void);
int _crt_atexit(void (*handler)(void));
__declspec(noreturn) void exit(int code);

#define A0_ARGV_UNEXPANDED 2 /* _crt_argv_unexpanded_arguments */

void mainCRTStartup(void) {
    _configure_narrow_argv(A0_ARGV_UNEXPANDED);
    _initialize_narrow_environment();
    exit(main(*__p___argc(), *__p___argv()));
}

/* Exit handlers run by the C runtime's exit (the collector's statistics use one). */
int atexit(void (*handler)(void)) { return _crt_atexit(handler); }

/* Thread-local variables (the runtime's per-thread state): the TLS directory the linker points the
 * executable's header at, as a C runtime's tlssup.c defines it. The variables go in `.tls$*`
 * sections, between `_tls_start` and `_tls_end`. */
typedef void(__stdcall *A0TlsCallback)(void *, unsigned long, void *);

uint32_t _tls_index = 0;
__attribute__((section(".tls"))) char _tls_start = 0;
__attribute__((section(".tls$ZZZ"))) char _tls_end = 0;
__attribute__((section(".CRT$XLA"))) A0TlsCallback __xl_a = 0;
__attribute__((section(".CRT$XLZ"))) A0TlsCallback __xl_z = 0;

typedef struct {
    uint64_t start, end, index, callbacks;
    uint32_t zero_fill, characteristics;
} A0TlsDirectory;

const A0TlsDirectory _tls_used = {
    (uint64_t)(uintptr_t)&_tls_start,
    (uint64_t)(uintptr_t)&_tls_end,
    (uint64_t)(uintptr_t)&_tls_index,
    (uint64_t)(uintptr_t)(&__xl_a + 1),
    0,
    0,
};
