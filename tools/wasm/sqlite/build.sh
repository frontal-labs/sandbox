#!/bin/sh
# Builds `sqlite3` as a WASI command, with the SQLite amalgamation that libsqlite3-sys
# bundles. Run it in the Linux dev container, which has clang and the WASI sysroot:
#
#   docker/run-linux.sh tools/wasm/sqlite/build.sh
#
# The output is tools/wasm/sqlite/build/sqlite3_tool.wasm. Install it with
# tools/wasm/build.sh, which copies it to the fixtures and prints its hash.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
OUT="$HERE/build"
SYSROOT=${WASI_SYSROOT:-/usr}
CC=${CC:-clang}

# The amalgamation comes from the crate that bundles it, so the version is pinned by
# Cargo.lock (libsqlite3-sys 0.37.0, SQLite 3.51.3).
PIN=$(mktemp -d)
trap 'rm -rf "$PIN"' EXIT
cat > "$PIN/Cargo.toml" <<TOML
[package]
name = "sqlite-source-pin"
version = "0.0.0"
edition = "2021"
[dependencies]
libsqlite3-sys = "=0.37.0"
TOML
mkdir -p "$PIN/src" && : > "$PIN/src/lib.rs"
(cd "$PIN" && cargo fetch -q)
SRC=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/libsqlite3-sys-0.37.0/sqlite3)

mkdir -p "$OUT"
DEFS="-DSQLITE_OS_OTHER=1 -DSQLITE_TEMP_STORE=3 -DSQLITE_THREADSAFE=0 -DSQLITE_OMIT_LOAD_EXTENSION -DSQLITE_OMIT_WAL -DSQLITE_OMIT_SHARED_CACHE"
"$CC" --target=wasm32-wasi --sysroot="$SYSROOT" -O2 $DEFS \
  -I"$SRC" -c "$SRC/sqlite3.c" -o "$OUT/sqlite3.o"
"$CC" --target=wasm32-wasi --sysroot="$SYSROOT" -O2 $DEFS \
  -I"$SRC" -c "$SRC/wasm32-wasi-vfs.c" -o "$OUT/vfs.o"
"$CC" --target=wasm32-wasi --sysroot="$SYSROOT" -O2 $DEFS \
  -I"$SRC" -c "$HERE/sqlite3_tool.c" -o "$OUT/tool.o"
"$CC" --target=wasm32-wasi --sysroot="$SYSROOT" -o "$OUT/sqlite3_tool.wasm" \
  "$OUT/tool.o" "$OUT/sqlite3.o" "$OUT/vfs.o"
echo "built $OUT/sqlite3_tool.wasm (SQLite from $SRC)"
