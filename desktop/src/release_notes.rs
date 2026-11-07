//! Curated, bundled release stories. No network, model call or disk scan while rendering.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    Home,
    Search,
    Messages,
    Files,
    Agent,
    Phone,
    About,
    History,
    Amoo,
    Studio,
    Calendar,
    Alarms,
    Gallery,
    BackScreen,
    Tools,
    Connect,
    Account,
    Settings,
    Notifications,
    Storage,
    AppStore,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Highlight {
    pub id: String,
    pub category: String,
    pub title: String,
    pub summary: String,
    pub image: String,
    pub image_alt: String,
    pub caption: String,
    pub action_label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub id: String,
    pub title: String,
    pub benefit: String,
    pub destination: Destination,
    pub versions: Vec<String>,
    pub details: Vec<String>,
    pub kind: String,
    pub condition: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: String,
    pub title: String,
    pub outcome: String,
    pub image: String,
    pub features: Vec<Feature>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseNotes {
    pub version: String,
    pub baseline: String,
    pub baseline_url: String,
    pub date: String,
    pub title: String,
    pub summary: String,
    pub captured_from: String,
    pub highlights: Vec<Highlight>,
    pub categories: Vec<Category>,
    pub boundaries: Vec<String>,
    #[serde(default)]
    pub video: Option<Video>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Video {
    pub chapters: Vec<Chapter>,
    pub title: String,
    pub summary: String,
    pub duration_seconds: u32,
    pub captured_from: String,
    pub poster: String,
    pub caption: String,
    pub file: String,
    pub native_file: String,
    pub fps: u32,
    pub frame_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chapter {
    pub second: u32,
    pub title: String,
}

pub const MANIFEST: &str = include_str!(concat!(
    "../assets/release-notes/",
    env!("CARGO_PKG_VERSION"),
    ".json"
));

impl ReleaseNotes {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let notes: Self = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        if notes.version != crate::build_info::VERSION {
            return Err("图文说明与客户端版本不一致".into());
        }
        if notes.highlights.is_empty()
            || notes.highlights.len() > 8
            || notes.title.trim().is_empty()
        {
            return Err("更新说明需要 1–8 个有标题的核心场景".into());
        }
        let mut ids = HashSet::new();
        for item in &notes.highlights {
            if item.id.is_empty()
                || !ids.insert(&item.id)
                || item.title.trim().is_empty()
                || item.summary.trim().is_empty()
                || item.image_alt.trim().is_empty()
                || item.caption.trim().is_empty()
                || item.action_label.trim().is_empty()
                || image_asset(&item.image).is_none()
            {
                return Err(format!("更新场景不完整或资源不存在：{}", item.id));
            }
        }
        if notes.baseline_url
            != format!(
                "https://github.com/xydzf520/stepx/releases/tag/v{}",
                notes.baseline
            )
        {
            return Err("公开版本来源不匹配".into());
        }
        if let Some(video) = &notes.video {
            if video.native_file != "release-notes/tour.h264"
                || video.fps != crate::release_video::FPS
                || video.frame_count != crate::release_video::FRAME_COUNT
                || video.file != "release-notes/tour.mp4"
                || !(5..=90).contains(&video.duration_seconds)
                || image_asset(&video.poster).is_none()
                || video.title.trim().is_empty()
                || video.summary.trim().is_empty()
                || video.captured_from.trim().is_empty()
                || video.caption.trim().is_empty()
            {
                return Err("视频说明或离线资源不完整".into());
            }
            if video.chapters.is_empty()
                || video.chapters.len() > 8
                || video.chapters[0].second != 0
                || video
                    .chapters
                    .iter()
                    .any(|c| c.second >= video.duration_seconds || c.title.trim().is_empty())
                || video
                    .chapters
                    .windows(2)
                    .any(|c| c[0].second >= c[1].second)
            {
                return Err("视频章节无效".into());
            }
        }
        if notes.categories.len() != 11 {
            return Err("需要完整的十一类更新目录".into());
        }
        let mut category_ids = HashSet::new();
        for c in &notes.categories {
            if c.id.is_empty()
                || !category_ids.insert(&c.id)
                || c.title.trim().is_empty()
                || c.outcome.trim().is_empty()
                || image_asset(&c.image).is_none()
                || c.features.is_empty()
                || c.features.len() > 12
            {
                return Err("更新分类不完整".into());
            }
            for f in &c.features {
                if f.id.is_empty()
                    || !ids.insert(&f.id)
                    || f.title.trim().is_empty()
                    || f.benefit.trim().is_empty()
                    || f.details.is_empty()
                    || f.details.len() > 3
                    || f.details.iter().any(|s| s.trim().is_empty())
                    || !["新增", "改进"].contains(&f.kind.as_str())
                    || f.versions.is_empty()
                    || f.versions.iter().any(|v| {
                        !history()
                            .iter()
                            .any(|h| h.heading.starts_with(&format!("{v} ")))
                    })
                {
                    return Err(format!("场景缺少有效的功能依据：{}", f.id));
                }
            }
        }
        if notes
            .highlights
            .iter()
            .any(|h| !category_ids.contains(&h.category))
        {
            return Err("重点更新缺少分类入口".into());
        }
        Ok(notes)
    }
}

pub const VIDEO_BYTES: &[u8] = include_bytes!(concat!(
    "../assets/release-notes/",
    env!("CARGO_PKG_VERSION"),
    "/tour.mp4"
));

pub fn current() -> Result<&'static ReleaseNotes, &'static str> {
    static NOTES: std::sync::OnceLock<Result<ReleaseNotes, String>> = std::sync::OnceLock::new();
    NOTES
        .get_or_init(|| ReleaseNotes::parse(MANIFEST))
        .as_ref()
        .map_err(String::as_str)
}

pub struct HistorySection {
    pub heading: String,
    pub lines: Vec<String>,
}
pub fn history() -> &'static [HistorySection] {
    static HISTORY: std::sync::OnceLock<Vec<HistorySection>> = std::sync::OnceLock::new();
    HISTORY.get_or_init(|| {
        let mut sections: Vec<HistorySection> = Vec::new();
        for line in include_str!("../../CHANGELOG.md").lines() {
            if let Some(heading) = line.strip_prefix("## ") {
                sections.push(HistorySection {
                    heading: heading.to_owned(),
                    lines: Vec::new(),
                });
            } else if let Some(section) = sections.last_mut() {
                if !line.trim().is_empty() {
                    section.lines.push(line.replace("**", ""));
                }
            }
        }
        sections
    })
}

pub fn image_asset(path: &str) -> Option<&'static [u8]> {
    Some(match path {
        "release-notes/tools.png" => include_bytes!(concat!(
            "../assets/release-notes/",
            env!("CARGO_PKG_VERSION"),
            "/tools.png"
        )),
        "release-notes/connection.png" => include_bytes!(concat!(
            "../assets/release-notes/",
            env!("CARGO_PKG_VERSION"),
            "/connection.png"
        )),
        "release-notes/capsule.png" => include_bytes!(concat!(
            "../assets/release-notes/",
            env!("CARGO_PKG_VERSION"),
            "/capsule.png"
        )),
        "release-notes/home.png" => include_bytes!(concat!(
            "../assets/release-notes/",
            env!("CARGO_PKG_VERSION"),
            "/home.png"
        )),
        "release-notes/search.png" => include_bytes!(concat!(
            "../assets/release-notes/",
            env!("CARGO_PKG_VERSION"),
            "/search.png"
        )),
        "release-notes/messages.png" => include_bytes!(concat!(
            "../assets/release-notes/",
            env!("CARGO_PKG_VERSION"),
            "/messages.png"
        )),
        "release-notes/files.png" => include_bytes!(concat!(
            "../assets/release-notes/",
            env!("CARGO_PKG_VERSION"),
            "/files.png"
        )),
        "release-notes/agent.png" => include_bytes!(concat!(
            "../assets/release-notes/",
            env!("CARGO_PKG_VERSION"),
            "/agent.png"
        )),
        "release-notes/phone.png" => include_bytes!(concat!(
            "../assets/release-notes/",
            env!("CARGO_PKG_VERSION"),
            "/phone.png"
        )),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_story_has_real_decodable_bounded_images() {
        let notes = ReleaseNotes::parse(MANIFEST).unwrap();
        assert_eq!(notes.baseline, "0.2.60");
        assert!(history()
            .first()
            .unwrap()
            .heading
            .starts_with(crate::build_info::VERSION));
        assert!(history()
            .iter()
            .any(|section| section.heading.starts_with("0.2.60")));
        for item in &notes.highlights {
            let bytes = image_asset(&item.image).unwrap();
            assert!(bytes.len() < 512 * 1024);
            let image = image::load_from_memory(bytes).unwrap();
            assert!(image.width() >= 600 && image.width() <= 1600 && image.height() >= 240);
        }
        assert!(!notes.boundaries.is_empty());
    }
    #[test]
    fn invalid_stories_never_load_remote_or_arbitrary_assets() {
        let original: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
        for replacement in [
            "https://example.org/image.png",
            "../../private.png",
            "release-notes/missing.png",
        ] {
            let mut v = original.clone();
            v["highlights"][0]["image"] = replacement.into();
            assert!(ReleaseNotes::parse(&v.to_string()).is_err());
        }
        let mut v = original.clone();
        v["highlights"][1]["id"] = v["highlights"][0]["id"].clone();
        assert!(ReleaseNotes::parse(&v.to_string()).is_err());
        v = original;
        v["version"] = "0.0.0".into();
        assert!(ReleaseNotes::parse(&v.to_string()).is_err());
    }
    #[test]
    fn catalogue_has_unique_evidenced_scenarios_and_safe_destinations() {
        let notes = current().unwrap();
        assert!(
            notes
                .categories
                .iter()
                .map(|c| c.features.len())
                .sum::<usize>() >= 65
        );
        assert_eq!(notes.categories[0].id, "alerts");
        for item in &notes.categories {
            assert!(image_asset(&item.image).is_some());
        }
        let mut v: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
        v["categories"][0]["features"][0]["versions"] = serde_json::json!(["0.2.999"]);
        assert!(ReleaseNotes::parse(&v.to_string()).is_err());
    }
}
