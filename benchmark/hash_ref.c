#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#define BUFFER_LEN 65536u
#define ROUNDS 2048u
#define FNV_PRIME UINT64_C(0x100000001b3)

static uint64_t hash_bytes_seeded(
    const uint8_t *data, size_t len, uint64_t seed
) {
    uint64_t hash = seed;
    for (size_t i = 0; i < len; ++i) {
        hash ^= (uint64_t)data[i];
        hash *= FNV_PRIME;
    }
    return hash;
}

int main(void) {
    uint8_t *data = malloc(BUFFER_LEN);
    if (data == NULL) {
        return 1;
    }
    for (size_t i = 0; i < BUFFER_LEN; ++i) {
        data[i] = (uint8_t)((i * 131u + 7u) % 251u);
    }

    uint64_t seed = 42;
    for (uint64_t round = 0; round < ROUNDS; ++round) {
        seed = hash_bytes_seeded(data, BUFFER_LEN, seed) ^
               (round * UINT64_C(0x9e3779b97f4a7c15));
    }
    printf(
        "hash %llu %llu\n",
        (unsigned long long)seed,
        (unsigned long long)data[BUFFER_LEN - 1]
    );
    free(data);
    return 0;
}
