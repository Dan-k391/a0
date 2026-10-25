// Arrays: the primes below 2,000,000 by the sieve of Eratosthenes, ten times.
package main

import "fmt"

func countPrimes(n int) int {
	composite := make([]bool, n+1)
	count := 0
	for p := 2; p <= n; p++ {
		if !composite[p] {
			count++
			for m := p * p; m <= n; m += p {
				composite[m] = true
			}
		}
	}
	return count
}

func main() {
	total := 0
	for round := 0; round < 10; round++ {
		total += countPrimes(2000000)
	}
	fmt.Println(total)
}
