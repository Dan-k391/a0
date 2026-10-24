/* The Lox runtime: values, objects, garbage collection, and the operations compiled Lox code calls.
 *
 * Values are NaN-boxed 64-bit words (as in clox). Compiled code keeps every live Lox value in a frame
 * of the runtime's shadow stack (lox_frame_enter/leave), so the collector finds all roots precisely:
 * the shadow stack, globals, constants and a few C-side temporaries. Runtime errors print jlox's
 * format ("message\n[line N]") and exit with status 70.
 */
#include "a0rt.h"

#include <math.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef uint64_t Value;

#define SIGN_BIT ((uint64_t)0x8000000000000000ull)
#define QNAN ((uint64_t)0x7ffc000000000000ull)
#define NIL_VAL (QNAN | 1)
#define FALSE_VAL (QNAN | 2)
#define TRUE_VAL (QNAN | 3)
#define BOOL_VAL(b) ((b) ? TRUE_VAL : FALSE_VAL)
#define IS_NUMBER(v) (((v) & QNAN) != QNAN)
#define IS_OBJ(v) (((v) & (QNAN | SIGN_BIT)) == (QNAN | SIGN_BIT))
#define AS_OBJ(v) ((Obj *)(uintptr_t)((v) & ~(SIGN_BIT | QNAN)))
#define OBJ_VAL(o) (SIGN_BIT | QNAN | (uint64_t)(uintptr_t)(o))

static double as_number(Value v) {
    double d;
    memcpy(&d, &v, sizeof d);
    return d;
}

static Value number_val(double d) {
    Value v;
    memcpy(&v, &d, sizeof v);
    return v;
}

#define EXIT_RUNTIME_ERROR 70
/* Deeper recursion than this reports "Stack overflow." (the interpreter allows 10000 IR calls). */
#define MAX_DEPTH 5000
/* Words of shadow stack for all frames. */
#define SHADOW_CAPACITY (4 * 1024 * 1024)

/* ---- Objects ------------------------------------------------------------------------------ */

typedef enum { OBJ_STRING, OBJ_CLOSURE, OBJ_NATIVE, OBJ_CLASS, OBJ_INSTANCE, OBJ_BOUND, OBJ_CELL } ObjType;

typedef struct Obj {
    struct Obj *next;
    ObjType type;
    bool marked;
} Obj;

typedef struct {
    Obj obj;
    int64_t length;
    uint32_t hash;
    char chars[];
} ObjString;

/* Compiled Lox functions, natives and stubs all take the call window: [callee, receiver, args...]. */
typedef Value (*LoxCode)(Value *window);

typedef struct ObjCell {
    Obj obj;
    Value value;
} ObjCell;

typedef struct {
    Obj obj;
    LoxCode code;
    ObjString *name;
    int64_t arity;
    int64_t cell_count;
    ObjCell *cells[];
} ObjClosure;

typedef struct {
    Obj obj;
    LoxCode code;
    int64_t arity;
} ObjNative;

typedef struct {
    ObjString *key;
    Value value;
} Entry;

typedef struct {
    int64_t count;
    int64_t capacity;
    Entry *entries;
} Table;

typedef struct {
    Obj obj;
    ObjString *name;
    Table methods;
} ObjClass;

typedef struct {
    Obj obj;
    ObjClass *klass;
    Table fields;
} ObjInstance;

typedef struct {
    Obj obj;
    Value receiver;
    ObjClosure *method;
} ObjBound;

#define IS_TYPE(v, t) (IS_OBJ(v) && AS_OBJ(v)->type == (t))

/* ---- Runtime state ------------------------------------------------------------------------ */

static Obj *objects;
static size_t bytes_allocated;
static size_t next_gc = 1024 * 1024;
static bool gc_stress;
static Obj **gray_stack;
static int64_t gray_count, gray_capacity;

static Value *shadow;
static int64_t shadow_top;
static int64_t depth;
static int64_t call_line;

static Value *globals;
static bool *global_defined;
static ObjString **global_names;
static int64_t global_count;

static Value *constants;
static int64_t constant_count;

static Table strings;
static ObjString *init_string;

/* C-side temporaries that must survive an allocation. */
static Value temp_roots[8];
static int temp_count;

