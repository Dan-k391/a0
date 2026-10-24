/* Dynamic values (`any`): boxing, type tests and equality. */
#include "internal.h"

#include <math.h>
#include <stdio.h>
#include <string.h>

static double as_double(uint64_t w) {
    double d;
    memcpy(&d, &w, sizeof d);
    return d;
}

uint64_t a0_any_from_float(double value) {
    if (isnan(value)) return 0x7ff8000000000000ull; /* canonical: never collides with a tag */
    uint64_t w;
    memcpy(&w, &value, sizeof w);
    return w;
}

#define INLINE_INT_LIMIT ((int64_t)1 << 47)

static int is_inline_int(uint64_t w) { return (w >> 48) == (A0_ANY_INT_TAG >> 48); }

uint64_t a0_any_from_int(int64_t value) {
    if (value >= -INLINE_INT_LIMIT && value < INLINE_INT_LIMIT) {
        return A0_ANY_INT_TAG | ((uint64_t)value & 0x0000ffffffffffffull);
    }
    A0BoxedInt *b = (A0BoxedInt *)a0_gc_alloc(&a0_builtin_int, sizeof(A0BoxedInt));
    b->value = value;
    return A0_ANY_REF_TAG | (uint64_t)(uintptr_t)b;
}

uint64_t a0_any_from_ref(void *object) {
    return object == NULL ? A0_ANY_UNIT : (A0_ANY_REF_TAG | (uint64_t)(uintptr_t)object);
}

static const A0Type *ref_type(uint64_t w) {
    if (!A0_ANY_IS_REF(w) || A0_ANY_PTR(w) == NULL) return NULL;
    return a0_gc_type_of(A0_ANY_PTR(w));
}

int64_t a0_any_is_int(uint64_t value) {
    return is_inline_int(value) || ref_type(value) == &a0_builtin_int;
}

int64_t a0_any_is_string(uint64_t value) { return ref_type(value) == &a0_builtin_string; }

int64_t a0_any_is_type(uint64_t value, const A0Type *type) {
    const A0Type *t = ref_type(value);
    return t != NULL && t->family == type;
}

static const char *type_name(uint64_t w) {
    if (A0_ANY_IS_FLOAT(w)) return "float";
    if (w == A0_ANY_UNIT) return "unit";
    if (w == A0_ANY_TRUE || w == A0_ANY_FALSE) return "bool";
    if (is_inline_int(w)) return "int";
    const A0Type *t = ref_type(w);
    if (t == NULL) return "unknown";
    return t->family->name;
}

void a0_any_cast_failed(uint64_t value, const uint8_t *expected, int64_t len) {
    char message[256];
    snprintf(message, sizeof message, "expected %.*s, found %s", (int)len, (const char *)expected,
             type_name(value));
    a0_panic(message);
}

int64_t a0_any_to_int(uint64_t value) {
    if (!a0_any_is_int(value)) a0_any_cast_failed(value, (const uint8_t *)"int", 3);
    /* Shift the 48-bit payload to the top and back to sign-extend it. */
    if (is_inline_int(value)) return (int64_t)(value << 16) >> 16;
    return ((A0BoxedInt *)A0_ANY_PTR(value))->value;
}

int64_t a0_any_eq(uint64_t a, uint64_t b) {
    if (A0_ANY_IS_FLOAT(a) || A0_ANY_IS_FLOAT(b)) {
        return A0_ANY_IS_FLOAT(a) && A0_ANY_IS_FLOAT(b) && as_double(a) == as_double(b);
    }
    if (a == b) return 1;
    const A0Type *ta = ref_type(a);
    const A0Type *tb = ref_type(b);
    if (ta == NULL || ta != tb) return 0;
    if (ta == &a0_builtin_string) {
        return a0_string_eq((A0String *)A0_ANY_PTR(a), (A0String *)A0_ANY_PTR(b));
    }
    if (ta == &a0_builtin_int) {
        return ((A0BoxedInt *)A0_ANY_PTR(a))->value == ((A0BoxedInt *)A0_ANY_PTR(b))->value;
    }
    return 0;
}
