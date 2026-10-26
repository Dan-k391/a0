/* Arrays: growable vectors of 64-bit words, or of bytes (A0_ELEM_BYTE, for `array<bool>`). */
#include "internal.h"

#include <stdio.h>
#include <string.h>

static A0Array *check(A0Array *a) {
    if (a == NULL) a0_panic_null();
    return a;
}

static size_t elem_size(int64_t elem_kind) {
    if (elem_kind >= A0_ELEM_BYTES) return (size_t)(elem_kind - A0_ELEM_BYTES);
    switch (elem_kind) {
    case A0_ELEM_BYTE: return 1;
    case A0_ELEM_16: return 2;
    case A0_ELEM_32: return 4;
    default: return sizeof(uint64_t);
    }
}

static size_t bytes_of(A0Array *a, int64_t count) { return elem_size(a->elem_kind) * (size_t)count; }

/* Small arrays keep their first elements inline, after the header, so creating one is a single
 * allocation; growing past them moves the elements to a buffer. */
#define INLINE_LIMIT 16

static int is_inline(A0Array *a) { return a->data == (uint64_t *)(a + 1); }

A0Array *a0_array_new(int64_t elem_kind, int64_t capacity) {
    if (capacity > 0 && capacity <= INLINE_LIMIT) {
        size_t size = sizeof(A0Array) + elem_size(elem_kind) * (size_t)capacity;
        A0Array *a = (A0Array *)a0_gc_alloc(&a0_builtin_array, size);
        a->elem_kind = elem_kind;
        a->data = (uint64_t *)(a + 1);
        a->cap = capacity;
        return a;
    }
    A0Array *a = (A0Array *)a0_gc_alloc(&a0_builtin_array, sizeof(A0Array));
    a->elem_kind = elem_kind;
    if (capacity > 0) {
        a->data = (uint64_t *)a0_gc_buffer_alloc(elem_size(elem_kind) * (size_t)capacity);
        a->cap = capacity;
    }
    return a;
}

static void store(A0Array *a, int64_t index, uint64_t value);

A0Array *a0_array_filled(int64_t elem_kind, int64_t count, uint64_t value, uint64_t loc, void *frame) {
    if (count < 0) {
        char message[96];
        snprintf(message, sizeof message, "negative array length: %lld", (long long)count);
        a0_rt_fail(message, loc, frame);
    }
    /* A reference value stays reachable: the caller keeps it rooted while the array is allocated. */
    A0Array *a = a0_array_new(elem_kind, count);
    a->len = count;
    if (elem_kind == A0_ELEM_BYTE) {
        memset(a->data, (int)(uint8_t)value, (size_t)count);
    } else if (value != 0) {
        for (int64_t i = 0; i < count; i++) store(a, i, value);
    }
    return a;
}

int64_t a0_array_len(A0Array *a) { return check(a)->len; }

static uint64_t load(A0Array *a, int64_t index) {
    switch (a->elem_kind) {
    case A0_ELEM_BYTE: return ((uint8_t *)a->data)[index];
    case A0_ELEM_16: return ((uint16_t *)a->data)[index];
    case A0_ELEM_32: return ((uint32_t *)a->data)[index];
    default: return a->data[index];
    }
}

static void store(A0Array *a, int64_t index, uint64_t value) {
    switch (a->elem_kind) {
    case A0_ELEM_BYTE: ((uint8_t *)a->data)[index] = (uint8_t)value; break;
    case A0_ELEM_16: ((uint16_t *)a->data)[index] = (uint16_t)value; break;
    case A0_ELEM_32: ((uint32_t *)a->data)[index] = (uint32_t)value; break;
    default: a->data[index] = value;
    }
}

uint64_t a0_array_get(A0Array *a, int64_t index, uint64_t loc, void *frame) {
    check(a);
    if (index < 0 || index >= a->len) a0_rt_out_of_bounds(a->len, index, loc, frame);
    return load(a, index);
}

void a0_array_set(A0Array *a, int64_t index, uint64_t value, uint64_t loc, void *frame) {
    check(a);
    if (index < 0 || index >= a->len) a0_rt_out_of_bounds(a->len, index, loc, frame);
    store(a, index, value);
}

void a0_array_push(A0Array *a, uint64_t value) {
    check(a);
    if (a->len == a->cap) {
        int64_t cap = a->cap < 8 ? 8 : a->cap * 2;
        if (is_inline(a)) {
            uint64_t *data = (uint64_t *)a0_gc_buffer_alloc(bytes_of(a, cap));
            memcpy(data, a->data, bytes_of(a, a->len));
            a->data = data;
        } else {
            a->data = (uint64_t *)a0_gc_buffer_realloc(a->data, bytes_of(a, a->cap), bytes_of(a, cap));
        }
        a->cap = cap;
    }
    store(a, a->len++, value);
}

uint64_t a0_array_pop(A0Array *a, uint64_t loc, void *frame) {
    check(a);
    if (a->len == 0) a0_rt_fail("pop from an empty array", loc, frame);
    return load(a, --a->len);
}

/* Elements of A0_ELEM_BYTES + size bytes each (inline structs): appended, removed and filled by
 * address. */
static void *element(A0Array *a, int64_t index) { return (char *)a->data + bytes_of(a, index); }

static void grow(A0Array *a) {
    int64_t cap = a->cap < 8 ? 8 : a->cap * 2;
    if (is_inline(a)) {
        uint64_t *data = (uint64_t *)a0_gc_buffer_alloc(bytes_of(a, cap));
        memcpy(data, a->data, bytes_of(a, a->len));
        a->data = data;
    } else {
        a->data = (uint64_t *)a0_gc_buffer_realloc(a->data, bytes_of(a, a->cap), bytes_of(a, cap));
    }
    a->cap = cap;
}

void a0_array_push_bytes(A0Array *a, const void *value) {
    check(a);
    if (a->len == a->cap) grow(a);
    memmove(element(a, a->len), value, bytes_of(a, 1));
    a->len++;
}

void *a0_array_pop_bytes(A0Array *a, uint64_t loc, void *frame) {
    check(a);
    if (a->len == 0) a0_rt_fail("pop from an empty array", loc, frame);
    a->len--;
    return element(a, a->len);
}

A0Array *a0_array_filled_bytes(int64_t elem_kind, int64_t count, const void *value, uint64_t loc,
                               void *frame) {
    if (count < 0) {
        char message[96];
        snprintf(message, sizeof message, "negative array length: %lld", (long long)count);
        a0_rt_fail(message, loc, frame);
    }
    A0Array *a = a0_array_new(elem_kind, count);
    a->len = count;
    size_t size = elem_size(elem_kind);
    for (int64_t i = 0; i < count; i++) memcpy(element(a, i), value, size);
    return a;
}

void a0_array_trace(A0Array *a) {
    if (a->elem_kind != A0_WORD_REF && a->elem_kind != A0_WORD_ANY) return;
    for (int64_t i = 0; i < a->len; i++) a0_gc_mark_word(a->data[i], a->elem_kind);
}

void a0_array_finalize(A0Array *a) {
    if (a->data != NULL && !is_inline(a)) a0_gc_buffer_free(a->data, bytes_of(a, a->cap));
}
