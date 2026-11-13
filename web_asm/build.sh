#!/usr/bin/env sh
# Compila examples/raylib_window.loi para a web (WebAssembly) com Emscripten.
#
# Pre-requisitos:
#   1) Emscripten ativo no shell (emcc no PATH, ex.: source emsdk_env.sh)
#   2) libraylib.a compilada para web (NAO e a libraylib do sistema):
#        cd raylib-5.5/src && make PLATFORM=PLATFORM_WEB
#      e RAYLIB_WEB apontando para essa pasta src
#
# Uso (da raiz do repositorio):
#   RAYLIB_WEB=/caminho/raylib-5.5/src sh examples/build_web.sh
#   python3 -m http.server -d web 8000   # abra http://localhost:8000
set -eu

: "${RAYLIB_WEB:?defina RAYLIB_WEB com a pasta src do raylib compilado para web}"
# mkdir -p web

# CC=emcc faz o loi chamar o Emscripten no lugar do cc.
# -sASYNCIFY      deixa o 'while not WindowShouldClose()' bloqueante funcionar no navegador
# -sUSE_GLFW=3    janela/entrada pelo GLFW do Emscripten
# -sALLOW_MEMORY_GROWTH  evita estourar a memoria inicial
CC=emcc loi build ../examples/raylib_window.loi -o index.html --release \
    --c-source ../vendor/raylib/raylib_shim.c \
    --cc-flag -I../vendor/raylib/include \
    --cc-flag "-L$RAYLIB_WEB" \
    --cc-flag -lraylib \
    --cc-flag -DPLATFORM_WEB \
    --cc-flag -sUSE_GLFW=3 \
    --cc-flag -sASYNCIFY \
    --cc-flag -sALLOW_MEMORY_GROWTH=1
