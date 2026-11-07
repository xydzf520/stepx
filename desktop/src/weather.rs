//! Real-Time Weather & Commute Context Engine (`Page::Home`).
//!
//! Provides live Open-Meteo weather fetching (via `curl` with short timeout),
//! local disk/memory caching, rich multi-city fallback snapshots, phone rain/wind
//! alert enrichment, and one-click context handoff prompts for Amoo and schedules.

use chrono::{Datelike, Duration as ChronoDuration, Local, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub const PRESET_CITIES: [&str; 6] = ["上海", "苏州", "杭州", "南京", "北京", "深圳"];

pub const WEATHER_SNAPSHOT_TTL: Duration = Duration::from_secs(600);
const WEATHER_DISPLAY_CACHE_TTL: Duration = Duration::from_secs(2 * 3600);
pub const PHONE_DAILY_ADVICE_CACHE_PATH: &str =
    "/sdcard/Android/data/com.stepx.companion.app/files/weather_daily_advice.json";

static WEATHER_CACHE_REVISION: AtomicU64 = AtomicU64::new(1);
// User selection is independent of cache updates: an old city fetch must not
// overwrite a more recent selection while its background request is in flight.
static WEATHER_CITY_REVISION: AtomicU64 = AtomicU64::new(1);

static PREFERRED_CITY_MEM: Mutex<Option<String>> = Mutex::new(None);
#[derive(Clone)]
struct CachedWeatherSnapshot {
    snapshot: WeatherSnapshot,
    stored_at: Instant,
    device_key: Option<String>,
}

impl CachedWeatherSnapshot {
    fn fresh_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.stored_at) < WEATHER_DISPLAY_CACHE_TTL
    }

    fn for_city_and_device(&self, city: &str, key: Option<&str>, now: Instant) -> Option<WeatherSnapshot> {
        (self.fresh_at(now)
            && (self.device_key.is_none() || self.device_key.as_deref() == key)
            && weather_city_matches(&self.snapshot, city))
            .then(|| self.snapshot.clone())
    }
}

static SNAPSHOT_MEM_CACHE: Mutex<Option<CachedWeatherSnapshot>> = Mutex::new(None);
static DAILY_ADVICE_MEM_CACHE: Mutex<Option<CachedDailyWeatherAdvice>> = Mutex::new(None);
static WEATHER_SNAPSHOT_CACHE: LazyLock<Mutex<HashMap<String, (Instant, WeatherSnapshot)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub type DailyTomorrowAdviceCache = CachedDailyWeatherAdvice;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedDailyWeatherAdvice {
    /// Calendar date in `YYYY-MM-DD` (local time). Cache is valid for the entire calendar day
    /// and only refreshes on the next day (`date_ymd != today_ymd`).
    pub date_ymd: String,
    pub city: String,
    pub today_short: String,
    pub tomorrow_advice: String,
    pub generated_by_amoo: bool,
}

/// Monotonic revision counter incremented when city, live weather or daily advice changes.
pub fn weather_cache_revision() -> u64 {
    WEATHER_CACHE_REVISION.load(Ordering::Relaxed)
}

