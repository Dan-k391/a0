/* Runtime errors: what failed, where, and the stack of source-level functions that led there.
 *
 * Both backends give the runtime a table of locations (a function as traces name it, a place in
 * the source, and for inlined code the location of the call it was inlined at). A failing operation
 * passes its location and, in native code, its frame pointer. Native code keeps frame pointers, and
 * a second table maps the return address of every call in generated code to the call's location,
 * so the frames are found by walking the frame pointer chain through generated code only, at no
 * cost to programs that do not fail. The interpreter has no native frames; it reports its own
 * through a callback. */
#include "internal.h"

#include <stdio.h>
#include <string.h>

static const A0LocationTable *g_locations;
static const A0CodeTable *g_code;
static A0FramesFn g_frames;

void a0_rt_set_debug(const A0LocationTable *locations, const A0CodeTable *code) {
    g_locations = locations;
    g_code = code;
}

void a0_rt_set_frames(A0FramesFn frames) { g_frames = frames; }

static const A0Location *location(uint64_t id) {
    if (g_locations == NULL || id == 0 || id > g_locations->count) return NULL;
    return &((const A0Location *)(g_locations + 1))[id - 1];
}

static const char *string_at(uint32_t offset) {
    return (const char *)g_locations + g_locations->strings + offset;
}

/* Whether `ra` returns into generated code; if it does, `*site` is the location of the call (0 if
 * the call has none, in a function traces leave out). */
static int call_site(uintptr_t ra, uint32_t *site) {
    if (g_code == NULL) return 0;
    for (uint64_t i = 0; i < g_code->count; i++) {
        const A0CodeFunction *f = &g_code->functions[i];
        uintptr_t start = (uintptr_t)f->start;
        if (ra <= start || ra > start + f->size) continue;
        /* The call instruction ends where the return address points. */
        uint32_t offset = (uint32_t)(ra - 1 - start);
        *site = 0;
        for (uint32_t r = f->first; r < f->first + f->count; r++) {
            const A0CodeRange *range = &g_code->ranges[r];
            if (offset >= range->start && offset < range->end) {
                *site = range->location;
                break;
            }
        }
        return 1;
    }
    return 0;
}

/* The locations of the active frames, innermost first: the failing operation's, then the call in
 * each caller. */
size_t a0_rt_frames(uint64_t loc, void *frame, uint32_t *out, size_t max) {
    size_t n = 0;
    out[n++] = (uint32_t)loc;
    if (frame != NULL) {
        /* Every generated frame starts with the caller's frame pointer and the return address. */
        void **fp = (void **)frame;
        uint32_t site;
        while (n < max && call_site((uintptr_t)fp[1], &site)) {
            out[n++] = site;
            void **next = (void **)fp[0];
            /* The stack grows down: a caller's frame is above its callee's. */
            if (next <= fp) break;
            fp = next;
        }
    } else if (g_frames != NULL) {
        n += g_frames(out + n, max - n);
    }
    return n;
}

typedef struct {
    char *buf;
    size_t cap;
    size_t len;
} Text;

static void add(Text *t, const char *s) {
    size_t n = strlen(s);
    for (size_t i = 0; i < n; i++) {
        if (t->len + 1 < t->cap) t->buf[t->len] = s[i];
        t->len++;
    }
    if (t->cap > 0) t->buf[t->len < t->cap ? t->len : t->cap - 1] = 0;
}

/* One frame: the function and place of `id`, then of each call it was inlined at. */
static void add_frame(Text *t, uint32_t id) {
    char line[512];
    for (const A0Location *l = location(id); l != NULL; l = location(l->parent)) {
        const char *file = string_at(l->file);
        if (*file) {
            snprintf(line, sizeof line, "    at %s (%s:%u:%u)\n", string_at(l->func), file, l->line,
                     l->col);
        } else {
            snprintf(line, sizeof line, "    at %s\n", string_at(l->func));
        }
        add(t, line);
    }
}

/* At most this many frames are shown; deep recursion repeats one call, shown once with a count. */
#define SHOWN_FRAMES 64

