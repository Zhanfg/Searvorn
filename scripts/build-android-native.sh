#!/bin/sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
TOOLCHAIN="${SEARVORN_RUST_TOOLCHAIN:-1.99.0}"
TARGET="aarch64-linux-android"
MIN_SDK="${SEARVORN_ANDROID_MIN_SDK:-26}"
NDK_VERSION="${SEARVORN_NDK_VERSION:-29.0.14206865}"
OUT="${SEARVORN_ANDROID_OUT:-$ROOT/android/app/build/generated/jniLibs}"

find_ndk() {
    if [ -n "${ANDROID_NDK_HOME:-}" ] && [ -d "$ANDROID_NDK_HOME" ]; then
        printf '%s\n' "$ANDROID_NDK_HOME"
        return 0
    fi

    for sdk in "${ANDROID_SDK_ROOT:-}" "${ANDROID_HOME:-}"; do
        if [ -n "$sdk" ] && [ -d "$sdk/ndk/$NDK_VERSION" ]; then
            printf '%s\n' "$sdk/ndk/$NDK_VERSION"
            return 0
        fi
    done

    return 1
}

NDK_ROOT="$(find_ndk || true)"
if [ -z "$NDK_ROOT" ]; then
    echo "Searvorn: Android NDK $NDK_VERSION not found." >&2
    echo "Set ANDROID_NDK_HOME or install the pinned NDK in ANDROID_SDK_ROOT/ndk/$NDK_VERSION." >&2
    exit 2
fi

PREBUILT=""
for candidate in "$NDK_ROOT"/toolchains/llvm/prebuilt/*; do
    if [ -d "$candidate/bin" ]; then
        PREBUILT="$candidate"
        break
    fi
done

if [ -z "$PREBUILT" ]; then
    echo "Searvorn: NDK LLVM prebuilt toolchain not found." >&2
    exit 2
fi

LINKER="$PREBUILT/bin/aarch64-linux-android${MIN_SDK}-clang"
STRIP="$PREBUILT/bin/llvm-strip"

if [ ! -x "$LINKER" ]; then
    echo "Searvorn: linker not found: $LINKER" >&2
    exit 2
fi

rustup target add "$TARGET" --toolchain "$TOOLCHAIN" >/dev/null
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$LINKER"

cargo +"$TOOLCHAIN" build \
    --manifest-path "$ROOT/Cargo.toml" \
    --package searvorn-core \
    --release \
    --target "$TARGET"

mkdir -p "$OUT/arm64-v8a"
cp "$ROOT/target/$TARGET/release/libsearvorn_core.so" "$OUT/arm64-v8a/libsearvorn_core.so"

if [ -x "$STRIP" ]; then
    "$STRIP" --strip-unneeded "$OUT/arm64-v8a/libsearvorn_core.so"
fi

printf 'Searvorn native: %s\n' "$OUT/arm64-v8a/libsearvorn_core.so"
