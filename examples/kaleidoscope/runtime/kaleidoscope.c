/* Kaleidoscope's runtime library: the helpers the LLVM tutorial's programs call through `extern`.
 * Unlike the tutorial, output goes to stdout rather than stderr. */
#include "a0rt.h"

#include <stdio.h>

/* putchard - putchar that takes a double and returns 0. */
double putchard(double x) {
    putchar((char)x);
    return 0;
}

/* printd - printf that takes a double, prints it as "%f\n", and returns 0. */
double printd(double x) {
    printf("%f\n", x);
    return 0;
}

const A0Symbol kaleidoscope_symbols[] = {
    {"putchard", (void *)&putchard},
    {"printd", (void *)&printd},
    {NULL, NULL},
};
