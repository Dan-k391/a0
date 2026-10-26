/* The C side of the standard library: files, standard input, the environment, time and string
 * operations. The library itself is HIR (crates/a0-hir/std), which declares these functions with
 * `extern "a0"`: they take and return A0 values. Booleans are int64_t 0 or 1. */
#define _CRT_SECURE_NO_WARNINGS /* fopen, strerror */
#define WIN32_LEAN_AND_MEAN
#include "internal.h"

#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifdef _WIN32
#include <windows.h>
#else
#include <dirent.h>
#include <sys/stat.h>
#include <unistd.h>
#endif

/* Why the last call that failed did (a file that could not be opened, a number that did not parse),
 * for the HIR side to put in an error value. The runtime runs one thread, and C functions have no
 * other way to return both a value and a reason. */
static A0_THREAD_LOCAL char last_error[512];
static int64_t last_failed;

static void fail(const char *message) {
    snprintf(last_error, sizeof last_error, "%s", message);
    last_failed = 1;
}

static void fail_path(const char *what, A0String *path) {
    char message[512];
    snprintf(message, sizeof message, "cannot %s %s: %s", what, (const char *)path->bytes,
             strerror(errno));
    fail(message);
}

A0String *a0_std_error(void) { return a0_string_new((const uint8_t *)last_error, (int64_t)strlen(last_error)); }

/* Whether the last call that can fail without saying so in its result (parsing) failed. */
int64_t a0_std_failed(void) { return last_failed; }

static A0String *check(A0String *s) {
    if (s == NULL) a0_panic_null();
    return s;
}

/* ---- a growable byte buffer, for building results before allocating them ---- */

typedef struct {
    char *bytes;
    size_t len, cap;
} Buffer;

static void buffer_add(Buffer *b, const char *bytes, size_t n) {
    if (b->len + n > b->cap) {
        size_t cap = b->cap < 64 ? 64 : b->cap;
        while (cap < b->len + n) cap *= 2;
        char *grown = (char *)realloc(b->bytes, cap);
        if (grown == NULL) a0_panic("out of memory");
        b->bytes = grown;
        b->cap = cap;
    }
    if (n > 0) memcpy(b->bytes + b->len, bytes, n);
    b->len += n;
}

/* The buffer as a string; frees the buffer. */
static A0String *buffer_string(Buffer *b) {
    A0String *s = a0_string_new((const uint8_t *)b->bytes, (int64_t)b->len);
    free(b->bytes);
    return s;
}

/* An array of strings made from C strings, kept rooted while its elements are allocated. */
static A0Array *string_array(char **items, int64_t count) {
    A0Array *a = a0_array_new(A0_WORD_REF, count);
    uint64_t *frame = a0_gc_frame_push(1);
    frame[0] = a0_any_from_ref(a);
    for (int64_t i = 0; i < count; i++) {
        A0String *s = a0_string_new((const uint8_t *)items[i], (int64_t)strlen(items[i]));
        a0_array_push(a, (uint64_t)(uintptr_t)s);
    }
    frame[0] = 0;
    a0_gc_frame_pop(1);
    return a;
}

/* ---- standard input ---- */

A0String *a0_std_read_line(void) {
    a0_rt_flush(); /* so that a prompt is seen before the program waits */
    if (a0_rt_has_debugger()) return NULL; /* standard input is the debugger's */
    Buffer b = {0};
    char chunk[4096];
    int any = 0;
    /* Waiting for input, other threads may collect. */
    a0_safe_begin();
    while (fgets(chunk, sizeof chunk, stdin) != NULL) {
        any = 1;
        size_t n = strlen(chunk);
        buffer_add(&b, chunk, n);
        if (n > 0 && chunk[n - 1] == '\n') break;
    }
    a0_safe_end();
    if (!any) {
        free(b.bytes);
        return NULL;
    }
    if (b.len > 0 && b.bytes[b.len - 1] == '\n') b.len--;
    if (b.len > 0 && b.bytes[b.len - 1] == '\r') b.len--;
    return buffer_string(&b);
}

