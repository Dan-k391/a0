/* The runtime's symbol table, used by the interpreter to resolve `extern func` declarations. */
#include "a0rt.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define SYM(f) {#f, (void *)&f}

const A0Symbol a0_rt_symbols[] = {
    /* A0 runtime */
    SYM(a0_rt_init),
    SYM(a0_rt_exit),
    SYM(a0_rt_flush),
    SYM(a0_rt_argc),
    SYM(a0_rt_argv),
    SYM(a0_rt_trap),
    SYM(a0_panic),
    SYM(a0_panic_bytes),
    SYM(a0_print_i64),
    SYM(a0_print_f64),
    SYM(a0_print_bytes),
    SYM(a0_print_cstr),
    SYM(a0_print_newline),
    SYM(a0_putchar),
    SYM(a0_eprint_bytes),
    SYM(a0_eprint_cstr),
    SYM(a0_format_f64),
    SYM(a0_alloc),
    SYM(a0_realloc),
    SYM(a0_free),
    SYM(a0_clock),
    /* C math library */
    SYM(sin),
    SYM(cos),
    SYM(tan),
    SYM(asin),
    SYM(acos),
    SYM(atan),
    SYM(atan2),
    SYM(sinh),
    SYM(cosh),
    SYM(tanh),
    SYM(exp),
    SYM(log),
    SYM(log10),
    SYM(log2),
    SYM(pow),
    SYM(sqrt),
    SYM(cbrt),
    SYM(floor),
    SYM(ceil),
    SYM(round),
    SYM(trunc),
    SYM(fabs),
    SYM(fmod),
    SYM(hypot),
    /* C standard library */
    SYM(putchar),
    SYM(getchar),
    SYM(malloc),
    SYM(calloc),
    SYM(realloc),
    SYM(free),
    SYM(memcpy),
    SYM(memmove),
    SYM(memset),
    SYM(memcmp),
    SYM(strlen),
    SYM(abort),
    SYM(exit),
    {NULL, NULL},
};
