"""A0 from Python: grammars and syntax trees, the HIR and core IR builders, and running and compiling
programs, over A0's C API (`crates/a0-capi/include/a0.h`, `docs/python.md`) with ctypes.

    grammar = a0.Grammar(open("calc.a0g").read())
    tree = grammar.parse("1 + 2")            # raises a0.Error with the syntax errors
    b = a0.HirBuilder("calc")
    ...                                      # b.let_named("x", b.fresh_type(), b.int(1)), ...
    module = b.finish().lower()              # a0.Hir, then a0.Module (core IR)
    status = module.run(["calc"])            # interpreted; module.build_executable("calc.exe")

The builders' methods are the C functions without their prefix (`a0_hir_let_named` is
`HirBuilder.let_named`; `if`, `while`, `return`, ... get a trailing underscore), taking Python
strings for names, ints for handles (`None` for none), and lists for arrays. The shared library is
found through `A0_LIBRARY`, or in the `target/release` or `target/debug` directory of the A0
checkout the package is in (`cargo build -p a0-capi` builds it).
"""

import ctypes
import keyword
import os
import sys
from ctypes import POINTER, byref, c_char_p, c_double, c_int, c_int32, c_int64, c_size_t
from ctypes import c_uint8, c_uint32, c_void_p

__all__ = ["Builder", "Diagnostic", "Error", "Grammar", "Hir", "HirBuilder", "Module", "Node",
           "Tree", "NONE", "version", "__version__"]

#: The version of this package, which the library it loads must match in its major version and be
#: at least as new as in its minor one (`docs/compatibility.md`).
__version__ = "1.0.0"

#: The handle that stands for "none" (`A0_NONE`).
NONE = 0xFFFFFFFF

# Core IR types (`A0Type`).
VOID, I1, I8, I16, I32, I64, F32, F64, PTR = range(9)


def _library_path():
    path = os.environ.get("A0_LIBRARY")
    if path:
        return path
    if sys.platform == "win32":
        name = "a0_capi.dll"
    elif sys.platform == "darwin":
        name = "liba0_capi.dylib"
    else:
        name = "liba0_capi.so"
    # An installed package (a wheel, a release archive) carries the library inside it.
    bundled = os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", name)
    if os.path.exists(bundled):
        return bundled
    root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
    for profile in ("release", "debug"):
        candidate = os.path.join(root, "target", profile, name)
        if os.path.exists(candidate):
            return candidate
    raise ImportError(f"cannot find {name}: build it with `cargo build -p a0-capi` or set A0_LIBRARY")


# Global, so that the interpreter finds the runtime's symbols in the process.
_lib = ctypes.CDLL(_library_path(), mode=getattr(ctypes, "RTLD_GLOBAL", 0))


class _Str(ctypes.Structure):
    _fields_ = [("ptr", c_char_p), ("len", c_size_t)]


def _fn(name, restype, *argtypes):
    f = getattr(_lib, name)
    f.restype = restype
    f.argtypes = list(argtypes)
    return f


def _taken(ptr, length):
    return ctypes.string_at(ptr, length.value).decode("utf-8", "replace") if ptr else ""


_version = _fn("a0_version", c_char_p)


def version():
    """The version of the A0 library."""
    return _version().decode()


def _check_library():
    want = [int(x) for x in __version__.split(".")]
    have = [int(x) for x in version().split(".")]
    if have[0] != want[0] or have[1] < want[1]:
        raise ImportError(f"the A0 library is version {version()}, and this package {__version__} "
                          f"needs {want[0]}.{want[1]} or a later {want[0]}.x")


_check_library()


def _deprecated(what, instead):
    """Warns that `what` is deprecated and will be removed in the next major version."""
    import warnings
    warnings.warn(f"{what} is deprecated; use {instead}", DeprecationWarning, stacklevel=3)


# ---------------------------------------------------------------------------------- diagnostics

_diag_count = _fn("a0_diagnostics_count", c_size_t, c_void_p)
_diag_message = _fn("a0_diagnostics_message", c_void_p, c_void_p, c_size_t, POINTER(c_size_t))
_diag_code = _fn("a0_diagnostics_code", c_void_p, c_void_p, c_size_t, POINTER(c_size_t))
_diag_span = _fn("a0_diagnostics_span", c_int, c_void_p, c_size_t, POINTER(c_uint32),
                 POINTER(c_uint32))
