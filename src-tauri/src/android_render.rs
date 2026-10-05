use anyhow::{Context, Result};
use jni::objects::{JObject, JValue};
use jni::JNIEnv;
use std::sync::mpsc;
use std::time::Duration;

// Attaching happens on the Android main thread, which may be busy laying out the
// WebView, so the wait is generous.
const TIMEOUT: Duration = Duration::from_secs(120);

/// The overlay class lives in the app's own dex, so it has to be looked up
/// through the Activity's class loader: `FindClass` on a JNIEnv that was not
/// entered from a Java frame only knows the boot class loader, and the
/// `NoClassDefFoundError` it throws would otherwise be rethrown at the main
/// thread's next Java boundary and take the process down.
const RENDER_SURFACE: &str = "quad_native.RenderSurface";

/// miniquad builds its GL context on an Activity surface and nothing else, so the
/// render cannot run as a child process here. This puts a SurfaceView over the
/// WebView; the surface callback is what eventually calls `quad_main`.
pub fn start(resolution: (u32, u32)) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let (width, height) = resolution;
    wry::prelude::dispatch(move |env, activity, _| {
        let _ = tx.send(attach(env, activity, width, height));
    });
    rx.recv_timeout(TIMEOUT)
        .context("the Android main thread did not answer")?
}

/// The overlay is sized in device pixels to the video resolution, because that
/// size is what miniquad reports as the screen size, and the chart layout is
/// derived from it.
fn attach(env: &mut JNIEnv, activity: &JObject, width: u32, height: u32) -> Result<()> {
    let result = attach_surface(env, activity, width, height);
    if result.is_err() {
        let _ = env.exception_clear();
    }
    result
}

fn attach_surface(env: &mut JNIEnv, activity: &JObject, width: u32, height: u32) -> Result<()> {
    let loader = env
        .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()
        .context("the Activity has no ClassLoader")?;
    let name = env.new_string(RENDER_SURFACE)?;
    let surface = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&name)],
        )
        .context(RENDER_SURFACE)?
        .l()?;
    let surface = env.new_global_ref(surface)?;

    env.call_static_method(
        &surface,
        "start",
        "(Landroid/app/Activity;II)V",
        &[JValue::Object(activity), JValue::Int(width as i32), JValue::Int(height as i32)],
    )
    .context("the render surface could not be attached")?;
    Ok(())
}
