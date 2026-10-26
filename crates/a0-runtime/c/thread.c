/* Threads: operating-system threads sharing one heap (D75, D76).
 *
 * Until a program starts its second thread nothing here costs anything: `a0_threads_active` is 0
 * and every lock is skipped. After that, allocation and collection take the heap lock, and a
 * collection stops the world: it raises `a0_gc_stop_requested` and waits until every other thread
 * has stopped, at an allocation (waiting for the heap), at a poll generated code makes at loop
 * heads and function entries, or in a region where it waits for something (a channel, another
 * thread, time, input). Stopped threads have every reference they hold on their shadow stacks, which
 * the collector scans like the main thread's.
 *
 * A thread runs a closure (`fn() -> any`), whose result `a0_thread_join` returns. Channels pass
 * `any` values between threads, in order, through a buffer of fixed capacity. */
#include "internal.h"

#include <stdlib.h>
#include <string.h>

/* ---- Atomics ---------------------------------------------------------------------------------- */

#if defined(_MSC_VER)
#include <intrin.h>
int64_t a0_atomic_load(volatile int64_t *p) { return _InterlockedCompareExchange64(p, 0, 0); }
void a0_atomic_store(volatile int64_t *p, int64_t v) { _InterlockedExchange64(p, v); }
int64_t a0_atomic_add(volatile int64_t *p, int64_t delta) {
    return _InterlockedExchangeAdd64(p, delta) + delta;
}
#else
int64_t a0_atomic_load(volatile int64_t *p) { return __atomic_load_n(p, __ATOMIC_SEQ_CST); }
void a0_atomic_store(volatile int64_t *p, int64_t v) { __atomic_store_n(p, v, __ATOMIC_SEQ_CST); }
int64_t a0_atomic_add(volatile int64_t *p, int64_t delta) {
    return __atomic_add_fetch(p, delta, __ATOMIC_SEQ_CST);
}
#endif

/* ---- Locks and threads of the platform -------------------------------------------------------- */

#if defined(A0_NO_THREADS)
typedef int Mutex;
typedef int Cond;
#define MUTEX_INIT 0
static void mutex_init(Mutex *m) { *m = 0; }
static void mutex_lock(Mutex *m) { (void)m; }
static int mutex_trylock(Mutex *m) { (void)m; return 1; }
static void mutex_unlock(Mutex *m) { (void)m; }
static void cond_init(Cond *c) { (void)c; }
static void cond_wait(Cond *c, Mutex *m) { (void)c; (void)m; }
static void cond_broadcast(Cond *c) { (void)c; }
static void yield_now(void) {}
#elif defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
typedef SRWLOCK Mutex;
typedef CONDITION_VARIABLE Cond;
#define MUTEX_INIT SRWLOCK_INIT
static void mutex_init(Mutex *m) { InitializeSRWLock(m); }
static void mutex_lock(Mutex *m) { AcquireSRWLockExclusive(m); }
static int mutex_trylock(Mutex *m) { return TryAcquireSRWLockExclusive(m) != 0; }
static void mutex_unlock(Mutex *m) { ReleaseSRWLockExclusive(m); }
static void cond_init(Cond *c) { InitializeConditionVariable(c); }
static void cond_wait(Cond *c, Mutex *m) { SleepConditionVariableSRW(c, m, INFINITE, 0); }
static void cond_broadcast(Cond *c) { WakeAllConditionVariable(c); }
static void yield_now(void) { SwitchToThread(); }
#else
#include <pthread.h>
#include <sched.h>
#include <unistd.h>
typedef pthread_mutex_t Mutex;
typedef pthread_cond_t Cond;
#define MUTEX_INIT PTHREAD_MUTEX_INITIALIZER
static void mutex_init(Mutex *m) { pthread_mutex_init(m, NULL); }
static void mutex_lock(Mutex *m) { pthread_mutex_lock(m); }
static int mutex_trylock(Mutex *m) { return pthread_mutex_trylock(m) == 0; }
static void mutex_unlock(Mutex *m) { pthread_mutex_unlock(m); }
static void cond_init(Cond *c) { pthread_cond_init(c, NULL); }
static void cond_wait(Cond *c, Mutex *m) { pthread_cond_wait(c, m); }
static void cond_broadcast(Cond *c) { pthread_cond_broadcast(c); }
static void yield_now(void) { sched_yield(); }
#endif

