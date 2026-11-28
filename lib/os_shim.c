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
#ifndef _XOPEN_SOURCE
#define _XOPEN_SOURCE 700
#endif
#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 200809L
#endif

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <ftw.h>
#include <limits.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
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

bool loi_os_path_exists(LoiSlice path) {
    if (path.len == 0 || path.data == NULL) {
        return false;
    }
    char* cpath = (char*)malloc(path.len + 1);
    if (cpath == NULL) {
        return false;
    }
    memcpy(cpath, path.data, path.len);
    cpath[path.len] = '\0';
    struct stat info;
    bool found = stat(cpath, &info) == 0;
    free(cpath);
    return found;
}

int32_t loi_build_write_atomic(LoiSlice path, LoiSlice data) {
    if (path.len == 0 || path.data == NULL || (data.len > 0 && data.data == NULL)) {
        return -1;
    }
    char* cpath = (char*)malloc(path.len + 5);
    if (cpath == NULL) {
        return -1;
    }
    memcpy(cpath, path.data, path.len);
    memcpy(cpath + path.len, ".tmp", 5);
    int output = open(cpath, O_WRONLY | O_CREAT | O_TRUNC, (mode_t)0644);
    if (output < 0) {
        free(cpath);
        return -1;
    }
    size_t written = 0;
    while (written < data.len) {
        ssize_t put = write(output, (const char*)data.data + written, data.len - written);
        if (put < 0 && errno == EINTR) {
            continue;
        }
        if (put <= 0) {
            close(output);
            unlink(cpath);
            free(cpath);
            return -1;
        }
        written += (size_t)put;
    }
    char* final_path = (char*)malloc(path.len + 1);
    if (final_path == NULL) {
        close(output);
        unlink(cpath);
        free(cpath);
        return -1;
    }
    memcpy(final_path, path.data, path.len);
    final_path[path.len] = '\0';
    if (close(output) != 0 || rename(cpath, final_path) != 0) {
        unlink(cpath);
        free(final_path);
        free(cpath);
        return -1;
    }
    free(final_path);
    free(cpath);
    return 0;
}

int32_t loi_build_mkdir(LoiSlice path) {
    if (path.len == 0 || path.data == NULL) {
        return -1;
    }
    char* cpath = (char*)malloc(path.len + 1);
    if (cpath == NULL) {
        return -1;
    }
    memcpy(cpath, path.data, path.len);
    cpath[path.len] = '\0';
    size_t length = strlen(cpath);
    for (size_t at = 1; at <= length; ++at) {
        if (cpath[at] == '/' || cpath[at] == '\0') {
            char saved = cpath[at];
            cpath[at] = '\0';
            if (mkdir(cpath, (mode_t)0777) != 0 && errno != EEXIST) {
                free(cpath);
                return -1;
            }
            cpath[at] = saved;
        }
    }
    free(cpath);
    return 0;
}

/* Deterministic, explicit directory listing for declared asset trees.
 *
 * The first call passes a zero-length buffer and receives the number of bytes
 * needed for the NUL-separated, lexicographically sorted relative paths. The
 * caller allocates that buffer in its own protect region and calls again.
 * Symlinks are ignored so a declared tree cannot silently escape the project.
 */
typedef struct {
    char** items;
    size_t count;
    size_t capacity;
} LoiPathList;

static int loi_compare_paths(const void* left, const void* right) {
    const char* const* a = (const char* const*)left;
    const char* const* b = (const char* const*)right;
    return strcmp(*a, *b);
}

static char* loi_join_path(const char* left, const char* right) {
    size_t left_len = strlen(left);
    size_t right_len = strlen(right);
    size_t slash = left_len > 0 && left[left_len - 1] != '/' ? 1u : 0u;
    char* joined = (char*)malloc(left_len + slash + right_len + 1);
    if (joined == NULL) {
        return NULL;
    }
    memcpy(joined, left, left_len);
    if (slash) {
        joined[left_len] = '/';
    }
    memcpy(joined + left_len + slash, right, right_len + 1);
    return joined;
}

static int loi_path_list_push(LoiPathList* list, char* value) {
    if (list->count == list->capacity) {
        size_t capacity = list->capacity == 0 ? 8u : list->capacity * 2u;
        char** items = (char**)realloc(list->items, capacity * sizeof(char*));
        if (items == NULL) {
            return -1;
        }
        list->items = items;
        list->capacity = capacity;
    }
    list->items[list->count++] = value;
    return 0;
}

static void loi_path_list_free(LoiPathList* list) {
    for (size_t at = 0; at < list->count; ++at) {
        free(list->items[at]);
    }
    free(list->items);
    list->items = NULL;
    list->count = 0;
    list->capacity = 0;
}