A0String *a0_std_read_all(void) {
    a0_rt_flush();
    Buffer b = {0};
    if (a0_rt_has_debugger()) return buffer_string(&b);
    char chunk[4096];
    size_t n;
    a0_safe_begin();
    while ((n = fread(chunk, 1, sizeof chunk, stdin)) > 0) buffer_add(&b, chunk, n);
    a0_safe_end();
    return buffer_string(&b);
}

/* ---- files ---- */

#ifdef _WIN32
/* Paths are UTF-8; Windows wants UTF-16. Returns a malloc'd string, or NULL. */
static wchar_t *wide(const char *s) {
    int n = MultiByteToWideChar(CP_UTF8, 0, s, -1, NULL, 0);
    if (n <= 0) return NULL;
    wchar_t *w = (wchar_t *)malloc(sizeof(wchar_t) * (size_t)n);
    if (w != NULL) MultiByteToWideChar(CP_UTF8, 0, s, -1, w, n);
    return w;
}

static FILE *open_file(A0String *path, const wchar_t *mode) {
    wchar_t *w = wide((const char *)path->bytes);
    if (w == NULL) {
        errno = EINVAL;
        return NULL;
    }
    FILE *f = _wfopen(w, mode);
    free(w);
    return f;
}
#define MODE(m) L##m
#else
static FILE *open_file(A0String *path, const char *mode) { return fopen((const char *)path->bytes, mode); }
#define MODE(m) m
#endif

A0String *a0_std_read_file(A0String *path) {
    FILE *f = open_file(check(path), MODE("rb"));
    if (f == NULL) {
        fail_path("read", path);
        return NULL;
    }
    Buffer b = {0};
    char chunk[16384];
    size_t n;
    while ((n = fread(chunk, 1, sizeof chunk, f)) > 0) buffer_add(&b, chunk, n);
    int bad = ferror(f);
    fclose(f);
    if (bad) {
        free(b.bytes);
        fail_path("read", path);
        return NULL;
    }
    return buffer_string(&b);
}

int64_t a0_std_write_file(A0String *path, A0String *text, int64_t append) {
    FILE *f = open_file(check(path), append ? MODE("ab") : MODE("wb"));
    if (f == NULL) {
        fail_path("write", path);
        return 0;
    }
    check(text);
    size_t written = fwrite(text->bytes, 1, (size_t)text->len, f);
    int closed = fclose(f);
    if (written != (size_t)text->len || closed != 0) {
        fail_path("write", path);
        return 0;
    }
    return 1;
}

/* 0: nothing there, 1: a file, 2: a directory. */
int64_t a0_std_path_kind(A0String *path) {
#ifdef _WIN32
    wchar_t *w = wide((const char *)check(path)->bytes);
    if (w == NULL) return 0;
    DWORD attributes = GetFileAttributesW(w);
    free(w);
    if (attributes == INVALID_FILE_ATTRIBUTES) return 0;
    return (attributes & FILE_ATTRIBUTE_DIRECTORY) ? 2 : 1;
#else
    struct stat st;
    if (stat((const char *)check(path)->bytes, &st) != 0) return 0;
    return S_ISDIR(st.st_mode) ? 2 : 1;
#endif
}

int64_t a0_std_remove(A0String *path) {
#ifdef _WIN32
    wchar_t *w = wide((const char *)check(path)->bytes);
    int r = w == NULL ? -1 : _wremove(w);
    free(w);
#else
    int r = remove((const char *)check(path)->bytes);
#endif
    if (r != 0) {
        fail_path("remove", path);
        return 0;
    }
    return 1;
}

static int compare_names(const void *a, const void *b) { return strcmp(*(char *const *)a, *(char *const *)b); }

static void add_name(char ***names, int64_t *count, int64_t *cap, const char *name) {
    if (*count == *cap) {
        *cap = *cap == 0 ? 16 : *cap * 2;
        char **grown = (char **)realloc(*names, sizeof(char *) * (size_t)*cap);
        if (grown == NULL) a0_panic("out of memory");
        *names = grown;
    }
    size_t n = strlen(name);
    char *copy = (char *)malloc(n + 1);
    if (copy == NULL) a0_panic("out of memory");
    memcpy(copy, name, n + 1);
    (*names)[(*count)++] = copy;
}