/* ---- The heap lock and stopping the world ----------------------------------------------------- */

int a0_threads_active;
volatile int64_t a0_gc_stop_requested;

static Mutex heap = MUTEX_INIT;
static Mutex tables = MUTEX_INIT;
static Mutex failing = MUTEX_INIT;
static A0_THREAD_LOCAL int heap_depth;
/* Threads that are stopped or waiting, and threads the collector knows of (gc.c attaches them). */
static volatile int64_t stopped;
static volatile int64_t thread_count;

void a0_heap_enter(void) {
    if (!a0_threads_active) return;
    if (heap_depth++ > 0) return;
    if (!mutex_trylock(&heap)) {
        /* Waiting for the heap, the thread counts as stopped: a collection may be what holds it. */
        a0_atomic_add(&stopped, 1);
        mutex_lock(&heap);
        a0_atomic_add(&stopped, -1);
    }
}

/* For a thread the collector does not know yet: waiting, it is not one the collector waits for. */
void a0_heap_enter_unattached(void) {
    if (!a0_threads_active) return;
    if (heap_depth++ > 0) return;
    mutex_lock(&heap);
}

void a0_heap_leave(void) {
    if (!a0_threads_active) return;
    if (--heap_depth > 0) return;
    mutex_unlock(&heap);
}

void a0_table_lock(void) {
    if (a0_threads_active) mutex_lock(&tables);
}

void a0_table_unlock(void) {
    if (a0_threads_active) mutex_unlock(&tables);
}

void a0_safe_begin(void) {
    if (a0_threads_active) a0_atomic_add(&stopped, 1);
}

void a0_safe_end(void) {
    if (!a0_threads_active) return;
    for (;;) {
        a0_atomic_add(&stopped, -1);
        if (!a0_atomic_load(&a0_gc_stop_requested)) return;
        /* A collection started: stay stopped until it ends (it holds the heap until then). */
        a0_atomic_add(&stopped, 1);
        mutex_lock(&heap);
        mutex_unlock(&heap);
    }
}

void a0_gc_safepoint(void) {
    a0_safe_begin();
    a0_safe_end();
}

int64_t *a0_gc_poll_address(void) { return (int64_t *)&a0_gc_stop_requested; }

void a0_stop_world(void) {
    if (!a0_threads_active) return;
    a0_atomic_store(&a0_gc_stop_requested, 1);
    while (a0_atomic_load(&stopped) < a0_atomic_load(&thread_count) - 1) yield_now();
}

void a0_start_world(void) {
    if (a0_threads_active) a0_atomic_store(&a0_gc_stop_requested, 0);
}

void a0_threads_attached(int64_t delta) { a0_atomic_add(&thread_count, delta); }

void a0_threads_reset(void) {
    a0_atomic_store(&thread_count, 0);
    a0_atomic_store(&stopped, 0);
}

void a0_fail_lock(void) {
    if (!a0_threads_active) return;
    mutex_lock(&failing);
}

/* ---- Thread objects ----------------------------------------------------------------------------- */

/* A thread's handle, an object: the closure it runs, its result, and its state. */
static const uint8_t thread_layout[3] = {A0_WORD_REF, A0_WORD_ANY, A0_WORD_RAW};
static const A0Type thread_type = {
    .name = "thread", .kind = A0_KIND_FIXED, .word_count = 3, .layout = (uint8_t *)thread_layout,
    .family = &thread_type};

typedef struct {
    void *closure;
    uint64_t result;
    A0_WORD_PTR(struct ThreadState *, state);
} ThreadObject;

