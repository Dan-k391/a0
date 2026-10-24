/* The garbage collector: object heap, type descriptors, shadow stack, roots and mark-sweep. */
#include "internal.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Every object is preceded by a header; object pointers point at the payload. */
typedef struct Header {
    struct Header *next;
    const A0Type *type;
    uint64_t size; /* header + payload, for accounting */
    uint32_t marked;
    uint32_t reserved;
} Header;

#define HEADER(p) ((Header *)(p) - 1)
#define PAYLOAD(h) ((void *)((h) + 1))

const A0Type a0_builtin_string = {"string", A0_KIND_STRING, 0, NULL, &a0_builtin_string};
const A0Type a0_builtin_array = {"array", A0_KIND_ARRAY, 0, NULL, &a0_builtin_array};
const A0Type a0_builtin_map = {"map", A0_KIND_MAP, 0, NULL, &a0_builtin_map};
const A0Type a0_builtin_int = {"int", A0_KIND_INT, 0, NULL, &a0_builtin_int};

static Header *objects;
static size_t bytes_allocated;
static size_t next_gc = 1024 * 1024;
static int stress = -1;
static int64_t collections;
static int64_t live_objects;

static const void **gray;
static int64_t gray_count, gray_capacity;

/* Shadow stack: words in the `any` encoding, scanned as roots. */
#define SHADOW_CAPACITY (4 * 1024 * 1024)
static uint64_t *shadow;
static int64_t shadow_top;

typedef struct {
    void *address;
    int64_t kind;
} Root;
static Root *roots;
static int64_t root_count, root_capacity;

/* ---- Type descriptors ------------------------------------------------------------------------- */

A0Type *a0_type_new(const uint8_t *name, int64_t name_len, int64_t word_count, const uint8_t *layout,
                    A0Type *family) {
    A0Type *t = (A0Type *)calloc(1, sizeof(A0Type));
    char *n = (char *)malloc((size_t)name_len + 1);
    uint8_t *l = (uint8_t *)malloc(word_count > 0 ? (size_t)word_count : 1);
    if (t == NULL || n == NULL || l == NULL) a0_panic("out of memory");
    memcpy(n, name, (size_t)name_len);
    n[name_len] = '\0';
    if (word_count > 0) memcpy(l, layout, (size_t)word_count);
    t->name = n;
    t->kind = A0_KIND_FIXED;
    t->word_count = (int32_t)word_count;
    t->layout = l;
    t->family = family != NULL ? family : t;
    return t;
}

const A0Type *a0_gc_type_of(const void *object) { return HEADER(object)->type; }

/* ---- Allocation ------------------------------------------------------------------------------- */

static void maybe_collect(size_t incoming) {
    if (stress < 0) {
        const char *s = getenv("A0_GC_STRESS");
        stress = s != NULL && s[0] != '\0' && s[0] != '0';
    }
    if (stress || bytes_allocated + incoming > next_gc) a0_gc_collect();
}

void *a0_gc_alloc(const A0Type *type, size_t payload_size) {
    size_t size = sizeof(Header) + payload_size;
    maybe_collect(size);
    Header *h = (Header *)calloc(1, size);
    if (h == NULL) a0_panic("out of memory");
    h->type = type;
    h->size = size;
    h->next = objects;
    objects = h;
    bytes_allocated += size;
    return PAYLOAD(h);
}

void *a0_gc_new(A0Type *type) { return a0_gc_alloc(type, sizeof(uint64_t) * (size_t)type->word_count); }

void *a0_gc_buffer_alloc(size_t size) {
    void *p = calloc(1, size > 0 ? size : 1);
    if (p == NULL) a0_panic("out of memory");
    bytes_allocated += size;
    return p;
}

void *a0_gc_buffer_realloc(void *buffer, size_t old_size, size_t new_size) {
    void *p = realloc(buffer, new_size > 0 ? new_size : 1);
    if (p == NULL) a0_panic("out of memory");
    if (new_size > old_size) memset((char *)p + old_size, 0, new_size - old_size);
    bytes_allocated += new_size - old_size;
    return p;
}

void a0_gc_buffer_free(void *buffer, size_t size) {
    free(buffer);
    bytes_allocated -= size;
}