/* The names in a directory, sorted (byte order), without "." and "..". */
A0Array *a0_std_list_dir(A0String *path) {
    char **names = NULL;
    int64_t count = 0, cap = 0;
#ifdef _WIN32
    size_t len = (size_t)check(path)->len;
    char *pattern = (char *)malloc(len + 3);
    if (pattern == NULL) a0_panic("out of memory");
    memcpy(pattern, path->bytes, len);
    memcpy(pattern + len, "\\*", 3);
    wchar_t *w = wide(pattern);
    free(pattern);
    WIN32_FIND_DATAW entry;
    HANDLE h = w == NULL ? INVALID_HANDLE_VALUE : FindFirstFileW(w, &entry);
    free(w);
    if (h == INVALID_HANDLE_VALUE) {
        errno = ENOENT;
        fail_path("list", path);
        return NULL;
    }
    do {
        char name[MAX_PATH * 4];
        if (WideCharToMultiByte(CP_UTF8, 0, entry.cFileName, -1, name, sizeof name, NULL, NULL) <= 0) {
            continue;
        }
        if (strcmp(name, ".") != 0 && strcmp(name, "..") != 0) add_name(&names, &count, &cap, name);
    } while (FindNextFileW(h, &entry));
    FindClose(h);
#else
    DIR *d = opendir((const char *)check(path)->bytes);
    if (d == NULL) {
        fail_path("list", path);
        return NULL;
    }
    struct dirent *entry;
    while ((entry = readdir(d)) != NULL) {
        if (strcmp(entry->d_name, ".") != 0 && strcmp(entry->d_name, "..") != 0) {
            add_name(&names, &count, &cap, entry->d_name);
        }
    }
    closedir(d);
#endif
    if (count > 1) qsort(names, (size_t)count, sizeof(char *), compare_names);
    A0Array *a = string_array(names, count);
    for (int64_t i = 0; i < count; i++) free(names[i]);
    free(names);
    return a;
}

/* ---- the environment ---- */

/* The program's arguments, without the program itself. */
A0Array *a0_std_args(void) {
    int32_t argc = a0_rt_argc();
    char **items = (char **)malloc(sizeof(char *) * (size_t)(argc > 1 ? argc : 1));
    if (items == NULL) a0_panic("out of memory");
    int64_t count = 0;
    for (int32_t i = 1; i < argc; i++) items[count++] = (char *)a0_rt_argv(i);
    A0Array *a = string_array(items, count);
    free(items);
    return a;
}

/* ---- time ---- */

