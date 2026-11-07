//! Battery readings are piggybacked on notification snapshots. Older companions
//! get a bounded, battery-only read; never poll the expensive device summary.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const FALLBACK_INTERVAL: Duration = Duration::from_secs(180);
pub const STALE_AFTER: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatteryState {
    pub level: Option<u8>,
    pub status: i32,
    pub plugged: i32,
}

impl BatteryState {
    pub fn valid(&self) -> bool {
        self.level.is_some_and(|v| v <= 100)
            && (1..=5).contains(&self.status)
            && (0..=15).contains(&self.plugged)
    }
    pub fn charging(&self) -> bool {
        self.plugged > 0 && self.status == 2
    }
    pub fn full(&self) -> bool {
        self.plugged > 0 && self.status == 5
    }
    pub fn label(&self) -> &'static str {
        if self.status == 1 && self.plugged > 0 {
            "已接电，充电状态未知"
        } else if self.status == 1 {
            "充电状态未知"
        } else if self.charging() {
            "正在充电"
        } else if self.full() {
            "电量已满"
        } else if self.plugged > 0 {
            "已接电，未充电"
        } else {
            "使用电池"
        }
    }
    pub fn parse_dumpsys(raw: &str) -> Result<Self> {
        let value = |key: &str| {
            raw.lines().find_map(|line| {
                let (k, v) = line.trim().split_once(':')?;
                (k == key).then_some(v.trim())
            })
        };
        let level = value("level").and_then(|v| v.parse::<u32>().ok());
        let scale = value("scale").and_then(|v| v.parse::<u32>().ok());
        let status = value("status").and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut plugged = 0;
        let mut known = false;
        for (key, flag) in [
            ("AC powered", 1),
            ("USB powered", 2),
            ("Wireless powered", 4),
            ("Dock powered", 8),
        ] {
            match value(key) {
                Some("true") => {
                    plugged |= flag;
                    known = true;
                }
                Some("false") => known = true,
                _ => {}
            }
        }
        let level = match (level, scale) {
            (Some(n), Some(d)) if d > 0 && n <= d => Some((100u64 * n as u64 / d as u64) as u8),
            _ => None,
        };
        let state = Self {
            level,
            status,
            plugged,
        };
        if !known || !state.valid() {
            bail!("未读到有效的手机电池状态");
        }
        Ok(state)
    }
}

/// Monotonic freshness and backoff, independent of the current page or AI work.
pub struct BatteryCache {
    pub reading: Option<BatteryState>,
    pub observed_at: Option<Instant>,
    due_at: Instant,
    failures: u32,
}
impl BatteryCache {
    pub fn new(now: Instant) -> Self {
        Self {
            reading: None,
            observed_at: None,
            due_at: now + Duration::from_secs(10),
            failures: 0,
        }
    }
    pub fn observe(&mut self, value: &BatteryState, now: Instant) -> bool {
        if !value.valid() {
            return false;
        }
        let changed = self.reading.as_ref() != Some(value) || self.stale(now);
        self.reading = Some(value.clone());
        self.observed_at = Some(now);
        self.due_at = now + FALLBACK_INTERVAL;
        self.failures = 0;
        changed
    }
    pub fn due(&self, now: Instant) -> bool {
        now >= self.due_at
    }
    pub fn stale(&self, now: Instant) -> bool {
        self.observed_at
            .is_some_and(|at| now.saturating_duration_since(at) >= STALE_AFTER)
    }
    pub fn fallback_result(
        &mut self,
        started: Instant,
        now: Instant,
        result: Result<BatteryState>,
    ) -> bool {
        // A later push receipt takes precedence over an older in-flight read.
        if self.observed_at.is_some_and(|at| at > started) {
            return false;
        }
        match result {
            Ok(value) if value.valid() => self.observe(&value, now),
            _ => {
                self.failures = (self.failures + 1).min(3);
                self.due_at = now + Duration::from_secs((180 * (1u64 << self.failures)).min(600));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(level: u8, status: i32, plugged: i32) -> BatteryState {
        BatteryState {
            level: Some(level),
            status,
            plugged,
        }
    }
    #[test]
    fn power_source_and_status_are_distinct() {
        assert!(state(66, 2, 2).charging());
        assert!(state(100, 5, 4).full());
        assert_eq!(state(80, 4, 1).label(), "已接电，未充电");
        assert_eq!(state(100, 5, 0).label(), "使用电池");
        assert!(!state(99, 2, 0).charging());
    }
    #[test]
    fn parse_zero_scaled_and_bad_readings() {
        let raw = "AC powered: false\nUSB powered: true\nlevel: 33\nscale: 50\nstatus: 2";
        assert_eq!(BatteryState::parse_dumpsys(raw).unwrap(), state(66, 2, 2));
        assert_eq!(
            BatteryState::parse_dumpsys(&raw.replace("33", "0"))
                .unwrap()
                .level,
            Some(0)
        );
        for bad in [
            raw.replace("50", "0"),
            raw.replace("33", "51"),
            "error: closed".into(),
        ] {
            assert!(BatteryState::parse_dumpsys(&bad).is_err());
        }
        let null: BatteryState =
            serde_json::from_str(r#"{"level":null,"status":-1,"plugged":-1}"#).unwrap();
        assert!(!null.valid());
    }
    #[test]
    fn rises_falls_and_unchanged_receipts_keep_cache_fresh() {
        let start = Instant::now();
        let mut cache = BatteryCache::new(start);
        for (secs, level) in [(0, 66), (30, 65), (60, 67)] {
            assert!(cache.observe(&state(level, 3, 0), start + Duration::from_secs(secs)));
        }
        assert!(!cache.observe(&state(67, 3, 0), start + Duration::from_secs(220)));
        assert!(!cache.due(start + Duration::from_secs(399)));
        assert!(cache.due(start + Duration::from_secs(400)));
        assert_eq!(cache.reading.unwrap().level, Some(67));
    }
    #[test]
    fn failures_preserve_last_value_backoff_and_late_reads_cannot_regress_push() {
        let start = Instant::now();
        let mut cache = BatteryCache::new(start);
        cache.observe(&state(66, 3, 0), start);
        cache.fallback_result(
            start,
            start + FALLBACK_INTERVAL,
            Err(anyhow::anyhow!("offline")),
        );
        assert_eq!(cache.reading.as_ref().unwrap().level, Some(66));
        assert!(!cache.due(start + Duration::from_secs(539)));
        assert!(cache.due(start + Duration::from_secs(540)));
        assert!(cache.stale(start + STALE_AFTER));
        cache.observe(&state(67, 2, 1), start + Duration::from_secs(541));
        assert!(!cache.fallback_result(
            start + Duration::from_secs(540),
            start + Duration::from_secs(542),
            Ok(state(65, 3, 0))
        ));
        assert_eq!(cache.reading.unwrap(), state(67, 2, 1));
    }
}
