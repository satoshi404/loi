#ifndef LOI_BINDGEN_H
#define LOI_BINDGEN_H

/*
 * loi binding generator
 * ---------------------
 * `loi --mode=generator gen.h --out=thirdparty/sdl2/sdl2.loi`
 *
 * Reads a tiny C header (usually just `#include <SDL2/SDL.h>`), lets the
 * system C preprocessor resolve every include, and writes a loi package that
 * declares the library's constants, types and functions. The generated file
 * only contains declarations (`extern` functions carrying
 * `@attr(link_name("..."))`), so it costs nothing at run time.
 */

typedef struct {
    const char *header;         /* the gen.h file to process                    */
    const char *out_path;       /* output .loi file; NULL writes to stdout      */
    const char *package;        /* @package name; NULL derives it from out_path */
    const char *strip_prefix;   /* C prefix to drop (e.g. "SDL_"); NULL = auto  */
    const char *include_flags;  /* extra preprocessor flags, e.g. " -I'dir'"    */
} BindgenOptions;

/* Returns 0 on success, non-zero after printing a diagnostic to stderr. */
int loi_bindgen(const BindgenOptions *opt);

#endif /* LOI_BINDGEN_H */