pub fn weather_selection_revision() -> u64 {
    WEATHER_CITY_REVISION.load(Ordering::Relaxed)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherHourlyPoint {
    pub hour_label: String,
    pub temp_c: i32,
    pub condition: String,
    pub pop_pct: u8,
    pub wind_kmh: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherDailyPoint {
    pub date_label: String,
    pub ymd: String,
    pub condition: String,
    pub high_c: i32,
    pub low_c: i32,
    pub pop_pct: u8,
    pub advice: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherSnapshot {
    pub city: String,
    pub region_label: String,
    pub temp_c: i32,
    pub feels_like_c: i32,
    pub condition: String,
    pub high_c: i32,
    pub low_c: i32,
    pub humidity_pct: u8,
    pub wind_label: String,
    pub aqi: u16,
    pub aqi_label: String,
    pub uv_label: String,
    pub umbrella_needed: bool,
    pub commute_advice: String,
    pub backscreen_badge: String,
    pub updated_label: String,
    pub hourly: Vec<WeatherHourlyPoint>,
    pub daily: Vec<WeatherDailyPoint>,
    pub from_live_api: bool,
}

impl WeatherSnapshot {
    /// Returns a realistic, seasonally accurate weather snapshot for the given city.
    pub fn default_for_city(city: &str) -> Self {
        let normalized = {
            let trimmed = city.trim();
            if trimmed.is_empty() {
                "上海"
            } else {
                trimmed
            }
        };
        let now = Local::now();
        let base_hour = now.hour();
        let today = now.date_naive();

        let make_hourly = |temps: [i32; 6],
                           conds: [&str; 6],
                           pops: [u8; 6],
                           winds: [u8; 6]|
         -> Vec<WeatherHourlyPoint> {
            (0..6)
                .map(|i| {
                    let h = (base_hour + i as u32) % 24;
                    let hour_label = if i == 0 {
                        "现在".to_string()
                    } else {
                        format!("{h:02}:00")
                    };
                    WeatherHourlyPoint {
                        hour_label,
                        temp_c: temps[i],
                        condition: conds[i].to_string(),
                        pop_pct: pops[i],
                        wind_kmh: winds[i],
                    }
                })
                .collect()
        };

        let make_daily = |specs: [(&str, i32, i32, u8, &str); 5]| -> Vec<WeatherDailyPoint> {
            specs
                .into_iter()
                .enumerate()
                .map(|(idx, (cond, high_c, low_c, pop_pct, advice))| {
                    let date = today + ChronoDuration::days(idx as i64);
                    let ymd = date.format("%Y-%m-%d").to_string();
                    let date_label = match idx {
                        0 => "今天".to_string(),
                        1 => "明天".to_string(),
                        _ => weekday_cn_label(date.weekday()),
                    };
                    WeatherDailyPoint {
                        date_label,
                        ymd,
                        condition: cond.to_string(),
                        high_c,
                        low_c,
                        pop_pct,
                        advice: advice.to_string(),
                    }
                })
                .collect()
        };

        match normalized {
            "苏州" => Self {
                city: "苏州".to_string(),
                region_label: "苏州 · 华东".to_string(),
                temp_c: 18,
                feels_like_c: 17,
                condition: "晴间多云".to_string(),
                high_c: 20,
                low_c: 13,
                humidity_pct: 62,
                wind_label: "东南风 2级".to_string(),
                aqi: 32,
                aqi_label: "优 32".to_string(),
                uv_label: "中等".to_string(),
                umbrella_needed: false,
                commute_advice: "早晚薄外套、中午单衣，不用带伞".to_string(),
                backscreen_badge: "苏州 18° 晴间多云".to_string(),
                updated_label: "刚刚更新".to_string(),
                hourly: make_hourly(
                    [18, 19, 20, 19, 17, 16],
                    ["晴间多云", "晴", "晴", "多云", "多云", "多云"],
                    [10, 10, 15, 20, 20, 15],
                    [11, 12, 14, 12, 10, 9],
                ),
                daily: make_daily([
                    ("晴间多云", 20, 13, 15, "早晚薄外套、中午单衣，不用带伞"),
                    ("多云", 21, 14, 20, "早晚薄外套、中午单衣，不用带伞"),
                    ("小雨", 18, 12, 75, "穿薄外套或卫衣，记得带伞"),
                    ("多云", 19, 11, 20, "穿薄外套或卫衣，不用带伞"),
                    ("晴", 21, 12, 10, "早晚薄外套、中午单衣，不用带伞"),
                ]),
                from_live_api: false,
            },
            "杭州" => Self {
                city: "杭州".to_string(),
                region_label: "杭州 · 华东".to_string(),
                temp_c: 17,
                feels_like_c: 16,
                condition: "小雨".to_string(),
                high_c: 19,
                low_c: 13,
                humidity_pct: 82,
                wind_label: "东北风 3级".to_string(),
                aqi: 28,
                aqi_label: "优 28".to_string(),
                uv_label: "弱".to_string(),
                umbrella_needed: true,
                commute_advice: "有小雨穿薄外套，出门带伞".to_string(),
                backscreen_badge: "杭州 17° 小雨 · 带伞".to_string(),
                updated_label: "刚刚更新".to_string(),
                hourly: make_hourly(
                    [17, 17, 18, 17, 16, 15],
                    ["小雨", "小雨", "阴", "小雨", "阴", "多云"],
                    [75, 80, 50, 65, 40, 30],
                    [16, 18, 15, 14, 12, 11],
                ),
                daily: make_daily([
                    ("小雨", 19, 13, 80, "穿薄外套或卫衣，记得带伞"),
                    ("阴转多云", 20, 13, 30, "早晚薄外套、中午单衣，不用带伞"),
                    ("多云", 22, 14, 15, "早晚薄外套、中午单衣，不用带伞"),
                    ("晴", 23, 14, 10, "早晚薄外套、中午单衣，不用带伞"),
                    ("多云", 21, 13, 20, "早晚薄外套、中午单衣，不用带伞"),
                ]),
                from_live_api: false,
            },
            "南京" => Self {
                city: "南京".to_string(),
                region_label: "南京 · 华东".to_string(),
                temp_c: 18,
                feels_like_c: 17,
                condition: "晴".to_string(),
                high_c: 21,
                low_c: 12,
                humidity_pct: 52,
                wind_label: "东风 2级".to_string(),
                aqi: 42,
                aqi_label: "优 42".to_string(),
                uv_label: "中等".to_string(),
                umbrella_needed: false,
                commute_advice: "早晚薄外套、中午单衣，不用带伞".to_string(),
                backscreen_badge: "南京 18° 晴".to_string(),
                updated_label: "刚刚更新".to_string(),
                hourly: make_hourly(
                    [18, 20, 21, 20, 17, 15],
                    ["晴", "晴", "晴", "多云", "多云", "晴"],
                    [5, 5, 10, 15, 10, 10],
                    [10, 12, 13, 11, 9, 8],
                ),
                daily: make_daily([
                    ("晴", 21, 12, 10, "早晚薄外套、中午单衣，不用带伞"),
                    ("多云", 20, 13, 20, "早晚薄外套、中午单衣，不用带伞"),
                    ("阴", 18, 11, 35, "穿薄外套或卫衣，不用带伞"),
                    ("晴", 21, 11, 10, "早晚薄外套、中午单衣，不用带伞"),
                    ("晴间多云", 22, 12, 15, "早晚薄外套、中午单衣，不用带伞"),
                ]),
                from_live_api: false,
            },
            "北京" => Self {
                city: "北京".to_string(),
                region_label: "北京 · 华北".to_string(),
                temp_c: 14,
                feels_like_c: 12,
                condition: "晴".to_string(),
                high_c: 17,
                low_c: 6,
                humidity_pct: 35,
                wind_label: "西北风 3级".to_string(),
                aqi: 45,
                aqi_label: "优 45".to_string(),
                uv_label: "中等".to_string(),
                umbrella_needed: false,
                commute_advice: "穿薄外套或卫衣，不用带伞".to_string(),
                backscreen_badge: "北京 14° 晴".to_string(),
                updated_label: "刚刚更新".to_string(),
                hourly: make_hourly(
                    [14, 16, 17, 15, 12, 10],
                    ["晴", "晴", "晴", "晴", "多云", "晴"],
                    [5, 5, 5, 10, 10, 5],
                    [18, 20, 19, 15, 12, 11],
                ),
                daily: make_daily([
                    ("晴", 17, 6, 5, "穿薄外套或卫衣，不用带伞"),
                    ("晴间多云", 18, 7, 10, "穿薄外套或卫衣，不用带伞"),
                    ("多云", 15, 5, 15, "穿薄外套或卫衣，不用带伞"),
                    ("晴", 16, 4, 5, "穿薄外套或卫衣，不用带伞"),
                    ("晴", 18, 6, 5, "穿薄外套或卫衣，不用带伞"),
                ]),
                from_live_api: false,
            },
            "深圳" => Self {
                city: "深圳".to_string(),
                region_label: "深圳 · 华南".to_string(),
                temp_c: 26,
                feels_like_c: 27,
                condition: "多云".to_string(),
                high_c: 28,
                low_c: 22,
                humidity_pct: 74,
                wind_label: "南风 2级".to_string(),
                aqi: 25,
                aqi_label: "优 25".to_string(),
                uv_label: "较强".to_string(),
                umbrella_needed: false,
                commute_advice: "穿短袖或薄长袖，不用带伞".to_string(),
                backscreen_badge: "深圳 26° 多云".to_string(),
                updated_label: "刚刚更新".to_string(),
                hourly: make_hourly(
                    [26, 27, 28, 27, 25, 24],
                    ["多云", "多云", "晴间多云", "多云", "阵雨", "多云"],
                    [20, 25, 20, 30, 45, 25],
                    [12, 14, 15, 13, 11, 10],
                ),
                daily: make_daily([
                    ("多云", 28, 22, 25, "穿短袖或薄长袖，不用带伞"),
                    ("阵雨转多云", 27, 22, 60, "穿短袖或薄长袖，记得带伞"),
                    ("多云", 28, 23, 20, "穿短袖或薄长袖，不用带伞"),
                    ("晴间多云", 29, 23, 15, "穿短袖或薄长袖，不用带伞"),
                    ("多云", 28, 22, 20, "穿短袖或薄长袖，不用带伞"),
                ]),
                from_live_api: false,
            },
            "上海 · 奉贤区" | "奉贤区" | "手机定位" => Self {
                city: "上海 · 奉贤区".to_string(),
                region_label: "手机定位 · 系统天气 MCP".to_string(),
                temp_c: 17,
                feels_like_c: 16,
                condition: "阴".to_string(),
                high_c: 18,
                low_c: 13,
                humidity_pct: 74,
                wind_label: "西北风 3级".to_string(),
                aqi: 23,
                aqi_label: "优 23".to_string(),
                uv_label: "最弱".to_string(),
                umbrella_needed: false,
                commute_advice: "穿薄外套，不用带伞（感冒易发）".to_string(),
                backscreen_badge: "上海 · 奉贤区 17° 阴".to_string(),
                updated_label: "手机天气 MCP 刚刚同步".to_string(),
                hourly: make_hourly(
                    [17, 18, 18, 17, 16, 15],
                    ["阴", "多云", "多云", "多云", "阴", "多云"],
                    [0, 0, 10, 10, 15, 10],
                    [15, 15, 15, 12, 10, 10],
                ),
                daily: make_daily([
                    ("多云", 18, 13, 0, "穿薄外套，不用带伞"),
                    ("多云", 21, 13, 0, "比今天暖 3°C，早晚薄外套、中午单衣，不用带伞"),
                    ("小雨", 22, 16, 65, "穿长袖或薄卫衣，记得带伞"),
                    ("小雨", 19, 13, 60, "穿薄外套或卫衣，记得带伞"),
                    ("多云", 17, 11, 10, "穿薄外套或卫衣，不用带伞"),
                ]),
                from_live_api: false,
            },
            _ => {
                let is_shanghai = normalized == "上海";
                let city_name = normalized.to_string();
                let region_label = if is_shanghai {
                    "上海 · 华东".to_string()
                } else {
                    format!("{city_name} · 华东")
                };
                Self {
                    city: city_name.clone(),
                    region_label,
                    temp_c: 19,
                    feels_like_c: 18,
                    condition: "多云".to_string(),
                    high_c: 21,
                    low_c: 14,
                    humidity_pct: 58,
                    wind_label: "东风 2级".to_string(),
                    aqi: 36,
                    aqi_label: "优 36".to_string(),
                    uv_label: "中等".to_string(),
                    umbrella_needed: false,
                    commute_advice: "早晚薄外套、中午单衣，不用带伞".to_string(),
                    backscreen_badge: format!("{city_name} 19° 多云"),
                    updated_label: "刚刚更新".to_string(),
                    hourly: make_hourly(
                        [19, 20, 21, 20, 18, 17],
                        ["多云", "多云", "晴", "多云", "多云", "阴"],
                        [15, 15, 10, 20, 25, 20],
                        [12, 14, 15, 13, 11, 10],
                    ),
                    daily: make_daily([
                        ("多云", 21, 14, 15, "早晚薄外套、中午单衣，不用带伞"),
                        ("晴间多云", 22, 15, 10, "早晚薄外套、中午单衣，不用带伞"),
                        ("小雨", 19, 14, 70, "穿薄外套或卫衣，记得带伞"),
                        ("多云", 20, 13, 20, "早晚薄外套、中午单衣，不用带伞"),
                        ("晴", 22, 14, 10, "早晚薄外套、中午单衣，不用带伞"),
                    ]),
                    from_live_api: false,
                }
            }
        }
    }

    /// Concise headline badge, e.g. `"上海 19°C 多云"`.
    pub fn headline_badge(&self) -> String {
        format!("{} {}°C {}", self.city, self.temp_c, self.condition)
    }

    /// Low-to-high temperature range label, e.g. `"14°~21°C"`.
    pub fn range_label(&self) -> String {
        format!("{}°~{}°C", self.low_c, self.high_c)
    }

    /// Returns tomorrow's `WeatherDailyPoint` (`daily[1]` when present, or a synthesized fallback).
    pub fn tomorrow_point(&self) -> WeatherDailyPoint {
        if let Some(point) = self.daily.get(1) {
            return point.clone();
        }
        let date = (Local::now().date_naive() + ChronoDuration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let pop_pct = if self.umbrella_needed { 60 } else { 10 };
        WeatherDailyPoint {
            date_label: "明天".to_string(),
            ymd: date,
            condition: self.condition.clone(),
            high_c: self.high_c,
            low_c: self.low_c,
            pop_pct,
            advice: synthesize_daily_advice(
                1,
                &self.condition,
                self.low_c,
                self.high_c,
                pop_pct,
                Some(self.high_c),
            ),
        }
    }

    /// Concise temperature delta label comparing tomorrow's high to today's high.
    pub fn tomorrow_temp_delta_label(&self) -> String {
        let diff = self.tomorrow_point().high_c - self.high_c;
        if diff >= 2 {
            format!("+{diff}°C")
        } else if diff <= -2 {
            format!("{diff}°C")
        } else {
            "温差小".to_string()
        }
    }

    /// Returns whether an umbrella is recommended for tomorrow.
    pub fn tomorrow_umbrella_needed(&self) -> bool {
        let t = self.tomorrow_point();
        t.condition.contains('雨') || t.pop_pct >= 40
    }

    /// Concise headline for tomorrow's weather, e.g. `"明天 多云 13°~21°C"`.
    pub fn tomorrow_headline(&self) -> String {
        let t = self.tomorrow_point();
        format!("明天 {} {}°~{}°C", t.condition, t.low_c, t.high_c)
    }

    /// Display-only advice: reuse an already generated in-memory result or the
    /// snapshot's local forecast advice. Rendering must never create a phone
    /// session, wait for a model, read disk, or initiate a device connection.
    pub fn tomorrow_advice(&self) -> String {
        self.tomorrow_advice_with_source().0
    }

    /// Return the advice and its provenance from the same in-memory snapshot.
    /// A local forecast fallback must not be presented as a model response.
    pub fn tomorrow_advice_with_source(&self) -> (String, &'static str) {
        if !cfg!(test) {
            let today = Local::now().format("%Y-%m-%d").to_string();
            if let Ok(cache) = DAILY_ADVICE_MEM_CACHE.lock() {
                if let Some(advice) = cache
                    .as_ref()
                    .filter(|advice| is_daily_advice_valid_for_today(advice, &today, &self.city))
                {
                    return (
                        advice.tomorrow_advice.clone(),
                        if advice.generated_by_amoo { "Amoo 天气建议" } else { "天气预报建议" },
                    );
                }
            }
        }
        let tomorrow = self.tomorrow_point();
        let advice = if !tomorrow.advice.trim().is_empty() {
            tomorrow.advice
        } else {
            synthesize_daily_advice(
                1,
                &tomorrow.condition,
                tomorrow.low_c,
                tomorrow.high_c,
                tomorrow.pop_pct,
                Some(self.high_c),
            )
        };
        (advice, "天气预报建议")
    }

    pub fn source_label(&self) -> &'static str {
        if !self.from_live_api {
            "天气待同步"
        } else if self.region_label.contains("系统天气 MCP") {
            "手机系统天气 MCP"
        } else {
            "Open-Meteo 在线天气"
        }
    }

    /// Concise schedule/reminder note for tomorrow's weather.
    pub fn build_tomorrow_schedule_note(&self) -> String {
        let t = self.tomorrow_point();
        format!(
            "【明日天气提醒 · {}】明天 {} {}°~{}°C · {}",
            self.city,
            t.condition,
            t.low_c,
            t.high_c,
            self.tomorrow_advice()
        )
    }

    /// Builds a natural, practical prompt that carries the weather data into Amoo or Home input.
    pub fn build_weather_context_prompt(
        &self,
        focus_slot: Option<&str>,
        schedule_titles: &[String],
    ) -> String {
        let mut out = format!(
            "[天气上下文 · {}] 当前 {} {}°C（体感 {}°C，今日 {}°~{}°C，湿度 {}%，{}，空气{}）。出行建议：{}。",
            self.city,
            self.condition,
            self.temp_c,
            self.feels_like_c,
            self.low_c,
            self.high_c,
            self.humidity_pct,
            self.wind_label,
            self.aqi_label,
            self.commute_advice.trim_end_matches('。')
        );

        if let Some(slot) = focus_slot.map(str::trim).filter(|s| !s.is_empty()) {
            out.push_str(&format!("重点关注时段：{slot}。"));
        }

        if !self.hourly.is_empty() {
            let hourly_preview: Vec<String> = self
                .hourly
                .iter()
                .take(4)
                .map(|h| {
                    if h.pop_pct >= 40 {
                        format!(
                            "{} {} {}°C(降水{}%)",
                            h.hour_label, h.condition, h.temp_c, h.pop_pct
                        )
                    } else {
                        format!("{} {} {}°C", h.hour_label, h.condition, h.temp_c)
                    }
                })
                .collect();
            out.push_str(&format!("未来几小时趋势：{}。", hourly_preview.join(" → ")));
        }

        let t = self.tomorrow_point();
        out.push_str(&format!(
            "明日预报与建议：明天 {} {}°~{}°C（{}，{}）。",
            t.condition,
            t.low_c,
            t.high_c,
            self.tomorrow_temp_delta_label(),
            self.tomorrow_advice()
        ));

        let cleaned_schedules: Vec<&str> = schedule_titles
            .iter()
            .map(String::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();

        if !cleaned_schedules.is_empty() {
            let joined_schedules = cleaned_schedules.join("；");
            out.push_str(&format!(
                "今日关联日程：{joined_schedules}。请结合上述天气和我的日程，给出具体的出行、穿衣与时间安排建议。"
            ));
        } else {
            out.push_str("请结合当前天气，帮我规划今天的出行、穿衣或日程安排。");
        }

        out
    }

    /// Returns a concise schedule title/note carrying weather, e.g. `"【上海 19°C 多云】出行与日程提醒（14°~21°C）"`.
    pub fn build_weather_schedule_note(&self, focus_slot: Option<&str>) -> String {
        if let Some(slot) = focus_slot.map(str::trim).filter(|s| !s.is_empty()) {
            format!(
                "【{} {}°C {}】{} · 出行与日程提醒（{}）",
                self.city,
                self.temp_c,
                self.condition,
                slot,
                self.range_label()
            )
        } else {
            format!(
                "【{} {}°C {}】出行与日程提醒（{}）",
                self.city,
                self.temp_c,
                self.condition,
                self.range_label()
            )
        }
    }

    /// Scans notification/SMS strings for rain/weather warnings and updates `umbrella_needed` / `commute_advice`.
    pub fn enrich_with_phone_cues(&mut self, cues: &[String]) {
        let rain_keywords = ["降雨", "大雨", "暴雨", "雷阵雨", "下雨", "小雨", "中雨", "阵雨", "带伞"];
        let wind_cool_keywords = ["大风", "降温", "寒潮"];

        let has_rain_cue = cues
            .iter()
            .any(|c| rain_keywords.iter().any(|kw| c.contains(kw)));
        let has_wind_or_cool_cue = cues
            .iter()
            .any(|c| wind_cool_keywords.iter().any(|kw| c.contains(kw)));

        if has_rain_cue {
            self.umbrella_needed = true;
            if self.commute_advice.contains("不用带伞") {
                self.commute_advice = self
                    .commute_advice
                    .replace("不用带伞", "有雨记得带伞");
            } else if self.commute_advice.contains("无雨无需带伞") {
                self.commute_advice = self
                    .commute_advice
                    .replace("今日无雨无需带伞", "有雨记得带伞")
                    .replace("无雨无需带伞", "有雨记得带伞");
            } else if !self.commute_advice.contains("带伞") {
                self.commute_advice = format!(
                    "{}，有雨记得带伞",
                    self.commute_advice.trim_end_matches('。')
                );
            }
            if !self.backscreen_badge.contains("带伞") {
                self.backscreen_badge = format!("{} · 带伞", self.backscreen_badge);
            }
        }

        if has_wind_or_cool_cue && !self.commute_advice.contains("防风保暖") {
            self.commute_advice = format!(
                "{}，注意防风保暖",
                self.commute_advice.trim_end_matches('。')
            );
        }
    }
}

fn weekday_cn_label(weekday: chrono::Weekday) -> String {
    match weekday {
        chrono::Weekday::Mon => "周一".to_string(),
        chrono::Weekday::Tue => "周二".to_string(),
        chrono::Weekday::Wed => "周三".to_string(),
        chrono::Weekday::Thu => "周四".to_string(),
        chrono::Weekday::Fri => "周五".to_string(),
        chrono::Weekday::Sat => "周六".to_string(),
        chrono::Weekday::Sun => "周日".to_string(),
    }
}

fn preferred_city_config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("stepx-companion")
            .join("weather_city.txt"),
    )
}

fn weather_snapshot_cache_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".cache")
            .join("stepx-companion")
            .join("weather_snapshot.json"),
    )
}

fn weather_daily_advice_cache_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".cache")
            .join("stepx-companion")
            .join("weather_daily_advice.json"),
    )
}

/// Returns `true` when `cached` was generated on `today_ymd` for `city` and has non-empty advice.
/// Once generated for `today_ymd`, the daily advice stays locked for the entire calendar day
/// and only updates on the next calendar day (`cached.date_ymd != today_ymd`).
pub fn is_daily_advice_valid_for_today(
    cached: &CachedDailyWeatherAdvice,
    today_ymd: &str,
    city: &str,
) -> bool {
    cached.date_ymd == today_ymd
        && cached.city == city
        && !cached.tomorrow_advice.trim().is_empty()
}

fn clean_amoo_weather_advice_line(raw: &str) -> String {
    let first_line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let cleaned = first_line
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '“' | '”' | '‘' | '’' | '`' | '。' | '！' | '!' | ' ' | '\t'
            )
        })
        .trim();
    cleaned.chars().take(28).collect()
}

