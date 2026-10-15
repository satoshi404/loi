#include "runtime.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <pthread.h>
#include <unistd.h>
#include <stdatomic.h>>

void *loi_alloc(size_t sz) {
    return malloc(sz ? sz : 1);
}

void loi_free(void *p) {
    free(p);
}

void loi_print_int(long v) {
    printf("%ld\n", v);
}

void loi_print_str(const char *s) {
    if (s) puts(s);
}

long loi_thread_spawn(void *(*fn)(void*), void *arg) {
    pthread_t th;
    if (pthread_create(&th, NULL, fn, arg) != 0) return -1;
    return (long)th;
}

int loi_thread_join(long tid) {
    pthread_t th = (pthread_t)tid;
    return pthread_join(th, NULL);
}

int loi_write_file(const char *path, const char *data) {
    if (!path) return -1;
    FILE *f = fopen(path, "w");
    if (!f) return -1;
    if (data) fputs(data, f);
    fclose(f);
    return 0;
}

/* ---------- standard library helpers ---------- */
long loi_abs(long v) { return v < 0 ? -v : v; }
long loi_min(long a, long b) { return a < b ? a : b; }
long loi_max(long a, long b) { return a > b ? a : b; }
double loi_sqrt(double v) { return sqrt(v); }
double loi_pow(double a, double b) { return pow(a, b); }
double loi_sin(double v) { return sin(v); }
double loi_cos(double v) { return cos(v); }
double loi_tan(double v) { return tan(v); }
double loi_exp(double v) { return exp(v); }
double loi_log(double v) { return log(v); }
double loi_floor(double v) { return floor(v); }
double loi_ceil(double v) { return ceil(v); }
size_t loi_strlen(const char *s) { return s ? strlen(s) : 0; }
void *loi_memcpy(void *d, const void *s, size_t n) { return memcpy(d, s, n); }
void *loi_memset(void *d, int v, size_t n) { return memset(d, v, n); }

/* ---------- GC with roots and tracking ---------- */
typedef struct GcObj {
    void *ptr;
    size_t size;
    int mark;
    struct GcObj *next;
} GcObj;

static GcObj *gc_head = NULL;

typedef struct GcRoot {
    void **slot;
    struct GcRoot *next;
} GcRoot;

static GcRoot *gc_roots = NULL;

void *loi_gc_alloc(size_t sz) {
    GcObj *o = (GcObj *)malloc(sizeof(GcObj));
    if (!o) return NULL;
    o->ptr = malloc(sz ? sz : 1);
    o->size = sz ? sz : 1;
    o->mark = 0;
    o->next = gc_head;
    gc_head = o;
    return o->ptr;
}

void loi_gc_add_root(void **slot) {
    if (!slot) return;
    GcRoot *r = (GcRoot *)malloc(sizeof(GcRoot));
    if (!r) return;
    r->slot = slot;
    r->next = gc_roots;
    gc_roots = r;
}

void loi_gc_remove_root(void **slot) {
    GcRoot **pp = &gc_roots;
    while (*pp) {
        if ((*pp)->slot == slot) {
            GcRoot *tmp = *pp;
            *pp = tmp->next;
            free(tmp);
            return;
        }
        pp = &(*pp)->next;
    }
}

/* Conservative marking: mark any object whose pointer is present in root slots. */
static void gc_mark(void) {
    for (GcObj *o = gc_head; o; o = o->next) o->mark = 0;
    for (GcRoot *r = gc_roots; r; r = r->next) {
        if (r->slot && *r->slot) {
            for (GcObj *o = gc_head; o; o = o->next) {
                if (o->ptr == *r->slot) o->mark = 1;
            }
        }
    }
}

void loi_gc_collect(void) {
    gc_mark();
    GcObj **pp = &gc_head;
    while (*pp) {
        if (!(*pp)->mark) {
            GcObj *tmp = *pp;
            *pp = tmp->next;
            free(tmp->ptr);
            free(tmp);
        } else {
            pp = &(*pp)->next;
        }
    }
}

/* ---------- region (metaprogramming memory regions) ---------- */
typedef struct { char *base; size_t cap; size_t used; } LoiRegion;
void *loi_region_new(size_t capacity) {
    LoiRegion *r = (LoiRegion *)malloc(sizeof(LoiRegion));
    if (!r) return NULL;
    r->base = (char *)malloc(capacity ? capacity : 1);
    r->cap = capacity ? capacity : 1;
    r->used = 0;
    return r;
}
void *loi_region_alloc(void *region, size_t sz) {
    LoiRegion *r = (LoiRegion *)region;
    if (!r || !r->base) return NULL;
    if (r->used + sz > r->cap) return NULL;
    void *p = r->base + r->used;
    r->used += sz;
    return p;
}
void loi_region_free(void *region) {
    LoiRegion *r = (LoiRegion *)region;
    if (!r) return;
    free(r->base);
    free(r);
}

/* channels */
struct LoiChannel {
    pthread_mutex_t m;
    pthread_cond_t not_empty;
    pthread_cond_t not_full;
    void **buf;
    size_t cap, count, head, tail;
    int closed;
};

LoiChannel *loi_chan_new(size_t capacity) {
    LoiChannel *c = (LoiChannel *)malloc(sizeof(LoiChannel));
    if (!c) return NULL;
    if (capacity == 0) capacity = 1;
    c->buf = (void **)malloc(capacity * sizeof(void *));
    if (!c->buf) { free(c); return NULL; }
    pthread_mutex_init(&c->m, NULL);
    pthread_cond_init(&c->not_empty, NULL);
    pthread_cond_init(&c->not_full, NULL);
    c->cap = capacity;
    c->count = c->head = c->tail = 0;
    c->closed = 0;
    return c;
}

void loi_chan_send(LoiChannel *c, void *val) {
    if (!c) return;
    pthread_mutex_lock(&c->m);
    while (c->count == c->cap && !c->closed) pthread_cond_wait(&c->not_full, &c->m);
    if (c->closed) { pthread_mutex_unlock(&c->m); return; }
    c->buf[c->tail] = val;
    c->tail = (c->tail + 1) % c->cap;
    c->count++;
    pthread_cond_signal(&c->not_empty);
    pthread_mutex_unlock(&c->m);
}

void *loi_chan_recv(LoiChannel *c) {
    if (!c) return NULL;
    pthread_mutex_lock(&c->m);
    while (c->count == 0 && !c->closed) pthread_cond_wait(&c->not_empty, &c->m);
    if (c->count == 0) { pthread_mutex_unlock(&c->m); return NULL; }
    void *v = c->buf[c->head];
    c->head = (c->head + 1) % c->cap;
    c->count--;
    pthread_cond_signal(&c->not_full);
    pthread_mutex_unlock(&c->m);
    return v;
}

void loi_chan_close(LoiChannel *c) {
    if (!c) return;
    pthread_mutex_lock(&c->m);
    c->closed = 1;
    pthread_cond_broadcast(&c->not_empty);
    pthread_cond_broadcast(&c->not_full);
    pthread_mutex_unlock(&c->m);
}

void loi_chan_free(LoiChannel *c) {
    if (!c) return;
    loi_chan_close(c);
    pthread_mutex_destroy(&c->m);
    pthread_cond_destroy(&c->not_empty);
    pthread_cond_destroy(&c->not_full);
    free(c->buf);
    free(c);
}
