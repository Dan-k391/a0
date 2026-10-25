/* A C program that uses A0 through its C API only. It builds a module with the builder, prints and
 * re-parses it, runs it in the interpreter, compiles it to an object file and to an executable,
 * compiles a calculator language whose grammar is an A0 grammar, and runs a HIR program.
 * tests/c_host.rs compiles it with the system C compiler, runs it and checks what it printed. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "a0.h"

#ifdef _WIN32
#define EXE "fib-native.exe"
#else
#define EXE "fib-native"
#endif

static int failures = 0;

/* Prints and frees diagnostics, if there are any. */
static void check(const char *what, A0Diagnostics *d) {
    size_t len;
    const char *text;
    if (d == NULL) return;
    text = a0_diagnostics_text(d, &len);
    printf("%s failed:\n%.*s\n", what, (int)len, text);
    a0_diagnostics_free(d);
    failures++;
}

static int contains(const char *text, size_t len, const char *word) {
    size_t n = strlen(word), i;
    for (i = 0; i + n <= len; i++) {
        if (memcmp(text + i, word, n) == 0) return 1;
    }
    return 0;
}

/* fib(n) = n < 2 ? n : fib(n - 1) + fib(n - 2). main prints fib(20) and the sum 1 + ... + 10
 * (with a loop and a variable), then returns 7. */
static A0Module *build_fib(void) {
    A0Builder *b = a0_builder_new(A0_STR("fib"));
    A0Type one_i64[1] = {A0_I64};
    A0Func print = a0_builder_declare_extern(b, A0_STR("a0_print_i64"), one_i64, 1, A0_VOID);
    A0Func newline = a0_builder_declare_extern(b, A0_STR("a0_print_newline"), NULL, 0, A0_VOID);
    A0Func fib = a0_builder_declare_function(b, A0_STR("fib"), one_i64, 1, A0_I64);
    A0Func main_fn = a0_builder_declare_function(b, A0_STR("main"), NULL, 0, A0_I32);
    A0Value n, one, two, args[1], left, right, result;
    A0Var i, sum;
    A0Diagnostics *d = NULL;
    A0Module *m;

    a0_builder_begin_function(b, fib);
    n = a0_builder_param(b, 0);
    two = a0_builder_iconst(b, A0_I64, 2);
    a0_builder_if(b, a0_builder_icmp(b, A0_STR("slt"), n, two));
    a0_builder_return(b, n);
    a0_builder_end_if(b);
    one = a0_builder_iconst(b, A0_I64, 1);
    args[0] = a0_builder_binary(b, A0_STR("isub"), n, one);
    left = a0_builder_call(b, fib, args, 1);
    args[0] = a0_builder_binary(b, A0_STR("isub"), n, two);
    right = a0_builder_call(b, fib, args, 1);
    a0_builder_return(b, a0_builder_binary(b, A0_STR("iadd"), left, right));
    a0_builder_end_function(b);

    a0_builder_begin_function(b, main_fn);
    args[0] = a0_builder_iconst(b, A0_I64, 20);
    result = a0_builder_call(b, fib, args, 1);
    a0_builder_call(b, print, &result, 1);
    a0_builder_call(b, newline, NULL, 0);
    i = a0_builder_declare_var(b, A0_STR("i"), A0_I64);
    sum = a0_builder_declare_var(b, A0_STR("sum"), A0_I64);
    a0_builder_set(b, i, a0_builder_iconst(b, A0_I64, 1));
    a0_builder_set(b, sum, a0_builder_iconst(b, A0_I64, 0));
    a0_builder_loop(b);
    a0_builder_if(b, a0_builder_icmp(b, A0_STR("sgt"), a0_builder_get(b, i),
                                     a0_builder_iconst(b, A0_I64, 10)));
    a0_builder_break(b);
    a0_builder_end_if(b);
    a0_builder_set(b, sum, a0_builder_binary(b, A0_STR("iadd"), a0_builder_get(b, sum),
                                             a0_builder_get(b, i)));
    a0_builder_set(b, i, a0_builder_binary(b, A0_STR("iadd"), a0_builder_get(b, i),
                                           a0_builder_iconst(b, A0_I64, 1)));
    a0_builder_end_loop(b);
    result = a0_builder_get(b, sum);
    a0_builder_call(b, print, &result, 1);
    a0_builder_call(b, newline, NULL, 0);
    a0_builder_return(b, a0_builder_iconst(b, A0_I32, 7));
    a0_builder_end_function(b);

    m = a0_builder_finish(b, &d);
    check("building fib", d);
    return m;
}

