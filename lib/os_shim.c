/* POSIX process/environment and file-handle shim for lib/os.loi.
 *
 * Build a Loi program that imports the library with:
 *   loi build main.loi -o main --c-source lib/os_shim.c
 *
 * The declarations mirror the `extern flow`s in lib/os.loi. Keeping the POSIX
 * calls here means the compiler needs no extra intrinsics and no foreign
 * pointer types leak into Loi values. File descriptors are owned by Loi's
 * linear `File` handle and released by `guard`.
 */
#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 199309L
#endif

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

typedef struct {
    void* data;
    size_t len;
} LoiSlice;

int64_t loi_os_unix_time(void) {
    return (int64_t)time(NULL);
}

void loi_os_sleep_ms(int32_t millis) {
    if (millis <= 0) {
        return;
    }
    struct timespec remaining = {
        .tv_sec = millis / 1000,
        .tv_nsec = (long)(millis % 1000) * 1000000L,
    };
    while (nanosleep(&remaining, &remaining) == -1 && errno == EINTR) {
    }
}

bool loi_os_has_env(const char* name) {
    return name != NULL && getenv(name) != NULL;
}

int32_t loi_os_env_i32(const char* name, int32_t fallback) {
    if (name == NULL) {
        return fallback;
    }
    const char* text = getenv(name);
    if (text == NULL || *text == '\0') {
        return fallback;
    }
    char* end = NULL;
    long value = strtol(text, &end, 10);
    if (end == text || *end != '\0' || value < INT32_MIN || value > INT32_MAX) {
        return fallback;
    }
    return (int32_t)value;
}

int64_t loi_os_getenv(LoiSlice name, LoiSlice* data) {
    if (data == NULL || (name.len > 0 && name.data == NULL)) {
        return -1;
    }
    char* key = (char*)malloc(name.len + 1);
    if (key == NULL) {
        return -1;
    }
    if (name.len > 0) {
        memcpy(key, name.data, name.len);
    }
    key[name.len] = '\0';
    const char* value = getenv(key);
    free(key);
    if (value == NULL) {
        return -1;
    }
    size_t length = strlen(value);
    size_t count = length < data->len ? length : data->len;
    if (count > 0) {
        if (data->data == NULL) {
            return -1;
        }
        memcpy(data->data, value, count);
    }
    return (int64_t)length;
}

int32_t loi_os_errno(void) {
    return (int32_t)errno;
}

int32_t loi_io_open(const char* path, uint32_t write) {
    if (path == NULL) {
        return -1;
    }
    int flags = write ? (O_WRONLY | O_CREAT | O_TRUNC) : O_RDONLY;
    int fd = open(path, flags, (mode_t)0644);
    return fd < 0 ? -1 : (int32_t)fd;
}

int64_t loi_io_read(int32_t fd, LoiSlice* data) {
    if (fd < 0 || data == NULL || (data->len > 0 && data->data == NULL)) {
        return -1;
    }
    ssize_t total = 0;
    while ((size_t)total < data->len) {
        ssize_t got = read((int)fd, (char*)data->data + total, data->len - (size_t)total);
        if (got < 0) {
            if (errno == EINTR) {
                continue;
            }
            return -1;
        }
        if (got == 0) {
            break;
        }
        total += got;
    }
    return (int64_t)total;
}

int64_t loi_io_write(int32_t fd, LoiSlice data) {
    if (fd < 0 || (data.len > 0 && data.data == NULL)) {
        return -1;
    }
    ssize_t total = 0;
    while ((size_t)total < data.len) {
        ssize_t put = write((int)fd, (const char*)data.data + total, data.len - (size_t)total);
        if (put < 0) {
            if (errno == EINTR) {
                continue;
            }
            return -1;
        }
        if (put == 0) {
            break;
        }
        total += put;
    }
    return (int64_t)total;
}
