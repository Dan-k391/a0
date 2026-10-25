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
/* Diagnostic i's code (E2001; `a0 explain` explains it), empty if it has none. */
const char *a0_diagnostics_code(const A0Diagnostics *d, size_t i, size_t *len);
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
/* break and continue for the loop `depth` loops out from the innermost (0 is the innermost). */
void a0_builder_break_out(A0Builder *b, uint32_t depth);
void a0_builder_continue_out(A0Builder *b, uint32_t depth);
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
/* The byte range of the source the node covers. Returns 0 for an invalid node. */
int a0_tree_span(const A0Tree *t, A0Node n, uint32_t *start, uint32_t *end);
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
/* Sets *warnings to the warnings found when the module was built (a0_hir_enable_warning), or to
 * NULL if there are none. */
void a0_hir_warnings(const A0Hir *h, A0Diagnostics **warnings);

/* ---- Building HIR (docs/hir.md, "Building HIR from a frontend") --------------------------- *
 *
 * The builder infers and checks types and, through the by-name functions, resolves names, so a
 * frontend needs no checker of its own. Handles: types, expressions, locals, structs, enums,
 * functions, globals, externs, bodies and match arms are all uint32_t. Every handle is checked;
 * a bad one, an unknown operator or builtin, or any type error is reported by a0_hir_finish.
 * Spans set with a0_hir_set_span are byte offsets into the frontend's source text and appear in
 * the diagnostics. */

typedef struct A0HirBuilder A0HirBuilder;
typedef uint32_t A0HirType;
typedef uint32_t A0HirExpr;
typedef uint32_t A0HirLocal;
typedef uint32_t A0HirStruct;
typedef uint32_t A0HirEnum;
typedef uint32_t A0HirFunc;
typedef uint32_t A0HirGlobal;
typedef uint32_t A0HirExtern;
typedef uint32_t A0HirBody;
typedef uint32_t A0HirArm;

/* A string in an array of names: { A0_STR("x") }. */
typedef struct A0Str {
    const char *ptr;
    size_t len;
} A0Str;

/* What an item handle refers to (a0_hir_type_param, a0_hir_lookup_*). */
enum { A0_HIR_STRUCT = 1, A0_HIR_ENUM = 2, A0_HIR_FUNC = 3, A0_HIR_GLOBAL = 4, A0_HIR_EXTERN = 5 };

A0HirBuilder *a0_hir_builder_new(const char *module_name, size_t len);
/* Continues building a parsed module (a prelude written in HIR text). Consumes `h`. */
A0HirBuilder *a0_hir_builder_from(A0Hir *h);
/* Frees a builder that will not be finished. */
void a0_hir_builder_free(A0HirBuilder *b);
/* Nodes built from now on are reported at start..end of the source. */
void a0_hir_set_span(A0HirBuilder *b, uint32_t start, uint32_t end);
void a0_hir_clear_span(A0HirBuilder *b);
/* Checks the module and returns it, or NULL with *diags. Frees the builder either way. */
A0Hir *a0_hir_finish(A0HirBuilder *b, A0Diagnostics **diags);

/* Types. "unit", "bool", "int", "float", "string", "any" and the names of declared structs and
 * enums are named types; `args` are type arguments (none: inferred). */
A0HirType a0_hir_named_type(A0HirBuilder *b, const char *name, size_t len, const A0HirType *args,
                            size_t arg_count);
/* A type to infer from how it is used. */
A0HirType a0_hir_fresh_type(A0HirBuilder *b);
A0HirType a0_hir_array_type(A0HirBuilder *b, A0HirType elem);
A0HirType a0_hir_map_type(A0HirBuilder *b, A0HirType key, A0HirType value);
A0HirType a0_hir_fn_type(A0HirBuilder *b, const A0HirType *params, size_t param_count,
                         A0HirType ret);
/* T?, a reference that may be null. */
A0HirType a0_hir_nullable_type(A0HirBuilder *b, A0HirType t);
A0HirType a0_hir_struct_type(A0HirBuilder *b, A0HirStruct s, const A0HirType *args,
                             size_t arg_count);
