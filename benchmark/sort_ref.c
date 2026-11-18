#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#define LEN 65536u
#define ROUNDS 16u

static void swap_int32(int32_t *a, size_t i, size_t j) {
    int32_t saved = a[i];
    a[i] = a[j];
    a[j] = saved;
}

static void sift_down(int32_t *values, size_t start, size_t end) {
    size_t root = start;
    size_t child = root * 2u + 1u;
    while (child < end) {
        size_t largest = root;
        if (values[child] > values[largest]) {
            largest = child;
        }
        size_t right = child + 1u;
        if (right < end && values[right] > values[largest]) {
            largest = right;
        }
        if (largest == root) {
            break;
        }
        swap_int32(values, root, largest);
        root = largest;
        child = root * 2u + 1u;
    }
}

static void heap_sort(int32_t *values, size_t len) {
    if (len < 2u) {
        return;
    }
    size_t start = len / 2u;
    while (start > 0u) {
        --start;
        sift_down(values, start, len);
    }
    size_t end = len;
    while (end > 1u) {
        --end;
        swap_int32(values, 0u, end);
        sift_down(values, 0u, end);
    }
}

int main(void) {
    int32_t *values = calloc(LEN, sizeof(*values));
    if (values == NULL) {
        return 1;
    }

    for (uint64_t round = 0; round < ROUNDS; ++round) {
        uint64_t state = UINT64_C(0x123456789abcdef0) +
                         round * UINT64_C(0x9e3779b97f4a7c15);
        for (size_t i = 0; i < LEN; ++i) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            values[i] = (int32_t)(state % UINT64_C(1000003));
        }
        heap_sort(values, LEN);
    }

    uint64_t checksum = 0;
    for (size_t i = 0; i < LEN; ++i) {
        checksum += (uint64_t)values[i] * (uint64_t)((i % 97u) + 1u);
    }
    printf(
        "sort %llu %llu\n",
        (unsigned long long)checksum,
        (unsigned long long)(uint64_t)LEN
    );
    free(values);
    return 0;
}