static void push_root(Value v) { temp_roots[temp_count++] = v; }
static void pop_root(void) { temp_count--; }

static void collect_garbage(void);

static void *reallocate(void *pointer, size_t old_size, size_t new_size) {
    bytes_allocated += new_size - old_size;
    if (new_size > old_size) {
        if (gc_stress || bytes_allocated > next_gc) collect_garbage();
    }
    if (new_size == 0) {
        free(pointer);
        return NULL;
    }
    void *result = realloc(pointer, new_size);
    if (result == NULL) a0_panic("out of memory");
    return result;
}

static Obj *allocate_object(size_t size, ObjType type) {
    Obj *object = (Obj *)reallocate(NULL, 0, size);
    object->type = type;
    object->marked = false;
    object->next = objects;
    objects = object;
    return object;
}

/* ---- Errors ------------------------------------------------------------------------------- */

static void runtime_error(int64_t line, const char *format, ...) {
    char message[512];
    va_list args;
    va_start(args, format);
    vsnprintf(message, sizeof message, format, args);
    va_end(args);
    fflush(stdout);
    fprintf(stderr, "%s\n[line %lld]\n", message, (long long)line);
    a0_rt_exit(EXIT_RUNTIME_ERROR);
}

/* ---- Strings and tables ------------------------------------------------------------------- */

static uint32_t hash_string(const char *key, int64_t length) {
    uint32_t hash = 2166136261u;
    for (int64_t i = 0; i < length; i++) {
        hash ^= (uint8_t)key[i];
        hash *= 16777619;
    }
    return hash;
}

static Entry *find_entry(Entry *entries, int64_t capacity, ObjString *key) {
    uint32_t index = key->hash & (uint32_t)(capacity - 1);
    Entry *tombstone = NULL;
    for (;;) {
        Entry *entry = &entries[index];
        if (entry->key == NULL) {
            if (entry->value == NIL_VAL) return tombstone != NULL ? tombstone : entry;
            if (tombstone == NULL) tombstone = entry;
        } else if (entry->key == key) {
            return entry;
        }
        index = (index + 1) & (uint32_t)(capacity - 1);
    }
}

static void adjust_capacity(Table *table, int64_t capacity) {
    Entry *entries = (Entry *)reallocate(NULL, 0, sizeof(Entry) * (size_t)capacity);
    for (int64_t i = 0; i < capacity; i++) {
        entries[i].key = NULL;
        entries[i].value = NIL_VAL;
    }
    table->count = 0;
    for (int64_t i = 0; i < table->capacity; i++) {
        Entry *entry = &table->entries[i];
        if (entry->key == NULL) continue;
        Entry *dest = find_entry(entries, capacity, entry->key);
        dest->key = entry->key;
        dest->value = entry->value;
        table->count++;
    }
    reallocate(table->entries, sizeof(Entry) * (size_t)table->capacity, 0);
    table->entries = entries;
    table->capacity = capacity;
}

static bool table_get(Table *table, ObjString *key, Value *value) {
    if (table->count == 0) return false;
    Entry *entry = find_entry(table->entries, table->capacity, key);
    if (entry->key == NULL) return false;
    *value = entry->value;
    return true;
}

/* Returns true if the key is new. May allocate (and so collect): callers keep key and value rooted. */
static bool table_set(Table *table, ObjString *key, Value value) {
    if ((table->count + 1) * 4 > table->capacity * 3) {
        adjust_capacity(table, table->capacity < 8 ? 8 : table->capacity * 2);
    }
    Entry *entry = find_entry(table->entries, table->capacity, key);
    bool is_new = entry->key == NULL;
    if (is_new && entry->value == NIL_VAL) table->count++;
    entry->key = key;
    entry->value = value;
    return is_new;
}

static void table_add_all(Table *from, Table *to) {
    for (int64_t i = 0; i < from->capacity; i++) {
        Entry *entry = &from->entries[i];
        if (entry->key != NULL) table_set(to, entry->key, entry->value);
    }
}

static void free_table(Table *table) {
    reallocate(table->entries, sizeof(Entry) * (size_t)table->capacity, 0);
    table->entries = NULL;
    table->count = table->capacity = 0;
}

