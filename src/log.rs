//! Optional line log (`--log <path>`), used to observe state changes.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static LOG: OnceLock<(Mutex<File>, Instant)> = OnceLock::new();

pub fn init(path: &Path) -> std::io::Result<()> {
    let file = File::create(path)?;
    let _ = LOG.set((Mutex::new(file), Instant::now()));
    Ok(())
}

pub fn line(msg: &str) {
    if let Some((file, start)) = LOG.get()
        && let Ok(mut f) = file.lock()
    {
        let _ = writeln!(f, "{:>8} {msg}", start.elapsed().as_millis());
        let _ = f.flush();
    }
}