A0HirType a0_hir_enum_type(A0HirBuilder *b, A0HirEnum e, const A0HirType *args, size_t arg_count);
/* Type parameter `index` of a generic struct, enum or function. */
A0HirType a0_hir_type_param(A0HirBuilder *b, uint32_t item_kind, uint32_t item, uint32_t index);
A0HirType a0_hir_type_of(A0HirBuilder *b, A0HirExpr e);
/* How diagnostics write a kind of type in the language's syntax: ("array", "[{}]"). */
void a0_hir_set_type_syntax(A0HirBuilder *b, const char *tag, size_t tag_len, const char *template_,
                            size_t template_len);
/* What diagnostics call a concept, in the language's words: ("variant", "case"). The concepts are
 * function, closure, struct, enum, variant, field, variable, null, unwrap, narrow, match, break
 * and continue. */
void a0_hir_set_term(A0HirBuilder *b, const char *term, size_t term_len, const char *text,
                     size_t text_len);
/* Turns a warning on: "unused_variable" or "unreachable_code". A finished module's warnings are
 * read with a0_hir_warnings. */
void a0_hir_enable_warning(A0HirBuilder *b, const char *warning, size_t len);
/* Where the name is in the next node built by name, so that a misspelled name gets a fix. */
void a0_hir_set_name_span(A0HirBuilder *b, uint32_t start, uint32_t end);
/* Where parameter `index` of `body` is declared, for messages about arguments. */
void a0_hir_set_param_span(A0HirBuilder *b, A0HirBody body, uint32_t index, uint32_t start,
                           uint32_t end);
/* The frontend's own error at the current span, with a code (a0_hir_report gives none). The
 * functions after it add to the error reported last: a related location, a note, a fix replacing
 * start..end. */
void a0_hir_report_code(A0HirBuilder *b, const char *code, size_t code_len, const char *message,
                        size_t len);
void a0_hir_report_label(A0HirBuilder *b, uint32_t start, uint32_t end, const char *message,
                         size_t len);
void a0_hir_report_note(A0HirBuilder *b, const char *note, size_t len);
void a0_hir_report_fix(A0HirBuilder *b, const char *message, size_t len, uint32_t start,
                       uint32_t end, const char *replacement, size_t replacement_len);

/* Declarations, in any order. Type parameters are names; none for a plain item. */
A0HirStruct a0_hir_declare_struct(A0HirBuilder *b, const char *name, size_t len,
                                  const A0Str *type_params, size_t type_param_count);
/* Returns the field's index. */
uint32_t a0_hir_add_field(A0HirBuilder *b, A0HirStruct s, const char *name, size_t len,
                          A0HirType type);
A0HirEnum a0_hir_declare_enum(A0HirBuilder *b, const char *name, size_t len,
                              const A0Str *type_params, size_t type_param_count);
/* Returns the variant's index. */
uint32_t a0_hir_add_variant(A0HirBuilder *b, A0HirEnum e, const char *name, size_t len,
                            const A0HirType *fields, size_t field_count);
A0HirGlobal a0_hir_declare_global(A0HirBuilder *b, const char *name, size_t len, A0HirType type);
/* A C function: it takes int, float, bool, strings (as const char *) and arrays of int, float or
 * bool (as pointers to their 64-bit elements), and returns one of the first three, unit or a
 * string (copied). */
A0HirExtern a0_hir_declare_extern(A0HirBuilder *b, const char *name, size_t len,
                                  const A0HirType *params, size_t param_count, A0HirType ret);
/* A C function called through `abi`: "C" (as above) or "a0", a runtime function that takes and
 * returns A0's own values (A0String *, A0Array *, ...; see a0rt.h). */
A0HirExtern a0_hir_declare_extern_abi(A0HirBuilder *b, const char *name, size_t len,
                                      const A0HirType *params, size_t param_count, A0HirType ret,
                                      const char *abi, size_t abi_len);
/* A function; give it its signature with a0_hir_set_signature (which may use its type
 * parameters) and its code through a0_hir_function_body. */
A0HirFunc a0_hir_declare_function(A0HirBuilder *b, const char *name, size_t len,
                                  const A0Str *type_params, size_t type_param_count);
