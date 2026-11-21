/* C side of examples/callback_signatures.loi. The typedef guards match the
 * generated translation unit, so including both in one build is safe. */
#include <stdint.h>

#ifndef LOICALLBACK_1_I32_I32_DEFINED
#define LOICALLBACK_1_I32_I32_DEFINED
typedef int32_t (*LoiCallback_1_i32_i32)(int32_t);
#endif

#ifndef LOICALLBACK_1_I32_VOID_DEFINED
#define LOICALLBACK_1_I32_VOID_DEFINED
typedef void (*LoiCallback_1_i32_void)(int32_t);
#endif

int32_t sum_mapped(int32_t count, LoiCallback_1_i32_i32 f) {
    int32_t total = 0;
    for (int32_t i = 0; i < count; i++) {
        total += f(i);
    }
    return total;
}

void visit(int32_t count, LoiCallback_1_i32_void f) {
    for (int32_t i = 0; i < count; i++) {
        f(i);
    }
}
