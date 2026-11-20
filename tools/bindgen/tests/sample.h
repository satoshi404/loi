#ifndef SAMPLE_H
#define SAMPLE_H

#include <stddef.h>

#define SAMPLE_FLAG 0x00000004u
#define SAMPLE_SHIFT (1 << 3)
#define SAMPLE_STR "hello"

typedef unsigned int Uint32;
typedef signed char Sint8;

typedef struct Thing { int x; double y; } Thing;
typedef struct { int w; int h; } Rect;

typedef struct Opaque Opaque;

typedef enum { MODE_A = 0, MODE_B = 1 } Mode;
typedef enum { SPARSE_A = 0, SPARSE_B = 2 } SparseMode;

Uint32 sample_init(Uint32 flags);
Thing sample_make(int x, double y);
Rect sample_rect(int w, int h);
int sample_get_size(Opaque *handle, int *w, int *h);
Opaque *sample_open(const char *name);
void sample_close(Opaque *handle);
void sample_copy_rect(const Rect *src, Rect *dst);
int sample_fill(char *buf, int len);
int sample_varargs(const char *fmt, ...);
Mode sample_next_mode(Mode mode);
int sample_mode_code(Mode mode);
int sample_sparse_code(SparseMode mode);
int sample_checksum(const unsigned char *bytes, size_t len);
int sample_sum_i32(const int *values, size_t len);
const unsigned char *sample_bad_pointer_return(void);
char *sample_bad_mutable_return(void);
int sample_bad_pointer_to_pointer(const unsigned char **values);

#endif
