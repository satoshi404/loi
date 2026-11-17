#!/usr/bin/env sh
# Compila um exemplo de raylib para a web (WebAssembly) com Emscripten.
#
# Pre-requisitos:
#   1) Emscripten ativo no shell (emcc no PATH, ex.: source emsdk_env.sh)
#   2) raylib compilada para web (NAO e a libraylib do sistema):
#        cd raylib-5.5/src && make PLATFORM=PLATFORM_WEB
#      e RAYLIB_WEB apontando para essa pasta src
#
# Uso (da raiz do repositorio):
#   RAYLIB_WEB=/caminho/raylib-5.5/src sh examples/build_web.sh examples/raylib_web.loi
#   python3 -m http.server -d web 8000   # abra http://localhost:8000
set -eu

SOURCE=${1:-examples/raylib_web.loi}
OUT_DIR=${OUT_DIR:-web}
: "${RAYLIB_WEB:?defina RAYLIB_WEB com a pasta src do raylib compilado para web}"

mkdir -p "$OUT_DIR"
NAME=$(basename "$SOURCE" .loi)

# CC=emcc faz o loi chamar o Emscripten no lugar do cc.
# -sASYNCIFY      deixa o 'while not WindowShouldClose()' bloqueante funcionar no navegador
# -sUSE_GLFW=3    janela/entrada pelo GLFW do Emscripten
# -sALLOW_MEMORY_GROWTH  evita estourar a memoria inicial
CC=emcc loi build "$SOURCE" -o "$OUT_DIR/$NAME.html" --release \
    --c-source vendor/raylib/raylib_shim.c \
    --cc-flag -Ivendor/raylib/include \
    --cc-flag "-L$RAYLIB_WEB" \
    --cc-flag -lraylib \
    --cc-flag -DPLATFORM_WEB \
    --cc-flag -sUSE_GLFW=3 \
    --cc-flag -sASYNCIFY \
    --cc-flag -sALLOW_MEMORY_GROWTH=1

echo "Pronto: $OUT_DIR/$NAME.html"