static int loi_collect_files(const char* root, const char* relative, LoiPathList* list) {
    char* directory = loi_join_path(root, relative);
    if (directory == NULL) {
        return -1;
    }
    DIR* handle = opendir(directory);
    free(directory);
    if (handle == NULL) {
        return -1;
    }

    struct dirent* entry = NULL;
    while ((entry = readdir(handle)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) {
            continue;
        }
        char* child = loi_join_path(relative, entry->d_name);
        char* full = loi_join_path(root, child == NULL ? "" : child);
        if (child == NULL || full == NULL) {
            free(child);
            free(full);
            closedir(handle);
            return -1;
        }
        struct stat info;
        if (lstat(full, &info) == 0) {
            if (S_ISDIR(info.st_mode)) {
                if (loi_collect_files(root, child, list) != 0) {
                    free(child);
                    free(full);
                    closedir(handle);
                    return -1;
                }
            } else if (S_ISREG(info.st_mode)) {
                if (loi_path_list_push(list, child) != 0) {
                    free(child);
                    free(full);
                    closedir(handle);
                    return -1;
                }
                child = NULL;
            }
        }
        free(child);
        free(full);
    }
    closedir(handle);
    return 0;
}

int64_t loi_build_list_files(LoiSlice root, LoiSlice* buffer) {
    if (root.len == 0 || root.data == NULL) {
        return -1;
    }
    char* croot = (char*)malloc(root.len + 1);
    if (croot == NULL) {
        return -1;
    }
    memcpy(croot, root.data, root.len);
    croot[root.len] = '\0';

    struct stat info;
    if (lstat(croot, &info) != 0 || !S_ISDIR(info.st_mode)) {
        free(croot);
        return -1;
    }

    LoiPathList list = { NULL, 0, 0 };
    if (loi_collect_files(croot, "", &list) != 0) {
        free(croot);
        loi_path_list_free(&list);
        return -1;
    }
    free(croot);
    if (list.count > 1) {
        qsort(list.items, list.count, sizeof(char*), loi_compare_paths);
    }

    size_t needed = 0;
    for (size_t at = 0; at < list.count; ++at) {
        needed += strlen(list.items[at]) + 1u;
    }
    if (buffer == NULL || buffer->len == 0 || buffer->data == NULL) {
        loi_path_list_free(&list);
        return (int64_t)needed;
    }
    if (buffer->len < needed) {
        loi_path_list_free(&list);
        return -1;
    }

    size_t at = 0;
    for (size_t item = 0; item < list.count; ++item) {
        size_t length = strlen(list.items[item]) + 1u;
        memcpy((char*)buffer->data + at, list.items[item], length);
        at += length;
    }
    loi_path_list_free(&list);
    return (int64_t)at;
}

/* Remove one declared clean path. FTW_DEPTH lets a directory be deleted only
 * after its children; FTW_PHYS keeps symlinks as entries instead of following
 * them outside the project. */
static int loi_remove_entry(const char* path, const struct stat* info, int type, struct FTW* state) {
    (void)info;
    (void)state;
    if (type == FTW_DNR || type == FTW_NS) {
        return -1;
    }
    return remove(path);
}

int32_t loi_build_remove(LoiSlice path) {
    if (path.len == 0 || path.data == NULL) {
        return -1;
    }
    char* cpath = (char*)malloc(path.len + 1);
    if (cpath == NULL) {
        return -1;
    }
    memcpy(cpath, path.data, path.len);
    cpath[path.len] = '\0';
    struct stat info;
    if (lstat(cpath, &info) != 0) {
        free(cpath);
        return 0;
    }
    int status = nftw(cpath, loi_remove_entry, 16, FTW_DEPTH | FTW_PHYS);
    free(cpath);
    return status == 0 ? 0 : -1;
}

/* Copy one declared file while preserving its permission bits, so an installed
 * executable remains executable. No program or shell is involved. */
int32_t loi_build_copy(LoiSlice source, LoiSlice destination) {
    if (source.len == 0 || source.data == NULL || destination.len == 0 || destination.data == NULL) {
        return -1;
    }
    char* from = (char*)malloc(source.len + 1);
    char* to = (char*)malloc(destination.len + 1);
    if (from == NULL || to == NULL) {
        free(from);
        free(to);
        return -1;
    }
    memcpy(from, source.data, source.len);
    from[source.len] = '\0';
    memcpy(to, destination.data, destination.len);
    to[destination.len] = '\0';

    struct stat info;
    int status = 0;
    int input = open(from, O_RDONLY);
    if (input < 0) {
        status = -1;
        goto done;
    }
    if (fstat(input, &info) != 0) {
        close(input);
        status = -1;
        goto done;
    }
    int output = open(to, O_WRONLY | O_CREAT | O_TRUNC, info.st_mode & 07777);
    if (output < 0) {
        close(input);
        status = -1;
        goto done;
    }
    char buffer[8192];
    for (;;) {
        ssize_t got = read(input, buffer, sizeof(buffer));
        if (got < 0) {
            if (errno == EINTR) {
                continue;
            }
            status = -1;
            break;
        }
        if (got == 0) {
            status = 0;
            break;
        }
        size_t written = 0;
        while (written < (size_t)got) {
            ssize_t put = write(output, buffer + written, (size_t)got - written);
            if (put < 0) {
                if (errno == EINTR) {
                    continue;
                }
                written = (size_t)got;
                status = -1;
                break;
            }
            if (put == 0) {
                status = -1;
                break;
            }
            written += (size_t)put;
        }
        if (status != 0) {
            break;
        }
    }
    if (close(output) != 0) {
        status = -1;
    }
    close(input);
    if (status == 0 && chmod(to, info.st_mode & 07777) != 0) {
        status = -1;
    }

done:
    free(from);
    free(to);
    return status;
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
