/* Functions that take and return C structs by value, of the shapes the C ABIs treat differently:
 * integers, floats and doubles in registers, mixed fields, odd sizes, and structs too large for
 * registers. Tin's tests call them in every mode, natively included, to check that A0 passes
 * structs as C does on each platform. */
#include <stdint.h>

typedef struct { int32_t a, b; } TinPair;
typedef struct { float x, y; } TinVec2f;
typedef struct { double x, y; } TinVec2;
typedef struct { double x, y, z; } TinVec3;
typedef struct { int8_t tag; int32_t count; double value; } TinMixed;
typedef struct { uint8_t r, g, b; } TinRgb;
typedef struct { int64_t a, b, c, d; } TinQuad;
typedef struct { float x; int32_t n; } TinFloatInt;

TinPair tin_pair_swap(TinPair p) {
    TinPair r = {p.b, p.a};
    return r;
}

TinVec2f tin_vec2f_scale(TinVec2f v, float k) {
    TinVec2f r = {v.x * k, v.y * k};
    return r;
}

TinVec2 tin_vec2_add(TinVec2 a, TinVec2 b) {
    TinVec2 r = {a.x + b.x, a.y + b.y};
    return r;
}

double tin_vec3_dot(TinVec3 a, TinVec3 b) {
    return a.x * b.x + a.y * b.y + a.z * b.z;
}

TinVec3 tin_vec3_cross(TinVec3 a, TinVec3 b) {
    TinVec3 r = {a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x};
    return r;
}

TinMixed tin_mixed_bump(TinMixed m) {
    TinMixed r = {(int8_t)(m.tag + 1), m.count * 2, m.value / 2};
    return r;
}

TinRgb tin_rgb_mix(TinRgb a, TinRgb b) {
    TinRgb r = {(uint8_t)((a.r + b.r) / 2), (uint8_t)((a.g + b.g) / 2), (uint8_t)((a.b + b.b) / 2)};
    return r;
}

int64_t tin_quad_sum(TinQuad q) {
    return q.a + q.b * 10 + q.c * 100 + q.d * 1000;
}

TinQuad tin_quad_make(int64_t n) {
    TinQuad q = {n, n + 1, n + 2, n + 3};
    return q;
}

TinFloatInt tin_float_int(TinFloatInt f, TinVec2 shift, int32_t a, int32_t b, int32_t c, int32_t d,
                          int32_t e, int32_t g, TinRgb last) {
    /* Past the argument registers: a struct among many arguments. */
    TinFloatInt r = {f.x + (float)shift.x + (float)shift.y, f.n + a + b + c + d + e + g + last.r};
    return r;
}