/* A builder used wrongly reports what went wrong instead of crashing. */
static void misuse(void) {
    A0Builder *b = a0_builder_new(A0_STR("bad"));
    A0Func f = a0_builder_declare_function(b, A0_STR("f"), NULL, 0, A0_I64);
    A0Diagnostics *d = NULL;
    size_t len;
    const char *message;
    a0_builder_begin_function(b, f);
    a0_builder_binary(b, A0_STR("no_such_op"), 0, 0);
    a0_builder_end_function(b);
    if (a0_builder_finish(b, &d) != NULL || d == NULL) {
        printf("misuse was not reported\n");
        failures++;
        return;
    }
    message = a0_diagnostics_message(d, 0, &len);
    printf("misuse: %d problems, first: %.*s\n", (int)a0_diagnostics_count(d), (int)len, message);
    a0_diagnostics_free(d);
}

/* ---- A calculator language: an A0 grammar, and a compiler from its trees to the core IR. ---- */

static const char CALC[] =
    "grammar calc;\n"
    "skip /[ \\t\\n]+/;\n"
    "token NUM /[0-9]+/;\n"
    "program = value:expr EOF;\n"
    "expr @inline = @prec atom ~ \"expected a number\" {\n"
    "    left \"+\" \"-\" => binary;\n"
    "    left \"*\" \"/\" => binary;\n"
    "};\n"
    "atom = NUM | \"(\" inner:expr \")\" ~ \"expected ')'\";\n";

static A0Value compile_calc(A0Builder *b, const A0Tree *t, A0Node n) {
    size_t len;
    const char *kind = a0_tree_kind(t, n, &len);
    if (len == 6 && memcmp(kind, "binary", 6) == 0) {
        A0Value x = compile_calc(b, t, a0_tree_field(t, n, A0_STR("left")));
        A0Value y = compile_calc(b, t, a0_tree_field(t, n, A0_STR("right")));
        const char *op = a0_tree_text(t, a0_tree_field(t, n, A0_STR("op")), &len);
        const char *name = *op == '+' ? "iadd" : *op == '-' ? "isub" : *op == '*' ? "imul" : "sdiv";
        return a0_builder_binary(b, name, 4, x, y);
    }
    if (a0_tree_field(t, n, A0_STR("inner")) != A0_NONE) {
        return compile_calc(b, t, a0_tree_field(t, n, A0_STR("inner")));
    }
    {
        char digits[32];
        const char *text = a0_tree_text(t, n, &len);
        if (len >= sizeof digits) len = sizeof digits - 1;
        memcpy(digits, text, len);
        digits[len] = 0;
        return a0_builder_iconst(b, A0_I64, atoll(digits));
    }
}