static ObjString *table_find_string(Table *table, const char *chars, int64_t length, uint32_t hash) {
    if (table->count == 0) return NULL;
    uint32_t index = hash & (uint32_t)(table->capacity - 1);
    for (;;) {
        Entry *entry = &table->entries[index];
        if (entry->key == NULL) {
            if (entry->value == NIL_VAL) return NULL;
        } else if (entry->key->length == length && entry->key->hash == hash &&
                   memcmp(entry->key->chars, chars, (size_t)length) == 0) {
            return entry->key;
        }
        index = (index + 1) & (uint32_t)(table->capacity - 1);
    }
}

/* Interned strings: equal strings are the same object, so equality is identity. */
static ObjString *intern(const char *chars, int64_t length) {
    uint32_t hash = hash_string(chars, length);
    ObjString *existing = table_find_string(&strings, chars, length, hash);
    if (existing != NULL) return existing;
    ObjString *s = (ObjString *)allocate_object(sizeof(ObjString) + (size_t)length + 1, OBJ_STRING);
    s->length = length;
    s->hash = hash;
    memcpy(s->chars, chars, (size_t)length);
    s->chars[length] = '\0';
    push_root(OBJ_VAL(s));
    table_set(&strings, s, NIL_VAL);
    pop_root();
    return s;
}

/* ---- Garbage collection ------------------------------------------------------------------- */

static void mark_object(Obj *object) {
    if (object == NULL || object->marked) return;
    object->marked = true;
    if (gray_count + 1 > gray_capacity) {
        gray_capacity = gray_capacity < 8 ? 8 : gray_capacity * 2;
        gray_stack = (Obj **)realloc(gray_stack, sizeof(Obj *) * (size_t)gray_capacity);
        if (gray_stack == NULL) a0_panic("out of memory");
    }
    gray_stack[gray_count++] = object;
}

static void mark_value(Value v) {
    if (IS_OBJ(v)) mark_object(AS_OBJ(v));
}

static void mark_table(Table *table) {
    for (int64_t i = 0; i < table->capacity; i++) {
        Entry *entry = &table->entries[i];
        mark_object((Obj *)entry->key);
        mark_value(entry->value);
    }
}

static void blacken(Obj *object) {
    switch (object->type) {
    case OBJ_CLOSURE: {
        ObjClosure *c = (ObjClosure *)object;
        mark_object((Obj *)c->name);
        for (int64_t i = 0; i < c->cell_count; i++) mark_object((Obj *)c->cells[i]);
        break;
    }
    case OBJ_CLASS: {
        ObjClass *k = (ObjClass *)object;
        mark_object((Obj *)k->name);
        mark_table(&k->methods);
        break;
    }
    case OBJ_INSTANCE: {
        ObjInstance *i = (ObjInstance *)object;
        mark_object((Obj *)i->klass);
        mark_table(&i->fields);
        break;
    }
    case OBJ_BOUND: {
        ObjBound *b = (ObjBound *)object;
        mark_value(b->receiver);
        mark_object((Obj *)b->method);
        break;
    }
    case OBJ_CELL:
        mark_value(((ObjCell *)object)->value);
        break;
    case OBJ_STRING:
    case OBJ_NATIVE:
        break;
    }
}

static size_t object_size(Obj *object) {
    switch (object->type) {
    case OBJ_STRING: return sizeof(ObjString) + (size_t)((ObjString *)object)->length + 1;
    case OBJ_CLOSURE: return sizeof(ObjClosure) + sizeof(ObjCell *) * (size_t)((ObjClosure *)object)->cell_count;
    case OBJ_NATIVE: return sizeof(ObjNative);
    case OBJ_CLASS: return sizeof(ObjClass);
    case OBJ_INSTANCE: return sizeof(ObjInstance);
    case OBJ_BOUND: return sizeof(ObjBound);
    case OBJ_CELL: return sizeof(ObjCell);
    }
    return 0;
}

static void free_object(Obj *object) {
    if (object->type == OBJ_CLASS) free_table(&((ObjClass *)object)->methods);
    if (object->type == OBJ_INSTANCE) free_table(&((ObjInstance *)object)->fields);
    reallocate(object, object_size(object), 0);
}