_diag_text = _fn("a0_diagnostics_text", c_void_p, c_void_p, POINTER(c_size_t))
_diag_render = _fn("a0_diagnostics_render", c_void_p, c_void_p, c_char_p, c_size_t, c_char_p,
                   c_size_t, POINTER(c_size_t))
_diag_free = _fn("a0_diagnostics_free", None, c_void_p)


class Diagnostic:
    """One error or warning: its code (`E2001`), message and span (byte offsets), if it has one."""

    def __init__(self, code, message, span):
        self.code, self.message, self.span = code, message, span

    def __repr__(self):
        return f"Diagnostic({self.code!r}, {self.message!r}, {self.span!r})"


class Error(Exception):
    """Errors A0 reported: `diagnostics` lists them, `str(error)` has one per line, and
    `render(name, source)` shows them against the source the way the `a0` tool does."""

    def __init__(self, handle):
        self._handle = handle
        n = c_size_t()
        self.diagnostics = []
        for i in range(_diag_count(handle)):
            message = _taken(_diag_message(handle, i, byref(n)), n)
            code = _taken(_diag_code(handle, i, byref(n)), n)
            start, end = c_uint32(), c_uint32()
            span = (start.value, end.value) if _diag_span(handle, i, byref(start), byref(end)) else None
            self.diagnostics.append(Diagnostic(code, message, span))
        super().__init__(_taken(_diag_text(handle, byref(n)), n))

    def render(self, name, source):
        n = c_size_t()
        name, source = name.encode(), source.encode()
        return _taken(_diag_render(self._handle, name, len(name), source, len(source), byref(n)), n)

    def __del__(self):
        if _diag_free is not None and self._handle:
            _diag_free(self._handle)
            self._handle = None


def _check(diags):
    if diags.value:
        raise Error(diags.value)


class _Owned:
    """A C object freed with `_free` when its Python object goes."""

    _free = None

    def __init__(self, handle):
        self._handle = handle

    def _take(self):
        handle, self._handle = self._handle, None
        if not handle:
            raise ValueError(f"this {type(self).__name__} was already used up")
        return handle

    def __del__(self):
        free = type(self)._free
        if free is not None and getattr(self, "_handle", None):
            free(self._handle)
            self._handle = None


# --------------------------------------------------------------------------------- grammars, trees

_grammar_new = _fn("a0_grammar_new", c_void_p, c_char_p, c_size_t, POINTER(c_void_p))
_grammar_free = _fn("a0_grammar_free", None, c_void_p)
_grammar_parse = _fn("a0_grammar_parse", c_void_p, c_void_p, c_char_p, c_size_t, POINTER(c_void_p))
_tree_free = _fn("a0_tree_free", None, c_void_p)
_tree_root = _fn("a0_tree_root", c_uint32, c_void_p)
_tree_kind = _fn("a0_tree_kind", c_void_p, c_void_p, c_uint32, POINTER(c_size_t))
_tree_text = _fn("a0_tree_text", c_void_p, c_void_p, c_uint32, POINTER(c_size_t))
_tree_span = _fn("a0_tree_span", c_int, c_void_p, c_uint32, POINTER(c_uint32), POINTER(c_uint32))
_tree_is_token = _fn("a0_tree_is_token", c_int, c_void_p, c_uint32)
_tree_child_count = _fn("a0_tree_child_count", c_uint32, c_void_p, c_uint32)
_tree_child = _fn("a0_tree_child", c_uint32, c_void_p, c_uint32, c_uint32)
_tree_child_field = _fn("a0_tree_child_field", c_void_p, c_void_p, c_uint32, c_uint32,
                        POINTER(c_size_t))
_tree_field = _fn("a0_tree_field", c_uint32, c_void_p, c_uint32, c_char_p, c_size_t)


class Grammar(_Owned):
    """A grammar in A0's notation (`docs/grammar.md`); raises `Error` if it is invalid."""

    _free = _grammar_free

    def __init__(self, text):
        data = text.encode()
        diags = c_void_p()
        handle = _grammar_new(data, len(data), byref(diags))
        _check(diags)
        super().__init__(handle)
        #: The grammar's text, which the driver gives the language server and the REPL.
        self.text = text

    def parse(self, text):
        """The syntax tree of `text`; raises `Error` with the syntax errors."""
        data = text.encode()
        diags = c_void_p()
        handle = _grammar_parse(self._handle, data, len(data), byref(diags))
        if diags.value:
            _tree_free(handle)
            raise Error(diags.value)
        return Tree(handle, text)


