use anyhow::{bail, Context, Result};
use jni::objects::{JObject, JValue};
use jni::JNIEnv;
use std::io::Read;
use std::mem::ManuallyDrop;
use std::os::fd::FromRawFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

// The copy runs on the Android main thread, which may be busy laying out the
// WebView, so the wait is generous.
const TIMEOUT: Duration = Duration::from_secs(120);

/// tauri-plugin-dialog hands the frontend the raw `content://` string on Android
/// (`DialogPlugin.kt::createPickFilesResult` builds it with `uri.toString()`),
/// and no filesystem call can open that, so the bytes are copied into `dir` and
/// the real path is handed back.
pub fn materialize(dir: &Path, uri: &str) -> Result<PathBuf> {
    let (tx, rx) = mpsc::channel();
    let dir = dir.to_path_buf();
    let owned = uri.to_owned();
    wry::prelude::dispatch(move |env, activity, _| {
        let _ = tx.send(copy(env, activity, &owned, &dir));
    });
    rx.recv_timeout(TIMEOUT)
        .context("the Android main thread did not answer")?
}

fn copy(env: &mut JNIEnv, activity: &JObject, uri: &str, dir: &Path) -> Result<PathBuf> {
    let uri_arg = env.new_string(uri)?;
    let parsed = env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&uri_arg)],
        )?
        .l()?;
    let mode = env.new_string("r")?;
    let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
    let stream = env
        .call_method(
            &resolver,
            "openFileDescriptor",
            "(Landroid/net/Uri;Ljava/lang/String;)Landroid/os/ParcelFileDescriptor;",
            &[JValue::Object(&parsed), JValue::Object(&mode)],
        )?
        .l()?;
    if stream.is_null() {
        bail!("openFileDescriptor refused {uri}");
    }
    let fd = env.call_method(&stream, "getFd", "()I", &[])?.i()?;

    let mut data = Vec::new();
    // The fd belongs to the Java ParcelFileDescriptor, so this must never run
    // File's destructor: ManuallyDrop keeps it alive for the read, and Java
    // closes it below.
    let mut file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
    file.read_to_end(&mut data)?;
    env.call_method(&stream, "close", "()V", &[])?;

    std::fs::create_dir_all(dir)?;
    let name = uri.rsplit('/').next().unwrap_or_default();
    let name = if name.is_empty() { "picked" } else { name };
    let path = dir.join(name.replace(|c| !matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-'), "_"));
    std::fs::write(&path, &data).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}
