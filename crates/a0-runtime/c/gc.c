/* The garbage collector: object heap, type descriptors, shadow stack, roots and mark-sweep. */
#include "internal.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

/* Generated code relies on these offsets (see A0Type in a0rt.h); a wrong one fails to compile. */
typedef char a0_object_type_offset_is_8[(sizeof(Header) - offsetof(Header, type) == 8) ? 1 : -1];
typedef char a0_header_is_16[(sizeof(Header) == 16) ? 1 : -1];
typedef char a0_type_family_offset_is_24[(offsetof(A0Type, family) == 24) ? 1 : -1];
typedef char a0_type_itable_offset_is_32[(offsetof(A0Type, itable) == 32) ? 1 : -1];
typedef char a0_array_data_offset_is_16[(offsetof(A0Array, data) == 16) ? 1 : -1];

#define BUILTIN(var, text, k) const A0Type var = {.name = text, .kind = k, .family = &var}
BUILTIN(a0_builtin_string, "string", A0_KIND_STRING);
BUILTIN(a0_builtin_array, "array", A0_KIND_ARRAY);
BUILTIN(a0_builtin_map, "map", A0_KIND_MAP);
BUILTIN(a0_builtin_int, "int", A0_KIND_INT);

static BigObject *big_objects;
static size_t bytes_allocated;
/* See the end of a0_gc_collect. */
#define GROWTH 3
#define MIN_HEAP (1024 * 1024)
static size_t next_gc = MIN_HEAP;
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

/* A0_GC_STATS=1 prints what the collector did when the program ends, to standard error. */
static double gc_time;
static double mark_time;
static size_t peak_bytes;

static void print_stats(void) {
    fprintf(stderr,
            "gc: %lld collections, %.1f ms (%.1f marking), peak %.1f MB, %lld objects live after the "
            "last\n",
            (long long)collections, 1000.0 * gc_time, 1000.0 * mark_time,
            (double)peak_bytes / (1024.0 * 1024.0),
            (long long)live_objects);
}

static int enabled(const char *name) {
    const char *s = getenv(name);
    return s != NULL && s[0] != '\0' && s[0] != '0';
}

static void maybe_collect(size_t incoming) {
    if (stress < 0) {
        stress = enabled("A0_GC_STRESS");
        if (enabled("A0_GC_STATS")) atexit(print_stats);
    }
    if (bytes_allocated > peak_bytes) peak_bytes = bytes_allocated;
    if (stress || bytes_allocated + incoming > next_gc) a0_gc_collect();
}

/* Small objects come from per-size free lists carved out of chunks, so allocating one is a list pop
 * and freeing it a list push; bigger ones use malloc. The collector sweeps small objects chunk by
 * chunk, through memory in order (a free cell has a null type), and only big ones through the
 * `objects` list. Buffers have chunks of their own, which are not swept. Chunks are never returned
 * to the system. */
#define GRANULE 16
#define SMALL_LIMIT 256
#define CHUNK_SIZE (64 * 1024)
#define CLASSES (SMALL_LIMIT / GRANULE + 1)

typedef struct FreeCell {
    struct FreeCell *next;
} FreeCell;

static FreeCell *object_cells[CLASSES];
static FreeCell *buffer_cells[CLASSES];

typedef struct {
    char *base;
    size_t cell;
} Chunk;

static Chunk *chunks;
static size_t chunk_count, chunk_cap;

/* `any` keeps only the low 48 bits of a reference (see a0rt.h). User-space addresses fit on every
 * supported target; check rather than corrupt references silently where they might not. */
static void *check_address(void *p, size_t size) {
    if (p == NULL) a0_panic("out of memory");
    if (((uint64_t)(uintptr_t)p + size) >> 48 != 0) a0_panic("heap address does not fit in 48 bits");
    return p;
}

/* A zeroed cell of `size` bytes (a multiple of GRANULE, at most SMALL_LIMIT) from `lists`; a new
 * chunk of objects is recorded for the sweep. */
