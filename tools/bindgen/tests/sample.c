#include "sample.h"

#include <string.h>

struct Opaque {
    int width;
    int height;
};

static struct Opaque shared = { 640, 480 };

Uint32 sample_init(Uint32 flags) {
    return flags ^ SAMPLE_FLAG;
}

Thing sample_make(int x, double y) {
    Thing thing = { x, y };
    return thing;
}

Rect sample_rect(int w, int h) {
    Rect rect = { w, h };
    return rect;
}

int sample_get_size(Opaque *handle, int *w, int *h) {
    if (handle == 0) {
        return -1;
    }
    *w = handle->width;
    *h = handle->height;
    return 0;
}

Opaque *sample_open(const char *name) {
    (void)name;
    return &shared;
}

void sample_close(Opaque *handle) {
    (void)handle;
}

void sample_copy_rect(const Rect *src, Rect *dst) {
    memcpy(dst, src, sizeof(*dst));
}

int sample_fill(char *buf, int len) {
    memset(buf, 'x', (size_t)len);
    return len;
}

int sample_varargs(const char *fmt, ...) {
    return (int)strlen(fmt);
}
