#ifndef LOI_PREPROCESSOR_H
#define LOI_PREPROCESSOR_H

/*
 * loi preprocessor
 * ----------------
 * The whole `@`/`#` metaprogramming layer: object/function-like and variadic
 * macros, `@stringify`/`@paste`, conditional compilation (`@if`/`@elif`/`@else`
 * /`@endif`), compile-time unrolling (`@repeat`/`@for`), and includes/imports
 * (`@inc`/`@cinc`, `import pkg.sub`) with automatic include guards.
 *
 * The driver calls `loi_preprocess` once, before lexing; everything it does is
 * erased by the time the parser runs, so there is no runtime cost.
 */

/* Run the full preprocessor pipeline over `src`.
 * `base_dir` is the directory of the source file (used to resolve includes);
 * pass "." when unknown. Returns a newly allocated NUL-terminated buffer that
 * the caller owns. */
char *loi_preprocess(const char *src, const char *base_dir);

#endif /* LOI_PREPROCESSOR_H */
