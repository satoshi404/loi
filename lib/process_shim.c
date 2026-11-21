/* Process spawning with captured output for lib/process.loi.
 *
 * Build a Loi program that imports the library with:
 *   loi build main.loi -o main --c-source lib/process_shim.c
 *
 * The declarations mirror the `extern flow`s in lib/process.loi. A child runs
 * through /bin/sh -c; lib/process.loi builds that command with shell-safe
 * quoting from an explicit program and argument list. stdout and stderr are
 * redirected to temporary files before exec, so a child that writes a lot to
 * either stream cannot deadlock the parent while it waits.
 */
#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

typedef struct {
    void* data;
    size_t len;
} LoiSlice;

#define LOI_PROCESS_SLOTS 64

typedef struct {
    int used;
    char* out;
    size_t out_len;
    char* err;
    size_t err_len;
    int32_t status;
} LoiProcess;

static LoiProcess loi_process_table[LOI_PROCESS_SLOTS];

static char* loi_process_temp_cstr(LoiSlice slice) {
    if (slice.len > 0 && slice.data == NULL) {
        return NULL;
    }
    char* text = (char*)malloc(slice.len + 1);
    if (text == NULL) {
        return NULL;
    }
    if (slice.len > 0) {
        memcpy(text, slice.data, slice.len);
    }
    text[slice.len] = '\0';
    return text;
}

static int loi_process_slurp(FILE* file, char** out, size_t* out_len) {
    *out = NULL;
    *out_len = 0;
    if (fseek(file, 0, SEEK_END) != 0) {
        return -1;
    }
    long end = ftell(file);
    if (end < 0) {
        return -1;
    }
    rewind(file);
    size_t len = (size_t)end;
    if (len == 0) {
        return 0;
    }
    char* buffer = (char*)malloc(len);
    if (buffer == NULL) {
        return -1;
    }
    if (fread(buffer, 1, len, file) != len) {
        free(buffer);
        return -1;
    }
    *out = buffer;
    *out_len = len;
    return 0;
}

static LoiProcess* loi_process_slot(int32_t handle) {
    if (handle <= 0 || (size_t)handle > LOI_PROCESS_SLOTS) {
        return NULL;
    }
    LoiProcess* slot = &loi_process_table[handle - 1];
    return slot->used ? slot : NULL;
}

int32_t loi_process_spawn(LoiSlice command) {
    char* text = loi_process_temp_cstr(command);
    if (text == NULL) {
        return -1;
    }
    FILE* out_file = tmpfile();
    FILE* err_file = tmpfile();
    if (out_file == NULL || err_file == NULL) {
        free(text);
        if (out_file != NULL) {
            fclose(out_file);
        }
        if (err_file != NULL) {
            fclose(err_file);
        }
        return -1;
    }

    pid_t pid = fork();
    if (pid < 0) {
        free(text);
        fclose(out_file);
        fclose(err_file);
        return -1;
    }
    if (pid == 0) {
        if (dup2(fileno(out_file), STDOUT_FILENO) < 0 ||
            dup2(fileno(err_file), STDERR_FILENO) < 0) {
            _exit(126);
        }
        execl("/bin/sh", "sh", "-c", text, (char*)NULL);
        _exit(127);
    }
    free(text);

    int wait_status = 0;
    while (waitpid(pid, &wait_status, 0) < 0) {
        if (errno != EINTR) {
            fclose(out_file);
            fclose(err_file);
            return -1;
        }
    }

    char* out = NULL;
    char* err = NULL;
    size_t out_len = 0;
    size_t err_len = 0;
    int ok = loi_process_slurp(out_file, &out, &out_len) == 0 &&
             loi_process_slurp(err_file, &err, &err_len) == 0;
    fclose(out_file);
    fclose(err_file);
    if (!ok) {
        free(out);
        free(err);
        return -1;
    }

    int32_t handle = -1;
    for (size_t i = 0; i < LOI_PROCESS_SLOTS; i++) {
        if (!loi_process_table[i].used) {
            handle = (int32_t)i + 1;
            break;
        }
    }
    if (handle < 0) {
        free(out);
        free(err);
        return -1;
    }

    LoiProcess* slot = &loi_process_table[handle - 1];
    slot->used = 1;
    slot->out = out;
    slot->out_len = out_len;
    slot->err = err;
    slot->err_len = err_len;
    if (WIFEXITED(wait_status)) {
        slot->status = (int32_t)WEXITSTATUS(wait_status);
    } else if (WIFSIGNALED(wait_status)) {
        slot->status = 128 + (int32_t)WTERMSIG(wait_status);
    } else {
        slot->status = -1;
    }
    return handle;
}

int64_t loi_process_output_len(int32_t handle, int32_t which) {
    LoiProcess* slot = loi_process_slot(handle);
    if (slot == NULL) {
        return -1;
    }
    if (which == 0) {
        return (int64_t)slot->out_len;
    }
    if (which == 1) {
        return (int64_t)slot->err_len;
    }
    return -1;
}

int64_t loi_process_read_output(int32_t handle, int32_t which, LoiSlice* data) {
    LoiProcess* slot = loi_process_slot(handle);
    if (slot == NULL || data == NULL || (data->len > 0 && data->data == NULL)) {
        return -1;
    }
    const char* source = NULL;
    size_t source_len = 0;
    if (which == 0) {
        source = slot->out;
        source_len = slot->out_len;
    } else if (which == 1) {
        source = slot->err;
        source_len = slot->err_len;
    } else {
        return -1;
    }
    size_t count = source_len < data->len ? source_len : data->len;
    if (count > 0) {
        memcpy(data->data, source, count);
    }
    return (int64_t)source_len;
}

int32_t loi_process_exit_code(int32_t handle) {
    LoiProcess* slot = loi_process_slot(handle);
    return slot == NULL ? -1 : slot->status;
}

void loi_process_release(int32_t handle) {
    LoiProcess* slot = loi_process_slot(handle);
    if (slot == NULL) {
        return;
    }
    free(slot->out);
    free(slot->err);
    slot->out = NULL;
    slot->err = NULL;
    slot->out_len = 0;
    slot->err_len = 0;
    slot->status = -1;
    slot->used = 0;
}
