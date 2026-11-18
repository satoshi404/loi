#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#define BUFFER_LEN 65536u
#define ROUNDS 1024u
#define POLY UINT32_C(0xedb88320)

static const uint32_t NIBBLE[16] = {
    UINT32_C(0x00000000), UINT32_C(0x1db71064), UINT32_C(0x3b6e20c8),
    UINT32_C(0x26d930ac), UINT32_C(0x76dc4190), UINT32_C(0x6b6b51f4),
    UINT32_C(0x4db26158), UINT32_C(0x5005713c), UINT32_C(0xedb88320),
    UINT32_C(0xf00f9344), UINT32_C(0xd6d6a3e8), UINT32_C(0xcb61b38c),
    UINT32_C(0x9b64c2b0), UINT32_C(0x86d3d2d4), UINT32_C(0xa00ae278),
    UINT32_C(0xbdbdf21c)
};

static uint32_t crc32_update(uint32_t state, const uint8_t *data, size_t len) {
    uint32_t crc = state;
    for (size_t i = 0; i < len; ++i) {
        crc ^= (uint32_t)data[i];
        crc = (crc >> 4) ^ NIBBLE[crc & 0xfu];
        crc = (crc >> 4) ^ NIBBLE[crc & 0xfu];
    }
    return crc;
}

int main(void) {
    uint8_t *data = malloc(BUFFER_LEN);
    if (data == NULL) {
        return 1;
    }
    for (size_t i = 0; i < BUFFER_LEN; ++i) {
        data[i] = (uint8_t)((i * 97u + 13u) % 251u);
    }

    uint32_t state = UINT32_MAX;
    for (uint64_t round = 0; round < ROUNDS; ++round) {
        state = crc32_update(state, data, BUFFER_LEN);
    }
    uint32_t result = state ^ UINT32_MAX;
    printf(
        "crc32 %llu %llu\n",
        (unsigned long long)result,
        (unsigned long long)data[BUFFER_LEN - 1]
    );
    free(data);
    return 0;
}
