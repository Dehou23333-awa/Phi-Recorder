#!/usr/bin/env python3
"""Patch the Gradle project that `tauri android init` generates.

Done as a script rather than sed so a template change that moves a value makes
noise here instead of quietly shipping an APK that the device will not run.
"""

import re
import sys
from pathlib import Path

APP_DIR = Path(sys.argv[1] if len(sys.argv) > 1 else "src-tauri/gen/android/app")
TARGET_SDK = 28

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
    for key in ("targetSdk", "useLegacyPackaging", "signingConfigs.getByName"):
        print(f"  {key}: {'ok' if key in text else 'MISSING'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