static void calc(const A0Grammar *g, const char *source) {
    A0Diagnostics *d = NULL;
    A0Tree *t = a0_grammar_parse(g, source, strlen(source), &d);
    A0Builder *b;
    A0Func print, main_fn;
    A0Type one_i64[1] = {A0_I64};
    A0Value v;
    A0Module *m;
    int32_t status;
    if (d != NULL) {
        size_t len;
        uint32_t start = 0, end = 0;
        const char *message = a0_diagnostics_message(d, 0, &len);
        a0_diagnostics_span(d, 0, &start, &end);
        printf("calc: %s: error at %u: %.*s\n", source, start, (int)len, message);
        a0_diagnostics_free(d);
        a0_tree_free(t);
        return;
    }
    b = a0_builder_new(A0_STR("calc"));
    print = a0_builder_declare_extern(b, A0_STR("a0_print_i64"), one_i64, 1, A0_VOID);
    main_fn = a0_builder_declare_function(b, A0_STR("main"), NULL, 0, A0_I32);
    a0_builder_begin_function(b, main_fn);
    v = compile_calc(b, t, a0_tree_field(t, a0_tree_root(t), A0_STR("value")));
    a0_builder_call(b, print, &v, 1);
    a0_builder_return(b, a0_builder_iconst(b, A0_I32, 0));
    a0_builder_end_function(b);
    a0_tree_free(t);
    m = a0_builder_finish(b, &d);
    check("compiling calc", d);
    if (m == NULL) return;
    printf("calc: %s = ", source);
    fflush(stdout);
    a0_run(m, NULL, 0, &status, &d);
    check("running calc", d);
    printf("\n");
    a0_module_free(m);
}

/* ---- A language with variables, compiled to HIR by name: the builder resolves the names. ---- */

static const char VARS[] =
    "grammar vars;\n"
    "skip /[ \\t\\n]+/;\n"
    "token NUM /[0-9]+/;\n"
    "token NAME /[a-z]+/;\n"
    "program = (!EOF stmt)* EOF;\n"
    "stmt @inline = let_stmt | print_stmt;\n"
    "let_stmt = \"let\" name:NAME \"=\" value:expr \";\";\n"
    "print_stmt = \"print\" value:expr \";\";\n"
    "expr @inline = @prec atom ~ \"expected an expression\" {\n"
    "    left \"+\" \"-\" => binary;\n"
    "    left \"*\" \"/\" => binary;\n"
    "};\n"
    "atom = NUM | NAME | \"(\" inner:expr \")\";\n";

static int is_kind(const A0Tree *t, A0Node n, const char *kind) {
    size_t len;
    const char *k = a0_tree_kind(t, n, &len);
    return len == strlen(kind) && memcmp(k, kind, len) == 0;
}

/* Diagnostics point at the node being compiled. */
static void at(A0HirBuilder *b, const A0Tree *t, A0Node n) {
    uint32_t start, end;
    if (a0_tree_span(t, n, &start, &end)) a0_hir_set_span(b, start, end);
}

static A0HirExpr vars_expr(A0HirBuilder *b, const A0Tree *t, A0Node n) {
    size_t len;
    const char *text;
    if (is_kind(t, n, "binary")) {
        A0HirExpr x = vars_expr(b, t, a0_tree_field(t, n, A0_STR("left")));
        A0HirExpr y = vars_expr(b, t, a0_tree_field(t, n, A0_STR("right")));
        const char *op = a0_tree_text(t, a0_tree_field(t, n, A0_STR("op")), &len);
        at(b, t, n);
        return a0_hir_binary(b, op, len, x, y);
    }
    if (a0_tree_field(t, n, A0_STR("inner")) != A0_NONE) {
        return vars_expr(b, t, a0_tree_field(t, n, A0_STR("inner")));
    }
    at(b, t, n);
    text = a0_tree_text(t, n, &len);
    if (is_kind(t, a0_tree_child(t, n, 0), "NAME")) return a0_hir_name(b, text, len);
    {
        char digits[32];
        if (len >= sizeof digits) len = sizeof digits - 1;
        memcpy(digits, text, len);
        digits[len] = 0;
        return a0_hir_int(b, atoll(digits));
    }
}

