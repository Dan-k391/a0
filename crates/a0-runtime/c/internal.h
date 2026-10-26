/* Declarations shared by the runtime's C files; not part of the public API. */
#ifndef A0RT_INTERNAL_H
#define A0RT_INTERNAL_H

#include "a0rt.h"

#include <stddef.h>

/* Built-in object types. */
extern const A0Type a0_builtin_string;
extern const A0Type a0_builtin_array;
extern const A0Type a0_builtin_map;
extern const A0Type a0_builtin_int;

typedef struct {
    int64_t value;
} A0BoxedInt;

/* Every object is preceded by a header; object pointers point at the payload. Small objects (up to
 * 256 bytes with the header) live in cells of chunks, which the sweep walks; big ones are also
 * preceded by a BigObject that links them for the sweep. */
typedef struct Header {
    uint32_t size;                      /* header + payload, rounded up to a cell, for accounting */
    uint32_t marked;
    A0_WORD_PTR(const A0Type *, type); /* read by generated code, just before the payload */
} Header;

typedef struct BigObject {
    A0_WORD_PTR(struct BigObject *, next);
    uint64_t size; /* everything allocated, BigObject included */
} BigObject;

#define HEADER(p) ((Header *)(p) - 1)
#define PAYLOAD(h) ((void *)((h) + 1))

/* Allocates an object with `payload_size` zeroed bytes. May collect first. */
void *a0_gc_alloc(const A0Type *type, size_t payload_size);

/* Memory owned by an object (array elements, map tables): counted toward the next collection, but
 * never starts one, so growing a buffer needs no rooting. */
void *a0_gc_buffer_alloc(size_t size);
void *a0_gc_buffer_realloc(void *buffer, size_t old_size, size_t new_size);
void a0_gc_buffer_free(void *buffer, size_t size);

/* Marking, for the tracers of built-in kinds. */
void a0_gc_mark_object(const void *object);
void a0_gc_mark_word(uint64_t word, int64_t word_kind);

/* Called by the collector for built-in kinds. */
void a0_array_trace(A0Array *a);
void a0_array_finalize(A0Array *a);
void a0_map_trace(A0Map *m);
void a0_map_finalize(A0Map *m);

uint64_t a0_string_hash(A0String *s);

/* Tells a debugger of a failure, before it is reported (rt.c). */
void a0_rt_tell_failure(const uint8_t *message, size_t len);

/* An `any` as text in `buf` (at most `cap` bytes with the NUL): a string's own text, a number, or
 * the type of other values. Returns the length it wanted. */
size_t a0_any_describe(uint64_t value, char *buf, size_t cap);

/* For failure paths: kept out of the functions that fail, so that their fast paths need no stack
 * buffers for messages. */
#if defined(_MSC_VER)
#define A0_COLD __declspec(noinline)
#else
#define A0_COLD __attribute__((noinline, cold))
#endif

#define A0_ANY_IS_REF(w) (((w) & A0_ANY_REF_TAG) == A0_ANY_REF_TAG)
#define A0_ANY_PTR(w) ((void *)(uintptr_t)((w) & 0x0000ffffffffffffull))
#define A0_ANY_IS_FLOAT(w) (((w) & A0_ANY_QNAN) != A0_ANY_QNAN)

#endif
