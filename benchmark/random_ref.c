#include <stdint.h>
#include <stdio.h>

#define DRAWS 100000000u
#define MULTIPLIER UINT64_C(2685821657736338717)

int main(void) {
    uint64_t state = UINT64_C(0x0123456789abcdef);
    uint64_t checksum = 0;
    for (uint64_t i = 0; i < DRAWS; ++i) {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        uint64_t value = state * MULTIPLIER;
        checksum ^= value;
        checksum = checksum * 31u + (value >> 32);
    }
    printf(
        "random %llu %llu\n",
        (unsigned long long)checksum,
        (unsigned long long)state
    );
    return 0;
}