void a0_hir_set_signature(A0HirBuilder *b, A0HirFunc f, const A0Str *param_names,
                          const A0HirType *param_types, size_t param_count, A0HirType ret);
A0HirBody a0_hir_function_body(A0HirBuilder *b, A0HirFunc f);
/* The body of a closure created inside `parent` (make the closure with a0_hir_lambda). */
A0HirBody a0_hir_new_closure_body(A0HirBuilder *b, A0HirBody parent, const A0Str *param_names,
                                  const A0HirType *param_types, size_t param_count, A0HirType ret);
A0HirLocal a0_hir_param(A0HirBuilder *b, A0HirBody body, uint32_t i);
A0HirLocal a0_hir_new_local(A0HirBuilder *b, A0HirBody body, const char *name, size_t len,
                            A0HirType type);
/* The body's code: an expression, usually a block. */
void a0_hir_set_root(A0HirBuilder *b, A0HirBody body, A0HirExpr root);
/* The item a type or value name visible in the current module refers to, with its kind in *kind;
 * A0_NONE if there is none. */
uint32_t a0_hir_lookup_type(A0HirBuilder *b, const char *name, size_t len, uint32_t *kind);
uint32_t a0_hir_lookup_value(A0HirBuilder *b, const char *name, size_t len, uint32_t *kind);

/* Expressions. Optional operands (an if without else, a return without value, a block without
 * a value) are A0_NONE. */
A0HirExpr a0_hir_unit(A0HirBuilder *b);
A0HirExpr a0_hir_bool(A0HirBuilder *b, int value);
A0HirExpr a0_hir_int(A0HirBuilder *b, int64_t value);
A0HirExpr a0_hir_float(A0HirBuilder *b, double value);
A0HirExpr a0_hir_string(A0HirBuilder *b, const char *s, size_t len);
/* null of a nullable type. */
A0HirExpr a0_hir_null(A0HirBuilder *b, A0HirType type);
A0HirExpr a0_hir_local(A0HirBuilder *b, A0HirLocal l);
A0HirExpr a0_hir_global(A0HirBuilder *b, A0HirGlobal g);
/* A function as a value (type arguments for a generic one, or none to infer them). */
A0HirExpr a0_hir_func(A0HirBuilder *b, A0HirFunc f, const A0HirType *type_args, size_t count);
/* `let l = init` (a statement of a block). */
A0HirExpr a0_hir_let(A0HirBuilder *b, A0HirLocal l, A0HirExpr init);
A0HirExpr a0_hir_set_local(A0HirBuilder *b, A0HirLocal l, A0HirExpr value);
A0HirExpr a0_hir_set_global(A0HirBuilder *b, A0HirGlobal g, A0HirExpr value);
A0HirExpr a0_hir_set_field(A0HirBuilder *b, A0HirExpr object, uint32_t field, A0HirExpr value);
A0HirExpr a0_hir_set_field_named(A0HirBuilder *b, A0HirExpr object, const char *name, size_t len,
                                 A0HirExpr value);
A0HirExpr a0_hir_set_index(A0HirBuilder *b, A0HirExpr target, A0HirExpr index, A0HirExpr value);
/* "-" or "!". */
A0HirExpr a0_hir_unary(A0HirBuilder *b, const char *op, size_t len, A0HirExpr x);
/* "+", "-", "*", "/", "%", "<", "<=", ">", ">=", "==", "!=", "&&", "||". */
A0HirExpr a0_hir_binary(A0HirBuilder *b, const char *op, size_t len, A0HirExpr x, A0HirExpr y);
A0HirExpr a0_hir_call(A0HirBuilder *b, A0HirFunc f, const A0HirType *type_args, size_t type_count,
                      const A0HirExpr *args, size_t arg_count);
A0HirExpr a0_hir_call_extern(A0HirBuilder *b, A0HirExtern x, const A0HirExpr *args,
                             size_t arg_count);
/* Calls a function value (a closure). */
A0HirExpr a0_hir_call_value(A0HirBuilder *b, A0HirExpr callee, const A0HirExpr *args,
                            size_t arg_count);