#define SPAWN_FRAMES 64

typedef struct ThreadState {
    ThreadObject *object;
    int64_t id;
    int done;
    int joined;
    Mutex lock;
    Cond finished;
    uint32_t spawned_at[SPAWN_FRAMES];
    size_t spawned_count;
#if defined(_WIN32) && !defined(A0_NO_THREADS)
    HANDLE handle;
#elif !defined(A0_NO_THREADS)
    pthread_t handle;
#endif
    struct ThreadState *next_running;
} ThreadState;

/* Running threads, whose handles stay alive until they end even if the program drops them. */
static ThreadState *running;
static int64_t next_id = 2;
static A0_THREAD_LOCAL ThreadState *self;

void a0_threads_mark(void) {
    for (ThreadState *t = running; t != NULL; t = t->next_running) a0_gc_mark_object(t->object);
}

size_t a0_thread_spawn_frames(const uint32_t **ids, int64_t *id) {
    if (self == NULL) return 0;
    *ids = self->spawned_at;
    *id = self->id;
    return self->spawned_count;
}

static uint64_t call_closure(void *closure) {
    typedef uint64_t (*Code)(void *);
    Code code = (Code)(uintptr_t)(*(uint64_t *)closure);
    return code(closure);
}

static A0ThreadStarter starter = call_closure;

void a0_rt_set_thread_starter(A0ThreadStarter s) { starter = s != NULL ? s : call_closure; }

/* The shadow stack of a spawned thread, in words (the main thread's is larger). */
#define THREAD_SHADOW (1024 * 1024)
/* The native stack of a spawned thread: an executable's main thread's, unless the interpreter, whose
 * calls take more of it, says otherwise. */
static size_t thread_stack = (size_t)8 * 1024 * 1024;

void a0_rt_set_thread_stack(uint64_t bytes) { thread_stack = (size_t)bytes; }
#define THREAD_STACK thread_stack

static void run(ThreadState *t) {
    self = t;
    a0_gc_attach(THREAD_SHADOW);
    /* The handle on the thread's own stack, so that the result is stored in a live object. */
    uint64_t *frame = a0_gc_frame_push(1);
    frame[0] = a0_any_from_ref(t->object);
    uint64_t result = starter(t->object->closure);
    uint64_t *exception = a0_gc_exception_slots();
    if (exception[0] != 0) a0_rt_uncaught(exception[1]);
    t->object->result = result;
    a0_heap_enter();
    for (ThreadState **link = &running; *link != NULL; link = &(*link)->next_running) {
        if (*link == t) {
            *link = t->next_running;
            break;
        }
    }
    a0_heap_leave();
    frame[0] = 0;
    a0_gc_frame_pop(1);
    a0_gc_detach();
    mutex_lock(&t->lock);
    t->done = 1;
    cond_broadcast(&t->finished);
    mutex_unlock(&t->lock);
}

#if defined(_WIN32) && !defined(A0_NO_THREADS)
static DWORD WINAPI thread_main(LPVOID arg) {
    run((ThreadState *)arg);
    return 0;
}
#elif !defined(A0_NO_THREADS)
static void *thread_main(void *arg) {
    run((ThreadState *)arg);
    return NULL;
}
#endif

/* Defined in trace.c: the locations of the active frames. */
size_t a0_rt_frames(uint64_t loc, void *frame, uint32_t *out, size_t max);