class Tree(_Owned):
    """A syntax tree; `root` is its root node."""

    _free = _tree_free

    def __init__(self, handle, text):
        super().__init__(handle)
        self.text = text

    @property
    def root(self):
        return Node(self, _tree_root(self._handle))


class Node:
    """A node or token of a tree. Nodes compare equal when they are the same node."""

    __slots__ = ("tree", "id")

    def __init__(self, tree, id):
        self.tree, self.id = tree, id

    def __eq__(self, other):
        return isinstance(other, Node) and other.tree is self.tree and other.id == self.id

    def __hash__(self):
        return hash(self.id)

    def __repr__(self):
        return f"<{self.kind} {self.text!r}>"

    @property
    def kind(self):
        """The rule or token kind (`let_stmt`, `NAME`, `"+"`)."""
        n = c_size_t()
        return _taken(_tree_kind(self.tree._handle, self.id, byref(n)), n)

    @property
    def text(self):
        n = c_size_t()
        return _taken(_tree_text(self.tree._handle, self.id, byref(n)), n)

    @property
    def span(self):
        """Byte offsets `(start, end)` into the parsed text."""
        start, end = c_uint32(), c_uint32()
        _tree_span(self.tree._handle, self.id, byref(start), byref(end))
        return start.value, end.value

    @property
    def is_token(self):
        return bool(_tree_is_token(self.tree._handle, self.id))

    @property
    def children(self):
        t = self.tree._handle
        return [Node(self.tree, _tree_child(t, self.id, i))
                for i in range(_tree_child_count(t, self.id))]

    @property
    def nodes(self):
        """The children that are nodes, not tokens."""
        return [c for c in self.children if not c.is_token]

    def field(self, name):
        """The first child labeled `name`, or None."""
        data = name.encode()
        n = _tree_field(self.tree._handle, self.id, data, len(data))
        return None if n == NONE else Node(self.tree, n)

    def fields(self, name):
        """Every child labeled `name`, in order."""
        t = self.tree._handle
        out = []
        n = c_size_t()
        for i in range(_tree_child_count(t, self.id)):
            label = _tree_child_field(t, self.id, i, byref(n))
            if label and _taken(label, n) == name:
                out.append(Node(self.tree, _tree_child(t, self.id, i)))
        return out


# ---------------------------------------------------------------------------- modules and running

_module_parse = _fn("a0_module_parse", c_void_p, c_char_p, c_size_t, POINTER(c_void_p))
_module_text = _fn("a0_module_text", c_void_p, c_void_p, POINTER(c_size_t))
_module_free = _fn("a0_module_free", None, c_void_p)
_run = _fn("a0_run", c_int, c_void_p, POINTER(c_char_p), c_size_t, POINTER(c_int32),
           POINTER(c_void_p))
_jit_run = _fn("a0_jit_run", c_int, c_void_p, POINTER(c_char_p), c_size_t, POINTER(c_int32),
               POINTER(c_void_p))
_write_object = _fn("a0_write_object", c_int, c_void_p, c_char_p, c_size_t, c_char_p, c_size_t,
                    POINTER(c_void_p))
_build_executable = _fn("a0_build_executable", c_int, c_void_p, c_char_p, c_size_t,
                        POINTER(c_void_p))
_hir_parse = _fn("a0_hir_parse", c_void_p, c_char_p, c_size_t, POINTER(c_void_p))
_hir_lower = _fn("a0_hir_lower", c_void_p, c_void_p, POINTER(c_void_p))
_hir_text = _fn("a0_hir_text", c_void_p, c_void_p, POINTER(c_size_t))
_hir_free = _fn("a0_hir_free", None, c_void_p)
_hir_warnings = _fn("a0_hir_warnings", None, c_void_p, POINTER(c_void_p))

#: What a program that panics exits with, interpreted or native.
PANIC_EXIT_CODE = 101