fn discover_online_phone_transport() -> Option<(crate::adb::Adb, String)> {
    let adb = crate::adb::Adb::discover().ok()?;
    let raw_devices = adb
        .run_timeout(&["devices", "-l"], Duration::from_secs(2))
        .ok()?;
    let transport = crate::device::parse_devices(&raw_devices)
        .into_iter()
        .find(|d| d.state == "device")?;
    Some((adb, transport.id))
}

/// Reads `/sdcard/Android/data/com.stepx.companion.app/files/weather_daily_advice.json` from the phone
/// and returns it if it is valid for `(today_ymd, city)`.
pub fn read_phone_daily_weather_advice(
    adb: &crate::adb::Adb,
    serial: &str,
    today_ymd: &str,
    city: &str,
) -> Option<CachedDailyWeatherAdvice> {
    let cmd = format!("cat '{PHONE_DAILY_ADVICE_CACHE_PATH}' 2>/dev/null");
    let raw = adb
        .run_timeout(&["-s", serial, "shell", &cmd], Duration::from_secs(2))
        .ok()?;
    let parsed: CachedDailyWeatherAdvice = serde_json::from_str(raw.trim()).ok()?;
    if is_daily_advice_valid_for_today(&parsed, today_ymd, city) {
        Some(parsed)
    } else {
        None
    }
}

/// Writes `advice` to `/sdcard/Android/data/com.stepx.companion.app/files/weather_daily_advice.json` on the phone.
pub fn write_phone_daily_weather_advice(
    adb: &crate::adb::Adb,
    serial: &str,
    advice: &CachedDailyWeatherAdvice,
) -> bool {
    let Ok(serialized) = serde_json::to_string(advice) else {
        return false;
    };
    let escaped = serialized.replace('\'', "'\\''");
    let cmd = format!(
        "mkdir -p '/sdcard/Android/data/com.stepx.companion.app/files' && printf '%s' '{escaped}' > '{PHONE_DAILY_ADVICE_CACHE_PATH}'"
    );
    adb.run_timeout(&["-s", serial, "shell", &cmd], Duration::from_secs(2))
        .is_ok()
}

fn save_desktop_daily_advice_cache(advice: &CachedDailyWeatherAdvice) {
    if let Some(path) = weather_daily_advice_cache_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(serialized) = serde_json::to_string_pretty(advice) {
            let _ = std::fs::write(path, serialized);
        }
    }
}

fn call_amoo_for_tomorrow_advice(
    city: &str,
    today_cond: &str,
    today_low: i32,
    today_high: i32,
    tmr_cond: &str,
    tmr_low: i32,
    tmr_high: i32,
    tmr_pop: u8,
) -> Option<String> {
    if let Ok(mock) = std::env::var("STEPX_WEATHER_MOCK_AMOO_ADVICE") {
        let cleaned = clean_amoo_weather_advice_line(&mock);
        if !cleaned.is_empty() {
            return Some(cleaned);
        }
    }
    if cfg!(test) {
        return None;
    }

    let (adb, transport_id) = discover_online_phone_transport()?;
    let device = crate::device::Device::inspect(&adb, &transport_id).ok()?;
    let mut bus = crate::amoo::AgentBus::connect(&adb, &device).ok()?;
    let session_key = bus.create_session().ok()?;

    let account = crate::amoo_usage::account_scope_fresh(&adb, &device)
        .ok()
        .flatten()
        .or_else(|| crate::amoo_usage::account_scope(&adb, &device).ok().flatten())
        .unwrap_or_else(|| "weather-ephemeral-scope".to_string());
    let _ = crate::inference_sessions::register(
        &crate::inference_sessions::default_context_root(),
        &device.serial,
        &account,
        &session_key,
    );

    let prompt = format!(
        "今天{city}{today_cond} {today_low}~{today_high}°C，明天{tmr_cond} {tmr_low}~{tmr_high}°C（降水概率{tmr_pop}%）。用一句简短家常话给明天穿衣和带伞建议，22字以内，不要标题、不要标点套话、不要AI感："
    );

    let mut reply_text: Option<String> = None;
    if let Ok(receipt) = bus.send_verbatim(&session_key, &prompt) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(300));
            let wait_rem = deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(Duration::from_secs(2));
            if wait_rem.is_zero() {
                break;
            }
            let Ok(history) = bus.call_with_timeout(
                "chat.history",
                serde_json::json!({"sessionId": session_key, "limit": 10}),
                wait_rem,
            ) else {
                break;
            };
            let Some(rows) = history["messages"].as_array() else {
                continue;
            };
            let Ok(messages) = crate::amoo::parse_messages(rows) else {
                continue;
            };
            let session_info = crate::openclaw::RealSessionInfo {
                key: session_key.clone(),
                messages,
                history_loaded: true,
                ..Default::default()
            };
            if receipt.finished(&session_info) {
                if let Some(msg) = session_info
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "assistant" && !m.text.trim().is_empty() && !m.is_error)
                {
                    let cleaned = clean_amoo_weather_advice_line(&msg.text);
                    if !cleaned.is_empty() {
                        reply_text = Some(cleaned);
                    }
                }
                break;
            }
        }
    }

    let _ = bus.call_with_timeout(
        "chat.abort",
        serde_json::json!({"sessionId": session_key, "sessionKey": session_key}),
        Duration::from_secs(2),
    );
    let _ = bus.call_with_timeout(
        "sessions.delete",
        serde_json::json!({"key": session_key}),
        Duration::from_secs(2),
    );

    reply_text
}

/// Resolves daily weather advice for `snapshot` on `today_ymd` using `mem_cache`,
/// desktop disk cache (`~/.cache/stepx-companion/weather_daily_advice.json`), and
/// phone on-device cache (`/sdcard/Android/data/com.stepx.companion.app/files/weather_daily_advice.json`),
/// calling Amoo at most once per calendar day (`date_ymd == today_ymd`).
pub fn resolve_daily_weather_advice_for_date(
    snapshot: &WeatherSnapshot,
    today_ymd: &str,
    mem_cache: &Mutex<Option<CachedDailyWeatherAdvice>>,
    use_disk_cache: bool,
) -> CachedDailyWeatherAdvice {
    // 1. Check in-memory cache first (0 ADB calls).
    if let Ok(guard) = mem_cache.lock() {
        if let Some(ref cached) = *guard {
            if is_daily_advice_valid_for_today(cached, today_ymd, &snapshot.city) {
                return cached.clone();
            }
        }
    }

    // 2. Check desktop local cache `~/.cache/stepx-companion/weather_daily_advice.json` (0 ADB calls).
    if use_disk_cache {
        if let Some(path) = weather_daily_advice_cache_path() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(parsed) = serde_json::from_str::<CachedDailyWeatherAdvice>(&raw) {
                    if is_daily_advice_valid_for_today(&parsed, today_ymd, &snapshot.city) {
                        if let Ok(mut guard) = mem_cache.lock() {
                            *guard = Some(parsed.clone());
                        }
                        WEATHER_CACHE_REVISION.fetch_add(1, Ordering::Relaxed);
                        return parsed;
                    }
                }
            }
        }
    }

    // 3. Before invoking AgentBus on the phone, check if `/sdcard/Android/data/com.stepx.companion.app/files/weather_daily_advice.json`
    //    on the phone already holds today's (`today_ymd`, `snapshot.city`) advice.
    let online_phone = if use_disk_cache && !cfg!(test) {
        discover_online_phone_transport()
    } else {
        None
    };
    if let Some((ref adb, ref serial)) = online_phone {
        if let Some(phone_cached) =
            read_phone_daily_weather_advice(adb, serial, today_ymd, &snapshot.city)
        {
            if let Ok(mut guard) = mem_cache.lock() {
                *guard = Some(phone_cached.clone());
            }
            WEATHER_CACHE_REVISION.fetch_add(1, Ordering::Relaxed);
            if use_disk_cache {
                save_desktop_daily_advice_cache(&phone_cached);
            }
            return phone_cached;
        }
    }

    // 4. Generate today's advice via Amoo (or concise local synthesis when offline).
    let t = snapshot.tomorrow_point();
    let (tomorrow_advice, generated_by_amoo) = match call_amoo_for_tomorrow_advice(
        &snapshot.city,
        &snapshot.condition,
        snapshot.low_c,
        snapshot.high_c,
        &t.condition,
        t.low_c,
        t.high_c,
        t.pop_pct,
    ) {
        Some(amoo_reply) => (amoo_reply, true),
        None => {
            let fallback = if !t.advice.trim().is_empty() {
                t.advice.clone()
            } else {
                synthesize_daily_advice(
                    1,
                    &t.condition,
                    t.low_c,
                    t.high_c,
                    t.pop_pct,
                    Some(snapshot.high_c),
                )
            };
            (fallback, false)
        }
    };

    let resolved = CachedDailyWeatherAdvice {
        date_ymd: today_ymd.to_string(),
        city: snapshot.city.clone(),
        today_short: snapshot.commute_advice.clone(),
        tomorrow_advice,
        generated_by_amoo,
    };

    if let Ok(mut guard) = mem_cache.lock() {
        *guard = Some(resolved.clone());
    }
    WEATHER_CACHE_REVISION.fetch_add(1, Ordering::Relaxed);

    if use_disk_cache {
        save_desktop_daily_advice_cache(&resolved);
        if let Some((ref adb, ref serial)) = online_phone {
            let _ = write_phone_daily_weather_advice(adb, serial, &resolved);
        }
    }

    resolved
}

/// Resolves once-per-day Amoo weather advice for `snapshot`.
/// Cached for the entire local calendar day (`YYYY-MM-DD`) in memory, `~/.cache/stepx-companion/weather_daily_advice.json`,
/// and `/sdcard/Android/data/com.stepx.companion.app/files/weather_daily_advice.json` on the phone.
pub fn resolve_daily_weather_advice(snapshot: &WeatherSnapshot) -> CachedDailyWeatherAdvice {
    let today_ymd = Local::now().format("%Y-%m-%d").to_string();
    if cfg!(test) {
        let local_cache = Mutex::new(None);
        return resolve_daily_weather_advice_for_date(snapshot, &today_ymd, &local_cache, false);
    }
    resolve_daily_weather_advice_for_date(snapshot, &today_ymd, &DAILY_ADVICE_MEM_CACHE, true)
}

/// Alias for `resolve_daily_weather_advice` syncing desktop and on-device daily Amoo tomorrow advice.
pub fn resolve_daily_amoo_tomorrow_advice(snapshot: &WeatherSnapshot) -> CachedDailyWeatherAdvice {
    resolve_daily_weather_advice(snapshot)
}

/// Reads `~/.config/stepx-companion/weather_city.txt`, defaulting to `"上海"`.
pub fn load_preferred_city() -> String {
    load_preferred_city_with(&PREFERRED_CITY_MEM, || {
        if cfg!(test) {
            return None;
        }
        preferred_city_config_path().and_then(|path| std::fs::read_to_string(path).ok())
    })
}

fn load_preferred_city_with(
    cache: &Mutex<Option<String>>,
    read_saved: impl FnOnce() -> Option<String>,
) -> String {
    if let Ok(guard) = cache.lock() {
        if let Some(ref c) = *guard {
            if !c.trim().is_empty() {
                return c.trim().to_string();
            }
        }
    }
    // Read outside the lock. A user may select another city while disk I/O is
    // pending; the newer in-memory choice wins over both disk data and fallback.
    let loaded = read_saved()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "上海".to_string());
    if let Ok(mut guard) = cache.lock() {
        if guard.is_none() {
            *guard = Some(loaded.clone());
        }
        return guard.as_deref().map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&loaded).to_string();
    }
    loaded
}

/// Writes preferred city to `~/.config/stepx-companion/weather_city.txt` unless `cfg!(test)`.
pub fn save_preferred_city(city: &str) {
    let trimmed = city.trim();
    let normalized = if trimmed.is_empty() { "上海" } else { trimmed };
    if let Ok(mut guard) = PREFERRED_CITY_MEM.lock() {
        *guard = Some(normalized.to_string());
        WEATHER_CITY_REVISION.fetch_add(1, Ordering::Relaxed);
    }
    WEATHER_CACHE_REVISION.fetch_add(1, Ordering::Relaxed);
    if cfg!(test) {
        return;
    }
    if let Some(path) = preferred_city_config_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, normalized);
    }
}

/// A pure in-memory read for Home report construction. Live refresh belongs to
/// existing background/manual jobs, which publish WEATHER_CACHE_REVISION.
/// None means the current city has no published snapshot yet; do not present
/// synthesized temperatures as current weather while waiting for that job.
pub fn cached_weather_snapshot() -> Option<WeatherSnapshot> {
    let preferred = PREFERRED_CITY_MEM.lock().ok().and_then(|c| c.clone());
    let cached = SNAPSHOT_MEM_CACHE.lock().ok().and_then(|c| c.clone())?;
    if !cached.fresh_at(Instant::now()) {
        return None;
    }
    let snapshot = cached.snapshot;
    if preferred
        .as_deref()
        .is_none_or(|city| weather_city_matches(&snapshot, city))
    {
        Some(snapshot)
    } else {
        None
    }
}

fn weather_city_matches(snapshot: &WeatherSnapshot, city: &str) -> bool {
    snapshot.city == city
        || (city == "上海" && snapshot.city.starts_with("上海"))
        || (matches!(city, "上海 · 奉贤区" | "奉贤区") && snapshot.city.contains("奉贤区"))
        || (city == "手机定位" && snapshot.region_label.contains("系统天气 MCP"))
}