static void *small_alloc(FreeCell **lists, size_t size, int objects) {
    size_t cls = size / GRANULE;
    if (lists[cls] == NULL) {
        /* Zeroed, so that every cell's type is null until it is allocated. */
        char *chunk = (char *)check_address(calloc(1, CHUNK_SIZE), CHUNK_SIZE);
        if (objects) {
            if (chunk_count == chunk_cap) {
                chunk_cap = chunk_cap == 0 ? 64 : chunk_cap * 2;
                chunks = (Chunk *)realloc(chunks, sizeof(Chunk) * chunk_cap);
                if (chunks == NULL) a0_panic("out of memory");
            }
            chunks[chunk_count].base = chunk;
            chunks[chunk_count].cell = size;
            chunk_count++;
        }
        /* Pushed from the end, so that cells are handed out in address order. */
        for (size_t off = (CHUNK_SIZE / size - 1) * size;; off -= size) {
            FreeCell *c = (FreeCell *)(chunk + off);
            c->next = lists[cls];
            lists[cls] = c;
            if (off == 0) break;
        }
    }
    FreeCell *c = lists[cls];
    lists[cls] = c->next;
    memset(c, 0, size);
    return c;
}

static void small_free(FreeCell **lists, void *p, size_t size) {
    /* Under stress, poison dead memory so a missing root shows up as garbage, not stale data. */
    if (stress) memset(p, 0xdd, size);
    FreeCell *c = (FreeCell *)p;
    c->next = lists[size / GRANULE];
    lists[size / GRANULE] = c;
}

/* A dead small object's cell goes back to its free list, its type null again. */
static void object_cell_free(Header *h, size_t size) {
    small_free(object_cells, h, size);
    h->type = NULL;
}

static size_t rounded(size_t size) { return (size + GRANULE - 1) / GRANULE * GRANULE; }

void *a0_gc_alloc(const A0Type *type, size_t payload_size) {
    size_t size = (sizeof(Header) + payload_size + GRANULE - 1) / GRANULE * GRANULE;
    maybe_collect(size);
    Header *h;
    if (size <= SMALL_LIMIT) {
        h = (Header *)small_alloc(object_cells, size, 1);
        h->size = (uint32_t)size;
    } else {
        size += sizeof(BigObject);
        BigObject *big = (BigObject *)check_address(calloc(1, size), size);
        big->next = big_objects;
        big->size = size;
        big_objects = big;
        h = (Header *)(big + 1);
    }
    h->type = type;
    bytes_allocated += size;
    return PAYLOAD(h);
}

void *a0_gc_new(A0Type *type) { return a0_gc_alloc(type, sizeof(uint64_t) * (size_t)type->word_count); }

/* Buffers (array elements, map tables) up to SMALL_LIMIT bytes come from the same free lists as
 * small objects. */
void *a0_gc_buffer_alloc(size_t size) {
    size_t r = rounded(size > 0 ? size : 1);
    void *p = r <= SMALL_LIMIT ? small_alloc(buffer_cells, r, 0) : calloc(1, r);
    if (p == NULL) a0_panic("out of memory");
    bytes_allocated += size;
    return p;
}

void *a0_gc_buffer_realloc(void *buffer, size_t old_size, size_t new_size) {
    if (rounded(old_size > 0 ? old_size : 1) > SMALL_LIMIT && rounded(new_size) > SMALL_LIMIT) {
        void *p = realloc(buffer, new_size);
        if (p == NULL) a0_panic("out of memory");
        if (new_size > old_size) memset((char *)p + old_size, 0, new_size - old_size);
        bytes_allocated += new_size - old_size;
        return p;
    }
    void *p = a0_gc_buffer_alloc(new_size);
    if (buffer != NULL) {
        memcpy(p, buffer, old_size < new_size ? old_size : new_size);
        a0_gc_buffer_free(buffer, old_size);
    }
    return p;
}

void a0_gc_buffer_free(void *buffer, size_t size) {
    if (buffer == NULL) return;
    size_t r = rounded(size > 0 ? size : 1);
    if (r <= SMALL_LIMIT) {
        small_free(buffer_cells, buffer, r);
    } else {
        free(buffer);
    }
    bytes_allocated -= size;
}

/* ---- Roots ------------------------------------------------------------------------------------ */

uint64_t *a0_gc_frame_push(int64_t slots) {
    if (shadow == NULL) {
        shadow = (uint64_t *)calloc(SHADOW_CAPACITY, sizeof(uint64_t));
        if (shadow == NULL) a0_panic("out of memory");
    }
    if (shadow_top + slots > SHADOW_CAPACITY) a0_rt_trap(A0_TRAP_STACK_OVERFLOW, 0, NULL);
    /* Every word above the top is zero (the float 0.0 in the `any` encoding, not a reference), since
     * frames are cleared before they are popped: the new frame needs no clearing. */
    uint64_t *frame = &shadow[shadow_top];
    shadow_top += slots;
    return frame;
}