class Module(_Owned):
    """A core IR module (`docs/ir.md`): run it, or compile it to native code."""

    _free = _module_free

    @classmethod
    def parse(cls, text):
        data = text.encode()
        diags = c_void_p()
        handle = _module_parse(data, len(data), byref(diags))
        _check(diags)
        return cls(handle)

    def __str__(self):
        n = c_size_t()
        return _taken(_module_text(self._handle, byref(n)), n)

    def run(self, args=()):
        """Interprets `main` with `args` (the program's name first) and returns its exit status.
        A panic is printed, with its stack trace, on standard error, and gives `PANIC_EXIT_CODE`,
        as in a native executable."""
        sys.stdout.flush()
        sys.stderr.flush()
        encoded = [a.encode() for a in args]
        argv = (c_char_p * max(len(encoded), 1))(*encoded)
        status = c_int32()
        diags = c_void_p()
        result = _run(self._handle, argv, len(encoded), byref(status), byref(diags))
        if result == 1:
            sys.stderr.write(str(Error(diags.value)) + "\n")
            sys.stderr.flush()
            return status.value
        _check(diags)
        return status.value

    def run_jit(self, args=()):
        """Compiles `main` into memory and runs it at native speed with `args`, returning its exit
        status. A panic is printed, with its stack trace, and ends the process with
        `PANIC_EXIT_CODE`, as a native executable does."""
        sys.stdout.flush()
        sys.stderr.flush()
        encoded = [a.encode() for a in args]
        argv = (c_char_p * max(len(encoded), 1))(*encoded)
        status = c_int32()
        diags = c_void_p()
        _jit_run(self._handle, argv, len(encoded), byref(status), byref(diags))
        _check(diags)
        return status.value

    def write_object(self, path, target=""):
        """Writes an object file for `target` (a name like `x86_64-linux`; the host by default)."""
        t, p = target.encode(), os.fspath(path).encode()
        diags = c_void_p()
        _write_object(self._handle, t, len(t), p, len(p), byref(diags))
        _check(diags)

    def build_executable(self, path):
        """Compiles and links a native executable for the host (needs the system linker)."""
        p = os.fspath(path).encode()
        diags = c_void_p()
        _build_executable(self._handle, p, len(p), byref(diags))
        _check(diags)


class Hir(_Owned):
    """A high-level IR module (`docs/hir.md`), from `HirBuilder.finish` or `Hir.parse`."""

    _free = _hir_free

    @classmethod
    def parse(cls, text):
        data = text.encode()
        diags = c_void_p()
        handle = _hir_parse(data, len(data), byref(diags))
        _check(diags)
        return cls(handle)

    def __str__(self):
        n = c_size_t()
        return _taken(_hir_text(self._handle, byref(n)), n)

    @property
    def warnings(self):
        """The warnings found while building, as an `Error` (which has `diagnostics` and `render`),
        or None."""
        diags = c_void_p()
        _hir_warnings(self._handle, byref(diags))
        return Error(diags.value) if diags.value else None

    def lower(self):
        """The core IR module, optimized unless `A0_OPT=0`."""
        diags = c_void_p()
        handle = _hir_lower(self._handle, byref(diags))
        _check(diags)
        return Module(handle)


# -------------------------------------------------------------------------------------- builders

