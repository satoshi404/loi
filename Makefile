CC ?= gcc
CFLAGS ?= -O2 -Wall -Wextra -std=gnu99 -Isrc
LDFLAGS ?=

SRC = src/token.c src/lexer.c src/ast.c src/parser.c src/typecheck.c src/codegen.c src/main.c
OBJ = $(SRC:.c=.o)
BIN = loi

all: $(BIN)

$(BIN): $(OBJ)
	$(CC) $(CFLAGS) -o $(BIN) $(OBJ) $(LDFLAGS)

%.o: %.c
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -f $(OBJ) $(BIN)

.PHONY: all clean
