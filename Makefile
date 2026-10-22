CC ?= gcc
CFLAGS ?= -O2 -Wall -Wextra -std=gnu99 -Isrc -Wno-unused-function
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

PREFIX ?= /usr/local
BINDIR = $(PREFIX)/bin
SHAREDIR = $(PREFIX)/share/loi

install: $(BIN)
	install -d $(BINDIR) $(SHAREDIR)/lib $(SHAREDIR)/thirdparty
	install -m 755 $(BIN) $(BINDIR)/$(BIN)
	install -m 644 lib/*.loi $(SHAREDIR)/lib/
	install -m 644 src/runtime.c src/runtime.h $(SHAREDIR)/lib/
	cp -r thirdparty/. $(SHAREDIR)/thirdparty/

uninstall:
	rm -f $(BINDIR)/$(BIN)
	rm -rf $(SHAREDIR)

test: $(BIN)
	@bash tests/run.sh

.PHONY: all clean test