/// Immediate, city-specific cache lookup for selection actions; never reads
/// preferences from disk or starts a phone/network request.
pub fn cached_weather_snapshot_for_city(city: &str) -> Option<WeatherSnapshot> {
    cached_weather_snapshot_for_city_and_device(city, None)
}

/// Only an explicitly matched device may reuse a phone weather snapshot.
/// City forecasts from the online provider are independent of the phone.
pub fn cached_weather_snapshot_for_city_and_device(
    city: &str,
    device: Option<&crate::device::Device>,
) -> Option<WeatherSnapshot> {
    let key = weather_device_key(device);
    SNAPSHOT_MEM_CACHE.lock().ok().and_then(|cache| {
        cache.as_ref().and_then(|cached|
            cached.for_city_and_device(city, key.as_deref(), Instant::now()))
    })
}

fn weather_device_key(device: Option<&crate::device::Device>) -> Option<String> {
    let device = device?;
    let identity = if !device.serial.trim().is_empty() {
        device.serial.trim()
    } else {
        device.transport.trim()
    };
    (!identity.is_empty()).then(|| identity.to_string())
}

/// A wireless/USB route change for the same identified phone is acceptable;
/// another phone or a disconnect must discard the pending weather response.
pub fn weather_request_matches_device(
    requested: Option<&crate::device::Device>,
    current: Option<&crate::device::Device>,
) -> bool {
    match (requested, current) {
        (None, None) => true,
        (Some(_), Some(_)) => {
            let requested_key = weather_device_key(requested);
            requested_key.is_some() && requested_key == weather_device_key(current)
        }
        _ => false,
    }
}

/// Invalidate only an incompatible phone-location display cache when the
/// selected phone changes. City forecasts and per-phone fetch caches survive.
/// This function is memory-only and safe for the connection lifecycle tick.
pub fn discard_weather_snapshot_for_other_device(device: Option<&crate::device::Device>) -> bool {
    let current_key = weather_device_key(device);
    let Ok(mut cache) = SNAPSHOT_MEM_CACHE.lock() else { return false; };
    let incompatible = cache.as_ref().is_some_and(|cached| {
        cached.device_key.is_some() && cached.device_key != current_key
    });
    if incompatible {
        *cache = None;
        WEATHER_CACHE_REVISION.fetch_add(1, Ordering::Relaxed);
    }
    incompatible
}

/// Publish only after the caller has checked its captured device selection.
/// This is memory-only so the UI callback never performs disk or phone I/O.
pub fn publish_weather_snapshot_for_device_selection(
    snapshot: &WeatherSnapshot,
    city_revision: u64,
    device: Option<&crate::device::Device>,
) -> Result<(), String> {
    let _preferred = PREFERRED_CITY_MEM
        .lock()
        .map_err(|_| "天气城市缓存暂不可用".to_string())?;
    if WEATHER_CITY_REVISION.load(Ordering::Relaxed) != city_revision {
        return Err("天气城市已变更，忽略过期结果".into());
    }
    let device_key = if snapshot.region_label.contains("系统天气 MCP") {
        Some(weather_device_key(device)
            .ok_or_else(|| "手机天气缺少对应设备，忽略未绑定结果".to_string())?)
    } else {
        None
    };
    let mut cache = SNAPSHOT_MEM_CACHE
        .lock()
        .map_err(|_| "天气缓存暂不可用".to_string())?;
    *cache = Some(CachedWeatherSnapshot {
        snapshot: snapshot.clone(), stored_at: Instant::now(), device_key,
    });
    WEATHER_CACHE_REVISION.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

fn publish_live_weather_snapshot(snapshot: &WeatherSnapshot, city_revision: u64) -> Result<(), String> {
    publish_weather_snapshot_for_device_selection(snapshot, city_revision, None)
}

/// Legacy city forecast lookup with a strict two-hour memory/disk lifetime.
/// No device selection is available here, so unbound phone snapshots are ignored.
/// Device-aware UI callers use the explicit phone fetch and guarded publication APIs.
pub fn load_or_default_weather(city: Option<&str>) -> WeatherSnapshot {
    let explicit_city = city.map(str::trim).filter(|s| !s.is_empty());
    let target_city = match explicit_city {
        Some(c) => c.to_string(),
        None => load_preferred_city(),
    };

    // Unbound legacy callers may reuse online city data, never a different
    // phone's location. The memory cache has the same age bound as disk.
    if let Some(cached) = cached_weather_snapshot_for_city(&target_city) {
        return cached;
    }

    if !cfg!(test) {
        if let Some(path) = weather_snapshot_cache_path() {
            let age = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|mtime| SystemTime::now().duration_since(mtime).ok());
            if let Some(age) = age.filter(|age| *age < WEATHER_DISPLAY_CACHE_TTL) {
                if let Ok(raw) = std::fs::read_to_string(&path) {
                    if let Ok(parsed) = serde_json::from_str::<WeatherSnapshot>(&raw) {
                        // Older disk files have no hardware identity. Treat
                        // phone snapshots as unbound instead of guessing.
                        let matches_request = !parsed.region_label.contains("系统天气 MCP")
                            && weather_city_matches(&parsed, &target_city);
                        if matches_request {
                            if let Ok(mut guard) = SNAPSHOT_MEM_CACHE.lock() {
                                *guard = Some(CachedWeatherSnapshot {
                                    snapshot: parsed.clone(),
                                    stored_at: Instant::now().checked_sub(age).unwrap_or_else(Instant::now),
                                    device_key: None,
                                });
                            }
                            WEATHER_CACHE_REVISION.fetch_add(1, Ordering::Relaxed);
                            return parsed;
                        }
                    }
                }
            }
        }

        if explicit_city.is_none()
            || matches!(
                explicit_city,
                Some("上海" | "上海 · 奉贤区" | "奉贤区" | "手机定位")
            )
        {
            if let Ok(live) = fetch_live_weather_snapshot(&target_city) {
                return live;
            }
            return WeatherSnapshot::default_for_city(&target_city);
        }
    }

    WeatherSnapshot::default_for_city(&target_city)
}

/// Maps city name to `(latitude, longitude, region_label)`.
pub fn city_coordinates(city: &str) -> (f64, f64, String) {
    match city.trim() {
        "上海" | "上海 · 奉贤区" | "奉贤区" => {
            (30.9646, 121.5024, "上海 · 奉贤区".to_string())
        }
        "苏州" => (31.2989, 120.5853, "苏州 · 华东".to_string()),
        "杭州" => (30.2741, 120.1551, "杭州 · 华东".to_string()),
        "南京" => (32.0603, 118.7969, "南京 · 华东".to_string()),
        "北京" => (39.9042, 116.4074, "北京 · 华北".to_string()),
        "深圳" => (22.5431, 114.0579, "深圳 · 华南".to_string()),
        other => {
            let name = if other.is_empty() { "上海" } else { other };
            (30.9646, 121.5024, format!("{name} · 华东"))
        }
    }
}

/// Maps WMO weather code to concise Chinese condition label.
pub fn wmo_code_to_condition(code: i64) -> &'static str {
    match code {
        0 => "晴",
        1 | 2 => "多云",
        3 => "阴",
        45 | 48 => "雾",
        51..=63 | 80..=81 => "小雨",
        65..=67 | 82 => "大雨",
        71..=77 | 85..=86 => "小雪",
        95..=99 => "雷阵雨",
        _ => "多云",
    }
}

fn is_rainy_wmo_code(code: i64) -> bool {
    matches!(code, 51..=67 | 80..=82 | 95..=99)
}

fn wind_speed_kmh_to_level(wind_kmh: f64) -> u8 {
    if wind_kmh < 6.0 {
        1
    } else if wind_kmh < 12.0 {
        2
    } else if wind_kmh < 20.0 {
        3
    } else if wind_kmh < 29.0 {
        4
    } else if wind_kmh < 39.0 {
        5
    } else {
        6
    }
}

/// Synthesizes concise, natural Chinese daily weather advice covering temperature change vs today,
/// clothing guidance, and rain / umbrella recommendation without filler copy.
pub fn synthesize_daily_advice(
    idx: usize,
    cond: &str,
    low_c: i32,
    high_c: i32,
    pop_pct: u8,
    today_high_c: Option<i32>,
) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(3);

    if idx == 1 {
        if let Some(th) = today_high_c {
            let diff = high_c - th;
            if diff >= 3 {
                parts.push(format!("比今天暖 {diff}°C"));
            } else if diff <= -3 {
                parts.push(format!("比今天冷 {}°C", -diff));
            }
        }
    }

    let cloth_part = if high_c >= 26 {
        "穿短袖或薄长袖".to_string()
    } else if high_c >= 20 && high_c - low_c >= 7 {
        "早晚薄外套、中午单衣".to_string()
    } else if high_c >= 20 {
        "穿长袖或薄卫衣".to_string()
    } else if high_c >= 15 {
        "穿薄外套或卫衣".to_string()
    } else if high_c >= 10 {
        "穿风衣或毛衣".to_string()
    } else {
        "穿厚外套保暖".to_string()
    };
    parts.push(cloth_part);

    let rain_part = if cond.contains('雨') || pop_pct >= 40 {
        "记得带伞".to_string()
    } else {
        "不用带伞".to_string()
    };
    parts.push(rain_part);

    parts.join("，")
}

/// Parses Open-Meteo forecast JSON into a `WeatherSnapshot`.
pub fn parse_open_meteo_response(
    city: &str,
    region_label: &str,
    raw_json: &str,
) -> Result<WeatherSnapshot, String> {
    let val: serde_json::Value =
        serde_json::from_str(raw_json).map_err(|e| format!("Open-Meteo JSON 解析失败: {e}"))?;

    let current = val
        .get("current")
        .ok_or_else(|| "Open-Meteo 响应缺少 current 字段".to_string())?;

    let fallback = WeatherSnapshot::default_for_city(city);

    let temp_c = current
        .get("temperature_2m")
        .and_then(|v| v.as_f64())
        .map(|f| f.round() as i32)
        .unwrap_or(fallback.temp_c);

    let feels_like_c = current
        .get("apparent_temperature")
        .and_then(|v| v.as_f64())
        .map(|f| f.round() as i32)
        .unwrap_or(temp_c);

    let humidity_pct = current
        .get("relative_humidity_2m")
        .and_then(|v| v.as_f64())
        .map(|f| f.round().clamp(0.0, 100.0) as u8)
        .unwrap_or(fallback.humidity_pct);

    let weather_code = current
        .get("weather_code")
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    let condition = wmo_code_to_condition(weather_code).to_string();

    let wind_kmh = current
        .get("wind_speed_10m")
        .and_then(|v| v.as_f64())
        .unwrap_or(10.0);
    let wind_level = wind_speed_kmh_to_level(wind_kmh);
    let wind_label = format!("微风 {wind_level}级");

    // Parse daily array
    let mut daily: Vec<WeatherDailyPoint> = Vec::new();
    if let Some(daily_obj) = val.get("daily") {
        let times = daily_obj
            .get("time")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let codes = daily_obj
            .get("weather_code")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let highs = daily_obj
            .get("temperature_2m_max")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let lows = daily_obj
            .get("temperature_2m_min")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let pops = daily_obj
            .get("precipitation_probability_max")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut first_day_high_c: Option<i32> = None;
        for idx in 0..times.len().min(5) {
            let ymd = times[idx]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string();
            let date_label = match idx {
                0 => "今天".to_string(),
                1 => "明天".to_string(),
                _ => chrono::NaiveDate::parse_from_str(&ymd, "%Y-%m-%d")
                    .map(|d| weekday_cn_label(d.weekday()))
                    .unwrap_or_else(|_| format!("第{}天", idx + 1)),
            };
            let d_code = codes.get(idx).and_then(|v| v.as_i64()).unwrap_or(1);
            let d_cond = wmo_code_to_condition(d_code).to_string();
            let d_high = highs
                .get(idx)
                .and_then(|v| v.as_f64())
                .map(|f| f.round() as i32)
                .unwrap_or(temp_c + 2);
            if idx == 0 {
                first_day_high_c = Some(d_high);
            }
            let d_low = lows
                .get(idx)
                .and_then(|v| v.as_f64())
                .map(|f| f.round() as i32)
                .unwrap_or(temp_c - 4);
            let d_pop = pops
                .get(idx)
                .and_then(|v| v.as_f64())
                .map(|f| f.round().clamp(0.0, 100.0) as u8)
                .unwrap_or(0);
            let advice =
                synthesize_daily_advice(idx, &d_cond, d_low, d_high, d_pop, first_day_high_c);
            daily.push(WeatherDailyPoint {
                date_label,
                ymd,
                condition: d_cond,
                high_c: d_high,
                low_c: d_low,
                pop_pct: d_pop,
                advice,
            });
        }
    }
    if daily.is_empty() {
        daily = fallback.daily.clone();
    }

    let high_c = daily.first().map(|d| d.high_c).unwrap_or(fallback.high_c);
    let low_c = daily.first().map(|d| d.low_c).unwrap_or(fallback.low_c);

    // Parse hourly array
    let mut hourly: Vec<WeatherHourlyPoint> = Vec::new();
    let now_hour = Local::now().hour() as usize;
    if let Some(hourly_obj) = val.get("hourly") {
        let temps = hourly_obj
            .get("temperature_2m")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let pops = hourly_obj
            .get("precipitation_probability")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let codes = hourly_obj
            .get("weather_code")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let start_idx = if now_hour < temps.len() { now_hour } else { 0 };
        for offset in 0..6 {
            let idx = start_idx + offset;
            if idx >= temps.len() {
                break;
            }
            let h = (now_hour + offset) % 24;
            let hour_label = if offset == 0 {
                "现在".to_string()
            } else {
                format!("{h:02}:00")
            };
            let h_temp = temps
                .get(idx)
                .and_then(|v| v.as_f64())
                .map(|f| f.round() as i32)
                .unwrap_or(temp_c);
            let h_pop = pops
                .get(idx)
                .and_then(|v| v.as_f64())
                .map(|f| f.round().clamp(0.0, 100.0) as u8)
                .unwrap_or(0);
            let h_code = codes
                .get(idx)
                .and_then(|v| v.as_i64())
                .unwrap_or(weather_code);
            hourly.push(WeatherHourlyPoint {
                hour_label,
                temp_c: h_temp,
                condition: wmo_code_to_condition(h_code).to_string(),
                pop_pct: h_pop,
                wind_kmh: wind_kmh.round().clamp(1.0, 120.0) as u8,
            });
        }
    }
    if hourly.is_empty() {
        hourly = fallback.hourly.clone();
    }

    let max_hourly_pop = hourly.iter().map(|h| h.pop_pct).max().unwrap_or(0);
    let today_pop = daily.first().map(|d| d.pop_pct).unwrap_or(0);
    let umbrella_needed =
        is_rainy_wmo_code(weather_code) || max_hourly_pop >= 55 || today_pop >= 60;

    let commute_advice = synthesize_daily_advice(
        0,
        &condition,
        low_c,
        high_c,
        if umbrella_needed {
            60.max(max_hourly_pop.max(today_pop))
        } else {
            max_hourly_pop.max(today_pop)
        },
        None,
    );

    let city_trimmed = if city.trim().is_empty() {
        "上海"
    } else {
        city.trim()
    };
    let backscreen_badge = if umbrella_needed {
        format!("{city_trimmed} {temp_c}° {condition} · 带伞")
    } else {
        format!("{city_trimmed} {temp_c}° {condition}")
    };

    let updated_label = format!("{} 实时更新", Local::now().format("%H:%M"));

    Ok(WeatherSnapshot {
        city: city_trimmed.to_string(),
        region_label: region_label.to_string(),
        temp_c,
        feels_like_c,
        condition,
        high_c,
        low_c,
        humidity_pct,
        wind_label,
        aqi: fallback.aqi,
        aqi_label: fallback.aqi_label,
        uv_label: fallback.uv_label,
        umbrella_needed,
        commute_advice,
        backscreen_badge,
        updated_label,
        hourly,
        daily,
        from_live_api: true,
    })
}

