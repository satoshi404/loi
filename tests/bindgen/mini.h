/* A tiny C header that exercises the binding generator. */
#ifndef MINI_H
#define MINI_H
#include <stdint.h>

#define MINI_MAGIC 0x2A
#define MINI_FLAG(n) (1u << (n))
#define MINI_FLAGS (MINI_FLAG(3) | MINI_FLAG(1))   /* a function-like macro, expanded: 10 */
#define MINI_NAME "mini"

typedef enum { MINI_RED, MINI_GREEN = 5, MINI_BLUE } MINI_Color;
typedef uint32_t MINI_Id;
typedef struct MINI_Handle MINI_Handle;              /* opaque: used through pointers */
typedef struct { int x; int y; } MINI_Point;
typedef union {
    uint32_t kind;
    struct { uint32_t kind; MINI_Point at; } move;
    uint8_t raw[16];
} MINI_Event;
typedef int (*MINI_Callback)(int);

extern int abs(int x);                               /* a real libc function */
MINI_Handle *MINI_open(const char *name, MINI_Id id);
int MINI_poll(MINI_Event *ev, MINI_Callback cb);
int MINI_printf(const char *fmt, ...);               /* variadic: skipped */
MINI_Point MINI_origin(void);                        /* struct by value: skipped */
#endif