void a0_gc_frame_pop(int64_t slots) {
    shadow_top -= slots;
    if (stress == 1) {
        for (int64_t i = 0; i < slots; i++) {
            if (shadow[shadow_top + i] != 0) a0_panic("a shadow stack frame was popped uncleared");
        }
    }
}

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

/* Marking reads every live object's header once, and those reads are cache misses. So an object is
 * pushed unmarked, with its header prefetched, and checked when it comes out of a small queue
 * behind the gray stack; by then its header has arrived. An object can then be pushed more than
 * once, which costs a word of stack. */
#if defined(__GNUC__) || defined(__clang__)
#define PREFETCH(p) __builtin_prefetch(p)
#elif defined(_MSC_VER) && (defined(_M_X64) || defined(_M_IX86))
#include <xmmintrin.h>
#define PREFETCH(p) _mm_prefetch((const char *)(p), _MM_HINT_T0)
#else
#define PREFETCH(p) ((void)(p))
#endif

void a0_gc_mark_object(const void *object) {
    if (object == NULL) return;
    PREFETCH(HEADER(object));
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

#define PREFETCH_DEPTH 8

/* Marks and traces everything reachable from the gray stack (see a0_gc_mark_object). */
static void drain_gray(void) {
    const void *queue[PREFETCH_DEPTH];
    int head = 0, count = 0;
    for (;;) {
        while (count < PREFETCH_DEPTH && gray_count > 0) {
            queue[(head + count) % PREFETCH_DEPTH] = gray[--gray_count];
            count++;
        }
        if (count == 0) return;
        const void *object = queue[head];
        head = (head + 1) % PREFETCH_DEPTH;
        count--;
        Header *h = HEADER(object);
        if (h->marked) continue;
        h->marked = 1;
        trace(object);
    }
}

static void finalize(void *object) {
    const A0Type *t = HEADER(object)->type;
    if (t->kind == A0_KIND_ARRAY) a0_array_finalize((A0Array *)object);
    if (t->kind == A0_KIND_MAP) a0_map_finalize((A0Map *)object);
}

void a0_gc_collect(void) {
    double start = a0_clock();
    for (int64_t i = 0; i < shadow_top; i++) a0_gc_mark_word(shadow[i], A0_WORD_ANY);
    for (int64_t i = 0; i < root_count; i++) {
        a0_gc_mark_word(*(uint64_t *)roots[i].address, roots[i].kind);
    }
    drain_gray();
    mark_time += a0_clock() - start;

    int64_t live = 0;
    for (size_t k = 0; k < chunk_count; k++) {
        char *base = chunks[k].base;
        size_t cell = chunks[k].cell;
        for (size_t off = 0; off + cell <= CHUNK_SIZE; off += cell) {
            Header *h = (Header *)(base + off);
            if (h->type == NULL) continue;
            if (h->marked) {
                h->marked = 0;
                live++;
            } else {
                finalize(PAYLOAD(h));
                bytes_allocated -= cell;
                object_cell_free(h, cell);
            }
        }
    }
    BigObject **link = &big_objects;
    while (*link != NULL) {
        BigObject *big = *link;
        Header *h = (Header *)(big + 1);
        if (h->marked) {
            h->marked = 0;
            live++;
            link = &big->next;
        } else {
            *link = big->next;
            finalize(PAYLOAD(h));
            bytes_allocated -= big->size;
            free(big);
        }
    }
    live_objects = live;
    collections++;
    gc_time += a0_clock() - start;
    /* The next collection comes when the heap has grown to GROWTH times what survived this one: a
     * collection costs about what survives, so the collector's share of the time goes as
     * 1 / (GROWTH - 1), and the heap peaks at about GROWTH times the live data. */
    size_t want = bytes_allocated * GROWTH;
    next_gc = want < MIN_HEAP ? MIN_HEAP : want;
}

int64_t a0_gc_collections(void) { return collections; }

int64_t a0_gc_live_objects(void) { return live_objects; }

void a0_panic_null(void) { a0_panic("null reference"); }
