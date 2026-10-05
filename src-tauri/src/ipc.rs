#[cfg(target_os = "android")]
use anyhow::Context;
#[cfg(not(target_os = "android"))]
use anyhow::bail;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "android")]
use std::time::Duration;

#[derive(Serialize, Deserialize)]
pub enum IPCEvent {
    Loading,
    Mixing,
    MixingSfx(u64),
    Sfx(u64),
    RenderFrame(u64),
    Frame(u64),
    Paused,
    Resumed,
    Done(f64),
}

pub mod client {
    use super::*;

    pub fn send<T: Serialize>(value: T) {
        #[cfg(target_os = "android")]
        super::local::publish(&value);

        #[cfg(not(target_os = "android"))]
        println!("{}", serde_json::to_string(&value).unwrap());
    }
}

/// The render loop speaks one protocol everywhere: two JSON request lines in
/// (`RenderParams`, then the output path), `IPCEvent`s out, and `pause` /
/// `resume` / `cancel` commands in while it runs.
///
/// Desktop runs the render as a child process and uses that process's
/// stdin/stdout. Android renders inside the app process (miniquad can only build
/// a GL context on an Activity surface, so there is no child to pipe to) and the
/// same messages travel through files under the cache dir.
pub mod server {
    use super::*;

    #[cfg(target_os = "android")]
    static RUNNING: AtomicBool = AtomicBool::new(false);
    static CANCELED: AtomicBool = AtomicBool::new(false);

    /// Called by whoever starts the render, before the first request line exists.
    #[cfg(target_os = "android")]
    pub fn begin() {
        CANCELED.store(false, Ordering::SeqCst);
        RUNNING.store(true, Ordering::SeqCst);
    }

    /// Lets the command reader stop polling instead of waiting forever.
    #[cfg(target_os = "android")]
    pub fn finish() {
        RUNNING.store(false, Ordering::SeqCst);
    }

    pub fn canceled() -> bool {
        CANCELED.load(Ordering::SeqCst)
    }

    /// `index` 0 carries the JSON `RenderParams`, 1 the JSON output path.
    #[cfg(target_os = "android")]
    pub fn request_line(index: usize) -> Result<String> {
        local::request_line(index)
    }

    #[cfg(not(target_os = "android"))]
    pub fn request_line(index: usize) -> Result<String> {
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line)? == 0 {
            bail!("the render request stopped before line {}", index + 1);
        }
        Ok(line)
    }

    /// A command sent after the render started. `Ok(None)` means the source is
    /// gone: stdin closed, or this render loop has ended.
    pub struct Commands {
        #[cfg(target_os = "android")]
        last: Option<String>,
    }

    impl Commands {
        pub fn new() -> Commands {
            Commands {
                #[cfg(target_os = "android")]
                last: None,
            }
        }

        #[cfg(not(target_os = "android"))]
        pub fn next(&mut self) -> Result<Option<String>> {
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(0) | Err(_) => Ok(None),
                Ok(_) => Ok(Some(line)),
            }
        }

        #[cfg(target_os = "android")]
        pub fn next(&mut self) -> Result<Option<String>> {
            loop {
                if !RUNNING.load(Ordering::SeqCst) {
                    return Ok(None);
                }
                if let Some(command) = local::read("command.txt") {
                    let command = command.trim().to_owned();
                    if !command.is_empty() && Some(&command) != self.last.as_ref() {
                        self.last = Some(command.clone());
                        if command == "cancel" {
                            CANCELED.store(true, Ordering::SeqCst);
                        }
                        return Ok(Some(command));
                    }
                }
                std::thread::sleep(Duration::from_millis(120));
            }
        }
    }

    /// How a render that cannot report an exit status still tells anyone. On
    /// desktop the exit code and stderr already do that.
    #[cfg(target_os = "android")]
    pub fn report_failure(err: &anyhow::Error) {
        local::report_failure(err);
    }
}

#[cfg(target_os = "android")]
pub mod local {
    use super::*;
    use std::io::{BufRead, Seek, SeekFrom, Write};
    use std::path::PathBuf;

    const EVENTS: &str = "events.log";
    const ERROR: &str = "error.txt";

    fn dir() -> Result<&'static PathBuf> {
        crate::common::RENDER_DIR.get().context("the render directory is not set up")
    }

    /// Both readers take the file as it stands, so a partially written message
    /// would be read as a truncated one; renaming publishes it whole.
    fn replace(name: &str, contents: &str) -> Result<()> {
        let dir = dir()?;
        let tmp = dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, contents)?;
        std::fs::rename(&tmp, dir.join(name))?;
        Ok(())
    }

    pub fn read(name: &str) -> Option<String> {
        std::fs::read_to_string(dir().ok()?.join(name)).ok()
    }

    /// Events are appended instead of overwritten so that a poller slower than the
    /// renderer still sees every one of them.
    pub fn publish<T: Serialize>(value: &T) {
        let json = serde_json::to_string(value).unwrap();
        if let Ok(path) = dir().map(|dir| dir.join(EVENTS)) {
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = file.write_all(format!("{json}\n").as_bytes());
            }
        }
    }

    /// Consumes whole lines from `read` on; a line still being written is left for
    /// the next call.
    pub fn events(read: &mut u64) -> Result<Vec<IPCEvent>> {
        let mut file = match std::fs::File::open(dir()?.join(EVENTS)) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err).context("cannot read the render events"),
        };
        file.seek(SeekFrom::Start(*read))?;
        let mut reader = std::io::BufReader::new(file);
        let mut found = Vec::new();
        loop {
            let mut line = String::new();
            let count = reader.read_line(&mut line)?;
            if count == 0 || !line.ends_with('\n') {
                break;
            }
            *read += count as u64;
            if let Ok(event) = serde_json::from_str::<IPCEvent>(line.trim()) {
                found.push(event);
            }
        }
        Ok(found)
    }

    pub fn request_line(index: usize) -> Result<String> {
        read(&format!("request-{index}.json")).with_context(|| format!("render request line {index} is missing"))
    }

    pub fn report_failure(err: &anyhow::Error) {
        let _ = replace(ERROR, &format!("{err:?}"));
    }

    /// The task clears the previous render's messages before starting a new one.
    pub fn clear() -> Result<()> {
        let dir = dir()?;
        for name in [EVENTS, ERROR, "command.txt"] {
            match std::fs::remove_file(dir.join(name)) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err).with_context(|| format!("cannot remove {name}")),
            }
        }
        Ok(())
    }

    pub fn write_request(index: usize, contents: &str) -> Result<()> {
        replace(&format!("request-{index}.json"), contents)
    }

    pub fn write_command(command: &str) -> Result<()> {
        replace("command.txt", command)
    }

    pub fn failure() -> Option<String> {
        read(ERROR)
    }
}
