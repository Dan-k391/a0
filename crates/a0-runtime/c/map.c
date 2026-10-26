/* Maps: hash maps that remember insertion order.
 *
 * Entries are appended to a dense array (so iteration follows insertion order); an open-addressing
 * index of entry numbers finds them by hash. Removal leaves a dead entry and a tombstone, both cleaned
 * up the next time the map is rebuilt. */
#include "internal.h"

#include <stdio.h>
#include <string.h>

typedef struct {
    uint64_t key;
    uint64_t value;
    uint64_t hash;
    int64_t live;
} Entry;

#define EMPTY (-1)
#define TOMBSTONE (-2)

/* Pointers are word slots, so that debuggers read maps the same way on every target. */
struct A0Map {
    int64_t count;      /* live entries */
    int64_t used;       /* entries in use, including dead ones */
    int64_t entry_cap;
    A0_WORD_PTR(Entry *, entries);
    int64_t index_cap;  /* a power of two, or 0 */
    A0_WORD_PTR(int64_t *, index);
    int64_t key_kind;
    int64_t value_kind;
    /* The entry the last lookup found. `has(m, k)` followed by `m[k]` (the usual way to read a key
     * that may be missing) then hashes `k` once; the key word must match exactly. */
    int64_t last;
};

static A0Map *check(A0Map *m) {
    if (m == NULL) a0_panic_null();
    return m;
}

A0Map *a0_map_new(int64_t key_kind, int64_t value_kind) {
    A0Map *m = (A0Map *)a0_gc_alloc(&a0_builtin_map, sizeof(A0Map));
    m->key_kind = key_kind;
    m->value_kind = value_kind;
    m->last = -1;
    return m;
}

static uint64_t hash_key(A0Map *m, uint64_t key) {
    if (m->key_kind == A0_KEY_STRING) {
        A0String *s = (A0String *)(uintptr_t)key;
        if (s == NULL) a0_panic_null();
        return a0_string_hash(s);
    }
    /* splitmix64 finalizer */
    uint64_t z = key + 0x9e3779b97f4a7c15ull;
    z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ull;
    z = (z ^ (z >> 27)) * 0x94d049bb133111ebull;
    return z ^ (z >> 31);
}

static int keys_equal(A0Map *m, uint64_t a, uint64_t b) {
    if (m->key_kind == A0_KEY_STRING) {
        return (int)a0_string_eq((A0String *)(uintptr_t)a, (A0String *)(uintptr_t)b);
    }
    return a == b;
}

/* The entry number holding `key`, or -1. */
static int64_t find(A0Map *m, uint64_t key, uint64_t hash) {
    if (m->index_cap == 0) return -1;
    uint64_t mask = (uint64_t)m->index_cap - 1;
    for (uint64_t slot = hash & mask;; slot = (slot + 1) & mask) {
        int64_t e = m->index[slot];
        if (e == EMPTY) return -1;
        if (e >= 0 && m->entries[e].hash == hash && keys_equal(m, m->entries[e].key, key)) return e;
    }
}

static void index_insert(A0Map *m, int64_t entry) {
    uint64_t mask = (uint64_t)m->index_cap - 1;
    uint64_t slot = m->entries[entry].hash & mask;
    while (m->index[slot] >= 0) slot = (slot + 1) & mask;
    m->index[slot] = entry;
}

/* Drops dead entries and rebuilds the index with room for at least `need` entries. */
static void rebuild(A0Map *m, int64_t need) {
    int64_t live = 0;
    for (int64_t i = 0; i < m->used; i++) {
        if (m->entries[i].live) m->entries[live++] = m->entries[i];
    }
    m->used = live;
    m->last = -1;
    int64_t entry_cap = m->entry_cap < 8 ? 8 : m->entry_cap;
    while (entry_cap < need) entry_cap *= 2;
    if (entry_cap != m->entry_cap) {
        m->entries = (Entry *)a0_gc_buffer_realloc(m->entries, sizeof(Entry) * (size_t)m->entry_cap,
                                                   sizeof(Entry) * (size_t)entry_cap);
        m->entry_cap = entry_cap;
    }
    int64_t index_cap = 8;
    while (index_cap < entry_cap * 2) index_cap *= 2;
    if (m->index != NULL) a0_gc_buffer_free(m->index, sizeof(int64_t) * (size_t)m->index_cap);
    m->index = (int64_t *)a0_gc_buffer_alloc(sizeof(int64_t) * (size_t)index_cap);
    m->index_cap = index_cap;
    for (int64_t i = 0; i < index_cap; i++) m->index[i] = EMPTY;
    for (int64_t i = 0; i < m->used; i++) index_insert(m, i);
}