static void collect_garbage(void) {
    if (shadow == NULL) return; /* not initialized yet */
    for (int64_t i = 0; i < shadow_top; i++) mark_value(shadow[i]);
    for (int64_t i = 0; i < global_count; i++) {
        mark_value(globals[i]);
        mark_object((Obj *)global_names[i]);
    }
    for (int64_t i = 0; i < constant_count; i++) mark_value(constants[i]);
    for (int i = 0; i < temp_count; i++) mark_value(temp_roots[i]);
    mark_object((Obj *)init_string);
    while (gray_count > 0) blacken(gray_stack[--gray_count]);
    /* The intern table holds strings weakly. */
    for (int64_t i = 0; i < strings.capacity; i++) {
        Entry *entry = &strings.entries[i];
        if (entry->key != NULL && !entry->key->obj.marked) {
            entry->key = NULL;
            entry->value = TRUE_VAL; /* tombstone */
        }
    }
    Obj *previous = NULL;
    Obj *object = objects;
    while (object != NULL) {
        if (object->marked) {
            object->marked = false;
            previous = object;
            object = object->next;
        } else {
            Obj *unreached = object;
            object = object->next;
            if (previous != NULL) previous->next = object;
            else objects = object;
            free_object(unreached);
        }
    }
    next_gc = bytes_allocated * 2 < 1024 * 1024 ? 1024 * 1024 : bytes_allocated * 2;
}

/* ---- Program setup ------------------------------------------------------------------------ */

void lox_init(int64_t nglobals, int64_t nconstants) {
    const char *stress = getenv("LOX_GC_STRESS");
    gc_stress = stress != NULL && stress[0] != '\0' && stress[0] != '0';
    shadow = (Value *)calloc(SHADOW_CAPACITY, sizeof(Value));
    globals = (Value *)calloc((size_t)(nglobals > 0 ? nglobals : 1), sizeof(Value));
    global_defined = (bool *)calloc((size_t)(nglobals > 0 ? nglobals : 1), sizeof(bool));
    global_names = (ObjString **)calloc((size_t)(nglobals > 0 ? nglobals : 1), sizeof(ObjString *));
    constants = (Value *)calloc((size_t)(nconstants > 0 ? nconstants : 1), sizeof(Value));
    if (!shadow || !globals || !global_defined || !global_names || !constants) a0_panic("out of memory");
    for (int64_t i = 0; i < nglobals; i++) globals[i] = NIL_VAL;
    for (int64_t i = 0; i < nconstants; i++) constants[i] = NIL_VAL;
    global_count = nglobals;
    constant_count = nconstants;
    init_string = intern("init", 4);
}

void lox_global_name(int64_t index, const char *chars, int64_t length) {
    global_names[index] = intern(chars, length);
}

void lox_string_constant(int64_t index, const char *chars, int64_t length) {
    constants[index] = OBJ_VAL(intern(chars, length));
}

Value lox_constant(int64_t index) { return constants[index]; }

/* ---- Frames ------------------------------------------------------------------------------- */

Value *lox_frame_enter(int64_t slots) {
    if (++depth > MAX_DEPTH || shadow_top + slots > SHADOW_CAPACITY) {
        runtime_error(call_line, "Stack overflow.");
    }
    Value *frame = &shadow[shadow_top];
    /* Zero is the number 0.0: harmless to the collector, and no stale pointers survive. */
    memset(frame, 0, sizeof(Value) * (size_t)slots);
    shadow_top += slots;
    return frame;
}

void lox_frame_leave(int64_t slots) {
    shadow_top -= slots;
    depth--;
}

/* ---- Globals ------------------------------------------------------------------------------ */

Value lox_global_get(int64_t index, int64_t line) {
    if (!global_defined[index]) {
        runtime_error(line, "Undefined variable '%s'.", global_names[index]->chars);
    }
    return globals[index];
}

void lox_global_set(int64_t index, Value value, int64_t line) {
    if (!global_defined[index]) {
        runtime_error(line, "Undefined variable '%s'.", global_names[index]->chars);
    }
    globals[index] = value;
}

void lox_global_define(int64_t index, Value value) {
    globals[index] = value;
    global_defined[index] = true;
}

/* ---- Printing ----------------------------------------------------------------------------- */

