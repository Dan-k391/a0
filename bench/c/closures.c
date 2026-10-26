/* Closures: a higher-order function (fold) called 100,000 times over 1,000 numbers with a function
 * and the variable it captures, as C writes closures. */
#include <stdio.h>

typedef struct {
    long scale;
} Env;

static long step(void *env, long acc, long x) { return acc + x * ((Env *)env)->scale; }

static long fold(const long *xs, long n, long init, long (*f)(void *, long, long), void *env) {
    long acc = init;
    for (long i = 0; i < n; i++) acc = f(env, acc, xs[i]);
    return acc;
}

int main(void) {
    static long xs[1000];
    for (long i = 0; i < 1000; i++) xs[i] = i;
    long total = 0;
    for (long k = 0; k < 100000; k++) {
        Env env = {k % 7};
        total += fold(xs, 1000, k, step, &env);
    }
    printf("%ld\n", total);
    return 0;
}