static void vars(const A0Grammar *g, const char *source) {
    A0Diagnostics *d = NULL;
    A0Tree *t = a0_grammar_parse(g, source, strlen(source), &d);
    A0HirBuilder *b = a0_hir_builder_new(A0_STR("vars"));
    A0HirType int_t = a0_hir_named_type(b, A0_STR("int"), NULL, 0);
    A0HirFunc main_fn = a0_hir_declare_function(b, A0_STR("main"), NULL, 0);
    A0HirBody body;
    A0HirExpr stmts[64];
    size_t count = 0;
    uint32_t i;
    A0Node root = a0_tree_root(t);
    A0Hir *h;
    A0Module *m;
    int32_t status;
    check("parsing vars", d);
    a0_hir_set_signature(b, main_fn, NULL, NULL, 0, int_t);
    body = a0_hir_function_body(b, main_fn);
    a0_hir_begin_body(b, body);
    for (i = 0; i < a0_tree_child_count(t, root) && count < 64; i++) {
        A0Node s = a0_tree_child(t, root, i);
        A0HirExpr value;
        if (a0_tree_is_token(t, s)) continue;
        value = vars_expr(b, t, a0_tree_field(t, s, A0_STR("value")));
        at(b, t, s);
        if (is_kind(t, s, "let_stmt")) {
            size_t len;
            const char *name = a0_tree_text(t, a0_tree_field(t, s, A0_STR("name")), &len);
            stmts[count++] = a0_hir_let_named(b, name, len, a0_hir_fresh_type(b), value);
        } else {
            A0HirExpr line[2];
            line[0] = a0_hir_builtin(b, A0_STR("str"), &value, 1);
            line[1] = a0_hir_string(b, A0_STR("\n"));
            line[0] = a0_hir_binary(b, A0_STR("+"), line[0], line[1]);
            stmts[count++] = a0_hir_builtin(b, A0_STR("print"), line, 1);
        }
    }
    a0_hir_set_root(b, body, a0_hir_block(b, stmts, count, a0_hir_int(b, 0)));
    a0_hir_end_body(b);
    a0_tree_free(t);
    h = a0_hir_finish(b, &d);
    if (h == NULL) {
        size_t len, code_len;
        uint32_t start = 0;
        const char *message = a0_diagnostics_message(d, 0, &len);
        const char *code = a0_diagnostics_code(d, 0, &code_len);
        a0_diagnostics_span(d, 0, &start, NULL);
        printf("vars error at %u: [%.*s] %.*s\n", start, (int)code_len, code, (int)len, message);
        a0_diagnostics_free(d);
        return;
    }
    m = a0_hir_lower(h, &d);
    check("lowering vars", d);
    a0_hir_free(h);
    if (m == NULL) return;
    printf("vars:\n");
    fflush(stdout);
    a0_run(m, NULL, 0, &status, &d);
    check("running vars", d);
    a0_module_free(m);
}

/* ---- HIR built by handle: a struct, `if let` and a closure ---- */

/* print(str(e) + "\n") */
static A0HirExpr print_line(A0HirBuilder *b, A0HirExpr e) {
    A0HirExpr parts[2];
    parts[0] = a0_hir_builtin(b, A0_STR("str"), &e, 1);
    parts[1] = a0_hir_string(b, A0_STR("\n"));
    parts[0] = a0_hir_binary(b, A0_STR("+"), parts[0], parts[1]);
    return a0_hir_builtin(b, A0_STR("print"), parts, 1);
}

/* struct Box { value: int, next: Box? }
 * fn main() -> int {
 *   let inner = new Box { next: null, value: 2 };
 *   let outer = new Box { value: 40, next: inner };
 *   if let n = outer.next { print(str(n.value) + "\n"); }
 *   let add = fn(x: int) -> int { x + outer.value };
 *   print(str(add(2)) + "\n");
 *   0
 * } */
