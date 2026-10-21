#ifndef LOI_RUNTIME_H
#define LOI_RUNTIME_H

#include <stddef.h>

void *loi_alloc(size_t sz);
void loi_free(void *p);
void loi_print_int(long v);
void loi_print_str(const char *s);
void loi_print_str_raw(const char *s);
void loi_print_bytes(const char *s, size_t n);
void loi_print_bytes_nl(const char *s, size_t n);

/* concurrency (pthreads) */
long loi_thread_spawn(void *(*fn)(void*), void *arg);
int loi_thread_join(long tid);

/* serialization helpers */
int loi_write_file(const char *path, const char *data);

/* GC hook (minimal) */
void *loi_gc_alloc(size_t sz);
void loi_gc_collect(void);
void *loi_region_new(size_t capacity);
void *loi_region_alloc(void *region, size_t sz);
void loi_region_free(void *region);

/* channels */
typedef struct LoiChannel LoiChannel;
LoiChannel *loi_chan_new(size_t capacity);
void loi_chan_send(LoiChannel *c, void *val);
void *loi_chan_recv(LoiChannel *c);
void loi_chan_close(LoiChannel *c);
void loi_chan_free(LoiChannel *c);
void loi_gc_add_root(void **slot);
void loi_gc_remove_root(void **slot);

/* std helpers */
long loi_abs(long v);
long loi_min(long a, long b);
long loi_max(long a, long b);
double loi_sqrt(double v);
double loi_pow(double a, double b);
double loi_sin(double v);
double loi_cos(double v);
double loi_tan(double v);
double loi_exp(double v);
double loi_log(double v);
double loi_floor(double v);
double loi_ceil(double v);
size_t loi_strlen(const char *s);
void *loi_memcpy(void *d, const void *s, size_t n);
void *loi_memset(void *d, int v, size_t n);

#endif
