# loi — C libraries: `thirdparty/`, `--lib` and the binding generator

loi reaches C libraries through `extern` declarations. Writing them by hand
does not scale, so there are three pieces that work together:

| Piece            | What it does                                                        |
| ---------------- | ------------------------------------------------------------------- |
| `thirdparty/`    | where bindings live; `@inc sdl2` finds `thirdparty/sdl2/sdl2.loi`   |
| `--lib=NAME,...` | links the C libraries when building (`--run`, `--emit=exe`, `asm`)  |
| `--mode=generator` | writes a binding from a C header                                  |

```sh
# 1. generate the binding from the C headers (once)
echo '#include <SDL2/SDL.h>' > gen.h
./loi --mode=generator gen.h --out=thirdparty/sdl2/sdl2.loi

# 2. build and run a program that uses it
./loi examples/sdl2_window.loi --lib=SDL2 --run
```

## Directory convention

```
lib/                 runtime of the language (std, std.math, std.os)
thirdparty/          bindings to external C libraries, vendor style
└── sdl2/
    └── sdl2.loi     generated, then adjusted by hand if you like
```

`lib/` is the language itself; `thirdparty/` is C. A binding file must begin
with `@package(name)`.

`@inc name` is resolved in this order: the including file's directory, the path
as written, `./lib/name`, `./thirdparty/name/name.loi`, `./thirdparty/name.loi`,
`./thirdparty/name`, `./packages`, `/usr/share/loi` and `/usr/local/share/loi`
(also their `thirdparty/` subdirectories), and `$LOI_PATH` (also
`$LOI_PATH/thirdparty/...`). `@inc sdl2.mixer` maps the dot to a path
(`thirdparty/sdl2/mixer/mixer.loi`, `thirdparty/sdl2/mixer.loi`).

## `--lib`

```sh
./loi game.loi --lib=SDL2 --run
./loi game.loi --lib=SDL2,SDL2_mixer --emit=exe --out=game
./loi game.loi --lib=SDL2 --emit=asm
```

A comma separated list (`--lib NAME` also works). Each name becomes linker
flags, trying in order:

1. `pkg-config --libs NAME` (then the lower-case name: `SDL2` → `sdl2`)
2. `<lower-case-name>-config --libs` (`sdl2-config`, `freetype-config`, ...)
3. `-lNAME`

Names may only contain letters, digits and `_ + . -`. `--lib` composes with
`$LOI_LDFLAGS`, which is still appended. With `--emit=asm` nothing is linked,
so the flags are recorded as a trailing `# loi: link with ...` comment in the
assembly. If a link fails and no `--lib` was given, the error suggests it.

## The generator

```sh
./loi --mode=generator gen.h --out=thirdparty/sdl2/sdl2.loi
```

`gen.h` is a tiny C file, normally just the library's umbrella include:

```c
#include <SDL2/SDL.h>
```

The generator asks the system C compiler (`$CC`, default `cc`) to preprocess
it, so every `#include` is resolved through the normal system paths, then parses
the C declarations itself (it has no libclang dependency, so it works wherever a
C compiler does). Options:

| Option                | Meaning                                                         |
| --------------------- | --------------------------------------------------------------- |
| `--out=FILE`          | output `.loi` (directories are created); stdout when omitted    |
| `--package=NAME`      | `@package` name; default is the output file's stem              |
| `--strip-prefix=PFX`  | C prefix to drop from names; default is detected (`SDL_`)       |
| `--lib=NAME`          | add the library's `pkg-config --cflags-only-I` include paths    |
| `-Idir`               | extra header search path                                        |

### What is exported

Only declarations from the library's own headers: the directory of each header
that `gen.h` includes directly (`/usr/include/SDL2/` for `<SDL2/SDL.h>`). Headers
of the C standard library (`stdint.h`, `stdlib.h`, ...) feed the type table but
are never exported.

| C                                   | loi                                                    |
| ----------------------------------- | ------------------------------------------------------ |
| `#define X 0x20u`, `enum { X = 1 }` | `X :: 0x20;`                                           |
| string `#define`                    | `X :: "text";`                                         |
| `typedef Uint32 Keycode;`           | `type Keycode = Uint32;`                               |
| `struct` / `union`                  | `struct` / `union` (packed ones get `@pack()`)         |
| incomplete struct (`SDL_Window`)    | `struct Window { char _opaque; }`, used through `Window*` |
| function                            | `@attr(link_name("SDL_Init")) extern int Init(flags: Uint32);` |
| function pointer, `void*`           | `void*`                                                |
| `uint8_t` ... `int64_t`, `size_t`   | `unsigned char` ... `long`, `unsigned long`            |

Macros are folded by the C preprocessor itself, so function-like macros used in
other macros (`SDL_WINDOWPOS_CENTERED`) and `sizeof(void*)` work.

Naming: the C prefix is dropped (`SDL_CreateWindow` → `CreateWindow`), a name
that is a loi keyword gets a trailing `_` (`type` → `type_`), and a clash with an
earlier name appends `_`. Every function keeps its real C symbol in `link_name`.

### What is skipped

The generator never aborts on something it cannot express; it lists the
function in a `// Not generated` comment at the end of the file instead:
variadic functions, functions that take or return a struct *by value* (the loi
backend does not coerce those), types it cannot represent, and structs with
bit-fields (emitted opaque). Enums become plain `int` plus constants.

## Using a binding

```loi
@package(demo)
@inc std
@inc sdl2

main = (): int !IO
{
    sdl2.Init(sdl2.INIT_VIDEO);
    ev: sdl2.Event;                       // a union, 56 bytes like SDL_Event
    while (sdl2.PollEvent(&ev) != 0) {
        if (ev.type_ == sdl2.QUIT) { return 0; }
        k: sdl2.Keycode = ev.key.keysym.sym;
    }
    return 0;
}
```

- `sdl2.NAME` constants, `sdl2.Type` types and `sdl2.fn(...)` calls are
  checked; see *Qualified names* in [`syntax.md`](syntax.md).
- Unions overlay their members, and field access chains through values,
  pointers and array fields (see *Unions* and *Field access* there).
- The binding is a plain declarations file, so it costs nothing at run time.

`examples/sdl2_window.loi` wraps the generated API in effects and capabilities
(`!Gfx`, `Gfx` / `Window` / `Renderer`) so the compiler checks that every window
and renderer is destroyed exactly once and that SDL is not used after `Quit`.

## Writing a binding by hand

Anything the generator writes can be written by hand, and generated files are
meant to be edited. The minimum is:

```loi
@package(mylib)
@attr(link_name("mylib_add"))
extern int add(a: int, b: int);
```

## Limitations

- Struct-by-value parameters and returns are skipped (see above).
- Names live in one flat namespace; two packages cannot define the same
  top-level function name (a binding's names come from `link_name`, so a
  generated `Init` does not clash with the C symbol, but it would clash with a
  loi function also called `Init`).
- Fixed-width integer types map to loi's signed/unsigned primitives; loi checks
  `int` against `unsigned int` strictly except for literals and `::` constants.