static void boxes(void) {
    A0HirBuilder *b = a0_hir_builder_new(A0_STR("boxes"));
    A0HirStruct box = a0_hir_declare_struct(b, A0_STR("Box"), NULL, 0);
    A0HirType int_t = a0_hir_named_type(b, A0_STR("int"), NULL, 0);
    A0HirType box_t = a0_hir_struct_type(b, box, NULL, 0);
    A0HirType maybe_box = a0_hir_nullable_type(b, box_t);
    A0HirFunc main_fn = a0_hir_declare_function(b, A0_STR("main"), NULL, 0);
    A0Str inner_fields[2] = {{A0_STR("next")}, {A0_STR("value")}};
    A0Str outer_fields[2] = {{A0_STR("value")}, {A0_STR("next")}};
    A0Str x_name[1] = {{A0_STR("x")}};
    A0HirExpr values[2], stmts[5], next, then;
    A0HirLocal n;
    A0HirBody body, closure;
    A0Diagnostics *d = NULL;
    A0Hir *h;
    A0Module *m;
    int32_t status;

    a0_hir_add_field(b, box, A0_STR("value"), int_t);
    a0_hir_add_field(b, box, A0_STR("next"), maybe_box);
    a0_hir_set_signature(b, main_fn, NULL, NULL, 0, int_t);
    body = a0_hir_function_body(b, main_fn);
    a0_hir_begin_body(b, body);

    values[0] = a0_hir_null(b, maybe_box);
    values[1] = a0_hir_int(b, 2);
    stmts[0] = a0_hir_let_named(b, A0_STR("inner"), a0_hir_fresh_type(b),
                                a0_hir_new_struct_named(b, box, inner_fields, values, 2));
    values[0] = a0_hir_int(b, 40);
    values[1] = a0_hir_name(b, A0_STR("inner"));
    stmts[1] = a0_hir_let_named(b, A0_STR("outer"), a0_hir_fresh_type(b),
                                a0_hir_new_struct_named(b, box, outer_fields, values, 2));

    next = a0_hir_field_named(b, a0_hir_name(b, A0_STR("outer")), A0_STR("next"));
    a0_hir_push_scope(b);
    n = a0_hir_declare_if_let(b, A0_STR("n"), next);
    then = print_line(b, a0_hir_field_named(b, a0_hir_name(b, A0_STR("n")), A0_STR("value")));
    a0_hir_pop_scope(b);
    stmts[2] = a0_hir_if_let(b, n, next, then, A0_NONE);

    closure = a0_hir_new_closure_body(b, body, x_name, &int_t, 1, int_t);
    a0_hir_begin_body(b, closure);
    a0_hir_set_root(b, closure,
                    a0_hir_binary(b, A0_STR("+"), a0_hir_name(b, A0_STR("x")),
                                  a0_hir_field_named(b, a0_hir_name(b, A0_STR("outer")),
                                                     A0_STR("value"))));
    a0_hir_end_body(b);
    stmts[3] = a0_hir_let_named(b, A0_STR("add"), a0_hir_fresh_type(b), a0_hir_lambda(b, closure));
    values[0] = a0_hir_int(b, 2);
    stmts[4] = print_line(b, a0_hir_call_named(b, A0_STR("add"), values, 1));

    a0_hir_set_root(b, body, a0_hir_block(b, stmts, 5, a0_hir_int(b, 0)));
    a0_hir_end_body(b);
    h = a0_hir_finish(b, &d);
    check("building boxes", d);
    if (h == NULL) return;
    m = a0_hir_lower(h, &d);
    check("lowering boxes", d);
    a0_hir_free(h);
    if (m == NULL) return;
    fflush(stdout);
    a0_run(m, NULL, 0, &status, &d);
    check("running boxes", d);
    a0_module_free(m);
}

/* fn twice(x: int) -> int { x * 2 } in module `lib`, used from the root module as `lib.twice`;
 * `lib.hidden`, which is private, is reported. */
