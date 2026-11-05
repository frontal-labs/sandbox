#!/bin/sh
# Runs inside the dev image as root. See build.sh.
set -eu
: "${WIZER_VERSION:=10.0.0}"
# Reproducible: clang takes __DATE__ and __TIME__ (in getbuildinfo.c) from this, so the build
# does not record when it ran.
export SOURCE_DATE_EPOCH=1700000000
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
# --without-mimalloc: mimalloc seeds its random cookies from the platform's randomness during
# initialisation, and Wizer would snapshot that seed, so no two builds would match.
CONFIG_SITE=../../Tools/wasm/config.site-wasm32-wasi \
  ../../Tools/wasm/wasi-env ../../configure -C \
    --host=wasm32-unknown-wasi --build="$(../../config.guess)" \
    --with-build-python="$SRC/builddir/build/python" --without-mimalloc >/tmp/configure-wasi.log 2>&1
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
# Reproducible: fixed modification times, sorted entries, no directory entries. Stored, not
# deflated: this interpreter has no zlib, so zipimport cannot inflate entries.
find "$STAGE" -exec touch -h -t 202001010000 {} +
(cd "$STAGE" && find . | LC_ALL=C sort | zip -q -0 -X -D -@ /out/python313.zip)
# 4. The reactor: the same objects, linked with a reactor entry (reactor/sandbox_reactor.c), so
#    Wizer can initialise the interpreter once and snapshot it. The command module above still
#    runs a script in one start; the snapshot does not pay that start on every run.
SYS=/opt/wasi-sdk/share/wasi-sysroot
CLANG="$WASI_SDK_PATH/bin/clang --target=wasm32-wasip1 --sysroot=$SYS"
cd "$SRC/builddir/wasi"
$CLANG -I. -I../../Include -I../../Include/internal -c /stage/reactor/sandbox_reactor.c -o /cache/reactor.o
MODS=$(grep "^MODOBJS=" Makefile | cut -d= -f2-)
OBJS=""
for o in $MODS; do [ -f "$o" ] && OBJS="$OBJS $o"; done
LIBS_A=$(find . -name "*.a" | grep -v "^./libpython" | tr "\n" " ")
$CLANG -o /cache/python-reactor.wasm "$SYS/lib/wasm32-wasip1/crt1-reactor.o" /cache/reactor.o \
  $OBJS libpython3.13.a $LIBS_A -ldl -lwasi-emulated-signal -lwasi-emulated-getpid \
  -lwasi-emulated-process-clocks -lm \
  -Wl,--export=sandbox_init -Wl,--export=sandbox_run -Wl,--export=sandbox_buffer
"$WASI_SDK_PATH/bin/llvm-strip" -o /cache/python-reactor.stripped.wasm /cache/python-reactor.wasm

# 5. Wizer, pinned by version, installed once into the cache volume. It runs sandbox_init
#    with the standard library mapped at /usr, and writes the initialised module.
WIZER=/cache/cargo/bin/wizer
if [ ! -x "$WIZER" ]; then
  cargo install --root /cache/cargo wizer --version "$WIZER_VERSION" --locked \
    --features="env_logger structopt" >/tmp/wizer-install.log 2>&1
fi
mkdir -p /cache/usr/lib /cache/work && cp /out/python313.zip /cache/usr/lib/python313.zip
# The snapshot keeps what the stdlib mapping reports about the zip, so its time is fixed as well.
touch -t 202001010000 /cache/usr/lib/python313.zip
# The preopens match what a run gets (src/wasi.rs): the work directory is fd 3, as "." and /usr
# follows it. wasi-libc reads the preopen table during initialisation, so the snapshot keeps it.
"$WIZER" --allow-wasi --mapdir .::/cache/work --mapdir /usr::/cache/usr --init-func sandbox_init \
  -o /out/python-init.wasm /cache/python-reactor.stripped.wasm

echo "python.wasm, python313.zip, and python-init.wasm are in tools/wasm/python/out"
