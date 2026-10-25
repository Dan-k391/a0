# Lispy

Calc's forms in parentheses: a second language defined by its grammar alone
([`lispy.a0g`](lispy.a0g)), to show that the forms (`docs/grammar.md`, "Languages without host
code") fix what programs mean, not how they look.

```
; comments start with a semicolon
(defn fib (n)
  (if (< n 2) (return n))
  (return (+ (fib (- n 1)) (fib (- n 2)))))
(print (fib 20))
```

`(let name value)`, `(set! name value)`, `(print values...)`, `(if cond then else)`,
`(while cond body...)`, `(repeat n body...)`, `(return value)`, `(do stmts...)`, operations
`(+ a b)` ... `(and a b)`, and calls `(f args...)`, including the standard mathematics (`sqrt`, ...).

```
a0 lang examples/lispy/lispy.a0g run examples/lispy/programs/fib.lisp
```

It is one of the playground's examples (`docs/playground.md`); `crates/a0/tests/dsl.rs` runs its
programs interpreted and native.
