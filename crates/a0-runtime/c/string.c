/* Strings: immutable byte strings in the GC heap. */
#include "internal.h"

#include <stdio.h>
#include <string.h>

static A0String *check(A0String *s) {
    if (s == NULL) a0_panic_null();
    return s;
}

A0String *a0_string_new(const uint8_t *bytes, int64_t len) {
    A0String *s = (A0String *)a0_gc_alloc(&a0_builtin_string, sizeof(A0String) + (size_t)len + 1);
    s->len = len;
    if (len > 0) memcpy(s->bytes, bytes, (size_t)len);
    s->bytes[len] = 0;
    return s;
}

/* The caller keeps `a` and `b` rooted. */
A0String *a0_string_concat(A0String *a, A0String *b) {
    check(a);
    check(b);
    A0String *s = (A0String *)a0_gc_alloc(&a0_builtin_string, sizeof(A0String) + (size_t)(a->len + b->len) + 1);
    s->len = a->len + b->len;
    memcpy(s->bytes, a->bytes, (size_t)a->len);
    memcpy(s->bytes + a->len, b->bytes, (size_t)b->len);
    s->bytes[s->len] = 0;
    return s;
}

int64_t a0_string_len(A0String *s) { return check(s)->len; }

uint64_t a0_string_hash(A0String *s) {
    if (s->hash == 0) {
        uint64_t h = 14695981039346656037ull;
        for (int64_t i = 0; i < s->len; i++) {
            h ^= s->bytes[i];
            h *= 1099511628211ull;
        }
        s->hash = h == 0 ? 1 : h;
    }
    return s->hash;
}

int64_t a0_string_eq(A0String *a, A0String *b) {
    if (a == b) return 1;
    if (a == NULL || b == NULL || a->len != b->len) return 0;
    /* Hashes are cached in the strings, so comparing the same long strings again is quick; strings
     * are not interned (D16). */
    if (a->len > 16) {
        uint64_t ha = a->hash != 0 ? a->hash : a0_string_hash(a);
        uint64_t hb = b->hash != 0 ? b->hash : a0_string_hash(b);
        if (ha != hb) return 0;
    }
    return memcmp(a->bytes, b->bytes, (size_t)a->len) == 0;
}

int64_t a0_string_compare(A0String *a, A0String *b) {
    check(a);
    check(b);
    int64_t n = a->len < b->len ? a->len : b->len;
    int c = memcmp(a->bytes, b->bytes, (size_t)n);
    if (c != 0) return c < 0 ? -1 : 1;
    return a->len < b->len ? -1 : a->len > b->len ? 1 : 0;
}

A0String *a0_string_substr(A0String *s, int64_t start, int64_t count) {
    check(s);
    if (start < 0 || count < 0 || start > s->len || count > s->len - start) a0_panic("substring out of range");
    /* `s` is rooted by the caller; copy out of it after allocating. */
    A0String *r = (A0String *)a0_gc_alloc(&a0_builtin_string, sizeof(A0String) + (size_t)count + 1);
    r->len = count;
    memcpy(r->bytes, s->bytes + start, (size_t)count);
    r->bytes[count] = 0;
    return r;
}

int64_t a0_string_byte_at(A0String *s, int64_t index) {
    check(s);
    if (index < 0 || index >= s->len) a0_panic("index out of bounds");
    return s->bytes[index];
}

A0String *a0_string_from_int(int64_t value) {
    char buf[32];
    int n = snprintf(buf, sizeof buf, "%lld", (long long)value);
    return a0_string_new((const uint8_t *)buf, n);
}

A0String *a0_string_from_float(double value) {
    char buf[32];
    int32_t n = a0_format_f64(value, buf);
    return a0_string_new((const uint8_t *)buf, n);
}

A0String *a0_string_from_bool(int64_t value) {
    return value ? a0_string_new((const uint8_t *)"true", 4) : a0_string_new((const uint8_t *)"false", 5);
}

void a0_string_print(A0String *s) {
    check(s);
    if (s->len > 0) fwrite(s->bytes, 1, (size_t)s->len, stdout);
}

void a0_string_eprint(A0String *s) {
    check(s);
    fflush(stdout);
    if (s->len > 0) fwrite(s->bytes, 1, (size_t)s->len, stderr);
}

void a0_string_panic(A0String *message) {
    check(message);
    a0_panic_bytes(message->bytes, message->len);
}
