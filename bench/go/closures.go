// Closures: a higher-order function (fold) called 100,000 times over 1,000 numbers with a closure
// that captures a variable, as code written with map and fold is.
package main

import "fmt"

func fold(xs []int, init int, f func(int, int) int) int {
	acc := init
	for _, x := range xs {
		acc = f(acc, x)
	}
	return acc
}

func main() {
	xs := make([]int, 0)
	for i := 0; i < 1000; i++ {
		xs = append(xs, i)
	}
	total := 0
	for k := 0; k < 100000; k++ {
		scale := k % 7
		total += fold(xs, k, func(acc int, x int) int { return acc + x*scale })
	}
	fmt.Println(total)
}