/* Seconds since 1970-01-01 UTC. */
double a0_std_now(void) {
    struct timespec ts;
    if (timespec_get(&ts, TIME_UTC) == 0) return (double)time(NULL);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

/* Seconds since some fixed moment, never going backwards: for measuring how long things take. */
double a0_std_monotonic(void) {
#ifdef _WIN32
    LARGE_INTEGER frequency, now;
    QueryPerformanceFrequency(&frequency);
    QueryPerformanceCounter(&now);
    return (double)now.QuadPart / (double)frequency.QuadPart;
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
#endif
}

void a0_std_sleep(double seconds) {
    if (!(seconds > 0)) return;
    a0_rt_flush();
    a0_safe_begin();
#ifdef _WIN32
    Sleep((DWORD)(seconds * 1000.0));
#else
    struct timespec ts;
    ts.tv_sec = (time_t)seconds;
    ts.tv_nsec = (long)((seconds - (double)ts.tv_sec) * 1e9);
    while (nanosleep(&ts, &ts) != 0 && errno == EINTR) {
    }
#endif
    a0_safe_end();
}

/* "2026-10-25T09:30:00Z" for seconds since 1970 (UTC). */
A0String *a0_std_format_time(double seconds) {
    time_t t = (time_t)floor(seconds);
    struct tm tm;
#ifdef _WIN32
    if (gmtime_s(&tm, &t) != 0) return a0_string_new((const uint8_t *)"", 0);
#else
    if (gmtime_r(&t, &tm) == NULL) return a0_string_new((const uint8_t *)"", 0);
#endif
    char text[64];
    size_t n = strftime(text, sizeof text, "%Y-%m-%dT%H:%M:%SZ", &tm);
    return a0_string_new((const uint8_t *)text, (int64_t)n);
}

/* ---- random numbers (xoshiro256**, seeded with splitmix64) ---- */

static uint64_t rng[4];
static int rng_seeded;

static uint64_t splitmix(uint64_t *x) {
    uint64_t z = (*x += 0x9e3779b97f4a7c15ull);
    z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ull;
    z = (z ^ (z >> 27)) * 0x94d049bb133111ebull;
    return z ^ (z >> 31);
}

static void seed(int64_t value) {
    uint64_t x = (uint64_t)value;
    for (int i = 0; i < 4; i++) rng[i] = splitmix(&x);
    rng_seeded = 1;
}

void a0_std_seed(int64_t value) {
    A0_TABLE_LOCK();
    seed(value);
    A0_TABLE_UNLOCK();
}

static uint64_t rotl(uint64_t x, int k) { return (x << k) | (x >> (64 - k)); }

static uint64_t next_random(void) {
    /* Unseeded programs get the same numbers every run, so that they are reproducible. */
    A0_TABLE_LOCK();
    if (!rng_seeded) seed(0);
    uint64_t result = rotl(rng[1] * 5, 7) * 9;
    uint64_t t = rng[1] << 17;
    rng[2] ^= rng[0];
    rng[3] ^= rng[1];
    rng[1] ^= rng[2];
    rng[0] ^= rng[3];
    rng[2] ^= t;
    rng[3] = rotl(rng[3], 45);
    A0_TABLE_UNLOCK();
    return result;
}

/* In [0, 1). */
double a0_std_random(void) { return (double)(next_random() >> 11) / 9007199254740992.0; }

/* In [low, high]; `low` if the range is empty. */
int64_t a0_std_random_int(int64_t low, int64_t high) {
    if (high <= low) return low;
    uint64_t span = (uint64_t)high - (uint64_t)low + 1;
    if (span == 0) return (int64_t)next_random(); /* the whole range of int */
    uint64_t limit = UINT64_MAX - UINT64_MAX % span;
    uint64_t r;
    do r = next_random();
    while (r >= limit);
    return (int64_t)((uint64_t)low + r % span);
}

/* ---- strings ---- */

/* Where `part` first occurs in `s` at or after byte `from`, or -1. */
int64_t a0_std_find(A0String *s, A0String *part, int64_t from) {
    check(s);
    check(part);
    if (from < 0) from = 0;
    if (part->len == 0) return from <= s->len ? from : -1;
    for (int64_t i = from; i + part->len <= s->len; i++) {
        if (s->bytes[i] == part->bytes[0] && memcmp(s->bytes + i, part->bytes, (size_t)part->len) == 0) {
            return i;
        }
    }
    return -1;
}

/* Where `part` last occurs in `s`, or -1. */
int64_t a0_std_find_last(A0String *s, A0String *part) {
    check(s);
    check(part);
    for (int64_t i = s->len - part->len; i >= 0; i--) {
        if (memcmp(s->bytes + i, part->bytes, (size_t)part->len) == 0) return i;
    }
    return -1;
}

int64_t a0_std_starts_with(A0String *s, A0String *prefix) {
    check(s);
    check(prefix);
    return prefix->len <= s->len && memcmp(s->bytes, prefix->bytes, (size_t)prefix->len) == 0;
}

int64_t a0_std_ends_with(A0String *s, A0String *suffix) {
    check(s);
    check(suffix);
    return suffix->len <= s->len &&
           memcmp(s->bytes + s->len - suffix->len, suffix->bytes, (size_t)suffix->len) == 0;
}

A0String *a0_std_replace(A0String *s, A0String *from, A0String *to) {
    check(s);
    check(from);
    check(to);
    if (from->len == 0) return s;
    Buffer b = {0};
    int64_t i = 0;
    for (int64_t at; (at = a0_std_find(s, from, i)) >= 0; i = at + from->len) {
        buffer_add(&b, (const char *)s->bytes + i, (size_t)(at - i));
        buffer_add(&b, (const char *)to->bytes, (size_t)to->len);
    }
    buffer_add(&b, (const char *)s->bytes + i, (size_t)(s->len - i));
    return buffer_string(&b);
}

static int is_space(uint8_t c) { return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\f' || c == '\v'; }

/* The pieces of `s` between occurrences of `separator`; with an empty separator, the words of `s`
 * (the pieces between runs of whitespace, none of them empty). */
A0Array *a0_std_split(A0String *s, A0String *separator) {
    check(s);
    check(separator);
    A0Array *a = a0_array_new(A0_WORD_REF, 0);
    uint64_t *frame = a0_gc_frame_push(1);
    frame[0] = a0_any_from_ref(a);
    if (separator->len == 0) {
        int64_t i = 0;
        while (i < s->len) {
            while (i < s->len && is_space(s->bytes[i])) i++;
            int64_t start = i;
            while (i < s->len && !is_space(s->bytes[i])) i++;
            if (i > start) a0_array_push(a, (uint64_t)(uintptr_t)a0_string_new(s->bytes + start, i - start));
        }
    } else {
        int64_t i = 0;
        for (;;) {
            int64_t at = a0_std_find(s, separator, i);
            int64_t end = at < 0 ? s->len : at;
            a0_array_push(a, (uint64_t)(uintptr_t)a0_string_new(s->bytes + i, end - i));
            if (at < 0) break;
            i = at + separator->len;
        }
    }
    frame[0] = 0;
    a0_gc_frame_pop(1);
    return a;
}

/* The lines of `s`, without their "\n" or "\r\n"; a last line without one counts too. */
A0Array *a0_std_lines(A0String *s) {
    check(s);
    A0Array *a = a0_array_new(A0_WORD_REF, 0);
    uint64_t *frame = a0_gc_frame_push(1);
    frame[0] = a0_any_from_ref(a);
    int64_t i = 0;
    while (i < s->len) {
        int64_t end = i;
        while (end < s->len && s->bytes[end] != '\n') end++;
        int64_t next = end < s->len ? end + 1 : end;
        if (end > i && s->bytes[end - 1] == '\r') end--;
        a0_array_push(a, (uint64_t)(uintptr_t)a0_string_new(s->bytes + i, end - i));
        i = next;
    }
    frame[0] = 0;
    a0_gc_frame_pop(1);
    return a;
}

A0String *a0_std_join(A0Array *parts, A0String *separator) {
    if (parts == NULL) a0_panic_null();
    check(separator);
    Buffer b = {0};
    for (int64_t i = 0; i < parts->len; i++) {
        A0String *part = check((A0String *)(uintptr_t)parts->data[i]);
        if (i > 0) buffer_add(&b, (const char *)separator->bytes, (size_t)separator->len);
        buffer_add(&b, (const char *)part->bytes, (size_t)part->len);
    }
    return buffer_string(&b);
}

/* Without leading (bit 1) and trailing (bit 2) whitespace. */
A0String *a0_std_trim(A0String *s, int64_t sides) {
    check(s);
    int64_t start = 0, end = s->len;
    if (sides & 1) {
        while (start < end && is_space(s->bytes[start])) start++;
    }
    if (sides & 2) {
        while (end > start && is_space(s->bytes[end - 1])) end--;
    }
    if (start == 0 && end == s->len) return s;
    return a0_string_new(s->bytes + start, end - start);
}

/* ASCII letters in upper or lower case; other bytes are kept. */
A0String *a0_std_change_case(A0String *s, int64_t upper) {
    check(s);
    A0String *r = a0_string_new(s->bytes, s->len);
    for (int64_t i = 0; i < r->len; i++) {
        uint8_t c = r->bytes[i];
        if (upper && c >= 'a' && c <= 'z') r->bytes[i] = (uint8_t)(c - 32);
        if (!upper && c >= 'A' && c <= 'Z') r->bytes[i] = (uint8_t)(c + 32);
    }
    return r;
}

A0String *a0_std_repeat(A0String *s, int64_t count) {
    check(s);
    Buffer b = {0};
    for (int64_t i = 0; i < count; i++) buffer_add(&b, (const char *)s->bytes, (size_t)s->len);
    return buffer_string(&b);
}

/* A decimal integer, with an optional sign and surrounding whitespace; sets the failure flag. */
int64_t a0_std_parse_int(A0String *s) {
    check(s);
    last_failed = 0;
    int64_t i = 0, end = s->len;
    while (i < end && is_space(s->bytes[i])) i++;
    while (end > i && is_space(s->bytes[end - 1])) end--;
    int negative = 0;
    if (i < end && (s->bytes[i] == '+' || s->bytes[i] == '-')) negative = s->bytes[i++] == '-';
    if (i == end) {
        fail("not an integer");
        return 0;
    }
    uint64_t value = 0;
    uint64_t limit = negative ? (uint64_t)INT64_MAX + 1 : (uint64_t)INT64_MAX;
    for (; i < end; i++) {
        uint8_t c = s->bytes[i];
        if (c < '0' || c > '9') {
            fail("not an integer");
            return 0;
        }
        if (value > (limit - (uint64_t)(c - '0')) / 10) {
            fail("integer out of range");
            return 0;
        }
        value = value * 10 + (uint64_t)(c - '0');
    }
    return negative ? (int64_t)(0 - value) : (int64_t)value;
}

/* A decimal floating-point number (as C's strtod reads it, also "inf" and "nan"), with surrounding
 * whitespace; sets the failure flag. */
double a0_std_parse_float(A0String *s) {
    check(s);
    last_failed = 0;
    const char *start = (const char *)s->bytes;
    char *end;
    errno = 0;
    double value = strtod(start, &end);
    const char *last = start + s->len;
    while (end < last && is_space((uint8_t)*end)) end++;
    if (end == start || end != last) {
        fail("not a number");
        return 0;
    }
    return value;
}

/* `x` with `decimals` digits after the point ("3.14"). */
A0String *a0_std_fixed(double x, int64_t decimals) {
    if (decimals < 0) decimals = 0;
    if (decimals > 30) decimals = 30;
    char text[512];
    int n = snprintf(text, sizeof text, "%.*f", (int)decimals, x);
    if (n < 0) n = 0;
    if (n >= (int)sizeof text) n = (int)sizeof text - 1;
    return a0_string_new((const uint8_t *)text, n);
}

A0String *a0_std_from_byte(int64_t byte) {
    uint8_t b = (uint8_t)byte;
    return a0_string_new(&b, 1);
}

/* The bytes of `s`, as ints. */
A0Array *a0_std_bytes(A0String *s) {
    check(s);
    A0Array *a = a0_array_new(A0_WORD_RAW, s->len);
    for (int64_t i = 0; i < s->len; i++) a0_array_push(a, s->bytes[i]);
    return a;
}

/* The string of bytes `bytes` (each taken modulo 256). */
A0String *a0_std_from_bytes(A0Array *bytes) {
    if (bytes == NULL) a0_panic_null();
    A0String *s = (A0String *)a0_gc_alloc(&a0_builtin_string, sizeof(A0String) + (size_t)bytes->len + 1);
    s->len = bytes->len;
    s->bytes[s->len] = 0;
    for (int64_t i = 0; i < bytes->len; i++) s->bytes[i] = (uint8_t)bytes->data[i];
    return s;
}

/* The UTF-8 encoding of the code point `c` (U+FFFD for one that is not valid). */
A0String *a0_std_from_code_point(int64_t c) {
    uint8_t b[4];
    int64_t n;
    if (c < 0 || c > 0x10ffff || (c >= 0xd800 && c <= 0xdfff)) c = 0xfffd;
    if (c < 0x80) {
        b[0] = (uint8_t)c;
        n = 1;
    } else if (c < 0x800) {
        b[0] = (uint8_t)(0xc0 | (c >> 6));
        b[1] = (uint8_t)(0x80 | (c & 0x3f));
        n = 2;
    } else if (c < 0x10000) {
        b[0] = (uint8_t)(0xe0 | (c >> 12));
        b[1] = (uint8_t)(0x80 | ((c >> 6) & 0x3f));
        b[2] = (uint8_t)(0x80 | (c & 0x3f));
        n = 3;
    } else {
        b[0] = (uint8_t)(0xf0 | (c >> 18));
        b[1] = (uint8_t)(0x80 | ((c >> 12) & 0x3f));
        b[2] = (uint8_t)(0x80 | ((c >> 6) & 0x3f));
        b[3] = (uint8_t)(0x80 | (c & 0x3f));
        n = 4;
    }
    return a0_string_new(b, n);
}