fn json_first(val: &serde_json::Value) -> &serde_json::Value {
    match val {
        serde_json::Value::Array(arr) => arr.first().unwrap_or(&serde_json::Value::Null),
        other => other,
    }
}

fn json_str(val: Option<&serde_json::Value>) -> String {
    let Some(v) = val else {
        return String::new();
    };
    match json_first(v) {
        serde_json::Value::String(s) => s.trim().to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

fn json_i64(val: Option<&serde_json::Value>) -> Option<i64> {
    let v = json_first(val?);
    match v {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.round() as i64)),
        serde_json::Value::String(s) => s
            .trim()
            .parse::<i64>()
            .ok()
            .or_else(|| s.trim().parse::<f64>().ok().map(|f| f.round() as i64)),
        _ => None,
    }
}

/// Parses StepOS `com.stepos.weather` AppFunctions GenericDocument JSON output
/// (`cmd app_function execute-app-function --package com.stepos.weather --function com.stepos.weather.appfunctions.WeatherFunctions#get_current_location_weather`)
/// into a rich `WeatherSnapshot`.
pub fn parse_stepos_appfunction_weather_json(
    by_date_json: &str,
    by_hour_json: Option<&str>,
) -> Result<WeatherSnapshot, String> {
    let root: serde_json::Value = serde_json::from_str(by_date_json)
        .map_err(|e| format!("StepOS 天气 MCP JSON 解析失败: {e}"))?;

    let ret = root
        .get("androidAppfunctionsReturnValue")
        .map(json_first)
        .unwrap_or(&root);
    let data = ret
        .get("data")
        .map(json_first)
        .filter(|v| !v.is_null())
        .ok_or_else(|| "StepOS 天气 MCP 响应缺少 data 字段".to_string())?;

    let region_obj = data.get("region").map(json_first);
    let city_obj = data.get("city").map(json_first);

    let province_raw = region_obj
        .and_then(|r| {
            let s = json_str(r.get("province"));
            (!s.is_empty()).then_some(s)
        })
        .or_else(|| {
            city_obj.and_then(|c| {
                let s = json_str(c.get("province"));
                (!s.is_empty()).then_some(s)
            })
        })
        .unwrap_or_else(|| "上海市".to_string());

    let district_or_city = region_obj
        .and_then(|r| {
            let area = json_str(r.get("area"));
            if !area.is_empty() {
                Some(area)
            } else {
                let c = json_str(r.get("city"));
                (!c.is_empty()).then_some(c)
            }
        })
        .or_else(|| {
            city_obj.and_then(|c| {
                let s = json_str(c.get("name"));
                (!s.is_empty()).then_some(s)
            })
        })
        .unwrap_or_default();

    let province_short = province_raw
        .trim_end_matches('市')
        .trim_end_matches('省')
        .trim();
    let city_display = if !district_or_city.is_empty()
        && district_or_city != province_raw
        && district_or_city != province_short
    {
        format!("{province_short} · {district_or_city}")
    } else if !province_short.is_empty() {
        province_short.to_string()
    } else {
        "上海".to_string()
    };

    let region_label = "手机定位 · 系统天气 MCP".to_string();

    let realtime = data
        .get("realtime")
        .map(json_first)
        .filter(|v| !v.is_null())
        .ok_or_else(|| "StepOS 天气 MCP 缺少 realtime 字段".to_string())?;

    let temp_c = json_i64(realtime.get("temp")).unwrap_or(17) as i32;
    let feels_like_c = json_i64(realtime.get("sendibleTemp"))
        .map(|v| v as i32)
        .unwrap_or(temp_c);
    let condition = {
        let w = json_str(realtime.get("weather"));
        if w.is_empty() {
            "多云".to_string()
        } else {
            w
        }
    };
    let humidity_pct = json_i64(realtime.get("humidity"))
        .unwrap_or(70)
        .clamp(0, 100) as u8;

    let wind_dir = {
        let d = json_str(realtime.get("windDirection"));
        if d.is_empty() {
            "微风".to_string()
        } else {
            d
        }
    };
    let wind_scale = json_i64(realtime.get("windScale")).unwrap_or(2).max(1);
    let wind_label = format!("{wind_dir} {wind_scale}级");

    let air_obj = realtime.get("airQuality").map(json_first);
    let aqi = air_obj
        .and_then(|a| json_i64(a.get("aqi")))
        .unwrap_or(25)
        .clamp(0, 500) as u16;
    let aqi_quality = air_obj
        .map(|a| json_str(a.get("quality")))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "优".to_string());
    let aqi_label = format!("{aqi_quality} {aqi}");

    let uvi = json_i64(realtime.get("uvi")).unwrap_or(1);
    let mut uv_label = match uvi {
        0..=2 => "最弱".to_string(),
        3..=4 => "弱".to_string(),
        5..=6 => "中等".to_string(),
        7..=9 => "强".to_string(),
        _ => "极强".to_string(),
    };

    // Parse daily array + today's lifeIndexes
    let mut daily: Vec<WeatherDailyPoint> = Vec::new();
    let mut today_dress: Option<String> = None;
    let mut today_cold: Option<String> = None;
    if let Some(daily_arr) = data.get("daily").and_then(|v| v.as_array()) {
        let mut first_day_high_c: Option<i32> = None;
        for (idx, item) in daily_arr.iter().take(7).enumerate() {
            let ymd = {
                let fd = json_str(item.get("fullDate"));
                if !fd.is_empty() {
                    fd
                } else {
                    json_str(item.get("date"))
                }
            };
            let week_raw = json_str(item.get("week"));
            let date_label = match idx {
                0 if week_raw.is_empty() => "今天".to_string(),
                1 if week_raw.is_empty() => "明天".to_string(),
                _ if !week_raw.is_empty() => week_raw,
                _ => format!("第{}天", idx + 1),
            };
            let d_cond = {
                let wd = json_str(item.get("weatherDay"));
                if !wd.is_empty() {
                    wd
                } else {
                    let wn = json_str(item.get("weatherNight"));
                    if !wn.is_empty() {
                        wn
                    } else {
                        condition.clone()
                    }
                }
            };
            let d_high = json_i64(item.get("tempHigh"))
                .map(|v| v as i32)
                .unwrap_or(temp_c + 1);
            if idx == 0 {
                first_day_high_c = Some(d_high);
            }
            let d_low = json_i64(item.get("tempLow"))
                .map(|v| v as i32)
                .unwrap_or(temp_c - 4);
            let d_pop = json_i64(item.get("pop")).unwrap_or(0).clamp(0, 100) as u8;

            let mut day_dress: Option<String> = None;
            let mut day_cold: Option<String> = None;
            if let Some(indexes) = item.get("lifeIndexes").and_then(|v| v.as_array()) {
                for idx_item in indexes {
                    let idx_type = json_str(idx_item.get("type"));
                    let level = json_str(idx_item.get("level"));
                    let remind = json_str(idx_item.get("remind"));
                    let note_val = if !remind.is_empty() { remind } else { level.clone() };
                    match idx_type.as_str() {
                        "DRESS_LIVE_INDEX" if !note_val.is_empty() => {
                            let dress_clean = if note_val.starts_with('穿') {
                                note_val
                            } else {
                                format!("穿{note_val}")
                            };
                            day_dress = Some(dress_clean);
                        }
                        "ULTRAVIOLET_LIVE_INDEX" => {
                            if idx == 0 && !level.is_empty() {
                                uv_label = level;
                            }
                        }
                        "COLDR_LIVE_INDEX" if !note_val.is_empty() => {
                            day_cold = Some(note_val);
                        }
                        _ => {}
                    }
                }
            }
            if idx == 0 {
                today_dress = day_dress.clone();
                today_cold = day_cold.clone();
            }

            let advice = if idx != 1 {
                if let Some(ref dress) = day_dress {
                    let rain_str = if d_cond.contains('雨') || d_pop >= 40 {
                        "记得带伞"
                    } else {
                        "不用带伞"
                    };
                    format!("{dress}，{rain_str}")
                } else {
                    synthesize_daily_advice(idx, &d_cond, d_low, d_high, d_pop, first_day_high_c)
                }
            } else {
                synthesize_daily_advice(idx, &d_cond, d_low, d_high, d_pop, first_day_high_c)
            };

            daily.push(WeatherDailyPoint {
                date_label,
                ymd,
                condition: d_cond,
                high_c: d_high,
                low_c: d_low,
                pop_pct: d_pop,
                advice,
            });
        }
    }

    let fallback = WeatherSnapshot::default_for_city("上海 · 奉贤区");
    if daily.is_empty() {
        daily = fallback.daily.clone();
    }

    let high_c = daily.first().map(|d| d.high_c).unwrap_or(temp_c + 1);
    let low_c = daily.first().map(|d| d.low_c).unwrap_or(temp_c - 4);

    // Parse hourly forecast from `by_hour_json` (or `data.hourly` if non-empty)
    let mut hourly: Vec<WeatherHourlyPoint> = Vec::new();
    let hourly_val_owned: Option<serde_json::Value> =
        by_hour_json.and_then(|raw| serde_json::from_str(raw).ok());
    let hourly_array = hourly_val_owned
        .as_ref()
        .and_then(|hr_root| {
            let hr_ret = hr_root
                .get("androidAppfunctionsReturnValue")
                .map(json_first)
                .unwrap_or(hr_root);
            hr_ret
                .get("data")
                .map(json_first)
                .and_then(|d| d.get("hourly"))
                .and_then(|h| h.as_array())
        })
        .or_else(|| data.get("hourly").and_then(|h| h.as_array()));

    if let Some(arr) = hourly_array {
        for (idx, h_item) in arr.iter().take(8).enumerate() {
            let raw_hour = json_str(h_item.get("hour"));
            let hour_label = if idx == 0 {
                "现在".to_string()
            } else if !raw_hour.is_empty() {
                raw_hour
            } else {
                format!("{idx:02}:00")
            };
            let h_temp = json_i64(h_item.get("temp"))
                .map(|v| v as i32)
                .unwrap_or(temp_c);
            let h_cond = {
                let c = json_str(h_item.get("weather"));
                if c.is_empty() {
                    condition.clone()
                } else {
                    c
                }
            };
            let h_pop = json_i64(h_item.get("pop")).unwrap_or(0).clamp(0, 100) as u8;
            let h_wind_scale = json_i64(h_item.get("windScale")).unwrap_or(wind_scale);
            let wind_kmh = (h_wind_scale.clamp(1, 12) * 5) as u8;
            hourly.push(WeatherHourlyPoint {
                hour_label,
                temp_c: h_temp,
                condition: h_cond,
                pop_pct: h_pop,
                wind_kmh,
            });
        }
    }
    if hourly.is_empty() {
        hourly = fallback.hourly.clone();
    }

    let max_hourly_pop = hourly.iter().map(|h| h.pop_pct).max().unwrap_or(0);
    let today_pop = daily.first().map(|d| d.pop_pct).unwrap_or(0);
    let umbrella_needed = condition.contains('雨')
        || daily.first().is_some_and(|d| d.condition.contains('雨'))
        || max_hourly_pop >= 45
        || today_pop >= 45;

    let commute_advice = if let Some(dress) = today_dress {
        let rain_str = if umbrella_needed {
            "出门带伞"
        } else {
            "不用带伞"
        };
        if let Some(cold) = today_cold.filter(|c| c.contains("易发")) {
            format!("{dress}，{rain_str}（{cold}）")
        } else {
            format!("{dress}，{rain_str}")
        }
    } else {
        synthesize_daily_advice(
            0,
            &condition,
            low_c,
            high_c,
            if umbrella_needed { 60 } else { 0 },
            None,
        )
    };

    let backscreen_badge = if umbrella_needed {
        format!("{city_display} {temp_c}° {condition} · 带伞")
    } else {
        format!("{city_display} {temp_c}° {condition}")
    };

    let update_time_raw = json_str(realtime.get("updateTime"));
    let updated_label = update_time_raw
        .split_whitespace()
        .nth(1)
        .and_then(|hms| hms.get(0..5))
        .map(|hm| format!("{hm} 手机天气 MCP 同步"))
        .unwrap_or_else(|| "手机天气 MCP 刚刚同步".to_string());

    Ok(WeatherSnapshot {
        city: city_display,
        region_label,
        temp_c,
        feels_like_c,
        condition,
        high_c,
        low_c,
        humidity_pct,
        wind_label,
        aqi,
        aqi_label,
        uv_label,
        umbrella_needed,
        commute_advice,
        backscreen_badge,
        updated_label,
        hourly,
        daily,
        from_live_api: true,
    })
}