static void modules(void) {
    A0HirBuilder *b = a0_hir_builder_new(A0_STR("modules"));
    A0HirType int_t = a0_hir_named_type(b, A0_STR("int"), NULL, 0);
    A0Str x_name[1] = {{A0_STR("x")}};
    A0HirFunc f;
    A0HirBody body;
    A0HirExpr arg, call, stmts[1];
    A0Diagnostics *d = NULL;
    A0Hir *h;
    A0Module *m;
    int32_t status;
    size_t len;
    const char *message;

    a0_hir_begin_module(b, A0_STR("lib"), 0);
    f = a0_hir_declare_function(b, A0_STR("twice"), NULL, 0);
    a0_hir_set_signature(b, f, x_name, &int_t, 1, int_t);
    body = a0_hir_function_body(b, f);
    a0_hir_begin_body(b, body);
    a0_hir_set_root(b, body,
                    a0_hir_binary(b, A0_STR("*"), a0_hir_name(b, A0_STR("x")), a0_hir_int(b, 2)));
    a0_hir_end_body(b);
    a0_hir_set_public(b, 0);
    f = a0_hir_declare_function(b, A0_STR("hidden"), NULL, 0);
    a0_hir_set_signature(b, f, NULL, NULL, 0, int_t);
    body = a0_hir_function_body(b, f);
    a0_hir_begin_body(b, body);
    a0_hir_set_root(b, body, a0_hir_int(b, 1));
    a0_hir_end_body(b);

    a0_hir_begin_module(b, A0_STR("main"), 1);
    a0_hir_import_module(b, A0_STR("lib"), A0_STR("lib"));
    f = a0_hir_declare_function(b, A0_STR("main"), NULL, 0);
    a0_hir_set_signature(b, f, NULL, NULL, 0, int_t);
    body = a0_hir_function_body(b, f);
    a0_hir_begin_body(b, body);
    arg = a0_hir_int(b, 21);
    call = a0_hir_call_member(b, A0_STR("lib"), A0_STR("twice"), &arg, 1);
    stmts[0] = print_line(b, call);
    a0_hir_set_root(b, body, a0_hir_block(b, stmts, 1, a0_hir_int(b, 0)));
    a0_hir_end_body(b);
    printf("modules: lib is a module: %d\n", a0_hir_is_module(b, A0_STR("lib")));
    h = a0_hir_finish(b, &d);
    check("building modules", d);
    if (h == NULL) return;
    m = a0_hir_lower(h, &d);
    check("lowering modules", d);
    a0_hir_free(h);
    if (m == NULL) return;
    fflush(stdout);
    a0_run(m, NULL, 0, &status, &d);
    check("running modules", d);
    a0_module_free(m);

    b = a0_hir_builder_new(A0_STR("private"));
    int_t = a0_hir_named_type(b, A0_STR("int"), NULL, 0);
    a0_hir_begin_module(b, A0_STR("lib"), 0);
    a0_hir_set_public(b, 0);
    f = a0_hir_declare_function(b, A0_STR("hidden"), NULL, 0);
    a0_hir_set_signature(b, f, NULL, NULL, 0, int_t);
    body = a0_hir_function_body(b, f);
    a0_hir_begin_body(b, body);
    a0_hir_set_root(b, body, a0_hir_int(b, 1));
    a0_hir_end_body(b);
    a0_hir_begin_module(b, A0_STR("main"), 1);
    a0_hir_import_module(b, A0_STR("lib"), A0_STR("lib"));
    f = a0_hir_declare_function(b, A0_STR("main"), NULL, 0);
    a0_hir_set_signature(b, f, NULL, NULL, 0, int_t);
    body = a0_hir_function_body(b, f);
    a0_hir_begin_body(b, body);
    a0_hir_set_root(b, body, a0_hir_call_member(b, A0_STR("lib"), A0_STR("hidden"), NULL, 0));
    a0_hir_end_body(b);
    if (a0_hir_finish(b, &d) != NULL || d == NULL) {
        printf("a private item was not reported\n");
        failures++;
        return;
    }
    message = a0_diagnostics_message(d, 0, &len);
    printf("modules: %.*s\n", (int)len, message);
    a0_diagnostics_free(d);
}