size_t a0_rt_trace_text(const uint32_t *ids, size_t count, char *buf, size_t cap) {
    Text t = {buf, cap, 0};
    if (cap > 0) buf[0] = 0;
    size_t shown = 0;
    size_t i = 0;
    while (i < count) {
        size_t run = 1;
        while (i + run < count && ids[i + run] == ids[i]) run++;
        if (location(ids[i]) == NULL) {
            /* A frame traces leave out. */
            i += run;
            continue;
        }
        if (shown == SHOWN_FRAMES) {
            char line[64];
            snprintf(line, sizeof line, "    ... %zu more frames\n", count - i);
            add(&t, line);
            break;
        }
        add_frame(&t, ids[i]);
        shown++;
        if (run > 1) {
            char line[80];
            snprintf(line, sizeof line, "    ... the frame above repeats %zu more times\n", run - 1);
            add(&t, line);
        }
        i += run;
    }
    return t.len;
}

/* Only the first thread to fail uses these (a0_fail_lock). */
static uint32_t g_ids[4096];
static char g_text[65536];

static A0_NORETURN void fail(const uint8_t *message, size_t len, const uint32_t *ids, size_t count) {
    a0_rt_tell_failure(message, len);
    a0_rt_write(2, (const uint8_t *)"panic: ", 7);
    a0_rt_write(2, message, len);
    a0_rt_write(2, (const uint8_t *)"\n", 1);
    if (count > 0) {
        size_t n = a0_rt_trace_text(ids, count, g_text, sizeof g_text);
        a0_rt_write(2, (const uint8_t *)g_text, n < sizeof g_text ? n : sizeof g_text - 1);
    }
    /* A spawned thread's stack ends where it started; the stack that spawned it follows. */
    const uint32_t *spawned;
    int64_t id;
    size_t spawned_count = a0_thread_spawn_frames(&spawned, &id);
    if (spawned_count > 0) {
        char line[64];
        int n = snprintf(line, sizeof line, "    in thread %lld, spawned\n", (long long)id);
        a0_rt_write(2, (const uint8_t *)line, (size_t)(n > 0 ? n : 0));
        size_t t = a0_rt_trace_text(spawned, spawned_count, g_text, sizeof g_text);
        a0_rt_write(2, (const uint8_t *)g_text, t < sizeof g_text ? t : sizeof g_text - 1);
    }
    a0_rt_exit(A0_PANIC_EXIT_CODE);
}

void a0_rt_fail_bytes(const uint8_t *message, int64_t len, uint64_t loc, void *frame) {
    a0_fail_lock();
    size_t count = a0_rt_frames(loc, frame, g_ids, sizeof g_ids / sizeof g_ids[0]);
    fail(message, (size_t)len, g_ids, count);
}

void a0_rt_fail(const char *message, uint64_t loc, void *frame) {
    a0_rt_fail_bytes((const uint8_t *)message, (int64_t)strlen(message), loc, frame);
}

void a0_rt_fail_frames(const char *message, const uint32_t *ids, size_t count) {
    a0_fail_lock();
    fail((const uint8_t *)message, strlen(message), ids, count);
}

/* Where the exception in flight was thrown: kept by a0_rt_throw for a0_rt_uncaught, since the
 * frames are gone by the time the exception reaches `main`. */
static A0_THREAD_LOCAL uint32_t g_thrown[1024];
static A0_THREAD_LOCAL size_t g_thrown_count;

void a0_rt_throw(uint64_t loc, void *frame) {
    g_thrown_count = a0_rt_frames(loc, frame, g_thrown, sizeof g_thrown / sizeof g_thrown[0]);
}

void a0_rt_uncaught(uint64_t value) {
    a0_fail_lock();
    char what[1024];
    a0_any_describe(value, what, sizeof what);
    char message[1100];
    int n = snprintf(message, sizeof message, "uncaught exception: %s", what);
    if (n < 0) n = 0;
    if ((size_t)n >= sizeof message) n = (int)sizeof message - 1;
    fail((const uint8_t *)message, (size_t)n, g_thrown, g_thrown_count);
}

A0_COLD void a0_rt_out_of_bounds(int64_t len, int64_t index, uint64_t loc, void *frame) {
    char message[128];
    snprintf(message, sizeof message, "index out of bounds: the length is %lld but the index is %lld",
             (long long)len, (long long)index);
    a0_rt_fail(message, loc, frame);
}