static void print_value(Value v) {
    if (IS_NUMBER(v)) {
        double d = as_number(v);
        if (isnan(d)) {
            fputs("nan", stdout);
        } else {
            a0_print_f64(d);
        }
        return;
    }
    if (v == NIL_VAL) { fputs("nil", stdout); return; }
    if (v == TRUE_VAL) { fputs("true", stdout); return; }
    if (v == FALSE_VAL) { fputs("false", stdout); return; }
    Obj *o = AS_OBJ(v);
    switch (o->type) {
    case OBJ_STRING: fwrite(((ObjString *)o)->chars, 1, (size_t)((ObjString *)o)->length, stdout); break;
    case OBJ_CLOSURE: printf("<fn %s>", ((ObjClosure *)o)->name->chars); break;
    case OBJ_NATIVE: fputs("<native fn>", stdout); break;
    case OBJ_CLASS: fputs(((ObjClass *)o)->name->chars, stdout); break;
    case OBJ_INSTANCE: printf("%s instance", ((ObjInstance *)o)->klass->name->chars); break;
    case OBJ_BOUND: printf("<fn %s>", ((ObjBound *)o)->method->name->chars); break;
    case OBJ_CELL: fputs("<cell>", stdout); break;
    }
}

void lox_print(Value v) {
    print_value(v);
    putchar('\n');
}

/* ---- Operators ---------------------------------------------------------------------------- */

static bool values_equal(Value a, Value b) {
    if (IS_NUMBER(a) && IS_NUMBER(b)) return as_number(a) == as_number(b);
    return a == b;
}

Value lox_equal(Value a, Value b) { return BOOL_VAL(values_equal(a, b)); }
Value lox_not_equal(Value a, Value b) { return BOOL_VAL(!values_equal(a, b)); }
Value lox_not(Value a) { return BOOL_VAL(a == NIL_VAL || a == FALSE_VAL); }

Value lox_add(Value a, Value b, int64_t line) {
    if (IS_NUMBER(a) && IS_NUMBER(b)) return number_val(as_number(a) + as_number(b));
    if (IS_TYPE(a, OBJ_STRING) && IS_TYPE(b, OBJ_STRING)) {
        /* Both operands stay rooted in the caller's frame while we allocate. */
        ObjString *x = (ObjString *)AS_OBJ(a);
        ObjString *y = (ObjString *)AS_OBJ(b);
        int64_t length = x->length + y->length;
        char *chars = (char *)malloc((size_t)length + 1);
        if (chars == NULL) a0_panic("out of memory");
        memcpy(chars, x->chars, (size_t)x->length);
        memcpy(chars + x->length, y->chars, (size_t)y->length);
        ObjString *result = intern(chars, length);
        free(chars);
        return OBJ_VAL(result);
    }
    runtime_error(line, "Operands must be two numbers or two strings.");
    return NIL_VAL;
}

#define NUMERIC_BINARY(name, expr)                                                                  \
    Value name(Value a, Value b, int64_t line) {                                                    \
        if (!IS_NUMBER(a) || !IS_NUMBER(b)) runtime_error(line, "Operands must be numbers.");       \
        double x = as_number(a), y = as_number(b);                                                  \
        return expr;                                                                                \
    }

NUMERIC_BINARY(lox_subtract, number_val(x - y))
NUMERIC_BINARY(lox_multiply, number_val(x * y))
NUMERIC_BINARY(lox_divide, number_val(x / y))
NUMERIC_BINARY(lox_less, BOOL_VAL(x < y))
NUMERIC_BINARY(lox_less_equal, BOOL_VAL(x <= y))
NUMERIC_BINARY(lox_greater, BOOL_VAL(x > y))
NUMERIC_BINARY(lox_greater_equal, BOOL_VAL(x >= y))

Value lox_negate(Value a, int64_t line) {
    if (!IS_NUMBER(a)) runtime_error(line, "Operand must be a number.");
    return number_val(-as_number(a));
}

/* ---- Cells and closures ------------------------------------------------------------------- */

Value lox_cell_new(Value value) {
    /* `value` is rooted by the caller (it is in a frame slot). */
    ObjCell *cell = (ObjCell *)allocate_object(sizeof(ObjCell), OBJ_CELL);
    cell->value = value;
    return OBJ_VAL(cell);
}