int64_t a0_map_len(A0Map *m) { return check(m)->count; }

/* The entry holding `key`, trying the last one found first. */
static int64_t lookup(A0Map *m, uint64_t key) {
    int64_t e = m->last;
    if (e >= 0 && e < m->used && m->entries[e].live && m->entries[e].key == key) return e;
    e = find(m, key, hash_key(m, key));
    if (e >= 0) m->last = e;
    return e;
}

int64_t a0_map_has(A0Map *m, uint64_t key) {
    check(m);
    return lookup(m, key) >= 0;
}

static A0_COLD A0_NORETURN void key_not_found(A0Map *m, uint64_t key, uint64_t loc, void *frame) {
    char message[160];
    if (m->key_kind == A0_KEY_STRING) {
        A0String *s = (A0String *)(uintptr_t)key;
        snprintf(message, sizeof message, "key not found: \"%.*s\"%s", s->len > 100 ? 100 : (int)s->len,
                 (const char *)s->bytes, s->len > 100 ? "..." : "");
    } else {
        snprintf(message, sizeof message, "key not found: %lld", (long long)key);
    }
    a0_rt_fail(message, loc, frame);
}

uint64_t a0_map_get(A0Map *m, uint64_t key, uint64_t loc, void *frame) {
    check(m);
    int64_t e = lookup(m, key);
    if (e < 0) key_not_found(m, key, loc, frame);
    return m->entries[e].value;
}

void a0_map_set(A0Map *m, uint64_t key, uint64_t value) {
    check(m);
    uint64_t hash = hash_key(m, key);
    int64_t e = find(m, key, hash);
    if (e >= 0) {
        m->entries[e].value = value;
        return;
    }
    if (m->used == m->entry_cap) rebuild(m, m->count + 1);
    e = m->used++;
    m->entries[e].key = key;
    m->entries[e].value = value;
    m->entries[e].hash = hash;
    m->entries[e].live = 1;
    m->count++;
    index_insert(m, e);
}

int64_t a0_map_remove(A0Map *m, uint64_t key) {
    check(m);
    uint64_t hash = hash_key(m, key);
    if (m->index_cap == 0) return 0;
    uint64_t mask = (uint64_t)m->index_cap - 1;
    for (uint64_t slot = hash & mask;; slot = (slot + 1) & mask) {
        int64_t e = m->index[slot];
        if (e == EMPTY) return 0;
        if (e >= 0 && m->entries[e].hash == hash && keys_equal(m, m->entries[e].key, key)) {
            m->index[slot] = TOMBSTONE;
            m->entries[e].live = 0;
            m->count--;
            return 1;
        }
    }
}

/* The caller keeps `m` rooted while the array is allocated. */
A0Array *a0_map_keys(A0Map *m) {
    check(m);
    int64_t kind = m->key_kind == A0_KEY_STRING ? A0_WORD_REF
                   : m->key_kind == A0_KEY_BOOL ? A0_ELEM_BYTE
                                                : A0_WORD_RAW;
    A0Array *a = a0_array_new(kind, m->count);
    for (int64_t i = 0; i < m->used; i++) {
        if (m->entries[i].live) a0_array_push(a, m->entries[i].key);
    }
    return a;
}

void a0_map_trace(A0Map *m) {
    for (int64_t i = 0; i < m->used; i++) {
        if (!m->entries[i].live) continue;
        if (m->key_kind == A0_KEY_STRING) a0_gc_mark_object((const void *)(uintptr_t)m->entries[i].key);
        a0_gc_mark_word(m->entries[i].value, m->value_kind);
    }
}

void a0_map_finalize(A0Map *m) {
    if (m->entries != NULL) a0_gc_buffer_free(m->entries, sizeof(Entry) * (size_t)m->entry_cap);
    if (m->index != NULL) a0_gc_buffer_free(m->index, sizeof(int64_t) * (size_t)m->index_cap);
}
