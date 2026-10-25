/* Arrays: the primes below 2,000,000 by the sieve of Eratosthenes, ten times. */
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>

static long count_primes(long n) {
    bool *composite = calloc((size_t)n + 1, sizeof(bool));
    long count = 0;
    for (long p = 2; p <= n; p++) {
        if (!composite[p]) {
            count++;
            for (long m = p * p; m <= n; m += p) composite[m] = true;
        }
    }
    free(composite);
    return count;
}

int main(void) {
    long total = 0;
    for (int round = 0; round < 10; round++) total += count_primes(2000000);
    printf("%ld\n", total);
    return 0;
}
