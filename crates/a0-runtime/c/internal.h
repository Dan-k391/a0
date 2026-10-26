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
extern const A0Type a0_builtin_object;

typedef struct {
    int64_t value;
} A0BoxedInt;

/* Every object is preceded by a header; object pointers point at the payload. Small objects (up to
 * 256 bytes with the header) live in cells of chunks, which the sweep walks; big ones are also
 * preceded by a BigObject that links them for the sweep. */
typedef struct Header {
    uint32_t size;                      /* header + payload, rounded up to a cell, for accounting */
    uint16_t marked;
    uint16_t flags; /* A0_FLAG_* */
    A0_WORD_PTR(const A0Type *, type); /* read by generated code, just before the payload */
} Header;

/* An interned string (object.c), kept for the rest of the program. */
#define A0_FLAG_INTERNED 1

typedef struct BigObject {
    A0_WORD_PTR(struct BigObject *, next);
    A0_WORD_PTR(struct BigObject *, prev); /* so that `a0_gc_free` unlinks one at once */
    uint64_t size;                         /* everything allocated, BigObject included */
    uint64_t reserved;
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
void a0_object_trace(A0Object *o);
void a0_object_finalize(A0Object *o);
/* Marks every interned string: they live for the rest of the program. */
void a0_intern_mark(void);

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

/* ---- Threads (thread.c, D76) ------------------------------------------------------------------ */

#if defined(__wasi__) || defined(__wasm__)
#define A0_NO_THREADS 1
#endif

#if defined(A0_NO_THREADS)
#define A0_THREAD_LOCAL
#elif defined(_MSC_VER)
#define A0_THREAD_LOCAL __declspec(thread)
#else
#define A0_THREAD_LOCAL _Thread_local
#endif

/* Sequentially consistent operations on 64-bit words shared between threads. */
int64_t a0_atomic_load(volatile int64_t *p);
void a0_atomic_store(volatile int64_t *p, int64_t v);
int64_t a0_atomic_add(volatile int64_t *p, int64_t delta); /* returns the new value */

/* Set (to 1, for good) before the program's second thread starts; until then nothing locks. */
extern int a0_threads_active;
/* Set while a collection waits for the other threads to stop; generated code polls it. */
extern volatile int64_t a0_gc_stop_requested;

/* The heap: allocation and collection. Entering waits as a stopped thread would (everything the
 * thread refers to is rooted where it allocates), so it is only entered where a collection could
 * start anyway. Reentrant on one thread. */
void a0_heap_enter(void);
void a0_heap_enter_unattached(void);
void a0_heap_leave(void);
/* The collector's count of threads changed (gc.c), or starts over (a0_rt_init). */
void a0_threads_attached(int64_t delta);
void a0_threads_reset(void);
void a0_gc_safepoint(void);
/* Short critical sections over the runtime's own tables (interned names, shapes, buffers): never
 * held where a collection could start, so waiting for it does not count as stopped. */
void a0_table_lock(void);
void a0_table_unlock(void);
/* A region where the thread waits (for a channel, a thread, time, input) and touches no object:
 * a collection may run meanwhile. Leaving waits for a collection in progress to end. */
void a0_safe_begin(void);
void a0_safe_end(void);
/* Waits until every other thread has stopped (the caller holds the heap), and lets them go. */
void a0_stop_world(void);
void a0_start_world(void);
/* Only one thread reports a failure; the others wait for the process to end. */
void a0_fail_lock(void);
/* Marks what threads keep alive (the handles of running threads). */
void a0_threads_mark(void);
/* The trace lines of the current thread's start, if it is not the main thread (trace.c). */
size_t a0_thread_spawn_frames(const uint32_t **ids, int64_t *id);

/* The collector's view of a thread (gc.c): its shadow stack and its exception in flight. */
void a0_gc_attach(int64_t shadow_capacity);
void a0_gc_detach(void);
void a0_gc_restart(void);
/* The current thread's exception slots (pending flag, value), roots of the collector. */
uint64_t *a0_gc_exception_slots(void);

#define A0_ANY_IS_REF(w) (((w) & A0_ANY_REF_TAG) == A0_ANY_REF_TAG)
#define A0_ANY_PTR(w) ((void *)(uintptr_t)((w) & 0x0000ffffffffffffull))
#define A0_ANY_IS_FLOAT(w) (((w) & A0_ANY_QNAN) != A0_ANY_QNAN)

#endif
