package quad_native;

import android.view.Surface;

/**
 * JNI bridge to miniquad's Android backend, which is linked into
 * libphi_recorder_lib.so. Each declaration has to match a
 * `Java_quad_1native_QuadNative_*` symbol there, so only the entry points this
 * app actually calls are declared: a native method that is never called is
 * never resolved.
 */
public final class QuadNative {
    static {
        // Tauri has already loaded the library to start Rust; loading it again
        // from the same class loader is a no-op, and this keeps the order here
        // independent of how Tauri decides to boot.
        System.loadLibrary("phi_recorder_lib");
    }

    private QuadNative() {}

    /** Puts the JavaVM and this Activity into ndk_context. It asserts on a second
     *  call, so RenderSurface runs it once per process. */
    public static native void initializeContext(Object activity);

    /** Calls `quad_main` in Rust, which starts the render loop on its own thread. */
    public static native void activityOnCreate(Object activity);

    public static native void activityOnDestroy();

    public static native void surfaceOnSurfaceCreated(Surface surface);

    public static native void surfaceOnSurfaceDestroyed(Surface surface);

    /** The reported size becomes miniquad's screen size, so it is the video size. */
    public static native void surfaceOnSurfaceChanged(Surface surface, int width, int height);
}
