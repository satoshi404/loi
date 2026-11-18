#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define KEYS 8u
#define STEPS 100000u

static const char *const KEYS_TEXT[KEYS] = {
    "", "a", "ab", "abc", "abcd", "abcde", "abcdef", "abcdefg"
};

typedef struct {
    const char *key;
    uint64_t value;
} Entry;

static int64_t find_entry(const Entry *entries, size_t len, const char *key) {
    for (size_t i = 0; i < len; ++i) {
        if (strcmp(entries[i].key, key) == 0) {
            return (int64_t)i;
        }
    }
    return -1;
}

static void set_entry(
    Entry *entries, size_t *len, const char *key, uint64_t value
) {
    int64_t at = find_entry(entries, *len, key);
    if (at >= 0) {
        entries[(size_t)at].value = value;
        return;
    }
    if (*len >= KEYS) {
        fprintf(stderr, "str_map_ref: key capacity exceeded\n");
        exit(1);
    }
    entries[*len].key = key;
    entries[*len].value = value;
    ++*len;
}

static uint64_t get_entry(const Entry *entries, size_t len, const char *key) {
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
        set_entry(entries, &len, KEYS_TEXT[i], i + 1u);
    }

    uint64_t state = UINT64_C(0x9e3779b97f4a7c15);
    uint64_t sum = 0;
    for (uint64_t step = 0; step < STEPS; ++step) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        uint64_t key = state % KEYS;
        const char *selected = KEYS_TEXT[key];
        sum += get_entry(entries, len, selected);
        set_entry(entries, &len, selected, sum);
    }
    printf(
        "str_map %llu %llu\n",
        (unsigned long long)sum,
        (unsigned long long)state
    );
    return 0;
}