uint64_t a0_thread_spawn(void *closure, uint64_t loc, void *frame) {
#if defined(A0_NO_THREADS)
    (void)closure;
    a0_rt_fail("threads are not supported on this platform", loc, frame);
#else
    if (closure == NULL) a0_panic_null();
    ThreadObject *object = (ThreadObject *)a0_gc_new((A0Type *)&thread_type);
    object->closure = closure;
    ThreadState *t = (ThreadState *)calloc(1, sizeof(ThreadState));
    if (t == NULL) a0_panic("out of memory");
    t->object = object;
    t->spawned_count = a0_rt_frames(loc, frame, t->spawned_at, SPAWN_FRAMES);
    mutex_init(&t->lock);
    cond_init(&t->finished);
    object->state = t;
    /* From here on, the heap is shared. */
    a0_threads_active = 1;
    a0_heap_enter();
    t->id = next_id++;
    t->next_running = running;
    running = t;
    a0_heap_leave();
#if defined(_WIN32)
    t->handle = CreateThread(NULL, THREAD_STACK, thread_main, t, STACK_SIZE_PARAM_IS_A_RESERVATION, NULL);
    if (t->handle == NULL) a0_rt_fail("cannot start a thread", loc, frame);
#else
    pthread_attr_t attr;
    pthread_attr_init(&attr);
    pthread_attr_setstacksize(&attr, THREAD_STACK);
    if (pthread_create(&t->handle, &attr, thread_main, t) != 0) {
        a0_rt_fail("cannot start a thread", loc, frame);
    }
    pthread_attr_destroy(&attr);
#endif
    return a0_any_from_ref(object);
#endif
}

static void *object_of(uint64_t handle, const A0Type *type, const char *what, uint64_t loc,
                       void *frame) {
    void *p = A0_ANY_IS_REF(handle) ? A0_ANY_PTR(handle) : NULL;
    if (p == NULL || a0_gc_type_of(p) != type) a0_rt_fail(what, loc, frame);
    return p;
}

uint64_t a0_thread_join(uint64_t handle, uint64_t loc, void *frame) {
    ThreadObject *object =
        (ThreadObject *)object_of(handle, &thread_type, "not a thread", loc, frame);
    ThreadState *t = object->state;
    if (t == NULL || t->joined) a0_rt_fail("this thread was joined already", loc, frame);
    t->joined = 1;
    a0_safe_begin();
    mutex_lock(&t->lock);
    while (!t->done) cond_wait(&t->finished, &t->lock);
    mutex_unlock(&t->lock);
#if defined(_WIN32) && !defined(A0_NO_THREADS)
    WaitForSingleObject(t->handle, INFINITE);
    CloseHandle(t->handle);
#elif !defined(A0_NO_THREADS)
    pthread_join(t->handle, NULL);
#endif
    a0_safe_end();
    object->state = NULL;
    free(t);
    return object->result;
}

int64_t a0_thread_started(void) { return a0_threads_active; }

int64_t a0_thread_id(void) { return self != NULL ? self->id : 1; }

int64_t a0_thread_parallelism(void) {
#if defined(A0_NO_THREADS)
    return 1;
#elif defined(_WIN32)
    SYSTEM_INFO info;
    GetSystemInfo(&info);
    return info.dwNumberOfProcessors > 0 ? (int64_t)info.dwNumberOfProcessors : 1;
#else
    long n = sysconf(_SC_NPROCESSORS_ONLN);
    return n > 0 ? (int64_t)n : 1;
#endif
}

/* ---- Channels ----------------------------------------------------------------------------------- */

/* A channel's handle, an object: its buffer (an array of `any`, used as a ring) and its state. */
static const uint8_t chan_layout[2] = {A0_WORD_REF, A0_WORD_RAW};
static const A0Type chan_type = {
    .name = "channel", .kind = A0_KIND_FIXED, .word_count = 2, .layout = (uint8_t *)chan_layout,
    .family = &chan_type};

typedef struct ChanState {
    Mutex lock;
    Cond changed;
    int64_t head, count, capacity;
    int closed;
} ChanState;

typedef struct {
    A0Array *buffer;
    A0_WORD_PTR(ChanState *, state);
} ChanObject;

static A0_THREAD_LOCAL int64_t receive_closed;

