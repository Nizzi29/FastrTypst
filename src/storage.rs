//! Sync-friendly storage.
//!
//! Every device appends its attempts to its own `attempts-<device>.jsonl` in
//! the data folder, and all such files in that folder are merged on load.
//! No file is ever written by two devices, so a data folder synced with
//! Nextcloud (or similar) cannot produce sync conflicts, even when devices
//! practice offline. Per-device settings live in a config file that is not
//! synced.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::stats::{Attempt, Settings};

const LOG_PREFIX: &str = "attempts-";
const LOG_SUFFIX: &str = ".jsonl";

fn dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", "fastertypst")
}

/// `~/.local/share/fastertypst`, used when no sync folder is set.
pub fn local_data_dir() -> PathBuf {
    dirs().map(|d| d.data_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
}

/// Expands a leading `~` to the home directory.
pub fn expand_home(path: &str) -> PathBuf {
    let path = path.trim();
    match (path.strip_prefix('~'), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            PathBuf::from(home).join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(path),
    }
}

/// Per-device configuration (`~/.config/fastertypst/config.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub settings: Settings,
    /// Names this device's log file. Generated once per device.
    pub device: String,
    /// Folder shared between devices, e.g. inside Nextcloud.
    pub sync_dir: Option<PathBuf>,
}

impl Config {
    fn path() -> Option<PathBuf> {
        dirs().map(|d| d.config_dir().join("config.json"))
    }

    pub fn load() -> Self {
        let path = Self::path();
        let mut config = match path.as_ref().map(fs::read_to_string) {
            Some(Ok(json)) => serde_json::from_str(&json).unwrap_or_else(|e| {
                let path = path.as_ref().unwrap();
                let backup = path.with_extension(format!("json.broken-{}", Attempt::now()));
                eprintln!("could not read {}: {e}; moved to {}", path.display(), backup.display());
                let _ = fs::rename(path, &backup);
                Self::default()
            }),
            _ => Self::default(),
        };
        if config.device.is_empty() {
            config.device = new_device_id();
            if let Err(e) = config.save() {
                eprintln!("could not save config: {e}");
            }
        }
        config
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no config directory")?;
        write_atomic(&path, &serde_json::to_string_pretty(self).map_err(|e| e.to_string())?)
    }

    pub fn data_dir(&self) -> PathBuf {
        self.sync_dir.clone().unwrap_or_else(local_data_dir)
    }

    pub fn log_path(&self) -> PathBuf {
        self.data_dir().join(format!("{LOG_PREFIX}{}{LOG_SUFFIX}", self.device))
    }
}

/// Host name plus a random suffix, safe for file names.
fn new_device_id() -> String {
    let host = fs::read_to_string("/etc/hostname").unwrap_or_default();
    let host: String = host
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .take(32)
        .collect();
    let host = if host.is_empty() { "device".to_string() } else { host };
    format!("{host}-{:04x}", rand::random::<u16>())
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// The merged history of all devices in a data folder.
pub struct History {
    /// All attempts, oldest first.
    pub attempts: Vec<Attempt>,
    /// Attempts per device.
    pub devices: Vec<(String, usize)>,
}

fn log_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut files: Vec<(String, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let device = name.strip_prefix(LOG_PREFIX)?.strip_suffix(LOG_SUFFIX)?.to_string();
            Some((device, entry.path()))
        })
        .collect();
    files.sort();
    files
}

/// Reads and merges all device logs in `dir`. Unreadable lines (e.g. a line
/// still being synced) are skipped; identical lines, as in a sync tool's
/// conflict copies, are counted once.
pub fn read_history(dir: &Path) -> History {
    let mut seen = HashSet::new();
    let mut attempts = Vec::new();
    let mut devices = Vec::new();
    for (device, path) in log_files(dir) {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let mut count = 0;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if !seen.insert(line.to_string()) {
                continue;
            }
            if let Ok(attempt) = serde_json::from_str::<Attempt>(line) {
                attempts.push(attempt);
                count += 1;
            }
        }
        if count > 0 {
            devices.push((device, count));
        }
    }
    // Stable: same-second attempts keep their order within a device.
    attempts.sort_by_key(|a| a.timestamp);
    History { attempts, devices }
}

/// Modification times of all device logs in `dir` except `own`, to detect
/// changes synced in from other devices.
pub fn snapshot(dir: &Path, own: &Path) -> Vec<(PathBuf, Option<SystemTime>)> {
    log_files(dir)
        .into_iter()
        .filter(|(_, path)| path != own)
        .map(|(_, path)| {
            let modified = fs::metadata(&path).and_then(|m| m.modified()).ok();
            (path, modified)
        })
        .collect()
}