/// Queries the connected StepOS phone's system weather MCP service (`com.stepos.weather`)
/// for `device`, using an in-memory 10-minute (`Duration::from_secs(600)`) snapshot cache
/// (`WEATHER_SNAPSHOT_CACHE`) keyed by `device.serial` so repeated calls within 10 minutes
/// return immediately with 0 ADB commands.
pub fn fetch_weather_for_phone(
    adb: &crate::adb::Adb,
    device: &crate::device::Device,
) -> Result<WeatherSnapshot, String> {
    // Only a hardware identity can safely own a phone cache entry. Wireless
    // addresses/ports may later be assigned to another phone.
    let serial_key = phone_weather_cache_key(device);

    if let Some(serial_key) = serial_key.as_ref() {
        if let Ok(mut guard) = WEATHER_SNAPSHOT_CACHE.lock() {
            guard.retain(|_, (cached_at, _)| cached_at.elapsed() < WEATHER_SNAPSHOT_TTL);
            if let Some((cached_at, snapshot)) = guard.get(serial_key) {
                if cached_at.elapsed() < WEATHER_SNAPSHOT_TTL {
                    return Ok(snapshot.clone());
                }
            }
        }
    }

    let target_serial = if !device.transport.trim().is_empty() {
        device.transport.trim()
    } else {
        device.serial.trim()
    };
    if target_serial.is_empty() {
        return Err("未检测到在线 StepOS 手机".to_string());
    }

    let by_date_cmd = "cmd app_function execute-app-function --package com.stepos.weather --function 'com.stepos.weather.appfunctions.WeatherFunctions#get_current_location_weather' --parameters '{\"type\":\"by_date\"}'";
    let by_date_out = adb
        .run_timeout(
            &["-s", target_serial, "shell", by_date_cmd],
            Duration::from_secs(4),
        )
        .map_err(|e| format!("调用手机系统天气 MCP (by_date) 失败: {e}"))?;

    let by_hour_cmd = "cmd app_function execute-app-function --package com.stepos.weather --function 'com.stepos.weather.appfunctions.WeatherFunctions#get_current_location_weather' --parameters '{\"type\":\"by_hour\"}'";
    let by_hour_out = adb
        .run_timeout(
            &["-s", target_serial, "shell", by_hour_cmd],
            Duration::from_secs(4),
        )
        .ok();

    let snapshot = parse_stepos_appfunction_weather_json(&by_date_out, by_hour_out.as_deref())?;

    if let Some(serial_key) = serial_key {
        if let Ok(mut guard) = WEATHER_SNAPSHOT_CACHE.lock() {
            guard.insert(serial_key, (Instant::now(), snapshot.clone()));
        }
    }

    Ok(snapshot)
}

fn phone_weather_cache_key(device: &crate::device::Device) -> Option<String> {
    let serial = device.serial.trim();
    (!serial.is_empty() && !serial.contains(':') && !serial.starts_with("adb-"))
        .then(|| serial.to_string())
}

/// Queries the connected StepOS phone's system weather MCP service (`com.stepos.weather`)
/// via `cmd app_function execute-app-function` for the user's real current location & weather.
pub fn fetch_phone_mcp_weather_snapshot() -> Result<WeatherSnapshot, String> {
    let adb = crate::adb::Adb::discover().map_err(|e| format!("未找到 ADB: {e}"))?;
    let device = discover_unique_weather_phone(&adb)?;
    fetch_weather_for_phone(&adb, &device)
}

fn unique_weather_transport(devices_out: &str) -> Result<String, String> {
    let mut online: Vec<_> = devices_out.lines().filter_map(|line| {
        let mut parts = line.split_whitespace();
        let id = parts.next()?;
        (parts.next()? == "device").then(|| id.to_string())
    }).collect();
    online.sort();
    online.dedup();
    match online.as_slice() {
        [serial] => Ok(serial.clone()),
        [] => Err("未检测到在线 StepOS 手机".into()),
        _ => Err("无法确定当前手机，请先在客户端选择设备".into()),
    }
}

fn discover_unique_weather_phone(adb: &crate::adb::Adb) -> Result<crate::device::Device, String> {
    let devices_out = adb.run_timeout(&["devices"], Duration::from_secs(2))
        .map_err(|e| format!("查询 ADB 设备列表失败: {e}"))?;
    let serial = unique_weather_transport(&devices_out)?;
    Ok(crate::device::Device { transport: serial.clone(), serial, ..Default::default() })
}

/// Compatibility entry point. Unselected calls never guess among online phones.
pub fn fetch_live_weather_snapshot(city: &str) -> Result<WeatherSnapshot, String> {
    fetch_live_weather_snapshot_for_selection(city, weather_selection_revision())
}

pub fn fetch_live_weather_snapshot_for_selection(
    city: &str,
    city_revision: u64,
) -> Result<WeatherSnapshot, String> {
    ensure_weather_selection_current(city_revision)?;
    // Legacy callers have no current device. Use city weather only; explicit
    // phone-location callers must use the device-bound API below.
    let snapshot = fetch_live_weather_snapshot_for_device_selection(city, city_revision, None)?;
    publish_live_weather_snapshot(&snapshot, city_revision)?;
    Ok(snapshot)
}

fn ensure_weather_selection_current(city_revision: u64) -> Result<(), String> {
    if weather_selection_revision() != city_revision {
        Err("天气城市已变更，忽略过期结果".into())
    } else {
        Ok(())
    }
}

