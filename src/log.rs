//! A small diary of what Riffly did with things that are hard to see —
//! where it saved the window's place, and whether that worked — kept in
//! `%LOCALAPPDATA%\Riffly\riffly.log` (elsewhere, beside the settings).
//!
//! It only ever holds a few hundred lines: past 256 KB it starts over.

use std::io::Write as _;
use std::path::PathBuf;

const LIMIT: u64 = 256 * 1024;

pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_STATE_HOME"))
        .map(PathBuf::from)
        .or_else(crate::workspace::default_config_dir)?;
    Some(base.join("Riffly").join("riffly.log"))
}

/// Append one line, stamped with the time.
pub fn write(message: impl AsRef<str>) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let too_big = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > LIMIT);
    let file = if too_big {
        std::fs::File::create(&path)
    } else {
        std::fs::OpenOptions::new().create(true).append(true).open(&path)
    };
    let Ok(mut file) = file else { return };
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(file, "[{seconds}] pid {} {}", std::process::id(), message.as_ref());
}
