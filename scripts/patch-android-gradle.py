#!/usr/bin/env python3
"""Patch the Gradle project that `tauri android init` generates.

Done as a script rather than sed so a template change that moves a value makes
noise here instead of quietly shipping an APK that the device will not run.

The generated project is not in the repository, so anything Android needs beyond
Rust lives here: the Java sources under `src-tauri/android` are copied in, and the
ProGuard keeps they depend on are appended.
"""

import re
import shutil
import sys
from pathlib import Path

APP_DIR = Path(sys.argv[1] if len(sys.argv) > 1 else "src-tauri/gen/android/app")
TARGET_SDK = 28

KEEP_RULES = """
# Phi Recorder's render surface is reached only through JNI, where the name is the
# interface: quad_native.QuadNative's declarations have to keep matching miniquad's
# generated symbols, and Rust looks RenderSurface.start up by name.
-keep class quad_native.** { *; }
"""

PATCH = """
    // Phi Recorder execs the ffmpeg binary it ships as a JNI library. AGP's
    // default keeps .so files compressed inside the APK, so nothing is ever
    // unpacked to a path the app can exec - and at targetSdk >= 29 Android
    // refuses to exec from app-writable storage at all.
    packaging {
        jniLibs {
            useLegacyPackaging = true
        }
    }
    lint {
        // targetSdk 28 is the point, not an oversight: it is what lets the app
        // exec the bundled ffmpeg. Sideloading is the distribution channel.
        disable += "ExpiredTargetSdkVersion"
    }
    buildTypes {
        getByName("release") {
            // Debug key: installable CI artifact without secrets. Replace before publishing.
            signingConfig = signingConfigs.getByName("debug")
        }
    }
"""


def main() -> int:
    candidates = list(APP_DIR.glob("build.gradle.kts")) + list(APP_DIR.glob("build.gradle"))
    if not candidates:
        print(f"error: no Gradle build file under {APP_DIR}", file=sys.stderr)
        return 1
    gradle = candidates[0]
    text = gradle.read_text(encoding="utf-8")

    text, n = re.subn(r"targetSdk(Api)?\s*=\s*\d+", f"targetSdk = {TARGET_SDK}", text)
    if not n:
        print(f"error: no targetSdk assignment found in {gradle}", file=sys.stderr)
        return 1
    print(f"{gradle}: targetSdk set to {TARGET_SDK} ({n} site(s))")

    if "useLegacyPackaging" not in text:
        text, n = re.subn(r"\nandroid\s*\{", "\nandroid {" + PATCH, text, count=1)
        if not n:
            print(f"error: no `android {{` block in {gradle}", file=sys.stderr)
            return 1
        print(f"{gradle}: unpacked native libs + debug-signed release")

    gradle.write_text(text, encoding="utf-8")
    for key in ("targetSdk", "useLegacyPackaging", "ExpiredTargetSdkVersion", "signingConfigs.getByName"):
        print(f"  {key}: {'ok' if key in text else 'MISSING'}")

    return 0 if (copy_java_sources() and keep_jni_names()) else 1


def copy_java_sources() -> bool:
    """`src-tauri/android` mirrors the generated project's own tree, so this is a
    plain copy of app/src/main/java."""
    src = APP_DIR.parents[2] / "android" / "app" / "src" / "main" / "java"
    if not src.is_dir():
        print(f"error: no Java sources under {src}", file=sys.stderr)
        return False
    dest = APP_DIR / "src" / "main" / "java"
    shutil.copytree(src, dest, dirs_exist_ok=True)
    for file in sorted(dest.rglob("*.java")):
        print(f"{file.relative_to(dest)}: copied")
    return True


def keep_jni_names() -> bool:
    rules = APP_DIR / "proguard-rules.pro"
    if not rules.is_file():
        print(f"error: {rules} is missing, so the R8 keeps have nowhere to go", file=sys.stderr)
        return False
    text = rules.read_text(encoding="utf-8")
    if "quad_native" not in text:
        rules.write_text(text.rstrip() + "\n" + KEEP_RULES, encoding="utf-8")
    print(f"{rules}: R8 keeps for the JNI names")
    return True


if __name__ == "__main__":
    sys.exit(main())
