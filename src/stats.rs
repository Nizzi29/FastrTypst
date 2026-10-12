//! Player profile: settings, attempt history (see `storage`), adaptive level
//! and per-construct weakness analysis.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::generator::Tag;
use crate::storage::{self, Config};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// The Typst source is shown; type it as fast as possible.
    Copy,
    /// Only the rendered formula is shown; write the Typst source yourself.
    Translate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    /// Require DIN typesetting (upright d, j, e). Otherwise these are ignored
    /// when comparing.
    pub strict_din: bool,
    pub live_preview: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { mode: Mode::Translate, strict_din: true, live_preview: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    /// Unix time in seconds.
    pub timestamp: u64,
    pub mode: Mode,
    pub source: String,
    pub tags: Vec<Tag>,
    /// Characters in the canonical source.
    pub chars: usize,
    /// From showing the formula to solving or skipping it.
    pub time_ms: u64,
    pub wrong_submits: u32,
    pub backspaces: u32,
    /// The solution was revealed before solving.
    pub revealed: bool,
    pub skipped: bool,
    /// The player went idle on this formula; its time is meaningless and
    /// excluded from all speed statistics.
    #[serde(default)]
    pub idle: bool,
}

impl Attempt {
    pub fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }

    /// Characters per minute, if this attempt counts towards speed.
    pub fn cpm(&self) -> Option<f32> {
        (!self.revealed && !self.skipped && !self.idle && self.time_ms > 0)
            .then(|| self.chars as f32 / (self.time_ms as f32 / 60_000.0))
    }

    fn penalty(&self) -> f32 {
        self.wrong_submits as f32 + if self.revealed || self.skipped { 2.0 } else { 0.0 }
    }
}

#[derive(Debug, Clone)]
pub struct TagStat {
    pub tag: Tag,
    pub samples: usize,
    pub cpm: Option<f32>,
    /// Average wrong submissions (reveals/skips count as two) per attempt.
    pub error_rate: f32,
    pub weakness: f32,
}

/// The in-memory profile. Levels are not stored: they are replayed from the
/// merged history of all devices.
pub struct Profile {
    pub config: Config,
    pub level_copy: f32,
    pub level_translate: f32,
    pub attempts: Vec<Attempt>,
    /// Attempts per device in the data folder.
    pub devices: Vec<(String, usize)>,
    /// Other devices' logs as last read, to notice newly synced attempts.
    snapshot: Vec<(PathBuf, Option<SystemTime>)>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            config: Config::default(),
            level_copy: MIN_LEVEL,
            level_translate: MIN_LEVEL,
            attempts: Vec::new(),
            devices: Vec::new(),
            snapshot: Vec::new(),
        }
    }
}

/// Number of recent attempts per mode considered for weakness analysis.
const WINDOW: usize = 400;
/// Attempts with a tag needed before its measured speed is trusted.
const MIN_SAMPLES: usize = 3;
/// Weakness assumed for rarely practiced constructs, so they get explored.
pub const EXPLORE_WEAKNESS: f32 = 0.6;

pub const MIN_LEVEL: f32 = 1.0;
pub const MAX_LEVEL: f32 = 10.0;

