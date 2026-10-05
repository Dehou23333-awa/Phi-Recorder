package quad_native;

import android.app.Activity;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.widget.FrameLayout;
import java.io.File;
import java.io.FileInputStream;
import java.nio.charset.StandardCharsets;

/**
 * The surface Phi Recorder renders on.
 *
 * miniquad builds its GL context out of an Activity surface and nothing else, so
 * the render cannot run as a child process. Rather than a second Activity, which
 * would mean editing a manifest in a project that `tauri android init` regenerates
 * on every build, this lays a SurfaceView over the WebView inside the Activity the
 * app already has.
 *
 * The view is sized to the video resolution in device pixels because that size is
 * what miniquad reports as the screen size afterwards, and the chart layout is
 * derived from it - the same way it is on desktop, where the render child simply
 * opens a window that big.
 */
public final class RenderSurface {
    private static final String TAG = "PhiRecorderRender";
    private static final String EVENTS = "render/events.log";
    private static final String ERROR = "render/error.txt";
    private static final long POLL = 400;

    private static Activity host;
    private static SurfaceView view;
    private static Handler handler;
    private static boolean contextReady;

    private RenderSurface() {}

    /** Called from Rust through JNI, on the Android main thread. */
    public static void start(Activity activity, int width, int height) {
        if (view != null) {
            // The task that owned the previous render already saw its Done event,
            // but this poll runs on its own clock, so the old overlay can still be
            // up when the next task starts. Leaving it would refuse the new render
            // and the new task would wait forever for events.
            Log.i(TAG, "releasing the surface the previous render left");
            detach();
        }
        host = activity;
        view = new QuadSurface(activity);
        FrameLayout content = (FrameLayout) activity.findViewById(android.R.id.content);
        content.addView(view, new FrameLayout.LayoutParams(width, height));

        // Handing over has to happen before the surface callbacks fire, because
        // miniquad drops messages sent before it installed its channel. Adding the
        // view only schedules those callbacks for the next layout pass, so this
        // still runs first.
        if (!contextReady) {
            QuadNative.initializeContext(activity);
            contextReady = true;
        }
        QuadNative.activityOnCreate(activity);

        handler = new Handler(Looper.getMainLooper());
        handler.postDelayed(poll, POLL);
    }

    /**
     * The render is over when the event log reaches Done or the failure file
     * appears; either way miniquad's loop has to be told to end before the
     * surface goes away, or its thread keeps spinning with a dead context.
     */
    private static final Runnable poll = new Runnable() {
        @Override
        public void run() {
            if (view == null) {
                return;
            }
            String events = read(EVENTS);
            if ((events != null && events.contains("\"Done\"")) || read(ERROR) != null) {
                detach();
                return;
            }
            handler.postDelayed(this, POLL);
        }
    };

    private static void detach() {
        Log.i(TAG, "render finished, releasing the surface");
        QuadNative.activityOnDestroy();
        FrameLayout content = (FrameLayout) host.findViewById(android.R.id.content);
        content.removeView(view);
        view = null;
        handler.removeCallbacksAndMessages(null);
        handler = null;
        host = null;
    }

    private static String read(String relative) {
        try {
            File file = new File(host.getCacheDir(), relative);
            if (!file.isFile()) {
                return null;
            }
            byte[] data = new byte[(int) file.length()];
            int offset = 0;
            try (FileInputStream in = new FileInputStream(file)) {
                while (offset < data.length) {
                    int count = in.read(data, offset, data.length - offset);
                    if (count <= 0) {
                        break;
                    }
                    offset += count;
                }
            }
            return new String(data, 0, offset, StandardCharsets.UTF_8);
        } catch (Exception e) {
            return null;
        }
    }

    private static final class QuadSurface extends SurfaceView implements SurfaceHolder.Callback {

        QuadSurface(Activity activity) {
            super(activity);
            // A SurfaceView normally composes behind its window, which the opaque
            // WebView would cover. Visible progress is not needed to render, but it
            // is what tells the user the phone is working rather than stuck.
            setZOrderMediaOverlay(true);
            getHolder().addCallback(this);
        }

        @Override
        public void surfaceCreated(SurfaceHolder holder) {
            QuadNative.surfaceOnSurfaceCreated(holder.getSurface());
        }

        @Override
        public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
            Log.i(TAG, "surface " + width + "x" + height);
            QuadNative.surfaceOnSurfaceChanged(holder.getSurface(), width, height);
        }

        @Override
        public void surfaceDestroyed(SurfaceHolder holder) {
            QuadNative.surfaceOnSurfaceDestroyed(holder.getSurface());
        }
    }
}
