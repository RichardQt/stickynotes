//! Pre-write backups of plum.sqlite (+ WAL sidecars).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::{PlumError, Result};

const KEEP: usize = 20;
const MIN_INTERVAL: Duration = Duration::from_secs(300);

static LAST_BACKUP_TIME: AtomicU64 = AtomicU64::new(0);

fn backup_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    Path::new(&base).join("stickynotes").join("backups")
}

/// Copy plum.sqlite + -wal + -shm into a timestamped backup dir.
/// Skips if a backup was made within MIN_INTERVAL (avoids hammering on autosave).
pub fn backup_if_needed(plum: &Path) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let last = LAST_BACKUP_TIME.load(Ordering::Relaxed);
    if last > 0 && now.saturating_sub(last) < MIN_INTERVAL.as_secs() {
        return Ok(());
    }

    let dir = backup_dir();
    if let Some(last_sys) = latest_backup_time(&dir) {
        if last_sys.elapsed().unwrap_or(MIN_INTERVAL * 2) < MIN_INTERVAL {
            LAST_BACKUP_TIME.store(now, Ordering::Relaxed);
            return Ok(());
        }
    }
    do_backup(plum, &dir)?;
    LAST_BACKUP_TIME.store(now, Ordering::Relaxed);
    Ok(())
}

pub fn do_backup(plum: &Path, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    let stamp = now_stamp();
    let dest = dir.join(stamp);
    fs::create_dir_all(&dest)?;

    let base = plum.to_path_buf();
    for suffix in ["", "-wal", "-shm"] {
        let src = {
            let mut p = base.clone();
            p.as_mut_os_string().push(suffix);
            p
        };
        if src.exists() {
            let name = src.file_name().unwrap_or_default();
            fs::copy(&src, dest.join(name))?;
        }
    }

    // Verify the main db copy is a valid SQLite file (header check, cheap).
    let copied = dest.join("plum.sqlite");
    if copied.exists() {
        let mut header = [0u8; 16];
        use std::io::Read;
        let mut f = fs::File::open(&copied)?;
        f.read_exact(&mut header)?;
        if &header[..15] != b"SQLite format 3" {
            return Err(PlumError::Other("backup integrity check failed".into()));
        }
    }

    prune(dir)?;
    Ok(())
}

fn latest_backup_time(dir: &Path) -> Option<std::time::SystemTime> {
    let mut newest: Option<(String, std::time::SystemTime)> = None;
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Ok(meta) = e.metadata() {
                if let Ok(t) = meta.modified() {
                    if newest.as_ref().map(|(n, _)| name > *n).unwrap_or(true) {
                        newest = Some((name, t));
                    }
                }
            }
        }
    }
    newest.map(|(_, t)| t)
}

fn prune(dir: &Path) -> Result<()> {
    let mut names: Vec<String> = fs::read_dir(dir)?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    while names.len() > KEEP {
        let oldest = names.remove(0);
        fs::remove_dir_all(dir.join(oldest))?;
    }
    Ok(())
}

fn now_stamp() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = t.as_secs();
    let (y, mo, d, h, mi, s) = civil_from_unix(secs as i64);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// Minimal UTC civil-date conversion (no chrono dependency).
fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
