/* Deterministic stdio byte I/O for lib/stdio.loi.
 *
 * Build a Loi program that imports the library with:
 *   loi build main.loi -o main --c-source lib/stdio_shim.c
 *
 * The declarations mirror the `extern flow`s in lib/stdio.loi. Using stdio
 * throughout keeps this shim compatible with the compiler's `print` and
 * `printf` output; do not mix it with the raw descriptor helpers in the os or
 * net shims on the same stream.
 */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

typedef struct {
    void* data;
    size_t len;
} LoiSlice;

int64_t loi_stdio_read_exact(LoiSlice* data) {
    if (data == NULL || (data->len > 0 && data->data == NULL)) {
        return -1;
    }
    size_t total = 0;
    while (total < data->len) {
        size_t got = fread((char*)data->data + total, 1, data->len - total, stdin);
        total += got;
        if (got == 0) {
            return ferror(stdin) ? -1 : (int64_t)total;
        }
    }
    return (int64_t)total;
}

int64_t loi_stdio_write_all(LoiSlice data) {
    if (data.len > 0 && data.data == NULL) {
        return -1;
    }
    size_t total = 0;
    while (total < data.len) {
        size_t put = fwrite((const char*)data.data + total, 1, data.len - total, stdout);
        total += put;
        if (put == 0) {
            return ferror(stdout) ? -1 : (int64_t)total;
        }
    }
    return (int64_t)total;
}

void loi_stdio_flush(void) {
    (void)fflush(stdout);
}
