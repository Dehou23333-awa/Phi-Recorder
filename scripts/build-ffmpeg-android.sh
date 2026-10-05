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

# x264 and ffmpeg both derive their binutils from a `<triple>-` prefix, while the
# NDK only ships `llvm-*` tools plus version-suffixed clang wrappers. Handing
# them the names they ask for beats teaching every recipe the NDK's layout.
CROSS_BIN="$PREFIX/cross-bin"
mkdir -p "$CROSS_BIN"
for tool in ar nm objcopy objdump ranlib readelf strip; do
	ln -sf "$TC/bin/llvm-$tool" "$CROSS_BIN/$TRIPLE-$tool"
done
ln -sf "$TC/bin/$TRIPLE$API-clang" "$CROSS_BIN/$TRIPLE-clang"
ln -sf "$TC/bin/$TRIPLE$API-clang++" "$CROSS_BIN/$TRIPLE-clang++"
# x264 asks for <triple>-gcc rather than <triple>-clang.
ln -sf "$TC/bin/$TRIPLE$API-clang" "$CROSS_BIN/$TRIPLE-gcc"
ln -sf "$TC/bin/$TRIPLE$API-clang++" "$CROSS_BIN/$TRIPLE-g++"
export PATH="$CROSS_BIN:$PATH"
# configure scripts take the compiler from CC, not from a --cc option (x264 has
# no such option and just warns it away).
export CC="$TRIPLE-clang" CXX="$TRIPLE-clang++"
# Resolve the compiler here, where the shell reports what is actually wrong,
# rather than letting x264 reduce it to "No working C compiler found."
"$CC" --version >/dev/null

# libx264 is the software encoder the app picks when hardware encoding is off,
# so it is not optional for a usable build. Built static + PIC so ffmpeg can
# absorb it into one executable.
if [ ! -e "$PREFIX/lib/libx264.a" ]; then
	git clone --depth 1 --branch "$X264_REF" https://code.videolan.org/videolan/x264.git "$SRC_DIR/x264"
	(
		cd "$SRC_DIR/x264"
		./configure \
			--host="$TRIPLE" \
			--cross-prefix="$TRIPLE-" \
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
		--cross-prefix="$TRIPLE-" \
		--cc="$TRIPLE-clang" --cxx="$TRIPLE-clang++" \
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
"$TRIPLE-readelf" -h "$PREFIX/bin/ffmpeg" | tee /dev/stderr | grep -q 'Type: *DYN' || {
	echo "ffmpeg is not a dynamic (PIE) executable" >&2
	exit 1
}
"$TRIPLE-strip" --strip-unneeded "$PREFIX/bin/ffmpeg" || true
ls -lh "$PREFIX/bin/ffmpeg"