/* ---- Roots ------------------------------------------------------------------------------------ */

uint64_t *a0_gc_frame_push(int64_t slots) {
    if (shadow == NULL) {
        shadow = (uint64_t *)calloc(SHADOW_CAPACITY, sizeof(uint64_t));
        if (shadow == NULL) a0_panic("out of memory");
    }
    if (shadow_top + slots > SHADOW_CAPACITY) a0_rt_trap(A0_TRAP_STACK_OVERFLOW);
    uint64_t *frame = &shadow[shadow_top];
    /* Zero is the float 0.0 in the `any` encoding: not a reference, so no stale pointers survive. */
    memset(frame, 0, sizeof(uint64_t) * (size_t)slots);
    shadow_top += slots;
    return frame;
}

void a0_gc_frame_pop(int64_t slots) { shadow_top -= slots; }

void a0_gc_add_root(void *address, int64_t word_kind) {
    if (root_count == root_capacity) {
        root_capacity = root_capacity < 16 ? 16 : root_capacity * 2;
        roots = (Root *)realloc(roots, sizeof(Root) * (size_t)root_capacity);
        if (roots == NULL) a0_panic("out of memory");
    }
    roots[root_count].address = address;
    roots[root_count].kind = word_kind;
    root_count++;
}

/* ---- Marking ---------------------------------------------------------------------------------- */

void a0_gc_mark_object(const void *object) {
    if (object == NULL) return;
    Header *h = HEADER(object);
    if (h->marked) return;
    h->marked = 1;
    if (gray_count == gray_capacity) {
        gray_capacity = gray_capacity < 64 ? 64 : gray_capacity * 2;
        gray = (const void **)realloc((void *)gray, sizeof(void *) * (size_t)gray_capacity);
        if (gray == NULL) a0_panic("out of memory");
    }
    gray[gray_count++] = object;
}

void a0_gc_mark_word(uint64_t word, int64_t word_kind) {
    if (word_kind == A0_WORD_REF) {
        a0_gc_mark_object((const void *)(uintptr_t)word);
    } else if (word_kind == A0_WORD_ANY && A0_ANY_IS_REF(word)) {
        a0_gc_mark_object(A0_ANY_PTR(word));
    }
}

static void trace(const void *object) {
    const A0Type *t = HEADER(object)->type;
    switch (t->kind) {
    case A0_KIND_FIXED: {
        const uint64_t *words = (const uint64_t *)object;
        for (int32_t i = 0; i < t->word_count; i++) a0_gc_mark_word(words[i], t->layout[i]);
        break;
    }
    case A0_KIND_ARRAY: a0_array_trace((A0Array *)object); break;
    case A0_KIND_MAP: a0_map_trace((A0Map *)object); break;
    default: break;
    }
}

static void finalize(void *object) {
    const A0Type *t = HEADER(object)->type;
    if (t->kind == A0_KIND_ARRAY) a0_array_finalize((A0Array *)object);
    if (t->kind == A0_KIND_MAP) a0_map_finalize((A0Map *)object);
}

void a0_gc_collect(void) {
    for (int64_t i = 0; i < shadow_top; i++) a0_gc_mark_word(shadow[i], A0_WORD_ANY);
    for (int64_t i = 0; i < root_count; i++) {
        a0_gc_mark_word(*(uint64_t *)roots[i].address, roots[i].kind);
    }
    while (gray_count > 0) trace(gray[--gray_count]);

    int64_t live = 0;
    Header **link = &objects;
    while (*link != NULL) {
        Header *h = *link;
        if (h->marked) {
            h->marked = 0;
            live++;
            link = &h->next;
        } else {
            *link = h->next;
            finalize(PAYLOAD(h));
            bytes_allocated -= h->size;
            free(h);
        }
    }
    live_objects = live;
    collections++;
    next_gc = bytes_allocated * 2 < 1024 * 1024 ? 1024 * 1024 : bytes_allocated * 2;
}

int64_t a0_gc_collections(void) { return collections; }

int64_t a0_gc_live_objects(void) { return live_objects; }

void a0_panic_null(void) { a0_panic("null reference"); }
