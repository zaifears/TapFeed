//! Plain-text app log in `%LOCALAPPDATA%\TapFeed\logs\app.log`, passwords removed.

use std::io::Write;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());
const MAX_BYTES: u64 = 2 * 1024 * 1024;

pub fn write(line: &str) {
    let _g = LOCK.lock();
    let path = crate::runtime::logs_dir().join("app.log");
    if path.metadata().map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&path, path.with_extension("old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(f, "{now}  {}", crate::mpv::redact(line));
    }
}
