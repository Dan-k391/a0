/* Dynamic objects: fields added by name at run time, for dynamically typed languages (D66).
 *
 * Field names are interned: one string per distinct name, kept for the rest of the program, so
 * names compare by pointer. An object's fields are a row of `any` slots, and its shape says which
 * name each slot holds. Shapes form a tree from the empty shape, each adding one name to its
 * parent, so objects that get the same fields in the same order share a shape, and finding a
 * field is a question about the shape, not the object. The answers are cached: generated code
 * keeps one per place that names a field (A0FieldCache: the shape it saw and the slot), and the
 * runtime keeps a table for the rest, by shape and name. Shapes are never freed. */
#include "internal.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ---- Interned names --------------------------------------------------------------------------- */

static A0String **names; /* open addressing by content hash; NULL is empty */
static int64_t name_count, name_cap;

static int same_text(const A0String *a, const A0String *b) {
    return a->len == b->len && memcmp(a->bytes, b->bytes, (size_t)a->len) == 0;
}

static void names_insert(A0String *s) {
    uint64_t mask = (uint64_t)name_cap - 1;
    uint64_t at = a0_string_hash(s) & mask;
    while (names[at] != NULL) at = (at + 1) & mask;
    names[at] = s;
}

static A0String *intern(A0String *s) {
    if (name_cap > 0) {
        uint64_t mask = (uint64_t)name_cap - 1;
        for (uint64_t at = a0_string_hash(s) & mask; names[at] != NULL; at = (at + 1) & mask) {
            if (same_text(names[at], s)) return names[at];
        }
    }
    if ((name_count + 1) * 2 > name_cap) {
        A0String **old = names;
        int64_t old_cap = name_cap;
        name_cap = name_cap == 0 ? 64 : name_cap * 2;
        names = (A0String **)calloc((size_t)name_cap, sizeof(A0String *));
        if (names == NULL) a0_panic("out of memory");
        for (int64_t i = 0; i < old_cap; i++) {
            if (old[i] != NULL) names_insert(old[i]);
        }
        free(old);
    }
    HEADER(s)->flags |= A0_FLAG_INTERNED;
    names_insert(s);
    name_count++;
    return s;
}

A0String *a0_intern(A0String *s) {
    if (s == NULL) a0_panic_null();
    if (HEADER(s)->flags & A0_FLAG_INTERNED) return s;
    /* The hash is computed (and stored) before the lock, so that the table's work is short. */
    a0_string_hash(s);
    a0_table_lock();
    A0String *r = intern(s);
    a0_table_unlock();
    return r;
}

void a0_intern_mark(void) {
    for (int64_t i = 0; i < name_cap; i++) {
        if (names[i] != NULL) a0_gc_mark_object(names[i]);
    }
}

/* ---- Shapes ----------------------------------------------------------------------------------- */

static A0Shape empty_shape;

/* The shape with `name` added to `shape`, made the first time it is asked for. */
static const A0Shape *shape_add_locked(const A0Shape *shape, A0String *name) {
    A0Shape *s = (A0Shape *)shape;
    for (int64_t i = 0; i < s->child_count; i++) {
        if (s->children[i]->name == name) return s->children[i];
    }
    A0Shape *child = (A0Shape *)calloc(1, sizeof(A0Shape));
    if (child == NULL) a0_panic("out of memory");
    child->parent = s;
    child->name = name;
    child->count = s->count + 1;
    if (s->child_count == s->child_cap) {
        s->child_cap = s->child_cap == 0 ? 2 : s->child_cap * 2;
        s->children = (A0Shape **)realloc(s->children, sizeof(A0Shape *) * (size_t)s->child_cap);
        if (s->children == NULL) a0_panic("out of memory");
    }
    s->children[s->child_count++] = child;
    return child;
}

static const A0Shape *shape_add(const A0Shape *shape, A0String *name) {
    a0_table_lock();
    const A0Shape *r = shape_add_locked(shape, name);
    a0_table_unlock();
    return r;
}

/* The slot of the interned `name` in objects of `shape`, or -1. */
static int64_t shape_find(const A0Shape *shape, const A0String *name) {
    for (const A0Shape *s = shape; s->parent != NULL; s = s->parent) {
        if (s->name == name) return s->count - 1;
    }
    return -1;
}

