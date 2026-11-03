#!/bin/sh
# Runs inside the dev image as root. See build.sh.
set -eu
mkdir -p /cache/src /out
cd /cache/src
if [ ! -d "Python-$PY_VERSION" ]; then
  curl -fsSL --proto '=https' --tlsv1.2 -o "Python-$PY_VERSION.tar.xz" \
    "https://www.python.org/ftp/python/$PY_VERSION/Python-$PY_VERSION.tar.xz"
  echo "$PY_SHA256  Python-$PY_VERSION.tar.xz" | sha256sum -c -
  tar -xJf "Python-$PY_VERSION.tar.xz"
fi
SRC=/cache/src/Python-$PY_VERSION

# 1. A native interpreter, which the cross build needs to run the code generators.
mkdir -p "$SRC/builddir/build" && cd "$SRC/builddir/build"
../../configure -C >/tmp/configure-build.log 2>&1
make -j3 python >/tmp/make-build.log 2>&1

# 2. The WASI build. wasi-env sets the SDK's compiler and archiver for configure.
cd "$SRC" && mkdir -p builddir/wasi && cd builddir/wasi
CONFIG_SITE=../../Tools/wasm/config.site-wasm32-wasi \
  ../../Tools/wasm/wasi-env ../../configure -C \
    --host=wasm32-unknown-wasi --build="$(../../config.guess)" \
    --with-build-python="$SRC/builddir/build/python" >/tmp/configure-wasi.log 2>&1
make -j3 python.wasm >/tmp/make-wasi.log 2>&1
"$WASI_SDK_PATH/bin/llvm-strip" -o /out/python.wasm python.wasm

# 3. The stdlib as lib/python313.zip, the archive CPython searches for under its home,
#    with the modules at the archive root. Without the test suite or bytecode caches.
STAGE=/cache/stage
rm -rf "$STAGE" && mkdir -p "$STAGE"
cp -a "$SRC/Lib/." "$STAGE/"
rm -rf "$STAGE/test"
find "$STAGE" -name __pycache__ -type d -prune -exec rm -rf {} +
rm -f /out/python313.zip
# Stored, not deflated: this interpreter has no zlib, so zipimport cannot inflate entries.
(cd "$STAGE" && zip -q -r -0 -X /out/python313.zip .)
echo "python.wasm and python313.zip are in tools/wasm/python/out"
