/* Document sink for lib/lsp.loi.
 *
 * A real LSP client sends the current document text in `didOpen` and
 * `didChange`. The compiler checks files, so the server needs a way to write
 * that text to the URI's path before spawning `loi check`. This shim keeps the
 * operation at the FFI boundary: both arguments are ordinary Loi byte slices,
 * so no dynamic `cstr` coercion is needed in the language.
 */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    void* data;
    size_t len;
} LoiSlice;

int64_t loi_lsp_write_document(LoiSlice path, LoiSlice text) {
    if (path.len == 0 || path.len > 4096 || (path.len > 0 && path.data == NULL)) {
        return -1;
    }
    if (text.len > 0 && text.data == NULL) {
        return -1;
    }

    char* filename = (char*)malloc(path.len + 1);
    if (filename == NULL) {
        return -1;
    }
    memcpy(filename, path.data, path.len);
    filename[path.len] = '\0';

    FILE* file = fopen(filename, "wb");
    free(filename);
    if (file == NULL) {
        return -1;
    }

    size_t written = 0;
    if (text.len > 0) {
        written = fwrite(text.data, 1, text.len, file);
    }
    if (fclose(file) != 0 || written != text.len) {
        return -1;
    }
    return (int64_t)written;
}

int64_t loi_lsp_document_len(LoiSlice path) {
    if (path.len == 0 || path.len > 4096 || (path.len > 0 && path.data == NULL)) {
        return -1;
    }
    char* filename = (char*)malloc(path.len + 1);
    if (filename == NULL) {
        return -1;
    }
    memcpy(filename, path.data, path.len);
    filename[path.len] = '\0';
    FILE* file = fopen(filename, "rb");
    if (file == NULL) {
        free(filename);
        return -1;
    }
    if (fseek(file, 0, SEEK_END) != 0) {
        fclose(file);
        free(filename);
        return -1;
    }
    long length = ftell(file);
    fclose(file);
    free(filename);
    if (length < 0) {
        return -1;
    }
    return (int64_t)length;
}

int64_t loi_lsp_read_document(LoiSlice path, LoiSlice* data) {
    if (path.len == 0 || path.len > 4096 || (path.len > 0 && path.data == NULL)) {
        return -1;
    }
    if (data == NULL || (data->len > 0 && data->data == NULL)) {
        return -1;
    }
    char* filename = (char*)malloc(path.len + 1);
    if (filename == NULL) {
        return -1;
    }
    memcpy(filename, path.data, path.len);
    filename[path.len] = '\0';
    FILE* file = fopen(filename, "rb");
    if (file == NULL) {
        free(filename);
        return -1;
    }
    size_t got = fread(data->data, 1, data->len, file);
    fclose(file);
    free(filename);
    return (int64_t)got;
}

int64_t loi_lsp_remove_document(LoiSlice path) {
    if (path.len == 0 || path.len > 4096 || (path.len > 0 && path.data == NULL)) {
        return -1;
    }
    char* filename = (char*)malloc(path.len + 1);
    if (filename == NULL) {
        return -1;
    }
    memcpy(filename, path.data, path.len);
    filename[path.len] = '\0';
    int result = remove(filename);
    free(filename);
    return result == 0 ? 0 : -1;
}