/* A closure over a body from a0_hir_new_closure_body. */
A0HirExpr a0_hir_lambda(A0HirBuilder *b, A0HirBody body);
/* A struct value with every field in declaration order. */
A0HirExpr a0_hir_new_struct(A0HirBuilder *b, A0HirStruct s, const A0HirType *type_args,
                            size_t type_count, const A0HirExpr *fields, size_t field_count);
A0HirExpr a0_hir_field(A0HirBuilder *b, A0HirExpr object, uint32_t field);
A0HirExpr a0_hir_field_named(A0HirBuilder *b, A0HirExpr object, const char *name, size_t len);
A0HirExpr a0_hir_variant(A0HirBuilder *b, A0HirEnum e, const A0HirType *type_args,
                         size_t type_count, uint32_t variant, const A0HirExpr *args,
                         size_t arg_count);
/* A match arm: `variant` (A0_NONE for `_`), locals bound to its payload, the arm's value. */
A0HirArm a0_hir_arm(A0HirBuilder *b, uint32_t variant, const A0HirLocal *binds, size_t bind_count,
                    A0HirExpr body);
A0HirExpr a0_hir_match(A0HirBuilder *b, A0HirExpr scrutinee, const A0HirArm *arms,
                       size_t arm_count);
A0HirExpr a0_hir_array(A0HirBuilder *b, const A0HirExpr *elems, size_t count);
A0HirExpr a0_hir_new_array(A0HirBuilder *b, A0HirType array_type);
A0HirExpr a0_hir_new_map(A0HirBuilder *b, A0HirType map_type);
A0HirExpr a0_hir_index(A0HirBuilder *b, A0HirExpr target, A0HirExpr index);
/* "len", "push", "pop", "has", "remove", "keys", "substr", "byte_at", "str", "print", "eprint",
 * "exit", "panic", "clock" (docs/hir.md). */
A0HirExpr a0_hir_builtin(A0HirBuilder *b, const char *name, size_t len, const A0HirExpr *args,
                         size_t arg_count);
/* `e as type`: conversions between int and float, and into and out of any. */
A0HirExpr a0_hir_cast(A0HirBuilder *b, A0HirExpr e, A0HirType type);
/* `e!`: the value of a nullable reference; stops the program if it is null. */
A0HirExpr a0_hir_unwrap(A0HirBuilder *b, A0HirExpr e);
/* `e is type`, for e of type any. */
A0HirExpr a0_hir_is(A0HirBuilder *b, A0HirExpr e, A0HirType type);
A0HirExpr a0_hir_if(A0HirBuilder *b, A0HirExpr cond, A0HirExpr then, A0HirExpr els);
/* `if let l = value { then } else { els }`: `l` holds `value` in `then` when it is not null. */
A0HirExpr a0_hir_if_let(A0HirBuilder *b, A0HirLocal l, A0HirExpr value, A0HirExpr then,
                        A0HirExpr els);
A0HirExpr a0_hir_while(A0HirBuilder *b, A0HirExpr cond, A0HirExpr body);
A0HirExpr a0_hir_loop(A0HirBuilder *b, A0HirExpr body);
/* Exceptions: `throw value` (any type), and `try { body } catch l { handler }` with `l` (from
 * a0_hir_declare_catch, an `any`) holding the thrown value in `handler`. */
A0HirExpr a0_hir_throw(A0HirBuilder *b, A0HirExpr value);
A0HirExpr a0_hir_try_catch(A0HirBuilder *b, A0HirExpr body, A0HirLocal l, A0HirExpr handler);
/* `e?`: the value of a nullable or a result (an enum whose first variant holds it), or else returns
 * the null or the failure from the function. */
A0HirExpr a0_hir_propagate(A0HirBuilder *b, A0HirExpr e);
A0HirExpr a0_hir_break(A0HirBuilder *b);
A0HirExpr a0_hir_continue(A0HirBuilder *b);
A0HirExpr a0_hir_return(A0HirBuilder *b, A0HirExpr value);
/* Statements, then an optional value. */
A0HirExpr a0_hir_block(A0HirBuilder *b, const A0HirExpr *items, size_t count, A0HirExpr tail);

