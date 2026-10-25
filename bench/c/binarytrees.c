/* Allocation: many short-lived binary trees and one long-lived one (the Benchmarks Game's
 * binary-trees, maximum depth 16), with malloc and free. */
#include <stdio.h>
#include <stdlib.h>

typedef struct Node {
    struct Node *left, *right;
} Node;

static Node *make(int depth) {
    Node *n = malloc(sizeof(Node));
    if (depth == 0) {
        n->left = n->right = NULL;
    } else {
        n->left = make(depth - 1);
        n->right = make(depth - 1);
    }
    return n;
}

static long check(Node *n) {
    return n->left == NULL ? 1 : 1 + check(n->left) + check(n->right);
}

static void release(Node *n) {
    if (n->left != NULL) {
        release(n->left);
        release(n->right);
    }
    free(n);
}

int main(void) {
    int max_depth = 16;
    Node *stretch = make(max_depth + 1);
    printf("%d %ld\n", max_depth + 1, check(stretch));
    release(stretch);
    Node *long_lived = make(max_depth);
    for (int depth = 4; depth <= max_depth; depth += 2) {
        long iterations = 1L << (max_depth - depth + 4);
        long total = 0;
        for (long i = 0; i < iterations; i++) {
            Node *t = make(depth);
            total += check(t);
            release(t);
        }
        printf("%ld %d %ld\n", iterations, depth, total);
    }
    printf("%d %ld\n", max_depth, check(long_lived));
    release(long_lived);
    return 0;
}
