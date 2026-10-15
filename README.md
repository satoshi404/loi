# loi Lexer

Lexer for the **loi** programming language.

## Build

```sh
make
```

## Usage

```sh
./loi path/to/file.loi
# or from stdin
./loi < file.loi
```

Each line prints `line:column  TOKEN_TYPE  lexeme`.

## Files

- `src/token.h` / `src/token.c` — token definitions
- `src/lexer.h` / `src/lexer.c` — lexer implementation
- `src/main.c` — small CLI driver that prints tokens