/* By name: scopes and name resolution, for frontends that leave them to A0. */
/* Puts a body's parameters (and type parameters) in scope until a0_hir_end_body. */
void a0_hir_begin_body(A0HirBuilder *b, A0HirBody body);
void a0_hir_end_body(A0HirBuilder *b);
void a0_hir_push_scope(A0HirBuilder *b);
void a0_hir_pop_scope(A0HirBuilder *b);
int a0_hir_is_declared_in_scope(A0HirBuilder *b, const char *name, size_t len);
A0HirLocal a0_hir_declare_local(A0HirBuilder *b, const char *name, size_t len, A0HirType type);
/* The binding of `if let name = value`, typed from the value; push a scope for it first. */
A0HirLocal a0_hir_declare_if_let(A0HirBuilder *b, const char *name, size_t len, A0HirExpr value);
/* The binding of `catch name`, an `any`; push a scope for it first. */
A0HirLocal a0_hir_declare_catch(A0HirBuilder *b, const char *name, size_t len);
/* `let name: type = value` with `name` in scope afterwards; `type` may be a0_hir_fresh_type. */
A0HirExpr a0_hir_let_named(A0HirBuilder *b, const char *name, size_t len, A0HirType type,
                           A0HirExpr value);
A0HirLocal a0_hir_lookup_local(A0HirBuilder *b, const char *name, size_t len);
/* A local, global or function by name; an unknown name is reported. */
A0HirExpr a0_hir_name(A0HirBuilder *b, const char *name, size_t len);
/* Calls a function, closure or builtin by name. */
A0HirExpr a0_hir_call_named(A0HirBuilder *b, const char *name, size_t len, const A0HirExpr *args,
                            size_t arg_count);
A0HirExpr a0_hir_assign_named(A0HirBuilder *b, const char *name, size_t len, A0HirExpr value);
/* Puts a generic item's type parameters in scope for a0_hir_named_type. */
void a0_hir_bind_type_params(A0HirBuilder *b, uint32_t item_kind, uint32_t item);
/* A struct value with fields given by name, in any order. */
A0HirExpr a0_hir_new_struct_named(A0HirBuilder *b, A0HirStruct s, const A0Str *field_names,
                                  const A0HirExpr *values, size_t count);
A0HirExpr a0_hir_variant_named(A0HirBuilder *b, A0HirEnum e, const char *name, size_t len,
                               const A0HirExpr *args, size_t arg_count);
/* Reports a language's own error at the current span. */
void a0_hir_report(A0HirBuilder *b, const char *message, size_t len);
/* Stands in for an expression that could not be built; causes no further errors. */
A0HirExpr a0_hir_error_expr(A0HirBuilder *b);

/* Modules (docs/hir.md, "Modules"). Items declared before the first module are the prelude, seen
 * by every module; the root module's items keep their names, another's are `module.item`. */
uint32_t a0_hir_begin_module(A0HirBuilder *b, const char *name, size_t len, int root);
/* Whether the declarations that follow can be used from other modules (default true). */
void a0_hir_set_public(A0HirBuilder *b, int is_public);
/* Lets the current module use the public items of module `name` as `alias.item`. */
void a0_hir_import_module(A0HirBuilder *b, const char *name, size_t len, const char *alias,
                          size_t alias_len);
/* Lets the current module use one public item of `module` by its name. */
void a0_hir_import_item(A0HirBuilder *b, const char *module, size_t module_len, const char *name,
                        size_t len);
int a0_hir_is_module(A0HirBuilder *b, const char *alias, size_t len);
/* `alias.name`, `alias.name(args)` and the type `alias.name<args>`; problems are reported. */
A0HirExpr a0_hir_member(A0HirBuilder *b, const char *alias, size_t alias_len, const char *name,
                        size_t len);
A0HirExpr a0_hir_call_member(A0HirBuilder *b, const char *alias, size_t alias_len,
                             const char *name, size_t len, const A0HirExpr *args,
                             size_t arg_count);
A0HirType a0_hir_member_type(A0HirBuilder *b, const char *alias, size_t alias_len,
                             const char *name, size_t len, const A0HirType *args,
                             size_t arg_count);

#ifdef __cplusplus
}
#endif

#endif