impl Profile {
    pub fn load() -> Self {
        let mut config = Config::load();
        match storage::migrate_legacy(&config) {
            Ok(Some(settings)) => {
                config.settings = settings;
                if let Err(e) = config.save() {
                    eprintln!("could not save config: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("could not migrate old profile: {e}"),
        }
        let mut profile = Self { config, ..Self::default() };
        profile.reload();
        profile
    }

    /// Re-reads all device logs and replays the levels.
    pub fn reload(&mut self) {
        let dir = self.config.data_dir();
        let history = storage::read_history(&dir);
        self.snapshot = storage::snapshot(&dir, &self.config.log_path());
        self.devices = history.devices;
        self.level_copy = MIN_LEVEL;
        self.level_translate = MIN_LEVEL;
        self.attempts.clear();
        for attempt in history.attempts {
            self.apply(attempt);
        }
    }

    /// Reloads if another device's attempts were synced in. Cheap enough to
    /// call before every formula.
    pub fn reload_if_changed(&mut self) -> bool {
        let snapshot = storage::snapshot(&self.config.data_dir(), &self.config.log_path());
        let changed = snapshot != self.snapshot;
        if changed {
            self.reload();
        }
        changed
    }

    pub fn save_settings(&self) -> Result<(), String> {
        self.config.save()
    }

    /// Switches to another data folder (`None`: local only), taking this
    /// device's log along.
    pub fn set_sync_dir(&mut self, dir: Option<PathBuf>) -> Result<(), String> {
        let old = self.config.log_path();
        let previous = std::mem::replace(&mut self.config.sync_dir, dir);
        let result = std::fs::create_dir_all(self.config.data_dir())
            .map_err(|e| format!("{}: {e}", self.config.data_dir().display()))
            .and_then(|()| storage::move_log(&old, &self.config.log_path()))
            .and_then(|()| self.config.save());
        if result.is_err() {
            self.config.sync_dir = previous;
        }
        self.reload();
        result
    }

    pub fn level(&self, mode: Mode) -> f32 {
        match mode {
            Mode::Copy => self.level_copy,
            Mode::Translate => self.level_translate,
        }
    }

    fn level_mut(&mut self, mode: Mode) -> &mut f32 {
        match mode {
            Mode::Copy => &mut self.level_copy,
            Mode::Translate => &mut self.level_translate,
        }
    }

    fn recent(&self, mode: Mode) -> impl Iterator<Item = &Attempt> {
        self.attempts.iter().rev().filter(move |a| a.mode == mode).take(WINDOW)
    }

    /// Mean characters per minute over recent attempts.
    pub fn average_cpm(&self, mode: Mode) -> Option<f32> {
        mean(self.recent(mode).filter_map(Attempt::cpm))
    }

    /// Records an attempt, saves it to this device's log and adapts the
    /// level. The attempt is kept in memory even if saving fails.
    pub fn record(&mut self, attempt: Attempt) -> Result<(), String> {
        let saved = storage::append(&self.config.log_path(), std::slice::from_ref(&attempt));
        self.apply(attempt);
        saved
    }

    /// Adds an attempt to the history and adapts the level of its mode.
    fn apply(&mut self, attempt: Attempt) {
        let avg = self.average_cpm(attempt.mode);
        let delta = if attempt.skipped || attempt.revealed {
            -0.5
        } else if attempt.wrong_submits > 0 {
            -0.15 * attempt.wrong_submits as f32
        } else {
            match (attempt.cpm(), avg) {
                // Faster than usual: level up quickly.
                (Some(cpm), Some(avg)) if cpm >= avg => 0.3,
                (Some(_), Some(_)) => 0.1,
                (Some(_), None) => 0.3,
                // Untimed (idle): no evidence about speed either way.
                (None, _) => 0.0,
            }
        };
        let level = self.level_mut(attempt.mode);
        *level = (*level + delta).clamp(MIN_LEVEL, MAX_LEVEL);
        self.attempts.push(attempt);
    }

    /// Per-construct statistics, weakest first.
    pub fn tag_stats(&self, mode: Mode) -> Vec<TagStat> {
        let overall = self.average_cpm(mode);
        // Ordered, so ties are listed the same way every frame.
        let mut by_tag: BTreeMap<Tag, Vec<&Attempt>> = BTreeMap::new();
        for a in self.recent(mode) {
            for &t in &a.tags {
                by_tag.entry(t).or_default().push(a);
            }
        }
        let mut stats: Vec<TagStat> = by_tag
            .into_iter()
            .map(|(tag, attempts)| {
                let cpm = mean(attempts.iter().filter_map(|a| a.cpm()));
                let error_rate =
                    attempts.iter().map(|a| a.penalty()).sum::<f32>() / attempts.len() as f32;
                let weakness = if attempts.len() < MIN_SAMPLES {
                    EXPLORE_WEAKNESS
                } else {
                    let slowness = match (overall, cpm) {
                        (Some(o), Some(c)) if c > 0.0 => (o / c - 1.0).max(0.0),
                        _ => 0.5,
                    };
                    slowness + 0.5 * error_rate
                };
                TagStat { tag, samples: attempts.len(), cpm, error_rate, weakness }
            })
            .collect();
        // Stable sort: equal weaknesses keep the tag order.
        stats.sort_by(|a, b| b.weakness.total_cmp(&a.weakness));
        stats
    }

    /// Weakness score per construct; unpracticed constructs get an
    /// exploration bonus.
    pub fn weakness(&self, mode: Mode) -> HashMap<Tag, f32> {
        self.tag_stats(mode)
            .into_iter()
            .map(|s| (s.tag, s.weakness))
            .collect()
    }
}

fn mean(xs: impl Iterator<Item = f32>) -> Option<f32> {
    let (sum, n) = xs.fold((0.0, 0), |(s, n), x| (s + x, n + 1));
    (n > 0).then(|| sum / n as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(tags: Vec<Tag>, time_ms: u64, wrong: u32) -> Attempt {
        Attempt {
            timestamp: 0,
            mode: Mode::Copy,
            source: String::new(),
            tags,
            chars: 30,
            time_ms,
            wrong_submits: wrong,
            backspaces: 0,
            revealed: false,
            skipped: false,
            idle: false,
        }
    }

    #[test]
    fn slow_constructs_are_weak() {
        let mut p = Profile::default();
        for _ in 0..5 {
            p.apply(attempt(vec![Tag::Fraction], 5_000, 0));
            p.apply(attempt(vec![Tag::Matrix], 20_000, 1));
        }
        let w = p.weakness(Mode::Copy);
        assert!(w[&Tag::Matrix] > w[&Tag::Fraction]);
    }

    #[test]
    fn idle_attempts_do_not_count_time() {
        let mut p = Profile::default();
        p.apply(attempt(vec![Tag::Fraction], 5_000, 0));
        let level = p.level_copy;
        let mut idle = attempt(vec![Tag::Fraction], 600_000, 0);
        idle.idle = true;
        p.apply(idle);
        assert_eq!(p.level_copy, level);
        assert_eq!(p.average_cpm(Mode::Copy), Some(360.0));
        assert_eq!(p.tag_stats(Mode::Copy)[0].cpm, Some(360.0));
    }

    #[test]
    fn tag_stats_order_is_stable() {
        let mut p = Profile::default();
        p.apply(attempt(vec![Tag::Root, Tag::Greek, Tag::Vector, Tag::Abs, Tag::Matrix], 5_000, 0));
        let order = |p: &Profile| p.tag_stats(Mode::Copy).iter().map(|s| s.tag).collect::<Vec<_>>();
        let first = order(&p);
        for _ in 0..50 {
            assert_eq!(order(&p), first);
        }
    }

    #[test]
    fn level_adapts() {
        let mut p = Profile::default();
        for _ in 0..10 {
            p.apply(attempt(vec![], 5_000, 0));
        }
        assert!(p.level_copy > 3.0);
        p.apply(attempt(vec![], 5_000, 4));
        assert!(p.level_copy < 3.5);
        assert_eq!(p.level_translate, 1.0);
    }
}
