//! Independent logical sizes for the full and compact shells.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Full,
    MiniAi,
    MiniNotify,
}
impl Mode {
    pub fn compact(self) -> bool {
        self != Self::Full
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}
impl Size {
    pub fn valid(self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && (360.0..=16_384.0).contains(&self.width)
            && (420.0..=16_384.0).contains(&self.height)
    }

    pub fn valid_full(self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && (900.0..=16_384.0).contains(&self.width)
            && (600.0..=16_384.0).contains(&self.height)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub full: Size,
    pub compact: Size,
    pub mode: Mode,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            full: Size {
                width: 1320.,
                height: 860.,
            },
            compact: Size {
                width: 600.,
                height: 760.,
            },
            mode: Mode::Full,
        }
    }
}
impl Preferences {
    pub fn load(path: &Path) -> Self {
        let mut value: Self = crate::local_store::read_bounded(path, 64 * 1024)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let defaults = Self::default();
        if !value.full.valid_full() {
            value.full = defaults.full;
        }
        if !value.compact.valid() {
            value.compact = defaults.compact;
        }
        value
    }
    pub fn size(&self, mode: Mode) -> Size {
        if mode.compact() {
            self.compact
        } else {
            self.full
        }
    }
    pub fn record(&mut self, mode: Mode, size: Option<Size>) {
        self.mode = mode;
        if mode.compact() {
            if let Some(size) = size.filter(|s| s.valid()) {
                self.compact = size;
            }
        } else if let Some(size) = size.filter(|s| s.valid_full()) {
            self.full = size;
        }
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let defaults = Self::default();
        let sanitized = Self {
            full: if self.full.valid_full() {
                self.full
            } else {
                defaults.full
            },
            compact: if self.compact.valid() {
                self.compact
            } else {
                defaults.compact
            },
            mode: self.mode,
        };
        let content = serde_json::to_string(&sanitized)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        crate::local_store::write_if_changed(path, content.as_bytes())
            .map(|_| ())
            .map_err(|err| {
                err.downcast::<std::io::Error>()
                    .unwrap_or_else(std::io::Error::other)
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes_round_trip_without_overwriting_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("window.json");
        let mut p = Preferences::default();
        p.record(
            Mode::Full,
            Some(Size {
                width: 1440.,
                height: 930.,
            }),
        );
        p.record(
            Mode::MiniAi,
            Some(Size {
                width: 690.,
                height: 810.,
            }),
        );
        p.record(Mode::MiniNotify, None); // A maximized/minimized window must not replace normal size.
        p.save(&path).unwrap();
        let loaded = Preferences::load(&path);
        assert_eq!(p, loaded);
        assert_eq!(loaded.full.width, 1440.);
        assert_eq!(loaded.size(Mode::MiniNotify).width, 690.);
    }
    #[test]
    fn corrupt_and_invalid_sizes_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("window.json");
        std::fs::write(&path, "broken").unwrap();
        assert_eq!(Preferences::load(&path), Preferences::default());
        let mut p = Preferences::default();
        p.full.width = -1.;
        p.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).full, Preferences::default().full);
        p.record(
            Mode::Full,
            Some(Size {
                width: f32::NAN,
                height: 20.,
            }),
        );
        assert_eq!(p.full.width, -1.);
    }
    #[test]
    fn compact_size_cannot_contaminate_full_mode_and_save_is_atomic_0600() {
        let compact_min = Size {
            width: 400.,
            height: 480.,
        };
        assert!(compact_min.valid());
        assert!(!compact_min.valid_full());
        assert!(!(Size {
            width: 359.,
            height: 420.,
        })
        .valid());
        assert!(!(Size {
            width: 360.,
            height: 419.,
        })
        .valid());
        assert!((Size {
            width: 900.,
            height: 600.,
        })
        .valid_full());
        assert!(!(Size {
            width: 899.,
            height: 600.,
        })
        .valid_full());
        assert!(!(Size {
            width: 900.,
            height: 599.,
        })
        .valid_full());
        assert!(!(Size {
            width: 16_385.,
            height: 900.,
        })
        .valid_full());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("window.json");

        // Corrupted window.json with 400x480 full entry falls back to default full size on load.
        std::fs::write(
            &path,
            r#"{"full":{"width":400.0,"height":480.0},"compact":{"width":400.0,"height":480.0},"mode":"full"}"#,
        )
        .unwrap();
        let loaded = Preferences::load(&path);
        assert_eq!(loaded.full, Preferences::default().full);
        assert_eq!(loaded.compact, compact_min);

        // Recording a compact 400x480 size while in Full mode never overwrites `full`.
        let mut prefs = Preferences::default();
        prefs.record(Mode::Full, Some(compact_min));
        assert_eq!(prefs.full, Preferences::default().full);
        prefs.record(Mode::MiniAi, Some(compact_min));
        assert_eq!(prefs.compact, compact_min);

        // Saving an invalid 400x480 `full` sanitizes persisted JSON and uses 0600 atomic write.
        let mut corrupted_in_mem = Preferences {
            full: compact_min,
            compact: compact_min,
            mode: Mode::Full,
        };
        corrupted_in_mem.save(&path).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
        let raw_saved: Preferences =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw_saved.full, Preferences::default().full);
        assert_eq!(raw_saved.compact, compact_min);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        corrupted_in_mem.full = Preferences::default().full;
        assert_eq!(Preferences::load(&path), corrupted_in_mem);
    }
}
