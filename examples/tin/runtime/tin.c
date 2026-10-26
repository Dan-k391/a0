/* Tin's runtime: the C functions Tin programs can call besides the C library's, for the
 * interpreter and the JIT (native executables link this library). */
#include "a0rt.h"

#include <stdint.h>

#include "cjson/cJSON.h"

/* A C string, which A0 copies into a string of its own (`extern fn tin_string(*u8) -> string`). */
const char *tin_string(const char *s) {
    return s;
}

/* The functions of abi.c. */
#define TIN_ABI(f) {#f, (void *)&f}
typedef struct { int32_t a, b; } TinPair;
typedef struct { float x, y; } TinVec2f;
typedef struct { double x, y; } TinVec2;
typedef struct { double x, y, z; } TinVec3;
typedef struct { int8_t tag; int32_t count; double value; } TinMixed;
typedef struct { uint8_t r, g, b; } TinRgb;
typedef struct { int64_t a, b, c, d; } TinQuad;
typedef struct { float x; int32_t n; } TinFloatInt;
TinPair tin_pair_swap(TinPair p);
TinVec2f tin_vec2f_scale(TinVec2f v, float k);
TinVec2 tin_vec2_add(TinVec2 a, TinVec2 b);
double tin_vec3_dot(TinVec3 a, TinVec3 b);
TinVec3 tin_vec3_cross(TinVec3 a, TinVec3 b);
TinMixed tin_mixed_bump(TinMixed m);
TinRgb tin_rgb_mix(TinRgb a, TinRgb b);
int64_t tin_quad_sum(TinQuad q);
TinQuad tin_quad_make(int64_t n);
TinFloatInt tin_float_int(TinFloatInt f, TinVec2 shift, int32_t a, int32_t b, int32_t c, int32_t d,
                          int32_t e, int32_t g, TinRgb last);

const A0Symbol tin_symbols[] = {
    {"tin_string", (void *)&tin_string},
    TIN_ABI(tin_pair_swap),
    TIN_ABI(tin_vec2f_scale),
    TIN_ABI(tin_vec2_add),
    TIN_ABI(tin_vec3_dot),
    TIN_ABI(tin_vec3_cross),
    TIN_ABI(tin_mixed_bump),
    TIN_ABI(tin_rgb_mix),
    TIN_ABI(tin_quad_sum),
    TIN_ABI(tin_quad_make),
    TIN_ABI(tin_float_int),
    /* cJSON (runtime/cjson, MIT), the C library Tin's JSON example uses. */
    TIN_ABI(cJSON_Version),
    TIN_ABI(cJSON_Parse),
    TIN_ABI(cJSON_Print),
    TIN_ABI(cJSON_PrintUnformatted),
    TIN_ABI(cJSON_Delete),
    TIN_ABI(cJSON_GetArraySize),
    TIN_ABI(cJSON_GetArrayItem),
    TIN_ABI(cJSON_GetObjectItem),
    TIN_ABI(cJSON_GetObjectItemCaseSensitive),
    TIN_ABI(cJSON_HasObjectItem),
    TIN_ABI(cJSON_GetErrorPtr),
    TIN_ABI(cJSON_GetStringValue),
    TIN_ABI(cJSON_GetNumberValue),
    TIN_ABI(cJSON_IsBool),
    TIN_ABI(cJSON_IsTrue),
    TIN_ABI(cJSON_IsNull),
    TIN_ABI(cJSON_IsNumber),
    TIN_ABI(cJSON_IsString),
    TIN_ABI(cJSON_IsArray),
    TIN_ABI(cJSON_IsObject),
    TIN_ABI(cJSON_CreateNull),
    TIN_ABI(cJSON_CreateBool),
    TIN_ABI(cJSON_CreateNumber),
    TIN_ABI(cJSON_CreateString),
    TIN_ABI(cJSON_CreateArray),
    TIN_ABI(cJSON_CreateObject),
    TIN_ABI(cJSON_AddItemToArray),
    TIN_ABI(cJSON_AddItemToObject),
    TIN_ABI(cJSON_AddNumberToObject),
    TIN_ABI(cJSON_AddStringToObject),
    TIN_ABI(cJSON_AddBoolToObject),
    TIN_ABI(cJSON_AddArrayToObject),
    TIN_ABI(cJSON_AddObjectToObject),
    TIN_ABI(cJSON_Duplicate),
    TIN_ABI(cJSON_Compare),
    TIN_ABI(cJSON_free),
    {0, 0},
};
