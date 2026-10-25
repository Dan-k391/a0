"""Pebble's compiler: one walk over the syntax tree that pebble.a0g gives, building A0's high-level IR
by name through the C API. The HIR builder resolves names, infers and checks types and reports
errors; A0 does the rest (the interpreter, native code, the collector).

    python pebble.py run programs/arrays.pb
    python pebble.py build programs/arrays.pb -o arrays
"""

import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))

import a0  # noqa: E402
from a0.driver import main  # noqa: E402

with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "pebble.a0g"),
          encoding="utf-8") as f:
    GRAMMAR = a0.Grammar(f.read())


def compile(text):
    """The program as HIR; raises `a0.Error` with its syntax, name and type errors."""
    tree = GRAMMAR.parse(text)
    b = a0.HirBuilder("pebble")
    b.enable_warning("unused_variable")
    Compiler(b).program(tree.root)
    return b.finish()


class Compiler:
    def __init__(self, b):
        self.b = b

    def program(self, root):
        # Every function first, so that they can call each other in any order, then their bodies.
        b = self.b
        functions = []
        for item in root.nodes:
            params = [p for p in item.nodes if p.kind == "param"]
            names = [p.field("name").text for p in params]
            types = [self.type(p.field("type")) for p in params]
            ret = item.field("ret")
            ret = self.type(ret) if ret else b.named_type("unit", [])
            b.at(item.field("name"))
            b.set_name_span(*item.field("name").span)
            f = b.declare_function(item.field("name").text, [])
            b.set_signature(f, (names, types), ret)
            body = b.function_body(f)
            for i, p in enumerate(params):
                b.set_param_span(body, i, *p.field("name").span)
            functions.append((body, item))
        for body, item in functions:
            b.begin_body(body)
            root = self.block(item.field("body"))
            b.end_body()
            b.set_root(body, root)

    def type(self, node):
        self.b.at(node)
        elem = node.field("elem")
        if elem is not None:
            return self.b.array_type(self.type(elem))
        return self.b.named_type(node.field("name").text, [])

    def block(self, node):
        b = self.b
        b.push_scope()
        items = [self.stmt(s) for s in node.nodes]
        b.pop_scope()
        b.at(node)
        return b.block(items, None)

    def stmt(self, node):
        b, kind = self.b, node.kind
        if kind == "let_stmt":
            value = self.expr(node.field("value"))
            b.at(node.field("name"))
            return b.let_named(node.field("name").text, b.fresh_type(), value)
        if kind == "if_stmt":
            cond = self.expr(node.field("cond"))
            then = self.block(node.field("then"))
            other = node.field("else")
            if other is not None:
                other = self.stmt(other) if other.kind == "if_stmt" else self.block(other)
            b.at(node)
            return b.if_(cond, then, other)
        if kind == "while_stmt":
            cond = self.expr(node.field("cond"))
            body = self.block(node.field("body"))
            b.at(node)
            return b.while_(cond, body)
        if kind == "for_stmt":
            return self.for_(node)
        if kind == "return_stmt":
            value = node.field("value")
            value = self.expr(value) if value is not None else None
            b.at(node)
            return b.return_(value)
        if kind == "block":
            return self.block(node)
        target, value = node.field("expr"), node.field("value")
        if value is None:
            return self.expr(target)
        return self.assign(target, value)

    def for_(self, node):
        """`for x in xs { body }` is `{ let xs' = xs; let i' = 0; while i' < len(xs') { let x =
        xs'[i']; i' = i' + 1; body } }`, with names a program cannot write."""
        b = self.b
        items = self.expr(node.field("iter"))
        b.at(node)
        b.push_scope()
        let_items = b.let_named(" items", b.fresh_type(), items)
        let_index = b.let_named(" index", b.named_type("int", []), b.int(0))
        cond = b.binary("<", b.name(" index"), b.builtin("len", [b.name(" items")]))
        b.push_scope()
        element = b.index(b.name(" items"), b.name(" index"))
        b.at(node.field("name"))
        let_element = b.let_named(node.field("name").text, b.fresh_type(), element)
        b.at(node)
        step = b.assign_named(" index", b.binary("+", b.name(" index"), b.int(1)))
        body = self.block(node.field("body"))
        b.pop_scope()
        b.at(node)
        loop = b.while_(cond, b.block([let_element, step, body], None))
        b.pop_scope()
        return b.block([let_items, let_index, loop], None)

    def assign(self, target, value):
        b = self.b
        target = unwrap(target)
        if target.kind == "index":
            container = self.expr(target.field("operand"))
            index = self.expr(target.field("index"))
            v = self.expr(value)
            b.at(target)
            return b.set_index(container, index, v)
        if target.kind != "name_expr":
            b.at(target)
            b.report_code("E1009", "only variables and elements can be assigned")
            return b.error_expr()
        v = self.expr(value)
        b.at(target)
        return b.assign_named(target.text, v)

    def expr(self, node):
        b = self.b
        node = unwrap(node)
        kind = node.kind
        if kind == "literal":
            return self.literal(node)
        if kind == "paren":
            return self.expr(node.field("inner"))
        if kind in ("binary", "logical"):
            left, right = self.expr(node.field("left")), self.expr(node.field("right"))
            b.at(node)
            return b.binary(node.field("op").text, left, right)
        if kind == "unary":
            operand = self.expr(node.field("operand"))
            b.at(node)
            return b.unary(node.field("op").text, operand)
        if kind == "call":
            return self.call(node)
        if kind == "index":
            container = self.expr(node.field("operand"))
            index = self.expr(node.field("index"))
            b.at(node)
            return b.index(container, index)
        if kind == "array":
            elems = [self.expr(e) for e in node.fields("elem")]
            b.at(node)
            if not elems:
                return b.new_array(b.array_type(b.fresh_type()))
            return b.array(elems)
        b.at(node)
        return b.name(node.text)

    def literal(self, node):
        b = self.b
        token = node.children[0]
        b.at(node)
        if token.kind == "INT":
            value = int(token.text)
            if value >= 2 ** 63:
                b.report_code("E0002", "this integer does not fit in 64 bits")
                return b.error_expr()
            return b.int(value)
        if token.kind == "FLOAT":
            return b.float(float(token.text))
        if token.kind == "STRING":
            return b.string(unquote(token.text))
        return b.bool(token.text == "true")

    def call(self, node):
        """`f(args)`: a function or a builtin (`str`, `len`, `push`, ...); `print(x)` prints `x` on a
        line."""
        b = self.b
        callee = unwrap(node.field("operand"))
        args = node.field("args")
        args = [self.expr(a) for a in args.fields("arg")] if args is not None else []
        b.at(node)
        if callee.kind != "name_expr":
            b.report_code("E2003", "only functions can be called")
            return b.error_expr()
        b.set_name_span(*callee.span)
        if callee.text == "print":
            value = b.builtin("print", args)
            newline = b.builtin("print", [b.string("\n")])
            return b.block([value, newline], None)
        return b.call_named(callee.text, args)


def unwrap(node):
    return node.children[0] if node.kind == "primary" else node


def unquote(text):
    escapes = {"n": "\n", "t": "\t", "r": "\r", "\\": "\\", '"': '"', "0": "\0"}
    out, i, body = [], 0, text[1:-1]
    while i < len(body):
        c = body[i]
        if c == "\\" and i + 1 < len(body):
            out.append(escapes.get(body[i + 1], body[i + 1]))
            i += 2
        else:
            out.append(c)
            i += 1
    return "".join(out)


if __name__ == "__main__":
    sys.exit(main("pebble", compile, grammar=GRAMMAR, repl_main="fn main() {\n{}}\n"))
