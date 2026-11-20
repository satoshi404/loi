/* C side of examples/callbacks.loi. The guard matches the one used by the
 * generated translation unit, so including both in one build is safe. */
#include <stdint.h>

#ifndef LOI_CALLBACK_DEFINED
#define LOI_CALLBACK_DEFINED
typedef int32_t (*LoiCallback)(int32_t, int32_t);
#endif

int32_t reduce_range(int32_t start, int32_t end, LoiCallback f) {
    int32_t acc = 0;
    for (int32_t i = start; i < end; i++) {
        acc = f(acc, i);
    }
    return acc;
}