uint64_t a0_chan_new(int64_t capacity) {
    if (capacity < 1) capacity = 1;
    ChanObject *c = (ChanObject *)a0_gc_new((A0Type *)&chan_type);
    uint64_t *frame = a0_gc_frame_push(1);
    frame[0] = a0_any_from_ref(c);
    A0Array *buffer = a0_array_new(A0_WORD_ANY, capacity);
    buffer->len = capacity;
    c->buffer = buffer;
    frame[0] = 0;
    a0_gc_frame_pop(1);
    ChanState *s = (ChanState *)calloc(1, sizeof(ChanState));
    if (s == NULL) a0_panic("out of memory");
    mutex_init(&s->lock);
    cond_init(&s->changed);
    s->capacity = capacity;
    c->state = s;
    return a0_any_from_ref(c);
}

A0_COLD static void deadlock(const char *what, uint64_t loc, void *frame) {
    a0_rt_fail(what, loc, frame);
}

void a0_chan_send(uint64_t handle, uint64_t value, uint64_t loc, void *frame) {
    ChanObject *c = (ChanObject *)object_of(handle, &chan_type, "not a channel", loc, frame);
    ChanState *s = c->state;
    for (;;) {
        /* Waiting for room, the thread is stopped; it puts the value in once it runs again. */
        a0_safe_begin();
        mutex_lock(&s->lock);
        if (s->count == s->capacity && !s->closed && !a0_threads_active) {
            mutex_unlock(&s->lock);
            a0_safe_end();
            deadlock("deadlock: sending to a full channel, with no other thread to receive", loc,
                     frame);
        }
        while (s->count == s->capacity && !s->closed) cond_wait(&s->changed, &s->lock);
        mutex_unlock(&s->lock);
        a0_safe_end();
        mutex_lock(&s->lock);
        if (s->closed) {
            mutex_unlock(&s->lock);
            a0_rt_fail("send on a closed channel", loc, frame);
        }
        if (s->count < s->capacity) {
            c->buffer->data[(s->head + s->count) % s->capacity] = value;
            s->count++;
            cond_broadcast(&s->changed);
            mutex_unlock(&s->lock);
            return;
        }
        mutex_unlock(&s->lock);
    }
}

uint64_t a0_chan_recv(uint64_t handle, uint64_t loc, void *frame) {
    ChanObject *c = (ChanObject *)object_of(handle, &chan_type, "not a channel", loc, frame);
    ChanState *s = c->state;
    for (;;) {
        a0_safe_begin();
        mutex_lock(&s->lock);
        if (s->count == 0 && !s->closed && !a0_threads_active) {
            mutex_unlock(&s->lock);
            a0_safe_end();
            deadlock("deadlock: receiving from an empty channel, with no other thread to send", loc,
                     frame);
        }
        while (s->count == 0 && !s->closed) cond_wait(&s->changed, &s->lock);
        mutex_unlock(&s->lock);
        a0_safe_end();
        /* Running again: the value moves from the buffer to the caller with no collection between. */
        mutex_lock(&s->lock);
        if (s->count > 0) {
            uint64_t v = c->buffer->data[s->head];
            c->buffer->data[s->head] = 0;
            s->head = (s->head + 1) % s->capacity;
            s->count--;
            cond_broadcast(&s->changed);
            mutex_unlock(&s->lock);
            receive_closed = 0;
            return v;
        }
        if (s->closed) {
            mutex_unlock(&s->lock);
            receive_closed = 1;
            return A0_ANY_UNIT;
        }
        mutex_unlock(&s->lock);
    }
}

int64_t a0_chan_closed(void) { return receive_closed; }

void a0_chan_close(uint64_t handle, uint64_t loc, void *frame) {
    ChanObject *c = (ChanObject *)object_of(handle, &chan_type, "not a channel", loc, frame);
    ChanState *s = c->state;
    mutex_lock(&s->lock);
    if (s->closed) {
        mutex_unlock(&s->lock);
        a0_rt_fail("close of a closed channel", loc, frame);
    }
    s->closed = 1;
    cond_broadcast(&s->changed);
    mutex_unlock(&s->lock);
}