# The builders' functions after their receiver, from a0.h: one letter per parameter (`s` a string,
# `u` a handle or type code, `i` an int64, `f` a double, `b` a boolean, `U` a list of handles, `N`
# a list of names, `P` a list of names and a list of handles of the same length, `B` bytes, `o` a
# handle returned besides the result, `D` diagnostics raised as `Error`), then `:` and the result
# (`u` a handle, `i` an int, `v` nothing). `tests/test_a0.py` checks them against the header.
HIR_FUNCTIONS = {
    "set_span": "uu:v",
    "clear_span": ":v",
    "named_type": "sU:u",
    "fresh_type": ":u",
    "array_type": "u:u",
    "ptr_type": "u:u",
    "size_of": "u:u",
    "map_type": "uu:u",
    "fn_type": "Uu:u",
    "nullable_type": "u:u",
    "struct_type": "uU:u",
    "enum_type": "uU:u",
    "type_param": "uuu:u",
    "type_of": "u:u",
    "set_type_syntax": "ss:v",
    "set_term": "ss:v",
    "enable_warning": "s:v",
    "set_name_span": "uu:v",
    "set_param_span": "uuuu:v",
    "report_code": "ss:v",
    "report_label": "uus:v",
    "report_note": "s:v",
    "report_fix": "suus:v",
    "declare_struct": "sN:u",
    "set_struct_inline": "u:v",
    "set_memory": "s:v",
    "add_field": "usu:u",
    "declare_enum": "sN:u",
    "add_variant": "usU:u",
    "declare_global": "su:u",
    "declare_interface": "s:u",
    "interface_self": "u:u",
    "interface_type": "u:u",
    "add_interface_method": "usUu:v",
    "declare_method": "usN:u",
    "method_self_type": "u:u",
    "implement": "uu:v",
    "add_bound": "uuuu:v",
    "method_call": "usU:u",
    "add_text": "sD:i",
    "declare_extern": "sUu:u",
    "declare_extern_abi": "sUus:u",
    "declare_function": "sN:u",
    "set_signature": "uPu:v",
    "function_body": "u:u",
    "new_closure_body": "uPu:u",
    "set_closure_name": "us:v",
    "param": "uu:u",
    "new_local": "usu:u",
    "set_root": "uu:v",
    "lookup_type": "so:u",
    "lookup_value": "so:u",
    "unit": ":u",
    "bool": "b:u",
    "int": "i:u",
    "float": "f:u",
    "string": "s:u",
    "null": "u:u",
    "local": "u:u",
    "global": "u:u",
    "func": "uU:u",
    "let": "uu:u",
    "set_local": "uu:u",
    "set_global": "uu:u",
    "set_field": "uuu:u",
    "set_field_named": "usu:u",
    "set_index": "uuu:u",
    "unary": "su:u",
    "binary": "suu:u",
    "call": "uUU:u",
    "call_extern": "uU:u",
    "call_value": "uU:u",
    "lambda": "u:u",
    "new_struct": "uUU:u",
    "field": "uu:u",
    "field_named": "us:u",
    "variant": "uUuU:u",
    "arm": "uUu:u",
    "match": "uU:u",
    "array": "U:u",
    "new_array": "u:u",
    "new_map": "u:u",
    "new_object": ":u",
    "index": "uu:u",
    "builtin": "sU:u",
    "cast": "uu:u",
    "unwrap": "u:u",
    "is": "uu:u",
    "if": "uuu:u",
    "if_let": "uuuu:u",
    "while": "uu:u",
    "loop": "u:u",
    "throw": "u:u",
    "try_catch": "uuu:u",
    "propagate": "u:u",
    "break": ":u",
    "continue": ":u",
    "return": "u:u",
    "block": "Uu:u",
    "begin_body": "u:v",
    "end_body": ":v",
    "push_scope": ":v",
    "pop_scope": ":v",
    "is_declared_in_scope": "s:i",
    "declare_local": "su:u",
    "declare_if_let": "su:u",
    "declare_catch": "s:u",
    "let_named": "suu:u",
    "lookup_local": "s:u",
    "name": "s:u",
    "call_named": "sU:u",
    "assign_named": "su:u",
    "bind_type_params": "uu:v",
    "new_struct_named": "uP:u",
    "variant_named": "usU:u",
    "report": "s:v",
    "error_expr": ":u",
    "begin_module": "sb:u",
    "set_public": "b:v",
    "import_module": "ss:v",
    "import_item": "ss:v",
    "is_module": "s:i",
    "member": "ss:u",
    "call_member": "ssU:u",
    "member_type": "ssU:u",
}

IR_FUNCTIONS = {
    "declare_extern": "sUu:u",
    "declare_function": "sUu:u",
    "add_data": "sB:u",
    "add_global_int": "sui:u",
    "add_global_float": "suf:u",
    "begin_function": "u:v",
    "end_function": ":v",
    "param": "u:u",
    "declare_var": "su:u",
    "get": "u:u",
    "set": "uu:v",
    "iconst": "ui:u",
    "fconst": "uf:u",
    "null": ":u",
    "func_addr": "u:u",
    "global_addr": "u:u",
    "binary": "suu:u",
    "unary": "su:u",
    "cast": "suu:u",
    "icmp": "suu:u",
    "fcmp": "suu:u",
    "select": "uuu:u",
    "load": "uu:u",
    "store": "uu:v",
    "ptradd": "uu:u",
    "stack_slot": "uu:u",
    "set_struct_param": "uusuuUU:v",
    "call": "uU:u",
    "call_indirect": "uUuU:u",
    "if": "u:v",
    "else": ":v",
    "end_if": ":v",
    "loop": ":v",
    "end_loop": ":v",
    "break": ":v",
    "continue": ":v",
    "break_out": "u:v",
    "continue_out": "u:v",
    "return": "u:v",
    "unreachable": ":v",
}

_C_TYPES = {
    "s": (c_char_p, c_size_t), "u": (c_uint32,), "i": (c_int64,), "f": (c_double,),
    "b": (c_int,), "U": (POINTER(c_uint32), c_size_t), "N": (POINTER(_Str), c_size_t),
    "P": (POINTER(_Str), POINTER(c_uint32), c_size_t), "B": (POINTER(c_uint8), c_size_t),
    "o": (POINTER(c_uint32),), "D": (POINTER(c_void_p),),
}
_RESULTS = {"u": c_uint32, "i": c_int32, "v": None}


