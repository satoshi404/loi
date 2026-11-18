#include <stdint.h>
#include <stdio.h>

#define KEYS 128u
#define STEPS 200000u

typedef struct {
    uint64_t key;
    uint64_t value;
} Entry;

static int64_t find_entry(const Entry *entries, size_t len, uint64_t key) {
    for (size_t i = 0; i < len; ++i) {
        if (entries[i].key == key) {
            return (int64_t)i;
        }
    }
    return -1;
}

static void set_entry(Entry *entries, size_t *len, uint64_t key, uint64_t value) {
    int64_t at = find_entry(entries, *len, key);
    if (at >= 0) {
        entries[(size_t)at].value = value;
        return;
    }
    entries[*len].key = key;
    entries[*len].value = value;
    ++*len;
}

static uint64_t get_entry(const Entry *entries, size_t len, uint64_t key) {
    int64_t at = find_entry(entries, len, key);
    if (at < 0) {
        return 0;
    }
    return entries[(size_t)at].value;
}

int main(void) {
    Entry entries[KEYS];
    size_t len = 0;
    for (uint64_t i = 0; i < KEYS; ++i) {
        set_entry(entries, &len, i, i * 7u + 11u);
    }

    uint64_t state = UINT64_C(0x8817264546332525);
    uint64_t sum = 0;
    for (uint64_t step = 0; step < STEPS; ++step) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        uint64_t key = state % KEYS;
        sum += get_entry(entries, len, key);
        set_entry(entries, &len, key, sum);
    }
    printf(
        "int_map %llu %llu\n",
        (unsigned long long)sum,
        (unsigned long long)state
    );
    return 0;
}
