/* Arrays: growable vectors of 64-bit words. */
#include "internal.h"

#include <string.h>

static A0Array *check(A0Array *a) {
    if (a == NULL) a0_panic_null();
    return a;
}

/* Small arrays keep their first elements inline, after the header, so creating one is a single
 * allocation; growing past them moves the elements to a buffer. */
#define INLINE_LIMIT 16

static int is_inline(A0Array *a) { return a->data == (uint64_t *)(a + 1); }

A0Array *a0_array_new(int64_t elem_kind, int64_t capacity) {
    if (capacity > 0 && capacity <= INLINE_LIMIT) {
        size_t size = sizeof(A0Array) + sizeof(uint64_t) * (size_t)capacity;
        A0Array *a = (A0Array *)a0_gc_alloc(&a0_builtin_array, size);
        a->elem_kind = elem_kind;
        a->data = (uint64_t *)(a + 1);
        a->cap = capacity;
        return a;
    }
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
        if (is_inline(a)) {
            uint64_t *data = (uint64_t *)a0_gc_buffer_alloc(sizeof(uint64_t) * (size_t)cap);
            memcpy(data, a->data, sizeof(uint64_t) * (size_t)a->len);
            a->data = data;
        } else {
            a->data = (uint64_t *)a0_gc_buffer_realloc(a->data, sizeof(uint64_t) * (size_t)a->cap,
                                                       sizeof(uint64_t) * (size_t)cap);
        }
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
    if (a->data != NULL && !is_inline(a)) a0_gc_buffer_free(a->data, sizeof(uint64_t) * (size_t)a->cap);
}