def _method(c_name, spec):
    params, result = spec.split(":")
    argtypes = [c_void_p]
    for p in params:
        argtypes.extend(_C_TYPES[p])
    f = _fn(c_name, _RESULTS[result], *argtypes)
    takes = [p for p in params if p not in "oD"]

    def method(self, *args):
        if len(args) != len(takes):
            raise TypeError(f"{c_name} takes {len(takes)} arguments ({params}), not {len(args)}")
        c_args, keep = [self._handle], []
        out, diags = c_uint32(), c_void_p()
        it = iter(args)
        for p in params:
            if p == "o":
                c_args.append(byref(out))
                continue
            if p == "D":
                c_args.append(byref(diags))
                continue
            a = next(it)
            if p == "s":
                data = a.encode()
                c_args += [data, len(data)]
            elif p == "u":
                c_args.append(NONE if a is None else a)
            elif p in "ifb":
                c_args.append(a)
            elif p == "U":
                a = [NONE if x is None else x for x in (a or ())]
                c_args += [(c_uint32 * max(len(a), 1))(*a), len(a)]
            elif p == "B":
                c_args += [(c_uint8 * max(len(a), 1))(*a), len(a)]
            elif p == "N":
                c_args += [_names(a, keep), len(a)]
            else:  # "P": names and handles
                names, handles = a
                if len(names) != len(handles):
                    raise ValueError(f"{c_name}: {len(names)} names but {len(handles)} values")
                handles = list(handles)
                c_args += [_names(names, keep), (c_uint32 * max(len(handles), 1))(*handles),
                           len(names)]
        value = f(*c_args)
        _check(diags)
        return (value, out.value) if "o" in params else value

    method.__name__ = c_name
    method.__doc__ = f"`{c_name}` (a0.h)."
    return method


def _names(names, keep):
    data = [n.encode() for n in names]
    keep.append(data)
    return (_Str * max(len(data), 1))(*[_Str(d, len(d)) for d in data])


def _install(cls, prefix, table):
    for name, spec in table.items():
        attr = name + "_" if keyword.iskeyword(name) else name
        setattr(cls, attr, _method(prefix + name, spec))


_hir_builder_new = _fn("a0_hir_builder_new", c_void_p, c_char_p, c_size_t)
_hir_builder_from = _fn("a0_hir_builder_from", c_void_p, c_void_p)
_hir_builder_free = _fn("a0_hir_builder_free", None, c_void_p)
_hir_finish = _fn("a0_hir_finish", c_void_p, c_void_p, POINTER(c_void_p))
_builder_new = _fn("a0_builder_new", c_void_p, c_char_p, c_size_t)
_builder_finish = _fn("a0_builder_finish", c_void_p, c_void_p, POINTER(c_void_p))


class HirBuilder(_Owned):
    """Builds a HIR module (`docs/hir.md`, "Building HIR from a frontend"): by name, with the
    builder resolving names, inferring and checking types, or by handle. Mistakes are reported by
    `finish`, which raises `Error` with every problem found."""

    _free = _hir_builder_free

    def __init__(self, module_name):
        data = module_name.encode()
        super().__init__(_hir_builder_new(data, len(data)))

    @classmethod
    def from_hir(cls, hir):
        """A builder that continues `hir` (which it uses up)."""
        builder = cls.__new__(cls)
        _Owned.__init__(builder, _hir_builder_from(hir._take()))
        return builder

    def at(self, node):
        """Attributes what is built next, and its errors, to `node` (a `Node` or a span)."""
        start, end = node.span if isinstance(node, Node) else node
        self.set_span(start, end)

    def finish(self):
        """The module; raises `Error` if anything was wrong. The builder is used up."""
        diags = c_void_p()
        handle = _hir_finish(self._take(), byref(diags))
        _check(diags)
        return Hir(handle)


class Builder(_Owned):
    """Builds a core IR module (`docs/ir.md`) instruction by instruction."""

    def __init__(self, module_name):
        data = module_name.encode()
        super().__init__(_builder_new(data, len(data)))

    def finish(self):
        """The module; raises `Error` if anything was wrong. The builder is used up."""
        diags = c_void_p()
        handle = _builder_finish(self._take(), byref(diags))
        _check(diags)
        return Module(handle)


_install(HirBuilder, "a0_hir_", HIR_FUNCTIONS)
_install(Builder, "a0_builder_", IR_FUNCTIONS)
