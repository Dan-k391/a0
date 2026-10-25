/* A0's C API: build, run and compile modules from C. Link with the a0_capi static library.
 *
 * Conventions (decision D2, docs/c-api.md):
 * - Objects (A0Builder, A0Module, ...) are opaque, created by A0 and freed with their a0_*_free.
 * - Things inside an object (values, functions, variables, globals, tree nodes) are uint32_t
 *   handles; A0_NONE stands for "none" (the result of a void call, a missing child).
 * - Strings are passed as pointer and length (A0_STR builds both from a literal) and need not be
 *   NUL-terminated. Strings A0 returns borrow from the object they came from: valid until that
 *   object changes or is freed, and not NUL-terminated either.
 * - Functions that can fail take an `A0Diagnostics **diags`. On failure they set it to the errors
 *   (free them with a0_diagnostics_free); on success they set it to NULL. It may be NULL.
 * - Misusing a builder is not undefined behavior: the mistake is recorded and reported by
 *   a0_builder_finish. Passing invalid pointers is undefined behavior. */
#ifndef A0_H
#define A0_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define A0_STR(s) (s), (sizeof(s) - 1)
#define A0_NONE UINT32_MAX

typedef struct A0Diagnostics A0Diagnostics;
typedef struct A0Builder A0Builder;
typedef struct A0Module A0Module;
typedef struct A0Grammar A0Grammar;
typedef struct A0Tree A0Tree;
typedef struct A0Hir A0Hir;

typedef uint32_t A0Value;
typedef uint32_t A0Func;
typedef uint32_t A0Var;
typedef uint32_t A0Global;
typedef uint32_t A0Node;

/* Core IR types. A0_VOID is "no type": a function that returns nothing. */
typedef enum A0Type {
    A0_VOID = 0,
    A0_I1 = 1,
    A0_I8 = 2,
    A0_I16 = 3,
    A0_I32 = 4,
    A0_I64 = 5,
    A0_F32 = 6,
    A0_F64 = 7,
    A0_PTR = 8
} A0Type;

/* The library's version, NUL-terminated. */
const char *a0_version(void);

/* ---- Diagnostics -------------------------------------------------------------------------- */

size_t a0_diagnostics_count(const A0Diagnostics *d);
const char *a0_diagnostics_message(const A0Diagnostics *d, size_t i, size_t *len);
/* Where diagnostic i is, as byte offsets into the text it is about. Returns 0 if it has no place. */
int a0_diagnostics_span(const A0Diagnostics *d, size_t i, uint32_t *start, uint32_t *end);
/* All diagnostics, one per line ("error[code]: message"). */
const char *a0_diagnostics_text(const A0Diagnostics *d, size_t *len);
void a0_diagnostics_free(A0Diagnostics *d);

/* ---- Building core IR (docs/ir.md) -------------------------------------------------------- */

A0Builder *a0_builder_new(const char *module_name, size_t len);
/* Declares a function defined elsewhere (the runtime, the C library). */
A0Func a0_builder_declare_extern(A0Builder *b, const char *name, size_t len, const A0Type *params,
                                 size_t param_count, A0Type ret);
/* Declares a function to define with a0_builder_begin_function, in any order. */
A0Func a0_builder_declare_function(A0Builder *b, const char *name, size_t len,
                                   const A0Type *params, size_t param_count, A0Type ret);
A0Global a0_builder_add_data(A0Builder *b, const char *name, size_t len, const uint8_t *bytes,
                             size_t byte_count);
A0Global a0_builder_add_global_int(A0Builder *b, const char *name, size_t len, A0Type type,
                                   int64_t init);
A0Global a0_builder_add_global_float(A0Builder *b, const char *name, size_t len, A0Type type,
                                     double init);

void a0_builder_begin_function(A0Builder *b, A0Func f);
void a0_builder_end_function(A0Builder *b);
A0Value a0_builder_param(A0Builder *b, uint32_t i);

/* Mutable variables, so that code needs no phi nodes. */
A0Var a0_builder_declare_var(A0Builder *b, const char *name, size_t len, A0Type type);
A0Value a0_builder_get(A0Builder *b, A0Var v);
void a0_builder_set(A0Builder *b, A0Var v, A0Value value);

A0Value a0_builder_iconst(A0Builder *b, A0Type type, int64_t value);
A0Value a0_builder_fconst(A0Builder *b, A0Type type, double value);
A0Value a0_builder_null(A0Builder *b);
A0Value a0_builder_func_addr(A0Builder *b, A0Func f);
A0Value a0_builder_global_addr(A0Builder *b, A0Global g);

/* `op` is an instruction's name in the text format: "iadd", "sdiv", "fmul", "and", "shl", ...
 * (binary), "ineg", "not", "fneg" (unary), "zext", "sitofp", "bitcast", ... (casts). */
