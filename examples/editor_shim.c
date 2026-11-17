/* Shim para o exemplo de editor de texto em Loi.
 *
 * Loi nao pode converter um [u8] dinamico em cstr, e `DrawText` so aceita
 * strings terminadas em NUL. Este shim faz a copia temporaria.
 *
 * Compile junto com o shim do raylib:
 *   loi build examples/text_editor.loi -o editor \
 *       --c-source examples/editor_shim.c \
 *       --c-source vendor/raylib/raylib_shim.c \
 *       --cc-flag -Ivendor/raylib/include \
 *       --cc-flag -lraylib --cc-flag -lm
 */
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "raylib.h"

typedef struct {
    void* data;
    size_t len;
} LoiSlice;

void loi_draw_text_slice(LoiSlice text, int32_t len, int32_t x, int32_t y,
                         int32_t size, Color color) {
    if (text.data == NULL || len <= 0 || size <= 0) {
        return;
    }
    if ((size_t)len > text.len) {
        len = (int32_t)text.len;
    }
    char* buffer = (char*)malloc((size_t)len + 1);
    if (buffer == NULL) {
        return;
    }
    memcpy(buffer, text.data, (size_t)len);
    buffer[len] = '\0';
    DrawText(buffer, x, y, size, color);
    free(buffer);
}

int32_t loi_measure_text_slice(LoiSlice text, int32_t len, int32_t size) {
    if (text.data == NULL || len <= 0 || size <= 0) {
        return 0;
    }
    if ((size_t)len > text.len) {
        len = (int32_t)text.len;
    }
    char* buffer = (char*)malloc((size_t)len + 1);
    if (buffer == NULL) {
        return 0;
    }
    memcpy(buffer, text.data, (size_t)len);
    buffer[len] = '\0';
    int32_t width = MeasureText(buffer, size);
    free(buffer);
    return width;
}