Value lox_cell_get(Value cell) { return ((ObjCell *)AS_OBJ(cell))->value; }
void lox_cell_set(Value cell, Value value) { ((ObjCell *)AS_OBJ(cell))->value = value; }

Value lox_closure_new(LoxCode code, Value name, int64_t arity, int64_t cell_count) {
    ObjClosure *c = (ObjClosure *)allocate_object(
        sizeof(ObjClosure) + sizeof(ObjCell *) * (size_t)cell_count, OBJ_CLOSURE);
    c->code = code;
    c->name = (ObjString *)AS_OBJ(name);
    c->arity = arity;
    c->cell_count = cell_count;
    for (int64_t i = 0; i < cell_count; i++) c->cells[i] = NULL;
    return OBJ_VAL(c);
}

void lox_closure_set_cell(Value closure, int64_t index, Value cell) {
    ((ObjClosure *)AS_OBJ(closure))->cells[index] = (ObjCell *)AS_OBJ(cell);
}

Value lox_closure_cell(Value closure, int64_t index) {
    return OBJ_VAL(((ObjClosure *)AS_OBJ(closure))->cells[index]);
}

/* ---- Natives ------------------------------------------------------------------------------ */

static Value native_clock(Value *window) {
    (void)window;
    return number_val(a0_clock());
}

Value lox_native_clock(void) {
    ObjNative *n = (ObjNative *)allocate_object(sizeof(ObjNative), OBJ_NATIVE);
    n->code = native_clock;
    n->arity = 0;
    return OBJ_VAL(n);
}

/* ---- Classes and instances ---------------------------------------------------------------- */

Value lox_class_new(Value name) {
    ObjClass *k = (ObjClass *)allocate_object(sizeof(ObjClass), OBJ_CLASS);
    k->name = (ObjString *)AS_OBJ(name);
    k->methods.count = k->methods.capacity = 0;
    k->methods.entries = NULL;
    return OBJ_VAL(k);
}

void lox_check_superclass(Value superclass, int64_t line) {
    if (!IS_TYPE(superclass, OBJ_CLASS)) runtime_error(line, "Superclass must be a class.");
}

/* Methods are copied down, which is equivalent to lookup through the chain because classes never
 * change after they are created; the subclass's own methods are added afterwards and win. */
void lox_class_inherit(Value klass, Value superclass) {
    table_add_all(&((ObjClass *)AS_OBJ(superclass))->methods, &((ObjClass *)AS_OBJ(klass))->methods);
}

void lox_class_add_method(Value klass, Value name, Value closure) {
    table_set(&((ObjClass *)AS_OBJ(klass))->methods, (ObjString *)AS_OBJ(name), closure);
}

static Value bind_method(Value receiver, ObjClosure *method) {
    push_root(receiver);
    ObjBound *b = (ObjBound *)allocate_object(sizeof(ObjBound), OBJ_BOUND);
    pop_root();
    b->receiver = receiver;
    b->method = method;
    return OBJ_VAL(b);
}

Value lox_get_property(Value object, Value name, int64_t line) {
    if (!IS_TYPE(object, OBJ_INSTANCE)) runtime_error(line, "Only instances have properties.");
    ObjInstance *instance = (ObjInstance *)AS_OBJ(object);
    ObjString *key = (ObjString *)AS_OBJ(name);
    Value value;
    if (table_get(&instance->fields, key, &value)) return value;
    if (table_get(&instance->klass->methods, key, &value)) {
        return bind_method(object, (ObjClosure *)AS_OBJ(value));
    }
    runtime_error(line, "Undefined property '%s'.", key->chars);
    return NIL_VAL;
}

/* jlox checks the object before evaluating the value being assigned. */
void lox_check_instance(Value object, int64_t line) {
    if (!IS_TYPE(object, OBJ_INSTANCE)) runtime_error(line, "Only instances have fields.");
}

Value lox_set_property(Value object, Value name, Value value, int64_t line) {
    if (!IS_TYPE(object, OBJ_INSTANCE)) runtime_error(line, "Only instances have fields.");
    table_set(&((ObjInstance *)AS_OBJ(object))->fields, (ObjString *)AS_OBJ(name), value);
    return value;
}