/// Fetch for the UI's captured device. This worker does not publish shared UI
/// data: the callback must first reject a changed device or city selection.
/// None never discovers a random ADB device. Phone location also never falls
/// back to an unrelated city's coordinates.
pub fn fetch_live_weather_snapshot_for_device_selection(
    city: &str,
    city_revision: u64,
    device: Option<&crate::device::Device>,
) -> Result<WeatherSnapshot, String> {
    ensure_weather_selection_current(city_revision)?;
    let requested_city = if city.trim().is_empty() {
        load_preferred_city()
    } else {
        city.trim().to_string()
    };
    let trimmed = requested_city.as_str();
    let is_other_preset = matches!(trimmed, "苏州" | "杭州" | "南京" | "北京" | "深圳");
    if !is_other_preset {
        let phone_result = match device {
            Some(device) => crate::adb::Adb::discover()
                .map_err(|e| format!("未找到 ADB: {e}"))
                .and_then(|adb| fetch_weather_for_phone(&adb, device)),
            None => Err("未选择手机，无法读取手机定位；请选择手机或指定城市".into()),
        };
        match phone_result {
            Ok(snapshot) if weather_city_matches(&snapshot, trimmed) => {
                ensure_weather_selection_current(city_revision)?;
                return Ok(snapshot);
            }
            Err(error) if trimmed == "手机定位" => return Err(error),
            Ok(_) | Err(_) => {}
        }
    }

    let target_city = if trimmed.is_empty() {
        load_preferred_city()
    } else {
        trimmed.to_string()
    };
    let (lat, lon, region_label) = city_coordinates(&target_city);
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}&current=temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m&hourly=temperature_2m,precipitation_probability,weather_code&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max&timezone=Asia%2FShanghai&forecast_days=5"
    );

    let output = std::process::Command::new("curl")
        .args(["-sS", "--max-time", "3", &url])
        .output()
        .map_err(|e| format!("调用 curl 获取天气失败: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "获取实时天气返回非零状态 ({}): {}",
            output.status,
            stderr.trim()
        ));
    }

    let raw_json = String::from_utf8_lossy(&output.stdout);
    let snapshot = parse_open_meteo_response(&target_city, &region_label, &raw_json)?;

    ensure_weather_selection_current(city_revision)?;

    if !cfg!(test) {
        if let Some(path) = weather_snapshot_cache_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(serialized) = serde_json::to_string_pretty(&snapshot) {
                let _ = std::fs::write(path, serialized);
            }
        }
    }

    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_legacy_discovery_refuses_ambiguous_endpoints() {
        assert_eq!(unique_weather_transport(
            "List of devices attached\nphone-a\tdevice\nphone-b\toffline\nphone-c\tunauthorized\n"
        ).unwrap(), "phone-a");
        assert_eq!(unique_weather_transport(
            "List of devices attached\nphone-a\tdevice\nphone-a\tdevice\n"
        ).unwrap(), "phone-a");
        assert!(unique_weather_transport(
            "List of devices attached\n10.0.1.7:5555\tdevice\n[fd00::7]:5555\tdevice\n"
        ).unwrap_err().contains("无法确定当前手机"));
        assert!(unique_weather_transport(
            "List of devices attached\nphone-a\toffline\n"
        ).is_err());
    }

    #[test]
    fn weather_callback_device_guard_allows_route_changes_but_rejects_replacement() {
        let original = crate::device::Device {
            serial: "weather-phone-a".into(), transport: "usb-a".into(), ..Default::default()
        };
        let wireless = crate::device::Device {
            transport: "10.0.1.7:5555".into(), ..original.clone()
        };
        let replacement = crate::device::Device {
            serial: "weather-phone-b".into(), ..wireless.clone()
        };
        assert!(weather_request_matches_device(Some(&original), Some(&wireless)));
        assert!(!weather_request_matches_device(Some(&original), Some(&replacement)));
        assert!(!weather_request_matches_device(Some(&original), None));
        assert!(!weather_request_matches_device(None, Some(&wireless)));
        assert!(weather_request_matches_device(None, None));
    }

    #[test]
    fn weather_display_cache_enforces_device_identity_and_original_two_hour_age() {
        let stored_at = Instant::now();
        let mut snapshot = WeatherSnapshot::default_for_city("上海");
        snapshot.from_live_api = true;
        snapshot.region_label = "手机定位 · 系统天气 MCP".into();
        let cache = CachedWeatherSnapshot {
            snapshot, stored_at, device_key: Some("phone-a".into())
        };
        assert!(cache.for_city_and_device("手机定位", Some("phone-a"), stored_at).is_some());
        assert!(cache.for_city_and_device("手机定位", Some("phone-b"), stored_at).is_none());
        assert!(cache.for_city_and_device("手机定位", None, stored_at).is_none());
        assert!(cache.for_city_and_device("深圳", Some("phone-a"), stored_at).is_none());
        assert!(cache.fresh_at(stored_at + WEATHER_DISPLAY_CACHE_TTL - Duration::from_secs(1)));
        assert!(!cache.fresh_at(stored_at + WEATHER_DISPLAY_CACHE_TTL));
        // Loading a 119-minute-old disk snapshot cannot grant another two hours.
        let imported = CachedWeatherSnapshot {
            stored_at: stored_at - Duration::from_secs(119 * 60), ..cache
        };
        assert!(imported.fresh_at(stored_at));
        assert!(!imported.fresh_at(stored_at + Duration::from_secs(60)));
    }

    #[test]
    fn phone_weather_cache_never_reuses_an_ip_alias_for_another_phone() {
        let phone_a = crate::device::Device {
            serial: "WEATHER_ISOLATION_TEST_A".into(),
            transport: "10.0.1.7:5555".into(), ..Default::default()
        };
        let phone_b = crate::device::Device {
            serial: "WEATHER_ISOLATION_TEST_B".into(), ..phone_a.clone()
        };
        let mut a = WeatherSnapshot::default_for_city("上海");
        a.temp_c = 11;
        let mut b = WeatherSnapshot::default_for_city("深圳");
        b.temp_c = 29;
        {
            let mut cache = WEATHER_SNAPSHOT_CACHE.lock().unwrap();
            cache.insert(phone_a.serial.clone(), (Instant::now(), a.clone()));
            cache.insert(phone_b.serial.clone(), (Instant::now(), b.clone()));
        }
        // A cache hit must not even try to execute this nonexistent ADB binary.
        let adb = crate::adb::Adb { path: PathBuf::from("/nonexistent-stepx-weather-test-adb") };
        assert_eq!(fetch_weather_for_phone(&adb, &phone_a).unwrap(), a);
        assert_eq!(fetch_weather_for_phone(&adb, &phone_b).unwrap(), b);
        let unknown = crate::device::Device {
            serial: phone_a.transport.clone(), ..phone_a.clone()
        };
        assert!(phone_weather_cache_key(&unknown).is_none());
        assert!(fetch_weather_for_phone(&adb, &unknown).is_err());
        let mut cache = WEATHER_SNAPSHOT_CACHE.lock().unwrap();
        cache.remove(&phone_a.serial);
        cache.remove(&phone_b.serial);
    }

    #[test]
    fn unselected_phone_location_fails_without_discovery_or_network_fallback() {
        let error = fetch_live_weather_snapshot_for_device_selection(
            "手机定位", weather_selection_revision(), None,
        ).unwrap_err();
        // Another parallel test may change city selection; both exits happen
        // before any ADB or network request.
        assert!(error.contains("未选择手机") || error.contains("城市已变更"));
    }

    #[test]
    fn preferred_city_load_restores_disk_once_and_preserves_a_newer_selection() {
        let cache = Mutex::new(None);
        assert_eq!(load_preferred_city_with(&cache, || Some(" 杭州\n".into())), "杭州");
        assert_eq!(load_preferred_city_with(&cache, || panic!("must use memory")), "杭州");
        for saved in [Some("杭州".to_string()), None] {
            let cache = Mutex::new(None);
            let city = load_preferred_city_with(&cache, || {
                *cache.lock().unwrap() = Some("苏州".to_string());
                saved
            });
            assert_eq!(city, "苏州");
            assert_eq!(cache.lock().unwrap().as_deref(), Some("苏州"));
        }
    }

    #[test]
    fn weather_source_and_city_matching_distinguish_live_phone_online_and_defaults() {
        let mut snapshot = WeatherSnapshot::default_for_city("上海");
        assert_eq!(snapshot.source_label(), "天气待同步");
        snapshot.from_live_api = true;
        assert_eq!(snapshot.source_label(), "Open-Meteo 在线天气");
        snapshot.city = "上海 · 奉贤区".into();
        snapshot.region_label = "手机定位 · 系统天气 MCP".into();
        assert_eq!(snapshot.source_label(), "手机系统天气 MCP");
        assert!(weather_city_matches(&snapshot, "上海"));
        assert!(weather_city_matches(&snapshot, "手机定位"));
        assert!(!weather_city_matches(&snapshot, "苏州"));
    }

    #[test]
    fn superseded_weather_selection_cannot_fetch_or_publish() {
        let stale_revision = u64::MAX;
        assert!(fetch_live_weather_snapshot_for_selection("上海", stale_revision)
            .unwrap_err().contains("城市已变更"));
        let fixture = WeatherSnapshot::default_for_city("上海");
        assert!(publish_live_weather_snapshot(&fixture, stale_revision)
            .unwrap_err().contains("城市已变更"));
    }

    #[test]
    fn preset_cities_have_distinct_realistic_snapshots() {
        assert_eq!(
            PRESET_CITIES,
            ["上海", "苏州", "杭州", "南京", "北京", "深圳"]
        );

        let sh = WeatherSnapshot::default_for_city("上海");
        assert_eq!(sh.city, "上海");
        assert_eq!(sh.region_label, "上海 · 华东");
        assert_eq!(sh.temp_c, 19);
        assert_eq!(sh.feels_like_c, 18);
        assert_eq!(sh.condition, "多云");
        assert_eq!(sh.high_c, 21);
        assert_eq!(sh.low_c, 14);
        assert_eq!(sh.humidity_pct, 58);
        assert_eq!(sh.wind_label, "东风 2级");
        assert_eq!(sh.aqi, 36);
        assert_eq!(sh.aqi_label, "优 36");
        assert_eq!(sh.uv_label, "中等");
        assert!(!sh.umbrella_needed);
        assert_eq!(sh.backscreen_badge, "上海 19° 多云");
        assert_eq!(sh.headline_badge(), "上海 19°C 多云");
        assert_eq!(sh.range_label(), "14°~21°C");
        assert_eq!(sh.hourly.len(), 6);
        assert_eq!(sh.daily.len(), 5);
        assert_eq!(sh.daily[0].date_label, "今天");
        assert_eq!(sh.daily[1].date_label, "明天");

        let hz = WeatherSnapshot::default_for_city("杭州");
        assert_eq!(hz.city, "杭州");
        assert_eq!(hz.condition, "小雨");
        assert!(hz.umbrella_needed);
        assert!(hz.commute_advice.contains("带伞"));

        let bj = WeatherSnapshot::default_for_city("北京");
        assert_eq!(bj.region_label, "北京 · 华北");
        assert!(bj.low_c < sh.low_c);

        let sz_south = WeatherSnapshot::default_for_city("深圳");
        assert_eq!(sz_south.region_label, "深圳 · 华南");
        assert!(sz_south.temp_c > sh.temp_c);
    }

    #[test]
    fn context_prompt_and_schedule_note_include_weather_and_schedules() {
        let sh = WeatherSnapshot::default_for_city("上海");
        let schedules = vec![
            "10:00-11:30 客户现场架构评审".to_string(),
            "15:00-16:00 产品周会".to_string(),
        ];
        let prompt = sh.build_weather_context_prompt(Some("10:00 外出参会"), &schedules);
        assert!(prompt.contains("[天气上下文 · 上海] 当前 多云 19°C（体感 18°C，今日 14°~21°C，湿度 58%，东风 2级，空气优 36）。"));
        assert!(prompt.contains("重点关注时段：10:00 外出参会。"));
        assert!(prompt.contains("今日关联日程：10:00-11:30 客户现场架构评审；15:00-16:00 产品周会。"));
        assert!(prompt.contains("请结合上述天气和我的日程，给出具体的出行、穿衣与时间安排建议。"));

        let prompt_no_sched = sh.build_weather_context_prompt(None, &[]);
        assert!(prompt_no_sched.ends_with("请结合当前天气，帮我规划今天的出行、穿衣或日程安排。"));

        let note = sh.build_weather_schedule_note(Some("14:00 外勤"));
        assert_eq!(note, "【上海 19°C 多云】14:00 外勤 · 出行与日程提醒（14°~21°C）");
    }

    #[test]
    fn enrich_with_phone_cues_updates_umbrella_and_advice() {
        let mut sh = WeatherSnapshot::default_for_city("上海");
        assert!(!sh.umbrella_needed);
        sh.enrich_with_phone_cues(&["气象台发布雷阵雨和大风降温预警，傍晚有大雨".to_string()]);
        assert!(sh.umbrella_needed);
        assert!(sh.commute_advice.contains("带伞"));
        assert!(sh.commute_advice.contains("防风保暖"));
        assert!(sh.backscreen_badge.contains("带伞"));
    }

    #[test]
    fn parse_open_meteo_response_extracts_live_fields() {
        let sample_json = r#"{
            "current": {
                "temperature_2m": 20.4,
                "relative_humidity_2m": 64,
                "apparent_temperature": 19.6,
                "weather_code": 61,
                "wind_speed_10m": 15.2
            },
            "hourly": {
                "temperature_2m": [20.1, 20.5, 21.0, 20.2, 19.4, 18.8, 18.0, 17.5, 17.2, 17.0, 16.8, 16.5, 16.2, 16.0, 16.5, 17.2, 18.4, 19.5, 20.1, 20.4, 19.8, 19.1, 18.5, 18.0],
                "precipitation_probability": [70, 65, 40, 20, 10, 10, 5, 5, 5, 10, 15, 20, 25, 30, 35, 40, 50, 60, 70, 65, 50, 30, 20, 10],
                "weather_code": [61, 61, 3, 2, 1, 1, 0, 0, 0, 1, 1, 2, 2, 3, 3, 51, 61, 61, 61, 51, 3, 2, 1, 1]
            },
            "daily": {
                "time": ["2026-11-05", "2026-11-06", "2026-11-07", "2026-11-08", "2026-11-09"],
                "weather_code": [61, 2, 0, 1, 3],
                "temperature_2m_max": [21.3, 22.0, 23.1, 21.8, 19.5],
                "temperature_2m_min": [14.8, 14.2, 13.9, 13.5, 12.1],
                "precipitation_probability_max": [75, 20, 5, 10, 30]
            }
        }"#;

        // Include the next day so the six-hour forecast is deterministic at night.
        let mut sample:serde_json::Value=serde_json::from_str(sample_json).unwrap();
        for key in ["temperature_2m","precipitation_probability","weather_code"] {
            let values=sample["hourly"][key].as_array_mut().unwrap();values.extend(values.clone());
        }
        let snap = parse_open_meteo_response("上海", "上海 · 华东", &sample.to_string()).expect("valid open-meteo json");
        assert!(snap.from_live_api);
        assert_eq!(snap.city, "上海");
        assert_eq!(snap.temp_c, 20);
        assert_eq!(snap.feels_like_c, 20);
        assert_eq!(snap.condition, "小雨");
        assert_eq!(snap.high_c, 21);
        assert_eq!(snap.low_c, 15);
        assert!(snap.umbrella_needed);
        assert!(snap.backscreen_badge.contains("带伞"));
        assert_eq!(snap.daily.len(), 5);
        assert_eq!(snap.hourly.len(), 6);
    }

    #[test]
    fn preferred_city_roundtrip_in_memory_during_test() {
        save_preferred_city("苏州");
        assert_eq!(load_preferred_city(), "苏州");
        let snap = load_or_default_weather(None);
        assert_eq!(snap.city, "苏州");
        save_preferred_city("上海");
        assert_eq!(load_preferred_city(), "上海");
    }

    #[test]
    fn home_hub_weather_card_carries_live_weather_context_into_amoo_prompt() {
        let wb = crate::toolbox::StepOsDigitalWellbeing {
            screen_time_minutes: 45,
            unlock_count: 8,
            top_apps: Vec::new(),
            alphame_running: true,
        };
        let input = crate::home_hub::HomeHubInput {
            hour: 9,
            minute: 30,
            today_ymd: "2026-11-05",
            device_connected: true,
            device_model: "PZ261A",
            firmware_profile: "StepOS",
            threads: &[],
            notifications: &[],
            otp_banners: &[],
            sms: &[],
            call_logs: &[],
            express_parcels: &[],
            calendar_events: &[],
            alarms: &[],
            wellbeing: &wb,
            phone_clipboard: "",
            remote_agent_files: &[],
            work_deliverables_count: 0,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        };
        // Publish an explicit synthetic live fixture; a default fallback alone
        // must never be treated as observed weather. Avoid global cache races.
        let mut snapshot = WeatherSnapshot::default_for_city("上海");
        snapshot.from_live_api = true;
        let scene = crate::home_hub::detect_scene(input.hour, false, false);
        let cards = crate::home_hub::build_life_context_cards_from_weather(
            &input,
            &scene,
            &[],
            0,
            Some(snapshot),
        );
        let weather_card = cards
            .iter()
            .find(|c| c.domain == crate::home_hub::HomeLifeDomain::WeatherAndCommute)
            .expect("weather context card");
        assert!(weather_card.title.contains("上海 19°C 多云"));
        assert!(weather_card.detail.contains("体感 18°C"));
        assert!(weather_card.detail.contains("明天 晴间多云 15°~22°C"));
        assert!(weather_card
            .detail
            .contains("早晚薄外套、中午单衣，不用带伞"));
        match &weather_card.intent {
            crate::home_hub::HomeAiIntent::AmooPrompt(prompt) => {
                assert!(prompt.contains("[天气上下文 · 上海]"));
                assert!(prompt.contains("当前 多云 19°C"));
                assert!(prompt.contains("明日预报与建议：明天 晴间多云 15°~22°C"));
            }
            other => panic!("expected AmooPrompt with weather context, got {other:?}"),
        }
    }

    #[test]
    fn parse_stepos_appfunction_weather_json_extracts_location_realtime_daily_hourly_and_life_indexes() {
        let by_date_json = r#"{
          "androidAppfunctionsReturnValue": [
            {
              "success": [true],
              "code": [0],
              "data": [
                {
                  "latitude": ["30.964592"],
                  "longitude": ["121.502411"],
                  "queryTime": ["2026-11-05 09:57:55"],
                  "region": [
                    {
                      "province": ["上海市"],
                      "area": [""],
                      "city": ["奉贤区"]
                    }
                  ],
                  "city": [
                    {
                      "cityId": [46],
                      "province": ["上海市"],
                      "name": ["奉贤区"],
                      "country": ["中国"]
                    }
                  ],
                  "realtime": [
                    {
                      "temp": ["17"],
                      "sendibleTemp": ["16"],
                      "weather": ["阴"],
                      "humidity": ["74"],
                      "windDirection": ["西北风"],
                      "windScale": ["3"],
                      "windSpeed": ["1.54"],
                      "uvi": [1],
                      "sunrise": ["06:12"],
                      "sunset": ["17:03"],
                      "updateTime": ["2026-11-05 09:57:32"],
                      "airQuality": [
                        {
                          "aqi": [23],
                          "pm10": [18],
                          "pm25": [16],
                          "quality": ["优"]
                        }
                      ]
                    }
                  ],
                  "daily": [
                    {
                      "date": ["11/5"],
                      "fullDate": ["2026-11-05"],
                      "week": ["今天"],
                      "tempLow": ["13"],
                      "tempHigh": ["18"],
                      "weatherDay": ["多云"],
                      "weatherNight": ["多云"],
                      "pop": [0],
                      "humidity": [74],
                      "aqi": ["30"],
                      "quality": ["优"],
                      "windDirectionDay": ["西北风"],
                      "windScaleDay": ["3"],
                      "lifeIndexes": [
                        {
                          "name": ["穿衣指数"],
                          "level": ["薄外套"],
                          "remind": ["薄外套"],
                          "type": ["DRESS_LIVE_INDEX"]
                        },
                        {
                          "name": ["紫外线指数"],
                          "level": ["最弱"],
                          "remind": ["无需防晒"],
                          "type": ["ULTRAVIOLET_LIVE_INDEX"]
                        },
                        {
                          "name": ["感冒指数"],
                          "level": ["易发"],
                          "remind": ["感冒易发"],
                          "type": ["COLDR_LIVE_INDEX"]
                        },
                        {
                          "name": ["洗车指数"],
                          "level": ["适宜"],
                          "remind": ["适宜洗车"],
                          "type": ["CAR_LIVE_INDEX"]
                        }
                      ]
                    },
                    {
                      "date": ["11/6"],
                      "fullDate": ["2026-11-06"],
                      "week": ["明天"],
                      "tempLow": ["15"],
                      "tempHigh": ["20"],
                      "weatherDay": ["多云"],
                      "weatherNight": ["小雨"],
                      "pop": [10],
                      "humidity": [70],
                      "aqi": ["32"],
                      "quality": ["优"],
                      "windDirectionDay": ["东南风"],
                      "windScaleDay": ["2"],
                      "lifeIndexes": []
                    }
                  ]
                }
              ],
              "message": ["查询成功"]
            }
          ]
        }"#;

        let by_hour_json = r#"{
          "androidAppfunctionsReturnValue": [
            {
              "success": [true],
              "code": [0],
              "data": [
                {
                  "hourly": [
                    {
                      "date": ["2026-11-05 09:00:00"],
                      "hour": ["09:00"],
                      "temp": ["17"],
                      "weather": ["阴"],
                      "pop": [0],
                      "humidity": [74],
                      "windDirection": ["西北风"],
                      "windScale": [3]
                    },
                    {
                      "date": ["2026-11-05 10:00:00"],
                      "hour": ["10:00"],
                      "temp": ["18"],
                      "weather": ["多云"],
                      "pop": [10],
                      "humidity": [70],
                      "windDirection": ["西北风"],
                      "windScale": [3]
                    }
                  ]
                }
              ]
            }
          ]
        }"#;

        let snap = parse_stepos_appfunction_weather_json(by_date_json, Some(by_hour_json))
            .expect("valid stepos weather mcp json");
        assert!(snap.from_live_api);
        assert_eq!(snap.city, "上海 · 奉贤区");
        assert_eq!(snap.region_label, "手机定位 · 系统天气 MCP");
        assert_eq!(snap.temp_c, 17);
        assert_eq!(snap.feels_like_c, 16);
        assert_eq!(snap.condition, "阴");
        assert_eq!(snap.high_c, 18);
        assert_eq!(snap.low_c, 13);
        assert_eq!(snap.humidity_pct, 74);
        assert_eq!(snap.wind_label, "西北风 3级");
        assert_eq!(snap.aqi, 23);
        assert_eq!(snap.aqi_label, "优 23");
        assert_eq!(snap.uv_label, "最弱");
        assert!(!snap.umbrella_needed);
        assert_eq!(snap.commute_advice, "穿薄外套，不用带伞（感冒易发）");
        assert_eq!(snap.updated_label, "09:57 手机天气 MCP 同步");
        assert_eq!(snap.daily.len(), 2);
        assert_eq!(snap.daily[0].date_label, "今天");
        assert_eq!(snap.daily[0].ymd, "2026-11-05");
        assert_eq!(snap.daily[0].advice, "穿薄外套，不用带伞");
        assert_eq!(snap.daily[1].date_label, "明天");
        assert_eq!(snap.daily[1].advice, "穿长袖或薄卫衣，不用带伞");
        assert_eq!(snap.tomorrow_temp_delta_label(), "+2°C");
        assert!(!snap.tomorrow_umbrella_needed());
        assert_eq!(snap.tomorrow_headline(), "明天 多云 15°~20°C");
        assert_eq!(snap.tomorrow_advice(), "穿长袖或薄卫衣，不用带伞");
        assert_eq!(
            snap.build_tomorrow_schedule_note(),
            "【明日天气提醒 · 上海 · 奉贤区】明天 多云 15°~20°C · 穿长袖或薄卫衣，不用带伞"
        );
        assert_eq!(snap.hourly.len(), 2);
        assert_eq!(snap.hourly[0].hour_label, "现在");
        assert_eq!(snap.hourly[0].temp_c, 17);
        assert_eq!(snap.hourly[1].hour_label, "10:00");
        assert_eq!(snap.hourly[1].temp_c, 18);
    }

    #[test]
    fn synthesize_daily_advice_and_tomorrow_methods_cover_all_scenarios() {
        assert_eq!(
            synthesize_daily_advice(1, "多云", 13, 21, 0, Some(17)),
            "比今天暖 4°C，早晚薄外套、中午单衣，不用带伞"
        );
        assert_eq!(
            synthesize_daily_advice(1, "小雨", 10, 14, 70, Some(19)),
            "比今天冷 5°C，穿风衣或毛衣，记得带伞"
        );

        let fx = WeatherSnapshot::default_for_city("上海 · 奉贤区");
        let t = fx.tomorrow_point();
        assert_eq!(t.date_label, "明天");
        assert_eq!(t.high_c, 21);
        assert_eq!(t.low_c, 13);
        assert_eq!(fx.tomorrow_temp_delta_label(), "+3°C");
        assert!(!fx.tomorrow_umbrella_needed());
        assert_eq!(fx.tomorrow_headline(), "明天 多云 13°~21°C");
        assert_eq!(
            fx.tomorrow_advice(),
            "比今天暖 3°C，早晚薄外套、中午单衣，不用带伞"
        );
        assert_eq!(
            fx.tomorrow_advice_with_source(),
            (
                "比今天暖 3°C，早晚薄外套、中午单衣，不用带伞".to_string(),
                "天气预报建议"
            ),
            "forecast fallback must not claim to be an Amoo-generated response"
        );
        assert!(fx
            .build_tomorrow_schedule_note()
            .starts_with("【明日天气提醒 · 上海 · 奉贤区】明天 多云 13°~21°C · "));

        let sz = WeatherSnapshot::default_for_city("深圳");
        assert!(sz.tomorrow_umbrella_needed());
        assert!(sz.tomorrow_advice().contains("记得带伞"));

        let prompt = fx.build_weather_context_prompt(None, &[]);
        assert!(prompt.contains("明日预报与建议：明天 多云 13°~21°C（+3°C，"));
    }

    #[test]
    fn daily_amoo_weather_advice_caches_for_entire_day_and_updates_only_on_next_day() {
        let fx = WeatherSnapshot::default_for_city("上海 · 奉贤区");
        let cache = Mutex::new(None);

        // Day 1 ("2026-11-05"): generate and cache advice
        let day1_first = resolve_daily_weather_advice_for_date(&fx, "2026-11-05", &cache, false);
        assert_eq!(day1_first.date_ymd, "2026-11-05");
        assert_eq!(day1_first.city, "上海 · 奉贤区");
        assert!(is_daily_advice_valid_for_today(
            &day1_first,
            "2026-11-05",
            "上海 · 奉贤区"
        ));
        assert!(!is_daily_advice_valid_for_today(
            &day1_first,
            "2026-11-06",
            "上海 · 奉贤区"
        ));

        // Simulate Amoo advice locked in for 2026-11-05
        if let Ok(mut guard) = cache.lock() {
            *guard = Some(CachedDailyWeatherAdvice {
                date_ymd: "2026-11-05".to_string(),
                city: "上海 · 奉贤区".to_string(),
                today_short: fx.commute_advice.clone(),
                tomorrow_advice: "明天暖和3度，早晚搭件薄外套，不用带伞".to_string(),
                generated_by_amoo: true,
            });
        }

        // Subsequent refresh on the SAME day ("2026-11-05") must return the cached Amoo advice without updating
        let mut modified_same_day = fx.clone();
        modified_same_day.daily[1].advice = "穿厚外套保暖，记得带伞".to_string();
        let same_day_second =
            resolve_daily_weather_advice_for_date(&modified_same_day, "2026-11-05", &cache, false);
        assert_eq!(
            same_day_second.tomorrow_advice,
            "明天暖和3度，早晚搭件薄外套，不用带伞"
        );
        assert!(same_day_second.generated_by_amoo);

        // On the NEXT day ("2026-11-06"), cache expires and refreshes with the new day's advice
        let next_day =
            resolve_daily_weather_advice_for_date(&modified_same_day, "2026-11-06", &cache, false);
        assert_eq!(next_day.date_ymd, "2026-11-06");
        assert_eq!(next_day.tomorrow_advice, "穿厚外套保暖，记得带伞");
    }

    #[cfg(unix)]
    #[test]
    fn fetch_weather_for_phone_caches_for_ten_minutes_with_zero_repeated_adb_commands_and_syncs_phone_daily_advice() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("adb_calls.log");
        let phone_advice_file = dir.path().join("phone_weather_daily_advice.json");
        let script_path = dir.path().join("fake_adb.sh");

        let script = format!(
            r#"#!/usr/bin/env bash
echo "$@" >> "{log}"
case "$*" in
  *"get_current_location_weather"*'"type":"by_date"'*)
    cat << 'EOF'
{{"androidAppfunctionsReturnValue":[{{"success":[true],"code":[0],"data":[{{"region":[{{"province":["上海市"],"city":["奉贤区"]}}],"realtime":[{{"temp":["18"],"sendibleTemp":["17"],"weather":["多云"],"humidity":["68"],"windDirection":["东南风"],"windScale":["2"],"uvi":[2],"updateTime":["2026-11-05 10:15:00"],"airQuality":[{{"aqi":[26],"quality":["优"]}}]}}],"daily":[{{"fullDate":["2026-11-05"],"week":["今天"],"tempLow":["13"],"tempHigh":["19"],"weatherDay":["多云"],"pop":[10],"lifeIndexes":[]}},{{"fullDate":["2026-11-06"],"week":["明天"],"tempLow":["14"],"tempHigh":["21"],"weatherDay":["晴"],"pop":[5],"lifeIndexes":[]}}]}}]}}]}}
EOF
    exit 0
    ;;
  *"get_current_location_weather"*'"type":"by_hour"'*)
    cat << 'EOF'
{{"androidAppfunctionsReturnValue":[{{"success":[true],"code":[0],"data":[{{"hourly":[{{"hour":["10:00"],"temp":["18"],"weather":["多云"],"pop":[10],"windScale":[2]}}]}}]}}]}}
EOF
    exit 0
    ;;
  *"cat '/sdcard/Android/data/com.stepx.companion.app/files/weather_daily_advice.json'"*)
    if [ -f "{phone_advice}" ]; then
      cat "{phone_advice}"
    fi
    exit 0
    ;;
  *"/sdcard/Android/data/com.stepx.companion.app/files/weather_daily_advice.json"*)
    # Extract JSON payload written via printf '%s' '...'
    python3 -c '