/* Answers by shape and name, for lookups that bring no cache of their own. */
#define TABLE_SIZE 1024
static const A0Shape *table_shapes[TABLE_SIZE];
static const A0String *table_names[TABLE_SIZE];
static int64_t table_slots[TABLE_SIZE];

static int64_t find(const A0Shape *shape, const A0String *name) {
    uintptr_t h = (uintptr_t)shape * 31u + (uintptr_t)name;
    size_t at = (size_t)((h ^ (h >> 11)) >> 4) & (TABLE_SIZE - 1);
    a0_table_lock();
    int64_t index;
    if (table_shapes[at] == shape && table_names[at] == name) {
        index = table_slots[at];
    } else {
        index = shape_find(shape, name);
        table_shapes[at] = shape;
        table_names[at] = name;
        table_slots[at] = index;
    }
    a0_table_unlock();
    return index;
}

/* ---- Objects ---------------------------------------------------------------------------------- */

static A0Object *check(A0Object *o) {
    if (o == NULL) a0_panic_null();
    return o;
}

A0Object *a0_object_new(void) {
    A0Object *o = (A0Object *)a0_gc_alloc(&a0_builtin_object, sizeof(A0Object));
    o->shape = &empty_shape;
    o->cap = A0_OBJECT_INLINE;
    o->slots = o->inline_slots;
    return o;
}

static int is_inline(A0Object *o) { return o->slots == o->inline_slots; }

int64_t a0_object_find_cached(A0Object *o, A0String *name, A0FieldCache *cache) {
    check(o);
    int64_t index = find(o->shape, a0_intern(name));
    cache->shape = o->shape;
    cache->index = index;
    cache->to = o->shape;
    return index;
}

int64_t a0_object_has(A0Object *o, A0String *name) {
    check(o);
    return find(o->shape, a0_intern(name)) >= 0;
}

A0_COLD static void no_field(A0String *name, uint64_t loc, void *frame) {
    char message[200];
    snprintf(message, sizeof message, "no field `%.*s`", (int)(name->len > 120 ? 120 : name->len),
             (const char *)name->bytes);
    a0_rt_fail(message, loc, frame);
}

uint64_t a0_object_get(A0Object *o, A0String *name, uint64_t loc, void *frame) {
    check(o);
    int64_t index = find(o->shape, a0_intern(name));
    if (index < 0) no_field(name, loc, frame);
    return o->slots[index];
}

void a0_object_set_cached(A0Object *o, A0String *name, uint64_t value, A0FieldCache *cache) {
    check(o);
    name = a0_intern(name);
    const A0Shape *from = o->shape;
    int64_t index = find(from, name);
    if (index < 0) {
        index = from->count;
        if (index == o->cap) {
            int64_t cap = o->cap * 2;
            size_t old_size = sizeof(uint64_t) * (size_t)o->cap;
            if (is_inline(o)) {
                uint64_t *slots = (uint64_t *)a0_gc_buffer_alloc(sizeof(uint64_t) * (size_t)cap);
                memcpy(slots, o->slots, old_size);
                o->slots = slots;
            } else {
                o->slots = (uint64_t *)a0_gc_buffer_realloc(o->slots, old_size,
                                                            sizeof(uint64_t) * (size_t)cap);
            }
            o->cap = cap;
        }
        o->shape = shape_add(from, name);
    }
    o->slots[index] = value;
    if (cache != NULL) {
        cache->shape = from;
        cache->index = index;
        cache->to = o->shape;
    }
}

void a0_object_set(A0Object *o, A0String *name, uint64_t value) {
    a0_object_set_cached(o, name, value, NULL);
}

A0Array *a0_object_names(A0Object *o) {
    check(o);
    int64_t count = o->shape->count;
    A0Array *a = a0_array_new(A0_WORD_REF, count);
    a->len = count;
    for (const A0Shape *s = o->shape; s->parent != NULL; s = s->parent) {
        a->data[s->count - 1] = (uint64_t)(uintptr_t)s->name;
    }
    return a;
}

void a0_object_trace(A0Object *o) {
    for (int64_t i = 0; i < o->shape->count; i++) a0_gc_mark_word(o->slots[i], A0_WORD_ANY);
}

void a0_object_finalize(A0Object *o) {
    if (!is_inline(o)) a0_gc_buffer_free(o->slots, sizeof(uint64_t) * (size_t)o->cap);
}