Value lox_super_get(Value superclass, Value receiver, Value name, int64_t line) {
    ObjString *key = (ObjString *)AS_OBJ(name);
    Value method;
    if (!table_get(&((ObjClass *)AS_OBJ(superclass))->methods, key, &method)) {
        runtime_error(line, "Undefined property '%s'.", key->chars);
    }
    return bind_method(receiver, (ObjClosure *)AS_OBJ(method));
}

/* ---- Calls -------------------------------------------------------------------------------- */

/* A class without `init`: the call evaluates to the new instance. */
static Value default_init(Value *window) { return window[1]; }

static void check_arity(int64_t arity, int64_t argc, int64_t line) {
    if (arity != argc) {
        runtime_error(line, "Expected %lld arguments but got %lld.", (long long)arity, (long long)argc);
    }
}

/* Prepares a call. The window is [callee, receiver, args...]; on return window[0] is the closure to
 * run (if any) and window[1] the receiver (`this`), and the result is the code to call with the
 * window. */
LoxCode lox_call_prepare(Value *window, int64_t argc, int64_t line) {
    Value callee = window[0];
    call_line = line;
    if (IS_OBJ(callee)) {
        Obj *o = AS_OBJ(callee);
        switch (o->type) {
        case OBJ_CLOSURE: {
            ObjClosure *c = (ObjClosure *)o;
            check_arity(c->arity, argc, line);
            window[1] = NIL_VAL;
            return c->code;
        }
        case OBJ_BOUND: {
            ObjBound *b = (ObjBound *)o;
            check_arity(b->method->arity, argc, line);
            window[0] = OBJ_VAL(b->method);
            window[1] = b->receiver;
            return b->method->code;
        }
        case OBJ_CLASS: {
            ObjClass *k = (ObjClass *)o;
            ObjInstance *instance = (ObjInstance *)allocate_object(sizeof(ObjInstance), OBJ_INSTANCE);
            instance->klass = k;
            instance->fields.count = instance->fields.capacity = 0;
            instance->fields.entries = NULL;
            window[1] = OBJ_VAL(instance);
            Value init;
            if (table_get(&k->methods, init_string, &init)) {
                ObjClosure *c = (ObjClosure *)AS_OBJ(init);
                check_arity(c->arity, argc, line);
                window[0] = init;
                return c->code;
            }
            check_arity(0, argc, line);
            return default_init;
        }
        case OBJ_NATIVE: {
            ObjNative *n = (ObjNative *)o;
            check_arity(n->arity, argc, line);
            return n->code;
        }
        default:
            break;
        }
    }
    runtime_error(line, "Can only call functions and classes.");
    return NULL;
}

/* ---- Symbol table for the interpreter ----------------------------------------------------- */

#define SYM(f) {#f, (void *)&f}

const A0Symbol lox_symbols[] = {
    SYM(lox_init),
    SYM(lox_global_name),
    SYM(lox_string_constant),
    SYM(lox_constant),
    SYM(lox_frame_enter),
    SYM(lox_frame_leave),
    SYM(lox_global_get),
    SYM(lox_global_set),
    SYM(lox_global_define),
    SYM(lox_print),
    SYM(lox_equal),
    SYM(lox_not_equal),
    SYM(lox_not),
    SYM(lox_add),
    SYM(lox_subtract),
    SYM(lox_multiply),
    SYM(lox_divide),
    SYM(lox_less),
    SYM(lox_less_equal),
    SYM(lox_greater),
    SYM(lox_greater_equal),
    SYM(lox_negate),
    SYM(lox_cell_new),
    SYM(lox_cell_get),
    SYM(lox_cell_set),
    SYM(lox_closure_new),
    SYM(lox_closure_set_cell),
    SYM(lox_closure_cell),
    SYM(lox_native_clock),
    SYM(lox_class_new),
    SYM(lox_check_superclass),
    SYM(lox_class_inherit),
    SYM(lox_class_add_method),
    SYM(lox_get_property),
    SYM(lox_check_instance),
    SYM(lox_set_property),
    SYM(lox_super_get),
    SYM(lox_call_prepare),
    {NULL, NULL},
};
