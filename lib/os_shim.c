/* POSIX process/environment shim for lib/os.loi.
 *
 * Build a Loi program that imports the library with:
 *   loi build main.loi -o main --c-source lib/os_shim.c
 *
 * The declarations mirror the `extern flow`s in lib/os.loi. Keeping the POSIX
 * calls here means the compiler needs no extra intrinsics and no foreign
 * pointer types leak into Loi values.
 */
#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 199309L
#endif

#include <errno.h>
#include <limits.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>

int32_t loi_os_pid(void) {
    return (int32_t)getpid();
}

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

int32_t loi_os_errno(void) {
    return (int32_t)errno;
}

void loi_os_exit(int32_t code) {
    exit((int)code);
}
