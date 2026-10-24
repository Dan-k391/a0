/* Arrays: growable vectors of 64-bit words. */
#include "internal.h"

static A0Array *check(A0Array *a) {
    if (a == NULL) a0_panic_null();
    return a;
}

A0Array *a0_array_new(int64_t elem_kind, int64_t capacity) {
    A0Array *a = (A0Array *)a0_gc_alloc(&a0_builtin_array, sizeof(A0Array));
    a->elem_kind = elem_kind;
    if (capacity > 0) {
        a->data = (uint64_t *)a0_gc_buffer_alloc(sizeof(uint64_t) * (size_t)capacity);
        a->cap = capacity;
    }
    return a;
}

int64_t a0_array_len(A0Array *a) { return check(a)->len; }

uint64_t a0_array_get(A0Array *a, int64_t index) {
    check(a);
    if (index < 0 || index >= a->len) a0_panic("index out of bounds");
    return a->data[index];
}

void a0_array_set(A0Array *a, int64_t index, uint64_t value) {
    check(a);
    if (index < 0 || index >= a->len) a0_panic("index out of bounds");
    a->data[index] = value;
}

void a0_array_push(A0Array *a, uint64_t value) {
    check(a);
    if (a->len == a->cap) {
        int64_t cap = a->cap < 8 ? 8 : a->cap * 2;
        a->data = (uint64_t *)a0_gc_buffer_realloc(a->data, sizeof(uint64_t) * (size_t)a->cap,
                                                   sizeof(uint64_t) * (size_t)cap);
        a->cap = cap;
    }
    a->data[a->len++] = value;
}

uint64_t a0_array_pop(A0Array *a) {
    check(a);
    if (a->len == 0) a0_panic("pop from an empty array");
    return a->data[--a->len];
}

void a0_array_trace(A0Array *a) {
    if (a->elem_kind == A0_WORD_RAW) return;
    for (int64_t i = 0; i < a->len; i++) a0_gc_mark_word(a->data[i], a->elem_kind);
}

void a0_array_finalize(A0Array *a) {
    if (a->data != NULL) a0_gc_buffer_free(a->data, sizeof(uint64_t) * (size_t)a->cap);
}
