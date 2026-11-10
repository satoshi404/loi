#ifndef SAMPLE_H
#define SAMPLE_H

#define SAMPLE_FLAG 0x00000004u
#define SAMPLE_SHIFT (1 << 3)
#define SAMPLE_STR "hello"

typedef unsigned int Uint32;
typedef signed char Sint8;

typedef struct Thing { int x; double y; } Thing;
typedef struct { int w; int h; } Rect;

typedef struct Opaque Opaque;

typedef enum { MODE_A = 0, MODE_B = 2 } Mode;

Uint32 sample_init(Uint32 flags);
Thing sample_make(int x, double y);
Rect sample_rect(int w, int h);
int sample_get_size(Opaque *handle, int *w, int *h);
Opaque *sample_open(const char *name);
void sample_close(Opaque *handle);
void sample_copy_rect(const Rect *src, Rect *dst);
int sample_fill(char *buf, int len);
int sample_varargs(const char *fmt, ...);

#endif