import sys, re
cmd = " ".join(sys.argv[1:])
m = re.search(r"printf \x27%s\x27 \x27(.*)\x27 >", cmd)
if m:
    open("{phone_advice}", "w").write(m.group(1))
' "$@"
    exit 0
    ;;
esac
exit 0
"#,
            log = log_path.display(),
            phone_advice = phone_advice_file.display()
        );
        std::fs::write(&script_path, script).unwrap();
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();

        let adb = crate::adb::Adb { path: script_path };
        let device = crate::device::Device {
            serial: "STEPOS_WEATHER_CACHE_TEST_01".to_string(),
            transport: "STEPOS_WEATHER_CACHE_TEST_01".to_string(),
            ..Default::default()
        };

        // Clear any prior entry for this test serial
        if let Ok(mut guard) = WEATHER_SNAPSHOT_CACHE.lock() {
            guard.remove(&device.serial);
        }

        // First call populates WEATHER_SNAPSHOT_CACHE via ADB
        let snap1 = fetch_weather_for_phone(&adb, &device).expect("first fetch");
        assert_eq!(snap1.city, "上海 · 奉贤区");
        assert_eq!(snap1.temp_c, 18);
        let log_after_first = std::fs::read_to_string(&log_path).unwrap_or_default();
        let first_call_lines = log_after_first.lines().count();
        assert!(first_call_lines >= 1, "first call should invoke adb");

        // Second call within 10 minutes must return cached snapshot with 0 additional ADB commands
        let snap2 = fetch_weather_for_phone(&adb, &device).expect("second cached fetch");
        assert_eq!(snap2, snap1);
        let log_after_second = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert_eq!(
            log_after_second.lines().count(),
            first_call_lines,
            "second fetch within 10-minute TTL must spawn 0 ADB commands"
        );

        // Verify on-device daily Amoo weather advice write + read roundtrip
        let advice = CachedDailyWeatherAdvice {
            date_ymd: "2026-11-05".to_string(),
            city: "上海 · 奉贤区".to_string(),
            today_short: "穿薄外套，不用带伞".to_string(),
            tomorrow_advice: "明天晴暖21度，早晚薄外套不用带伞".to_string(),
            generated_by_amoo: true,
        };
        assert!(write_phone_daily_weather_advice(
            &adb,
            &device.serial,
            &advice
        ));
        let loaded = read_phone_daily_weather_advice(
            &adb,
            &device.serial,
            "2026-11-05",
            "上海 · 奉贤区",
        )
        .expect("should load today's advice from phone");
        assert_eq!(loaded, advice);
        assert!(
            read_phone_daily_weather_advice(&adb, &device.serial, "2026-11-06", "上海 · 奉贤区")
                .is_none(),
            "stale date on phone must not match next day"
        );
    }
}
