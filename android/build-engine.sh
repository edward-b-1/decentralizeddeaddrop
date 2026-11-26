#!/bin/sh
# Builds the Rust engine for Android and generates its Kotlin bindings.
# Run this before the first Gradle build, and again whenever the Rust code changes.
#
#   ANDROID_NDK_HOME must point at the NDK (see the README at the top of the repo).
#   Pass "debug" for a faster, larger build:  ./build-engine.sh debug
set -e
cd "$(dirname "$0")/.."

profile="${1:-release}"
flag="--release"
[ "$profile" = "debug" ] && flag=""

cargo ndk -t arm64-v8a -t x86_64 --platform 31 -o android/app/src/main/jniLibs build $flag -p deaddrop-ffi --lib

# The bindings are read out of a library built for this computer.
cargo build -p deaddrop-ffi --lib
host_library="$(ls target/debug/libdeaddrop_ffi.so target/debug/libdeaddrop_ffi.dylib target/debug/deaddrop_ffi.dll 2>/dev/null | head -n 1)"
rm -rf android/app/src/main/java/com/decentralizeddeaddrop/engine
cargo run -q -p deaddrop-ffi --bin uniffi-bindgen -- generate --library "$host_library" \
    --language kotlin --no-format --out-dir android/app/src/main/java
echo "Engine built ($profile) and bindings generated."
