CC ?= gcc
CFLAGS ?= -O2 -Wall -Wextra -std=gnu99 -Isrc
LDFLAGS ?=

SRC = src/token.c src/lexer.c src/ast.c src/parser.c src/typecheck.c src/codegen.c src/preprocessor.c src/bindgen.c src/main.c
OBJ = $(SRC:.c=.o)
BIN = loi

all: $(BIN)

$(BIN): $(OBJ)
	$(CC) $(CFLAGS) -o $(BIN) $(OBJ) $(LDFLAGS)

%.o: %.c $(wildcard src/*.h)
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -f $(OBJ) $(BIN)

test: $(BIN)
	@bash tests/run.sh

.PHONY: all clean test
