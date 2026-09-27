//! Simple file logger that writes to %TEMP%\phonon-installer.log.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;

static LOG_FILE: Mutex<Option<String>> = Mutex::new(None);

pub fn init() {
    let path = log_file_path();
    *LOG_FILE.lock().unwrap() = Some(path);

    let _ = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&*LOG_FILE.lock().unwrap().as_ref().unwrap());

    log::set_logger(&FileLogger).expect("set_logger");
    log::set_max_level(log::LevelFilter::Trace);

    log::info!("=== Phonon Installer Log ===");
}

pub fn log_file_path() -> String {
    let temp = std::env::var("TEMP").unwrap_or_else(|_| ".".to_string());
    format!("{}\\phonon-installer.log", temp)
}

struct FileLogger;

impl log::Log for FileLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let guard = LOG_FILE.lock().unwrap();
        if let Some(path) = guard.as_ref() {
            if let Ok(mut file) = OpenOptions::new().append(true).open(path) {
                let line = format!(
                    "[{}] {}: {}\r\n",
                    record.level(),
                    chrono_like_ts(),
                    record.args()
                );
                let _ = file.write_all(line.as_bytes());
            }
        }
    }

    fn flush(&self) {}
}

fn chrono_like_ts() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let days = secs / 86400;
    let rem = secs % 86400;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    let (y, mo, d) = epoch_days_to_ymd(days as i64);
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, m, s)
}

/// Convert days since 1970-01-01 to (year, month, day).
fn epoch_days_to_ymd(days: i64) -> (i64, u32, u32) {
    let mut y = 1970i64;
    let mut d = days;
    loop {
        let leap = is_leap(y);
        let yr_days = if leap { 366 } else { 365 };
        if d >= yr_days {
            d -= yr_days;
            y += 1;
        } else {
            break;
        }
    }
    let leap = is_leap(y);
    let months: [i64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mo = 1u32;
    for &md in &months {
        if d >= md {
            d -= md;
            mo += 1;
        } else {
            break;
        }
    }
    (y, mo, (d + 1) as u32)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
