#!/usr/bin/env bash
# Cross-compile an `ffmpeg` executable for Android arm64.
#
# The resulting binary ships inside the APK as a JNI library, which is the only
# app-owned location Android lets a process exec from, and it is what Phi
# Recorder drives for the final encode.
set -Eeuo pipefail

# configure scripts put the actual failure in config.log and only print a
# one-liner to stdout, which makes a CI log useless on its own.
trap 'tail -40 "$(pwd)/config.log" 2>/dev/null || true' ERR

: "${ANDROID_NDK_HOME:?ANDROID_NDK_HOME must point at an NDK (r26 or newer)}"

API="${API:-24}"
TRIPLE=aarch64-linux-android
FFMPEG_REF="${FFMPEG_REF:-n7.1}"
X264_REF="${X264_REF:-stable}"
PREFIX="${PREFIX:-$HOME/ffmpeg-android}"
SRC_DIR="${SRC_DIR:-$HOME/.ffmpeg-src}"

case "$(uname -s)" in
	Linux) HOST_TAG=linux-x86_64 ;;
	Darwin) HOST_TAG=darwin-x86_64 ;;
	*)
		echo "unsupported host: $(uname -s)" >&2
		exit 1
		;;
esac

TC="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$HOST_TAG"
SYSROOT="$TC/sysroot"
JOBS="${JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu)}"

if [ ! -d "$TC" ]; then
	echo "$TC does not exist - is ANDROID_NDK_HOME ($ANDROID_NDK_HOME) an actual NDK?" >&2
	exit 1
fi

mkdir -p "$PREFIX" "$SRC_DIR"

if [ -x "$PREFIX/bin/ffmpeg" ] && [ "${FORCE_REBUILD:-0}" != 1 ]; then
	echo "reusing $PREFIX/bin/ffmpeg (FORCE_REBUILD=1 to rebuild)"
	exit 0
fi

# x264 and ffmpeg derive their binutils from a `<triple>-` prefix, a name the NDK
# does not ship. Do NOT fake that with symlinks: `aarch64-linux-androidNN-clang`
# is a shell wrapper that execs the `clang` sitting next to the path it was
# *called through*, so invoking it via a link elsewhere dies with
# ".../cross-bin/clang: No such file or directory". Give every tool its absolute
# path instead; both configure scripts take theirs from the environment.
TC_BIN="$TC/bin"
CC="$TC_BIN/$TRIPLE$API-clang"
CXX="$TC_BIN/$TRIPLE$API-clang++"
AR="$TC_BIN/llvm-ar"
RANLIB="$TC_BIN/llvm-ranlib"
STRIP="$TC_BIN/llvm-strip"
NM="$TC_BIN/llvm-nm"
OBJCOPY="$TC_BIN/llvm-objcopy"
export CC CXX AR RANLIB STRIP NM OBJCOPY

for tool in "$CC" "$AR" "$RANLIB" "$STRIP" "$NM" "$OBJCOPY"; do
	[ -x "$tool" ] || { echo "$tool is missing or not executable - did the NDK layout change?" >&2; exit 1; }
done
"$CC" --version | head -2

# libx264 is the software encoder the app picks when hardware encoding is off,
# so it is not optional for a usable build. Built static + PIC so ffmpeg can
# absorb it into one executable.
if [ ! -e "$PREFIX/lib/libx264.a" ]; then
	git clone --depth 1 --branch "$X264_REF" https://code.videolan.org/videolan/x264.git "$SRC_DIR/x264"
	(
		cd "$SRC_DIR/x264"
		./configure \
			--host="$TRIPLE" \
			--sysroot="$SYSROOT" \
			--prefix="$PREFIX" \
			--enable-static --enable-pic \
			--disable-cli --disable-opencl \
			--extra-cflags="-O3"
		make -j"$JOBS"
		make install
	)
fi

git clone --depth 1 --branch "$FFMPEG_REF" https://github.com/FFmpeg/FFmpeg.git "$SRC_DIR/ffmpeg"
(
	cd "$SRC_DIR/ffmpeg"
	# mediacodec + jni give the device's hardware encoders, which is what an
	# Android encode should use; loudnorm/amix are native filters, so no
	# further external libraries are needed.
	PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig" ./configure \
		--target-os=android \
		--arch=aarch64 --cpu=armv8-a \
		--cc="$CC" --cxx="$CXX" \
		--ar="$AR" --ranlib="$RANLIB" --strip="$STRIP" \
		--nm="$NM" --objcopy="$OBJCOPY" --objdump="$TC_BIN/llvm-objdump" \
		--sysroot="$SYSROOT" \
		--prefix="$PREFIX" \
		--enable-cross-compile \
		--pkg-config="$(command -v pkg-config)" \
		--pkg-config-flags=--static \
		--enable-static --disable-shared --enable-pic \
		--disable-doc --disable-debug \
		--enable-gpl --enable-libx264 \
		--enable-jni --enable-mediacodec --enable-neon \
		--extra-cflags="-O3 -fPIC -I$PREFIX/include" \
		--extra-ldflags="-L$PREFIX/lib -Wl,-z,max-page-size=16384"
	make -j"$JOBS"
	make install
)

# The exec shim runs the binary through /system/bin/linker64, which loads
# dynamic ELF - a static build would be silently unusable on device.
"$TC_BIN/llvm-readelf" -h "$PREFIX/bin/ffmpeg" | tee /dev/stderr | grep -q 'Type: *DYN' || {
	echo "ffmpeg is not a dynamic (PIE) executable" >&2
	exit 1
}
"$STRIP" --strip-unneeded "$PREFIX/bin/ffmpeg" || true
ls -lh "$PREFIX/bin/ffmpeg"