/* Bad handles and calls out of order come back as diagnostics, not crashes. */
static void hir_misuse(void) {
    A0HirBuilder *b = a0_hir_builder_new(A0_STR("bad"));
    A0Diagnostics *d = NULL;
    size_t len;
    const char *message;
    a0_hir_binary(b, A0_STR("+"), a0_hir_int(b, 1), 99999);
    a0_hir_declare_local(b, A0_STR("x"), a0_hir_named_type(b, A0_STR("int"), NULL, 0));
    if (a0_hir_finish(b, &d) != NULL || d == NULL) {
        printf("HIR misuse was not reported\n");
        failures++;
        return;
    }
    message = a0_diagnostics_message(d, 0, &len);
    printf("hir misuse: %d problems, first: %.*s\n", (int)a0_diagnostics_count(d), (int)len,
           message);
    a0_diagnostics_free(d);
}

/* ---- The high-level IR, from text ---- */

static const char GREET[] =
    "module \"greet\";\n"
    "fn greet(name: string) -> string {\n"
    "  \"hello, \" + name + \" from HIR\"\n"
    "}\n"
    "fn main() -> int {\n"
    "  print(greet(\"C\") + \"\\n\");\n"
    "  0\n"
    "}\n";

int main(void) {
    A0Diagnostics *d = NULL;
    A0Module *m, *again;
    A0Grammar *g;
    A0Hir *h;
    const char *text;
    const char *args[1] = {"fib"};
    size_t len;
    int32_t status = -1;
    int r;

    printf("a0 %s\n", a0_version());

    m = build_fib();
    if (m == NULL) return 1;
    text = a0_module_text(m, &len);
    printf("text: %s\n", contains(text, len, "func @fib(%0: i64) -> i64") ? "has @fib" : "no @fib");
    again = a0_module_parse(text, len, &d);
    check("parsing the text", d);

    fflush(stdout);
    r = a0_run(again, args, 1, &status, &d);
    check("running fib", d);
    printf("run: %d, status %d\n", r, (int)status);

    if (a0_write_object(m, A0_STR("aarch64-linux"), A0_STR("fib-aarch64-linux.o"), &d) == 0) {
        printf("object: fib-aarch64-linux.o\n");
    }
    check("writing an object", d);
    if (a0_build_executable(m, A0_STR(EXE), &d) == 0) printf("executable: %s\n", EXE);
    check("building an executable", d);
    a0_module_free(again);
    a0_module_free(m);

    misuse();

    g = a0_grammar_new(CALC, sizeof CALC - 1, &d);
    check("loading the calc grammar", d);
    if (g != NULL) {
        calc(g, "1 + 2 * (10 - 3)");
        calc(g, "6 * 7");
        calc(g, "1 +");
        a0_grammar_free(g);
    }

    h = a0_hir_parse(GREET, sizeof GREET - 1, &d);
    check("parsing HIR", d);
    if (h != NULL) {
        m = a0_hir_lower(h, &d);
        check("lowering HIR", d);
        if (m != NULL) {
            fflush(stdout);
            a0_run(m, NULL, 0, &status, &d);
            check("running HIR", d);
            a0_module_free(m);
        }
        a0_hir_free(h);
    }

    g = a0_grammar_new(VARS, sizeof VARS - 1, &d);
    check("loading the vars grammar", d);
    if (g != NULL) {
        vars(g, "let x = 3 + 4; let y = x * 3; print x; print y;");
        vars(g, "let a = 1; print b;");
        a0_grammar_free(g);
    }
    boxes();
    modules();
    hir_misuse();

    printf("failures: %d\n", failures);
    return failures == 0 ? 0 : 1;
}
