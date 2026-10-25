// Allocation: many short-lived binary trees and one long-lived one (the Benchmarks Game's
// binary-trees, maximum depth 16), left to Go's garbage collector.
package main

import "fmt"

type Node struct{ left, right *Node }

func make_(depth int) *Node {
	if depth == 0 {
		return &Node{}
	}
	return &Node{make_(depth - 1), make_(depth - 1)}
}

func (n *Node) check() int {
	if n.left == nil {
		return 1
	}
	return 1 + n.left.check() + n.right.check()
}

func main() {
	maxDepth := 16
	fmt.Println(maxDepth+1, make_(maxDepth+1).check())
	longLived := make_(maxDepth)
	for depth := 4; depth <= maxDepth; depth += 2 {
		iterations := 1 << (maxDepth - depth + 4)
		total := 0
		for i := 0; i < iterations; i++ {
			total += make_(depth).check()
		}
		fmt.Println(iterations, depth, total)
	}
	fmt.Println(maxDepth, longLived.check())
}
