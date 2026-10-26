# Dynamic languages

A0's HIR is typed, but a dynamic language is not at a disadvantage: `any` holds a value of any
type, and Lox (`examples/lox`), whose every value is an `any`, runs faster than clox, the book's
own bytecode machine, on five of its six benchmarks (`docs/benchmarks.md`).

## `any`

`v as any` boxes a value, `v is T` tests what an `any` holds and `v as T` unpacks it (panicking with
`expected int, found string` if it is something else). Integers that fit in 48 bits, floats,
booleans and references fit in the `any` itself; larger integers are boxed. The optimizer decides
tests statically where it can: once a helper such as `fn truthy(v: any) -> bool` is inlined at a
call with `x as any`, `(x as any) is bool` folds to a constant (D27).

## Semantics in HIR: a prelude

Lox's semantics (calls, classes, operators, truthiness, errors) are HIR functions in its prelude
(`examples/lox/prelude.hir`), written in the HIR text format and added to every program. The
compiler turns `a + b` into a call of the prelude's `add`, which tests the operands' types; the
optimizer inlines the small functions and folds the tests away where the types are known. Writing
semantics as HIR keeps the compiler small and lets every backend run the same code.

## Dynamic objects

`new object` is an object whose fields are named at run time (D66): `field(o, "name")`,
`set_field(o, "name", v)`, `has_field`, `field_names`. Objects have *shapes*, shared by objects
whose fields were added in the same order, so a field is a slot found through the shape; and every
place in the program that names a field with a literal keeps a cache of the shape it saw last, so a
property access is a comparison and a load. Names are interned, so comparing them is comparing
pointers. B0 writes them as fields: `o.name = "x"`.

## Exceptions

`throw` raises any value and `try`/`catch` handles it (`docs/hir.md`, "Errors"); an exception
nobody catches ends the program with a stack trace. Lox uses them for runtime errors, as jlox
does; a typed language can use `std.Result` and `?` instead, or both, as B0 does.