A0Value a0_builder_binary(A0Builder *b, const char *op, size_t len, A0Value x, A0Value y);
A0Value a0_builder_unary(A0Builder *b, const char *op, size_t len, A0Value x);
A0Value a0_builder_cast(A0Builder *b, const char *op, size_t len, A0Value x, A0Type to);
/* `cond` as in the text format: "eq", "ne", "slt", "sle", "sgt", "sge", "ult", ... for icmp;
 * "eq", "ne", "lt", "le", "gt", "ge" for fcmp. The result is an A0_I1. */
A0Value a0_builder_icmp(A0Builder *b, const char *cond, size_t len, A0Value x, A0Value y);
A0Value a0_builder_fcmp(A0Builder *b, const char *cond, size_t len, A0Value x, A0Value y);
A0Value a0_builder_select(A0Builder *b, A0Value cond, A0Value x, A0Value y);

A0Value a0_builder_load(A0Builder *b, A0Type type, A0Value ptr);
void a0_builder_store(A0Builder *b, A0Value value, A0Value ptr);
A0Value a0_builder_ptradd(A0Builder *b, A0Value ptr, A0Value offset);

/* Returns A0_NONE for a function that returns nothing. */
A0Value a0_builder_call(A0Builder *b, A0Func f, const A0Value *args, size_t arg_count);
A0Value a0_builder_call_indirect(A0Builder *b, A0Value callee, const A0Type *params,
                                 size_t param_count, A0Type ret, const A0Value *args,
                                 size_t arg_count);

/* Structured control flow: matched if/else/end_if and loop/end_loop. */
void a0_builder_if(A0Builder *b, A0Value cond);
void a0_builder_else(A0Builder *b);
void a0_builder_end_if(A0Builder *b);
void a0_builder_loop(A0Builder *b);
void a0_builder_end_loop(A0Builder *b);
void a0_builder_break(A0Builder *b);
void a0_builder_continue(A0Builder *b);
/* `value` is A0_NONE in a function that returns nothing. */
void a0_builder_return(A0Builder *b, A0Value value);
void a0_builder_unreachable(A0Builder *b);

/* Verifies the module and returns it, or NULL with *diags. Frees the builder either way. */
A0Module *a0_builder_finish(A0Builder *b, A0Diagnostics **diags);

/* ---- Modules ------------------------------------------------------------------------------ */

/* Parses and verifies a module in the text format. */
A0Module *a0_module_parse(const char *text, size_t len, A0Diagnostics **diags);
/* The module in the text format. */
const char *a0_module_text(A0Module *m, size_t *len);
void a0_module_free(A0Module *m);

/* Interprets @main with the given arguments (args[0] is the program's name). Returns 0 with the
 * exit status in *status; 1 if the program trapped or panicked (*status is 101 and *diags says
 * why); -1 if it could not run (*diags). Output is flushed before returning. */
int a0_run(const A0Module *m, const char *const *args, size_t arg_count, int32_t *status,
           A0Diagnostics **diags);

/* Compiles to an object file for `target` ("x86_64-linux", "aarch64-macos", ...; an empty name
 * means the host). Returns 0 on success, -1 with *diags. */
int a0_write_object(const A0Module *m, const char *target, size_t target_len, const char *path,
                    size_t path_len, A0Diagnostics **diags);
/* Compiles and links an executable for the host with the system linker. */
int a0_build_executable(const A0Module *m, const char *path, size_t path_len,
                        A0Diagnostics **diags);

/* ---- Grammars and syntax trees (docs/grammar.md) ------------------------------------------ */

A0Grammar *a0_grammar_new(const char *text, size_t len, A0Diagnostics **diags);
void a0_grammar_free(A0Grammar *g);
/* Always returns a tree; syntax errors appear in it as `error` nodes and in *diags. */
A0Tree *a0_grammar_parse(const A0Grammar *g, const char *text, size_t len, A0Diagnostics **diags);
void a0_tree_free(A0Tree *t);

A0Node a0_tree_root(const A0Tree *t);
/* The rule or token kind: "binary", "NUMBER", "\"+\"". */
const char *a0_tree_kind(const A0Tree *t, A0Node n, size_t *len);
/* The source text the node covers. */
const char *a0_tree_text(const A0Tree *t, A0Node n, size_t *len);
int a0_tree_is_token(const A0Tree *t, A0Node n);
uint32_t a0_tree_child_count(const A0Tree *t, A0Node n);
A0Node a0_tree_child(const A0Tree *t, A0Node n, uint32_t i);
/* The field label of child i, or NULL if it has none. */
const char *a0_tree_child_field(const A0Tree *t, A0Node n, uint32_t i, size_t *len);
/* The first child labeled `field`, or A0_NONE. */
A0Node a0_tree_field(const A0Tree *t, A0Node n, const char *field, size_t len);

/* ---- The high-level IR (docs/hir.md) ------------------------------------------------------ */

/* Parses and checks a HIR module in the text format. */
A0Hir *a0_hir_parse(const char *text, size_t len, A0Diagnostics **diags);
/* Lowers (and optimizes) a HIR module with a `main` to core IR. */
A0Module *a0_hir_lower(const A0Hir *h, A0Diagnostics **diags);
void a0_hir_free(A0Hir *h);

#ifdef __cplusplus
}
#endif

#endif
