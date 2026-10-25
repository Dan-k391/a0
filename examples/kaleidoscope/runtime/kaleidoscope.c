/* Kaleidoscope's runtime library: the helpers the LLVM tutorial's programs call through `extern`.
 * Unlike the tutorial, output goes to stdout rather than stderr. It goes through a0_rt_write, as
 * the A0 runtime's own does, so that a debugger running the program can take it. */
#include "a0rt.h"

#include <stdio.h>

/* putchard - putchar that takes a double and returns 0. */
double putchard(double x) {
    uint8_t c = (uint8_t)(char)x;
    a0_rt_write(1, &c, 1);
    return 0;
}

/* printd - printf that takes a double, prints it as "%f\n", and returns 0. */
double printd(double x) {
    char text[512];
    int n = snprintf(text, sizeof text, "%f\n", x);
    if (n > (int)sizeof text - 1) n = (int)sizeof text - 1;
    if (n > 0) a0_rt_write(1, (const uint8_t *)text, (size_t)n);
    return 0;
}

const A0Symbol kaleidoscope_symbols[] = {
    {"putchard", (void *)&putchard},
    {"printd", (void *)&printd},
    {NULL, NULL},
};