/// Appends attempts to a log as one JSON object per line.
pub fn append(path: &Path, attempts: &[Attempt]) -> Result<(), String> {
    let mut lines = String::new();
    for attempt in attempts {
        lines += &serde_json::to_string(attempt).map_err(|e| e.to_string())?;
        lines.push('\n');
    }
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    // Never continue a line cut off by a crash.
    if file.metadata().map_err(|e| e.to_string())?.len() > 0 {
        let mut last = [0u8];
        file.seek(SeekFrom::End(-1)).map_err(|e| e.to_string())?;
        file.read_exact(&mut last).map_err(|e| e.to_string())?;
        if last[0] != b'\n' {
            lines.insert(0, '\n');
        }
    }
    file.write_all(lines.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}

/// Moves a log to a new location, merging it into an existing log there.
pub fn move_log(from: &Path, to: &Path) -> Result<(), String> {
    if from == to || !from.exists() {
        return Ok(());
    }
    fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
    if !to.exists() && fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let existing: HashSet<String> = fs::read_to_string(to)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .collect();
    let text = fs::read_to_string(from).map_err(|e| e.to_string())?;
    let missing: Vec<Attempt> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !existing.contains(*l))
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    append(to, &missing)?;
    fs::remove_file(from).map_err(|e| e.to_string())
}

/// Profile format before per-device logs (`profile.json`).
#[derive(Deserialize)]
struct LegacyProfile {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    attempts: Vec<Attempt>,
}

/// Moves a pre-sync `profile.json` into this device's log. Returns its
/// settings if there was one to migrate.
pub fn migrate_legacy(config: &Config) -> Result<Option<Settings>, String> {
    let path = local_data_dir().join("profile.json");
    let Ok(json) = fs::read_to_string(&path) else { return Ok(None) };
    let legacy: LegacyProfile =
        serde_json::from_str(&json).map_err(|e| format!("{}: {e}", path.display()))?;
    append(&config.log_path(), &legacy.attempts)?;
    fs::rename(&path, path.with_extension("json.migrated")).map_err(|e| e.to_string())?;
    Ok(Some(legacy.settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::Mode;

    fn attempt(timestamp: u64, source: &str) -> Attempt {
        Attempt {
            timestamp,
            mode: Mode::Copy,
            source: source.into(),
            tags: vec![],
            chars: source.len(),
            time_ms: 1000,
            wrong_submits: 0,
            backspaces: 0,
            revealed: false,
            skipped: false,
            idle: false,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fastertypst-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn merges_devices_in_time_order() {
        let dir = temp_dir("merge");
        append(&dir.join("attempts-pc.jsonl"), &[attempt(1, "a"), attempt(3, "c")]).unwrap();
        append(&dir.join("attempts-laptop.jsonl"), &[attempt(2, "b")]).unwrap();
        // A truncated line from an unfinished sync, and a conflict copy.
        fs::write(dir.join("attempts-tablet.jsonl"), "{\"timestamp\": 4, \"mo").unwrap();
        fs::copy(dir.join("attempts-pc.jsonl"), dir.join("attempts-pc (conflicted copy).jsonl")).unwrap();
        let h = read_history(&dir);
        let order: Vec<&str> = h.attempts.iter().map(|a| a.source.as_str()).collect();
        assert_eq!(order, ["a", "b", "c"]);
        assert_eq!(h.devices, [("laptop".to_string(), 1), ("pc".to_string(), 2)]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn append_repairs_truncated_line() {
        let dir = temp_dir("truncated");
        let log = dir.join("attempts-pc.jsonl");
        fs::write(&log, "{\"broken").unwrap();
        append(&log, &[attempt(1, "a")]).unwrap();
        assert_eq!(read_history(&dir).attempts.len(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn move_merges_without_duplicates() {
        let dir = temp_dir("move");
        let (from, to) = (dir.join("local/attempts-pc.jsonl"), dir.join("sync/attempts-pc.jsonl"));
        append(&from, &[attempt(1, "a"), attempt(2, "b")]).unwrap();
        append(&to, &[attempt(1, "a")]).unwrap();
        move_log(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(read_history(&dir.join("sync")).attempts.len(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn home_expansion() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        assert_eq!(expand_home("~/Nextcloud/x"), home.join("Nextcloud/x"));
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
    }
}
