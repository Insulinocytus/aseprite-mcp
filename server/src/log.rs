use parking_lot::Mutex;
use std::{fs::File, io::Write, path::Path, time::Instant};

/// Append-only log file. Without a path (tests) it discards everything.
pub struct Log {
    start: Instant,
    file: Option<Mutex<File>>,
}

impl Log {
    /// Creates/truncates the file, so callers must only do this once the port is bound
    /// (an overlapping old Server would otherwise interleave writes into the same file).
    pub fn create(path: Option<&Path>) -> std::io::Result<Self> {
        let file = path.map(File::create).transpose()?.map(Mutex::new);
        Ok(Self { start: Instant::now(), file })
    }

    pub fn line(&self, msg: impl AsRef<str>) {
        if let Some(file) = &self.file {
            let mut f = file.lock();
            let _ = writeln!(f, "[{:>8.3}] {}", self.start.elapsed().as_secs_f64(), msg.as_ref());
        }
    }
}
