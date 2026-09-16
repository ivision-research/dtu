use std::{
    fs::File,
    io::{BufWriter, Write},
    sync::Mutex,
};

pub struct FileLogger {
    file: Mutex<BufWriter<File>>,
    level: log::Level,
}

impl Drop for FileLogger {
    fn drop(&mut self) {
        let Ok(mut writer) = self.file.lock() else {
            return;
        };
        _ = writer.flush();
    }
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }
    fn flush(&self) {
        _ = self.file.lock().expect("poisoned mutex").flush();
    }
    fn log(&self, record: &log::Record) {
        if record.level() > self.level
            || record
                .module_path()
                .is_some_and(|it| !it.starts_with("dtu"))
        {
            return;
        }

        let mut writer = self.file.lock().expect("poisoned mutex");

        if let Some(path) = record.module_path() {
            _ = write!(
                writer,
                "{} - {} - {}\n",
                record.level(),
                path,
                record.args()
            );
        } else {
            _ = write!(writer, "{} - {}\n", record.level(), record.args());
        }

        _ = writer.flush();
    }
}

impl FileLogger {
    pub fn new(file: File, level: log::Level) -> Self {
        Self {
            file: Mutex::new(BufWriter::new(file)),
            level,
        }
    }
}

pub struct StderrLogger {
    level: log::Level,
}

impl StderrLogger {
    pub fn new(level: log::Level) -> Self {
        Self { level }
    }
}

impl log::Log for StderrLogger {
    fn log(&self, record: &log::Record) {
        if record.level() > self.level
            || record
                .module_path()
                .is_some_and(|it| !it.starts_with("dtu"))
        {
            return;
        }

        if let Some(path) = record.module_path() {
            eprintln!("{} - {} - {}", record.level(), path, record.args());
        } else {
            eprintln!("{} - {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.level
    }
}
