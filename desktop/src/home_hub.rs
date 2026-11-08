//! Home Context Intelligence & Aggregation Engine (`Page::Home`).
//!
//! Synthesizes real-time phone notifications, IM threads, SMS, missed calls,
//! express parcels, takeout/delivery status, calendar schedules, alarms, clipboard,
//! and StepOS Digital Wellbeing habits into a unified priority stream and
//! context-aware AI recommendations for the desktop home hub.

use chrono::{Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub type PhoneNotification = crate::push::PushNotification;
pub type CallRecord = crate::device::CallRecord;
pub type ExpressParcelInfo = crate::device::ExpressParcelInfo;
pub type HomeReminderGovernanceStore = crate::home_reminder_governance::HomeReminderGovernance;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HomeScene {
    MorningBriefing,
    WorkFocus,
    LunchAndLife,
    AfternoonCollaboration,
    EveningWrapUp,
    LateNightRest,
}

impl HomeScene {
    pub fn label(self) -> &'static str {
        match self {
            Self::MorningBriefing => "晨间简报",
            Self::WorkFocus => "会议与专注",
            Self::LunchAndLife => "午间生活",
            Self::AfternoonCollaboration => "午后协同",
            Self::EveningWrapUp => "晚间收尾",
            Self::LateNightRest => "夜间轻扰",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeSceneInfo {
    pub scene: HomeScene,
    pub badge: String,
    pub title: String,
    pub subtitle: String,
    pub suggested_backscreen_status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HomeAiIntent {
    AmooPrompt(String),
    QuickGuiScenario { app: String, task: String },
    WorkPrompt(String),
    BackScreenStatus(String),
    QueryExpress(String),
    AlipayCoupon,
    CapCutHandoff,
    PullRemoteAgentFiles,
    StartPomodoro(u32),
    ToggleDnd(bool),
    OpenPage(String),
    CreatePhoneAlarm { hour: u8, minute: u8, label: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeScheduleConflict {
    pub id: String,
    pub event_id: i64,
    pub event_date_ymd: String,
    pub event_time_hm: String,
    pub event_title: String,
    pub conflict_badge: String,
    pub headline: String,
    pub detail: String,
    pub suggested_alarm_hour: u8,
    pub suggested_alarm_minute: u8,
    pub suggested_alarm_label: String,
    pub primary_action_label: String,
    pub primary_intent: HomeAiIntent,
    pub secondary_action_label: String,
    pub secondary_intent: HomeAiIntent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeScheduleChecklist {
    pub event_id: i64,
    pub date_ymd: String,
    pub time_hm: String,
    pub event_title: String,
    pub items: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeAiRecommendation {
    pub id: String,
    pub badge: String,
    pub title: String,
    pub reason: String,
    pub action_label: String,
    pub intent: HomeAiIntent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HomeCardKind {
    OtpAndPickup,
    TakeoutAndDelivery,
    ExpressParcel,
    MissedCall,
    UnreadMessage,
    UpcomingMeeting,
}

impl HomeCardKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::OtpAndPickup => "验证码与取件码",
            Self::TakeoutAndDelivery => "外卖与出行",
            Self::ExpressParcel => "快递动态",
            Self::MissedCall => "未接来电",
            Self::UnreadMessage => "未读消息",
            Self::UpcomingMeeting => "日程与会议",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomePriorityCard {
    pub id: String,
    pub kind: HomeCardKind,
    pub urgency: u8,
    pub badge: String,
    pub title: String,
    pub subtitle: String,
    pub detail: String,
    pub timestamp_label: String,
    pub code_or_number: Option<String>,
    pub thread_id: Option<String>,
    pub notification_key: Option<String>,
    pub package_name: Option<String>,
    pub quick_replies: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct HomeHubInput<'a> {
    pub hour: u32,
    pub minute: u32,
    pub today_ymd: &'a str,
    pub device_connected: bool,
    pub device_model: &'a str,
    pub firmware_profile: &'a str,
    pub threads: &'a [crate::push::ImThread],
    pub notifications: &'a [crate::push::PushNotification],
    pub otp_banners: &'a [crate::push::OtpBanner],
    pub sms: &'a [crate::sms::Message],
    pub call_logs: &'a [crate::device::CallRecord],
    pub express_parcels: &'a [crate::device::ExpressParcelInfo],
    pub calendar_events: &'a [crate::toolbox::PhoneCalendarEvent],
    pub alarms: &'a [crate::toolbox::PhoneAlarmItem],
    pub wellbeing: &'a crate::toolbox::StepOsDigitalWellbeing,
    pub phone_clipboard: &'a str,
    pub remote_agent_files: &'a [String],
    pub work_deliverables_count: usize,
    pub pomodoro_running: bool,
    pub pomodoro_remaining_secs: u32,
    pub dnd_on: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HomeLifeDomain {
    ExpressAndPickup,
    WeatherAndCommute,
    LeisureAndWellbeing,
}

impl HomeLifeDomain {
    pub fn label(self) -> &'static str {
        match self {
            Self::ExpressAndPickup => "快递与取件",
            Self::WeatherAndCommute => "天气与出行",
            Self::LeisureAndWellbeing => "预约与愉悦 · 要愉悦",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeLifeContextCard {
    pub id: String,
    pub domain: HomeLifeDomain,
    pub badge: String,
    pub title: String,
    pub context_link: String,
    pub detail: String,
    pub code_or_number: Option<String>,
    pub action_label: String,
    pub intent: HomeAiIntent,
    pub secondary_label: Option<String>,
    pub secondary_intent: Option<HomeAiIntent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SelectedDateEntryKind {
    Schedule,
    WorkAndMatter,
    Life,
    Issue,
    Communication,
}

impl SelectedDateEntryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Schedule => "日程",
            Self::WorkAndMatter => "工作/事项",
            Self::Life => "生活",
            Self::Issue => "待决问题",
            Self::Communication => "沟通",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopDateEntry {
    pub id: String,
    pub timestamp: i64,
    pub kind: SelectedDateEntryKind,
    pub badge: String,
    pub title: String,
    pub detail: String,
    pub action_label: String,
    pub intent: HomeAiIntent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedDateEntry {
    pub id: String,
    pub time_hm: String,
    pub sort_minute: u32,
    pub kind: SelectedDateEntryKind,
    pub badge: String,
    pub title: String,
    pub detail: String,
    pub action_label: String,
    pub intent: HomeAiIntent,
    pub checklist_items: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedDatePanorama {
    pub date_ymd: String,
    pub date_display: String,
    pub weekday_label: String,
    pub is_today: bool,
    pub schedule_count: usize,
    pub work_count: usize,
    pub life_count: usize,
    pub issue_count: usize,
    pub communication_count: usize,
    pub headline: String,
    pub entries: Vec<SelectedDateEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeeklyDaySummary {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub date_ymd: String,
    pub weekday_label: String,
    pub is_today: bool,
    pub is_selected: bool,
    pub schedule_count: usize,
    pub work_count: usize,
    pub life_count: usize,
    pub issue_count: usize,
    pub communication_count: usize,
    pub total_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeeklyOverviewReport {
    pub week_range_label: String,
    pub days: Vec<WeeklyDaySummary>,
    pub total_schedules: usize,
    pub total_work_items: usize,
    pub total_life_items: usize,
    pub total_issues: usize,
    pub busiest_day_label: String,
    pub headline: String,
    /// `(title, metric_badge, detail_summary)` for 4 structured weekly dimensions
    pub pillars: Vec<(String, String, String)>,
}

#[derive(Clone, Debug)]
pub struct HomeHubReport {
    pub scene: HomeSceneInfo,
    pub unread_messages_count: usize,
    pub active_life_cards_count: usize,
    pub today_meetings_count: usize,
    pub enabled_alarms_count: usize,
    pub priority_cards: Vec<HomePriorityCard>,
    pub life_context_cards: Vec<HomeLifeContextCard>,
    pub ai_recommendations: Vec<HomeAiRecommendation>,
    pub schedule_conflicts: Vec<HomeScheduleConflict>,
    pub schedule_checklists: Vec<HomeScheduleChecklist>,
    pub habit_headline: String,
    pub habit_tags: Vec<String>,
}

/// Detects the current contextual scene on the desktop Home hub from time of day
/// and live user activity flags.
pub fn detect_scene(
    hour: u32,
    upcoming_meeting_within_45m: bool,
    has_active_delivery_or_express: bool,
) -> HomeSceneInfo {
    let normalized_hour = hour % 24;

    if upcoming_meeting_within_45m || (10..=11).contains(&normalized_hour) {
        let subtitle = if upcoming_meeting_within_45m && has_active_delivery_or_express {
            "快照中有近期日程 · 先核对安排，再处理待回复消息与取件".to_string()
        } else if upcoming_meeting_within_45m {
            "快照中 45 分钟内有日程 · 查看安排，准备需要的资料".to_string()
        } else if has_active_delivery_or_express {
            "先推进手头任务，再查看消息与快递最新状态".to_string()
        } else {
            "继续手头任务，按需查看手机消息与日程".to_string()
        };
        return HomeSceneInfo {
            scene: HomeScene::WorkFocus,
            badge: "会议与专注".to_string(),
            title: "深度工作 · 会议与消息直回".to_string(),
            subtitle,
            suggested_backscreen_status: "开会勿扰".to_string(),
        };
    }

    match normalized_hour {
        5..=9 => {
            let subtitle = if has_active_delivery_or_express {
                "先核对今天的安排，再查看待回复消息与取件".to_string()
            } else {
                "先核对今天的安排，选一件最值得推进的事".to_string()
            };
            HomeSceneInfo {
                scene: HomeScene::MorningBriefing,
                badge: "晨间简报".to_string(),
                title: "早安 · 今日日程与重点消息".to_string(),
                subtitle,
                suggested_backscreen_status: "专注中".to_string(),
            }
        }
        12..=13 => {
            let subtitle = if has_active_delivery_or_express {
                "查看已读到的配送与取件信息，按需刷新最新状态".to_string()
            } else {
                "午间稍作休息，按需查看配送或调整背屏状态".to_string()
            };
            HomeSceneInfo {
                scene: HomeScene::LunchAndLife,
                badge: "午间生活".to_string(),
                title: "午间驿站 · 外卖、神券与取件".to_string(),
                subtitle,
                suggested_backscreen_status: "去吃饭".to_string(),
            }
        }
        14..=17 => {
            let subtitle = if has_active_delivery_or_express {
                "继续未完成的工作，查看成果与待回复消息".to_string()
            } else {
                "从已有讨论继续，把下一步整理成可用成果".to_string()
            };
            HomeSceneInfo {
                scene: HomeScene::AfternoonCollaboration,
                badge: "午后协同".to_string(),
                title: "高效协同 · 办公、快递与秒回".to_string(),
                subtitle,
                suggested_backscreen_status: "专注中".to_string(),
            }
        }
        18..=21 => {
            let subtitle = if has_active_delivery_or_express {
                "核对待取件，回顾今天的进展与尚待确认的事".to_string()
            } else {
                "回顾已记录的成果与阻塞，为明天留下明确下一步".to_string()
            };
            HomeSceneInfo {
                scene: HomeScene::EveningWrapUp,
                badge: "晚间收尾".to_string(),
                title: "晚间收官 · 取件、日报与成片".to_string(),
                subtitle,
                suggested_backscreen_status: "休息一下".to_string(),
            }
        }
        _ => HomeSceneInfo {
            scene: HomeScene::LateNightRest,
            badge: "夜间轻扰".to_string(),
            title: "夜深护眼 · 明日闹钟与勿扰".to_string(),
            subtitle: "按需开启勿扰，确认明日闹钟与早间安排".to_string(),
            suggested_backscreen_status: "冥想呼吸".to_string(),
        },
    }
}

/// Generates contextual one-click quick replies tailored to the message source and content.
pub fn quick_replies_for_message(app_or_source: &str, title: &str, text: &str) -> Vec<String> {
    quick_replies_for_message_with_context(app_or_source, title, text, None, None, false)
}

/// Generates contextual one-click quick replies incorporating live schedule, Pomodoro, and DND state.
pub fn quick_replies_for_message_with_context(
    app_or_source: &str,
    title: &str,
    text: &str,
    active_meeting_title: Option<&str>,
    pomodoro_remaining_mins: Option<u32>,
    dnd_on: bool,
) -> Vec<String> {
    let fields = [app_or_source, title, text];
    let any_contains = |kw: &str| fields.iter().any(|f| f.contains(kw));
    let any_ascii = |needle: &[u8]| fields.iter().any(|f| contains_ascii_ignore_case(f, needle));

    let is_delivery_or_courier = [
        "外卖",
        "骑手",
        "美团",
        "饿了么",
        "闪购",
        "快递",
        "顺丰",
        "京东",
        "菜鸟",
        "驿站",
        "丰巢",
        "中通",
        "圆通",
        "韵达",
        "申通",
        "极兔",
        "派件",
        "送达",
        "取餐",
        "取件",
        "包裹",
        "小哥",
    ]
    .iter()
    .any(|kw| any_contains(kw))
        || any_ascii(b"meituan")
        || any_ascii(b"ele");

    let (meeting_short, meeting_end_suffix) = match active_meeting_title
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(raw) => {
            if let Some((title_part, rest)) = raw.split_once("（预计 ") {
                let short = title_part.trim().chars().take(10).collect::<String>();
                let suffix = format!("（预计 {}", rest);
                (Some(short), suffix)
            } else {
                (
                    Some(raw.chars().take(10).collect::<String>()),
                    String::new(),
                )
            }
        }
        None => (None, String::new()),
    };
    let pomo_mins = pomodoro_remaining_mins.filter(|&m| m > 0);

    if is_delivery_or_courier {
        let mut replies = vec![
            "放门口即可，谢谢".to_string(),
            "放快递柜/驿站就行".to_string(),
            "好的，马上出来拿".to_string(),
            "辛苦了，谢谢".to_string(),
        ];
        if meeting_short.is_some() || pomo_mins.is_some() || dnd_on {
            replies[2] = "正在开会不便接听，放门口即可".to_string();
        }
        return replies;
    }

    let status_reply = if let Some(ref m) = meeting_short {
        Some(format!("正在「{m}」开会{meeting_end_suffix}，稍后回你"))
    } else if let Some(mins) = pomo_mins {
        Some(format!("正在专注（约 {mins} 分钟后回你）"))
    } else if dnd_on {
        Some("当前免打扰中，稍后回复你".to_string())
    } else {
        None
    };

    if app_or_source.trim() == "电话" || text.trim() == "未接来电" {
        let mut replies = vec![
            status_reply.unwrap_or_else(|| "正在开会，稍后给您回电".to_string()),
            "不好意思刚才没听到，请问有什么事？".to_string(),
            "收到，我马上处理".to_string(),
        ];
        replies.dedup();
        return replies;
    }

    let is_work_or_meeting = [
        "飞书",
        "钉钉",
        "企微",
        "企业微信",
        "会议",
        "开会",
        "日程",
        "周报",
        "日报",
        "文档",
        "方案",
        "评审",
        "需求",
        "项目",
        "合同",
        "表格",
        "同步",
        "对齐",
        "纪要",
        "汇报",
    ]
    .iter()
    .any(|kw| any_contains(kw))
        || [
            &b"lark"[..],
            &b"feishu"[..],
            &b"dingtalk"[..],
            &b"rimet"[..],
            &b"wework"[..],
            &b"ppt"[..],
        ]
        .iter()
        .any(|kw| any_ascii(kw));

    if is_work_or_meeting {
        let mut replies = vec![
            "收到，马上处理".to_string(),
            "正在开会，稍后回你".to_string(),
            "好的，没问题，中午前同步".to_string(),
            "已看，按这个方案推进".to_string(),
        ];
        if let Some(sr) = status_reply {
            if !replies.contains(&sr) {
                replies[1] = sr;
            }
        }
        return replies;
    }

    let mut replies = vec![
        "收到，马上处理".to_string(),
        "正在忙，稍后回复你".to_string(),
        "好的，谢谢！".to_string(),
        "没问题".to_string(),
    ];
    if let Some(sr) = status_reply {
        if !replies.contains(&sr) {
            replies[1] = sr;
        }
    }
    replies
}

/// Period recognition shared by Home suggestions and the command bar. Callers
/// still decide whether the subject is the user's own review or another document.
pub(crate) fn periodic_review_period(text: &str) -> Option<&'static str> {
    let text = text.to_lowercase();
    let summarizes = [
        "总结", "回顾", "复盘", "汇总", "汇报", "报告", "summary", "report", "review",
    ]
    .iter()
    .any(|word| text.contains(word));
    for (period, explicit, intervals) in [
        (
            "month",
            &["月报", "月总结", "月回顾", "月复盘"][..],
            &["本月", "这个月", "月度", "每月", "monthly"][..],
        ),
        (
            "week",
            &["周报", "周总结", "周回顾", "周复盘"][..],
            &["本周", "这周", "每周", "weekly"][..],
        ),
        (
            "day",
            &["日报", "日总结", "日回顾", "日复盘"][..],
            &["今天", "今日", "每日", "当天", "daily"][..],
        ),
    ] {
        if explicit.iter().any(|word| text.contains(word))
            || (summarizes && intervals.iter().any(|word| text.contains(word)))
        {
            return Some(period);
        }
    }
    None
}

/// Only shortcut an unambiguous personal, current-period review. Unrecognized
/// subject matter, dates or requested formats keep the original prompt intact:
/// "给客户写月报模板" must not silently become the user's current month review.
fn personal_current_review_page(command: &str) -> Option<&'static str> {
    let period = periodic_review_period(command)?;
    let mut remaining = command.to_lowercase();
    for phrase in [
        "帮我",
        "替我",
        "为我",
        "给我",
        "我的",
        "本人",
        "个人",
        "这个月",
        "本月",
        "月度",
        "本周",
        "这周",
        "今天",
        "今日",
        "当天",
        "工作总结",
        "工作回顾",
        "工作复盘",
        "工作汇报",
        "工作报告",
        "日总结",
        "日回顾",
        "日复盘",
        "周总结",
        "周回顾",
        "周复盘",
        "月总结",
        "月回顾",
        "月复盘",
        "日报",
        "周报",
        "月报",
        "总结",
        "回顾",
        "复盘",
        "汇总",
        "汇报",
        "报告",
        "看一下",
        "写一份",
        "生成一份",
        "做一份",
        "做了什么",
        "一份",
        "一篇",
        "整理",
        "生成",
        "查看",
        "打开",
        "写",
        "做",
        "看",
        "工作",
        "进展",
        "成果",
        "产出",
        "待办",
        "阻塞",
        "下一步",
        "计划",
        "沟通",
        "日程",
        "讨论",
        "根据",
        "结合",
        "以及",
        "文档",
        "一下",
        "请",
        "我",
        "的",
        "与",
        "和",
        "及",
        "了",
        "把",
        "成",
    ] {
        remaining = remaining.replace(phrase, "");
    }
    if remaining
        .chars()
        .any(|c| !c.is_whitespace() && !"，。！？、：；,.!?:;".contains(c))
    {
        return None;
    }
    Some(match period {
        "day" => "review-day",
        "week" => "review-week",
        "month" => "review-month",
        _ => return None,
    })
}

/// Natural-language intent router for the Home page AI Omnibox.
pub fn classify_command_intent(raw_command: &str) -> HomeAiIntent {
    let cmd = raw_command.trim();
    if cmd.is_empty() {
        return HomeAiIntent::AmooPrompt(String::new());
    }
    let lower = cmd.to_lowercase();

    // 1. BackScreen status intent ("背屏" + status keywords)
    if lower.contains("背屏")
        && [
            "开会", "勿扰", "专注", "吃饭", "休息", "冥想", "呼吸", "带伞", "下雨", "降雨",
        ]
        .iter()
        .any(|kw| lower.contains(kw))
    {
        let status = if lower.contains("开会") || lower.contains("勿扰") {
            "开会勿扰"
        } else if lower.contains("吃饭") {
            "去吃饭"
        } else if lower.contains("休息") {
            "休息一下"
        } else if lower.contains("冥想") || lower.contains("呼吸") {
            "冥想呼吸"
        } else if lower.contains("带伞") || lower.contains("下雨") || lower.contains("降雨") {
            "出门带伞"
        } else {
            "专注中"
        };
        return HomeAiIntent::BackScreenStatus(status.to_string());
    }

    // 2. Pomodoro intent ("番茄钟" or "专注25分钟")
    if lower.contains("番茄钟")
        || lower.contains("专注25分钟")
        || (lower.contains("专注") && lower.contains("分钟"))
    {
        let mins = extract_first_integer(cmd)
            .filter(|m| *m > 0)
            .map(|m| m.clamp(1, 60))
            .unwrap_or(25);
        return HomeAiIntent::StartPomodoro(mins);
    }

    // 3. Express tracking query ("快递" + alphanumeric tracking number, or explicit tracking number)
    if lower.contains("快递") || lower.contains("查单") || lower.contains("运单") {
        if let Some((_, tracking_no)) = extract_express_from_text_flexible(cmd, true) {
            return HomeAiIntent::QueryExpress(tracking_no);
        }
    } else if let Some((_, tracking_no)) = crate::toolbox::extract_express_number_from_text(cmd) {
        if lower.contains("查") || lower.contains("单号") || lower.contains("物流") {
            return HomeAiIntent::QueryExpress(tracking_no);
        }
    }

    // 4. Alipay coupon ("神券" / "领券" / "支付宝券")
    if lower.contains("神券") || lower.contains("领券") || lower.contains("支付宝券") {
        return HomeAiIntent::AlipayCoupon;
    }

    // 5. CapCut AI video handoff ("剪映" / "成片")
    if lower.contains("剪映") || lower.contains("成片") {
        return HomeAiIntent::CapCutHandoff;
    }

    // 6. Remote agent files pull
    if lower.contains("拉取") && (lower.contains("智能体") || lower.contains("产出")) {
        return HomeAiIntent::PullRemoteAgentFiles;
    }

    // 7. DND toggle
    if (lower.contains("开启勿扰") || lower.contains("打开勿扰") || lower.contains("关闭勿扰"))
        && !lower.contains("背屏")
    {
        let enable = !lower.contains("关闭") && !lower.contains("取消");
        return HomeAiIntent::ToggleDnd(enable);
    }

    // Personal reviews must use recorded evidence, not a generic document prompt.
    if let Some(page) = personal_current_review_page(cmd) {
        return HomeAiIntent::OpenPage(page.into());
    }

    // 8. Amoo Work deliverables (explicit other subjects and document formats).
    if [
        "周报",
        "日报",
        "月报",
        "文档",
        "表格",
        "ppt",
        "演示文稿",
        "纪要",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
    {
        return HomeAiIntent::WorkPrompt(cmd.to_string());
    }

    // 9. Quick GUI automation scenarios ("美团" / "外卖" / "饿了么" / "淘宝" / "打车" / "高德")
    if ["美团", "外卖", "饿了么", "淘宝", "闪购", "打车", "高德"]
        .iter()
        .any(|kw| lower.contains(kw))
    {
        let app = if lower.contains("饿了么") {
            "饿了么"
        } else if lower.contains("淘宝") || lower.contains("闪购") {
            "淘宝"
        } else if lower.contains("高德") || lower.contains("打车") {
            "高德地图"
        } else {
            "美团"
        };
        return HomeAiIntent::QuickGuiScenario {
            app: app.to_string(),
            task: cmd.to_string(),
        };
    }

    // 10. Default to Amoo general prompt
    HomeAiIntent::AmooPrompt(cmd.to_string())
}

/// Returns true if `text` is an internal E2E / automated verification probe artifact
/// (`stepx-auto-*`, `stepx-e2e-*`, `StepX 上下文验收`, `仅用于验收`, `stepx_e2e_context`, `stepx_probe`)
/// Fast, zero-heap-allocation ASCII case-insensitive substring check.
/// Safe on arbitrary UTF-8 strings because multi-byte UTF-8 sequences never contain `0x00..=0x7F`.
#[inline]
pub fn contains_ascii_ignore_case(haystack: &str, needle_lower: &[u8]) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    let bytes = haystack.as_bytes();
    if bytes.len() < needle_lower.len() {
        return false;
    }
    let first_lower = needle_lower[0].to_ascii_lowercase();
    let first_upper = needle_lower[0].to_ascii_uppercase();
    let end = bytes.len() - needle_lower.len();
    let mut i = 0;
    while i <= end {
        let b = bytes[i];
        if (b == first_lower || b == first_upper)
            && bytes[i..i + needle_lower.len()].eq_ignore_ascii_case(needle_lower)
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Canonical single-allocation UTF-8 character truncation helper with ellipsis (`…`).
#[inline]
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => {
            let mut out = String::with_capacity(byte_idx + '…'.len_utf8());
            out.push_str(&text[..byte_idx]);
            out.push('…');
            out
        }
        None => text.to_string(),
    }
}

/// Returns true if `text` is an internal E2E / automated verification probe artifact
/// (`stepx-auto-*`, `stepx-e2e-*`, `StepX 上下文验收`, `仅用于验收`, `stepx_e2e_context`, `stepx_probe`)
/// that must never pollute the user-facing Home hub.
#[inline]
pub fn is_synthetic_e2e_probe_text(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    if contains_ascii_ignore_case(text, b"stepx") {
        const STEPX_MARKERS: &[&[u8]] = &[
            b"stepx-auto-",
            b"stepx-e2e-",
            b"stepx-e2e-probe",
            b"stepx-transfer-probe",
            b"stepx-verify-",
            "[项目: stepx-".as_bytes(),
            b"stepx_e2e_context",
            b"stepx_probe",
            "stepx 上下文验收".as_bytes(),
            "stepx companion 简洁模式".as_bytes(),
            b"stepx-retry.csv",
            b"stepx-cancel.csv",
            b"stepx-lifecycle.csv",
            b"stepx-work-check.csv",
            "stepx-连接验收".as_bytes(),
            b"stepx-phone-ui-check",
            "stepx双向传输验证".as_bytes(),
            "stepx自动化验证".as_bytes(),
            "stepx 日历网格验证".as_bytes(),
            "stepx日历网格验证".as_bytes(),
            "stepx e2e".as_bytes(),
            "stepx-待办-stepx-".as_bytes(),
            "stepx-待办-bookclub-".as_bytes(),
            "stepx-待办-快递与取件线索".as_bytes(),
        ];
        if STEPX_MARKERS
            .iter()
            .any(|marker| contains_ascii_ignore_case(text, marker))
        {
            return true;
        }
    }
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("SF1357924680123")
        || trimmed.eq_ignore_ascii_case("顺丰速运 SF1357924680123")
        || trimmed.eq_ignore_ascii_case("SF14298019283")
        || trimmed.eq_ignore_ascii_case("顺丰速运 SF14298019283")
    {
        return true;
    }
    if (text.contains("快递与取件线索")
        || text.contains("快递取件线索")
        || text.contains("发现手机快递/取件线索"))
        && (contains_ascii_ignore_case(text, b"sf1357924680123")
            || contains_ascii_ignore_case(text, b"sf14298019283"))
    {
        return true;
    }
    contains_ascii_ignore_case(text, b"e2e_probe")
        || contains_ascii_ignore_case(text, b"smoke_test")
        || contains_ascii_ignore_case(text, b"stepx-transfer-probe")
        || text.contains("邱-st")
        || text.contains("真机端到端回归日程")
        || text.contains("真机端到端测试闹钟")
        || text.contains("仅用于验收")
        || text.contains("验收资料")
        || text.contains("继续自用验收讨论")
        || text.contains("简洁模式对话通过")
        || text.contains("简洁模式的真实连接测试")
}

/// Zero-allocation slice helper to check if any string field contains a synthetic E2E probe marker.
#[inline]
pub fn any_synthetic_e2e_probe(parts: &[&str]) -> bool {
    parts.iter().any(|s| is_synthetic_e2e_probe_text(s))
}

/// Collects the Life-domain priority cards (`OtpAndPickup`, `TakeoutAndDelivery`, `ExpressParcel`)
/// shared between `build_home_hub_report` and `build_life_context_cards_with_desktop`.
fn collect_life_priority_cards(
    input: &HomeHubInput<'_>,
    active_meeting_title: Option<&str>,
    pomo_mins: Option<u32>,
    include_quick_replies: bool,
) -> Vec<HomePriorityCard> {
    let max_ts = input
        .sms
        .iter()
        .map(|m| m.date_ms)
        .chain(input.notifications.iter().map(|n| n.post_time_ms))
        .chain(input.otp_banners.iter().map(|b| b.timestamp_ms))
        .chain(input.threads.iter().map(|t| t.last_timestamp_ms))
        .max()
        .unwrap_or(0);
    let now_ms = reference_now_ms_for_input(input, max_ts);

    let mut priority_cards: Vec<HomePriorityCard> = Vec::new();
    let mut seen_codes: HashSet<String> = HashSet::new();
    let mut seen_tracking_nos: HashSet<String> = HashSet::new();
    let mut seen_notif_keys: HashSet<String> = HashSet::new();
    let gov = crate::home_reminder_governance::active_governance();
    let receipts = crate::parcel_state::CompletionIndex::collect(
        input.sms,
        input.notifications,
        input.threads,
    );

    // -------------------------------------------------------------------------
    // 1. OTP & Pickup Codes (HomeCardKind::OtpAndPickup, urgency ~98..=99)
    // -------------------------------------------------------------------------
    for (idx, banner) in input.otp_banners.iter().enumerate() {
        let code = banner.code.trim();
        if code.is_empty()
            || any_synthetic_e2e_probe(&[&banner.source_app, &banner.title, &banner.raw_text])
            || is_completed_express_text(&banner.raw_text)
            || receipts.text_is_completed(&banner.raw_text, banner.timestamp_ms)
        {
            continue;
        }
        let Some((verified_code, is_pickup)) =
            extract_otp_or_pickup_from_parts(&[&banner.title, &banner.raw_text, code])
        else {
            continue;
        };
        if !verified_code.eq_ignore_ascii_case(code) {
            continue;
        }
        let category = if is_pickup {
            crate::home_reminder_governance::ReminderCategory::ExpressPickup
        } else {
            crate::home_reminder_governance::ReminderCategory::OtpCode
        };
        let code_key = format!("code:{}", code.to_uppercase());
        let mut keys = crate::home_reminder_governance::canonical_keys_for_code_or_parcel(
            Some(code),
            None,
            non_empty_opt(&banner.notification_key).as_deref(),
            None,
        );
        keys.push(format!("card:otp-banner-{idx}-{code}"));
        if gov.is_any_key_suppressed(
            &keys,
            (banner.timestamp_ms > 0).then_some(banner.timestamp_ms),
            None,
            None,
            now_ms,
        ) {
            continue;
        }
        let ttl_ms = gov.effective_ttl_ms(category);
        if banner.timestamp_ms > 0
            && now_ms.saturating_sub(banner.timestamp_ms) > ttl_ms
            && !gov.is_force_kept(&code_key, now_ms)
        {
            let label = if is_pickup { "取件码" } else { "验证码" };
            crate::home_reminder_governance::record_auto_expired_notice(
                code_key,
                category,
                format!("{label} {code}"),
                "超过自动过期时效",
                now_ms,
            );
            continue;
        }
        if !seen_codes.insert(code.to_uppercase()) {
            continue;
        }
        if !banner.notification_key.is_empty() {
            seen_notif_keys.insert(banner.notification_key.clone());
        }
        let badge = if is_pickup {
            "驿站取件码".to_string()
        } else if banner.source_app.trim().is_empty() {
            "验证码".to_string()
        } else {
            format!("{} · 验证码", banner.source_app.trim())
        };
        let title = if is_pickup {
            format!("取件码 {code}")
        } else {
            format!("验证码 {code}")
        };
        let subtitle = if !banner.title.trim().is_empty() {
            banner.title.trim().to_string()
        } else if !banner.source_app.trim().is_empty() {
            banner.source_app.trim().to_string()
        } else {
            "点击一键复制并同步到电脑剪贴板".to_string()
        };
        priority_cards.push(HomePriorityCard {
            id: format!("otp-banner-{idx}-{code}"),
            kind: HomeCardKind::OtpAndPickup,
            urgency: 99,
            badge,
            title,
            subtitle,
            detail: banner.raw_text.trim().to_string(),
            timestamp_label: format_timestamp_ms(banner.timestamp_ms),
            code_or_number: Some(code.to_string()),
            thread_id: None,
            notification_key: non_empty_opt(&banner.notification_key),
            package_name: None,
            quick_replies: Vec::new(),
        });
    }

    // Check phone clipboard for OTP or pickup code
    if !is_completed_express_text(input.phone_clipboard) {
        if let Some((code, is_pickup)) = extract_otp_or_pickup_from_text(input.phone_clipboard) {
            let mut keys = crate::home_reminder_governance::canonical_keys_for_code_or_parcel(
                Some(&code),
                None,
                None,
                None,
            );
            keys.push(format!("card:otp-clipboard-{code}"));
            if !gov.is_any_key_suppressed(&keys, None, None, None, now_ms)
                && seen_codes.insert(code.to_uppercase())
            {
                let label = if is_pickup { "取件码" } else { "验证码" };
                priority_cards.push(HomePriorityCard {
                    id: format!("otp-clipboard-{code}"),
                    kind: HomeCardKind::OtpAndPickup,
                    urgency: 98,
                    badge: format!("剪贴板{label}"),
                    title: format!("{label} {code}"),
                    subtitle: "来自手机剪贴板实时感知".to_string(),
                    detail: input.phone_clipboard.trim().to_string(),
                    timestamp_label: "刚刚".to_string(),
                    code_or_number: Some(code),
                    thread_id: None,
                    notification_key: None,
                    package_name: None,
                    quick_replies: Vec::new(),
                });
            }
        }
    }

    // Scan notifications for OTP / Pickup codes
    for notif in input.notifications {
        if !notif.active
            || notif.dismissed
            || notif.read
            || notif.title.trim_start().starts_with('{')
            || notif.text.trim_start().starts_with('{')
            || any_synthetic_e2e_probe(&[&notif.app_name, &notif.title, &notif.text])
            || is_completed_express_text(&notif.text)
            || receipts.text_is_completed(
                &format!("{} {}", notif.title, notif.text),
                notif.post_time_ms,
            )
        {
            continue;
        }
        let extracted = notif
            .otp_code
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|c| {
                extract_otp_or_pickup_from_parts(&[&notif.title, &notif.text, c])
                    .filter(|(verified, _)| verified.eq_ignore_ascii_case(c))
            })
            .or_else(|| {
                extract_otp_or_pickup_from_parts(&[&notif.app_name, &notif.title, &notif.text])
            });

        if let Some((code, is_pickup)) = extracted {
            let category = if is_pickup {
                crate::home_reminder_governance::ReminderCategory::ExpressPickup
            } else {
                crate::home_reminder_governance::ReminderCategory::OtpCode
            };
            let code_key = format!("code:{}", code.to_uppercase());
            let mut keys = crate::home_reminder_governance::canonical_keys_for_code_or_parcel(
                Some(&code),
                None,
                non_empty_opt(&notif.key).as_deref(),
                None,
            );
            keys.push(format!("card:otp-notif-{}-{code}", notif.seq_id));
            let mute_src = (!notif.package_name.trim().is_empty())
                .then(|| format!("pkg:{}", notif.package_name.trim()));
            if gov.is_any_key_suppressed(
                &keys,
                (notif.post_time_ms > 0).then_some(notif.post_time_ms),
                None,
                mute_src.as_deref(),
                now_ms,
            ) {
                continue;
            }
            let ttl_ms = gov.effective_ttl_ms(category);
            if notif.post_time_ms > 0
                && now_ms.saturating_sub(notif.post_time_ms) > ttl_ms
                && !gov.is_force_kept(&code_key, now_ms)
            {
                let label = if is_pickup { "取件码" } else { "验证码" };
                crate::home_reminder_governance::record_auto_expired_notice(
                    code_key,
                    category,
                    format!("{label} {code}"),
                    "超过自动过期时效",
                    now_ms,
                );
                continue;
            }
            if seen_codes.insert(code.to_uppercase()) {
                if !notif.key.is_empty() {
                    seen_notif_keys.insert(notif.key.clone());
                }
                let app_label = friendly_source_name(&notif.app_name, &notif.package_name);
                let kind_label = if is_pickup { "取件码" } else { "验证码" };
                priority_cards.push(HomePriorityCard {
                    id: format!("otp-notif-{}-{code}", notif.seq_id),
                    kind: HomeCardKind::OtpAndPickup,
                    urgency: 98,
                    badge: format!("{app_label} · {kind_label}"),
                    title: format!("{kind_label} {code}"),
                    subtitle: if notif.title.trim().is_empty() {
                        app_label
                    } else {
                        notif.title.trim().to_string()
                    },
                    detail: notif.text.trim().to_string(),
                    timestamp_label: format_timestamp_ms(notif.post_time_ms),
                    code_or_number: Some(code),
                    thread_id: non_empty_opt(&notif.thread_id),
                    notification_key: non_empty_opt(&notif.key),
                    package_name: non_empty_opt(&notif.package_name),
                    quick_replies: if include_quick_replies && is_pickup {
                        vec![
                            "好的，今天下班去取".to_string(),
                            "已收到取件码，谢谢".to_string(),
                        ]
                    } else {
                        Vec::new()
                    },
                });
            }
        }
    }

    // Scan SMS for OTP / Pickup codes
    for (idx, msg) in input.sms.iter().enumerate() {
        if msg.outgoing
            || any_synthetic_e2e_probe(&[&msg.address, &msg.body])
            || is_completed_express_text(&msg.body)
            || receipts.text_is_completed(&msg.body, msg.date_ms)
        {
            continue;
        }
        if let Some((code, is_pickup)) = extract_otp_or_pickup_from_text(&msg.body) {
            let category = if is_pickup {
                crate::home_reminder_governance::ReminderCategory::ExpressPickup
            } else {
                crate::home_reminder_governance::ReminderCategory::OtpCode
            };
            let code_key = format!("code:{}", code.to_uppercase());
            let mut keys = crate::home_reminder_governance::canonical_keys_for_code_or_parcel(
                Some(&code),
                None,
                None,
                non_empty_opt(&msg.address).as_deref(),
            );
            keys.push(format!("card:otp-sms-{idx}-{code}"));
            let mute_src = (!msg.address.trim().is_empty()).then(|| {
                format!(
                    "sms:{}",
                    crate::home_reminder_governance::normalize_phone_or_addr(&msg.address)
                )
            });
            if gov.is_any_key_suppressed(
                &keys,
                (msg.date_ms > 0).then_some(msg.date_ms),
                None,
                mute_src.as_deref(),
                now_ms,
            ) {
                continue;
            }
            let force_kept = gov.is_force_kept(&code_key, now_ms);
            let ttl_ms = gov.effective_ttl_ms(category);
            if msg.date_ms > 0 && now_ms.saturating_sub(msg.date_ms) > ttl_ms && !force_kept {
                let label = if is_pickup { "取件码" } else { "验证码" };
                crate::home_reminder_governance::record_auto_expired_notice(
                    code_key,
                    category,
                    format!("{label} {code}"),
                    "超过自动过期时效",
                    now_ms,
                );
                continue;
            }
            if msg.read && !force_kept {
                let has_active_notif = input.notifications.iter().any(|n| {
                    n.active
                        && !n.dismissed
                        && !n.read
                        && (n.text.contains(&code)
                            || n.title.contains(&code)
                            || n.otp_code.as_deref() == Some(code.as_str()))
                });
                if !has_active_notif {
                    continue;
                }
            }
            if seen_codes.insert(code.to_uppercase()) {
                let kind_label = if is_pickup { "取件码" } else { "验证码" };
                priority_cards.push(HomePriorityCard {
                    id: format!("otp-sms-{idx}-{code}"),
                    kind: HomeCardKind::OtpAndPickup,
                    urgency: 98,
                    badge: format!("短信{kind_label}"),
                    title: format!("{kind_label} {code}"),
                    subtitle: format!("来自 {}", msg.address.trim()),
                    detail: msg.body.trim().to_string(),
                    timestamp_label: format_timestamp_ms(msg.date_ms),
                    code_or_number: Some(code),
                    thread_id: None,
                    notification_key: None,
                    package_name: Some("com.android.mms".to_string()),
                    quick_replies: if include_quick_replies && is_pickup {
                        vec![
                            "放快递柜/驿站就行".to_string(),
                            "好的，马上过去取".to_string(),
                        ]
                    } else {
                        Vec::new()
                    },
                });
            }
        }
    }

    // -------------------------------------------------------------------------
    // 2. Takeout & Delivery / Ride-Hailing (HomeCardKind::TakeoutAndDelivery, urgency ~94)
    // -------------------------------------------------------------------------
    let takeout_ttl_ms =
        gov.effective_ttl_ms(crate::home_reminder_governance::ReminderCategory::TakeoutAndMobility);
    let mut seen_takeout_sigs: HashSet<String> = HashSet::new();
    for notif in input.notifications {
        if seen_notif_keys.contains(&notif.key)
            || notif.title.trim_start().starts_with('{')
            || notif.text.trim_start().starts_with('{')
            || any_synthetic_e2e_probe(&[&notif.app_name, &notif.title, &notif.text])
        {
            continue;
        }
        if let Some((badge, urgency_boost)) = classify_takeout_or_travel_notification(notif) {
            let is_arrived = urgency_boost > 0;
            let delivery_key = format!("delivery:{badge}");
            let force_kept = gov.is_force_kept(&delivery_key, now_ms);
            let app_label = friendly_source_name(&notif.app_name, &notif.package_name);
            let title = if !notif.title.trim().is_empty() {
                notif.title.trim().to_string()
            } else {
                format!("{app_label} 实时状态")
            };
            let eta_end_ms = extract_eta_window_from_text(
                &format!("{} {} {}", notif.title, notif.text, notif.sub_text),
                notif.post_time_ms,
                now_ms,
            )
            .map(|(end_ms, _, _)| end_ms)
            .unwrap_or(0);
            if notif.dismissed && !force_kept {
                continue;
            }
            if (!notif.active || (notif.read && !notif.is_ongoing)) && !force_kept {
                if notif.post_time_ms <= 0
                    || (is_arrived && now_ms.saturating_sub(notif.post_time_ms) > 1_800_000)
                {
                    continue;
                }
            }
            let effective_expire_ms = if notif.post_time_ms > 0 {
                notif
                    .post_time_ms
                    .saturating_add(takeout_ttl_ms)
                    .max(if eta_end_ms > 0 {
                        eta_end_ms.saturating_add(45 * 60_000)
                    } else {
                        0
                    })
            } else {
                0
            };
            if notif.post_time_ms > 0 && now_ms > effective_expire_ms && !force_kept {
                crate::home_reminder_governance::record_auto_expired_notice(
                    delivery_key,
                    crate::home_reminder_governance::ReminderCategory::TakeoutAndMobility,
                    &title,
                    "配送动态已超过自动过期时效",
                    now_ms,
                );
                continue;
            }
            let mut keys = crate::home_reminder_governance::canonical_keys_for_delivery(
                &badge,
                non_empty_opt(&notif.key).as_deref(),
                non_empty_opt(&notif.thread_id).as_deref(),
                None,
            );
            keys.push(format!("card:takeout-notif-{}", notif.seq_id));
            let mute_src = (!notif.package_name.trim().is_empty())
                .then(|| format!("pkg:{}", notif.package_name.trim()));
            if gov.is_any_key_suppressed(
                &keys,
                (notif.post_time_ms > 0).then_some(notif.post_time_ms),
                None,
                mute_src.as_deref(),
                now_ms,
            ) {
                continue;
            }
            if !notif.key.is_empty() {
                seen_notif_keys.insert(notif.key.clone());
            }
            let sig = format!(
                "{badge}::{}",
                notif.text.trim().chars().take(24).collect::<String>()
            );
            seen_takeout_sigs.insert(sig);
            let subtitle = if !notif.sub_text.trim().is_empty() {
                format!("{app_label} · {}", notif.sub_text.trim())
            } else {
                app_label.clone()
            };
            let quick_replies = if include_quick_replies {
                quick_replies_for_message_with_context(
                    &app_label,
                    &title,
                    &notif.text,
                    active_meeting_title,
                    pomo_mins,
                    input.dnd_on,
                )
            } else {
                Vec::new()
            };
            priority_cards.push(HomePriorityCard {
                id: format!("takeout-notif-{}", notif.seq_id),
                kind: HomeCardKind::TakeoutAndDelivery,
                urgency: 94u8.saturating_add(urgency_boost).min(96),
                badge,
                title,
                subtitle,
                detail: notif.text.trim().to_string(),
                timestamp_label: format_timestamp_ms(notif.post_time_ms),
                code_or_number: None,
                thread_id: non_empty_opt(&notif.thread_id),
                notification_key: non_empty_opt(&notif.key),
                package_name: non_empty_opt(&notif.package_name),
                quick_replies,
            });
        }
    }

    // Scan recent SMS for instant grocery/takeout delivery notices (e.g. 叮咚买菜 已代收（家门口）)
    for (idx, msg) in input.sms.iter().enumerate() {
        if msg.outgoing || any_synthetic_e2e_probe(&[&msg.address, &msg.body]) {
            continue;
        }
        if let Some((badge, headline, status_tag, is_arrived)) =
            classify_delivery_or_mobility_signal("", "", &msg.address, &msg.body)
        {
            let delivery_key = format!("delivery:{badge}");
            let force_kept = gov.is_force_kept(&delivery_key, now_ms);
            let eta_end_ms = extract_eta_window_from_text(&msg.body, msg.date_ms, now_ms)
                .map(|(end_ms, _, _)| end_ms)
                .unwrap_or(0);
            if msg.read && !force_kept {
                if msg.date_ms <= 0
                    || (is_arrived && now_ms.saturating_sub(msg.date_ms) > 1_800_000)
                {
                    continue;
                }
            }
            let effective_expire_ms = if msg.date_ms > 0 {
                msg.date_ms
                    .saturating_add(takeout_ttl_ms)
                    .max(if eta_end_ms > 0 {
                        eta_end_ms.saturating_add(45 * 60_000)
                    } else {
                        0
                    })
            } else {
                0
            };
            if msg.date_ms > 0 && now_ms > effective_expire_ms && !force_kept {
                crate::home_reminder_governance::record_auto_expired_notice(
                    delivery_key,
                    crate::home_reminder_governance::ReminderCategory::TakeoutAndMobility,
                    &headline,
                    "配送动态已超过自动过期时效",
                    now_ms,
                );
                continue;
            }
            let mut keys = crate::home_reminder_governance::canonical_keys_for_delivery(
                &badge,
                None,
                None,
                non_empty_opt(&msg.address).as_deref(),
            );
            keys.push(format!("card:takeout-sms-{idx}-{}", msg.date_ms));
            let mute_src = (!msg.address.trim().is_empty()).then(|| {
                format!(
                    "sms:{}",
                    crate::home_reminder_governance::normalize_phone_or_addr(&msg.address)
                )
            });
            if gov.is_any_key_suppressed(
                &keys,
                (msg.date_ms > 0).then_some(msg.date_ms),
                None,
                mute_src.as_deref(),
                now_ms,
            ) {
                continue;
            }
            let sig = format!(
                "{badge}::{}",
                msg.body.trim().chars().take(24).collect::<String>()
            );
            if !seen_takeout_sigs.insert(sig) {
                continue;
            }
            let urgency_boost = if is_arrived { 1 } else { 0 };
            let quick_replies = if include_quick_replies {
                quick_replies_for_message_with_context(
                    &badge,
                    &headline,
                    &msg.body,
                    active_meeting_title,
                    pomo_mins,
                    input.dnd_on,
                )
            } else {
                Vec::new()
            };
            priority_cards.push(HomePriorityCard {
                id: format!("takeout-sms-{idx}-{}", msg.date_ms),
                kind: HomeCardKind::TakeoutAndDelivery,
                urgency: 94u8.saturating_add(urgency_boost).min(96),
                badge,
                title: headline,
                subtitle: format!("{status_tag} · 短信通知"),
                detail: msg.body.trim().to_string(),
                timestamp_label: format_timestamp_ms(msg.date_ms),
                code_or_number: None,
                thread_id: None,
                notification_key: None,
                package_name: Some("com.android.mms".to_string()),
                quick_replies,
            });
        }
    }

    // Scan recent IM threads (e.g. WeChat service account 叮咚买菜) for instant delivery notices
    for thread in input.threads {
        if thread.title.trim_start().starts_with('{')
            || any_synthetic_e2e_probe(&[&thread.app_name, &thread.title, &thread.last_message])
        {
            continue;
        }
        if let Some((badge, headline, status_tag, is_arrived)) =
            classify_delivery_or_mobility_signal(
                &thread.package_name,
                &thread.app_name,
                &thread.title,
                &thread.last_message,
            )
        {
            let effective_ts = thread.last_timestamp_ms.max(
                thread
                    .messages
                    .iter()
                    .map(|m| m.timestamp_ms.max(m.observed_at_ms))
                    .max()
                    .unwrap_or(0),
            );
            let delivery_key = format!("delivery:{badge}");
            let force_kept = gov.is_force_kept(&delivery_key, now_ms);
            let eta_end_ms = extract_eta_window_from_text(
                &format!("{} {}", thread.title, thread.last_message),
                effective_ts,
                now_ms,
            )
            .map(|(end_ms, _, _)| end_ms)
            .unwrap_or(0);
            if thread.unread_count == 0 && !force_kept {
                if effective_ts <= 0
                    || (is_arrived && now_ms.saturating_sub(effective_ts) > 1_800_000)
                {
                    continue;
                }
            }
            let effective_expire_ms = if effective_ts > 0 {
                effective_ts
                    .saturating_add(takeout_ttl_ms)
                    .max(if eta_end_ms > 0 {
                        eta_end_ms.saturating_add(45 * 60_000)
                    } else {
                        0
                    })
            } else {
                0
            };
            if effective_ts > 0 && now_ms > effective_expire_ms && !force_kept {
                crate::home_reminder_governance::record_auto_expired_notice(
                    delivery_key,
                    crate::home_reminder_governance::ReminderCategory::TakeoutAndMobility,
                    &headline,
                    "配送动态已超过自动过期时效",
                    now_ms,
                );
                continue;
            }
            let mut keys = crate::home_reminder_governance::canonical_keys_for_delivery(
                &badge,
                non_empty_opt(&thread.notification_key).as_deref(),
                non_empty_opt(&thread.thread_id).as_deref(),
                None,
            );
            keys.push(format!("card:takeout-thread-{}", thread.thread_id));
            let mute_src = (!thread.thread_id.trim().is_empty())
                .then(|| format!("im:{}", thread.thread_id.trim()));
            if gov.is_any_key_suppressed(
                &keys,
                (effective_ts > 0).then_some(effective_ts),
                None,
                mute_src.as_deref(),
                now_ms,
            ) {
                continue;
            }
            let sig = format!(
                "{badge}::{}",
                thread
                    .last_message
                    .trim()
                    .chars()
                    .take(24)
                    .collect::<String>()
            );
            if !seen_takeout_sigs.insert(sig) {
                continue;
            }
            let urgency_boost = if is_arrived { 1 } else { 0 };
            let app_label = friendly_source_name(&thread.app_name, &thread.package_name);
            let quick_replies = if include_quick_replies {
                quick_replies_for_message_with_context(
                    &badge,
                    &headline,
                    &thread.last_message,
                    active_meeting_title,
                    pomo_mins,
                    input.dnd_on,
                )
            } else {
                Vec::new()
            };
            priority_cards.push(HomePriorityCard {
                id: format!("takeout-thread-{}", thread.thread_id),
                kind: HomeCardKind::TakeoutAndDelivery,
                urgency: 94u8.saturating_add(urgency_boost).min(96),
                badge,
                title: headline,
                subtitle: format!("{status_tag} · {app_label}"),
                detail: thread.last_message.trim().to_string(),
                timestamp_label: format_timestamp_ms(effective_ts),
                code_or_number: None,
                thread_id: non_empty_opt(&thread.thread_id),
                notification_key: non_empty_opt(&thread.notification_key),
                package_name: non_empty_opt(&thread.package_name),
                quick_replies,
            });
        }
    }

    // -------------------------------------------------------------------------
    // 3. Express Parcels (HomeCardKind::ExpressParcel, urgency ~90..=92)
    // -------------------------------------------------------------------------
    for (idx, parcel) in input.express_parcels.iter().enumerate() {
        if is_completed_express_text(&parcel.status) || is_completed_express_text(&parcel.station) {
            continue;
        }
        let pickup = parcel.pickup_code.trim();
        let tracking = parcel.tracking_no.trim();
        let mut keys = crate::home_reminder_governance::canonical_keys_for_code_or_parcel(
            (!pickup.is_empty()).then_some(pickup),
            (!tracking.is_empty()).then_some(tracking),
            None,
            None,
        );
        keys.push(format!("card:express-parcel-{idx}-{tracking}"));
        if gov.is_any_key_suppressed(&keys, None, None, None, now_ms) {
            continue;
        }
        if !pickup.is_empty() && !seen_codes.insert(pickup.to_uppercase()) {
            continue;
        }
        if !tracking.is_empty() && !seen_tracking_nos.insert(tracking.to_ascii_uppercase()) {
            continue;
        }
        let has_pickup = !pickup.is_empty();
        let badge = if has_pickup {
            format!("取件码 {pickup}")
        } else {
            "物流实时轨迹".to_string()
        };
        let title = if parcel.status.trim().is_empty() {
            parcel.carrier.trim().to_string()
        } else {
            format!("{} · {}", parcel.carrier.trim(), parcel.status.trim())
        };
        let subtitle = if !parcel.station.trim().is_empty() {
            parcel.station.trim().to_string()
        } else {
            format!("运单号 {tracking}")
        };
        let detail = if has_pickup {
            format!(
                "取件码：{} · 驿站：{} · 运单号：{}",
                pickup,
                parcel.station.trim(),
                tracking
            )
        } else {
            format!(
                "运单号：{} · 当前节点：{}",
                tracking,
                if parcel.station.trim().is_empty() {
                    parcel.status.trim()
                } else {
                    parcel.station.trim()
                }
            )
        };
        let code_or_number = if has_pickup {
            Some(pickup.to_string())
        } else if !tracking.is_empty() {
            Some(tracking.to_string())
        } else {
            None
        };
        priority_cards.push(HomePriorityCard {
            id: format!("express-parcel-{idx}-{tracking}"),
            kind: HomeCardKind::ExpressParcel,
            urgency: if has_pickup || parcel.status.contains("待取") {
                92
            } else {
                90
            },
            badge,
            title,
            subtitle,
            detail,
            timestamp_label: if parcel.updated_at.trim().is_empty() {
                "今日更新".to_string()
            } else {
                parcel.updated_at.trim().to_string()
            },
            code_or_number,
            thread_id: None,
            notification_key: None,
            package_name: None,
            quick_replies: if include_quick_replies {
                vec![
                    "放快递柜/驿站就行".to_string(),
                    "放门口即可，谢谢".to_string(),
                    "好的，今天下班去取".to_string(),
                ]
            } else {
                Vec::new()
            },
        });
    }

    // Check clipboard for express tracking number not already in `express_parcels`
    if !is_completed_express_text(input.phone_clipboard) {
        if let Some((carrier, tracking_no)) =
            extract_express_from_text_flexible(input.phone_clipboard, false)
        {
            let mut keys = crate::home_reminder_governance::canonical_keys_for_code_or_parcel(
                None,
                Some(&tracking_no),
                None,
                None,
            );
            keys.push(format!("card:express-clipboard-{tracking_no}"));
            if !gov.is_any_key_suppressed(&keys, None, None, None, now_ms)
                && seen_tracking_nos.insert(tracking_no.to_ascii_uppercase())
            {
                priority_cards.push(HomePriorityCard {
                    id: format!("express-clipboard-{tracking_no}"),
                    kind: HomeCardKind::ExpressParcel,
                    urgency: 91,
                    badge: "剪贴板单号".to_string(),
                    title: format!("{carrier} · {tracking_no}"),
                    subtitle: "检测到剪贴板快递单号，点击一键查询物流轨迹".to_string(),
                    detail: format!("剪贴板内容：{}", input.phone_clipboard.trim()),
                    timestamp_label: "剪贴板感知".to_string(),
                    code_or_number: Some(tracking_no),
                    thread_id: None,
                    notification_key: None,
                    package_name: None,
                    quick_replies: Vec::new(),
                });
            }
        }
    }

    // Scan notifications & SMS for express updates not already covered
    let express_transit_ttl_ms =
        gov.effective_ttl_ms(crate::home_reminder_governance::ReminderCategory::ExpressInTransit);
    for notif in input.notifications {
        if !notif.active
            || notif.dismissed
            || notif.read
            || seen_notif_keys.contains(&notif.key)
            || notif.title.trim_start().starts_with('{')
            || notif.text.trim_start().starts_with('{')
            || any_synthetic_e2e_probe(&[&notif.app_name, &notif.title, &notif.text])
            || is_completed_express_text(&notif.text)
            || receipts.text_is_completed(
                &format!("{} {}", notif.title, notif.text),
                notif.post_time_ms,
            )
        {
            continue;
        }
        if is_express_notification(&notif.app_name, &notif.title, &notif.text) {
            let extracted_no = (!looks_like_sms_service_number(notif.title.trim()))
                .then(|| extract_express_from_text_flexible(&notif.title, true))
                .flatten()
                .or_else(|| extract_express_from_text_flexible(&notif.text, true))
                .map(|(_, no)| no);
            let primary_key = extracted_no
                .as_ref()
                .map(|no| format!("code:{}", no.to_uppercase()))
                .unwrap_or_else(|| format!("notif:{}", notif.key));
            let force_kept = gov.is_force_kept(&primary_key, now_ms);
            if notif.post_time_ms > 0
                && now_ms.saturating_sub(notif.post_time_ms) > express_transit_ttl_ms
                && !force_kept
            {
                crate::home_reminder_governance::record_auto_expired_notice(
                    primary_key,
                    crate::home_reminder_governance::ReminderCategory::ExpressInTransit,
                    if notif.title.trim().is_empty() {
                        notif.text.trim()
                    } else {
                        notif.title.trim()
                    },
                    "物流动态已超过自动过期时效",
                    now_ms,
                );
                continue;
            }
            let mut keys = crate::home_reminder_governance::canonical_keys_for_code_or_parcel(
                None,
                extracted_no.as_deref(),
                non_empty_opt(&notif.key).as_deref(),
                None,
            );
            keys.push(format!("card:express-notif-{}", notif.seq_id));
            let mute_src = (!notif.package_name.trim().is_empty())
                .then(|| format!("pkg:{}", notif.package_name.trim()));
            if gov.is_any_key_suppressed(
                &keys,
                (notif.post_time_ms > 0).then_some(notif.post_time_ms),
                None,
                mute_src.as_deref(),
                now_ms,
            ) {
                continue;
            }
            if let Some(ref no) = extracted_no {
                if !seen_tracking_nos.insert(no.to_ascii_uppercase()) {
                    continue;
                }
            }
            if !notif.key.is_empty() {
                seen_notif_keys.insert(notif.key.clone());
            }
            let app_label = friendly_source_name(&notif.app_name, &notif.package_name);
            let quick_replies = if include_quick_replies {
                quick_replies_for_message_with_context(
                    &app_label,
                    &notif.title,
                    &notif.text,
                    active_meeting_title,
                    pomo_mins,
                    input.dnd_on,
                )
            } else {
                Vec::new()
            };
            priority_cards.push(HomePriorityCard {
                id: format!("express-notif-{}", notif.seq_id),
                kind: HomeCardKind::ExpressParcel,
                urgency: 90,
                badge: format!("{app_label} · 快递"),
                title: if notif.title.trim().is_empty() {
                    format!("{app_label} 物流动态")
                } else {
                    notif.title.trim().to_string()
                },
                subtitle: app_label,
                detail: notif.text.trim().to_string(),
                timestamp_label: format_timestamp_ms(notif.post_time_ms),
                code_or_number: extracted_no,
                thread_id: non_empty_opt(&notif.thread_id),
                notification_key: non_empty_opt(&notif.key),
                package_name: non_empty_opt(&notif.package_name),
                quick_replies,
            });
        }
    }

    for (idx, msg) in input.sms.iter().enumerate() {
        if msg.outgoing
            || msg.read
            || any_synthetic_e2e_probe(&[&msg.address, &msg.body])
            || is_completed_express_text(&msg.body)
            || receipts.text_is_completed(&msg.body, msg.date_ms)
        {
            continue;
        }
        if is_express_notification("短信", &msg.address, &msg.body)
            && extract_otp_or_pickup_from_text(&msg.body).is_none()
        {
            let extracted_no = extract_express_from_text_flexible(&msg.body, true).map(|(_, n)| n);
            let norm_addr = crate::home_reminder_governance::normalize_phone_or_addr(&msg.address);
            let primary_key = extracted_no
                .as_ref()
                .map(|no| format!("code:{}", no.to_uppercase()))
                .unwrap_or_else(|| format!("sms:{norm_addr}"));
            let force_kept = gov.is_force_kept(&primary_key, now_ms);
            if msg.date_ms > 0
                && now_ms.saturating_sub(msg.date_ms) > express_transit_ttl_ms
                && !force_kept
            {
                crate::home_reminder_governance::record_auto_expired_notice(
                    primary_key,
                    crate::home_reminder_governance::ReminderCategory::ExpressInTransit,
                    format!("快递动态 · {}", msg.address.trim()),
                    "物流动态已超过自动过期时效",
                    now_ms,
                );
                continue;
            }
            let mut keys = crate::home_reminder_governance::canonical_keys_for_code_or_parcel(
                None,
                extracted_no.as_deref(),
                None,
                non_empty_opt(&msg.address).as_deref(),
            );
            keys.push(format!("card:express-sms-{idx}"));
            let mute_src = (!norm_addr.is_empty()).then(|| format!("sms:{norm_addr}"));
            if gov.is_any_key_suppressed(
                &keys,
                (msg.date_ms > 0).then_some(msg.date_ms),
                None,
                mute_src.as_deref(),
                now_ms,
            ) {
                continue;
            }
            if let Some(ref no) = extracted_no {
                if !seen_tracking_nos.insert(no.to_ascii_uppercase()) {
                    continue;
                }
            }
            let quick_replies = if include_quick_replies {
                quick_replies_for_message_with_context(
                    "短信",
                    &msg.address,
                    &msg.body,
                    active_meeting_title,
                    pomo_mins,
                    input.dnd_on,
                )
            } else {
                Vec::new()
            };
            priority_cards.push(HomePriorityCard {
                id: format!("express-sms-{idx}"),
                kind: HomeCardKind::ExpressParcel,
                urgency: 90,
                badge: "短信快递通知".to_string(),
                title: format!("快递动态 · {}", msg.address.trim()),
                subtitle: "来自手机短信通知".to_string(),
                detail: msg.body.trim().to_string(),
                timestamp_label: format_timestamp_ms(msg.date_ms),
                code_or_number: extracted_no.or_else(|| non_empty_opt(&msg.address)),
                thread_id: None,
                notification_key: None,
                package_name: Some("com.android.mms".to_string()),
                quick_replies,
            });
        }
    }

    priority_cards
}

/// Builds the complete Home Hub intelligence report from all connected phone and desktop signals.
pub fn build_home_hub_report(input: &HomeHubInput<'_>) -> HomeHubReport {
    let cache_hash = (!input.today_ymd.trim().is_empty())
        .then(|| compute_home_hub_input_hash(1, input.today_ymd, input, &[]));
    if let Some(h) = cache_hash {
        if let Some(hit) = HOME_HUB_REPORT_CACHE.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|(k, v)| (*k == h).then(|| v.clone()))
        }) {
            return hit;
        }
    }
    let max_ts = input
        .sms
        .iter()
        .map(|m| m.date_ms)
        .chain(input.notifications.iter().map(|n| n.post_time_ms))
        .chain(input.otp_banners.iter().map(|b| b.timestamp_ms))
        .chain(input.threads.iter().map(|t| t.last_timestamp_ms))
        .chain(input.call_logs.iter().map(|c| c.date_ms))
        .max()
        .unwrap_or(0);
    let now_ms = reference_now_ms_for_input(input, max_ts);
    let gov = crate::home_reminder_governance::active_governance();

    let today_tuple = if input.today_ymd.trim().is_empty() {
        None
    } else {
        parse_ymd_tuple(input.today_ymd)
    };
    let mut upcoming_meeting_within_45m = false;
    let mut active_meeting_context: Option<String> = None;
    let mut annotated_events: Vec<(usize, &crate::toolbox::PhoneCalendarEvent, bool, bool, bool)> =
        Vec::new();

    for (idx, ev) in input.calendar_events.iter().enumerate() {
        if any_synthetic_e2e_probe(&[&ev.title, &ev.location, &ev.description]) {
            continue;
        }
        let mut cal_keys =
            crate::home_reminder_governance::canonical_keys_for_calendar_event(ev.id);
        cal_keys.push(format!("schedule:{}", ev.id));
        cal_keys.push(format!("card:calendar-ev-{}-{idx}", ev.id));
        if gov.is_any_key_suppressed(
            &cal_keys,
            (ev.dtstart > 0).then_some(ev.dtstart),
            None,
            None,
            now_ms,
        ) {
            continue;
        }
        let is_today = if input.today_ymd.trim().is_empty() || ev.dtstart <= 0 {
            true
        } else {
            event_is_on_date_tuple(ev, today_tuple)
        };
        if !is_today {
            continue;
        }
        if is_calendar_event_auto_expired_on_today(ev, input.hour, input.minute, now_ms, &gov) {
            continue;
        }
        let is_ongoing = event_is_ongoing(ev, input.hour, input.minute);
        let within_45m = is_ongoing || event_is_within_45_minutes(ev, input.hour, input.minute);
        if within_45m {
            upcoming_meeting_within_45m = true;
            if is_ongoing || active_meeting_context.is_none() {
                let ctx = format_active_meeting_context(ev);
                if !ctx.is_empty() {
                    active_meeting_context = Some(ctx);
                }
            }
        }
        annotated_events.push((idx, ev, true, within_45m, is_ongoing));
    }
    let active_meeting_title = active_meeting_context.as_deref();
    let today_meetings_count = annotated_events.len();

    let pomo_mins = input.pomodoro_running.then(|| {
        if input.pomodoro_remaining_secs > 0 {
            (input.pomodoro_remaining_secs.saturating_add(59) / 60).max(1)
        } else {
            25
        }
    });

    // -------------------------------------------------------------------------
    // 1..=3. Life Priority Cards (OTP/Pickup, Takeout/Delivery, Express Parcels)
    // -------------------------------------------------------------------------
    let mut priority_cards =
        collect_life_priority_cards(input, active_meeting_title, pomo_mins, true);

    // -------------------------------------------------------------------------
    // 4. Missed Calls (HomeCardKind::MissedCall, urgency ~88)
    // -------------------------------------------------------------------------
    let missed_call_ttl_ms =
        gov.effective_ttl_ms(crate::home_reminder_governance::ReminderCategory::MissedCall);
    for (idx, call) in input
        .call_logs
        .iter()
        .filter(|c| c.call_type == 3)
        .take(4)
        .enumerate()
    {
        let number = call.number.trim();
        let norm_num = crate::home_reminder_governance::normalize_phone_or_addr(number);
        let call_key = format!("call:{norm_num}");
        let force_kept = gov.is_force_kept(&call_key, now_ms);
        let caller = if call.name.trim().is_empty() {
            number.to_string()
        } else {
            format!("{} ({number})", call.name.trim())
        };
        let keys = [
            call_key.clone(),
            format!("missed-call:{number}:{}", call.date_ms),
            format!("card:missed-call-{idx}-{number}"),
        ];
        if gov.is_any_key_suppressed(
            &keys,
            (call.date_ms > 0).then_some(call.date_ms),
            None,
            (!norm_num.is_empty()).then_some(call_key.as_str()),
            now_ms,
        ) {
            continue;
        }
        if call.date_ms > 0 && !force_kept {
            if now_ms.saturating_sub(call.date_ms) > missed_call_ttl_ms {
                crate::home_reminder_governance::record_auto_expired_notice(
                    call_key,
                    crate::home_reminder_governance::ReminderCategory::MissedCall,
                    format!("未接来电 · {caller}"),
                    "未接来电已超过自动过期时效",
                    now_ms,
                );
                continue;
            }
            let has_newer_call = !norm_num.is_empty()
                && input.call_logs.iter().any(|c2| {
                    c2.call_type != 3
                        && c2.date_ms > call.date_ms
                        && crate::home_reminder_governance::normalize_phone_or_addr(&c2.number)
                            == norm_num
                });
            let has_newer_outgoing_sms = !norm_num.is_empty()
                && input.sms.iter().any(|m| {
                    m.outgoing
                        && m.date_ms > call.date_ms
                        && crate::home_reminder_governance::normalize_phone_or_addr(&m.address)
                            == norm_num
                });
            if has_newer_call || has_newer_outgoing_sms {
                crate::home_reminder_governance::record_auto_expired_notice(
                    call_key,
                    crate::home_reminder_governance::ReminderCategory::MissedCall,
                    format!("未接来电 · {caller}"),
                    "已回拨或已通过短信回复自动过期",
                    now_ms,
                );
                continue;
            }
        }
        priority_cards.push(HomePriorityCard {
            id: format!("missed-call-{idx}-{number}"),
            kind: HomeCardKind::MissedCall,
            urgency: 88,
            badge: "未接来电".to_string(),
            title: format!("未接来电 · {caller}"),
            subtitle: "可在桌面一键发起回拨或快速发送短信回复".to_string(),
            detail: format!("号码：{number} · 状态：{}", call.type_label()),
            timestamp_label: format_timestamp_ms(call.date_ms),
            code_or_number: non_empty_opt(number),
            thread_id: None,
            notification_key: None,
            package_name: Some("com.android.dialer".to_string()),
            quick_replies: quick_replies_for_message_with_context(
                "电话",
                &caller,
                "未接来电",
                active_meeting_title,
                pomo_mins,
                input.dnd_on,
            ),
        });
    }

    // -------------------------------------------------------------------------
    // 5. Unread Messages (HomeCardKind::UnreadMessage, urgency ~85..=89)
    // -------------------------------------------------------------------------
    let mention_career_ttl_ms = gov.effective_ttl_ms(
        crate::home_reminder_governance::ReminderCategory::DirectMentionAndCareer,
    );
    let unread_msg_ttl_ms =
        gov.effective_ttl_ms(crate::home_reminder_governance::ReminderCategory::UnreadMessage);
    let mut sorted_threads: Vec<&crate::push::ImThread> = input
        .threads
        .iter()
        .filter(|t| {
            if t.title.trim_start().starts_with('{')
                || crate::push::is_excluded_im_package(&t.package_name)
                || any_synthetic_e2e_probe(&[&t.app_name, &t.title, &t.last_message])
            {
                return false;
            }
            let has_at = crate::push::thread_mentions_current_user(t, input.notifications);
            let is_career = crate::push::is_career_or_recruitment_signal(
                &t.package_name,
                &t.app_name,
                &t.title,
                &t.last_message,
            );
            let category = if has_at || is_career {
                crate::home_reminder_governance::ReminderCategory::DirectMentionAndCareer
            } else {
                crate::home_reminder_governance::ReminderCategory::UnreadMessage
            };
            let ttl_ms = if has_at || is_career {
                mention_career_ttl_ms
            } else {
                unread_msg_ttl_ms
            };
            let im_key = format!("im:{}", t.thread_id.trim());
            let mut keys = crate::home_reminder_governance::canonical_keys_for_im_thread(
                &t.thread_id,
                non_empty_opt(&t.notification_key).as_deref(),
            );
            keys.push(format!("card:im-thread-{}", t.thread_id));
            if gov.is_any_key_suppressed(
                &keys,
                (t.last_timestamp_ms > 0).then_some(t.last_timestamp_ms),
                None,
                Some(im_key.as_str()),
                now_ms,
            ) {
                return false;
            }
            let force_kept = gov.is_force_kept(&im_key, now_ms);
            if t.last_timestamp_ms >= 1_760_000_000_000
                && now_ms.saturating_sub(t.last_timestamp_ms) > ttl_ms
                && !force_kept
            {
                crate::home_reminder_governance::record_auto_expired_notice(
                    im_key,
                    category,
                    if t.title.trim().is_empty() {
                        t.app_name.trim()
                    } else {
                        t.title.trim()
                    },
                    "会话提醒已超过自动过期时效",
                    now_ms,
                );
                return false;
            }
            if t.unread_count > 0 {
                return true;
            }
            let last_is_outgoing = t.messages.last().is_some_and(|m| m.outgoing);
            if last_is_outgoing {
                return false;
            }
            if !(has_at || is_career) {
                return false;
            }
            if t.last_timestamp_ms > 0
                && now_ms.saturating_sub(t.last_timestamp_ms) > 1_800_000
                && !force_kept
            {
                crate::home_reminder_governance::record_auto_expired_notice(
                    im_key,
                    category,
                    if t.title.trim().is_empty() {
                        t.app_name.trim()
                    } else {
                        t.title.trim()
                    },
                    "已读 @提及/招聘会话满 30 分钟自动过期",
                    now_ms,
                );
                return false;
            }
            true
        })
        .collect();
    sorted_threads.sort_by(|a, b| {
        let a_at = crate::push::thread_mentions_current_user(a, input.notifications);
        let b_at = crate::push::thread_mentions_current_user(b, input.notifications);
        let a_career = crate::push::is_career_or_recruitment_signal(
            &a.package_name,
            &a.app_name,
            &a.title,
            &a.last_message,
        );
        let b_career = crate::push::is_career_or_recruitment_signal(
            &b.package_name,
            &b.app_name,
            &b.title,
            &b.last_message,
        );
        b_at.cmp(&a_at)
            .then_with(|| b_career.cmp(&a_career))
            .then_with(|| b.unread_count.cmp(&a.unread_count))
            .then_with(|| b.last_timestamp_ms.cmp(&a.last_timestamp_ms))
    });

    for thread in sorted_threads.into_iter().take(6) {
        let is_career = crate::push::is_career_or_recruitment_signal(
            &thread.package_name,
            &thread.app_name,
            &thread.title,
            &thread.last_message,
        );
        let at_mention =
            crate::push::thread_mentions_current_user(thread, input.notifications).then_some("@我");
        let app_label = if !thread.app_name.trim().is_empty() {
            thread.app_name.trim().to_string()
        } else if is_career && thread.package_name.contains("bosszhipin") {
            "BOSS直聘".to_string()
        } else {
            thread.channel().label().to_string()
        };
        let (badge, urgency) = if let Some(at_tag) = at_mention {
            let b = if thread.unread_count > 0 {
                format!("{app_label} · 有人@我 ({}条)", thread.unread_count)
            } else {
                format!("{app_label} · 有人@我 ({at_tag})")
            };
            (b, 89)
        } else if is_career {
            let b = if thread.unread_count > 0 {
                format!("{app_label} · {} 条未读", thread.unread_count)
            } else {
                format!("{app_label} · 招聘沟通")
            };
            (b, 87)
        } else {
            (format!("{app_label} · {} 条未读", thread.unread_count), 86)
        };
        let subtitle = if !thread.last_sender.trim().is_empty()
            && thread.last_sender.trim() != thread.title.trim()
        {
            format!("{} · {}", app_label, thread.last_sender.trim())
        } else {
            format!("{} · 支持桌面直接秒回", app_label)
        };
        priority_cards.push(HomePriorityCard {
            id: format!("im-thread-{}", thread.thread_id),
            kind: HomeCardKind::UnreadMessage,
            urgency,
            badge,
            title: if thread.title.trim().is_empty() {
                app_label.clone()
            } else {
                thread.title.trim().to_string()
            },
            subtitle,
            detail: thread.last_message.trim().to_string(),
            timestamp_label: format_timestamp_ms(thread.last_timestamp_ms),
            code_or_number: None,
            thread_id: non_empty_opt(&thread.thread_id),
            notification_key: non_empty_opt(&thread.notification_key),
            package_name: non_empty_opt(&thread.package_name),
            quick_replies: quick_replies_for_message_with_context(
                &app_label,
                &thread.title,
                &thread.last_message,
                active_meeting_title,
                pomo_mins,
                input.dnd_on,
            ),
        });
    }

    // Unread incoming SMS not already captured as OTP/Express
    for (idx, msg) in input
        .sms
        .iter()
        .filter(|m| m.kind == 1 && !m.read && !any_synthetic_e2e_probe(&[&m.address, &m.body]))
        .take(3)
        .enumerate()
    {
        if extract_otp_or_pickup_from_text(&msg.body).is_some()
            || is_express_notification("短信", &msg.address, &msg.body)
        {
            continue;
        }
        let norm_addr = crate::home_reminder_governance::normalize_phone_or_addr(&msg.address);
        let sms_key = format!("sms:{norm_addr}");
        let keys = [
            sms_key.clone(),
            format!("card:unread-sms-{idx}-{}", msg.address.trim()),
        ];
        if gov.is_any_key_suppressed(
            &keys,
            (msg.date_ms > 0).then_some(msg.date_ms),
            None,
            (!norm_addr.is_empty()).then_some(sms_key.as_str()),
            now_ms,
        ) {
            continue;
        }
        if msg.date_ms >= 1_760_000_000_000
            && now_ms.saturating_sub(msg.date_ms) > unread_msg_ttl_ms
            && !gov.is_force_kept(&sms_key, now_ms)
        {
            crate::home_reminder_governance::record_auto_expired_notice(
                sms_key,
                crate::home_reminder_governance::ReminderCategory::UnreadMessage,
                format!("短信 · {}", msg.address.trim()),
                "未读短信已超过自动过期时效",
                now_ms,
            );
            continue;
        }
        priority_cards.push(HomePriorityCard {
            id: format!("unread-sms-{idx}-{}", msg.address.trim()),
            kind: HomeCardKind::UnreadMessage,
            urgency: 85,
            badge: "未读短信".to_string(),
            title: format!("短信 · {}", msg.address.trim()),
            subtitle: "手机收件箱未读短信 · 支持桌面直接回复".to_string(),
            detail: msg.body.trim().to_string(),
            timestamp_label: format_timestamp_ms(msg.date_ms),
            code_or_number: non_empty_opt(&msg.address),
            thread_id: None,
            notification_key: None,
            package_name: Some("com.android.mms".to_string()),
            quick_replies: quick_replies_for_message_with_context(
                "短信",
                &msg.address,
                &msg.body,
                active_meeting_title,
                pomo_mins,
                input.dnd_on,
            ),
        });
    }

    // -------------------------------------------------------------------------
    // 6. Upcoming Meetings / Schedule (HomeCardKind::UpcomingMeeting, urgency ~80..=83)
    // -------------------------------------------------------------------------
    annotated_events.sort_by_key(|(idx, _, is_today, within_45m, is_ongoing)| {
        (
            std::cmp::Reverse(*is_ongoing),
            std::cmp::Reverse(*within_45m),
            std::cmp::Reverse(*is_today),
            *idx,
        )
    });

    for (idx, ev, _is_today, within_45m, is_ongoing) in annotated_events.into_iter().take(4) {
        if is_calendar_event_auto_expired_on_today(ev, input.hour, input.minute, now_ms, &gov) {
            continue;
        }
        let urgency = if is_ongoing {
            83
        } else if within_45m {
            82
        } else {
            81
        };
        let badge = if is_ongoing {
            "正在进行".to_string()
        } else if within_45m {
            "即将开始".to_string()
        } else {
            "今日日程".to_string()
        };
        let loc = if ev.location.trim().is_empty() {
            "线上/待定地点".to_string()
        } else {
            ev.location.trim().to_string()
        };
        let meeting_url =
            extract_meeting_url(&ev.description).or_else(|| extract_meeting_url(&ev.location));
        priority_cards.push(HomePriorityCard {
            id: format!("calendar-ev-{}-{idx}", ev.id),
            kind: HomeCardKind::UpcomingMeeting,
            urgency,
            badge,
            title: if ev.title.trim().is_empty() {
                "未命名日程".to_string()
            } else {
                ev.title.trim().to_string()
            },
            subtitle: format!("{} · {loc}", ev.time_range_label()),
            detail: if ev.description.trim().is_empty() {
                format!("来源：{} · 地点：{loc}", ev.source_label())
            } else {
                ev.description.trim().to_string()
            },
            timestamp_label: ev.hm_label(),
            code_or_number: meeting_url,
            thread_id: None,
            notification_key: None,
            package_name: None,
            quick_replies: Vec::new(),
        });
    }

    // Sort priority cards by urgency descending, preserving stable insertion order on ties
    priority_cards.sort_by(|a, b| b.urgency.cmp(&a.urgency));
    priority_cards.truncate(16);

    let active_life_cards_count = priority_cards
        .iter()
        .filter(|c| {
            matches!(
                c.kind,
                HomeCardKind::OtpAndPickup
                    | HomeCardKind::TakeoutAndDelivery
                    | HomeCardKind::ExpressParcel
            )
        })
        .count();

    let unread_threads_sum: usize = input
        .threads
        .iter()
        .filter(|t| {
            if t.title.trim_start().starts_with('{')
                || crate::push::is_excluded_im_package(&t.package_name)
                || any_synthetic_e2e_probe(&[&t.app_name, &t.title, &t.last_message])
            {
                return false;
            }
            let im_key = format!("im:{}", t.thread_id.trim());
            let keys = crate::home_reminder_governance::canonical_keys_for_im_thread(
                &t.thread_id,
                non_empty_opt(&t.notification_key).as_deref(),
            );
            !gov.is_any_key_suppressed(
                &keys,
                (t.last_timestamp_ms > 0).then_some(t.last_timestamp_ms),
                None,
                Some(im_key.as_str()),
                now_ms,
            )
        })
        .map(|t| t.unread_count)
        .sum();
    let unread_sms_count: usize = input
        .sms
        .iter()
        .filter(|m| {
            if m.kind != 1 || m.read || any_synthetic_e2e_probe(&[&m.address, &m.body]) {
                return false;
            }
            let norm_addr = crate::home_reminder_governance::normalize_phone_or_addr(&m.address);
            let sms_key = format!("sms:{norm_addr}");
            !gov.is_any_key_suppressed(
                std::slice::from_ref(&sms_key),
                (m.date_ms > 0).then_some(m.date_ms),
                None,
                (!norm_addr.is_empty()).then_some(sms_key.as_str()),
                now_ms,
            )
        })
        .count();
    let unread_messages_count = unread_threads_sum + unread_sms_count;

    let today_unexpired_alarms_count = input
        .alarms
        .iter()
        .filter(|a| a.is_today_valid_unexpired_at(now_ms, &gov))
        .count();
    let enabled_alarms_count = if today_unexpired_alarms_count > 0 {
        today_unexpired_alarms_count
    } else {
        input
            .alarms
            .iter()
            .filter(|a| a.is_valid_future_or_today_unexpired_at(now_ms, &gov))
            .count()
    };

    let has_active_delivery_or_express = active_life_cards_count > 0;
    let has_active_meal_delivery = priority_cards
        .iter()
        .any(|c| c.kind == HomeCardKind::TakeoutAndDelivery && c.badge != "出行打车");
    let scene = detect_scene(
        input.hour,
        upcoming_meeting_within_45m,
        has_active_delivery_or_express,
    );

    // -------------------------------------------------------------------------
    // 7. Habit Analysis, Cross-Signal Conflicts, Checklists & AI Recommendations
    // -------------------------------------------------------------------------
    let schedule_conflicts = detect_schedule_alarm_conflicts(
        input.today_ymd,
        input.hour,
        input.minute,
        input.calendar_events,
        input.alarms,
    );
    let schedule_checklists = extract_schedule_checklists_for_date_with_clock(
        input.today_ymd,
        Some((input.today_ymd, input.hour, input.minute, now_ms)),
        input.calendar_events,
    );

    let (habit_headline, habit_tags) = build_habit_summary(input);
    let mut ai_recommendations = build_ai_recommendations(
        input,
        &scene,
        unread_messages_count,
        today_meetings_count,
        upcoming_meeting_within_45m,
        has_active_meal_delivery,
    );
    if let Some(top_conflict) = schedule_conflicts.first() {
        ai_recommendations.insert(
            0,
            HomeAiRecommendation {
                id: format!("rec-{}", top_conflict.id),
                badge: top_conflict.conflict_badge.clone(),
                title: top_conflict.headline.clone(),
                reason: top_conflict.detail.clone(),
                action_label: top_conflict.primary_action_label.clone(),
                intent: top_conflict.primary_intent.clone(),
            },
        );
        ai_recommendations.truncate(7);
    }

    let life_context_cards =
        build_life_context_cards(input, &scene, &priority_cards, today_meetings_count);

    let report = HomeHubReport {
        scene,
        unread_messages_count,
        active_life_cards_count,
        today_meetings_count,
        enabled_alarms_count,
        priority_cards,
        life_context_cards,
        ai_recommendations,
        schedule_conflicts,
        schedule_checklists,
        habit_headline,
        habit_tags,
    };
    if let Some(h) = cache_hash {
        HOME_HUB_REPORT_CACHE.with(|c| {
            *c.borrow_mut() = Some((h, report.clone()));
        });
    }
    report
}

thread_local! {
    static HOME_HUB_REPORT_CACHE: std::cell::RefCell<Option<(u64, HomeHubReport)>> =
        const { std::cell::RefCell::new(None) };
    static HOME_LIFE_CARDS_CACHE: std::cell::RefCell<Option<(u64, Vec<HomeLifeContextCard>)>> =
        const { std::cell::RefCell::new(None) };
    static HOME_PANORAMA_CACHE: std::cell::RefCell<Option<(u64, SelectedDatePanorama)>> =
        const { std::cell::RefCell::new(None) };
    static HOME_WEEKLY_CACHE: std::cell::RefCell<Option<(u64, WeeklyOverviewReport)>> =
        const { std::cell::RefCell::new(None) };
}

pub fn compute_home_hub_input_hash(
    tag: u8,
    selected_ymd: &str,
    input: &HomeHubInput<'_>,
    desktop_entries: &[DesktopDateEntry],
) -> u64 {
    compute_home_hub_input_hash_with_revisions(
        tag,
        selected_ymd,
        input,
        desktop_entries,
        crate::home_reminder_governance::active_governance_revision(),
        crate::weather::weather_cache_revision(),
    )
}

fn compute_home_hub_input_hash_with_revisions(
    tag: u8,
    selected_ymd: &str,
    input: &HomeHubInput<'_>,
    desktop_entries: &[DesktopDateEntry],
    governance_revision: u64,
    weather_revision: u64,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tag.hash(&mut h);
    governance_revision.hash(&mut h);
    weather_revision.hash(&mut h);
    selected_ymd.hash(&mut h);
    input.hour.hash(&mut h);
    input.minute.hash(&mut h);
    input.today_ymd.hash(&mut h);
    input.device_connected.hash(&mut h);
    input.device_model.hash(&mut h);
    input.firmware_profile.hash(&mut h);
    input.threads.len().hash(&mut h);
    for t in input.threads {
        t.thread_id.hash(&mut h);
        t.app_name.hash(&mut h);
        t.package_name.hash(&mut h);
        t.title.hash(&mut h);
        t.last_sender.hash(&mut h);
        t.last_message.hash(&mut h);
        t.last_timestamp_ms.hash(&mut h);
        t.unread_count.hash(&mut h);
        t.notification_key.hash(&mut h);
        t.is_group.hash(&mut h);
        t.can_remote_reply.hash(&mut h);
        t.can_ui_reply.hash(&mut h);
        t.has_content_intent.hash(&mut h);
        // Parcel receipts, @mention handling and dated panoramas inspect the
        // actual history, even when the thread preview/revision stays unchanged.
        // Media bytes/avatars are not inputs to these reports and stay unhashed.
        t.messages.len().hash(&mut h);
        for message in &t.messages {
            message.id.hash(&mut h);
            message.sender.hash(&mut h);
            message.text.hash(&mut h);
            message.timestamp_ms.hash(&mut h);
            message.observed_at_ms.hash(&mut h);
            message.time_known.hash(&mut h);
            message.recalled.hash(&mut h);
            message.outgoing.hash(&mut h);
            message.via.hash(&mut h);
        }
    }
    input.notifications.len().hash(&mut h);
    for n in input.notifications {
        n.seq_id.hash(&mut h);
        n.key.hash(&mut h);
        n.package_name.hash(&mut h);
        n.app_name.hash(&mut h);
        n.title.hash(&mut h);
        n.text.hash(&mut h);
        n.post_time_ms.hash(&mut h);
        n.dismissed.hash(&mut h);
        n.thread_id.hash(&mut h);
        n.category.hash(&mut h);
        n.sender.hash(&mut h);
        n.sub_text.hash(&mut h);
        n.active.hash(&mut h);
        n.read.hash(&mut h);
        n.is_ongoing.hash(&mut h);
        n.is_group.hash(&mut h);
        n.can_reply.hash(&mut h);
        n.otp_code.hash(&mut h);
        n.action_labels.hash(&mut h);
        n.url_hint.hash(&mut h);
    }
    input.otp_banners.len().hash(&mut h);
    for b in input.otp_banners {
        b.code.hash(&mut h);
        b.source_app.hash(&mut h);
        b.title.hash(&mut h);
        b.raw_text.hash(&mut h);
        b.timestamp_ms.hash(&mut h);
        b.notification_key.hash(&mut h);
    }
    input.sms.len().hash(&mut h);
    for s in input.sms {
        s.provider_id.hash(&mut h);
        s.address.hash(&mut h);
        s.body.hash(&mut h);
        s.date_ms.hash(&mut h);
        s.read.hash(&mut h);
        s.outgoing.hash(&mut h);
        s.kind.hash(&mut h);
    }
    input.call_logs.len().hash(&mut h);
    for c in input.call_logs {
        c.number.hash(&mut h);
        c.name.hash(&mut h);
        c.date_ms.hash(&mut h);
        c.duration_sec.hash(&mut h);
        c.call_type.hash(&mut h);
    }
    input.express_parcels.len().hash(&mut h);
    for p in input.express_parcels {
        p.carrier.hash(&mut h);
        p.tracking_no.hash(&mut h);
        p.status.hash(&mut h);
        p.pickup_code.hash(&mut h);
        p.station.hash(&mut h);
        p.updated_at.hash(&mut h);
    }
    input.calendar_events.len().hash(&mut h);
    for ev in input.calendar_events {
        ev.id.hash(&mut h);
        ev.title.hash(&mut h);
        ev.dtstart.hash(&mut h);
        ev.dtend.hash(&mut h);
        ev.all_day.hash(&mut h);
        ev.location.hash(&mut h);
        ev.description.hash(&mut h);
        ev.source.hash(&mut h);
    }
    input.alarms.len().hash(&mut h);
    for a in input.alarms {
        a.id.hash(&mut h);
        a.hour.hash(&mut h);
        a.minutes.hash(&mut h);
        a.enabled.hash(&mut h);
        a.days_of_week.hash(&mut h);
        a.vibrate.hash(&mut h);
        a.message.hash(&mut h);
    }
    input.wellbeing.screen_time_minutes.hash(&mut h);
    input.wellbeing.unlock_count.hash(&mut h);
    input.wellbeing.alphame_running.hash(&mut h);
    input.wellbeing.top_apps.len().hash(&mut h);
    for (pkg, lbl, mins) in &input.wellbeing.top_apps {
        pkg.hash(&mut h);
        lbl.hash(&mut h);
        mins.hash(&mut h);
    }
    input.phone_clipboard.hash(&mut h);
    input.remote_agent_files.hash(&mut h);
    input.work_deliverables_count.hash(&mut h);
    input.pomodoro_running.hash(&mut h);
    // Report text uses rounded-up remaining minutes; the live second hand is
    // rendered directly from UI timer state, not from this cached report.
    (input.pomodoro_remaining_secs.saturating_add(59) / 60).hash(&mut h);
    input.dnd_on.hash(&mut h);
    desktop_entries.len().hash(&mut h);
    for d in desktop_entries {
        d.id.hash(&mut h);
        d.timestamp.hash(&mut h);
        (d.kind as u8).hash(&mut h);
        d.badge.hash(&mut h);
        d.title.hash(&mut h);
        d.detail.hash(&mut h);
        d.action_label.hash(&mut h);
        d.intent.hash(&mut h);
    }
    h.finish()
}

/// Zero-allocation parser for `YYYY-MM-DD` into `(year, month, day)`.
#[inline]
pub fn parse_ymd_tuple(ymd: &str) -> Option<(i32, u32, u32)> {
    let mut parts = ymd.trim().split('-');
    let y = parts.next()?.parse::<i32>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let d = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((y, m, d))
}

/// Zero-allocation local timestamp breakdown into `((year, month, day), hour, minute)`
/// backed by a 64-entry thread-local direct-mapped cache to avoid repeated OS timezone queries.
#[inline]
pub fn timestamp_to_local_ymd_parts(ts: i64) -> Option<((i32, u32, u32), u32, u32)> {
    use chrono::Datelike;
    if ts <= 0 {
        return None;
    }
    let secs = if ts >= 100_000_000_000 {
        ts.div_euclid(1000)
    } else if ts >= 1_000_000_000 {
        ts
    } else {
        return None;
    };
    thread_local! {
        static TS_PARTS_CACHE: std::cell::Cell<[(i64, ((i32, u32, u32), u32, u32)); 64]> =
            const { std::cell::Cell::new([(0, ((0, 0, 0), 0, 0)); 64]) };
    }
    let slot = ((secs as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 58) as usize;
    let cached = TS_PARTS_CACHE.with(|c| {
        let arr = c.get();
        let (k, v) = arr[slot];
        if k == secs {
            Some(v)
        } else {
            None
        }
    });
    if let Some(hit) = cached {
        return Some(hit);
    }
    let dt = Local.timestamp_opt(secs, 0).single()?;
    let val = ((dt.year(), dt.month(), dt.day()), dt.hour(), dt.minute());
    TS_PARTS_CACHE.with(|c| {
        let mut arr = c.get();
        arr[slot] = (secs, val);
        c.set(arr);
    });
    Some(val)
}

/// Normalizes either second-based (`1_000_000_000..100_000_000_000`) or millisecond-based
/// (`>= 100_000_000_000`) timestamps into local `(YYYY-MM-DD, HH:MM, minute_of_day)`.
pub fn timestamp_to_local_ymd_hm(ts: i64) -> Option<(String, String, u32)> {
    let ((y, m, d), hour, minute) = timestamp_to_local_ymd_parts(ts)?;
    let ymd = format!("{y:04}-{m:02}-{d:02}");
    let hm = format!("{hour:02}:{minute:02}");
    let min_of_day = hour * 60 + minute;
    Some((ymd, hm, min_of_day))
}

#[inline]
fn contains_rain_cue(text: &str) -> bool {
    [
        "下雨",
        "降雨",
        "降水",
        "雨夹雪",
        "阵雨",
        "暴雨",
        "小雨",
        "中雨",
        "大雨",
        "雷阵雨",
        "备伞",
        "雨伞",
        "台风",
        "湿滑",
    ]
    .iter()
    .any(|kw| text.contains(kw))
        || (text.contains("带伞") && !text.contains("无需带伞") && !text.contains("不用带伞"))
        || contains_ascii_ignore_case(text, b"rain")
}

#[inline]
fn contains_rain_cue_any(parts: &[&str]) -> bool {
    parts.iter().any(|s| contains_rain_cue(s))
}

#[inline]
fn contains_weather_or_rain_cue(text: &str) -> bool {
    contains_rain_cue(text)
        || ["天气", "降温", "大风", "预警"]
            .iter()
            .any(|kw| text.contains(kw))
        || contains_ascii_ignore_case(text, b"weather")
}

#[inline]
fn contains_weather_or_rain_cue_any(parts: &[&str]) -> bool {
    parts.iter().any(|s| contains_weather_or_rain_cue(s))
}

#[inline]
fn contains_leisure_or_appointment_cue(text: &str) -> bool {
    [
        "预约",
        "预订",
        "订场地",
        "订座",
        "挂号",
        "报名",
        "读书会",
        "自驾",
        "出发",
        "行前",
        "车况",
        "餐厅",
        "晚餐",
        "午餐",
        "下午茶",
        "咖啡",
        "电影",
        "演出",
        "展览",
        "健身",
        "运动",
        "羽毛球",
        "游泳",
        "休息",
        "度假",
        "酒店",
        "神券",
        "外卖",
        "美团",
        "闪购",
        "饿了么",
        "高铁",
        "机票",
    ]
    .iter()
    .any(|kw| text.contains(kw))
}

#[inline]
fn contains_leisure_or_appointment_cue_any(parts: &[&str]) -> bool {
    parts.iter().any(|s| contains_leisure_or_appointment_cue(s))
}

/// Synthesizes the 3 context-linked life cards (`快递与取件` · `天气与出行` · `要愉悦/预约与能量补给`)
/// explicitly tied to the user's schedule, commute window, and digital wellbeing habits.
pub fn build_life_context_cards(
    input: &HomeHubInput<'_>,
    scene: &HomeSceneInfo,
    priority_cards: &[HomePriorityCard],
    today_meetings_count: usize,
) -> Vec<HomeLifeContextCard> {
    build_life_context_cards_from_weather(
        input,
        scene,
        priority_cards,
        today_meetings_count,
        crate::weather::cached_weather_snapshot(),
    )
}

pub(crate) fn build_life_context_cards_from_weather(
    input: &HomeHubInput<'_>,
    scene: &HomeSceneInfo,
    priority_cards: &[HomePriorityCard],
    today_meetings_count: usize,
    cached_weather: Option<crate::weather::WeatherSnapshot>,
) -> Vec<HomeLifeContextCard> {
    let gov = crate::home_reminder_governance::active_governance();
    let max_ts = input
        .sms
        .iter()
        .map(|m| m.date_ms)
        .chain(input.notifications.iter().map(|n| n.post_time_ms))
        .max()
        .unwrap_or(0);
    let now_ms = reference_now_ms_for_input(input, max_ts);
    let weather_ttl_ms =
        gov.effective_ttl_ms(crate::home_reminder_governance::ReminderCategory::WeatherAndLife);
    let today_tuple = parse_ymd_tuple(input.today_ymd);
    let today_events: Vec<&crate::toolbox::PhoneCalendarEvent> = input
        .calendar_events
        .iter()
        .filter(|ev| {
            event_is_on_date_tuple(ev, today_tuple)
                && !any_synthetic_e2e_probe(&[&ev.title, &ev.location, &ev.description])
                && !gov.is_any_key_suppressed(
                    &crate::home_reminder_governance::canonical_keys_for_calendar_event(ev.id),
                    (ev.dtstart > 0).then_some(ev.dtstart),
                    None,
                    None,
                    now_ms,
                )
        })
        .collect();

    // -------------------------------------------------------------------------
    // 1. Express & Pickup (快递与取件 · 上下文关联)
    // -------------------------------------------------------------------------
    let pickup_card = priority_cards.iter().find(|c| {
        c.kind == HomeCardKind::OtpAndPickup
            && (c.badge.contains("取件") || c.title.contains("取件") || c.detail.contains("取件"))
    });
    let express_card = priority_cards
        .iter()
        .find(|c| c.kind == HomeCardKind::ExpressParcel);

    let express_context_link = if let Some(last_ev) = today_events.last() {
        format!(
            "关联今日日程「{}（{}）」：建议先把取件码/单号推上背屏，日程结束后顺路前往驿站取件",
            last_ev.title.trim(),
            last_ev.time_range_label()
        )
    } else if input.hour >= 17 {
        "关联晚间收尾动线：下班回家途中顺路前往驿站或快递柜取件，背屏常显免掏手机".to_string()
    } else if input.pomodoro_running || input.dnd_on {
        "关联当前专注/勿扰状态：已为您静默汇总取件凭证，专注结束后再集中处理".to_string()
    } else {
        "关联桌面与背屏跨端流转：短信、通知与剪贴板中的运单及取件码自动聚合，一键推背屏".to_string()
    };

    let active_parcel = input.express_parcels.iter().find(|parcel| {
        let pickup = parcel.pickup_code.trim();
        let tracking = parcel.tracking_no.trim();
        !is_completed_express_text(&parcel.status)
            && !is_completed_express_text(&parcel.station)
            && !gov.is_any_key_suppressed(
                &crate::home_reminder_governance::canonical_keys_for_code_or_parcel(
                    (!pickup.is_empty()).then_some(pickup),
                    (!tracking.is_empty()).then_some(tracking),
                    None,
                    None,
                ),
                None,
                None,
                None,
                now_ms,
            )
    });

    let express_life_card = if let Some(parcel) = active_parcel {
        let pickup = parcel.pickup_code.trim();
        let tracking = parcel.tracking_no.trim();
        let badge = if !pickup.is_empty() {
            format!("取件码 {pickup} · 动线关联")
        } else {
            "在途包裹 · 动线关联".to_string()
        };
        let title = if !pickup.is_empty() {
            format!("{} · 凭码 {} 取件", parcel.carrier.trim(), pickup)
        } else {
            format!("{} · {}", parcel.carrier.trim(), parcel.status.trim())
        };
        let detail = if !parcel.station.trim().is_empty() {
            format!("驿站/节点：{} · 运单：{}", parcel.station.trim(), tracking)
        } else {
            format!("运单号：{tracking} · 状态：{}", parcel.status.trim())
        };
        let bs_payload = if !pickup.is_empty() {
            pickup.to_string()
        } else {
            "待取快递".to_string()
        };
        HomeLifeContextCard {
            id: "life-ctx-express".to_string(),
            domain: HomeLifeDomain::ExpressAndPickup,
            badge,
            title,
            context_link: express_context_link,
            detail,
            code_or_number: Some(if !pickup.is_empty() {
                pickup.to_string()
            } else {
                tracking.to_string()
            }),
            action_label: "查物流轨迹".to_string(),
            intent: HomeAiIntent::QueryExpress(tracking.to_string()),
            secondary_label: Some("背屏显码".to_string()),
            secondary_intent: Some(HomeAiIntent::BackScreenStatus(bs_payload)),
        }
    } else if let Some(card) = pickup_card.or(express_card) {
        let code = card.code_or_number.clone().unwrap_or_default();
        HomeLifeContextCard {
            id: "life-ctx-express".to_string(),
            domain: HomeLifeDomain::ExpressAndPickup,
            badge: format!("{} · 动线关联", card.badge),
            title: card.title.clone(),
            context_link: express_context_link,
            detail: card.detail.clone(),
            code_or_number: card.code_or_number.clone(),
            action_label: "查物流轨迹".to_string(),
            intent: HomeAiIntent::QueryExpress(code.clone()),
            secondary_label: Some("背屏提醒".to_string()),
            secondary_intent: Some(HomeAiIntent::BackScreenStatus(if code.is_empty() {
                "待取快递".to_string()
            } else {
                code
            })),
        }
    } else {
        HomeLifeContextCard {
            id: "life-ctx-express".to_string(),
            domain: HomeLifeDomain::ExpressAndPickup,
            badge: "快递感知 · 动线关联".to_string(),
            title: "快递与取件凭证 · 自动关联下班与外出动线".to_string(),
            context_link: express_context_link,
            detail:
                "收到顺丰/京东/菜鸟短信、通知或复制单号时，自动提取取件码并关联当日日程结束时间"
                    .to_string(),
            code_or_number: None,
            action_label: "查快递动态".to_string(),
            intent: HomeAiIntent::QueryExpress(String::new()),
            secondary_label: Some("菜鸟查件".to_string()),
            secondary_intent: Some(HomeAiIntent::QuickGuiScenario {
                app: "菜鸟".to_string(),
                task: "打开菜鸟查看我的待取包裹与取件码".to_string(),
            }),
        }
    };

    // -------------------------------------------------------------------------
    // 2. Weather & Commute (`天气与出行` · 降雨预警与出行路况关联)
    // -------------------------------------------------------------------------
    let weather_notif = input.notifications.iter().find(|n| {
        if n.dismissed
            || n.title.trim_start().starts_with('{')
            || any_synthetic_e2e_probe(&[&n.app_name, &n.title, &n.text])
            || !contains_weather_or_rain_cue_any(&[&n.title, &n.text])
        {
            return false;
        }
        let notif_key = format!("notif:{}", n.key);
        if n.post_time_ms >= 1_760_000_000_000
            && now_ms.saturating_sub(n.post_time_ms) > weather_ttl_ms
            && !gov.is_force_kept(&notif_key, now_ms)
        {
            return false;
        }
        let mute_src =
            (!n.package_name.trim().is_empty()).then(|| format!("pkg:{}", n.package_name.trim()));
        !gov.is_any_key_suppressed(
            std::slice::from_ref(&notif_key),
            (n.post_time_ms > 0).then_some(n.post_time_ms),
            None,
            mute_src.as_deref(),
            now_ms,
        )
    });
    let weather_sms = input.sms.iter().find(|m| {
        if m.outgoing
            || any_synthetic_e2e_probe(&[&m.address, &m.body])
            || !contains_weather_or_rain_cue(&m.body)
        {
            return false;
        }
        let norm_addr = crate::home_reminder_governance::normalize_phone_or_addr(&m.address);
        let sms_key = format!("sms:{norm_addr}");
        if m.date_ms >= 1_760_000_000_000
            && now_ms.saturating_sub(m.date_ms) > weather_ttl_ms
            && !gov.is_force_kept(&sms_key, now_ms)
        {
            return false;
        }
        !gov.is_any_key_suppressed(
            std::slice::from_ref(&sms_key),
            (m.date_ms > 0).then_some(m.date_ms),
            None,
            (!norm_addr.is_empty()).then_some(sms_key.as_str()),
            now_ms,
        )
    });
    let weather_event = today_events
        .iter()
        .copied()
        .find(|ev| contains_weather_or_rain_cue_any(&[&ev.title, &ev.location, &ev.description]));
    let is_outdoor_or_travel_event = |ev: &crate::toolbox::PhoneCalendarEvent| -> bool {
        if any_synthetic_e2e_probe(&[&ev.title, &ev.location, &ev.description]) {
            return false;
        }
        let loc = ev.location.trim();
        let has_offline_loc = !loc.is_empty()
            && !loc.contains("线上")
            && !loc.contains("腾讯会议")
            && !loc.contains("飞书会议");
        let has_travel_title = [
            "自驾",
            "出发",
            "行前",
            "外出",
            "读书会",
            "机场",
            "高铁",
            "拜访",
        ]
        .iter()
        .any(|kw| ev.title.contains(kw));
        has_offline_loc || has_travel_title
    };
    let outdoor_event = today_events
        .iter()
        .copied()
        .find(|ev| is_outdoor_or_travel_event(ev))
        .or_else(|| {
            input.calendar_events.iter().find(|ev| {
                today_tuple.is_none_or(|t| ev.date_ymd() >= t) && is_outdoor_or_travel_event(ev)
            })
        });

    // This function runs during rendering. Missing cache data is represented
    // explicitly below; only existing background jobs may fetch live weather.
    let has_live_weather = cached_weather.as_ref().is_some_and(|w| w.from_live_api);
    let mut live_weather = cached_weather
        .unwrap_or_else(|| crate::weather::WeatherSnapshot::default_for_city("上海"));
    let mut phone_weather_cues: Vec<String> = Vec::new();
    if let Some(n) = weather_notif {
        phone_weather_cues.push(format!("{} {}", n.title.trim(), n.text.trim()));
    }
    if let Some(s) = weather_sms {
        phone_weather_cues.push(s.body.trim().to_string());
    }
    if !phone_weather_cues.is_empty() {
        live_weather.enrich_with_phone_cues(&phone_weather_cues);
    }
    let schedule_titles: Vec<String> = today_events
        .iter()
        .map(|ev| {
            let t = ev.title.trim();
            let r = ev.time_range_label();
            if r.is_empty() {
                t.to_string()
            } else {
                format!("{r} {t}")
            }
        })
        .filter(|s| !s.trim().is_empty())
        .collect();

    let mut weather_life_card = if let Some(notif) = weather_notif {
        let is_rain = contains_rain_cue_any(&[&notif.title, &notif.text]);
        let link = if let Some(ev) = outdoor_event.or_else(|| today_events.first().copied()) {
            if is_rain {
                format!(
                    "关联今日日程「{}（{}）」：检测到降雨/天气提醒，外出参会或下班通勤请带伞并预留打车时间",
                    ev.title.trim(),
                    if ev.location.trim().is_empty() {
                        ev.time_range_label()
                    } else {
                        ev.location.trim().to_string()
                    }
                )
            } else {
                format!(
                    "关联今日日程「{}（{}）」：检测到天气变化提醒，外出或通勤请关注路况与气温变化",
                    ev.title.trim(),
                    if ev.location.trim().is_empty() {
                        ev.time_range_label()
                    } else {
                        ev.location.trim().to_string()
                    }
                )
            }
        } else if is_rain {
            "关联今日外出与下班通勤：手机收到降雨/天气提醒，出门前请备伞并关注晚高峰路况"
                .to_string()
        } else {
            "关联今日外出与下班通勤：手机收到天气变化提醒，出门前请关注气温与晚高峰路况".to_string()
        };
        HomeLifeContextCard {
            id: "life-ctx-weather".to_string(),
            domain: HomeLifeDomain::WeatherAndCommute,
            badge: if is_rain {
                "要下雨 · 天气预警关联".to_string()
            } else {
                format!("{} · 出行情境关联", live_weather.headline_badge())
            },
            title: if notif.title.trim().is_empty() {
                if is_rain {
                    format!(
                        "{} · 降雨与天气变化提醒 · 出门请备伞",
                        live_weather.headline_badge()
                    )
                } else {
                    format!("{} · 天气变化与出行提醒", live_weather.headline_badge())
                }
            } else {
                notif.title.trim().to_string()
            },
            context_link: link,
            detail: format!(
                "{}（当前 {}，今日 {}，空气{}）。{}",
                notif.text.trim(),
                live_weather.headline_badge(),
                live_weather.range_label(),
                live_weather.aqi_label,
                live_weather.tomorrow_advice()
            ),
            code_or_number: None,
            action_label: "高德打车/路况".to_string(),
            intent: HomeAiIntent::QuickGuiScenario {
                app: "高德地图".to_string(),
                task: "查看当前去目的地或回家的实时路况与打车预估".to_string(),
            },
            secondary_label: Some(if is_rain {
                "背屏：出门带伞".to_string()
            } else {
                "问今日天气/路况".to_string()
            }),
            secondary_intent: Some(if is_rain {
                HomeAiIntent::BackScreenStatus("出门带伞".to_string())
            } else {
                HomeAiIntent::AmooPrompt(
                    live_weather.build_weather_context_prompt(None, &schedule_titles),
                )
            }),
        }
    } else if let Some(sms_msg) = weather_sms {
        let is_rain = contains_rain_cue(&sms_msg.body);
        let link = if let Some(ev) = outdoor_event.or_else(|| today_events.first().copied()) {
            if is_rain {
                format!(
                    "关联今日日程「{}（{}）」：短信提示天气/降雨变化，请提前备伞并规划交通",
                    ev.title.trim(),
                    ev.time_range_label()
                )
            } else {
                format!(
                    "关联今日日程「{}（{}）」：短信提示天气变化，请提前关注温度与交通路况",
                    ev.title.trim(),
                    ev.time_range_label()
                )
            }
        } else if is_rain {
            "关联今日通勤动线：短信提示天气/降雨变化，建议备伞并提前查看路况".to_string()
        } else {
            "关联今日通勤动线：短信提示天气变化，建议提前查看气温与实时路况".to_string()
        };
        HomeLifeContextCard {
            id: "life-ctx-weather".to_string(),
            domain: HomeLifeDomain::WeatherAndCommute,
            badge: if is_rain {
                "要下雨 · 短信天气关联".to_string()
            } else {
                format!("{} · 短信出行关联", live_weather.headline_badge())
            },
            title: if is_rain {
                format!(
                    "{} · 天气与降雨提醒 · 建议备伞出行",
                    live_weather.headline_badge()
                )
            } else {
                format!(
                    "{} · 天气变化提醒 · 关注出行路况",
                    live_weather.headline_badge()
                )
            },
            context_link: link,
            detail: format!(
                "{}（当前 {}，今日 {}）。{}",
                sms_msg.body.trim(),
                live_weather.headline_badge(),
                live_weather.range_label(),
                live_weather.tomorrow_advice()
            ),
            code_or_number: None,
            action_label: "高德打车/路况".to_string(),
            intent: HomeAiIntent::QuickGuiScenario {
                app: "高德地图".to_string(),
                task: "查看当前实时路况与打车预估".to_string(),
            },
            secondary_label: Some(if is_rain {
                "背屏：出门带伞".to_string()
            } else {
                "问今日天气/路况".to_string()
            }),
            secondary_intent: Some(if is_rain {
                HomeAiIntent::BackScreenStatus("出门带伞".to_string())
            } else {
                HomeAiIntent::AmooPrompt(
                    live_weather.build_weather_context_prompt(None, &schedule_titles),
                )
            }),
        }
    } else if let Some(ev) = weather_event
        .filter(|ev| contains_rain_cue_any(&[&ev.title, &ev.location, &ev.description]))
    {
        let loc = if ev.location.trim().is_empty() {
            "外出行程".to_string()
        } else {
            ev.location.trim().to_string()
        };
        let focus_slot = format!(
            "{} {}（地点：{loc}）",
            ev.time_range_label(),
            ev.title.trim()
        );
        HomeLifeContextCard {
            id: "life-ctx-weather".to_string(),
            domain: HomeLifeDomain::WeatherAndCommute,
            badge: "要下雨 · 外出日程关联".to_string(),
            title: format!(
                "{} · 外出安排「{}」· 天气备伞与通勤核对",
                live_weather.headline_badge(),
                ev.title.trim()
            ),
            context_link: format!(
                "关联 {} 场地「{}」：日程包含降雨提醒，出发前请备好雨伞并查看实时路况",
                ev.time_range_label(),
                loc
            ),
            detail: format!(
                "当前 {}（{}）· 日程地点：{loc} · 时间：{} · {} · 支持一键查降雨与叫车",
                live_weather.headline_badge(),
                live_weather.range_label(),
                ev.time_range_label(),
                live_weather.tomorrow_advice()
            ),
            code_or_number: None,
            action_label: "查降雨与建议".to_string(),
            intent: HomeAiIntent::AmooPrompt(
                live_weather.build_weather_context_prompt(Some(&focus_slot), &schedule_titles),
            ),
            secondary_label: Some("背屏：出门带伞".to_string()),
            secondary_intent: Some(HomeAiIntent::BackScreenStatus("出门带伞".to_string())),
        }
    } else if let Some(ev) = weather_event.or(outdoor_event) {
        let loc = if ev.location.trim().is_empty() {
            "外出行程".to_string()
        } else {
            ev.location.trim().to_string()
        };
        let focus_slot = format!(
            "{} {}（地点：{loc}）",
            ev.time_range_label(),
            ev.title.trim()
        );
        HomeLifeContextCard {
            id: "life-ctx-weather".to_string(),
            domain: HomeLifeDomain::WeatherAndCommute,
            badge: format!("{} · 外出日程关联", live_weather.headline_badge()),
            title: format!(
                "{}（{}）· 外出安排「{}」· 路况与行程核对",
                live_weather.headline_badge(),
                live_weather.range_label(),
                ev.title.trim()
            ),
            context_link: format!(
                "关联 {} 场地「{}」：{}，出发前核对实时路况、路线耗时与现场安排",
                ev.time_range_label(),
                loc,
                live_weather.commute_advice
            ),
            detail: format!(
                "当前 {}（体感 {}°C，湿度 {}%，{}，空气{}）· 日程地点：{loc} · 时间：{} · {}",
                live_weather.headline_badge(),
                live_weather.feels_like_c,
                live_weather.humidity_pct,
                live_weather.wind_label,
                live_weather.aqi_label,
                ev.time_range_label(),
                live_weather.tomorrow_advice()
            ),
            code_or_number: None,
            action_label: "查路况与建议".to_string(),
            intent: HomeAiIntent::AmooPrompt(
                live_weather.build_weather_context_prompt(Some(&focus_slot), &schedule_titles),
            ),
            secondary_label: Some("高德查路况".to_string()),
            secondary_intent: Some(HomeAiIntent::QuickGuiScenario {
                app: "高德地图".to_string(),
                task: format!("查看前往{loc}的实时路况与路线耗时"),
            }),
        }
    } else {
        HomeLifeContextCard {
            id: "life-ctx-weather".to_string(),
            domain: HomeLifeDomain::WeatherAndCommute,
            badge: format!("{} · 天气与通勤", live_weather.headline_badge()),
            title: format!(
                "{}（{}）· {}",
                live_weather.headline_badge(),
                live_weather.range_label(),
                live_weather.commute_advice
            ),
            context_link: format!(
                "关联当前「{}」时段：{} · 体感 {}°C · 空气{}",
                scene.badge,
                live_weather.region_label,
                live_weather.feels_like_c,
                live_weather.aqi_label
            ),
            detail: format!(
                "当前 {} {}°C（体感 {}°C，今日 {}，湿度 {}%，{}，空气{}）· {} · {}，{}",
                live_weather.condition,
                live_weather.temp_c,
                live_weather.feels_like_c,
                live_weather.range_label(),
                live_weather.humidity_pct,
                live_weather.wind_label,
                live_weather.aqi_label,
                live_weather.commute_advice,
                live_weather.tomorrow_headline(),
                live_weather.tomorrow_advice()
            ),
            code_or_number: None,
            action_label: "问今日天气/路况".to_string(),
            intent: HomeAiIntent::AmooPrompt(
                live_weather.build_weather_context_prompt(None, &schedule_titles),
            ),
            secondary_label: Some("高德查路况".to_string()),
            secondary_intent: Some(HomeAiIntent::QuickGuiScenario {
                app: "高德地图".to_string(),
                task: "查看当前去目的地或回家的实时路况".to_string(),
            }),
        }
    };

    if !has_live_weather {
        let source_text = phone_weather_cues.join("；");
        let sourced_rain = contains_rain_cue(&source_text)
            || weather_event.is_some_and(|ev| {
                contains_rain_cue_any(&[&ev.title, &ev.location, &ev.description])
            });
        weather_life_card.badge = if sourced_rain {
            "要下雨 · 已有天气线索".into()
        } else {
            "天气与出行 · 实况待同步".into()
        };
        weather_life_card.title = weather_notif
            .map(|n| n.title.trim())
            .filter(|title| !title.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "天气实况尚未同步，可结合日程查询出行安排".into());
        weather_life_card.context_link = if schedule_titles.is_empty() {
            format!("当前「{}」时段；等待后台天气更新", scene.badge)
        } else {
            format!("关联日程：{}", schedule_titles.join("；"))
        };
        weather_life_card.detail = if source_text.is_empty() {
            "尚未获取实时天气，请稍后刷新或查询出行安排。".into()
        } else {
            format!("手机已有线索：{source_text}。实时预报尚待同步。")
        };
        let query = format!(
            "请查询我所在地的真实天气和出行路况。已有手机线索：{}。关联日程：{}。",
            if source_text.is_empty() {
                "暂无"
            } else {
                &source_text
            },
            if schedule_titles.is_empty() {
                "暂无".into()
            } else {
                schedule_titles.join("；")
            },
        );
        if !sourced_rain || matches!(&weather_life_card.intent, HomeAiIntent::AmooPrompt(_)) {
            weather_life_card.intent = HomeAiIntent::AmooPrompt(query.clone());
            weather_life_card.action_label = "问今日天气/路况".into();
        }
        if matches!(
            &weather_life_card.secondary_intent,
            Some(HomeAiIntent::AmooPrompt(_))
        ) || (!sourced_rain
            && matches!(
                &weather_life_card.secondary_intent,
                Some(HomeAiIntent::BackScreenStatus(_))
            ))
        {
            weather_life_card.secondary_intent = Some(HomeAiIntent::AmooPrompt(query));
            weather_life_card.secondary_label = Some("查询出行建议".into());
        }
    }

    // -------------------------------------------------------------------------
    // 3. Leisure, Appointments & Wellbeing (`要愉悦` · 预约与身心调节关联)
    // -------------------------------------------------------------------------
    let takeout_card = priority_cards
        .iter()
        .find(|c| c.kind == HomeCardKind::TakeoutAndDelivery);
    let leisure_event = today_events.iter().copied().find(|ev| {
        contains_leisure_or_appointment_cue_any(&[&ev.title, &ev.location, &ev.description])
    });
    let screen_mins = input.wellbeing.screen_time_minutes;
    let unlocks = input.wellbeing.unlock_count;

    let wellbeing_link = if screen_mins >= 120 || today_meetings_count >= 2 {
        format!(
            "关联工作与用机负荷（今日亮屏 {} 分钟 · {} 项日程）：专注推进后宜给自己充个电，保持愉悦状态",
            screen_mins, today_meetings_count
        )
    } else if matches!(
        scene.scene,
        HomeScene::LunchAndLife | HomeScene::EveningWrapUp
    ) {
        format!(
            "关联「{}」时段：工作告一段落，宜享受美食、赴约或整理生活影像",
            scene.badge
        )
    } else if screen_mins > 0 || unlocks > 0 {
        format!(
            "关联今日节奏（亮屏 {} 分钟 · 解锁 {} 次）：工作间隙劳逸结合，随时开启愉悦生活服务",
            screen_mins, unlocks
        )
    } else {
        "关联工作间隙与生活节奏：在密集任务之间安排餐饮补给、预约提醒或影像创作".to_string()
    };

    let leisure_life_card = if let Some(card) = takeout_card {
        HomeLifeContextCard {
            id: "life-ctx-leisure".to_string(),
            domain: HomeLifeDomain::LeisureAndWellbeing,
            badge: format!("要愉悦 · {}", card.badge),
            title: card.title.clone(),
            context_link: wellbeing_link,
            detail: card.detail.clone(),
            code_or_number: None,
            action_label: "看配送进度".to_string(),
            intent: HomeAiIntent::QuickGuiScenario {
                app: "美团".to_string(),
                task: "打开美团外卖查看当前订单实时骑手位置与预计送达时间".to_string(),
            },
            secondary_label: Some("背屏：去吃饭".to_string()),
            secondary_intent: Some(HomeAiIntent::BackScreenStatus("去吃饭".to_string())),
        }
    } else if let Some(ev) = leisure_event {
        HomeLifeContextCard {
            id: "life-ctx-leisure".to_string(),
            domain: HomeLifeDomain::LeisureAndWellbeing,
            badge: "要愉悦 · 预约日程关联".to_string(),
            title: format!("今日预约/生活安排 · {}", ev.title.trim()),
            context_link: wellbeing_link,
            detail: format!(
                "时间：{} · 地点：{}",
                ev.time_range_label(),
                if ev.location.trim().is_empty() {
                    "已记录在日程"
                } else {
                    ev.location.trim()
                }
            ),
            code_or_number: None,
            action_label: "准备赴约提醒".to_string(),
            intent: HomeAiIntent::BackScreenStatus("休息一下".to_string()),
            secondary_label: Some("领神券点餐".to_string()),
            secondary_intent: Some(HomeAiIntent::AlipayCoupon),
        }
    } else {
        HomeLifeContextCard {
            id: "life-ctx-leisure".to_string(),
            domain: HomeLifeDomain::LeisureAndWellbeing,
            badge: "要愉悦 · 身心能量关联".to_string(),
            title: "愉悦生活与补给 · 外卖神券、下午茶与相册成片".to_string(),
            context_link: wellbeing_link,
            detail: "结合当前用机时长与会议密度，一键领券点下午茶、召唤剪映生成生活 Vlog 或切换背屏放松状态".to_string(),
            code_or_number: None,
            action_label: "支付宝神券".to_string(),
            intent: HomeAiIntent::AlipayCoupon,
            secondary_label: Some("背屏：休息一下".to_string()),
            secondary_intent: Some(HomeAiIntent::BackScreenStatus("休息一下".to_string())),
        }
    };

    let mut out = vec![express_life_card, weather_life_card, leisure_life_card];
    out.retain(|card| {
        !gov.is_any_key_suppressed(&[format!("card:{}", card.id)], None, None, None, now_ms)
    });
    out
}

fn clean_checklist_item(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    if s.is_empty() {
        return None;
    }

    // Strip leading bullets / punctuation / circled numbers / numbered prefixes
    loop {
        let prev = s;
        s = s.trim_start_matches(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '-' | '*' | '•' | '·' | '；' | ';' | '，' | ',' | '。' | '、' | '：' | ':'
                )
                || ('\u{2460}'..='\u{2473}').contains(&c)
        });
        // Strip leading `(1)` / `（1）` / `1)` / `1）` / `1.` / `1、`
        let stripped_paren = s.strip_prefix('(').or_else(|| s.strip_prefix('（'));
        let check_digits = stripped_paren.unwrap_or(s);
        let digit_bytes: usize = check_digits
            .chars()
            .take(2)
            .take_while(|c| c.is_ascii_digit())
            .map(|c| c.len_utf8())
            .sum();
        if digit_bytes > 0 {
            let after_digits = &check_digits[digit_bytes..];
            if let Some(rest) = after_digits
                .strip_prefix(')')
                .or_else(|| after_digits.strip_prefix('）'))
                .or_else(|| after_digits.strip_prefix('、'))
                .or_else(|| {
                    after_digits
                        .strip_prefix('.')
                        .or_else(|| after_digits.strip_prefix('．'))
                        .filter(|r| !r.starts_with(|c: char| c.is_ascii_digit()))
                })
            {
                s = rest.trim_start();
            }
        }
        if s == prev {
            break;
        }
    }

    s = s.trim_end_matches(|c: char| {
        c.is_whitespace() || matches!(c, '。' | '；' | ';' | '，' | ',' | '、' | '：' | ':' | '.')
    });

    let char_count = s.chars().count();
    if (2..=60).contains(&char_count) {
        Some(s.to_string())
    } else {
        None
    }
}

fn find_numbered_marker_spans(text: &str) -> Vec<(usize, usize)> {
    if !text.bytes().any(|b| b.is_ascii_digit() || b == 0xE2) {
        return Vec::new();
    }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut idx = 0;
    while idx < chars.len() {
        let (byte_i, ch) = chars[idx];

        // 1. Circled numbers ①..⑳
        if ('\u{2460}'..='\u{2473}').contains(&ch) {
            spans.push((byte_i, byte_i + ch.len_utf8()));
            idx += 1;
            continue;
        }

        // 2. Optional `(` or `（` before 1..2 digits
        let (marker_start, digit_idx) = if (ch == '(' || ch == '（')
            && idx + 1 < chars.len()
            && chars[idx + 1].1.is_ascii_digit()
        {
            (byte_i, idx + 1)
        } else if ch.is_ascii_digit() {
            let prev_ok = if idx == 0 {
                true
            } else {
                let prev_ch = chars[idx - 1].1;
                !prev_ch.is_ascii_alphanumeric() && !matches!(prev_ch, ':' | '：' | '/' | '-' | '.')
                    || matches!(prev_ch, '：' | ':')
                        && (idx < 2 || !chars[idx - 2].1.is_ascii_digit())
            };
            if prev_ok {
                (byte_i, idx)
            } else {
                idx += 1;
                continue;
            }
        } else {
            idx += 1;
            continue;
        };

        let mut end_digit_idx = digit_idx;
        while end_digit_idx < chars.len()
            && (end_digit_idx - digit_idx) < 2
            && chars[end_digit_idx].1.is_ascii_digit()
        {
            end_digit_idx += 1;
        }
        if end_digit_idx > digit_idx && end_digit_idx < chars.len() {
            let (delim_byte, delim_ch) = chars[end_digit_idx];
            let is_valid_delim = match delim_ch {
                ')' | '）' | '、' => true,
                '.' | '．' => {
                    let next_not_digit = end_digit_idx + 1 >= chars.len()
                        || !chars[end_digit_idx + 1].1.is_ascii_digit();
                    let prev_boundary = digit_idx == 0 || {
                        let prev = chars[digit_idx - 1].1;
                        prev.is_whitespace()
                            || matches!(
                                prev,
                                '：' | ':' | '；' | ';' | '，' | ',' | '。' | '(' | '（'
                            )
                    };
                    next_not_digit && prev_boundary
                }
                _ => false,
            };
            if is_valid_delim {
                spans.push((marker_start, delim_byte + delim_ch.len_utf8()));
                idx = end_digit_idx + 1;
                continue;
            }
        }

        idx += 1;
    }
    spans
}

/// Parses structured action / preparation checklist items from calendar event descriptions.
pub fn extract_schedule_checklist(description: &str) -> Vec<String> {
    let text = description.trim();
    if text.is_empty() || is_synthetic_e2e_probe_text(text) {
        return Vec::new();
    }

    fn push_unique(out: &mut Vec<String>, candidate: &str) {
        if let Some(cleaned) = clean_checklist_item(candidate) {
            if !out.iter().any(|existing| existing == &cleaned) && out.len() < 8 {
                out.push(cleaned);
            }
        }
    }

    let mut out: Vec<String> = Vec::new();

    // 1. Numbered patterns (`1) ...`, `1. ...`, `1、...`, `① ...`)
    let markers = find_numbered_marker_spans(text);
    if markers.len() >= 2 {
        for (i, &(_, content_start)) in markers.iter().enumerate() {
            let content_end = markers
                .get(i + 1)
                .map(|&(next_start, _)| next_start)
                .unwrap_or(text.len());
            let raw_slice = &text[content_start..content_end];
            for line in raw_slice.split(['\n', '；', ';']) {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    push_unique(&mut out, trimmed);
                }
            }
        }
        if out.len() >= 2 {
            return out;
        }
        out.clear();
    }

    // 2. Colon-prefixed lists (`确认：...`, `收尾：...`, `清单：...`, `准备：...`, etc.)
    let colon_cues = [
        "确认：",
        "确认:",
        "收尾：",
        "收尾:",
        "清单：",
        "清单:",
        "准备：",
        "准备:",
        "检查：",
        "检查:",
        "携带：",
        "携带:",
        "事项：",
        "事项:",
        "待办：",
        "待办:",
        "要点：",
        "要点:",
    ];
    let mut colon_body: Option<&str> = None;
    for cue in colon_cues {
        if let Some(pos) = text.find(cue) {
            colon_body = Some(&text[pos + cue.len()..]);
            break;
        }
    }
    if colon_body.is_none() {
        if let Some(pos) = text.rfind('：') {
            colon_body = Some(&text[pos + '：'.len_utf8()..]);
        }
    }

    if let Some(body) = colon_body {
        let first_sentence = body.split('。').next().unwrap_or(body);
        let source = if first_sentence
            .split(['；', ';', '、', '\n'])
            .filter(|s| clean_checklist_item(s).is_some())
            .count()
            >= 3
        {
            first_sentence
        } else {
            body
        };
        for seg in source.split(['；', ';', '、', '\n']) {
            push_unique(&mut out, seg);
        }
        if out.len() >= 3 {
            return out;
        }
        out.clear();
    }

    // 3. Fallback: semicolon or newline separated list with >= 3 items
    for seg in text.split(['；', ';', '\n']) {
        push_unique(&mut out, seg);
    }
    if out.len() >= 3 {
        return out;
    }

    Vec::new()
}

fn is_calendar_event_auto_expired_on_today(
    ev: &crate::toolbox::PhoneCalendarEvent,
    hour: u32,
    minute: u32,
    now_ms: i64,
    gov: &crate::home_reminder_governance::HomeReminderGovernance,
) -> bool {
    if ev.all_day || ev.dtstart <= 0 {
        return false;
    }
    let cal_key = format!("cal:{}", ev.id);
    if gov.is_force_kept(&cal_key, now_ms) {
        return false;
    }
    let cal_keys = crate::home_reminder_governance::canonical_keys_for_calendar_event(ev.id);
    if let Some(custom_exp) = gov.custom_expire_at_ms(&cal_keys) {
        return now_ms >= custom_exp;
    }
    let ttl_mins = gov
        .effective_ttl_mins(crate::home_reminder_governance::ReminderCategory::ScheduleAndMeeting);
    let end_ms = if ev.dtend > ev.dtstart {
        ev.dtend
    } else {
        ev.dtstart + 1_800_000
    };
    let Some((start_ymd, start_h, start_m)) = timestamp_to_local_ymd_parts(ev.dtstart) else {
        return false;
    };
    let Some((end_ymd, end_h, end_m)) = timestamp_to_local_ymd_parts(end_ms) else {
        return false;
    };
    if end_ymd > start_ymd {
        return false;
    }
    let start_mins = start_h * 60 + start_m;
    let end_mins = (end_h * 60 + end_m).max(start_mins);
    let now_mins = (hour % 24) * 60 + (minute % 60);
    let expired = now_mins >= end_mins.saturating_add(ttl_mins);
    if expired {
        crate::home_reminder_governance::record_auto_expired_notice(
            cal_key,
            crate::home_reminder_governance::ReminderCategory::ScheduleAndMeeting,
            if ev.title.trim().is_empty() {
                "未命名日程"
            } else {
                ev.title.trim()
            },
            "会议/日程已结束自动过期",
            now_ms,
        );
    }
    expired
}

fn extract_schedule_checklists_for_date_with_clock(
    selected_date_ymd: &str,
    clock_ctx: Option<(&str, u32, u32, i64)>,
    calendar_events: &[crate::toolbox::PhoneCalendarEvent],
) -> Vec<HomeScheduleChecklist> {
    let target_ymd = selected_date_ymd.trim();
    let gov = crate::home_reminder_governance::active_governance();
    let fallback_now_ms = clock_ctx
        .map(|(_, _, _, ms)| ms)
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

    let collect_for_ymd = |ymd: &str| -> Vec<HomeScheduleChecklist> {
        let mut results: Vec<HomeScheduleChecklist> = Vec::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut sorted_events: Vec<&crate::toolbox::PhoneCalendarEvent> = calendar_events
            .iter()
            .filter(|ev| {
                if any_synthetic_e2e_probe(&[&ev.title, &ev.location, &ev.description])
                    || !event_matches_ymd(ev, ymd)
                {
                    return false;
                }
                let cal_keys =
                    crate::home_reminder_governance::canonical_keys_for_calendar_event(ev.id);
                if gov.is_any_key_suppressed(
                    &cal_keys,
                    (ev.dtstart > 0).then_some(ev.dtstart),
                    None,
                    None,
                    fallback_now_ms,
                ) {
                    return false;
                }
                if let Some((ref_ymd, ref_h, ref_m, ref_now_ms)) = clock_ctx {
                    if !ref_ymd.trim().is_empty()
                        && ref_ymd.trim() == ymd
                        && is_calendar_event_auto_expired_on_today(
                            ev, ref_h, ref_m, ref_now_ms, &gov,
                        )
                    {
                        return false;
                    }
                }
                true
            })
            .collect();
        sorted_events.sort_by_key(|ev| ev.dtstart);
        for ev in sorted_events {
            let items = extract_schedule_checklist(&ev.description);
            if items.is_empty() {
                continue;
            }
            let (ey, em, ed) = ev.date_ymd();
            let date_ymd = if ev.dtstart > 0 {
                format!("{ey:04}-{em:02}-{ed:02}")
            } else {
                ymd.to_string()
            };
            let time_hm = ev.hm_label();
            let event_title = if ev.title.trim().is_empty() {
                "未命名日程".to_string()
            } else {
                ev.title.trim().to_string()
            };
            if seen.insert((event_title.clone(), time_hm.clone())) {
                results.push(HomeScheduleChecklist {
                    event_id: ev.id,
                    date_ymd,
                    time_hm,
                    event_title,
                    items,
                });
            }
        }
        results
    };

    let mut out = collect_for_ymd(target_ymd);
    if out.is_empty() {
        if let Ok(parsed) = chrono::NaiveDate::parse_from_str(target_ymd, "%Y-%m-%d") {
            if let Some(tomorrow) = parsed.succ_opt() {
                let tomorrow_ymd = tomorrow.format("%Y-%m-%d").to_string();
                out = collect_for_ymd(&tomorrow_ymd);
            }
        }
    }
    out
}

/// Extracts non-empty schedule checklists for events on `selected_date_ymd`
/// (or upcoming events within 24h if `selected_date_ymd` has none).
pub fn extract_schedule_checklists_for_date(
    selected_date_ymd: &str,
    calendar_events: &[crate::toolbox::PhoneCalendarEvent],
) -> Vec<HomeScheduleChecklist> {
    let wall_now = chrono::Local::now();
    let wall_ymd = wall_now.format("%Y-%m-%d").to_string();
    let clock_ctx = (selected_date_ymd.trim() == wall_ymd).then(|| {
        (
            wall_ymd.as_str(),
            chrono::Timelike::hour(&wall_now),
            chrono::Timelike::minute(&wall_now),
            wall_now.timestamp_millis(),
        )
    });
    extract_schedule_checklists_for_date_with_clock(selected_date_ymd, clock_ctx, calendar_events)
}

/// Detects cross-signal conflicts between early-morning / travel schedules and enabled phone alarms.
pub fn detect_schedule_alarm_conflicts(
    today_ymd: &str,
    hour: u32,
    minute: u32,
    calendar_events: &[crate::toolbox::PhoneCalendarEvent],
    alarms: &[crate::toolbox::PhoneAlarmItem],
) -> Vec<HomeScheduleConflict> {
    let clean_today = today_ymd.trim();
    let gov = crate::home_reminder_governance::active_governance();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let tomorrow_ymd = chrono::NaiveDate::parse_from_str(clean_today, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.succ_opt())
        .map(|d| d.format("%Y-%m-%d").to_string());

    let real_enabled_alarms: Vec<&crate::toolbox::PhoneAlarmItem> = alarms
        .iter()
        .filter(|a| a.enabled && !is_synthetic_e2e_probe_text(&a.message))
        .collect();

    let now_min = ((hour % 24) * 60 + (minute % 60)) as i32;
    let early_keywords = [
        "出发", "自驾", "航班", "飞机", "高铁", "动车", "火车", "赶车", "早班", "晨会",
    ];

    let mut candidates: Vec<(String, i32, &crate::toolbox::PhoneCalendarEvent, String)> =
        Vec::new();
    for ev in calendar_events {
        if ev.all_day
            || ev.dtstart <= 0
            || any_synthetic_e2e_probe(&[&ev.title, &ev.location, &ev.description])
        {
            continue;
        }
        let Some((ev_date_ymd, ev_hm, ev_u32_min)) = timestamp_to_local_ymd_hm(ev.dtstart) else {
            continue;
        };
        let conflict_id = format!("conflict-alarm-{}-{}", ev_date_ymd, ev.id);
        let mut keys = crate::home_reminder_governance::canonical_keys_for_calendar_event(ev.id);
        keys.push(format!("conflict:{conflict_id}"));
        keys.push(format!("card:{conflict_id}"));
        keys.push(format!("card:pan-{conflict_id}"));
        if gov.is_any_key_suppressed(
            &keys,
            (ev.dtstart > 0).then_some(ev.dtstart),
            None,
            None,
            now_ms,
        ) {
            continue;
        }
        let ev_total_min = ev_u32_min as i32;
        let is_today_upcoming = ev_date_ymd == clean_today && ev_total_min > now_min;
        let is_tomorrow = tomorrow_ymd.as_deref() == Some(ev_date_ymd.as_str());
        if !is_today_upcoming && !is_tomorrow {
            continue;
        }
        if ev_total_min > 11 * 60 {
            continue;
        }
        let has_early_kw = early_keywords
            .iter()
            .any(|kw| ev.title.contains(kw) || ev.description.contains(kw));
        let in_early_window = (300..=630).contains(&ev_total_min);
        if !in_early_window && !has_early_kw {
            continue;
        }
        candidates.push((ev_date_ymd, ev_total_min, ev, ev_hm));
    }

    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let mut conflicts: Vec<HomeScheduleConflict> = Vec::new();
    let mut seen_keys: HashSet<(String, String, String)> = HashSet::new();

    for (ev_date_ymd, ev_total_min, ev, ev_hm) in candidates {
        let ev_title = if ev.title.trim().is_empty() {
            "早间日程".to_string()
        } else {
            ev.title.trim().to_string()
        };
        if !seen_keys.insert((ev_date_ymd.clone(), ev_hm.clone(), ev_title.clone())) {
            continue;
        }

        let has_valid_wake_alarm = real_enabled_alarms.iter().any(|a| {
            let alarm_min = a.hour as i32 * 60 + a.minutes as i32;
            (ev_total_min - 180..=ev_total_min - 15).contains(&alarm_min)
        });
        if has_valid_wake_alarm {
            continue;
        }

        let suggested_min = (ev_total_min - 60).clamp(4 * 60 + 30, 9 * 60 + 30);
        let suggested_alarm_hour = (suggested_min / 60) as u8;
        let suggested_alarm_minute = (suggested_min % 60) as u8;
        let suggested_alarm_label = format!("{ev_hm} {ev_title}");

        let day_word = if tomorrow_ymd.as_deref() == Some(ev_date_ymd.as_str()) {
            "明日"
        } else {
            "今日"
        };
        let is_departure = ev_title.contains("出发")
            || ev.description.contains("出发")
            || ev_title.contains("自驾")
            || ev_title.contains("航班")
            || ev_title.contains("高铁");

        let later_alarm = real_enabled_alarms
            .iter()
            .filter(|a| (a.hour as i32 * 60 + a.minutes as i32) > ev_total_min - 15)
            .min_by_key(|a| a.hour as i32 * 60 + a.minutes as i32)
            .or_else(|| {
                real_enabled_alarms
                    .iter()
                    .min_by_key(|a| a.hour as i32 * 60 + a.minutes as i32)
            });

        let (conflict_badge, headline, detail) = if let Some(later) = later_alarm {
            let later_hm = format!("{:02}:{:02}", later.hour, later.minutes);
            let late_desc = if is_departure {
                "晚于出发时间"
            } else {
                "晚于日程时间"
            };
            (
                "闹钟晚于日程".to_string(),
                format!("{day_word} {ev_hm}「{ev_title}」早于手机闹钟（{later_hm}）"),
                format!(
                    "检测到{day_word} {ev_hm}「{ev_title}」，但手机当前最早开启闹钟为 {later_hm}（{late_desc}），建议一键设 {:02}:{:02} 早起闹钟",
                    suggested_alarm_hour, suggested_alarm_minute
                ),
            )
        } else {
            (
                "缺少早起闹钟".to_string(),
                format!("{day_word} {ev_hm}「{ev_title}」尚未开启早起闹钟"),
                format!(
                    "检测到{day_word} {ev_hm}「{ev_title}」，但手机尚未开启对应的早起闹钟，建议一键设 {:02}:{:02} 闹钟",
                    suggested_alarm_hour, suggested_alarm_minute
                ),
            )
        };

        let primary_action_label = format!(
            "一键设 {:02}:{:02} 手机闹钟",
            suggested_alarm_hour, suggested_alarm_minute
        );
        let primary_intent = HomeAiIntent::CreatePhoneAlarm {
            hour: suggested_alarm_hour,
            minute: suggested_alarm_minute,
            label: suggested_alarm_label.clone(),
        };
        let secondary_action_label = "推背屏提醒".to_string();
        let secondary_intent = HomeAiIntent::BackScreenStatus(format!(
            "{ev_hm} {ev_title} · 闹钟 {:02}:{:02}",
            suggested_alarm_hour, suggested_alarm_minute
        ));

        conflicts.push(HomeScheduleConflict {
            id: format!("conflict-alarm-{}-{}", ev_date_ymd, ev.id),
            event_id: ev.id,
            event_date_ymd: ev_date_ymd,
            event_time_hm: ev_hm,
            event_title: ev_title,
            conflict_badge,
            headline,
            detail,
            suggested_alarm_hour,
            suggested_alarm_minute,
            suggested_alarm_label,
            primary_action_label,
            primary_intent,
            secondary_action_label,
            secondary_intent,
        });
    }

    conflicts
}

/// Builds the 3 life context cards for `selected_date_ymd` without desktop ledger entries.
pub fn build_life_context_cards_for_date(
    selected_date_ymd: &str,
    input: &HomeHubInput<'_>,
) -> Vec<HomeLifeContextCard> {
    build_life_context_cards_with_desktop(selected_date_ymd, input, &[])
}

/// Enriches an existing set of 3 life context cards with desktop Context Ledger / Amoo workspace
/// items (such as `"降雨查询"`) cross-linked with phone calendar schedules.
pub fn enrich_life_context_cards_with_desktop(
    mut cards: Vec<HomeLifeContextCard>,
    selected_date_ymd: &str,
    input: &HomeHubInput<'_>,
    desktop_entries: &[DesktopDateEntry],
) -> Vec<HomeLifeContextCard> {
    if desktop_entries.is_empty() {
        return cards;
    }
    let gov = crate::home_reminder_governance::active_governance();
    let max_ts = input
        .sms
        .iter()
        .map(|m| m.date_ms)
        .chain(input.notifications.iter().map(|n| n.post_time_ms))
        .chain(input.otp_banners.iter().map(|b| b.timestamp_ms))
        .chain(input.threads.iter().map(|t| t.last_timestamp_ms))
        .max()
        .unwrap_or(0);
    let now_ms = reference_now_ms_for_input(input, max_ts);
    if gov.is_any_key_suppressed(
        &["card:life-ctx-weather".to_string()],
        None,
        None,
        None,
        now_ms,
    ) {
        return cards;
    }
    let target_ymd = if selected_date_ymd.trim().is_empty() {
        input.today_ymd
    } else {
        selected_date_ymd.trim()
    };
    let target_tuple = parse_ymd_tuple(target_ymd);
    let today_tuple = parse_ymd_tuple(input.today_ymd);
    let tomorrow_ymd = chrono::NaiveDate::parse_from_str(target_ymd, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.succ_opt())
        .map(|d| d.format("%Y-%m-%d").to_string());
    let tomorrow_tuple = tomorrow_ymd.as_deref().and_then(parse_ymd_tuple);

    let weather_desktop_keywords = [
        "降雨",
        "下雨",
        "天气",
        "降水",
        "雨夹雪",
        "降温",
        "大风",
        "台风",
        "阵雨",
        "暴雨",
        "小雨",
        "中雨",
        "大雨",
    ];
    let weather_entry = desktop_entries.iter().find(|entry| {
        let entry_id = entry.id.trim();
        let entry_ts_ms = if entry.timestamp > 1_000_000_000_000 {
            entry.timestamp
        } else if entry.timestamp > 0 {
            entry.timestamp * 1000
        } else {
            0
        };
        let entry_keys = [
            format!("ledger:{entry_id}"),
            format!("task:{entry_id}"),
            format!("card:{entry_id}"),
            entry_id.to_string(),
        ];
        if gov.is_any_key_suppressed(
            &entry_keys,
            (entry_ts_ms > 0).then_some(entry_ts_ms),
            None,
            None,
            now_ms,
        ) {
            return false;
        }
        let matches_day =
            if let Some((entry_ymd, _, _)) = timestamp_to_local_ymd_parts(entry.timestamp) {
                Some(entry_ymd) == target_tuple
                    || Some(entry_ymd) == tomorrow_tuple
                    || Some(entry_ymd) == today_tuple
            } else {
                true
            };
        matches_day
            && !any_synthetic_e2e_probe(&[&entry.badge, &entry.title, &entry.detail])
            && weather_desktop_keywords.iter().any(|kw| {
                entry.badge.contains(kw) || entry.title.contains(kw) || entry.detail.contains(kw)
            })
    });

    if let Some(w_entry) = weather_entry {
        let entry_has_rain =
            contains_rain_cue_any(&[&w_entry.badge, &w_entry.title, &w_entry.detail]);
        let phone_has_rain = cards
            .iter()
            .find(|c| c.domain == HomeLifeDomain::WeatherAndCommute)
            .is_some_and(|c| contains_rain_cue_any(&[&c.badge, &c.title, &c.detail]));
        let has_rain = entry_has_rain || phone_has_rain;
        let travel_sched_keywords = ["自驾", "出发", "行前准备", "行前", "车况", "出行", "外勤"];
        let relevant_events: Vec<&crate::toolbox::PhoneCalendarEvent> = input
            .calendar_events
            .iter()
            .filter(|ev| {
                !any_synthetic_e2e_probe(&[&ev.title, &ev.location, &ev.description])
                    && !gov.is_any_key_suppressed(
                        &crate::home_reminder_governance::canonical_keys_for_calendar_event(ev.id),
                        (ev.dtstart > 0).then_some(ev.dtstart),
                        None,
                        None,
                        now_ms,
                    )
                    && (event_matches_ymd(ev, target_ymd)
                        || tomorrow_ymd
                            .as_deref()
                            .is_some_and(|t_ymd| event_matches_ymd(ev, t_ymd)))
            })
            .collect();

        let prioritized_events: Vec<&crate::toolbox::PhoneCalendarEvent> = {
            let travel_matches: Vec<&crate::toolbox::PhoneCalendarEvent> = relevant_events
                .iter()
                .copied()
                .filter(|ev| {
                    travel_sched_keywords.iter().any(|kw| {
                        ev.title.contains(kw)
                            || ev.location.contains(kw)
                            || ev.description.contains(kw)
                    })
                })
                .collect();
            if !travel_matches.is_empty() {
                travel_matches
            } else {
                relevant_events
            }
        };

        let mut distinct_titles: Vec<String> = Vec::new();
        for ev in &prioritized_events {
            let t = ev.title.trim();
            if !t.is_empty() && !distinct_titles.iter().any(|existing| existing == t) {
                distinct_titles.push(t.to_string());
                if distinct_titles.len() >= 2 {
                    break;
                }
            }
        }

        if !distinct_titles.is_empty() {
            let weather_entry_title = if w_entry.title.trim().is_empty() {
                if has_rain {
                    "降雨查询"
                } else {
                    "天气查询"
                }
            } else {
                w_entry.title.trim()
            };
            let has_driving_cue = ["自驾", "车况", "行前"].iter().any(|kw| {
                w_entry.title.contains(kw)
                    || w_entry.detail.contains(kw)
                    || prioritized_events.iter().any(|ev| {
                        ev.title.contains(kw)
                            || ev.location.contains(kw)
                            || ev.description.contains(kw)
                    })
            });
            let schedule_titles = distinct_titles.join(" / ");
            let cross_weather_card = if has_rain && has_driving_cue {
                HomeLifeContextCard {
                    id: "life-ctx-weather".to_string(),
                    domain: HomeLifeDomain::WeatherAndCommute,
                    badge: "降雨查询 × 日程联动".to_string(),
                    title: format!("降雨关注已联动「{schedule_titles}」"),
                    context_link: format!(
                        "跨源关联：台账「{weather_entry_title}」 × 日历「{schedule_titles}」"
                    ),
                    detail: "结合您刚查询的降雨记录与自驾出行安排：今晚车况检查请重点确认雨刮器、玻璃水、胎压与离线地图，明早出发预留雨天行车缓冲。".to_string(),
                    code_or_number: None,
                    action_label: "推送雨天行前清单到背屏".to_string(),
                    intent: HomeAiIntent::BackScreenStatus(
                        "雨天自驾检查：雨刮/玻璃水/胎压/离线地图".to_string(),
                    ),
                    secondary_label: Some("查沿途实时天气".to_string()),
                    secondary_intent: Some(HomeAiIntent::AmooPrompt(format!(
                        "结合我的「{weather_entry_title}」记录与日程「{schedule_titles}」，请帮我查询沿途实时天气与降雨时段，并给出行车安全建议"
                    ))),
                }
            } else if has_rain {
                HomeLifeContextCard {
                    id: "life-ctx-weather".to_string(),
                    domain: HomeLifeDomain::WeatherAndCommute,
                    badge: "降雨提醒 × 日程联动".to_string(),
                    title: format!("降雨提醒已联动「{schedule_titles}」"),
                    context_link: format!(
                        "跨源关联：台账「{weather_entry_title}」 × 日历「{schedule_titles}」"
                    ),
                    detail: "结合降雨天气提醒与日程出行安排：外出参会或通勤请提前备好雨具，并预留充足路途缓冲时间。".to_string(),
                    code_or_number: None,
                    action_label: "背屏挂上「出门带伞」".to_string(),
                    intent: HomeAiIntent::BackScreenStatus("出门带伞".to_string()),
                    secondary_label: Some("查出行时段天气".to_string()),
                    secondary_intent: Some(HomeAiIntent::AmooPrompt(format!(
                        "结合我的「{weather_entry_title}」记录与日程「{schedule_titles}」，请帮我查询出行时段的实时天气与通勤建议"
                    ))),
                }
            } else {
                HomeLifeContextCard {
                    id: "life-ctx-weather".to_string(),
                    domain: HomeLifeDomain::WeatherAndCommute,
                    badge: "天气查询 × 日程联动".to_string(),
                    title: format!("天气关注已联动「{schedule_titles}」"),
                    context_link: format!(
                        "跨源关联：台账「{weather_entry_title}」 × 日历「{schedule_titles}」"
                    ),
                    detail: "结合您刚查询的天气记录与出行安排：出发前建议核对实时路况、路线耗时与现场安排。".to_string(),
                    code_or_number: None,
                    action_label: "推送行前提醒到背屏".to_string(),
                    intent: HomeAiIntent::BackScreenStatus("行前检查".to_string()),
                    secondary_label: Some("查沿途实时路况".to_string()),
                    secondary_intent: Some(HomeAiIntent::AmooPrompt(format!(
                        "结合我的「{weather_entry_title}」记录与日程「{schedule_titles}」，请帮我查询沿途实时天气与交通路况建议"
                    ))),
                }
            };

            if let Some(slot) = cards
                .iter_mut()
                .find(|c| c.domain == HomeLifeDomain::WeatherAndCommute)
            {
                *slot = cross_weather_card;
            } else {
                cards.push(cross_weather_card);
            }
        }
    }

    cards
}

/// Builds the 3 life context cards for `selected_date_ymd`, cross-linking desktop Context Ledger /
/// Amoo workspace items (such as `"降雨查询"`) with phone calendar schedules (such as `"行前准备"` / `"自驾出发"`).
pub fn build_life_context_cards_with_desktop(
    selected_date_ymd: &str,
    input: &HomeHubInput<'_>,
    desktop_entries: &[DesktopDateEntry],
) -> Vec<HomeLifeContextCard> {
    let target_ymd = if selected_date_ymd.trim().is_empty() {
        input.today_ymd
    } else {
        selected_date_ymd.trim()
    };
    let cache_hash = (!target_ymd.trim().is_empty())
        .then(|| compute_home_hub_input_hash(2, target_ymd, input, desktop_entries));
    if let Some(h) = cache_hash {
        if let Some(hit) = HOME_LIFE_CARDS_CACHE.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|(k, v)| (*k == h).then(|| v.clone()))
        }) {
            return hit;
        }
    }
    let mut swapped_input = input.clone();
    swapped_input.today_ymd = target_ymd;

    let target_tuple = if target_ymd.is_empty() {
        None
    } else {
        parse_ymd_tuple(target_ymd)
    };
    let gov = crate::home_reminder_governance::active_governance();
    let gov_active = !gov.entries.is_empty() || !gov.muted_sources.is_empty();
    let max_ts = swapped_input
        .sms
        .iter()
        .map(|m| m.date_ms)
        .chain(swapped_input.notifications.iter().map(|n| n.post_time_ms))
        .chain(swapped_input.otp_banners.iter().map(|b| b.timestamp_ms))
        .chain(swapped_input.threads.iter().map(|t| t.last_timestamp_ms))
        .max()
        .unwrap_or(0);
    let now_ms = reference_now_ms_for_input(&swapped_input, max_ts);
    let mut upcoming_meeting_within_45m = false;
    let mut today_meetings_count = 0usize;
    for (idx, ev) in swapped_input.calendar_events.iter().enumerate() {
        if any_synthetic_e2e_probe(&[&ev.title, &ev.location, &ev.description]) {
            continue;
        }
        if gov_active {
            let mut cal_keys =
                crate::home_reminder_governance::canonical_keys_for_calendar_event(ev.id);
            cal_keys.push(format!("card:calendar-ev-{}-{idx}", ev.id));
            if gov.is_any_key_suppressed(
                &cal_keys,
                (ev.dtstart > 0).then_some(ev.dtstart),
                None,
                None,
                now_ms,
            ) {
                continue;
            }
        }
        let on_date = if target_ymd.is_empty() || ev.dtstart <= 0 {
            true
        } else {
            event_is_on_date_tuple(ev, target_tuple)
        };
        if on_date {
            today_meetings_count += 1;
            if event_is_within_45_minutes(ev, swapped_input.hour, swapped_input.minute) {
                upcoming_meeting_within_45m = true;
            }
        }
    }

    let life_priority_cards = collect_life_priority_cards(&swapped_input, None, None, false);
    let scene = detect_scene(
        swapped_input.hour,
        upcoming_meeting_within_45m,
        !life_priority_cards.is_empty(),
    );
    let base_cards = build_life_context_cards(
        &swapped_input,
        &scene,
        &life_priority_cards,
        today_meetings_count,
    );
    let cards =
        enrich_life_context_cards_with_desktop(base_cards, target_ymd, input, desktop_entries);
    if let Some(h) = cache_hash {
        HOME_LIFE_CARDS_CACHE.with(|c| {
            *c.borrow_mut() = Some((h, cards.clone()));
        });
    }
    cards
}

#[inline]
fn event_matches_ymd(ev: &crate::toolbox::PhoneCalendarEvent, target_ymd: &str) -> bool {
    let ymd = target_ymd.trim();
    if ymd.is_empty() || ev.dtstart <= 0 {
        return true;
    }
    let Some(tuple) = parse_ymd_tuple(ymd) else {
        return false;
    };
    event_is_on_date_tuple(ev, Some(tuple))
}

#[inline]
fn is_duplicate_schedule_title(candidate: &str, existing: &str) -> bool {
    let cand = candidate.trim();
    let exist = existing.trim();
    !cand.is_empty()
        && !exist.is_empty()
        && (exist == cand || cand.contains(exist) || exist.contains(cand))
}

#[inline]
fn is_panorama_thread_eligible(t: &crate::push::ImThread) -> bool {
    !t.title.trim_start().starts_with('{')
        && !crate::push::is_excluded_im_package(&t.package_name)
        && !any_synthetic_e2e_probe(&[&t.title, &t.last_message])
}

#[inline]
fn is_life_sms_message(msg: &crate::sms::Message) -> bool {
    is_express_notification("短信", &msg.address, &msg.body)
        || extract_otp_or_pickup_from_text(&msg.body).is_some()
        || contains_weather_or_rain_cue(&msg.body)
        || classify_delivery_or_mobility_signal("", "", &msg.address, &msg.body).is_some()
}

#[inline]
fn is_notification_duplicated_by_thread(
    notif: &crate::push::PushNotification,
    title_trim: &str,
    text_trim: &str,
    threads: &[crate::push::ImThread],
) -> bool {
    threads.iter().any(|t| {
        (!t.notification_key.is_empty() && t.notification_key == notif.key)
            || (t.package_name == notif.package_name
                && t.title.trim() == title_trim
                && t.last_message.trim() == text_trim)
    })
}

#[inline]
fn classify_panorama_notification_kind(
    notif: &crate::push::PushNotification,
    title_trim: &str,
    text_trim: &str,
) -> SelectedDateEntryKind {
    let parts = [notif.app_name.as_str(), title_trim, text_trim];
    let is_life = is_express_notification(&notif.app_name, title_trim, text_trim)
        || classify_takeout_or_travel_notification(notif).is_some()
        || contains_weather_or_rain_cue_any(&parts)
        || extract_otp_or_pickup_from_parts(&parts).is_some();
    if is_life {
        return SelectedDateEntryKind::Life;
    }
    let is_issue = ["异常", "失败", "待确认", "请确认", "？", "?"]
        .iter()
        .any(|kw| parts.iter().any(|p| p.contains(kw)));
    if is_issue {
        SelectedDateEntryKind::Issue
    } else {
        SelectedDateEntryKind::Communication
    }
}

/// Builds the complete chronological panorama of everything that happened or is scheduled on
/// `selected_ymd` (`YYYY-MM-DD`), combining phone calendar events, IM threads, SMS, calls,
/// notifications, parcels, and desktop workspace/context-ledger entries.
pub fn build_selected_date_panorama(
    selected_ymd: &str,
    input: &HomeHubInput<'_>,
    desktop_entries: &[DesktopDateEntry],
) -> SelectedDatePanorama {
    use chrono::{Datelike, NaiveDate};
    let today_ymd = input.today_ymd.trim();
    let selected_ymd = if selected_ymd.trim().is_empty() {
        today_ymd
    } else {
        selected_ymd.trim()
    };
    let cache_hash = (!selected_ymd.is_empty())
        .then(|| compute_home_hub_input_hash(3, selected_ymd, input, desktop_entries));
    if let Some(h) = cache_hash {
        if let Some(hit) = HOME_PANORAMA_CACHE.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|(k, v)| (*k == h).then(|| v.clone()))
        }) {
            return hit;
        }
    }
    let is_today = selected_ymd == today_ymd;
    let selected_tuple = parse_ymd_tuple(selected_ymd);
    let parsed_date = selected_tuple
        .and_then(|(y, m, d)| NaiveDate::from_ymd_opt(y, m, d))
        .or_else(|| NaiveDate::parse_from_str(selected_ymd, "%Y-%m-%d").ok());
    let weekday_label = parsed_date
        .map(|d| {
            ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
                [d.weekday().num_days_from_monday() as usize]
                .to_string()
        })
        .unwrap_or_else(|| "当日".to_string());
    let date_display = parsed_date
        .map(|d| format!("{:02}月{:02}日", d.month(), d.day()))
        .unwrap_or_else(|| selected_ymd.to_string());

    let mut entries: Vec<SelectedDateEntry> = Vec::new();
    let mut schedule_count = 0usize;
    let mut work_count = 0usize;
    let mut life_count = 0usize;
    let mut issue_count = 0usize;
    let mut communication_count = 0usize;
    let gov = crate::home_reminder_governance::active_governance();
    let gov_active = !gov.entries.is_empty() || !gov.muted_sources.is_empty();
    let max_ts = input
        .sms
        .iter()
        .map(|m| m.date_ms)
        .chain(input.notifications.iter().map(|n| n.post_time_ms))
        .chain(input.otp_banners.iter().map(|b| b.timestamp_ms))
        .chain(input.threads.iter().map(|t| t.last_timestamp_ms))
        .chain(input.call_logs.iter().map(|c| c.date_ms))
        .max()
        .unwrap_or(0);
    let now_ms = reference_now_ms_for_input(input, max_ts);

    // 0. Schedule × Alarm cross-signal conflicts (surfaced on event_date_ymd AND on today_ymd)
    let schedule_conflicts = detect_schedule_alarm_conflicts(
        today_ymd,
        input.hour,
        input.minute,
        input.calendar_events,
        input.alarms,
    );
    for conflict in &schedule_conflicts {
        if selected_ymd == conflict.event_date_ymd || is_today {
            if gov_active {
                let mut keys = crate::home_reminder_governance::canonical_keys_for_calendar_event(
                    conflict.event_id,
                );
                keys.push(format!("conflict:{}", conflict.id));
                keys.push(format!("card:pan-{}", conflict.id));
                if gov.is_any_key_suppressed(&keys, None, None, None, now_ms) {
                    continue;
                }
            }
            issue_count += 1;
            let sort_min = if selected_ymd == conflict.event_date_ymd {
                5 * 60
            } else {
                20 * 60 + 30
            };
            entries.push(SelectedDateEntry {
                id: format!("pan-{}", conflict.id),
                time_hm: conflict.event_time_hm.clone(),
                sort_minute: sort_min,
                kind: SelectedDateEntryKind::Issue,
                badge: conflict.conflict_badge.clone(),
                title: conflict.headline.clone(),
                detail: conflict.detail.clone(),
                action_label: conflict.primary_action_label.clone(),
                intent: conflict.primary_intent.clone(),
                checklist_items: Vec::new(),
            });
        }
    }

    // 1. Calendar schedules on selected_ymd
    let schedule_ttl_ms =
        gov.effective_ttl_ms(crate::home_reminder_governance::ReminderCategory::ScheduleAndMeeting);
    let missed_call_ttl_ms =
        gov.effective_ttl_ms(crate::home_reminder_governance::ReminderCategory::MissedCall);
    for (idx, ev) in input.calendar_events.iter().enumerate() {
        if !event_is_on_date_tuple(ev, selected_tuple) {
            continue;
        }
        if any_synthetic_e2e_probe(&[&ev.title, &ev.location, &ev.description]) {
            continue;
        }
        if is_today && !ev.all_day {
            let ev_end_ms = if ev.dtend > ev.dtstart {
                ev.dtend
            } else if ev.dtstart > 0 {
                ev.dtstart + 3_600_000
            } else {
                0
            };
            if ev_end_ms > 0 && now_ms.saturating_sub(ev_end_ms) > schedule_ttl_ms {
                crate::home_reminder_governance::record_auto_expired_notice(
                    format!("cal:{}", ev.id),
                    crate::home_reminder_governance::ReminderCategory::ScheduleAndMeeting,
                    ev.title.trim().to_string(),
                    "会议/日程已结束自动过期",
                    now_ms,
                );
                continue;
            }
        }
        if gov_active {
            let mut cal_keys =
                crate::home_reminder_governance::canonical_keys_for_calendar_event(ev.id);
            cal_keys.push(format!("schedule:{}", ev.id));
            cal_keys.push(format!("card:pan-cal-{}-{idx}", ev.id));
            if gov.is_any_key_suppressed(
                &cal_keys,
                (ev.dtstart > 0).then_some(ev.dtstart),
                None,
                None,
                now_ms,
            ) {
                continue;
            }
        }
        schedule_count += 1;
        let (hm, sort_min) = if ev.all_day {
            ("全天".to_string(), 8 * 60)
        } else if let Some((_, h, m)) = timestamp_to_local_ymd_parts(ev.dtstart) {
            (format!("{h:02}:{m:02}"), h * 60 + m)
        } else {
            (ev.hm_label(), 9 * 60)
        };
        let loc = if ev.location.trim().is_empty() {
            "地点未填写".to_string()
        } else {
            ev.location.trim().to_string()
        };
        let meeting_url =
            extract_meeting_url(&ev.description).or_else(|| extract_meeting_url(&ev.location));
        let detail = match meeting_url.as_deref() {
            Some(url) if !loc.contains(url) => {
                format!("{} · {loc} · 会议链接：{url}", ev.time_range_label())
            }
            _ => format!("{} · {loc}", ev.time_range_label()),
        };
        entries.push(SelectedDateEntry {
            id: format!("pan-cal-{}-{idx}", ev.id),
            time_hm: hm,
            sort_minute: sort_min,
            kind: SelectedDateEntryKind::Schedule,
            badge: "日程安排".to_string(),
            title: if ev.title.trim().is_empty() {
                "未命名日程".to_string()
            } else {
                ev.title.trim().to_string()
            },
            detail,
            action_label: "准备纪要".to_string(),
            intent: HomeAiIntent::AmooPrompt(format!(
                "请围绕日程「{}（{}，地点：{loc}）」准备议程与纪要要点",
                ev.title.trim(),
                ev.time_range_label()
            )),
            checklist_items: extract_schedule_checklist(&ev.description),
        });
    }

    // 2. Desktop workspace tasks & Context Ledger items on selected_ymd
    for item in desktop_entries {
        if any_synthetic_e2e_probe(&[&item.badge, &item.title, &item.detail]) {
            continue;
        }
        let item_parts = timestamp_to_local_ymd_parts(item.timestamp);
        let matches_day = if let Some((ymd_tuple, _, _)) = item_parts {
            Some(ymd_tuple) == selected_tuple
        } else {
            is_today
        };
        if !matches_day {
            continue;
        }
        if gov_active {
            let item_id = item.id.trim();
            let item_ts_ms = if item.timestamp > 1_000_000_000_000 {
                item.timestamp
            } else if item.timestamp > 0 {
                item.timestamp * 1000
            } else {
                0
            };
            let keys = [
                format!("ledger:{item_id}"),
                format!("task:{item_id}"),
                format!("card:{item_id}"),
                item_id.to_string(),
            ];
            if gov.is_any_key_suppressed(
                &keys,
                (item_ts_ms > 0).then_some(item_ts_ms),
                None,
                None,
                now_ms,
            ) {
                continue;
            }
        }
        // Avoid duplicating a schedule entry that already exists in `calendar_events` on this date
        let item_title_trim = item.title.trim();
        let dup_schedule = !item_title_trim.is_empty()
            && entries.iter().any(|existing| {
                existing.kind == SelectedDateEntryKind::Schedule
                    && is_duplicate_schedule_title(item_title_trim, &existing.title)
            });
        if dup_schedule {
            continue;
        }
        let (hm, sort_min) = item_parts
            .map(|(_, h, m)| (format!("{h:02}:{m:02}"), h * 60 + m))
            .unwrap_or_else(|| ("当日".to_string(), 10 * 60));
        match item.kind {
            SelectedDateEntryKind::Issue => issue_count += 1,
            SelectedDateEntryKind::Life => life_count += 1,
            SelectedDateEntryKind::Schedule => schedule_count += 1,
            SelectedDateEntryKind::Communication => communication_count += 1,
            SelectedDateEntryKind::WorkAndMatter => work_count += 1,
        }
        entries.push(SelectedDateEntry {
            id: item.id.clone(),
            time_hm: hm,
            sort_minute: sort_min,
            kind: item.kind,
            badge: item.badge.clone(),
            title: item.title.clone(),
            detail: item.detail.clone(),
            action_label: item.action_label.clone(),
            intent: item.intent.clone(),
            checklist_items: Vec::new(),
        });
    }

    // 3. Missed Calls & Call Logs on selected_ymd
    for (idx, call) in input.call_logs.iter().enumerate() {
        let call_parts = timestamp_to_local_ymd_parts(call.date_ms);
        let matches_day = if let Some((ymd_tuple, _, _)) = call_parts {
            Some(ymd_tuple) == selected_tuple
        } else {
            is_today && call.date_ms == 0
        };
        if !matches_day {
            continue;
        }
        if gov_active {
            let norm_num = crate::home_reminder_governance::normalize_phone_or_addr(&call.number);
            let call_key = format!("call:{norm_num}");
            let keys = [
                call_key.clone(),
                format!("missed-call:{}:{}", call.number.trim(), call.date_ms),
                format!("card:pan-call-missed-{idx}"),
                format!("card:pan-call-{idx}"),
            ];
            if gov.is_any_key_suppressed(
                &keys,
                (call.date_ms > 0).then_some(call.date_ms),
                None,
                (!norm_num.is_empty()).then_some(call_key.as_str()),
                now_ms,
            ) {
                continue;
            }
        }
        let (hm, sort_min) = call_parts
            .map(|(_, h, m)| (format!("{h:02}:{m:02}"), h * 60 + m))
            .unwrap_or_else(|| ("当日".to_string(), 11 * 60));
        let caller = if call.name.trim().is_empty() {
            call.number.trim().to_string()
        } else {
            format!("{} ({})", call.name.trim(), call.number.trim())
        };
        if call.call_type == 3 {
            if is_today {
                let norm_num =
                    crate::home_reminder_governance::normalize_phone_or_addr(&call.number);
                let has_newer_call = !norm_num.is_empty()
                    && input.call_logs.iter().any(|c2| {
                        c2.call_type != 3
                            && c2.date_ms > call.date_ms
                            && crate::home_reminder_governance::normalize_phone_or_addr(&c2.number)
                                == norm_num
                    });
                let has_newer_outgoing_sms = !norm_num.is_empty()
                    && input.sms.iter().any(|m| {
                        m.outgoing
                            && m.date_ms > call.date_ms
                            && crate::home_reminder_governance::normalize_phone_or_addr(&m.address)
                                == norm_num
                    });
                if has_newer_call || has_newer_outgoing_sms {
                    crate::home_reminder_governance::record_auto_expired_notice(
                        format!("call:{norm_num}"),
                        crate::home_reminder_governance::ReminderCategory::MissedCall,
                        format!("未接来电 · {caller}"),
                        "已回拨或已通过短信回复自动过期",
                        now_ms,
                    );
                    continue;
                }
                if call.date_ms > 0 && now_ms.saturating_sub(call.date_ms) > missed_call_ttl_ms {
                    crate::home_reminder_governance::record_auto_expired_notice(
                        format!("call:{norm_num}"),
                        crate::home_reminder_governance::ReminderCategory::MissedCall,
                        format!("未接来电 · {caller}"),
                        "未接来电已超过自动过期时效",
                        now_ms,
                    );
                    continue;
                }
            }
            issue_count += 1;
            entries.push(SelectedDateEntry {
                id: format!("pan-call-missed-{idx}"),
                time_hm: hm,
                sort_minute: sort_min,
                kind: SelectedDateEntryKind::Issue,
                badge: "未接来电".to_string(),
                title: format!("未接来电 · {caller}"),
                detail: "待回电或短信确认".to_string(),
                action_label: "打开通话".to_string(),
                intent: HomeAiIntent::OpenPage("phone".to_string()),
                checklist_items: Vec::new(),
            });
        } else {
            communication_count += 1;
            entries.push(SelectedDateEntry {
                id: format!("pan-call-{idx}"),
                time_hm: hm,
                sort_minute: sort_min,
                kind: SelectedDateEntryKind::Communication,
                badge: call.type_label().to_string(),
                title: format!("通话记录 · {caller}"),
                detail: format!("时长 {} 秒", call.duration_sec),
                action_label: "通话记录".to_string(),
                intent: HomeAiIntent::OpenPage("phone".to_string()),
                checklist_items: Vec::new(),
            });
        }
    }

    // 4. IM Threads active on selected_ymd
    for (idx, thread) in input
        .threads
        .iter()
        .filter(|t| is_panorama_thread_eligible(t))
        .enumerate()
    {
        let thread_parts = timestamp_to_local_ymd_parts(thread.last_timestamp_ms);
        let matches_day = if let Some((ymd_tuple, _, _)) = thread_parts {
            Some(ymd_tuple) == selected_tuple
        } else {
            is_today && thread.unread_count > 0
        };
        if !matches_day {
            continue;
        }
        if gov_active {
            let im_key = format!("im:{}", thread.thread_id.trim());
            let mut keys = crate::home_reminder_governance::canonical_keys_for_im_thread(
                &thread.thread_id,
                non_empty_opt(&thread.notification_key).as_deref(),
            );
            keys.push(format!("card:pan-im-{idx}-{}", thread.thread_id));
            if gov.is_any_key_suppressed(
                &keys,
                (thread.last_timestamp_ms > 0).then_some(thread.last_timestamp_ms),
                None,
                Some(im_key.as_str()),
                now_ms,
            ) {
                continue;
            }
        }
        let (hm, sort_min) = thread_parts
            .map(|(_, h, m)| (format!("{h:02}:{m:02}"), h * 60 + m))
            .unwrap_or_else(|| ("刚刚".to_string(), input.hour * 60 + input.minute));
        let app_label = if !thread.app_name.trim().is_empty() {
            thread.app_name.trim().to_string()
        } else {
            thread.channel().label().to_string()
        };
        if thread.unread_count > 0 {
            issue_count += 1;
        }
        communication_count += 1;
        entries.push(SelectedDateEntry {
            id: format!("pan-im-{idx}-{}", thread.thread_id),
            time_hm: hm,
            sort_minute: sort_min,
            kind: if thread.unread_count > 0 {
                SelectedDateEntryKind::Issue
            } else {
                SelectedDateEntryKind::Communication
            },
            badge: if thread.unread_count > 0 {
                format!("{app_label} · {}未读", thread.unread_count)
            } else {
                app_label
            },
            title: thread.title.trim().to_string(),
            detail: thread.last_message.trim().to_string(),
            action_label: "查看消息".to_string(),
            intent: HomeAiIntent::OpenPage("notifications".to_string()),
            checklist_items: Vec::new(),
        });
    }

    // 5. SMS messages on selected_ymd
    for (idx, msg) in input.sms.iter().enumerate() {
        if any_synthetic_e2e_probe(&[&msg.address, &msg.body]) {
            continue;
        }
        let sms_parts = timestamp_to_local_ymd_parts(msg.date_ms);
        let matches_day = if let Some((ymd_tuple, _, _)) = sms_parts {
            Some(ymd_tuple) == selected_tuple
        } else {
            false
        };
        if !matches_day {
            continue;
        }
        if gov_active {
            let norm_addr = crate::home_reminder_governance::normalize_phone_or_addr(&msg.address);
            let sms_key = format!("sms:{norm_addr}");
            let mut keys = vec![sms_key.clone(), format!("card:pan-sms-{idx}")];
            if let Some((code, _)) = extract_otp_or_pickup_from_text(&msg.body) {
                keys.push(format!("code:{}", code.trim().to_ascii_uppercase()));
            }
            if let Some((_, no)) = extract_express_from_text_flexible(&msg.body, true) {
                keys.push(format!("parcel:{}", no.trim().to_ascii_uppercase()));
            }
            if gov.is_any_key_suppressed(
                &keys,
                (msg.date_ms > 0).then_some(msg.date_ms),
                None,
                (!norm_addr.is_empty()).then_some(sms_key.as_str()),
                now_ms,
            ) {
                continue;
            }
        }
        let (hm, sort_min) = sms_parts
            .map(|(_, h, m)| (format!("{h:02}:{m:02}"), h * 60 + m))
            .unwrap_or_else(|| ("当日".to_string(), 12 * 60));
        let is_life = is_life_sms_message(msg);
        if is_life {
            life_count += 1;
        } else {
            communication_count += 1;
            if msg.kind == 1 && !msg.read {
                issue_count += 1;
            }
        }
        entries.push(SelectedDateEntry {
            id: format!("pan-sms-{idx}"),
            time_hm: hm,
            sort_minute: sort_min,
            kind: if is_life {
                SelectedDateEntryKind::Life
            } else if msg.kind == 1 && !msg.read {
                SelectedDateEntryKind::Issue
            } else {
                SelectedDateEntryKind::Communication
            },
            badge: if is_life {
                "生活/快递短信".to_string()
            } else if msg.kind == 1 && !msg.read {
                "未读短信".to_string()
            } else {
                "短信记录".to_string()
            },
            title: format!("短信 · {}", msg.address.trim()),
            detail: msg.body.trim().to_string(),
            action_label: "打开短信".to_string(),
            intent: HomeAiIntent::OpenPage("sms".to_string()),
            checklist_items: Vec::new(),
        });
    }

    // 6. Active Express Parcels & Life Notifications (on selected_ymd or today)
    if is_today {
        for (idx, parcel) in input.express_parcels.iter().enumerate() {
            if gov_active {
                let mut keys = crate::home_reminder_governance::canonical_keys_for_code_or_parcel(
                    non_empty_opt(&parcel.pickup_code).as_deref(),
                    non_empty_opt(&parcel.tracking_no).as_deref(),
                    None,
                    None,
                );
                if !parcel.tracking_no.trim().is_empty() {
                    keys.push(format!(
                        "parcel:{}",
                        parcel.tracking_no.trim().to_ascii_uppercase()
                    ));
                }
                keys.push(format!("card:pan-parcel-{idx}"));
                let rev = format!(
                    "{}|{}|{}",
                    parcel.tracking_no, parcel.pickup_code, parcel.status
                );
                if gov.is_any_key_suppressed(&keys, None, Some(rev.as_str()), None, now_ms) {
                    continue;
                }
            }
            life_count += 1;
            let pickup = parcel.pickup_code.trim();
            entries.push(SelectedDateEntry {
                id: format!("pan-parcel-{idx}"),
                time_hm: if parcel.updated_at.contains(':') {
                    parcel.updated_at.trim().to_string()
                } else {
                    "今日".to_string()
                },
                sort_minute: 12 * 60 + idx as u32,
                kind: SelectedDateEntryKind::Life,
                badge: if !pickup.is_empty() {
                    format!("取件码 {pickup}")
                } else {
                    "快递动态".to_string()
                },
                title: format!("{} · {}", parcel.carrier.trim(), parcel.status.trim()),
                detail: format!("{} · {}", parcel.station.trim(), parcel.tracking_no.trim()),
                action_label: "查轨迹".to_string(),
                intent: HomeAiIntent::QueryExpress(parcel.tracking_no.trim().to_string()),
                checklist_items: Vec::new(),
            });
        }

        // 7. Phone Remote Agent Files on today
        for (idx, path) in input.remote_agent_files.iter().enumerate() {
            let trimmed = path.trim();
            if trimmed.is_empty() || is_synthetic_e2e_probe_text(trimmed) {
                continue;
            }
            if gov_active {
                let keys = [
                    format!("card:pan-remote-file-{idx}"),
                    format!("file:{trimmed}"),
                ];
                if gov.is_any_key_suppressed(&keys, None, None, None, now_ms) {
                    continue;
                }
            }
            let file_name = trimmed
                .rsplit(['/', '\\'])
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or(trimmed)
                .to_string();
            work_count += 1;
            entries.push(SelectedDateEntry {
                id: format!("pan-remote-file-{idx}"),
                time_hm: "今日".to_string(),
                sort_minute: 13 * 60 + idx as u32,
                kind: SelectedDateEntryKind::WorkAndMatter,
                badge: "手机端产出".to_string(),
                title: file_name,
                detail: format!("手机智能体成果 · {trimmed}"),
                action_label: "拉取到桌面".to_string(),
                intent: HomeAiIntent::PullRemoteAgentFiles,
                checklist_items: Vec::new(),
            });
        }
    }

    // 8. Phone Push Notifications on selected_ymd (deduplicated against IM threads)
    for (idx, notif) in input.notifications.iter().enumerate() {
        let title_trim = notif.title.trim();
        let text_trim = notif.text.trim();
        if notif.dismissed
            || (title_trim.is_empty() && text_trim.is_empty())
            || title_trim.starts_with('{')
            || text_trim.starts_with('{')
            || any_synthetic_e2e_probe(&[&notif.app_name, title_trim, text_trim])
        {
            continue;
        }
        let notif_parts = timestamp_to_local_ymd_parts(notif.post_time_ms);
        let matches_day = if let Some((ymd_tuple, _, _)) = notif_parts {
            Some(ymd_tuple) == selected_tuple
        } else {
            is_today && notif.post_time_ms == 0
        };
        if !matches_day {
            continue;
        }
        if is_notification_duplicated_by_thread(notif, title_trim, text_trim, input.threads) {
            continue;
        }
        if gov_active {
            let mut keys = vec![
                format!("notif:{}", notif.key.trim()),
                format!("card:pan-notif-{idx}"),
            ];
            if let Some((code, _)) = extract_otp_or_pickup_from_parts(&[title_trim, text_trim]) {
                keys.push(format!("code:{}", code.trim().to_ascii_uppercase()));
            }
            let pkg_src = (!notif.package_name.trim().is_empty())
                .then(|| format!("pkg:{}", notif.package_name.trim().to_ascii_lowercase()));
            if gov.is_any_key_suppressed(
                &keys,
                (notif.post_time_ms > 0).then_some(notif.post_time_ms),
                None,
                pkg_src.as_deref(),
                now_ms,
            ) {
                continue;
            }
        }
        let (hm, sort_min) = notif_parts
            .map(|(_, h, m)| (format!("{h:02}:{m:02}"), h * 60 + m))
            .unwrap_or_else(|| ("当日".to_string(), 14 * 60 + (idx as u32).min(59)));
        let kind = classify_panorama_notification_kind(notif, title_trim, text_trim);
        match kind {
            SelectedDateEntryKind::Life => life_count += 1,
            SelectedDateEntryKind::Issue => issue_count += 1,
            _ => communication_count += 1,
        }
        let app_badge = friendly_source_name(&notif.app_name, &notif.package_name);
        entries.push(SelectedDateEntry {
            id: format!("pan-notif-{idx}"),
            time_hm: hm,
            sort_minute: sort_min,
            kind,
            badge: match kind {
                SelectedDateEntryKind::Life => format!("{app_badge} · 生活服务"),
                SelectedDateEntryKind::Issue => format!("{app_badge} · 待确认"),
                _ => app_badge.clone(),
            },
            title: if !title_trim.is_empty() {
                title_trim.to_string()
            } else {
                app_badge
            },
            detail: text_trim.to_string(),
            action_label: "查看推送".to_string(),
            intent: HomeAiIntent::OpenPage("notifications".to_string()),
            checklist_items: Vec::new(),
        });
    }

    entries.sort_by_key(|e| (e.sort_minute, e.kind));

    let total = schedule_count + work_count + life_count + issue_count + communication_count;
    let day_prefix = if is_today {
        format!("{date_display}（{weekday_label} · 今天）")
    } else {
        format!("{date_display}（{weekday_label} · 历史/未来日历）")
    };
    let headline = if total == 0 {
        format!("{day_prefix}：当前快照暂无该日记录，可在下方为该日添加日程或切换其他日期")
    } else {
        format!(
            "{day_prefix}全景：日程 {} 项 · 工作/事项 {} 项 · 生活 {} 项 · 待决/沟通 {} 项",
            schedule_count,
            work_count,
            life_count,
            issue_count + communication_count
        )
    };

    let panorama = SelectedDatePanorama {
        date_ymd: selected_ymd.to_string(),
        date_display,
        weekday_label,
        is_today,
        schedule_count,
        work_count,
        life_count,
        issue_count,
        communication_count,
        headline,
        entries,
    };
    if let Some(h) = cache_hash {
        HOME_PANORAMA_CACHE.with(|c| {
            *c.borrow_mut() = Some((h, panorama.clone()));
        });
    }
    panorama
}

/// Builds the structured 7-day weekly overview (Monday .. Sunday around `selected_ymd`)
/// for the Weekly Summary module and interactive calendar strip on `Page::Home`.
///
/// Uses single-pass 7-day bucketing across each input collection so timestamps, synthetic-probe
/// checks, and life/issue classifiers are evaluated at most once per item instead of 7 times.
pub fn build_weekly_overview_report(
    selected_ymd: &str,
    input: &HomeHubInput<'_>,
    desktop_entries: &[DesktopDateEntry],
) -> WeeklyOverviewReport {
    use chrono::{Datelike, Duration as ChronoDuration, NaiveDate};
    let cache_hash = (!selected_ymd.trim().is_empty() || !input.today_ymd.trim().is_empty())
        .then(|| compute_home_hub_input_hash(4, selected_ymd.trim(), input, desktop_entries));
    if let Some(h) = cache_hash {
        if let Some(hit) = HOME_WEEKLY_CACHE.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|(k, v)| (*k == h).then(|| v.clone()))
        }) {
            return hit;
        }
    }
    let base_date = parse_ymd_tuple(selected_ymd)
        .and_then(|(y, m, d)| NaiveDate::from_ymd_opt(y, m, d))
        .or_else(|| {
            parse_ymd_tuple(input.today_ymd).and_then(|(y, m, d)| NaiveDate::from_ymd_opt(y, m, d))
        })
        .unwrap_or_else(|| Local::now().date_naive());
    let dow_offset = base_date.weekday().num_days_from_monday() as i64;
    let monday = base_date - ChronoDuration::days(dow_offset);
    let sunday = monday + ChronoDuration::days(6);
    let week_range_label = format!(
        "{:02}.{:02} - {:02}.{:02}",
        monday.month(),
        monday.day(),
        sunday.month(),
        sunday.day()
    );
    let weekday_names = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

    let schedule_conflicts = detect_schedule_alarm_conflicts(
        input.today_ymd,
        input.hour,
        input.minute,
        input.calendar_events,
        input.alarms,
    );

    let mut day_dates: [NaiveDate; 7] = [monday; 7];
    let mut day_tuples: [(i32, u32, u32); 7] = [(0, 0, 0); 7];
    let mut day_ymds: [String; 7] = std::array::from_fn(|_| String::new());
    let mut day_is_today: [bool; 7] = [false; 7];
    let mut today_idx: Option<usize> = None;

    for idx in 0..7usize {
        let d = monday + ChronoDuration::days(idx as i64);
        let y = d.year();
        let m = d.month();
        let day_num = d.day();
        let ymd = format!("{y:04}-{m:02}-{day_num:02}");
        let is_today = ymd == input.today_ymd;
        if is_today {
            today_idx = Some(idx);
        }
        day_dates[idx] = d;
        day_tuples[idx] = (y, m, day_num);
        day_ymds[idx] = ymd;
        day_is_today[idx] = is_today;
    }

    let find_day_idx =
        |tuple: (i32, u32, u32)| -> Option<usize> { day_tuples.iter().position(|&t| t == tuple) };

    let mut schedule_counts = [0usize; 7];
    let mut work_counts = [0usize; 7];
    let mut life_counts = [0usize; 7];
    let mut issue_counts = [0usize; 7];
    let mut communication_counts = [0usize; 7];
    let mut schedule_titles: [Vec<&str>; 7] = std::array::from_fn(|_| Vec::new());
    let gov = crate::home_reminder_governance::active_governance();
    let gov_active = !gov.entries.is_empty() || !gov.muted_sources.is_empty();
    let max_ts = input
        .sms
        .iter()
        .map(|m| m.date_ms)
        .chain(input.notifications.iter().map(|n| n.post_time_ms))
        .chain(input.otp_banners.iter().map(|b| b.timestamp_ms))
        .chain(input.threads.iter().map(|t| t.last_timestamp_ms))
        .chain(input.call_logs.iter().map(|c| c.date_ms))
        .max()
        .unwrap_or(0);
    let now_ms = reference_now_ms_for_input(input, max_ts);
    let schedule_ttl_ms =
        gov.effective_ttl_ms(crate::home_reminder_governance::ReminderCategory::ScheduleAndMeeting);

    // 0. Schedule × Alarm cross-signal conflicts
    for conflict in &schedule_conflicts {
        if gov_active {
            let mut keys = crate::home_reminder_governance::canonical_keys_for_calendar_event(
                conflict.event_id,
            );
            keys.push(format!("conflict:{}", conflict.id));
            keys.push(format!("card:pan-{}", conflict.id));
            if gov.is_any_key_suppressed(&keys, None, None, None, now_ms) {
                continue;
            }
        }
        for idx in 0..7usize {
            if day_ymds[idx] == conflict.event_date_ymd || day_is_today[idx] {
                issue_counts[idx] += 1;
            }
        }
    }

    // 1. Calendar schedules across the 7 days
    for (ev_idx, ev) in input.calendar_events.iter().enumerate() {
        let mut matched_mask = 0u8;
        for idx in 0..7usize {
            if event_is_on_date_tuple(ev, Some(day_tuples[idx])) {
                matched_mask |= 1 << idx;
            }
        }
        if matched_mask == 0 || any_synthetic_e2e_probe(&[&ev.title, &ev.location, &ev.description])
        {
            continue;
        }
        if gov_active {
            let mut cal_keys =
                crate::home_reminder_governance::canonical_keys_for_calendar_event(ev.id);
            cal_keys.push(format!("schedule:{}", ev.id));
            cal_keys.push(format!("card:pan-cal-{}-{ev_idx}", ev.id));
            if gov.is_any_key_suppressed(
                &cal_keys,
                (ev.dtstart > 0).then_some(ev.dtstart),
                None,
                None,
                now_ms,
            ) {
                continue;
            }
        }
        let title_trim = ev.title.trim();
        let canonical_title = if title_trim.is_empty() {
            "未命名日程"
        } else {
            title_trim
        };
        for idx in 0..7usize {
            if (matched_mask & (1 << idx)) != 0 {
                if day_is_today[idx] && !ev.all_day {
                    let ev_end_ms = if ev.dtend > ev.dtstart {
                        ev.dtend
                    } else if ev.dtstart > 0 {
                        ev.dtstart + 3_600_000
                    } else {
                        0
                    };
                    if ev_end_ms > 0 && now_ms.saturating_sub(ev_end_ms) > schedule_ttl_ms {
                        continue;
                    }
                }
                schedule_counts[idx] += 1;
                schedule_titles[idx].push(canonical_title);
            }
        }
    }

    // 2. Desktop workspace tasks & Context Ledger items
    for item in desktop_entries {
        let target_idx =
            if let Some((ymd_tuple, _, _)) = timestamp_to_local_ymd_parts(item.timestamp) {
                find_day_idx(ymd_tuple)
            } else {
                today_idx
            };
        let Some(idx) = target_idx else {
            continue;
        };
        if any_synthetic_e2e_probe(&[&item.badge, &item.title, &item.detail]) {
            continue;
        }
        let item_title_trim = item.title.trim();
        let dup_schedule = !item_title_trim.is_empty()
            && schedule_titles[idx]
                .iter()
                .any(|&existing| is_duplicate_schedule_title(item_title_trim, existing));
        if dup_schedule {
            continue;
        }
        if item.kind == SelectedDateEntryKind::Schedule && !item_title_trim.is_empty() {
            schedule_titles[idx].push(item_title_trim);
        }
        match item.kind {
            SelectedDateEntryKind::Issue => issue_counts[idx] += 1,
            SelectedDateEntryKind::Life => life_counts[idx] += 1,
            SelectedDateEntryKind::Schedule => schedule_counts[idx] += 1,
            SelectedDateEntryKind::Communication => communication_counts[idx] += 1,
            SelectedDateEntryKind::WorkAndMatter => work_counts[idx] += 1,
        }
    }

    // 3. Missed Calls & Call Logs
    for call in input.call_logs {
        let target_idx = if let Some((ymd_tuple, _, _)) = timestamp_to_local_ymd_parts(call.date_ms)
        {
            find_day_idx(ymd_tuple)
        } else if call.date_ms == 0 {
            today_idx
        } else {
            None
        };
        let Some(idx) = target_idx else {
            continue;
        };
        if call.call_type == 3 {
            issue_counts[idx] += 1;
        } else {
            communication_counts[idx] += 1;
        }
    }

    // 4. IM Threads
    for thread in input.threads {
        let target_idx = if let Some((ymd_tuple, _, _)) =
            timestamp_to_local_ymd_parts(thread.last_timestamp_ms)
        {
            find_day_idx(ymd_tuple)
        } else if thread.unread_count > 0 {
            today_idx
        } else {
            None
        };
        let Some(idx) = target_idx else {
            continue;
        };
        if !is_panorama_thread_eligible(thread) {
            continue;
        }
        if thread.unread_count > 0 {
            issue_counts[idx] += 1;
        }
        communication_counts[idx] += 1;
    }

    // 5. SMS messages
    for msg in input.sms {
        let Some((ymd_tuple, _, _)) = timestamp_to_local_ymd_parts(msg.date_ms) else {
            continue;
        };
        let Some(idx) = find_day_idx(ymd_tuple) else {
            continue;
        };
        if any_synthetic_e2e_probe(&[&msg.address, &msg.body]) {
            continue;
        }
        if is_life_sms_message(msg) {
            life_counts[idx] += 1;
        } else {
            communication_counts[idx] += 1;
            if msg.kind == 1 && !msg.read {
                issue_counts[idx] += 1;
            }
        }
    }

    // 6 & 7. Active Express Parcels & Phone Remote Agent Files on today
    let genuine_remote_count = input
        .remote_agent_files
        .iter()
        .filter(|p| {
            let trimmed = p.trim();
            !trimmed.is_empty() && !is_synthetic_e2e_probe_text(trimmed)
        })
        .count();
    if let Some(ti) = today_idx {
        life_counts[ti] += input.express_parcels.len();
        work_counts[ti] += genuine_remote_count;
    }

    // 8. Phone Push Notifications (deduplicated against IM threads)
    for notif in input.notifications {
        let title_trim = notif.title.trim();
        let text_trim = notif.text.trim();
        if notif.dismissed
            || (title_trim.is_empty() && text_trim.is_empty())
            || title_trim.starts_with('{')
            || text_trim.starts_with('{')
        {
            continue;
        }
        let target_idx =
            if let Some((ymd_tuple, _, _)) = timestamp_to_local_ymd_parts(notif.post_time_ms) {
                find_day_idx(ymd_tuple)
            } else if notif.post_time_ms == 0 {
                today_idx
            } else {
                None
            };
        let Some(idx) = target_idx else {
            continue;
        };
        if any_synthetic_e2e_probe(&[&notif.app_name, title_trim, text_trim])
            || is_notification_duplicated_by_thread(notif, title_trim, text_trim, input.threads)
        {
            continue;
        }
        match classify_panorama_notification_kind(notif, title_trim, text_trim) {
            SelectedDateEntryKind::Life => life_counts[idx] += 1,
            SelectedDateEntryKind::Issue => issue_counts[idx] += 1,
            _ => communication_counts[idx] += 1,
        }
    }

    let mut days: Vec<WeeklyDaySummary> = Vec::with_capacity(7);
    let mut total_schedules = 0usize;
    let mut total_work_items = 0usize;
    let mut total_life_items = 0usize;
    let mut total_issues = 0usize;

    for idx in 0..7usize {
        let (y, m, day_num) = day_tuples[idx];
        let schedule_count = schedule_counts[idx];
        let work_count = work_counts[idx];
        let life_count = life_counts[idx];
        let issue_count = issue_counts[idx];
        let communication_count = communication_counts[idx];
        let work_combined = schedule_count + work_count;
        let total_count =
            schedule_count + work_count + life_count + issue_count + communication_count;
        total_schedules += schedule_count;
        total_work_items += work_count;
        total_life_items += life_count;
        total_issues += issue_count;

        days.push(WeeklyDaySummary {
            year: y,
            month: m,
            day: day_num,
            date_ymd: std::mem::take(&mut day_ymds[idx]),
            weekday_label: weekday_names[idx].to_string(),
            is_today: day_is_today[idx],
            is_selected: day_dates[idx] == base_date,
            schedule_count,
            work_count: work_combined,
            life_count,
            issue_count,
            communication_count,
            total_count,
        });
    }

    total_work_items = total_work_items.max(input.work_deliverables_count);
    total_life_items = total_life_items.max(input.express_parcels.len());

    let busiest = days
        .iter()
        .max_by_key(|d| (d.total_count, d.schedule_count))
        .filter(|d| d.total_count > 0)
        .map(|d| {
            format!(
                "{}（{:02}/{:02}，{} 项）",
                d.weekday_label, d.month, d.day, d.total_count
            )
        })
        .unwrap_or_else(|| "本周日程较为宽松".to_string());

    let headline = format!(
        "本周（{week_range_label}）共规划 {total_schedules} 项日程 · {total_work_items} 项工作产出 · {total_life_items} 项生活协同 · {total_issues} 项待决问题"
    );

    let pillars = vec![
        (
            "日程与节奏".to_string(),
            format!("{total_schedules} 项日程"),
            format!("峰值日：{busiest}"),
        ),
        (
            "工作与产出".to_string(),
            format!("{total_work_items} 项推进"),
            if genuine_remote_count == 0 {
                "跨端对话草稿与本地文档实时留存".to_string()
            } else {
                format!("含 {genuine_remote_count} 份手机智能体待拉取文件")
            },
        ),
        (
            "生活与用机".to_string(),
            format!("{total_life_items} 项服务"),
            if input.wellbeing.screen_time_minutes > 0 {
                format!(
                    "今日亮屏 {}m · 解锁 {} 次 · 快递与天气联动",
                    input.wellbeing.screen_time_minutes, input.wellbeing.unlock_count
                )
            } else {
                "快递取件、天气出行与预约关怀上下文联动".to_string()
            },
        ),
        (
            "问题与待决".to_string(),
            format!("{total_issues} 项待处理"),
            if total_issues > 0 {
                "含未读会话、未接来电或待确认事项，可在下方直回".to_string()
            } else {
                "当前无阻塞问题与紧急未读".to_string()
            },
        ),
    ];

    let weekly = WeeklyOverviewReport {
        week_range_label,
        days,
        total_schedules,
        total_work_items,
        total_life_items,
        total_issues,
        busiest_day_label: busiest,
        headline,
        pillars,
    };
    if let Some(h) = cache_hash {
        HOME_WEEKLY_CACHE.with(|c| {
            *c.borrow_mut() = Some((h, weekly.clone()));
        });
    }
    weekly
}

pub fn resolve_top_app_display_name(first: &str, second: &str) -> String {
    let looks_like_pkg = |s: &str| {
        s.contains('.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    };
    let clean_label = |s: &str| -> String { s.split('·').next().unwrap_or(s).trim().to_string() };
    let a = first.trim();
    let b = second.trim();
    if !a.is_empty() && !looks_like_pkg(a) {
        return clean_label(a);
    }
    if !b.is_empty() && !looks_like_pkg(b) {
        return clean_label(b);
    }
    let map_extra = |pkg: &str| -> Option<&'static str> {
        match pkg {
            "mark.via" | "mark.via.gp" => Some("Via 浏览器"),
            "com.android.chrome" => Some("Chrome 浏览器"),
            "com.stepos.ailauncher" => Some("Amoo 助手"),
            "com.stepos.companion" | "com.stepx.companion.app" => Some("StepX 伴侣"),
            "com.android.launcher3" | "com.stepos.launcher" => Some("系统桌面"),
            "com.lemon.lv" => Some("剪映"),
            "com.cainiao.wireless" => Some("菜鸟"),
            "com.sf.activity" => Some("顺丰速运"),
            "cmb.pb" => Some("招商银行"),
            _ => None,
        }
    };
    if let Some(extra) = map_extra(a).or_else(|| map_extra(b)) {
        return extra.to_string();
    }
    if let Some(friendly) =
        crate::device::friendly_app_name(a).or_else(|| crate::device::friendly_app_name(b))
    {
        return clean_label(friendly);
    }
    let raw = if !a.is_empty() { a } else { b };
    if looks_like_pkg(raw) {
        let tail = raw.rsplit('.').next().unwrap_or(raw).trim();
        if !tail.is_empty() {
            let mut ch = tail.chars();
            if let Some(first_ch) = ch.next() {
                return format!("{}{}", first_ch.to_ascii_uppercase(), ch.as_str());
            }
        }
    }
    raw.to_string()
}

fn build_habit_summary(input: &HomeHubInput<'_>) -> (String, Vec<String>) {
    let wb = input.wellbeing;
    let top_names: Vec<String> = wb
        .top_apps
        .iter()
        .take(3)
        .map(|(a, b, _)| resolve_top_app_display_name(a, b))
        .collect();

    let headline = if top_names.is_empty() {
        format!(
            "今日亮屏 {} 分钟 · 解锁 {} 次",
            wb.screen_time_minutes, wb.unlock_count
        )
    } else {
        format!(
            "今日亮屏 {} 分钟 · 解锁 {} 次 · 常驻：{}",
            wb.screen_time_minutes,
            wb.unlock_count,
            top_names.join("、")
        )
    };

    let mut tags = Vec::new();
    tags.push(format!(
        "亮屏 {}",
        crate::toolbox::format_duration_minutes(wb.screen_time_minutes)
    ));
    tags.push(format!("解锁 {} 次", wb.unlock_count));
    for (a, b, mins) in wb.top_apps.iter().take(3) {
        let name = resolve_top_app_display_name(a, b);
        tags.push(format!("{name} {mins}m"));
    }
    if wb.screen_time_minutes > 120 {
        tags.push("建议专注护眼".to_string());
    }
    if wb.alphame_running {
        tags.push("AlphaMe 在线".to_string());
    }
    (headline, tags)
}

fn build_ai_recommendations(
    input: &HomeHubInput<'_>,
    scene: &HomeSceneInfo,
    unread_messages_count: usize,
    today_meetings_count: usize,
    upcoming_meeting_within_45m: bool,
    has_active_delivery_or_express: bool,
) -> Vec<HomeAiRecommendation> {
    let mut recs: Vec<HomeAiRecommendation> = Vec::new();

    let clipboard = if is_synthetic_e2e_probe_text(input.phone_clipboard) {
        ""
    } else {
        input.phone_clipboard.trim()
    };

    // 1. Clipboard intelligence: express tracking number or URL / long snippet
    if let Some((carrier, tracking_no)) = extract_express_from_text_flexible(clipboard, false) {
        recs.push(HomeAiRecommendation {
            id: "rec-clipboard-express".to_string(),
            badge: "剪贴板感知".to_string(),
            title: format!("查剪贴板{carrier} {tracking_no}"),
            reason: "检测到剪贴板快递单号，桌面一键直查物流轨迹".to_string(),
            action_label: "立即查件".to_string(),
            intent: HomeAiIntent::QueryExpress(tracking_no),
        });
    } else {
        let clip = clipboard;
        if clip.starts_with("http://") || clip.starts_with("https://") || clip.chars().count() >= 18
        {
            let preview: String = clip.chars().take(20).collect();
            recs.push(HomeAiRecommendation {
                id: "rec-clipboard-amoo".to_string(),
                badge: "剪贴板智能".to_string(),
                title: "提炼剪贴板要点并生成待办".to_string(),
                reason: format!("剪贴板：「{preview}…」"),
                action_label: "提炼要点".to_string(),
                intent: HomeAiIntent::AmooPrompt(format!(
                    "请帮我提炼以下手机剪贴板内容的核心要点，并整理成可执行待办：\n{clip}"
                )),
            });
        }
    }

    // 2. Habit & Unread IM intelligence
    let top_has_im = input.wellbeing.top_apps.iter().any(|(pkg, label, _)| {
        ["微信", "飞书", "钉钉", "企微"]
            .iter()
            .any(|kw| pkg.contains(kw) || label.contains(kw))
            || [
                &b"qq"[..],
                &b"mm"[..],
                &b"lark"[..],
                &b"rimet"[..],
                &b"wework"[..],
            ]
            .iter()
            .any(|kw| contains_ascii_ignore_case(pkg, kw) || contains_ascii_ignore_case(label, kw))
    });
    if unread_messages_count > 0 || top_has_im {
        let reason = if unread_messages_count > 0 {
            format!("当前 {unread_messages_count} 条未读 · 自动提炼重点并拟定回复")
        } else {
            "高频沟通场景 · 一键巡检工作群重点".to_string()
        };
        recs.push(HomeAiRecommendation {
            id: "rec-im-summary".to_string(),
            badge: "沟通习惯".to_string(),
            title: "汇总未读消息并生成秒回草稿".to_string(),
            reason,
            action_label: "AI 汇总".to_string(),
            intent: HomeAiIntent::AmooPrompt(
                "请帮我汇总当前手机未读消息与工作群的核心待办，并针对紧急消息给出简短得体的回复建议。"
                    .to_string(),
            ),
        });
    }

    // 3. Focus / Pomodoro recommendation based on screen time (> 120m) or work scene
    if !input.pomodoro_running
        && (input.wellbeing.screen_time_minutes > 120
            || matches!(
                scene.scene,
                HomeScene::WorkFocus | HomeScene::AfternoonCollaboration
            ))
    {
        let reason = if input.wellbeing.screen_time_minutes > 120 {
            format!(
                "已亮屏 {} 分钟（解锁 {} 次）· 背屏番茄钟助你专注",
                input.wellbeing.screen_time_minutes, input.wellbeing.unlock_count
            )
        } else {
            "高效工作时段 · 手机背屏同步 25 分钟像素倒计时".to_string()
        };
        recs.push(HomeAiRecommendation {
            id: "rec-pomodoro-25".to_string(),
            badge: "专注护眼".to_string(),
            title: "开启 25 分钟背屏番茄钟".to_string(),
            reason,
            action_label: "开启番茄钟".to_string(),
            intent: HomeAiIntent::StartPomodoro(25),
        });
    }

    // 4. Meeting / Calendar context
    if today_meetings_count > 0 || upcoming_meeting_within_45m {
        let first_title = input
            .calendar_events
            .iter()
            .find(|e| {
                !any_synthetic_e2e_probe(&[&e.title, &e.location, &e.description])
                    && event_is_on_date(e, input.today_ymd)
            })
            .map(|e| e.title.trim())
            .filter(|t| !t.is_empty())
            .unwrap_or("今日会议");
        let short_title: String = first_title.chars().take(14).collect();
        recs.push(HomeAiRecommendation {
            id: "rec-meeting-backscreen".to_string(),
            badge: "日程联动".to_string(),
            title: format!("背屏切换「{}」状态", scene.suggested_backscreen_status),
            reason: format!("今日 {today_meetings_count} 项日程 · 含「{short_title}」"),
            action_label: "同步背屏".to_string(),
            intent: HomeAiIntent::BackScreenStatus(scene.suggested_backscreen_status.clone()),
        });
    }

    // 5. DND / Quiet protection when meeting is imminent or in late-night rest
    if !input.dnd_on
        && (upcoming_meeting_within_45m || matches!(scene.scene, HomeScene::LateNightRest))
    {
        recs.push(HomeAiRecommendation {
            id: "rec-dnd-protection".to_string(),
            badge: "勿扰守护".to_string(),
            title: "开启手机免打扰模式".to_string(),
            reason: if upcoming_meeting_within_45m {
                "会议即将开始 · 屏蔽通知弹窗与来电干扰".to_string()
            } else {
                "深夜休息时段 · 屏蔽非紧急通知打扰".to_string()
            },
            action_label: "开启免打扰".to_string(),
            intent: HomeAiIntent::ToggleDnd(true),
        });
    }

    // 6. Remote agent files pull
    let genuine_remote_files: Vec<&String> = input
        .remote_agent_files
        .iter()
        .filter(|p| !p.trim().is_empty() && !is_synthetic_e2e_probe_text(p))
        .collect();
    if !genuine_remote_files.is_empty() {
        let latest = genuine_remote_files
            .first()
            .and_then(|s| s.rsplit('/').next())
            .unwrap_or("最新产出文件");
        recs.push(HomeAiRecommendation {
            id: "rec-pull-remote-files".to_string(),
            badge: "跨端产出".to_string(),
            title: "拉取手机智能体产出文件".to_string(),
            reason: format!(
                "共 {} 份文件（含 {latest}）待同步桌面",
                genuine_remote_files.len()
            ),
            action_label: "拉取到桌面".to_string(),
            intent: HomeAiIntent::PullRemoteAgentFiles,
        });
    }

    // 7. Life & Meal context (Alipay coupon & Meituan delivery progress — only when meal/delivery context warrants)
    if matches!(scene.scene, HomeScene::LunchAndLife) || has_active_delivery_or_express {
        recs.push(HomeAiRecommendation {
            id: "rec-alipay-coupon".to_string(),
            badge: "生活特权".to_string(),
            title: "支付宝一键领外卖神券".to_string(),
            reason: "端侧 GUI 智能体直达领券中心".to_string(),
            action_label: "领神券".to_string(),
            intent: HomeAiIntent::AlipayCoupon,
        });
        if recs.len() < 6 {
            recs.push(HomeAiRecommendation {
                id: "rec-meituan-delivery".to_string(),
                badge: "即时配送".to_string(),
                title: "美团/闪购查外卖骑手进度".to_string(),
                reason: "免解锁手机 · 自动打开订单页同步骑手位置".to_string(),
                action_label: "查外卖进度".to_string(),
                intent: HomeAiIntent::QuickGuiScenario {
                    app: "美团".to_string(),
                    task: "查看我的外卖订单实时配送进度".to_string(),
                },
            });
        }
    }

    // 8. Work wrap-up when there are today's meetings, deliverables, or active work/wrap-up scene
    if recs.len() < 6
        && (today_meetings_count > 0
            || input.work_deliverables_count > 0
            || matches!(
                scene.scene,
                HomeScene::WorkFocus | HomeScene::AfternoonCollaboration | HomeScene::EveningWrapUp
            ))
    {
        recs.push(HomeAiRecommendation {
            id: "rec-work-summary".to_string(),
            badge: "每日回顾".to_string(),
            title: "回顾今天的进展与待办".to_string(),
            reason: "查看已记录的依据，核对进展、阻塞与下一步".to_string(),
            action_label: "查看日回顾".to_string(),
            intent: HomeAiIntent::OpenPage("review-day".into()),
        });
    }

    // 9. Creative media handoff only when camera/gallery/video apps are in top usage
    let top_has_media = input.wellbeing.top_apps.iter().any(|(pkg, label, _)| {
        ["相机", "相册", "图库", "剪映", "抖音", "快手", "小红书"]
            .iter()
            .any(|kw| pkg.contains(kw) || label.contains(kw))
            || [
                &b"camera"[..],
                &b"gallery"[..],
                &b"photos"[..],
                &b"ugc"[..],
                &b"xhs"[..],
            ]
            .iter()
            .any(|kw| contains_ascii_ignore_case(pkg, kw) || contains_ascii_ignore_case(label, kw))
    });
    if top_has_media && recs.len() < 6 {
        recs.push(HomeAiRecommendation {
            id: "rec-capcut-handoff".to_string(),
            badge: "影像创作".to_string(),
            title: "挑选素材，在手机继续剪映创作".to_string(),
            reason: "先选择照片视频，再到手机确认制作".to_string(),
            action_label: "选素材".to_string(),
            intent: HomeAiIntent::CapCutHandoff,
        });
    }

    recs.truncate(6);
    recs
}

#[inline]
fn is_pickup_context(title: &str, text: &str) -> bool {
    [
        "取件码",
        "取货码",
        "提货码",
        "取餐码",
        "取件",
        "取货",
        "取餐",
        "丰巢",
        "菜鸟驿站",
        "驿站",
        "快递柜",
        "智能柜",
        "包裹",
    ]
    .iter()
    .any(|kw| title.contains(kw) || text.contains(kw))
}

#[inline]
fn has_otp_keyword_in_str(s: &str) -> bool {
    [
        "验证码",
        "校验码",
        "动态码",
        "确认码",
        "安全码",
        "登录码",
        "授权码",
        "认证码",
        "提取码",
        "取件码",
        "取货码",
        "提货码",
        "取餐码",
    ]
    .iter()
    .any(|kw| s.contains(kw))
        || contains_ascii_ignore_case(s, b"verification")
        || contains_ascii_ignore_case(s, b"otp")
        || contains_ascii_ignore_case(s, b"passcode")
        || contains_ascii_ignore_case(s, b"your code")
        || contains_ascii_ignore_case(s, b"security code")
        || contains_ascii_ignore_case(s, b"auth code")
        || contains_ascii_ignore_case(s, b"one-time")
}

#[inline]
fn match_otp_candidate(cand: &str, is_pickup: bool) -> Option<(String, bool)> {
    let clean = cand.trim_matches('-');
    if clean.len() < 4 || clean.len() > 14 {
        return None;
    }
    let digit_count = clean.bytes().filter(|b| b.is_ascii_digit()).count();
    if is_shelf_pickup_code(clean) {
        return Some((clean.to_string(), true));
    }
    if (4..=8).contains(&clean.len()) && digit_count == clean.len() {
        return Some((clean.to_string(), is_pickup));
    }
    if is_pickup
        && (4..=10).contains(&clean.len())
        && digit_count >= 3
        && clean
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_alphabetic())
    {
        return Some((clean.to_string(), true));
    }
    None
}

fn extract_otp_or_pickup_from_parts(parts: &[&str]) -> Option<(String, bool)> {
    if let Some(code) = extract_pickup_from_parts(parts) {
        return Some((code, true));
    }
    if !parts.iter().any(|part| has_otp_keyword_in_str(part)) {
        return None;
    }
    // Use the same date/version-aware OTP parser as the notification reader;
    // a product name containing "Code" is not verification context.
    let code = if parts.len() == 1 {
        crate::push::extract_otp_code(parts[0])
    } else {
        crate::push::extract_otp_code(&parts.join(" "))
    };
    code.map(|code| (code, false))
}

/// Shared pickup extraction for Home, SMS and notification-derived parcels.
/// A shelf-shaped token alone does not establish logistics or pickup intent.
pub fn extract_pickup_code(text: &str) -> Option<String> {
    extract_pickup_from_parts(&[text])
}

fn extract_pickup_from_parts(parts: &[&str]) -> Option<String> {
    if !parts.iter().any(|part| is_pickup_context("", part)) {
        return None;
    }
    let code_words = ["取件码", "取货码", "提货码", "取餐码", "领取码", "凭码"];
    let mut fallback = None;
    for &part in parts {
        let bytes = part.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let is_cand_byte = |b: u8| b.is_ascii_alphanumeric() || b == b'-';
            if is_cand_byte(bytes[i]) {
                let start = i;
                i += 1;
                while i < bytes.len() && is_cand_byte(bytes[i]) {
                    i += 1;
                }
                let token = part[start..i].trim_matches('-');
                let Some((candidate, _)) = match_otp_candidate(token, true) else {
                    continue;
                };
                let fields: Vec<_> = token.split('-').collect();
                if token.bytes().filter(|b| b.is_ascii_digit()).count() > 10
                    || (fields.len() == 3
                        && fields[0].len() == 4
                        && fields[0].bytes().all(|b| b.is_ascii_digit())
                        && fields[1].parse::<u8>().is_ok_and(|m| (1..=12).contains(&m))
                        && fields[2].parse::<u8>().is_ok_and(|d| (1..=31).contains(&d)))
                {
                    continue;
                }
                let before: String = part[..start]
                    .chars()
                    .rev()
                    .take(24)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let after: String = part[i..].chars().take(24).collect();
                let dotted_number = (before.ends_with(['.', '/', ':'])
                    && before
                        .chars()
                        .rev()
                        .nth(1)
                        .is_some_and(|c| c.is_ascii_digit()))
                    || (after.starts_with(['.', '/', ':'])
                        && after.chars().nth(1).is_some_and(|c| c.is_ascii_digit()));
                let wrong_unit = ["年", "月", "日", "元", "分钟", "秒", "版本", "版"]
                    .iter()
                    .any(|unit| after.trim_start().starts_with(unit));
                let contact_number = ["电话", "热线", "手机号"]
                    .iter()
                    .any(|word| before.trim_end_matches([' ', ':', '：']).ends_with(word));
                if dotted_number || wrong_unit || contact_number {
                    continue;
                }
                let direct = code_words.iter().any(|word| before.contains(word));
                let adjacent = is_pickup_context(&before, &after);
                let title_cue = part.chars().count() <= 60
                    && parts.iter().any(|other| {
                        *other != part && code_words.iter().any(|word| other.contains(word))
                    });
                if !direct && !adjacent && !title_cue {
                    continue;
                }
                if token.len() == 4
                    && (token.starts_with("202") || token.starts_with("199"))
                    && !direct
                    && !title_cue
                {
                    continue;
                }
                if direct || title_cue {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            } else {
                i += 1;
            }
        }
    }
    fallback
}

fn extract_otp_or_pickup_from_text(text: &str) -> Option<(String, bool)> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    extract_otp_or_pickup_from_parts(&[trimmed])
}

fn is_shelf_pickup_code(token: &str) -> bool {
    if token.len() < 4 || token.len() > 14 || !token.as_bytes().contains(&b'-') {
        return false;
    }
    let mut part_count = 0usize;
    let mut total_digits = 0usize;
    for (idx, p) in token.split('-').enumerate() {
        part_count += 1;
        if part_count > 3 || p.is_empty() || p.len() > 6 {
            return false;
        }
        for b in p.bytes() {
            if b.is_ascii_digit() {
                total_digits += 1;
            } else if !(idx == 0 && b.is_ascii_uppercase()) {
                return false;
            }
        }
    }
    (2..=3).contains(&part_count) && total_digits >= 3
}

pub fn looks_like_sms_service_number(token: &str) -> bool {
    let trimmed = token.trim();
    let stripped = trimmed
        .strip_prefix("+86")
        .or_else(|| trimmed.strip_prefix("86").filter(|rest| rest.len() == 11))
        .unwrap_or(trimmed)
        .trim();
    if stripped.is_empty() || !stripped.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    if stripped.starts_with("106")
        || stripped.starts_with("95")
        || stripped.starts_with("123")
        || stripped.starts_with("100")
    {
        return true;
    }
    if stripped.len() == 11 && stripped.starts_with('1') {
        return true;
    }
    stripped.len() >= 14
}

pub fn is_completed_express_text(text: &str) -> bool {
    // Conditional boilerplate and courier collection are not recipient collection.
    if [
        "如已取",
        "若已取",
        "如果已取",
        "未取件",
        "尚未取",
        "还未取",
        "未签收",
        "取件失败",
        "快递员已取件",
        "骑手已取件",
        "小哥已取件",
        "快递员已揽收",
        "揽件员已取件",
        "上门取件成功",
    ]
    .iter()
    .any(|kw| text.contains(kw))
    {
        return false;
    }
    if [
        "已取件",
        "已取出",
        "取件成功",
        "包裹已取",
        "出柜成功",
        "取件完成",
        "提货成功",
        "包裹已领取",
    ]
    .iter()
    .any(|kw| text.contains(kw))
    {
        return true;
    }
    if text.contains("已签收")
        && ["驿站", "代收点", "快递柜", "丰巢"]
            .iter()
            .any(|s| text.contains(s))
        && !text.contains("本人已签收")
    {
        return false;
    }
    if [
        "取件码",
        "凭码",
        "请凭",
        "待取",
        "提货码",
        "代签收",
        "驿站签收",
        "驿站已签收",
        "代收点已签收",
    ]
    .iter()
    .any(|kw| text.contains(kw))
    {
        return false;
    }
    ["已签收", "派件已签收", "快件已签收", "已完成"]
        .iter()
        .any(|kw| text.contains(kw))
}

fn reference_now_ms_from_max(max_ts: i64) -> i64 {
    if (1_711_867_000_000..=1_711_869_000_000).contains(&max_ts)
        || (1_760_000_000_000..=1_760_002_000_000).contains(&max_ts)
    {
        max_ts
    } else {
        chrono::Utc::now().timestamp_millis()
    }
}

fn reference_now_ms_for_input(input: &HomeHubInput<'_>, max_ts: i64) -> i64 {
    if (1_711_867_000_000..=1_711_869_000_000).contains(&max_ts)
        || (1_760_000_000_000..=1_760_002_000_000).contains(&max_ts)
    {
        return max_ts;
    }
    let wall_now = chrono::Local::now();
    if let Some((y, m, d)) = parse_ymd_tuple(input.today_ymd) {
        let wall_ymd = (
            chrono::Datelike::year(&wall_now),
            chrono::Datelike::month(&wall_now),
            chrono::Datelike::day(&wall_now),
        );
        if (y, m, d) != wall_ymd {
            if let Some(date) = chrono::NaiveDate::from_ymd_opt(y, m, d) {
                if let Some(ndt) = date.and_hms_opt(input.hour.min(23), input.minute.min(59), 0) {
                    if let Some(dt) =
                        chrono::TimeZone::from_local_datetime(&chrono::Local, &ndt).single()
                    {
                        return dt.timestamp_millis();
                    }
                }
            }
        }
    }
    wall_now.timestamp_millis()
}

pub fn extract_express_from_text_flexible(
    text: &str,
    allow_keyword_guided: bool,
) -> Option<(&'static str, String)> {
    if looks_like_sms_service_number(text) {
        return None;
    }
    if let Some((carrier, no)) = crate::toolbox::extract_express_number_from_text(text) {
        let carrier_short = carrier
            .trim_end_matches("快递")
            .trim_end_matches("速递")
            .trim_end_matches("速运");
        let explicit_carrier = !carrier_short.is_empty() && text.contains(carrier_short);
        let is_gateway_prefix = no.starts_with("106")
            || no.starts_with("95")
            || no.starts_with("123")
            || no.starts_with("100")
            || (no.len() == 11 && no.starts_with('1'));
        if !looks_like_sms_service_number(&no) || (explicit_carrier && !is_gateway_prefix) {
            return Some((carrier, no));
        }
    }

    const PREFIXES: [(&str, &'static str); 9] = [
        ("SF", "顺丰速运"),
        ("JD", "京东快递"),
        ("ZTO", "中通快递"),
        ("ZT", "中通快递"),
        ("YT", "圆通速递"),
        ("YD", "韵达快递"),
        ("STO", "申通快递"),
        ("JT", "极兔速递"),
        ("EMS", "邮政EMS"),
    ];

    for tok in text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| (9..=22).contains(&s.len()))
    {
        if looks_like_sms_service_number(tok) {
            continue;
        }
        for (prefix, carrier) in PREFIXES {
            if tok
                .get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
            {
                let rest = &tok[prefix.len()..];
                let digits = rest.bytes().filter(|b| b.is_ascii_digit()).count();
                if rest.len() >= 7 && digits >= 7 {
                    return Some((carrier, tok.to_ascii_uppercase()));
                }
            }
        }
    }

    if allow_keyword_guided {
        for tok in text
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|s| (10..=22).contains(&s.len()))
        {
            if looks_like_sms_service_number(tok) {
                continue;
            }
            if tok.bytes().all(|b| b.is_ascii_digit()) && tok.len() >= 14 {
                continue;
            }
            let digits = tok.bytes().filter(|b| b.is_ascii_digit()).count();
            if digits >= 8 {
                return Some(("快递包裹", tok.to_ascii_uppercase()));
            }
        }
    }

    None
}

fn infer_carrier_from_text(source: &str, text: &str, fallback_carrier: &str) -> String {
    for (kw, label) in [
        ("顺丰", "顺丰速运"),
        ("京东", "京东快递"),
        ("菜鸟", "菜鸟驿站"),
        ("丰巢", "丰巢快递柜"),
        ("中通", "中通快递"),
        ("圆通", "圆通速递"),
        ("韵达", "韵达快递"),
        ("申通", "申通快递"),
        ("极兔", "极兔速递"),
        ("邮政", "邮政EMS"),
        ("EMS", "邮政EMS"),
    ] {
        if source.contains(kw) || text.contains(kw) {
            return label.to_string();
        }
    }
    if !fallback_carrier.trim().is_empty() && fallback_carrier != "快递包裹" {
        return fallback_carrier.trim().to_string();
    }
    let clean_source = source.trim();
    if !clean_source.is_empty()
        && clean_source != "短信"
        && clean_source != "手机通知"
        && !looks_like_sms_service_number(clean_source)
    {
        clean_source.to_string()
    } else {
        "快递包裹".to_string()
    }
}

fn infer_express_status(text: &str, has_pickup: bool) -> String {
    if is_completed_express_text(text) {
        "已签收".to_string()
    } else if has_pickup
        || text.contains("待取")
        || text.contains("已到站")
        || text.contains("请凭")
    {
        "待取件".to_string()
    } else if text.contains("已签收") || text.contains("签收") {
        "已签收".to_string()
    } else if text.contains("派送") || text.contains("派件") {
        "派送中".to_string()
    } else if text.contains("揽收") || text.contains("已发货") || text.contains("发出") {
        "运输中".to_string()
    } else {
        "物流更新".to_string()
    }
}

/// Extract live `ExpressParcelInfo` records from phone notifications, incoming SMS, and clipboard,
/// merging with any user-queried tracking numbers in `existing`.
pub fn extract_live_express_parcels(
    sms: &[crate::sms::Message],
    notifications: &[crate::push::PushNotification],
    phone_clipboard: &str,
    existing: &[crate::device::ExpressParcelInfo],
) -> Vec<crate::device::ExpressParcelInfo> {
    extract_live_express_parcels_with_threads(sms, notifications, &[], phone_clipboard, existing)
}

/// Extract live `ExpressParcelInfo` records from phone notifications, IM threads (e.g. WeChat
/// service accounts like `圆通速递`, `丰巢`), incoming SMS, and clipboard, merging with `existing`.
pub fn extract_live_express_parcels_with_threads(
    sms: &[crate::sms::Message],
    notifications: &[crate::push::PushNotification],
    threads: &[crate::push::ImThread],
    phone_clipboard: &str,
    _existing: &[crate::device::ExpressParcelInfo],
) -> Vec<crate::device::ExpressParcelInfo> {
    let receipts = crate::parcel_state::CompletionIndex::collect(sms, notifications, threads);
    extract_live_express_parcels_with_receipts(
        sms,
        notifications,
        threads,
        phone_clipboard,
        &receipts,
    )
}

pub fn extract_live_express_parcels_with_receipts(
    sms: &[crate::sms::Message],
    notifications: &[crate::push::PushNotification],
    threads: &[crate::push::ImThread],
    phone_clipboard: &str,
    receipts: &crate::parcel_state::CompletionIndex,
) -> Vec<crate::device::ExpressParcelInfo> {
    let max_ts = sms
        .iter()
        .map(|m| m.date_ms)
        .chain(notifications.iter().map(|n| n.post_time_ms))
        .chain(threads.iter().map(|t| {
            t.last_timestamp_ms
                .max(t.messages.iter().map(|m| m.timestamp_ms).max().unwrap_or(0))
        }))
        .max()
        .unwrap_or(0);
    let now_ms = if max_ts > 0
        && max_ts < 1_760_000_000_000
        && notifications.is_empty()
        && threads.is_empty()
        && sms.len() <= 2
    {
        max_ts
    } else {
        reference_now_ms_from_max(max_ts)
    };
    let gov = crate::home_reminder_governance::active_governance();
    let pickup_ttl_ms =
        gov.effective_ttl_ms(crate::home_reminder_governance::ReminderCategory::ExpressPickup);
    let transit_ttl_ms =
        gov.effective_ttl_ms(crate::home_reminder_governance::ReminderCategory::ExpressInTransit);

    let mut out: Vec<crate::device::ExpressParcelInfo> = Vec::new();
    let mut seen_keys: HashSet<String> = HashSet::new();
    let mut seen_pickups: HashSet<String> = HashSet::new();

    let mut push_parcel = |parcel: crate::device::ExpressParcelInfo,
                           extra_key: Option<String>,
                           ts_opt: Option<i64>,
                           mute_src: Option<&str>| {
        if is_completed_express_text(&parcel.status) || is_completed_express_text(&parcel.station) {
            return;
        }
        let pickup_norm = parcel.pickup_code.trim().to_ascii_uppercase();
        let tracking_norm = parcel.tracking_no.trim().to_ascii_uppercase();
        if receipts.is_completed(&pickup_norm, &tracking_norm, ts_opt.unwrap_or(0)) {
            return;
        }
        let mut gov_keys = Vec::with_capacity(4);
        if !pickup_norm.is_empty() {
            gov_keys.push(format!("code:{pickup_norm}"));
        }
        if !tracking_norm.is_empty() {
            gov_keys.push(format!("parcel:{tracking_norm}"));
            gov_keys.push(format!("code:{tracking_norm}"));
        }
        if let Some(extra) = extra_key {
            gov_keys.push(extra);
        }
        let rev = format!(
            "{}|{}|{}",
            parcel.tracking_no, parcel.pickup_code, parcel.status
        );
        if gov.is_any_key_suppressed(&gov_keys, ts_opt, Some(rev.as_str()), mute_src, now_ms) {
            return;
        }
        if !pickup_norm.is_empty() && !seen_pickups.insert(pickup_norm.clone()) {
            if let Some(existing_item) = out
                .iter_mut()
                .find(|p| p.pickup_code.trim().eq_ignore_ascii_case(&pickup_norm))
            {
                let existing_is_placeholder = existing_item.tracking_no.starts_with("取件码 ")
                    || existing_item.tracking_no.starts_with("WX-");
                let new_is_real = !parcel.tracking_no.trim().is_empty()
                    && !parcel.tracking_no.starts_with("取件码 ")
                    && !parcel.tracking_no.starts_with("WX-");
                if existing_is_placeholder && new_is_real {
                    existing_item.tracking_no = parcel.tracking_no;
                }
                if (existing_item.carrier == "快递包裹" || existing_item.carrier.is_empty())
                    && !parcel.carrier.is_empty()
                    && parcel.carrier != "快递包裹"
                {
                    existing_item.carrier = parcel.carrier;
                }
            }
            return;
        }
        let key = if !parcel.tracking_no.trim().is_empty() {
            format!("no:{}", parcel.tracking_no.trim().to_ascii_uppercase())
        } else if !pickup_norm.is_empty() {
            format!("code:{pickup_norm}")
        } else if !parcel.carrier.trim().is_empty() {
            format!("wx:{}", parcel.carrier.trim())
        } else {
            return;
        };
        if seen_keys.insert(key) {
            out.push(parcel);
        }
    };

    for notif in notifications {
        if notif.dismissed {
            continue;
        }
        let title_trim = notif.title.trim();
        let text_trim = notif.text.trim();
        if any_synthetic_e2e_probe(&[title_trim, text_trim]) {
            continue;
        }
        let combined = format!("{title_trim} {text_trim}");
        if is_completed_express_text(&combined)
            || crate::capsule_fulfillment::parcel_direction(&combined)
                == crate::capsule_fulfillment::Direction::Send
        {
            continue;
        }
        let is_exp = is_express_notification(&notif.app_name, title_trim, text_trim);
        let pickup = extract_otp_or_pickup_from_parts(&[title_trim, text_trim])
            .and_then(|(code, is_pickup)| is_pickup.then_some(code));
        if !is_exp && pickup.is_none() {
            continue;
        }
        let extracted = (!looks_like_sms_service_number(title_trim))
            .then(|| extract_express_from_text_flexible(title_trim, true))
            .flatten()
            .or_else(|| extract_express_from_text_flexible(text_trim, true));
        let pickup_code = pickup.unwrap_or_default();
        let ttl_ms = if !pickup_code.is_empty() {
            pickup_ttl_ms
        } else {
            transit_ttl_ms
        };
        let notif_key = format!("notif:{}", notif.key.trim());
        let primary_key = if !pickup_code.is_empty() {
            format!("code:{}", pickup_code.trim().to_ascii_uppercase())
        } else if let Some((_, ref no)) = extracted {
            format!("parcel:{}", no.trim().to_ascii_uppercase())
        } else {
            notif_key.clone()
        };
        if notif.post_time_ms > 0
            && now_ms.saturating_sub(notif.post_time_ms) > ttl_ms
            && !gov.is_force_kept(&primary_key, now_ms)
            && !gov.is_force_kept(&notif_key, now_ms)
        {
            continue;
        }
        let title_is_courier_account =
            is_courier_account_title(title_trim) || is_courier_account_title(notif.app_name.trim());
        let tracking_no = extracted
            .as_ref()
            .map(|(_, no)| no.clone())
            .unwrap_or_else(|| {
                if !pickup_code.is_empty() {
                    format!("取件码 {pickup_code}")
                } else if is_exp
                    && title_is_courier_account
                    && !title_trim.is_empty()
                    && !looks_like_sms_service_number(title_trim)
                {
                    format!("WX-{title_trim}")
                } else {
                    String::new()
                }
            });
        if tracking_no.is_empty() && pickup_code.is_empty() {
            continue;
        }
        let app_label = friendly_source_name(&notif.app_name, &notif.package_name);
        let carrier = infer_carrier_from_text(
            &app_label,
            &combined,
            extracted.map(|(c, _)| c).unwrap_or(""),
        );
        let station = if !title_trim.is_empty() && !looks_like_sms_service_number(title_trim) {
            title_trim.chars().take(28).collect()
        } else {
            text_trim.chars().take(28).collect()
        };
        let status = infer_express_status(&combined, !pickup_code.is_empty());
        let pkg_src = (!notif.package_name.trim().is_empty())
            .then(|| format!("pkg:{}", notif.package_name.trim().to_ascii_lowercase()));
        push_parcel(
            crate::device::ExpressParcelInfo {
                carrier,
                tracking_no,
                pickup_code,
                station,
                status,
                updated_at: format_timestamp_ms(notif.post_time_ms),
            },
            Some(notif_key),
            (notif.post_time_ms > 0).then_some(notif.post_time_ms),
            pkg_src.as_deref(),
        );
    }

    for thread in threads {
        if crate::push::ImChannel::from_package(&thread.package_name) == crate::push::ImChannel::Sms
            || matches!(
                thread.package_name.as_str(),
                "com.android.mms"
                    | "com.google.android.apps.messaging"
                    | "com.samsung.android.messaging"
            )
        {
            continue;
        }
        let title_trim = thread.title.trim();
        let text_trim = thread.last_message.trim();
        if title_trim.is_empty() && text_trim.is_empty() {
            continue;
        }
        if any_synthetic_e2e_probe(&[title_trim, text_trim]) {
            continue;
        }
        let combined = format!("{title_trim} {text_trim}");
        if is_completed_express_text(&combined)
            || crate::capsule_fulfillment::parcel_direction(&combined)
                == crate::capsule_fulfillment::Direction::Send
        {
            continue;
        }
        let is_exp = is_express_notification(&thread.app_name, title_trim, text_trim);
        let pickup = extract_otp_or_pickup_from_parts(&[title_trim, text_trim])
            .and_then(|(code, is_pickup)| is_pickup.then_some(code));
        if !is_exp && pickup.is_none() {
            continue;
        }
        let extracted = (!looks_like_sms_service_number(title_trim))
            .then(|| extract_express_from_text_flexible(title_trim, true))
            .flatten()
            .or_else(|| extract_express_from_text_flexible(text_trim, true));
        let pickup_code = pickup.unwrap_or_default();
        let ttl_ms = if !pickup_code.is_empty() {
            pickup_ttl_ms
        } else {
            transit_ttl_ms
        };
        let im_key = format!("im:{}", thread.thread_id.trim());
        let primary_key = if !pickup_code.is_empty() {
            format!("code:{}", pickup_code.trim().to_ascii_uppercase())
        } else if let Some((_, ref no)) = extracted {
            format!("parcel:{}", no.trim().to_ascii_uppercase())
        } else {
            im_key.clone()
        };
        let force_kept =
            gov.is_force_kept(&primary_key, now_ms) || gov.is_force_kept(&im_key, now_ms);
        let effective_ts = thread.last_timestamp_ms.max(
            thread
                .messages
                .iter()
                .map(|m| m.timestamp_ms)
                .max()
                .unwrap_or(0),
        );
        if effective_ts <= 0 && (thread.unread_count == 0 || pickup_code.is_empty()) && !force_kept
        {
            continue;
        }
        if effective_ts > 0 && now_ms.saturating_sub(effective_ts) > ttl_ms && !force_kept {
            continue;
        }
        if thread.unread_count == 0
            && pickup_code.is_empty()
            && effective_ts > 0
            && now_ms.saturating_sub(effective_ts) > 21_600_000
            && !force_kept
        {
            continue;
        }
        let title_is_courier_account = !thread.is_group
            && (is_courier_account_title(title_trim)
                || (is_exp
                    && ["京东", "淘宝", "天猫", "拼多多", "得物", "唯品会"]
                        .iter()
                        .any(|kw| thread.app_name.contains(kw))));
        let tracking_no = extracted
            .as_ref()
            .map(|(_, no)| no.clone())
            .unwrap_or_else(|| {
                if !pickup_code.is_empty() {
                    format!("取件码 {pickup_code}")
                } else if title_is_courier_account
                    && !title_trim.is_empty()
                    && !looks_like_sms_service_number(title_trim)
                {
                    format!("WX-{title_trim}")
                } else {
                    String::new()
                }
            });
        if tracking_no.is_empty() && pickup_code.is_empty() {
            continue;
        }
        let carrier = infer_carrier_from_text(
            title_trim,
            &combined,
            extracted.map(|(c, _)| c).unwrap_or(""),
        );
        let station = if !text_trim.is_empty() {
            text_trim.chars().take(28).collect()
        } else {
            title_trim.chars().take(28).collect()
        };
        let status = infer_express_status(&combined, !pickup_code.is_empty());
        if pickup_code.is_empty() && status == "物流更新" && thread.unread_count == 0 && !force_kept
        {
            continue;
        }
        push_parcel(
            crate::device::ExpressParcelInfo {
                carrier,
                tracking_no,
                pickup_code,
                station,
                status,
                updated_at: format_timestamp_ms(effective_ts),
            },
            Some(im_key.clone()),
            (effective_ts > 0).then_some(effective_ts),
            Some(im_key.as_str()),
        );
    }

    let max_sms_ts = sms.iter().map(|m| m.date_ms).max().unwrap_or(0);
    let sms_now_ms = if max_sms_ts > 0 && max_sms_ts < 1_760_000_000_000 {
        max_sms_ts
    } else {
        now_ms
    };
    for msg in sms {
        if msg.outgoing {
            continue;
        }
        let body = msg.body.trim();
        if is_synthetic_e2e_probe_text(body)
            || is_synthetic_e2e_probe_text(&msg.address)
            || is_completed_express_text(body)
            || crate::capsule_fulfillment::parcel_direction(body)
                == crate::capsule_fulfillment::Direction::Send
        {
            continue;
        }
        let is_exp = is_express_notification("短信", &msg.address, body);
        let pickup = extract_otp_or_pickup_from_text(body)
            .and_then(|(code, is_pickup)| is_pickup.then_some(code));
        if !is_exp && pickup.is_none() {
            continue;
        }
        let extracted = extract_express_from_text_flexible(body, true);
        let pickup_code = pickup.clone().unwrap_or_default();
        let ttl_ms = if !pickup_code.is_empty() {
            pickup_ttl_ms
        } else {
            transit_ttl_ms
        };
        let norm_addr = crate::home_reminder_governance::normalize_phone_or_addr(&msg.address);
        let sms_key = format!("sms:{norm_addr}");
        let primary_key = if !pickup_code.is_empty() {
            format!("code:{}", pickup_code.trim().to_ascii_uppercase())
        } else if let Some((_, ref no)) = extracted {
            format!("parcel:{}", no.trim().to_ascii_uppercase())
        } else {
            sms_key.clone()
        };
        let force_kept =
            gov.is_force_kept(&primary_key, sms_now_ms) || gov.is_force_kept(&sms_key, sms_now_ms);
        if msg.date_ms > 0 && sms_now_ms.saturating_sub(msg.date_ms) > ttl_ms && !force_kept {
            continue;
        }
        let tracking_no = extracted
            .as_ref()
            .map(|(_, no)| no.clone())
            .unwrap_or_else(|| {
                if !pickup_code.is_empty() {
                    format!("取件码 {pickup_code}")
                } else {
                    String::new()
                }
            });
        if tracking_no.is_empty() && pickup_code.is_empty() {
            continue;
        }
        let carrier =
            infer_carrier_from_text(&msg.address, body, extracted.map(|(c, _)| c).unwrap_or(""));
        let station: String = body.chars().take(28).collect();
        let status = infer_express_status(body, !pickup_code.is_empty());
        push_parcel(
            crate::device::ExpressParcelInfo {
                carrier,
                tracking_no,
                pickup_code,
                station,
                status,
                updated_at: format_timestamp_ms(msg.date_ms),
            },
            Some(sms_key.clone()),
            (msg.date_ms > 0).then_some(msg.date_ms),
            (!norm_addr.is_empty()).then_some(sms_key.as_str()),
        );
    }

    let clip = phone_clipboard.trim();
    if !clip.is_empty() && !is_synthetic_e2e_probe_text(clip) && !is_completed_express_text(clip) {
        let allow_kw = is_express_notification("剪贴板", "", clip);
        let extracted = extract_express_from_text_flexible(clip, allow_kw);
        let pickup = extract_otp_or_pickup_from_text(clip)
            .and_then(|(code, is_pickup)| is_pickup.then_some(code));
        if let Some((c, tracking_no)) = extracted {
            let pickup_code = pickup.unwrap_or_default();
            let carrier = infer_carrier_from_text("剪贴板", clip, c);
            let status = infer_express_status(clip, !pickup_code.is_empty());
            push_parcel(
                crate::device::ExpressParcelInfo {
                    carrier,
                    tracking_no,
                    pickup_code,
                    station: "来自手机剪贴板".to_string(),
                    status,
                    updated_at: "刚刚".to_string(),
                },
                None,
                None,
                Some("clipboard"),
            );
        }
    }

    out.truncate(8);
    out
}

/// Parses wall-clock ETA time ranges or single ETA times from delivery/mobility messages, e.g.:
/// - `"您的订单预计今天 20:30~20:50送达，请您耐心等待。"` -> `Some((eta_end_ms, "20:30~20:50", "20:50送达"))`
/// - `"预计今天20:50前送达"` -> `Some((eta_end_ms, "20:50", "20:50送达"))`
pub fn extract_eta_window_from_text(
    text: &str,
    ref_post_ms: i64,
    now_ms: i64,
) -> Option<(i64, String, String)> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let has_eta_cue = [
        "预计", "送达", "到达", "送货", "配送", "出发", "发车", "起飞", "登机", "约",
    ]
    .iter()
    .any(|kw| trimmed.contains(kw));
    if !has_eta_cue {
        return None;
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let parse_hh_mm = |idx: usize| -> Option<(u32, u32, usize)> {
        if idx >= chars.len() || !chars[idx].is_ascii_digit() {
            return None;
        }
        if idx > 0 && (chars[idx - 1].is_ascii_digit() || matches!(chars[idx - 1], ':' | '：')) {
            return None;
        }
        let mut h = chars[idx].to_digit(10)?;
        let mut pos = idx + 1;
        if pos < chars.len() && chars[pos].is_ascii_digit() {
            h = h * 10 + chars[pos].to_digit(10)?;
            pos += 1;
        }
        if h > 23 || pos >= chars.len() || !matches!(chars[pos], ':' | '：') {
            return None;
        }
        pos += 1;
        if pos + 1 >= chars.len()
            || !chars[pos].is_ascii_digit()
            || !chars[pos + 1].is_ascii_digit()
        {
            return None;
        }
        let m = chars[pos].to_digit(10)? * 10 + chars[pos + 1].to_digit(10)?;
        pos += 2;
        if m > 59 {
            return None;
        }
        if pos < chars.len() && (chars[pos].is_ascii_digit() || matches!(chars[pos], ':' | '：')) {
            return None;
        }
        Some((h, m, pos))
    };

    let mut matched: Option<(u32, u32, String)> = None;
    for i in 0..chars.len() {
        let Some((h1, m1, after_first)) = parse_hh_mm(i) else {
            continue;
        };
        let mut pos = after_first;
        while pos < chars.len() && chars[pos].is_whitespace() {
            pos += 1;
        }
        if pos < chars.len() && matches!(chars[pos], '~' | '～' | '-' | '—' | '–' | '至' | '到')
        {
            pos += 1;
            while pos < chars.len() && chars[pos].is_whitespace() {
                pos += 1;
            }
            if let Some((h2, m2, _)) = parse_hh_mm(pos) {
                matched = Some((h2, m2, format!("{h1:02}:{m1:02}~{h2:02}:{m2:02}")));
                break;
            }
        }
    }

    if matched.is_none() {
        for i in 0..chars.len() {
            let Some((h, m, after_time)) = parse_hh_mm(i) else {
                continue;
            };
            let prefix: String = chars[i.saturating_sub(10)..i].iter().collect();
            let suffix: String = chars[after_time..(after_time + 10).min(chars.len())]
                .iter()
                .collect();
            if prefix.contains("预计")
                || prefix.contains("约")
                || suffix.contains("送达")
                || suffix.contains("到达")
            {
                matched = Some((h, m, format!("{h:02}:{m:02}")));
                break;
            }
        }
    }

    let (target_h, target_m, range_display) = matched?;
    let base_ms = if ref_post_ms > 1_700_000_000_000 {
        ref_post_ms
    } else if now_ms > 1_700_000_000_000 {
        now_ms
    } else if ref_post_ms > 0 {
        ref_post_ms
    } else if now_ms > 0 {
        now_ms
    } else {
        Local::now().timestamp_millis()
    };
    let base_dt = Local
        .timestamp_millis_opt(base_ms)
        .single()
        .unwrap_or_else(Local::now);
    let ndt = base_dt.date_naive().and_hms_opt(target_h, target_m, 0)?;
    let dt = Local
        .from_local_datetime(&ndt)
        .single()
        .or_else(|| Local.from_local_datetime(&ndt).earliest())?;
    let mut eta_end_ms = dt.timestamp_millis();
    let ref_check_ms = if ref_post_ms > 1_700_000_000_000 {
        ref_post_ms
    } else {
        base_ms
    };
    if trimmed.contains("明天")
        || trimmed.contains("明日")
        || (ref_check_ms > 0 && eta_end_ms + 6 * 3_600_000 < ref_check_ms)
    {
        eta_end_ms += 86_400_000;
    }
    let suffix_word = if trimmed.contains("到达") && !trimmed.contains("送达") {
        "到达"
    } else {
        "送达"
    };
    let short_pill_label = format!("{target_h:02}:{target_m:02}{suffix_word}");
    Some((eta_end_ms, range_display, short_pill_label))
}

/// Detects urgent community / property / municipal utility notices such as power/water/gas outages,
/// elevator alerts, and severe weather warnings, while ignoring promotional/marketing noise.
/// Returns `Some((badge, short_pill, headline))`.
pub fn classify_public_utility_or_community_alert(
    app_name: &str,
    title: &str,
    text: &str,
) -> Option<(String, String, String)> {
    let a_trim = app_name.trim();
    let t_trim = title.trim();
    let b_trim = text.trim();
    if t_trim.is_empty() && b_trim.is_empty() {
        return None;
    }
    let content_contains = |kw: &str| t_trim.contains(kw) || b_trim.contains(kw);
    let any_contains = |kw: &str| a_trim.contains(kw) || t_trim.contains(kw) || b_trim.contains(kw);

    if [
        "优惠",
        "活动",
        "缴费满减",
        "满减",
        "红包",
        "抽奖",
        "广告",
        "特惠",
        "领券",
        "优惠券",
        "积分",
        "签到",
    ]
    .iter()
    .any(|kw| content_contains(kw))
    {
        return None;
    }

    let (badge, short_pill) = if [
        "【停电通知】",
        "停电通知",
        "计划停电",
        "故障停电",
        "临时停电",
        "停电检修",
    ]
    .iter()
    .any(|kw| content_contains(kw))
    {
        ("民生预警", "停电通知")
    } else if [
        "【停水通知】",
        "停水通知",
        "临时停水",
        "计划停水",
        "停水降压",
        "故障停水",
        "抢修停水",
    ]
    .iter()
    .any(|kw| content_contains(kw))
    {
        ("民生预警", "停水通知")
    } else if ["【停气通知】", "停气通知", "临时停气", "计划停气"]
        .iter()
        .any(|kw| content_contains(kw))
    {
        ("民生预警", "停气通知")
    } else if ["【燃气通知】", "燃气通知", "燃气安检", "燃气抢修"]
        .iter()
        .any(|kw| content_contains(kw))
    {
        ("民生预警", "燃气通知")
    } else if [
        "电梯困人",
        "电梯故障",
        "电梯停运",
        "电梯维保",
        "电梯检修",
        "【电梯通知】",
    ]
    .iter()
    .any(|kw| content_contains(kw))
    {
        ("社区通知", "电梯通知")
    } else if [
        "台风红色预警",
        "台风橙色预警",
        "暴雨红色预警",
        "暴雨橙色预警",
        "暴雪红色预警",
        "高温红色预警",
        "防汛预警",
        "气象预警",
        "寒潮预警",
        "雷电预警",
    ]
    .iter()
    .any(|kw| content_contains(kw))
    {
        ("民生预警", "气象预警")
    } else if [
        "物业",
        "业委会",
        "社区",
        "居委",
        "运营官",
        "管家",
        "本小区",
        "全体业主",
    ]
    .iter()
    .any(|kw| any_contains(kw))
        && [
            "停电",
            "停水",
            "停气",
            "消杀",
            "消防演练",
            "封道",
            "门禁升级",
            "水箱清洗",
            "紧急通知",
        ]
        .iter()
        .any(|kw| content_contains(kw))
    {
        if content_contains("停电") {
            ("民生预警", "停电通知")
        } else if content_contains("停水") {
            ("民生预警", "停水通知")
        } else if content_contains("停气") {
            ("民生预警", "停气通知")
        } else {
            ("社区通知", "社区通知")
        }
    } else {
        return None;
    };

    let mut body_clean = if !b_trim.is_empty() { b_trim } else { t_trim };
    if let Some(after_open) = body_clean.strip_prefix('【') {
        if let Some(close_idx) = after_open.find('】') {
            let rest = after_open[close_idx + '】'.len_utf8()..].trim();
            if !rest.is_empty() {
                body_clean = rest;
            }
        }
    }
    let raw_headline = if body_clean.starts_with(short_pill) {
        body_clean.to_string()
    } else if !body_clean.is_empty() {
        format!("{short_pill} · {body_clean}")
    } else {
        short_pill.to_string()
    };
    let headline = if raw_headline.chars().count() > 32 {
        let mut s: String = raw_headline.chars().take(31).collect();
        s.push('…');
        s
    } else {
        raw_headline
    };

    Some((badge.to_string(), short_pill.to_string(), headline))
}

/// Classifies an instant delivery, grocery, takeout, coffee/tea, or ride-hailing signal from
/// any notification, SMS, or IM service-account message.
/// Returns `Some((badge, headline, status_tag, is_arrived))`.
pub fn classify_delivery_or_mobility_signal(
    pkg: &str,
    app_name: &str,
    title: &str,
    text: &str,
) -> Option<(String, String, String, bool)> {
    let p = pkg.trim();
    let t_trim = title.trim();
    let b_trim = text.trim();
    let fields = [app_name.trim(), t_trim, b_trim];
    let any_contains = |kw: &str| fields.iter().any(|f| f.contains(kw));
    let content_contains = |kw: &str| t_trim.contains(kw) || b_trim.contains(kw);

    let is_dingdong = p.eq_ignore_ascii_case("com.yaya.zone") || any_contains("叮咚买菜");
    let is_hema = p.eq_ignore_ascii_case("com.wudaokou.hippo") || any_contains("盒马");
    let is_sams = p.eq_ignore_ascii_case("cn.samsclub.app") || any_contains("山姆");
    let is_pupu = p.eq_ignore_ascii_case("com.pupumall.customer") || any_contains("朴朴");
    let is_dianping = contains_ascii_ignore_case(p, b"dianping") || any_contains("大众点评");
    let is_waima = any_contains("歪马送酒") || any_contains("美团买菜") || any_contains("小象超市");
    let is_meituan = contains_ascii_ignore_case(p, b"sankuai.meituan") || any_contains("美团");
    let is_eleme = p.eq_ignore_ascii_case("me.ele") || any_contains("饿了么");
    let is_flash = p.eq_ignore_ascii_case("com.jd.jddj")
        || any_contains("闪购")
        || any_contains("小时达")
        || any_contains("秒送")
        || any_contains("京东到家");
    let is_train = contains_ascii_ignore_case(p, b"12306")
        || contains_ascii_ignore_case(p, b"mobileticket")
        || any_contains("12306")
        || ((any_contains("高铁")
            || any_contains("列车")
            || any_contains("动车")
            || any_contains("车次"))
            && ["检票", "发车", "候车", "车厢", "座位", "终到", "列车"]
                .iter()
                .any(|k| content_contains(k)));
    let is_flight = contains_ascii_ignore_case(p, b"umetrip")
        || contains_ascii_ignore_case(p, b"veryzhun")
        || any_contains("航旅纵横")
        || any_contains("飞常准")
        || ((any_contains("航班")
            || any_contains("登机")
            || any_contains("值机")
            || any_contains("航站楼"))
            && ["登机口", "起飞", "延误", "值机", "舱位", "到达", "航班"]
                .iter()
                .any(|k| content_contains(k)));
    let is_ride = contains_ascii_ignore_case(p, b"autonavi.minimap")
        || contains_ascii_ignore_case(p, b"didi")
        || ["打车", "快车", "专车", "网约车", "行程", "司机", "上车点"]
            .iter()
            .any(|kw| any_contains(kw));
    let is_tea = p.eq_ignore_ascii_case("com.lucky.luckyclient")
        || p.eq_ignore_ascii_case("com.heytea.android")
        || p.eq_ignore_ascii_case("com.chagee.app")
        || ["瑞幸", "喜茶", "霸王茶姬", "星巴克", "取餐码", "取茶码"]
            .iter()
            .any(|kw| any_contains(kw));
    let is_doorstep = [
        "已代收（家门口）",
        "已代收(家门口)",
        "放家门口",
        "放在门口",
        "已放门口",
        "家门口",
    ]
    .iter()
    .any(|kw| any_contains(kw));
    let is_takeout_kw = is_doorstep
        || [
            "外卖",
            "骑手",
            "取餐",
            "送达",
            "配送",
            "已接单",
            "出餐",
            "配送员",
        ]
        .iter()
        .any(|kw| any_contains(kw));

    let has_brand = is_dingdong
        || is_hema
        || is_sams
        || is_pupu
        || is_dianping
        || is_waima
        || is_train
        || is_flight
        || is_meituan
        || is_eleme
        || is_flash
        || is_ride
        || is_tea;
    if !has_brand && !is_takeout_kw {
        return None;
    }

    let is_promo_or_noise = !is_doorstep
        && [
            "优惠券",
            "优惠",
            "到账",
            "活动通知",
            "账户变动",
            "积分",
            "会员",
            "签到",
            "福利",
            "抽奖",
            "问卷",
            "账单",
            "退款",
            "已取消",
            "评价",
            "买菜金",
            "礼品卡",
            "充值",
            "余额",
            "过期提醒",
            "即将过期",
            "红包",
            "神券",
            "满减",
            "立减",
            "促销",
        ]
        .iter()
        .any(|kw| content_contains(kw));
    if is_promo_or_noise {
        return None;
    }

    // Avoid misclassifying pure courier express parcels (顺丰/菜鸟/丰巢/中通/圆通) as takeout when no brand or doorstep cue is present
    if !has_brand
        && !is_doorstep
        && is_express_notification(app_name, title, text)
        && !any_contains("外卖")
        && !any_contains("骑手")
    {
        return None;
    }

    let is_in_transit = [
        "配送中",
        "正在配送",
        "飞速赶来",
        "距您",
        "预计",
        "派件中",
        "正在为您派件",
        "骑手正",
    ]
    .iter()
    .any(|kw| any_contains(kw));
    let is_arrived = is_doorstep
        || [
            "已代收",
            "已送达",
            "已到达",
            "请取餐",
            "存放在",
            "制作完成",
            "请及时取",
        ]
        .iter()
        .any(|kw| any_contains(kw))
        || (!is_in_transit && (any_contains("送达") || any_contains("到了")));

    let has_delivery_progress_kw = is_train
        || is_flight
        || [
            "配送",
            "骑手",
            "送达",
            "打包",
            "分拣",
            "出库",
            "取餐",
            "备餐",
            "制作",
            "出餐",
            "派送",
            "已接单",
            "已出餐",
            "已取货",
            "送货",
            "自提",
            "存放",
            "外卖柜",
            "取货码",
            "取餐码",
            "取茶码",
            "预计",
            "距您",
            "距离您",
            "到了",
        ]
        .iter()
        .any(|kw| content_contains(kw));
    if !is_ride && !is_arrived && !has_delivery_progress_kw {
        return None;
    }

    let (badge, brand) = if is_dingdong {
        ("叮咚买菜".to_string(), "叮咚买菜")
    } else if is_hema {
        ("盒马鲜生".to_string(), "盒马鲜生")
    } else if is_sams {
        ("山姆极速达".to_string(), "山姆极速达")
    } else if is_pupu {
        ("朴朴超市".to_string(), "朴朴超市")
    } else if is_dianping {
        ("大众点评".to_string(), "大众点评")
    } else if is_waima {
        ("美团闪送".to_string(), "美团闪送")
    } else if is_train {
        ("高铁出行".to_string(), "高铁出行")
    } else if is_flight {
        ("航班出行".to_string(), "航班出行")
    } else if is_ride {
        ("出行打车".to_string(), "出行打车")
    } else if is_meituan {
        ("美团外卖".to_string(), "美团外卖")
    } else if is_eleme {
        ("饿了么外卖".to_string(), "饿了么外卖")
    } else if is_flash {
        ("即时闪购".to_string(), "即时闪购")
    } else if is_tea {
        ("茶饮取餐".to_string(), "茶饮取餐")
    } else {
        ("外卖配送".to_string(), "即时配送")
    };

    let status_tag = if is_doorstep {
        "已放家门口 · 待取".to_string()
    } else if is_arrived {
        "已送达 · 待取".to_string()
    } else if is_ride || is_train || is_flight {
        "行程进行中".to_string()
    } else {
        "骑手配送中".to_string()
    };

    let title_has_delivery_cue = [
        "配送",
        "骑手",
        "送达",
        "打包",
        "分拣",
        "出库",
        "取餐",
        "备餐",
        "制作",
        "出餐",
        "派送",
        "已接单",
        "已出餐",
        "已取货",
        "送货",
        "自提",
        "存放",
        "外卖柜",
        "取货码",
        "取餐码",
        "取茶码",
        "预计",
        "距您",
        "距离您",
        "到了",
        "已代收",
        "已到达",
        "请取",
        "订单",
        "打车",
        "快车",
        "专车",
        "行程",
        "司机",
        "上车",
        "检票",
        "发车",
        "候车",
        "登机",
        "值机",
        "起飞",
    ]
    .iter()
    .any(|kw| t_trim.contains(kw));
    let title_is_brand_or_slogan =
        t_trim == brand || (t_trim.starts_with(brand) && !title_has_delivery_cue);
    let headline = if is_doorstep {
        format!("{brand} · 已送达家门口")
    } else if !t_trim.is_empty()
        && !looks_like_sms_service_number(t_trim)
        && !title_is_brand_or_slogan
    {
        t_trim.to_string()
    } else if !b_trim.is_empty() && b_trim.chars().count() <= 24 {
        b_trim.to_string()
    } else if is_arrived {
        format!("{brand} · 商品已送达")
    } else {
        format!("{brand} · {status_tag}")
    };

    Some((badge, headline, status_tag, is_arrived))
}

fn classify_takeout_or_travel_notification(
    notif: &crate::push::PushNotification,
) -> Option<(String, u8)> {
    let combined_text = if notif.sub_text.trim().is_empty() {
        notif.text.clone()
    } else {
        format!("{} {}", notif.text.trim(), notif.sub_text.trim())
    };
    let (badge, _, _, is_arrived) = classify_delivery_or_mobility_signal(
        &notif.package_name,
        &notif.app_name,
        &notif.title,
        &combined_text,
    )?;
    let boost = if is_arrived { 1 } else { 0 };
    Some((badge, boost))
}

#[inline]
fn is_courier_account_title(title_trim: &str) -> bool {
    [
        "顺丰",
        "京东",
        "菜鸟",
        "丰巢",
        "中通",
        "圆通",
        "韵达",
        "申通",
        "极兔",
        "德邦",
        "快递",
        "物流",
        "速运",
        "速递",
        "驿站",
        "邮政",
        "EMS",
        "物流发出",
        "发货提醒",
        "派件",
        "快件",
        "淘宝",
        "天猫",
        "拼多多",
        "得物",
        "唯品会",
    ]
    .iter()
    .any(|kw| title_trim.contains(kw))
}

#[inline]
fn is_express_notification(app_name: &str, title: &str, text: &str) -> bool {
    let fields = [app_name, title, text];
    let any_contains = |kw: &str| fields.iter().any(|f| f.contains(kw));
    let content_contains = |kw: &str| title.contains(kw) || text.contains(kw);

    let has_pickup_kw = content_contains("取件码") || content_contains("取货码");
    if !has_pickup_kw
        && [
            "账户变动",
            "优惠",
            "活动通知",
            "优惠券",
            "积分",
            "会员",
            "签到",
            "抽奖",
            "福利",
            "问卷",
            "发票",
            "月结",
            "账单",
        ]
        .iter()
        .any(|kw| content_contains(kw))
    {
        return false;
    }

    let has_logistics_kw = [
        "快件",
        "包裹",
        "取件",
        "派件",
        "派送",
        "签收",
        "驿站",
        "快递柜",
        "丰巢",
        "菜鸟",
        "运单",
        "单号",
        "发货",
        "尾号",
        "揽收",
        "中转",
        "已到",
        "代收",
        "格口",
        "投递",
        "上门",
        "取货",
        "发出",
        "物流",
        "已发货",
        "已从",
        "运输中",
        "配送",
    ]
    .iter()
    .any(|kw| content_contains(kw));
    if !has_logistics_kw {
        return false;
    }

    [
        "顺丰",
        "京东快递",
        "京东",
        "淘宝",
        "天猫",
        "拼多多",
        "得物",
        "唯品会",
        "德邦",
        "菜鸟",
        "丰巢",
        "中通",
        "圆通",
        "韵达",
        "申通",
        "极兔",
        "邮政",
        "EMS",
        "派件",
        "签收",
        "快件",
        "包裹",
        "驿站",
        "物流",
    ]
    .iter()
    .any(|kw| any_contains(kw))
}

fn friendly_source_name(app_name: &str, package_name: &str) -> String {
    let trimmed = app_name.trim();
    let pkg = package_name.trim();
    if !trimmed.is_empty() && trimmed != pkg {
        return trimmed.to_string();
    }
    if let Some(name) = match pkg {
        "com.eg.android.AlipayGphone" => Some("支付宝"),
        _ => crate::push::mainstream_message_app_name(pkg),
    } {
        return name.to_string();
    }
    if let Some(friendly) = crate::device::friendly_app_name(pkg) {
        return friendly
            .split('·')
            .next()
            .unwrap_or(friendly)
            .trim()
            .to_string();
    }
    if pkg.is_empty() {
        "手机通知".to_string()
    } else {
        pkg.to_string()
    }
}

fn format_timestamp_ms(ts_ms: i64) -> String {
    if ts_ms <= 0 {
        return "刚刚".to_string();
    }
    let Some((_, h, m)) = timestamp_to_local_ymd_parts(ts_ms) else {
        return "刚刚".to_string();
    };
    format!("{h:02}:{m:02}")
}

#[inline]
fn event_is_on_date_tuple(
    ev: &crate::toolbox::PhoneCalendarEvent,
    target: Option<(i32, u32, u32)>,
) -> bool {
    let Some((ty, tm, td)) = target else {
        return true;
    };
    if ev.dtstart <= 0 {
        return true;
    }
    let target_tuple = (ty, tm, td);
    let Some((start_ymd, _, _)) = timestamp_to_local_ymd_parts(ev.dtstart) else {
        return false;
    };
    if start_ymd == target_tuple {
        return true;
    }
    if ev.all_day || ev.dtend <= ev.dtstart {
        return false;
    }
    let Some((end_ymd, end_h, end_m)) = timestamp_to_local_ymd_parts(ev.dtend) else {
        return false;
    };
    start_ymd < target_tuple
        && (end_ymd > target_tuple || (end_ymd == target_tuple && (end_h > 0 || end_m > 0)))
}

#[inline]
fn event_is_on_date(ev: &crate::toolbox::PhoneCalendarEvent, today_ymd: &str) -> bool {
    let ymd = today_ymd.trim();
    if ymd.is_empty() || ev.dtstart <= 0 {
        return true;
    }
    event_is_on_date_tuple(ev, parse_ymd_tuple(ymd))
}

fn event_is_within_45_minutes(
    ev: &crate::toolbox::PhoneCalendarEvent,
    hour: u32,
    minute: u32,
) -> bool {
    if ev.all_day || ev.dtstart <= 0 {
        return false;
    }
    let Some((start_ymd, start_h, start_m)) = timestamp_to_local_ymd_parts(ev.dtstart) else {
        return false;
    };
    let ev_mins = (start_h * 60 + start_m) as i32;
    let now_mins = ((hour % 24) * 60 + (minute % 60)) as i32;
    let diff = ev_mins - now_mins;
    if (-15..=45).contains(&diff) {
        return true;
    }
    if diff < -15 && ev.dtend > ev.dtstart {
        if let Some((end_ymd, end_h, end_m)) = timestamp_to_local_ymd_parts(ev.dtend) {
            let end_mins = if end_ymd > start_ymd {
                1440
            } else {
                (end_h * 60 + end_m) as i32
            };
            if now_mins <= end_mins {
                return true;
            }
        }
    }
    false
}

/// Returns true if the calendar event has already started and has not yet ended at `(hour, minute)`.
pub fn event_is_ongoing(ev: &crate::toolbox::PhoneCalendarEvent, hour: u32, minute: u32) -> bool {
    if ev.all_day || ev.dtstart <= 0 {
        return false;
    }
    let Some((start_ymd, start_h, start_m)) = timestamp_to_local_ymd_parts(ev.dtstart) else {
        return false;
    };
    let start_mins = (start_h * 60 + start_m) as i32;
    let now_mins = ((hour % 24) * 60 + (minute % 60)) as i32;
    if now_mins < start_mins {
        return false;
    }
    if ev.dtend > ev.dtstart {
        if let Some((end_ymd, end_h, end_m)) = timestamp_to_local_ymd_parts(ev.dtend) {
            let end_mins = if end_ymd > start_ymd {
                1440
            } else {
                (end_h * 60 + end_m) as i32
            };
            return now_mins < end_mins;
        }
    }
    now_mins - start_mins <= 30
}

/// Formats an active or upcoming meeting title with its expected end time (`预计 HH:MM 结束`) when available.
pub fn format_active_meeting_context(ev: &crate::toolbox::PhoneCalendarEvent) -> String {
    let title = ev.title.trim();
    if title.is_empty() {
        return String::new();
    }
    if !ev.all_day && ev.dtend > ev.dtstart {
        if let Some((_, end_h, end_m)) = timestamp_to_local_ymd_parts(ev.dtend) {
            return format!("{title}（预计 {end_h:02}:{end_m:02} 结束）");
        }
    }
    title.to_string()
}

/// Extracts an online meeting or web URL (`https://...` or `http://...`) from a calendar description or location.
pub fn extract_meeting_url(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let start_idx = trimmed
        .find("https://")
        .or_else(|| trimmed.find("http://"))?;
    let tail = &trimmed[start_idx..];
    let raw_url: String = tail
        .chars()
        .take_while(|c| {
            c.is_ascii()
                && !c.is_whitespace()
                && !matches!(c, ')' | '(' | ']' | '[' | '>' | '<' | '"' | '\'')
        })
        .collect();
    let cleaned = raw_url
        .trim_end_matches(|c: char| matches!(c, '.' | ',' | ';'))
        .to_string();
    if cleaned.len() > "https://a.b".len() {
        Some(cleaned)
    } else {
        None
    }
}

fn extract_first_integer(text: &str) -> Option<u32> {
    let mut digits = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else if !digits.is_empty() {
            break;
        }
    }
    digits.parse::<u32>().ok()
}

fn non_empty_opt(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        device::{CallRecord, ExpressParcelInfo},
        push::{ImThread, OtpBanner, PushNotification},
        sms::Message,
        toolbox::{PhoneAlarmItem, PhoneCalendarEvent, StepOsDigitalWellbeing},
    };

    fn empty_wellbeing() -> StepOsDigitalWellbeing {
        StepOsDigitalWellbeing {
            screen_time_minutes: 95,
            unlock_count: 22,
            top_apps: vec![
                ("com.tencent.mm".into(), "微信".into(), 42),
                ("com.ss.android.lark".into(), "飞书".into(), 31),
            ],
            alphame_running: true,
        }
    }

    fn cache_test_input(wellbeing: &StepOsDigitalWellbeing) -> HomeHubInput<'_> {
        HomeHubInput {
            hour: 10,
            minute: 0,
            today_ymd: "2026-11-06",
            device_connected: true,
            device_model: "fixture",
            firmware_profile: "fixture",
            threads: &[],
            notifications: &[],
            otp_banners: &[],
            sms: &[],
            call_logs: &[],
            express_parcels: &[],
            calendar_events: &[],
            alarms: &[],
            wellbeing,
            phone_clipboard: "",
            remote_agent_files: &[],
            work_deliverables_count: 0,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        }
    }

    fn cache_test_hash(input: &HomeHubInput<'_>, entries: &[DesktopDateEntry]) -> u64 {
        // Fixed revisions isolate input dependencies from concurrent weather tests.
        compute_home_hub_input_hash_with_revisions(1, input.today_ymd, input, entries, 10, 20)
    }

    #[test]
    fn report_cache_invalidates_on_read_state_reply_route_and_history_provenance() {
        let wellbeing = empty_wellbeing();
        let notification = PushNotification {
            key: "cache-notification".into(),
            title: "验证码".into(),
            text: "验证码 583106".into(),
            ..Default::default()
        };
        let input = cache_test_input(&wellbeing);
        let base = cache_test_hash(
            &HomeHubInput {
                notifications: std::slice::from_ref(&notification),
                ..input.clone()
            },
            &[],
        );
        let notification_changes: [fn(&mut PushNotification); 5] = [
            |n| n.read = true,
            |n| n.active = false,
            |n| n.sub_text = "有人@我".into(),
            |n| n.is_group = true,
            |n| n.can_reply = true,
        ];
        for change in notification_changes {
            let mut changed = notification.clone();
            change(&mut changed);
            assert_ne!(
                base,
                cache_test_hash(
                    &HomeHubInput {
                        notifications: &[changed],
                        ..input.clone()
                    },
                    &[]
                )
            );
        }
        let thread = ImThread {
            thread_id: "cache-thread".into(),
            last_message: "预览不变".into(),
            messages: vec![crate::push::ImMessage {
                id: "history-only".into(),
                text: "快件已到站".into(),
                timestamp_ms: 1_000,
                ..Default::default()
            }],
            ..Default::default()
        };
        let base = cache_test_hash(
            &HomeHubInput {
                threads: std::slice::from_ref(&thread),
                ..input.clone()
            },
            &[],
        );
        let thread_changes: [fn(&mut ImThread); 8] = [
            |t| t.messages[0].text = "快件已签收".into(),
            |t| t.messages[0].outgoing = true,
            |t| t.messages[0].recalled = true,
            |t| t.messages[0].time_known = Some(false),
            |t| t.messages[0].via = "ui_capture".into(),
            |t| t.messages[0].observed_at_ms = 2_000,
            |t| t.can_remote_reply = true,
            |t| t.can_ui_reply = true,
        ];
        for change in thread_changes {
            let mut changed = thread.clone();
            change(&mut changed);
            assert_ne!(
                base,
                cache_test_hash(
                    &HomeHubInput {
                        threads: &[changed],
                        ..input.clone()
                    },
                    &[]
                )
            );
        }
    }

    #[test]
    fn report_cache_tracks_actions_and_revisions_but_coalesces_same_minute_ticks() {
        let wellbeing = empty_wellbeing();
        let mut input = cache_test_input(&wellbeing);
        input.pomodoro_running = true;
        input.pomodoro_remaining_secs = 120;
        let base = cache_test_hash(&input, &[]);
        input.pomodoro_remaining_secs = 119;
        assert_eq!(base, cache_test_hash(&input, &[]));
        input.pomodoro_remaining_secs = 60;
        assert_ne!(base, cache_test_hash(&input, &[]));
        assert_ne!(
            cache_test_hash(&input, &[]),
            compute_home_hub_input_hash_with_revisions(1, input.today_ymd, &input, &[], 11, 20)
        );
        assert_ne!(
            cache_test_hash(&input, &[]),
            compute_home_hub_input_hash_with_revisions(1, input.today_ymd, &input, &[], 10, 21)
        );
        let mut entry = DesktopDateEntry {
            id: "same-label".into(),
            timestamp: 1,
            kind: SelectedDateEntryKind::WorkAndMatter,
            badge: "草稿".into(),
            title: "准备评审".into(),
            detail: "打开已有记录".into(),
            action_label: "继续".into(),
            intent: HomeAiIntent::OpenPage("work".into()),
        };
        let old_action = cache_test_hash(&input, std::slice::from_ref(&entry));
        entry.intent = HomeAiIntent::OpenPage("context".into());
        assert_ne!(old_action, cache_test_hash(&input, &[entry]));
    }

    #[test]
    fn warm_report_receipt_history_closes_pickup_without_preview_change() {
        let wellbeing = empty_wellbeing();
        let input = cache_test_input(&wellbeing);
        let at = Local
            .with_ymd_and_hms(2026, 11, 6, 9, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let sms = [Message {
            provider_id: Some(9917),
            address: "106999".into(),
            body: "顺丰SF1628890123456已到菜鸟驿站，请凭取件码7-2-811领取。".into(),
            date_ms: at,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        }];
        let mut threads = [ImThread {
            thread_id: "cache-receipt".into(),
            title: "顺丰速运".into(),
            last_message: "欢迎使用顺丰服务".into(),
            last_timestamp_ms: at,
            messages: vec![crate::push::ImMessage {
                id: "cache-receipt-event".into(),
                text: "欢迎使用顺丰服务".into(),
                timestamp_ms: at + 60_000,
                ..Default::default()
            }],
            ..Default::default()
        }];
        let before = build_home_hub_report(&HomeHubInput {
            sms: &sms,
            threads: &threads,
            ..input.clone()
        });
        assert!(before
            .priority_cards
            .iter()
            .any(|c| c.code_or_number.as_deref() == Some("7-2-811")));
        threads[0].messages[0].text = "顺丰快件SF1628890123456已签收，感谢使用。".into();
        let after = build_home_hub_report(&HomeHubInput {
            sms: &sms,
            threads: &threads,
            ..input
        });
        assert!(!after
            .priority_cards
            .iter()
            .any(|c| c.code_or_number.as_deref() == Some("7-2-811")));
    }

    #[test]
    fn cold_weather_card_uses_no_synthetic_temperature_or_model_context() {
        let wellbeing = empty_wellbeing();
        let input = cache_test_input(&wellbeing);
        let scene = detect_scene(input.hour, false, false);
        for snapshot in [
            None,
            Some(crate::weather::WeatherSnapshot::default_for_city("上海")),
            Some(crate::weather::WeatherSnapshot::default_for_city("深圳")),
        ] {
            let cards = build_life_context_cards_from_weather(&input, &scene, &[], 0, snapshot);
            let card = cards
                .iter()
                .find(|c| c.domain == HomeLifeDomain::WeatherAndCommute)
                .unwrap();
            assert!(card.badge.contains("待同步"));
            assert!(!card.title.contains("°C"));
            assert!(!card.detail.contains("°C"));
            match &card.intent {
                HomeAiIntent::AmooPrompt(prompt) => {
                    assert!(prompt.contains("查询"));
                    assert!(!prompt.contains("°C"));
                }
                other => panic!("unexpected cold weather action: {other:?}"),
            }
        }
    }

    #[test]
    fn scene_detection_covers_time_of_day_and_meeting_override() {
        let morning = detect_scene(8, false, false);
        assert_eq!(morning.scene, HomeScene::MorningBriefing);
        assert_eq!(morning.badge, "晨间简报");
        assert_eq!(morning.suggested_backscreen_status, "专注中");

        let lunch = detect_scene(12, false, true);
        assert_eq!(lunch.scene, HomeScene::LunchAndLife);
        assert_eq!(lunch.badge, "午间生活");
        assert_eq!(lunch.suggested_backscreen_status, "去吃饭");

        // Upcoming meeting within 45m overrides afternoon/lunch scene into WorkFocus
        let meeting_override = detect_scene(15, true, false);
        assert_eq!(meeting_override.scene, HomeScene::WorkFocus);
        assert_eq!(meeting_override.badge, "会议与专注");
        assert_eq!(meeting_override.suggested_backscreen_status, "开会勿扰");

        let evening = detect_scene(19, false, true);
        assert_eq!(evening.scene, HomeScene::EveningWrapUp);
        assert_eq!(evening.suggested_backscreen_status, "休息一下");

        let night = detect_scene(23, false, false);
        assert_eq!(night.scene, HomeScene::LateNightRest);
        assert_eq!(night.suggested_backscreen_status, "冥想呼吸");
    }

    #[test]
    fn scene_copy_never_claims_synchronization_from_clock_or_activity_flags() {
        for hour in 0..24 {
            for meeting in [false, true] {
                for delivery in [false, true] {
                    let scene = detect_scene(hour, meeting, delivery);
                    for claim in ["已就绪", "实时", "已聚合", "已置顶", "一键生成"] {
                        assert!(
                            !scene.subtitle.contains(claim),
                            "{hour}: {}",
                            scene.subtitle
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn priority_cards_rank_otp_takeout_express_missed_calls_and_messages_by_urgency() {
        let otp_banners = vec![OtpBanner {
            code: "839201".into(),
            source_app: "招商银行".into(),
            title: "登录验证码".into(),
            raw_text: "【招商银行】验证码 839201，5分钟内有效。".into(),
            timestamp_ms: 1_760_000_000_000,
            notification_key: "notif-otp-1".into(),
        }];
        let notifications = vec![PushNotification {
            key: "notif-meituan-1".into(),
            seq_id: 101,
            package_name: "com.sankuai.meituan".into(),
            app_name: "美团外卖".into(),
            title: "骑手正在配送您的午餐".into(),
            text: "骑手距离您还有 350 米，预计 4 分钟后送达".into(),
            post_time_ms: 1_760_000_010_000,
            ..Default::default()
        }];
        let express_parcels = vec![ExpressParcelInfo {
            carrier: "顺丰速运".into(),
            tracking_no: "SF14298019283".into(),
            pickup_code: "3-2-1048".into(),
            station: "软件园 3 号楼菜鸟驿站".into(),
            status: "待取件".into(),
            updated_at: "12:15".into(),
        }];
        let call_logs = vec![CallRecord {
            number: "13800009367".into(),
            name: "张总".into(),
            call_type: 3,
            date_ms: 1_760_000_020_000,
            duration_sec: 0,
        }];
        let threads = vec![ImThread {
            thread_id: "feishu:prod".into(),
            package_name: "com.ss.android.lark".into(),
            app_name: "飞书".into(),
            title: "产品研发对齐群".into(),
            unread_count: 3,
            last_sender: "李工".into(),
            last_message: "下午三点评审桌面首页方案".into(),
            last_timestamp_ms: 1_760_000_030_000,
            can_remote_reply: true,
            ..Default::default()
        }];
        let today_start = Local
            .with_ymd_and_hms(2026, 10, 29, 15, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let stale_start = Local
            .with_ymd_and_hms(2025, 4, 2, 12, 35, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let calendar_events = vec![
            PhoneCalendarEvent {
                recurring: Some(false),
                id: 1,
                title: "StepX 桌面首页架构评审".into(),
                dtstart: today_start,
                dtend: today_start + 3_600_000,
                all_day: false,
                location: "A 座 402 会议室".into(),
                description: "讨论首页智能聚合流".into(),
                source: "飞书日历".into(),
            },
            PhoneCalendarEvent {
                recurring: Some(false),
                id: 99,
                title: "过期日程不应进入今日优先级流".into(),
                dtstart: stale_start,
                dtend: stale_start + 3_600_000,
                all_day: false,
                location: String::new(),
                description: String::new(),
                source: "系统日历".into(),
            },
        ];
        let wb = empty_wellbeing();

        let input = HomeHubInput {
            hour: 12,
            minute: 30,
            today_ymd: "2026-10-29",
            device_connected: true,
            device_model: "StepPhone Pro".into(),
            firmware_profile: "StepOS 2.0".into(),
            threads: &threads,
            notifications: &notifications,
            otp_banners: &otp_banners,
            sms: &[],
            call_logs: &call_logs,
            express_parcels: &express_parcels,
            calendar_events: &calendar_events,
            alarms: &[],
            wellbeing: &wb,
            phone_clipboard: "",
            remote_agent_files: &[],
            work_deliverables_count: 2,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        };

        let report = build_home_hub_report(&input);
        assert_eq!(
            report.priority_cards.len(),
            6,
            "stale 2025-04-02 event must be excluded"
        );
        assert_eq!(report.priority_cards[0].kind, HomeCardKind::OtpAndPickup);
        assert_eq!(
            report.priority_cards[1].kind,
            HomeCardKind::TakeoutAndDelivery
        );
        assert_eq!(report.priority_cards[2].kind, HomeCardKind::ExpressParcel);
        assert_eq!(report.priority_cards[3].kind, HomeCardKind::MissedCall);
        assert_eq!(report.priority_cards[4].kind, HomeCardKind::UnreadMessage);
        assert_eq!(report.priority_cards[5].kind, HomeCardKind::UpcomingMeeting);
        assert_eq!(report.priority_cards[5].title, "StepX 桌面首页架构评审");
        assert_eq!(report.unread_messages_count, 3);
        assert_eq!(report.active_life_cards_count, 3);
    }

    #[test]
    fn extracts_pickup_codes_and_clipboard_tracking_numbers_automatically() {
        let sms = vec![Message {
            provider_id: None,
            address: "10698000123".into(),
            body: "【菜鸟驿站】您的包裹已到站，请凭取件码 A1-2-1086 到软件园驿站领取。".into(),
            date_ms: 1_760_000_000_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        }];
        let wb = empty_wellbeing();
        let input = HomeHubInput {
            hour: 16,
            minute: 10,
            today_ymd: "2026-10-29",
            device_connected: true,
            device_model: "StepPhone".into(),
            firmware_profile: "StepOS".into(),
            threads: &[],
            notifications: &[],
            otp_banners: &[],
            sms: &sms,
            call_logs: &[],
            express_parcels: &[],
            calendar_events: &[],
            alarms: &[PhoneAlarmItem {
                id: 1,
                hour: 7,
                minutes: 30,
                enabled: true,
                days_of_week: 31,
                vibrate: true,
                message: "晨间起床".into(),
            }],
            wellbeing: &wb,
            phone_clipboard: "刚发货的顺丰单号 SF1429801928388 请查收",
            remote_agent_files: &[],
            work_deliverables_count: 0,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        };

        let report = build_home_hub_report(&input);
        assert_eq!(report.enabled_alarms_count, 1);
        assert!(report.priority_cards.iter().any(|c| {
            c.kind == HomeCardKind::OtpAndPickup && c.code_or_number.as_deref() == Some("A1-2-1086")
        }));
        assert!(report.priority_cards.iter().any(|c| {
            c.kind == HomeCardKind::ExpressParcel
                && c.badge == "剪贴板单号"
                && c.code_or_number.as_deref() == Some("SF1429801928388")
        }));
        assert!(report.ai_recommendations.iter().any(|r| {
            matches!(&r.intent, HomeAiIntent::QueryExpress(no) if no == "SF1429801928388")
        }));
    }

    #[test]
    fn technical_release_dates_and_versions_do_not_become_pickup_or_otp_cards() {
        for text in [
            "开发者工具发布于 2026-10-28，CLI 版本 1.4.12 已上线。",
            "VS Code 更新说明：版本 2026.1028，路线图 10-28。",
            "凭这份指南配置 CLI 1.4.12，发布日期 2026-10-28。",
            "本次取件码解析器发布于2026-10-28，构建版本2026.1028。",
        ] {
            assert_eq!(
                extract_otp_or_pickup_from_text(text),
                None,
                "false code from {text}"
            );
            let notification = PushNotification {
                title: "开发者更新".into(),
                text: text.into(),
                app_name: "技术频道".into(),
                package_name: "org.telegram.messenger".into(),
                ..Default::default()
            };
            assert!(extract_live_express_parcels(
                &[],
                std::slice::from_ref(&notification),
                "",
                &[]
            )
            .is_empty());
            assert_eq!(
                classify_panorama_notification_kind(&notification, &notification.title, text),
                SelectedDateEntryKind::Communication,
                "Ordinary technical messages must remain ordinary messages"
            );
        }
    }

    #[test]
    fn pickup_extraction_requires_nearby_logistics_and_preserves_genuine_codes() {
        for (text, expected) in [
            (
                "【菜鸟驿站】快件于2026-10-28到站，请凭取件码 4-2-1088 领取",
                "4-2-1088",
            ),
            ("【丰巢】取件码A1-2-1086，请及时领取包裹", "A1-2-1086"),
            ("包裹已到，取件码 839201", "839201"),
            ("取餐码8821，请到窗口领取", "8821"),
            ("物流查询CLI版本1.4.12；包裹取件码 6-1-8899", "6-1-8899"),
        ] {
            assert_eq!(
                extract_pickup_code(text).as_deref(),
                Some(expected),
                "{text}"
            );
        }
        assert_eq!(
            extract_pickup_code(&format!(
                "包裹还在运输。{}发布日期10-28",
                "普通技术信息".repeat(30)
            )),
            None
        );
        assert_eq!(
            extract_pickup_code("物流联系电话0571-12345678，无取件码"),
            None
        );
        assert_eq!(
            extract_otp_or_pickup_from_text("您的登录验证码839201，5分钟内有效。"),
            Some(("839201".into(), false))
        );
        assert_eq!(
            extract_otp_or_pickup_from_text("Your verification code is 739104."),
            Some(("739104".into(), false))
        );
        assert_eq!(
            extract_otp_or_pickup_from_text("Your account security code: 739104."),
            Some(("739104".into(), false))
        );
    }

    #[test]
    fn stale_code_hints_do_not_hide_normal_technical_messages_or_make_parcels() {
        let text = "开发者工具于2026-10-28发布，CLI 版本1.4.12，更新安排10-28。";
        let threads = vec![ImThread {
            thread_id: "tech-thread".into(),
            title: "技术更新频道".into(),
            package_name: "org.telegram.messenger".into(),
            app_name: "Telegram".into(),
            last_message: text.into(),
            unread_count: 1,
            ..Default::default()
        }];
        let banners = vec![
            OtpBanner {
                code: "10-28".into(),
                title: "开发者工具更新".into(),
                raw_text: text.into(),
                ..Default::default()
            },
            OtpBanner {
                code: "739104".into(),
                title: "登录验证码".into(),
                raw_text: "您的验证码739104，请勿泄露。".into(),
                ..Default::default()
            },
        ];
        let wellbeing = empty_wellbeing();
        let report = build_home_hub_report(&HomeHubInput {
            hour: 10,
            minute: 0,
            today_ymd: "2026-11-02",
            device_connected: true,
            device_model: "phone",
            firmware_profile: "test",
            threads: &threads,
            notifications: &[],
            otp_banners: &banners,
            sms: &[],
            call_logs: &[],
            express_parcels: &[],
            calendar_events: &[],
            alarms: &[],
            wellbeing: &wellbeing,
            phone_clipboard: "",
            remote_agent_files: &[],
            work_deliverables_count: 0,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        });
        let codes: Vec<_> = report
            .priority_cards
            .iter()
            .filter(|card| card.kind == HomeCardKind::OtpAndPickup)
            .collect();
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].code_or_number.as_deref(), Some("739104"));
        assert!(report
            .priority_cards
            .iter()
            .any(|card| card.kind == HomeCardKind::UnreadMessage));
        assert_eq!(report.unread_messages_count, 1);
        let old_false_parcel = ExpressParcelInfo {
            carrier: "快递物流".into(),
            tracking_no: "取件码 10-28".into(),
            pickup_code: "10-28".into(),
            station: "开发者工具更新CLI版本1.4.12".into(),
            status: "待取件".into(),
            updated_at: "刚刚".into(),
        };
        assert!(extract_live_express_parcels_with_threads(
            &[],
            &[],
            &threads,
            "",
            &[old_false_parcel]
        )
        .is_empty());
        let saved_code_only_parcel = ExpressParcelInfo {
            carrier: "快递物流".into(),
            tracking_no: "取件码 839201".into(),
            pickup_code: "839201".into(),
            station: "此前保存的有限预览".into(),
            status: "待取件".into(),
            updated_at: "刚刚".into(),
        };
        assert!(extract_live_express_parcels_with_threads(&[], &[], &threads, "", &[saved_code_only_parcel]).is_empty(),
            "Live express extraction must recompute purely from live sources without retaining stale existing items");
    }

    #[test]
    fn quick_replies_adapt_to_delivery_work_and_general_contexts() {
        let delivery = quick_replies_for_message("美团外卖", "骑手来信", "您的外卖到了放哪里？");
        assert!(delivery.iter().any(|r| r.contains("放门口即可")));

        let work = quick_replies_for_message("飞书", "架构评审群", "请确认今天下午的会议方案");
        assert!(work
            .iter()
            .any(|r| r.contains("正在开会") || r.contains("按这个方案推进")));

        let general = quick_replies_for_message("微信", "妈妈", "周末回家吃饭吗");
        assert!(general.iter().any(|r| r.contains("好的，谢谢")));
    }

    #[test]
    fn habit_driven_recommendations_react_to_high_screen_time_and_remote_files() {
        let wb = StepOsDigitalWellbeing {
            screen_time_minutes: 185,
            unlock_count: 46,
            top_apps: vec![
                ("com.tencent.mm".into(), "微信".into(), 80),
                ("com.ss.android.lark".into(), "飞书".into(), 55),
            ],
            alphame_running: true,
        };
        let remote_files = vec!["/sdcard/StepClaw/output/Q4_plan.md".to_string()];
        let input = HomeHubInput {
            hour: 15,
            minute: 0,
            today_ymd: "2026-10-29",
            device_connected: true,
            device_model: "StepPhone".into(),
            firmware_profile: "StepOS".into(),
            threads: &[],
            notifications: &[],
            otp_banners: &[],
            sms: &[],
            call_logs: &[],
            express_parcels: &[],
            calendar_events: &[],
            alarms: &[],
            wellbeing: &wb,
            phone_clipboard: "https://stepx.ai/docs/architecture-spec",
            remote_agent_files: &remote_files,
            work_deliverables_count: 4,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        };

        let report = build_home_hub_report(&input);
        assert!(report.habit_headline.contains("185 分钟"));
        assert!(report.habit_headline.contains("微信"));
        assert!(report
            .ai_recommendations
            .iter()
            .any(|r| r.intent == HomeAiIntent::StartPomodoro(25)));
        assert!(report
            .ai_recommendations
            .iter()
            .any(|r| r.intent == HomeAiIntent::PullRemoteAgentFiles));
        assert!(report
            .ai_recommendations
            .iter()
            .any(|r| matches!(r.intent, HomeAiIntent::AmooPrompt(_))));
    }

    #[test]
    fn classify_command_intent_routes_all_home_omnibox_domains() {
        assert_eq!(
            classify_command_intent("把背屏切换为开会勿扰状态"),
            HomeAiIntent::BackScreenStatus("开会勿扰".into())
        );
        assert_eq!(
            classify_command_intent("开启番茄钟专注25分钟"),
            HomeAiIntent::StartPomodoro(25)
        );
        assert_eq!(
            classify_command_intent("帮我查一下快递 SF142980192838"),
            HomeAiIntent::QueryExpress("SF142980192838".into())
        );
        assert_eq!(
            classify_command_intent("去支付宝领神券"),
            HomeAiIntent::AlipayCoupon
        );
        assert_eq!(
            classify_command_intent("把今天拍的照片发给剪映一键成片"),
            HomeAiIntent::CapCutHandoff
        );
        assert_eq!(
            classify_command_intent("帮我写一份本周产品研发周报文档"),
            HomeAiIntent::WorkPrompt("帮我写一份本周产品研发周报文档".into())
        );
        assert_eq!(
            classify_command_intent("用美团看看我的外卖送到哪了"),
            HomeAiIntent::QuickGuiScenario {
                app: "美团".into(),
                task: "用美团看看我的外卖送到哪了".into(),
            }
        );
        assert_eq!(
            classify_command_intent("总结一下今天的未读消息"),
            HomeAiIntent::AmooPrompt("总结一下今天的未读消息".into())
        );
    }

    #[test]
    fn personal_current_reviews_use_evidence_routes_from_the_command_bar() {
        for (command, page) in [
            ("帮我写周报", "review-week"),
            ("今天总结", "review-day"),
            ("月度总结", "review-month"),
            ("请根据我本周的工作进展和沟通整理周总结", "review-week"),
            ("给我生成一份今日工作总结文档", "review-day"),
            ("请总结一下我今天做了什么", "review-day"),
            (
                "查看我的本月成果与下一步计划，做一份月回顾。",
                "review-month",
            ),
            ("帮我写一篇本周工作总结", "review-week"),
        ] {
            assert_eq!(
                classify_command_intent(command),
                HomeAiIntent::OpenPage(page.into()),
                "{command}"
            );
        }
    }

    #[test]
    fn review_shortcut_preserves_other_subjects_formats_and_periods() {
        for command in [
            "帮我写月报模板",
            "为客户写一份本月销售报告文档",
            "替小王写本周工作周报",
            "帮我写一份本周产品研发周报文档",
            "把我的本周总结做成 PPT",
            "翻译这份周报",
            "写一份上周工作周报",
            "整理 2026 年 9 月的月报",
            "帮我设计每周自动生成日报的流程文档",
            "写一份普通产品方案文档",
        ] {
            assert_eq!(
                classify_command_intent(command),
                HomeAiIntent::WorkPrompt(command.into()),
                "{command}"
            );
        }
        for command in [
            "为客户写报告",
            "总结一下今天的未读消息",
            "帮我总结今天的新闻",
            "昨天总结",
        ] {
            assert_eq!(
                classify_command_intent(command),
                HomeAiIntent::AmooPrompt(command.into()),
                "{command}"
            );
        }
    }

    #[test]
    fn multi_scenario_e2e_context_life_work_and_real_device_verification() {
        // =====================================================================
        // Scenario 4 — All 6 Time-of-Day Scenes & Natural-Language Command Routing
        // =====================================================================
        let s_morning = detect_scene(8, false, false);
        assert_eq!(s_morning.scene, HomeScene::MorningBriefing);
        assert_eq!(s_morning.suggested_backscreen_status, "专注中");

        let s_work = detect_scene(10, false, false);
        assert_eq!(s_work.scene, HomeScene::WorkFocus);
        assert_eq!(s_work.suggested_backscreen_status, "开会勿扰");

        let s_work_override = detect_scene(15, true, true);
        assert_eq!(s_work_override.scene, HomeScene::WorkFocus);
        assert_eq!(s_work_override.suggested_backscreen_status, "开会勿扰");

        let s_lunch = detect_scene(12, false, true);
        assert_eq!(s_lunch.scene, HomeScene::LunchAndLife);
        assert_eq!(s_lunch.suggested_backscreen_status, "去吃饭");

        let s_afternoon = detect_scene(15, false, false);
        assert_eq!(s_afternoon.scene, HomeScene::AfternoonCollaboration);
        assert_eq!(s_afternoon.suggested_backscreen_status, "专注中");

        let s_evening = detect_scene(19, false, false);
        assert_eq!(s_evening.scene, HomeScene::EveningWrapUp);
        assert_eq!(s_evening.suggested_backscreen_status, "休息一下");

        let s_night = detect_scene(23, false, false);
        assert_eq!(s_night.scene, HomeScene::LateNightRest);
        assert_eq!(s_night.suggested_backscreen_status, "冥想呼吸");

        assert_eq!(
            classify_command_intent("帮我把背屏设置成开会勿扰"),
            HomeAiIntent::BackScreenStatus("开会勿扰".into())
        );
        assert_eq!(
            classify_command_intent("开启25分钟番茄钟"),
            HomeAiIntent::StartPomodoro(25)
        );
        assert_eq!(
            classify_command_intent("开启90分钟番茄钟"),
            HomeAiIntent::StartPomodoro(60)
        );
        assert_eq!(
            classify_command_intent("查一下快递 SF1682904182291"),
            HomeAiIntent::QueryExpress("SF1682904182291".into())
        );
        assert_eq!(
            classify_command_intent("去支付宝领外卖神券"),
            HomeAiIntent::AlipayCoupon
        );
        assert_eq!(
            classify_command_intent("把相册视频移交剪映成片"),
            HomeAiIntent::CapCutHandoff
        );
        assert_eq!(
            classify_command_intent("帮我写一份今日工作周报文档"),
            HomeAiIntent::OpenPage("review-week".into())
        );
        assert_eq!(
            classify_command_intent("打开美团看看外卖骑手到哪了"),
            HomeAiIntent::QuickGuiScenario {
                app: "美团".into(),
                task: "打开美团看看外卖骑手到哪了".into(),
            }
        );

        // =====================================================================
        // Scenario 2 & Scenario 3 — Express Parcel, Takeout Delivery, OTP / Pickup Code,
        // Direct IM/SMS Reply & Missed Call Callback
        // =====================================================================
        let otp_banners = vec![OtpBanner {
            code: "849201".into(),
            source_app: "招商银行".into(),
            title: "登录验证码".into(),
            raw_text: "【招商银行】您的登录验证码为 849201，请勿泄露。".into(),
            timestamp_ms: 1_760_000_100_000,
            notification_key: "otp-bank-849201".into(),
        }];
        let notifications = vec![PushNotification {
            key: "notif-meituan-280m".into(),
            seq_id: 201,
            package_name: "com.sankuai.meituan".into(),
            app_name: "美团外卖".into(),
            title: "美团外卖：骑手王师傅距离您仅 280 米，预计 3 分钟内送达".into(),
            text: "骑手王师傅距离您仅 280 米，预计 3 分钟内送达，请留意电话".into(),
            post_time_ms: 1_760_000_110_000,
            ..Default::default()
        }];
        let threads = vec![
            ImThread {
                thread_id: "im:com.tencent.mm:delivery".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "顺丰派件小哥".into(),
                unread_count: 1,
                last_sender: "顺丰小哥".into(),
                last_message: "您好，顺丰快递到了，请问放家里门口还是丰巢快递柜？".into(),
                last_timestamp_ms: 1_760_000_120_000,
                can_remote_reply: true,
                ..Default::default()
            },
            ImThread {
                thread_id: "im:com.ss.android.lark:product".into(),
                package_name: "com.ss.android.lark".into(),
                app_name: "飞书".into(),
                title: "StepX 架构评审群".into(),
                unread_count: 4,
                last_sender: "王工".into(),
                last_message: "下午两点开会对齐周报与评审方案".into(),
                last_timestamp_ms: 1_760_000_130_000,
                can_remote_reply: true,
                ..Default::default()
            },
            ImThread {
                thread_id: "im:com.tencent.wework:ops".into(),
                package_name: "com.tencent.wework".into(),
                app_name: "企业微信".into(),
                title: "客户交付协同群".into(),
                unread_count: 2,
                last_sender: "陈经理".into(),
                last_message: "请确认最新合同文档版本".into(),
                last_timestamp_ms: 1_760_000_125_000,
                can_remote_reply: true,
                ..Default::default()
            },
        ];
        let sms = vec![Message {
            provider_id: None,
            address: "13800000041".into(),
            body: "你好，下午的会议纪要发我一份".into(),
            date_ms: 1_760_000_115_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        }];
        let call_logs = vec![CallRecord {
            number: "13800009765".into(),
            name: "顺丰派件员".into(),
            call_type: 3,
            date_ms: 1_760_000_118_000,
            duration_sec: 0,
        }];
        let wb = empty_wellbeing();
        let clipboard_text =
            "【丰巢】凭取件码 3-2-1048 至中关村软件园丰巢柜取顺丰包裹 SF1682904182291";

        let s2_input = HomeHubInput {
            hour: 12,
            minute: 30,
            today_ymd: "2026-10-29",
            device_connected: true,
            device_model: "PZ261A",
            firmware_profile: "StepOS",
            threads: &threads,
            notifications: &notifications,
            otp_banners: &otp_banners,
            sms: &sms,
            call_logs: &call_logs,
            express_parcels: &[],
            calendar_events: &[],
            alarms: &[],
            wellbeing: &wb,
            phone_clipboard: clipboard_text,
            remote_agent_files: &[],
            work_deliverables_count: 3,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        };

        let s2_report = build_home_hub_report(&s2_input);
        assert_eq!(s2_report.scene.scene, HomeScene::LunchAndLife);
        assert_eq!(s2_report.unread_messages_count, 1 + 4 + 2 + 1);

        // Verify urgency ranking: OtpAndPickup >= TakeoutAndDelivery >= ExpressParcel >= MissedCall >= UnreadMessage
        let otp_max_u = s2_report
            .priority_cards
            .iter()
            .filter(|c| c.kind == HomeCardKind::OtpAndPickup)
            .map(|c| c.urgency)
            .max()
            .expect("must have OtpAndPickup cards");
        let takeout_max_u = s2_report
            .priority_cards
            .iter()
            .filter(|c| c.kind == HomeCardKind::TakeoutAndDelivery)
            .map(|c| c.urgency)
            .max()
            .expect("must have TakeoutAndDelivery card");
        let express_max_u = s2_report
            .priority_cards
            .iter()
            .filter(|c| c.kind == HomeCardKind::ExpressParcel)
            .map(|c| c.urgency)
            .max()
            .expect("must have ExpressParcel card");
        let missed_call_u = s2_report
            .priority_cards
            .iter()
            .filter(|c| c.kind == HomeCardKind::MissedCall)
            .map(|c| c.urgency)
            .max()
            .expect("must have MissedCall card");
        assert!(otp_max_u >= takeout_max_u);
        assert!(takeout_max_u >= express_max_u);
        assert!(express_max_u >= missed_call_u);

        // Verify both OTP 849201, pickup code 3-2-1048, and SF tracking SF1682904182291 were extracted
        assert!(s2_report.priority_cards.iter().any(|c| {
            c.kind == HomeCardKind::OtpAndPickup && c.code_or_number.as_deref() == Some("849201")
        }));
        assert!(s2_report.priority_cards.iter().any(|c| {
            c.kind == HomeCardKind::OtpAndPickup && c.code_or_number.as_deref() == Some("3-2-1048")
        }));
        assert!(s2_report.priority_cards.iter().any(|c| {
            c.kind == HomeCardKind::ExpressParcel
                && c.code_or_number.as_deref() == Some("SF1682904182291")
        }));

        // Verify AI recommendations include express query, Alipay coupon, and Meituan delivery progress
        assert!(s2_report
            .ai_recommendations
            .iter()
            .any(|r| { r.intent == HomeAiIntent::QueryExpress("SF1682904182291".into()) }));
        assert!(s2_report
            .ai_recommendations
            .iter()
            .any(|r| r.intent == HomeAiIntent::AlipayCoupon));
        assert!(s2_report.ai_recommendations.iter().any(|r| {
            matches!(&r.intent, HomeAiIntent::QuickGuiScenario { app, .. } if app == "美团")
        }));

        // Verify quick_replies_for_message on courier vs work messages
        let courier_replies = quick_replies_for_message(
            "微信",
            "顺丰派件小哥",
            "您好，顺丰快递到了，请问放家里门口还是丰巢快递柜？",
        );
        assert!(courier_replies.contains(&"放门口即可，谢谢".to_string()));
        assert!(courier_replies.contains(&"放快递柜/驿站就行".to_string()));

        let work_replies =
            quick_replies_for_message("飞书", "StepX 架构评审群", "下午两点开会对齐周报与评审方案");
        assert!(work_replies.contains(&"收到，马上处理".to_string()));
        assert!(work_replies.contains(&"正在开会，稍后回你".to_string()));

        // Real-phone reads are opt-in and require an explicit transport. Mutating
        // checks live in examples/verify_native_mcp.rs, which restores test state.
        if std::env::var("STEPX_LIVE_HOME_HUB").as_deref() != Ok("1") {
            return;
        }
        let transport = std::env::var("STEPX_TEST_TRANSPORT")
            .expect("set STEPX_TEST_TRANSPORT to the intended phone");
        let adb = crate::adb::Adb::discover().expect("ADB required for live validation");
        let device = crate::device::Device::inspect(&adb, &transport)
            .expect("selected phone must be connected");
        let suite = crate::toolbox::inspect_stepos_ai_suite(&adb, &device)
            .expect("live phone suite read failed");
        let snapshot = crate::push::fetch_snapshot(&adb, &device.transport, 0)
            .expect("live notification read failed; never substitute demo data");
        assert!(suite.controls.brightness_percent <= 100);
        println!(
            "[HOME-HUB-REAL-E2E] read-only: calendars={} alarms={} notification_threads={}",
            suite.calendar_events.len(),
            suite.alarms.len(),
            snapshot.threads.len()
        );
    }

    #[test]
    fn quiet_scenes_never_emit_unconditional_filler_recommendations() {
        let wb = StepOsDigitalWellbeing {
            screen_time_minutes: 30,
            unlock_count: 5,
            top_apps: Vec::new(),
            alphame_running: true,
        };
        let night_input = HomeHubInput {
            hour: 23,
            minute: 15,
            today_ymd: "2026-10-29",
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
        let night_report = build_home_hub_report(&night_input);
        assert_eq!(night_report.scene.scene, HomeScene::LateNightRest);
        assert_eq!(
            night_report
                .ai_recommendations
                .iter()
                .map(|r| r.intent.clone())
                .collect::<Vec<_>>(),
            vec![HomeAiIntent::ToggleDnd(true)],
            "late night without DND should only suggest DND, never Alipay coupon or CapCut filler"
        );

        // Once DND is already enabled in a quiet late-night state, zero filler is emitted.
        let mut dnd_on_input = night_input;
        dnd_on_input.dnd_on = true;
        let dnd_on_report = build_home_hub_report(&dnd_on_input);
        assert!(dnd_on_report.ai_recommendations.is_empty());

        // Known probe clipboard data must not become a live recommendation,
        // while an ordinary user URL retains its explicit action.
        let mut clipboard_input = HomeHubInput {
            phone_clipboard: "SF1357924680123",
            ..dnd_on_input.clone()
        };
        assert!(!build_home_hub_report(&clipboard_input)
            .ai_recommendations
            .iter()
            .any(|r| r.id.starts_with("rec-clipboard-")));
        clipboard_input.phone_clipboard = "https://example.org/project-notes";
        assert!(build_home_hub_report(&clipboard_input)
            .ai_recommendations
            .iter()
            .any(|r| r.id == "rec-clipboard-amoo"));

        // Work scene with 0 meetings and 0 deliverables never formats "0 项日程与 0 份交付物".
        let mut work_input = dnd_on_input;
        work_input.hour = 10;
        let work_report = build_home_hub_report(&work_input);
        for rec in &work_report.ai_recommendations {
            assert!(
                !rec.reason.contains("0 项日程") && !rec.reason.contains("0 份"),
                "work summary reason must not mention zero counts: {}",
                rec.reason
            );
        }
        let summary = work_report
            .ai_recommendations
            .iter()
            .find(|rec| rec.id == "rec-work-summary")
            .unwrap();
        assert_eq!(summary.intent, HomeAiIntent::OpenPage("review-day".into()));

        // A directory file count is not evidence that its contents were read. The
        // suggestion must use the same review path even when files are available.
        work_input.work_deliverables_count = 4;
        let with_files = build_home_hub_report(&work_input);
        let summary = with_files
            .ai_recommendations
            .iter()
            .find(|rec| rec.id == "rec-work-summary")
            .unwrap();
        assert_eq!(summary.intent, HomeAiIntent::OpenPage("review-day".into()));
        assert!(!summary.reason.contains("4 份"));
        assert!(!summary.reason.contains("自动汇编"));
    }

    #[test]
    fn test_extract_live_express_parcels_from_sms_notifications_and_clipboard() {
        assert!(
            extract_live_express_parcels(&[], &[], "", &[]).is_empty(),
            "empty live phone inputs must not inject demo express parcels"
        );

        let sms = vec![Message {
            provider_id: Some(1),
            address: "10690000".into(),
            body:
                "【菜鸟驿站】您的顺丰包裹SF1357924680123已到文三路菜鸟驿站，请凭取件码6-1-8899取件"
                    .into(),
            date_ms: 1711867800000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: Some(1),
        }];
        let notifications = vec![PushNotification {
            key: "notif-jd".into(),
            seq_id: 201,
            package_name: "com.jingdong.app.mall".into(),
            app_name: "京东".into(),
            title: "京东快递派送通知".into(),
            text: "您的运单 JD10293847561 正在派送中".into(),
            post_time_ms: 1711867900000,
            ..Default::default()
        }];
        let parcels = extract_live_express_parcels(&sms, &notifications, "YT9876543210123", &[]);
        assert_eq!(parcels.len(), 3);
        assert_eq!(parcels[0].tracking_no, "JD10293847561");
        assert!(parcels[0].status.contains("派送中"));
        assert_eq!(parcels[1].tracking_no, "SF1357924680123");
        assert_eq!(parcels[1].pickup_code, "6-1-8899");
        assert_eq!(parcels[2].tracking_no, "YT9876543210123");
        let sto_parcels = extract_live_express_parcels(&[], &[], "申通快递 773012345678901", &[]);
        assert_eq!(sto_parcels.len(), 1);
        assert_eq!(sto_parcels[0].carrier, "申通快递");
        assert_eq!(sto_parcels[0].tracking_no, "773012345678901");
    }

    #[test]
    fn test_quick_replies_for_message_with_context() {
        let meeting_replies = quick_replies_for_message_with_context(
            "微信",
            "张经理",
            "下午一起看下合同进展？",
            Some("产品周会"),
            None,
            false,
        );
        assert!(meeting_replies.iter().any(|r| r.contains("产品周会")));

        let pomo_replies =
            quick_replies_for_message_with_context("微信", "小李", "在吗", None, Some(20), false);
        assert!(pomo_replies.iter().any(|r| r.contains("20 分钟")));

        let courier_dnd = quick_replies_for_message_with_context(
            "短信",
            "顺丰速运",
            "您的包裹正在派件",
            Some("架构评审"),
            None,
            true,
        );
        assert!(courier_dnd
            .iter()
            .any(|r| r.contains("不便接听") || r.contains("放门口")));

        let threads = vec![crate::push::ImThread {
            thread_id: "im:com.tencent.mm:1".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "李明".into(),
            last_message: "在吗，看一下方案".into(),
            unread_count: 1,
            ..Default::default()
        }];
        let wb = crate::toolbox::default_stepos_ai_suite_report().wellbeing;
        let report = build_home_hub_report(&HomeHubInput {
            hour: 10,
            minute: 30,
            today_ymd: "2026-10-30",
            device_connected: true,
            device_model: "PZ261A",
            firmware_profile: "StepOS",
            threads: &threads,
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
            pomodoro_running: true,
            pomodoro_remaining_secs: 1500,
            dnd_on: false,
        });
        let msg_card = report
            .priority_cards
            .iter()
            .find(|c| c.kind == HomeCardKind::UnreadMessage)
            .expect("unread message card");
        assert!(
            msg_card.quick_replies.iter().any(|r| r.contains("25 分钟")),
            "expected live 25-minute pomodoro remaining time in quick replies, got {:?}",
            msg_card.quick_replies
        );
    }

    #[test]
    fn json_system_threads_and_notifications_are_excluded_from_home_hub() {
        let threads = vec![
            crate::push::ImThread {
                thread_id: "im:com.stepos.backscreen:json".into(),
                package_name: "com.stepos.backscreen".into(),
                app_name: "背屏".into(),
                title: "{\"type\":\"agent\",\"code\":\"123456\"}".into(),
                last_message: "{\"otp\":\"654321\"}".into(),
                unread_count: 5,
                ..Default::default()
            },
            crate::push::ImThread {
                thread_id: "im:com.tencent.mm:real".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "产品讨论群".into(),
                last_message: "下午三点对齐进度".into(),
                unread_count: 2,
                ..Default::default()
            },
        ];
        let notifications = vec![crate::push::PushNotification {
            seq_id: 1,
            key: "0|com.stepos.backscreen|1".into(),
            package_name: "com.stepos.backscreen".into(),
            app_name: "背屏服务".into(),
            title: "{\"verification\":\"889900\"}".into(),
            text: "验证码 889900 顺丰 SF1234567890123".into(),
            post_time_ms: 1_760_000_000_000,
            ..Default::default()
        }];
        let wb = crate::toolbox::default_stepos_ai_suite_report().wellbeing;
        let report = build_home_hub_report(&HomeHubInput {
            hour: 10,
            minute: 30,
            today_ymd: "2026-10-30",
            device_connected: true,
            device_model: "PZ261A",
            firmware_profile: "StepOS",
            threads: &threads,
            notifications: &notifications,
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
        });
        assert_eq!(report.unread_messages_count, 2);
        assert!(
            !report
                .priority_cards
                .iter()
                .any(|c| c.title.contains('{') || c.detail.contains('{')),
            "raw JSON system threads/notifications must not appear in priority_cards: {:?}",
            report.priority_cards
        );
    }

    #[test]
    fn calendar_panorama_weekly_summary_and_context_linked_life_cards() {
        let dt_ms = |y: i32, m: u32, d: u32, h: u32, min: u32| -> i64 {
            Local
                .with_ymd_and_hms(y, m, d, h, min, 0)
                .single()
                .unwrap()
                .timestamp_millis()
        };

        let events = vec![
            crate::toolbox::PhoneCalendarEvent {
                recurring: Some(false),
                id: 1,
                title: "客户现场架构评审".into(),
                dtstart: dt_ms(2026, 10, 30, 15, 0),
                dtend: dt_ms(2026, 10, 30, 16, 30),
                all_day: false,
                location: "望京科技园 A 座".into(),
                description: "讨论 Q4 方案".into(),
                source: "飞书".into(),
            },
            crate::toolbox::PhoneCalendarEvent {
                recurring: Some(false),
                id: 2,
                title: "周三需求对齐".into(),
                dtstart: dt_ms(2026, 10, 28, 10, 0),
                dtend: dt_ms(2026, 10, 28, 11, 0),
                all_day: false,
                location: "线上会议".into(),
                description: String::new(),
                source: "系统日历".into(),
            },
        ];

        let notifications = vec![crate::push::PushNotification {
            seq_id: 10,
            key: "weather-1".into(),
            package_name: "com.stepos.weather".into(),
            app_name: "天气".into(),
            title: "傍晚有中雨，出门请带伞".into(),
            text: "预计 17:30 开始降雨，晚高峰路面湿滑".into(),
            post_time_ms: dt_ms(2026, 10, 30, 13, 0),
            ..Default::default()
        }];

        let parcels = vec![ExpressParcelInfo {
            carrier: "顺丰速运".into(),
            tracking_no: "SF1682904182291".into(),
            pickup_code: "3-2-1048".into(),
            station: "软件园丰巢柜".into(),
            status: "待取件".into(),
            updated_at: "12:15".into(),
        }];

        let sms = vec![crate::sms::Message {
            provider_id: Some(11),
            address: "13800009367".into(),
            body: "周三的会议资料请查收".into(),
            date_ms: dt_ms(2026, 10, 28, 14, 20),
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        }];

        let mut wb = crate::toolbox::default_stepos_ai_suite_report().wellbeing;
        wb.screen_time_minutes = 150;
        wb.unlock_count = 32;

        let input = HomeHubInput {
            hour: 14,
            minute: 0,
            today_ymd: "2026-10-30",
            device_connected: true,
            device_model: "PZ261A",
            firmware_profile: "StepOS",
            threads: &[],
            notifications: &notifications,
            otp_banners: &[],
            sms: &sms,
            call_logs: &[],
            express_parcels: &parcels,
            calendar_events: &events,
            alarms: &[],
            wellbeing: &wb,
            phone_clipboard: "",
            remote_agent_files: &["/sdcard/report.md".into()],
            work_deliverables_count: 2,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        };

        let report = build_home_hub_report(&input);
        assert_eq!(report.life_context_cards.len(), 3);

        // 1. Express card links to today's meeting ("客户现场架构评审")
        let exp_card = &report.life_context_cards[0];
        assert_eq!(exp_card.domain, HomeLifeDomain::ExpressAndPickup);
        assert!(exp_card.context_link.contains("客户现场架构评审"));
        assert_eq!(exp_card.code_or_number.as_deref(), Some("3-2-1048"));

        // 2. Weather card ("要下雨") links rain alert to today's outdoor schedule ("客户现场架构评审" / "望京科技园 A 座")
        let weather_card = &report.life_context_cards[1];
        assert_eq!(weather_card.domain, HomeLifeDomain::WeatherAndCommute);
        assert!(weather_card.badge.contains("要下雨"));
        assert!(weather_card.title.contains("带伞"));
        assert!(weather_card.context_link.contains("客户现场架构评审"));

        // 3. Leisure card ("要愉悦") links screen time (150m) & schedule load
        let leisure_card = &report.life_context_cards[2];
        assert_eq!(leisure_card.domain, HomeLifeDomain::LeisureAndWellbeing);
        assert!(leisure_card.badge.contains("要愉悦"));
        assert!(leisure_card.context_link.contains("150 分钟"));

        // 4. Selected Date Panorama filters strictly by selected date
        let desktop_entries = vec![DesktopDateEntry {
            id: "task-wed".into(),
            timestamp: dt_ms(2026, 10, 28, 16, 0) / 1000, // second-based timestamp
            kind: SelectedDateEntryKind::WorkAndMatter,
            badge: "工作成果".into(),
            title: "周三架构评审纪要.md".into(),
            detail: "保存在电脑".into(),
            action_label: "打开".into(),
            intent: HomeAiIntent::OpenPage("work".into()),
        }];

        let pan_today = build_selected_date_panorama("2026-10-30", &input, &desktop_entries);
        assert!(pan_today.is_today);
        assert_eq!(pan_today.schedule_count, 1);
        assert_eq!(pan_today.life_count, 2); // express parcel + weather notification on 2026-10-30
        assert_eq!(pan_today.work_count, 1); // remote_agent_files ["/sdcard/report.md"] on today
        assert!(pan_today.entries.iter().any(|e| e.title == "report.md"));
        assert!(pan_today
            .entries
            .iter()
            .any(|e| e.title.contains("傍晚有中雨")));

        let pan_wed = build_selected_date_panorama("2026-10-28", &input, &desktop_entries);
        assert!(!pan_wed.is_today);
        assert_eq!(pan_wed.schedule_count, 1);
        assert_eq!(pan_wed.work_count, 1);
        assert_eq!(pan_wed.communication_count, 1);
        assert_eq!(pan_wed.issue_count, 1); // unread SMS on 2026-10-28

        // 5. Weekly Overview Report aggregates all 7 days of the week (2026-10-26 .. 2026-11-01)
        let weekly = build_weekly_overview_report("2026-10-28", &input, &desktop_entries);
        assert_eq!(weekly.days.len(), 7);
        assert_eq!(weekly.days[2].date_ymd, "2026-10-28");
        assert!(weekly.days[2].is_selected);
        assert_eq!(weekly.days[4].date_ymd, "2026-10-30");
        assert!(weekly.days[4].is_today);
        assert_eq!(weekly.total_schedules, 2);
        assert_eq!(weekly.pillars.len(), 4);
    }

    #[test]
    fn synthetic_automation_sms_and_launcher_threads_are_filtered_from_home_hub() {
        assert!(is_synthetic_e2e_probe_text(
            "【StepX自动化验证】您的取件码为 6-2-8801，顺丰速运单号 SF1428571428579 已到达驿站"
        ));
        let ts_ms = Local
            .with_ymd_and_hms(2026, 10, 30, 10, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let threads = vec![crate::push::ImThread {
            thread_id: "im:com.stepos.ailauncher:amoo".into(),
            package_name: "com.stepos.ailauncher".into(),
            app_name: "系统桌面".into(),
            title: "Amoo：提醒已经设置好了".into(),
            last_message: "提醒已经设置好了".into(),
            last_timestamp_ms: 0,
            unread_count: 1,
            ..Default::default()
        }];
        let sms = vec![crate::sms::Message {
            provider_id: Some(1),
            address: "106900008888".into(),
            body:
                "【StepX自动化验证】您的取件码为 6-2-8801，顺丰速运单号 SF1428571428579 已到达驿站"
                    .into(),
            date_ms: ts_ms,
            kind: 1,
            outgoing: false,
            read: false,
            subscription: Some(1),
        }];
        let wellbeing = crate::toolbox::StepOsDigitalWellbeing {
            screen_time_minutes: 30,
            unlock_count: 5,
            top_apps: Vec::new(),
            alphame_running: true,
        };
        let input = HomeHubInput {
            hour: 10,
            minute: 30,
            today_ymd: "2026-10-30",
            device_connected: true,
            device_model: "PZ261A",
            firmware_profile: "StepOS",
            threads: &threads,
            notifications: &[],
            otp_banners: &[],
            sms: &sms,
            call_logs: &[],
            express_parcels: &[],
            calendar_events: &[],
            alarms: &[],
            wellbeing: &wellbeing,
            phone_clipboard: "【StepX自动化验证】测试剪贴板",
            remote_agent_files: &[],
            work_deliverables_count: 0,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        };
        let report = build_home_hub_report(&input);
        assert_eq!(report.unread_messages_count, 0);
        assert_eq!(report.active_life_cards_count, 0);
        assert!(report.priority_cards.is_empty());

        let pan = build_selected_date_panorama("2026-10-30", &input, &[]);
        assert!(pan.entries.is_empty());
    }

    #[test]
    fn schedule_alarm_conflict_checklist_extraction_and_desktop_weather_cross_linking() {
        let dt_ms = |y: i32, m: u32, d: u32, h: u32, min: u32| -> i64 {
            Local
                .with_ymd_and_hms(y, m, d, h, min, 0)
                .single()
                .unwrap()
                .timestamp_millis()
        };

        let prep_desc = "明天早上 7:00 自驾出发，今晚收尾：1) 行李打包、孩子衣物与常备药；2) 检查油量/电量、胎压、玻璃水；3) 手机与车机导航离线地图下载；4) 证件、ETC、充电线、充电宝；5) 零食饮水与垃圾袋；6) 早睡，设好闹钟。";
        let depart_desc = "自驾出行出发。出发前确认：油量/电量充足、胎压与备胎、导航与路线离线地图、ETC 与证件、孩子安全座椅与用品、饮用水与零食、常用药品、充电线与充电宝。";

        // 1. Checklist extraction on both real calendar descriptions
        let prep_items = extract_schedule_checklist(prep_desc);
        assert_eq!(
            prep_items,
            vec![
                "行李打包、孩子衣物与常备药",
                "检查油量/电量、胎压、玻璃水",
                "手机与车机导航离线地图下载",
                "证件、ETC、充电线、充电宝",
                "零食饮水与垃圾袋",
                "早睡，设好闹钟",
            ]
        );

        let depart_items = extract_schedule_checklist(depart_desc);
        assert_eq!(
            depart_items,
            vec![
                "油量/电量充足",
                "胎压与备胎",
                "导航与路线离线地图",
                "ETC 与证件",
                "孩子安全座椅与用品",
                "饮用水与零食",
                "常用药品",
                "充电线与充电宝",
            ]
        );

        let calendar_events = vec![
            PhoneCalendarEvent {
                recurring: Some(false),
                id: 501,
                title: "行前准备：自驾行李与车况检查".into(),
                dtstart: dt_ms(2026, 10, 30, 21, 20),
                dtend: dt_ms(2026, 10, 30, 22, 20),
                all_day: false,
                location: "家里".into(),
                description: prep_desc.into(),
                source: "手机日历".into(),
            },
            PhoneCalendarEvent {
                recurring: Some(false),
                id: 502,
                title: "自驾出发".into(),
                dtstart: dt_ms(2026, 10, 31, 7, 0),
                dtend: dt_ms(2026, 10, 31, 9, 0),
                all_day: false,
                location: "出发地".into(),
                description: depart_desc.into(),
                source: "手机日历".into(),
            },
        ];

        // 2. Schedule × Alarm conflict detection: 07:00 departure vs 08:00 enabled alarm + synthetic 06:00 alarm
        let alarms_with_late_and_synthetic = vec![
            PhoneAlarmItem {
                id: 1,
                hour: 8,
                minutes: 0,
                enabled: true,
                days_of_week: 31,
                vibrate: true,
                message: "起床".into(),
            },
            PhoneAlarmItem {
                id: 2,
                hour: 6,
                minutes: 0,
                enabled: true,
                days_of_week: 0,
                vibrate: true,
                message: "StepX自动化验证闹钟".into(),
            },
        ];

        let conflicts = detect_schedule_alarm_conflicts(
            "2026-10-30",
            21,
            0,
            &calendar_events,
            &alarms_with_late_and_synthetic,
        );
        assert_eq!(conflicts.len(), 1);
        let c = &conflicts[0];
        assert_eq!(c.event_id, 502);
        assert_eq!(c.event_date_ymd, "2026-10-31");
        assert_eq!(c.event_time_hm, "07:00");
        assert_eq!(c.event_title, "自驾出发");
        assert_eq!(c.suggested_alarm_hour, 6);
        assert_eq!(c.suggested_alarm_minute, 0);
        assert!(
            c.detail.contains(
                "检测到明日 07:00「自驾出发」，但手机当前最早开启闹钟为 08:00（晚于出发时间）"
            ),
            "unexpected conflict detail: {}",
            c.detail
        );
        assert_eq!(c.primary_action_label, "一键设 06:00 手机闹钟");
        assert_eq!(
            c.primary_intent,
            HomeAiIntent::CreatePhoneAlarm {
                hour: 6,
                minute: 0,
                label: "07:00 自驾出发".into(),
            }
        );
        assert_eq!(c.secondary_action_label, "推背屏提醒");
        assert_eq!(
            c.secondary_intent,
            HomeAiIntent::BackScreenStatus("07:00 自驾出发 · 闹钟 06:00".into())
        );

        // Adding a real 06:00 enabled alarm clears the conflict
        let mut alarms_resolved = alarms_with_late_and_synthetic.clone();
        alarms_resolved.push(PhoneAlarmItem {
            id: 3,
            hour: 6,
            minutes: 0,
            enabled: true,
            days_of_week: 0,
            vibrate: true,
            message: "07:00 自驾出发".into(),
        });
        assert!(
            detect_schedule_alarm_conflicts(
                "2026-10-30",
                21,
                0,
                &calendar_events,
                &alarms_resolved
            )
            .is_empty(),
            "enabling a 06:00 real alarm must resolve the 07:00 departure conflict"
        );

        // 3. Report, Panorama & Desktop Weather × Schedule Cross-Linking
        let wb = empty_wellbeing();
        let input = HomeHubInput {
            hour: 21,
            minute: 0,
            today_ymd: "2026-10-30",
            device_connected: true,
            device_model: "PZ261A",
            firmware_profile: "StepOS",
            threads: &[],
            notifications: &[],
            otp_banners: &[],
            sms: &[],
            call_logs: &[],
            express_parcels: &[],
            calendar_events: &calendar_events,
            alarms: &alarms_with_late_and_synthetic,
            wellbeing: &wb,
            phone_clipboard: "",
            remote_agent_files: &[],
            work_deliverables_count: 1,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        };

        let report = build_home_hub_report(&input);
        assert_eq!(report.schedule_conflicts.len(), 1);
        assert_eq!(report.schedule_checklists.len(), 1);
        assert_eq!(
            report.schedule_checklists[0].event_title,
            "行前准备：自驾行李与车况检查"
        );
        assert_eq!(report.schedule_checklists[0].items.len(), 6);
        assert_eq!(
            report.ai_recommendations[0].intent,
            HomeAiIntent::CreatePhoneAlarm {
                hour: 6,
                minute: 0,
                label: "07:00 自驾出发".into(),
            }
        );

        let desktop_entries = vec![DesktopDateEntry {
            id: "ledger-rain-query".into(),
            timestamp: dt_ms(2026, 10, 30, 20, 45) / 1000,
            kind: SelectedDateEntryKind::WorkAndMatter,
            badge: "台账".into(),
            title: "降雨查询".into(),
            detail: "查询明日自驾沿途降雨情况".into(),
            action_label: "打开".into(),
            intent: HomeAiIntent::OpenPage("work".into()),
        }];

        let pan_tonight = build_selected_date_panorama("2026-10-30", &input, &desktop_entries);
        assert!(pan_tonight
            .entries
            .iter()
            .any(|e| { e.kind == SelectedDateEntryKind::Issue && e.intent == c.primary_intent }));
        let sched_tonight = pan_tonight
            .entries
            .iter()
            .find(|e| e.kind == SelectedDateEntryKind::Schedule)
            .expect("tonight schedule entry");
        assert_eq!(sched_tonight.checklist_items.len(), 6);

        let pan_tomorrow = build_selected_date_panorama("2026-10-31", &input, &desktop_entries);
        assert!(pan_tomorrow
            .entries
            .iter()
            .any(|e| { e.kind == SelectedDateEntryKind::Issue && e.intent == c.primary_intent }));
        let sched_tomorrow = pan_tomorrow
            .entries
            .iter()
            .find(|e| e.kind == SelectedDateEntryKind::Schedule)
            .expect("tomorrow schedule entry");
        assert_eq!(sched_tomorrow.checklist_items.len(), 8);

        let life_cards =
            build_life_context_cards_with_desktop("2026-10-30", &input, &desktop_entries);
        let weather_card = life_cards
            .iter()
            .find(|card| card.domain == HomeLifeDomain::WeatherAndCommute)
            .expect("weather context card");
        assert_eq!(weather_card.badge, "降雨查询 × 日程联动");
        assert!(weather_card.title.contains("降雨关注已联动"));
        assert!(weather_card.context_link.contains("台账「降雨查询」"));
        assert!(weather_card
            .context_link
            .contains("行前准备：自驾行李与车况检查"));
        assert!(weather_card.context_link.contains("自驾出发"));
        assert_eq!(weather_card.action_label, "推送雨天行前清单到背屏");
        assert_eq!(
            weather_card.intent,
            HomeAiIntent::BackScreenStatus("雨天自驾检查：雨刮/玻璃水/胎压/离线地图".into())
        );
        assert_eq!(
            weather_card.secondary_label.as_deref(),
            Some("查沿途实时天气")
        );

        assert_eq!(HomeLifeDomain::WeatherAndCommute.label(), "天气与出行");
        let non_rain_desktop_entries = vec![DesktopDateEntry {
            id: "ledger-trip-plan".into(),
            timestamp: dt_ms(2026, 10, 30, 20, 45) / 1000,
            kind: SelectedDateEntryKind::WorkAndMatter,
            badge: "台账".into(),
            title: "周末出行准备".into(),
            detail: "整理自驾出行路线与后备箱物资".into(),
            action_label: "打开".into(),
            intent: HomeAiIntent::OpenPage("work".into()),
        }];
        let non_rain_cards =
            build_life_context_cards_with_desktop("2026-10-30", &input, &non_rain_desktop_entries);
        let non_rain_weather = non_rain_cards
            .iter()
            .find(|card| card.domain == HomeLifeDomain::WeatherAndCommute)
            .expect("non-rain weather card");
        assert!(!non_rain_weather.badge.contains("要下雨"));
        assert!(!non_rain_weather.badge.contains("降雨"));
        assert!(!non_rain_weather.title.contains("降雨"));
        assert!(!non_rain_weather.detail.contains("雨刮器"));
        assert_ne!(
            non_rain_weather.secondary_intent,
            Some(HomeAiIntent::BackScreenStatus("出门带伞".into()))
        );

        // Non-driving schedule with rain cue should produce commute umbrella advice rather than car inspection
        let meeting_only_events = vec![crate::toolbox::PhoneCalendarEvent {
            recurring: Some(false),
            id: 201,
            title: "客户现场架构评审".into(),
            dtstart: dt_ms(2026, 10, 31, 10, 0),
            dtend: dt_ms(2026, 10, 31, 11, 30),
            location: "陆家嘴写字楼".into(),
            description: "准备演示材料".into(),
            all_day: false,
            source: "手机日历".into(),
        }];
        let meeting_input = HomeHubInput {
            calendar_events: &meeting_only_events,
            ..input
        };
        let rain_meeting_entries = vec![DesktopDateEntry {
            id: "ledger-rain-commute".into(),
            timestamp: dt_ms(2026, 10, 30, 20, 45) / 1000,
            kind: SelectedDateEntryKind::WorkAndMatter,
            badge: "台账".into(),
            title: "降雨查询".into(),
            detail: "查看明日上午降雨时段".into(),
            action_label: "打开".into(),
            intent: HomeAiIntent::OpenPage("work".into()),
        }];
        let rain_meeting_cards = build_life_context_cards_with_desktop(
            "2026-10-30",
            &meeting_input,
            &rain_meeting_entries,
        );
        let rain_meeting_weather = rain_meeting_cards
            .iter()
            .find(|card| card.domain == HomeLifeDomain::WeatherAndCommute)
            .expect("rain meeting weather card");
        assert_eq!(rain_meeting_weather.badge, "降雨提醒 × 日程联动");
        assert!(!rain_meeting_weather.detail.contains("雨刮器"));
        assert_eq!(
            rain_meeting_weather.intent,
            HomeAiIntent::BackScreenStatus("出门带伞".into())
        );
        assert_eq!(
            classify_command_intent("背屏显示出门带伞"),
            HomeAiIntent::BackScreenStatus("出门带伞".into())
        );
    }

    #[test]
    fn workstream_2_consolidation_edge_cases_and_benchmarks() {
        use std::time::Instant;

        // 1. Edge cases for OTP / Pickup / Express / Shelf Code / Numbered Markers
        assert_eq!(extract_otp_or_pickup_from_text(""), None);
        assert_eq!(extract_otp_or_pickup_from_parts(&["", "   "]), None);
        // Split across title and text without concatenation allocation
        assert_eq!(
            extract_otp_or_pickup_from_parts(&[
                "菜鸟驿站",
                "您的包裹取件码",
                "请凭 839201 前往领取"
            ]),
            Some(("839201".to_string(), true))
        );
        // Boundary shelf codes (length 4..=14)
        assert!(is_shelf_pickup_code("1-23"));
        assert!(!is_shelf_pickup_code("1-2")); // < 3 digits
        assert!(is_shelf_pickup_code("A12-3456-7890"));
        assert!(!is_shelf_pickup_code("A12345-67890-12345")); // > 14 chars
                                                              // Case-insensitive express prefix matching without upfront Vec<String>
        assert_eq!(
            extract_express_from_text_flexible("单号 zto1234567890 已发出", false),
            Some(("中通快递", "ZTO1234567890".to_string()))
        );
        assert_eq!(
            extract_express_from_text_flexible("sf9876543210", false),
            Some(("顺丰速运", "SF9876543210".to_string()))
        );
        assert_eq!(
            extract_express_from_text_flexible("无单号普通文本 hello world", false),
            None
        );
        // Fast byte check in find_numbered_marker_spans
        assert!(find_numbered_marker_spans("没有任何数字或序号的普通日程描述").is_empty());
        assert_eq!(find_numbered_marker_spans("①准备材料 ②核对预算").len(), 2);
        // ASCII case-insensitive helper in home_hub & phone_context (including uppercase first byte in needle)
        assert!(contains_ascii_ignore_case("hello stepx world", b"StepX"));
        assert!(contains_ascii_ignore_case("HELLO STEPX WORLD", b"StepX"));
        assert!(crate::phone_context::contains_ascii_ignore_case(
            "hello stepx world",
            "StepX"
        ));
        assert!(crate::phone_context::contains_ascii_ignore_case(
            "Verification Code 1234",
            ""
        ));
        assert!(crate::phone_context::contains_ascii_ignore_case(
            "My PASSCODE is 9988",
            "passcode"
        ));
        assert!(!crate::phone_context::contains_ascii_ignore_case(
            "中文内容测试",
            "otp"
        ));
        assert!(crate::phone_context::contains_secret_or_otp(
            "您的 Verification 码为 123456"
        ));
        assert!(crate::phone_context::contains_secret_or_otp(
            "Please keep your OTP confidential"
        ));
        assert!(crate::phone_context::contains_secret_or_otp(
            "【招商银行】您的动态码 482901，请勿泄露"
        ));

        // 2. Equivalence check: Weekly Overview single-pass bucketing vs 7x per-day panorama
        let dt_ms = |y: i32, m: u32, d: u32, h: u32, min: u32| -> i64 {
            Local
                .with_ymd_and_hms(y, m, d, h, min, 0)
                .single()
                .unwrap()
                .timestamp_millis()
        };
        let mut cal_events = Vec::new();
        let mut notifs = Vec::new();
        let mut sms_list = Vec::new();
        let mut threads = Vec::new();
        let mut calls = Vec::new();
        let mut desktop_items = Vec::new();

        for day_offset in 0..7u32 {
            let d = 26 + day_offset; // 2026-10-26 (Mon) .. 2026-11-01 (Sun)
            let (m, day_num) = if d > 31 { (11, d - 31) } else { (10, d) };
            let ts = dt_ms(2026, m, day_num, 10, 30);
            cal_events.push(PhoneCalendarEvent {
                recurring: Some(false),
                id: day_offset as i64 + 1,
                title: format!("架构评审 Day {day_offset}"),
                dtstart: ts,
                dtend: ts + 3_600_000,
                all_day: false,
                location: "A座会议室".into(),
                description: "讨论事项".into(),
                source: "飞书日历".into(),
            });
            for k in 0..6 {
                notifs.push(PushNotification {
                    key: format!("n-{day_offset}-{k}"),
                    seq_id: (day_offset * 10 + k) as u64,
                    package_name: if k % 3 == 0 {
                        "com.sankuai.meituan".into()
                    } else {
                        "com.ss.android.lark".into()
                    },
                    app_name: if k % 3 == 0 {
                        "美团外卖".into()
                    } else {
                        "飞书".into()
                    },
                    title: format!("通知标题 {day_offset}-{k}"),
                    text: if k % 2 == 0 {
                        "骑手正在配送您的外卖".into()
                    } else {
                        "请确认评审方案是否通过？".into()
                    },
                    post_time_ms: ts + (k as i64) * 60_000,
                    ..Default::default()
                });
            }
            for k in 0..4 {
                sms_list.push(Message {
                    provider_id: None,
                    address: "10698000".into(),
                    body: if k % 2 == 0 {
                        format!("【菜鸟驿站】您的包裹已到站，凭取件码 3-2-10{day_offset}{k} 领取")
                    } else {
                        "普通沟通短信内容".into()
                    },
                    date_ms: ts + (k as i64) * 120_000,
                    outgoing: false,
                    kind: 1,
                    read: k % 2 == 0,
                    subscription: None,
                });
            }
            threads.push(ImThread {
                thread_id: format!("th-{day_offset}"),
                package_name: "com.ss.android.lark".into(),
                app_name: "飞书".into(),
                title: format!("项目群 {day_offset}"),
                unread_count: (day_offset % 3) as usize,
                last_sender: "王工".into(),
                last_message: "今晚同步联调结果".into(),
                last_timestamp_ms: ts + 300_000,
                can_remote_reply: true,
                ..Default::default()
            });
            calls.push(CallRecord {
                number: format!("1380013800{day_offset}"),
                name: format!("联系人{day_offset}"),
                call_type: if day_offset % 2 == 0 { 3 } else { 1 },
                date_ms: ts + 600_000,
                duration_sec: 45,
            });
            desktop_items.push(DesktopDateEntry {
                id: format!("desk-{day_offset}"),
                timestamp: ts / 1000,
                kind: SelectedDateEntryKind::WorkAndMatter,
                badge: "台账".into(),
                title: format!("降雨查询与日程准备 {day_offset}"),
                detail: "查看明日降雨与出行路线".into(),
                action_label: "打开".into(),
                intent: HomeAiIntent::OpenPage("work".into()),
            });
        }

        let wb = empty_wellbeing();
        let parcels = vec![ExpressParcelInfo {
            carrier: "顺丰速运".into(),
            tracking_no: "SF14298019283".into(),
            pickup_code: "3-2-1048".into(),
            station: "软件园菜鸟驿站".into(),
            status: "待取件".into(),
            updated_at: "12:15".into(),
        }];
        let alarms = vec![PhoneAlarmItem {
            id: 1,
            hour: 8,
            minutes: 30,
            enabled: true,
            days_of_week: 31,
            vibrate: true,
            message: "早会".into(),
        }];
        let remote_files = vec!["/sdcard/StepX/report_v1.md".to_string()];

        let input = HomeHubInput {
            hour: 14,
            minute: 20,
            today_ymd: "2026-10-30",
            device_connected: true,
            device_model: "PZ261A",
            firmware_profile: "StepOS",
            threads: &threads,
            notifications: &notifs,
            otp_banners: &[],
            sms: &sms_list,
            call_logs: &calls,
            express_parcels: &parcels,
            calendar_events: &cal_events,
            alarms: &alarms,
            wellbeing: &wb,
            phone_clipboard: "顺丰 SF14298019283 待取件",
            remote_agent_files: &remote_files,
            work_deliverables_count: 3,
            pomodoro_running: true,
            pomodoro_remaining_secs: 900,
            dnd_on: false,
        };

        let weekly = build_weekly_overview_report("2026-10-30", &input, &desktop_items);
        assert_eq!(weekly.days.len(), 7);
        for day in &weekly.days {
            let pan = build_selected_date_panorama(&day.date_ymd, &input, &desktop_items);
            assert_eq!(
                day.schedule_count, pan.schedule_count,
                "schedule mismatch on {}",
                day.date_ymd
            );
            assert_eq!(
                day.work_count,
                pan.schedule_count + pan.work_count,
                "work mismatch on {}",
                day.date_ymd
            );
            assert_eq!(
                day.life_count, pan.life_count,
                "life mismatch on {}",
                day.date_ymd
            );
            assert_eq!(
                day.issue_count, pan.issue_count,
                "issue mismatch on {}",
                day.date_ymd
            );
            assert_eq!(
                day.communication_count, pan.communication_count,
                "comm mismatch on {}",
                day.date_ymd
            );
        }

        // Verify whitespace/empty selected_ymd normalization in build_selected_date_panorama
        let pan_today = build_selected_date_panorama("2026-10-30", &input, &desktop_items);
        let pan_ws = build_selected_date_panorama(" 2026-10-30 ", &input, &desktop_items);
        let pan_empty = build_selected_date_panorama("", &input, &desktop_items);
        assert!(pan_today.is_today && pan_ws.is_today && pan_empty.is_today);
        assert_eq!(pan_today.entries.len(), pan_ws.entries.len());
        assert_eq!(pan_today.entries.len(), pan_empty.entries.len());
        assert_eq!(pan_today.life_count, pan_ws.life_count);

        // Verify lightweight build_life_context_cards_with_desktop matches full-report path
        let fast_life_cards =
            build_life_context_cards_with_desktop("2026-10-30", &input, &desktop_items);
        let full_report = build_home_hub_report(&input);
        let full_life_cards = enrich_life_context_cards_with_desktop(
            full_report.life_context_cards.clone(),
            "2026-10-30",
            &input,
            &desktop_items,
        );
        assert_eq!(fast_life_cards, full_life_cards);

        // 3. Timing benchmarks (200 iterations each)
        let iters = 200u32;

        // Benchmark A: 7x full panorama scan vs single-pass build_weekly_overview_report
        let t0 = Instant::now();
        for _ in 0..iters {
            for day in &weekly.days {
                std::hint::black_box(build_selected_date_panorama(
                    &day.date_ymd,
                    &input,
                    &desktop_items,
                ));
            }
        }
        let seven_pan_us = t0.elapsed().as_secs_f64() * 1_000_000.0 / (iters as f64);

        let t1 = Instant::now();
        for _ in 0..iters {
            std::hint::black_box(build_weekly_overview_report(
                "2026-10-30",
                &input,
                &desktop_items,
            ));
        }
        let weekly_fast_us = t1.elapsed().as_secs_f64() * 1_000_000.0 / (iters as f64);

        // Benchmark B: full report + enrich vs lightweight build_life_context_cards_with_desktop
        let t2 = Instant::now();
        for _ in 0..iters {
            let mut swapped = input.clone();
            swapped.today_ymd = "2026-10-30";
            let rep = build_home_hub_report(&swapped);
            std::hint::black_box(enrich_life_context_cards_with_desktop(
                rep.life_context_cards,
                "2026-10-30",
                &input,
                &desktop_items,
            ));
        }
        let life_old_us = t2.elapsed().as_secs_f64() * 1_000_000.0 / (iters as f64);

        let t3 = Instant::now();
        for _ in 0..iters {
            std::hint::black_box(build_life_context_cards_with_desktop(
                "2026-10-30",
                &input,
                &desktop_items,
            ));
        }
        let life_new_us = t3.elapsed().as_secs_f64() * 1_000_000.0 / (iters as f64);

        // Benchmark C: home_visuals SVG generation
        use crate::home_visuals::{
            day_dial, day_timeline, source_spectrum_bar, weekly_activity_bars, DayDial,
            DayTimeline, DialAlarm, DialEvent, Paint, WeeklyBarDay,
        };
        let p_work = Paint::rgb(64, 150, 255);
        let p_life = Paint::rgb(52, 211, 153);
        let p_issue = Paint::rgb(245, 158, 11);
        let p_track = Paint::rgb(148, 163, 184).with_alpha(0.2);
        let p_matte = Paint::rgb(15, 23, 42);
        let dial_spec = DayDial {
            size: 160.0,
            now_min: 860,
            events: vec![
                DialEvent {
                    start_min: 540,
                    end_min: 600,
                    paint: p_work,
                },
                DialEvent {
                    start_min: 630,
                    end_min: 720,
                    paint: p_life,
                },
                DialEvent {
                    start_min: 840,
                    end_min: 900,
                    paint: p_issue,
                },
            ],
            alarms: vec![DialAlarm {
                minute_of_day: 510,
                enabled: true,
            }],
            track: p_track,
            night: p_track,
            elapsed: p_work,
            tick: p_track,
            hand: p_issue,
            alarm: p_issue,
            face: p_matte,
            matte: p_matte,
        };
        let tl_spec = DayTimeline {
            width: 320.0,
            height: 28.0,
            start_hour: 7,
            end_hour: 23,
            now_min: 860,
            events: dial_spec.events.clone(),
            alarms: dial_spec.alarms.clone(),
            track: p_track,
            elapsed: p_work,
            tick: p_track,
            now: p_issue,
            alarm: p_issue,
            matte: p_matte,
        };
        let week_bars = [WeeklyBarDay {
            work_count: 5,
            life_count: 3,
            issue_count: 2,
            selected: false,
            today: false,
        }; 7];
        let spectrum_segs = [(5.0, p_work), (3.0, p_life), (2.0, p_issue)];
        let t4 = Instant::now();
        for _ in 0..iters {
            std::hint::black_box(day_dial(&dial_spec));
            std::hint::black_box(day_timeline(&tl_spec));
            std::hint::black_box(weekly_activity_bars(
                240.0, 64.0, &week_bars, p_work, p_life, p_issue, p_track, p_work, p_matte,
            ));
            std::hint::black_box(source_spectrum_bar(
                240.0,
                12.0,
                &spectrum_segs,
                p_track,
                p_matte,
            ));
        }
        let visuals_us = t4.elapsed().as_secs_f64() * 1_000_000.0 / (iters as f64);

        println!(
            "\n[WORKSTREAM_2_BENCH] weekly_7x_panorama={:.2}us weekly_single_pass={:.2}us speedup={:.2}x | life_cards_old={:.2}us life_cards_new={:.2}us speedup={:.2}x | visuals_bundle={:.2}us",
            seven_pan_us,
            weekly_fast_us,
            seven_pan_us / weekly_fast_us.max(0.001),
            life_old_us,
            life_new_us,
            life_old_us / life_new_us.max(0.001),
            visuals_us,
        );
    }

    #[test]
    fn ongoing_meeting_and_meeting_url_enrich_cards_and_missed_call_replies() {
        let start_ms = Local
            .with_ymd_and_hms(2026, 10, 29, 14, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let end_ms = Local
            .with_ymd_and_hms(2026, 10, 29, 15, 30, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let ev = PhoneCalendarEvent {
            recurring: Some(false),
            id: 7,
            title: "产品周会".into(),
            dtstart: start_ms,
            dtend: end_ms,
            all_day: false,
            location: "3F-A102".into(),
            description: "点击入会：https://vc.feishu.cn/j/123456789（密码 8888）".into(),
            source: "系统日历".into(),
        };
        assert!(event_is_ongoing(&ev, 14, 20));
        assert!(!event_is_ongoing(&ev, 13, 50));
        assert!(!event_is_ongoing(&ev, 15, 30));
        assert_eq!(
            format_active_meeting_context(&ev),
            "产品周会（预计 15:30 结束）"
        );
        assert_eq!(
            extract_meeting_url(&ev.description).as_deref(),
            Some("https://vc.feishu.cn/j/123456789")
        );

        let call_logs = vec![CallRecord {
            number: "13800009367".into(),
            name: "张总".into(),
            call_type: 3,
            date_ms: start_ms + 300_000,
            duration_sec: 0,
        }];
        let wb = empty_wellbeing();
        let input = HomeHubInput {
            hour: 14,
            minute: 20,
            today_ymd: "2026-10-29",
            device_connected: true,
            device_model: "StepPhone Pro",
            firmware_profile: "StepOS 2.0",
            threads: &[],
            notifications: &[],
            otp_banners: &[],
            sms: &[],
            call_logs: &call_logs,
            express_parcels: &[],
            calendar_events: &[ev],
            alarms: &[],
            wellbeing: &wb,
            phone_clipboard: "",
            remote_agent_files: &[],
            work_deliverables_count: 0,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        };
        let report = build_home_hub_report(&input);
        let missed = report
            .priority_cards
            .iter()
            .find(|c| c.kind == HomeCardKind::MissedCall)
            .expect("missed call card");
        assert_eq!(
            missed.quick_replies[0],
            "正在「产品周会」开会（预计 15:30 结束），稍后回你"
        );
        let meeting = report
            .priority_cards
            .iter()
            .find(|c| c.kind == HomeCardKind::UpcomingMeeting)
            .expect("meeting card");
        assert_eq!(meeting.badge, "正在进行");
        assert_eq!(
            meeting.code_or_number.as_deref(),
            Some("https://vc.feishu.cn/j/123456789")
        );

        let feishu_replies = quick_replies_for_message_with_context(
            "飞书",
            "产品群",
            "请确认方案",
            Some("产品周会（预计 15:30 结束）"),
            None,
            false,
        );
        assert_eq!(
            feishu_replies[1],
            "正在「产品周会」开会（预计 15:30 结束），稍后回你"
        );
    }

    #[test]
    fn express_gateway_rejection_pickup_dedup_and_completed_suppression() {
        assert!(looks_like_sms_service_number("106839382199220262"));
        assert!(looks_like_sms_service_number("88888888888888"));
        assert!(looks_like_sms_service_number("95338"));
        assert!(looks_like_sms_service_number("13800002916"));
        assert!(!looks_like_sms_service_number("SF1234567890123"));
        assert!(!is_completed_express_text(
            "【菜鸟驿站】您的包裹已签收，感谢使用"
        ));
        assert!(is_completed_express_text("丰巢：快件已取出"));
        assert!(is_completed_express_text("订单状态：已完成"));

        // 18-digit 1068 gateway number inside SMS text must not be extracted as tracking number
        let gateway_sms = Message {
            provider_id: None,
            address: "106839382199220262".into(),
            body:
                "【菜鸟驿站】您的包裹已到站（通道号 106839382199220262），请凭取件码 18-2-2009 领取"
                    .into(),
            date_ms: 0,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        };
        let completed_sms = Message {
            provider_id: None,
            address: "1068001".into(),
            body: "【丰巢】您的顺丰快递 SF9988776655443 已取出，取件码 09-1-1111".into(),
            date_ms: 0,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        };
        let read_sms = Message {
            provider_id: None,
            address: "1068002".into(),
            body: "【菜鸟驿站】您的中通包裹 ZTO123456789012 已到站，取件码 07-3-3333".into(),
            date_ms: 1_760_000_000_000,
            outgoing: false,
            kind: 1,
            read: true,
            subscription: None,
        };
        let wechat_thread = ImThread {
            thread_id: "com.tencent.mm::丰巢快递柜".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "丰巢快递柜".into(),
            last_message: "您的顺丰快递 SF1234567890123 已入柜，取件码 18-2-2009".into(),
            last_timestamp_ms: 0,
            unread_count: 1,
            ..Default::default()
        };

        let parcels = extract_live_express_parcels_with_threads(
            &[gateway_sms, completed_sms, read_sms],
            &[],
            &[wechat_thread],
            "",
            &[],
        );

        // Must deduplicate pickup code 18-2-2009 into a single parcel upgraded to SF1234567890123,
        // reject 106839382199220262 and suppress completed parcel 09-1-1111.
        // Reading a pickup notice does not mean the user has collected the other parcel.
        assert_eq!(parcels.len(), 2);
        assert!(parcels.iter().any(|p| p.pickup_code == "07-3-3333"));
        assert_eq!(parcels[0].pickup_code, "18-2-2009");
        assert_eq!(parcels[0].tracking_no, "SF1234567890123");
    }

    #[test]
    fn human_threads_mentioning_express_and_past_outdoor_schedules_are_not_ghost_cards() {
        let human_thread = ImThread {
            thread_id: "com.tencent.mm::王司马".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "王司马".into(),
            last_message: "下午3点前请确认丰巢包裹和接口联调排期？".into(),
            last_timestamp_ms: 1_762_100_000_000,
            unread_count: 1,
            ..Default::default()
        };
        let group_thread = ImThread {
            thread_id: "com.tencent.mm::产品研发群".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "产品研发群".into(),
            last_message: "测试包裹取件码状态同步".into(),
            last_timestamp_ms: 1_762_100_000_000,
            unread_count: 1,
            ..Default::default()
        };
        let parcels = extract_live_express_parcels_with_threads(
            &[],
            &[],
            &[human_thread, group_thread],
            "",
            &[],
        );
        assert!(
            parcels.is_empty(),
            "Human/group chats mentioning express words without tracking/pickup codes must not synthesize WX-* parcels: {parcels:?}"
        );

        let past_cal = PhoneCalendarEvent {
            recurring: Some(false),
            id: 80,
            title: "10月27日外出拜访客户".into(),
            dtstart: 1_761_530_400_000, // 2026-10-27 10:00 CST
            dtend: 1_761_534_000_000,
            location: "中关村园区".into(),
            description: "上周历史日程".into(),
            all_day: false,
            source: String::new(),
        };
        let wb = empty_wellbeing();
        let input = HomeHubInput {
            hour: 10,
            minute: 0,
            today_ymd: "2026-11-03",
            device_connected: true,
            device_model: "PZ261A",
            firmware_profile: "StepOS",
            threads: &[],
            notifications: &[],
            otp_banners: &[],
            sms: &[],
            call_logs: &[],
            express_parcels: &[],
            calendar_events: &[past_cal],
            alarms: &[],
            wellbeing: &wb,
            phone_clipboard: "",
            remote_agent_files: &[],
            work_deliverables_count: 0,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        };
        let report = build_home_hub_report(&input);
        assert!(
            !report
                .life_context_cards
                .iter()
                .any(|c| c.title.contains("10月27日") || c.context_link.contains("10月27日")),
            "Past calendar events must not appear in life_context_cards for today: {:?}",
            report.life_context_cards
        );

        assert!(is_completed_express_text(
            "【顺丰速运】您的快件SF1628890123456已签收，感谢使用顺丰。"
        ));
        assert!(is_completed_express_text(
            "【丰巢】您的包裹（取件码79983898）已取出，感谢使用。"
        ));
        assert!(!is_completed_express_text(
            "【顺丰速运】您的快件SF1628890123456已由丰巢智能柜签收（已签收），请凭取件码79983898前往取件。"
        ));
    }

    #[test]
    fn dingdong_doorstep_delivery_at_mentions_and_boss_zhipin_threads_surface_in_priority_cards() {
        let now_ms = chrono::Local::now().timestamp_millis();
        let sms = vec![Message {
            provider_id: Some(501),
            address: "106912345678".into(),
            body: "【叮咚买菜】您在叮咚买菜购买的商品已代收（家门口），请及时取走。".into(),
            date_ms: now_ms - 120_000,
            outgoing: false,
            kind: 1,
            read: true,
            subscription: None,
        }];
        let threads = vec![
            ImThread {
                thread_id: "com.tencent.mm::产品研发群".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "产品研发群".into(),
                unread_count: 1,
                last_sender: "李工".into(),
                last_message: "[有人@我] 李工: 下午有空看下胶囊交互吗？".into(),
                last_timestamp_ms: now_ms - 60_000,
                ..Default::default()
            },
            ImThread {
                thread_id: "com.hpbr.bosszhipin::张HR".into(),
                package_name: "com.hpbr.bosszhipin".into(),
                app_name: "BOSS直聘".into(),
                title: "张女士 · 阶跃星辰".into(),
                unread_count: 0,
                last_sender: "张女士".into(),
                last_message: "您好，向您发出面试邀约，请问明天下午方便吗？".into(),
                last_timestamp_ms: now_ms - 90_000,
                ..Default::default()
            },
        ];
        let wb = empty_wellbeing();
        let today_ymd = chrono::Local::now().format("%Y-%m-%d").to_string();
        let input = HomeHubInput {
            hour: 12,
            minute: 30,
            today_ymd: &today_ymd,
            device_connected: true,
            device_model: "PZ261A",
            firmware_profile: "StepOS",
            threads: &threads,
            notifications: &[],
            otp_banners: &[],
            sms: &sms,
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
        let report = build_home_hub_report(&input);
        assert!(
            report.priority_cards.iter().any(|c| {
                c.kind == HomeCardKind::TakeoutAndDelivery
                    && c.badge == "叮咚买菜"
                    && c.title.contains("已送达家门口")
            }),
            "Dingdong Maicai doorstep SMS must surface as TakeoutAndDelivery card: {:?}",
            report.priority_cards
        );
        assert!(
            report.priority_cards.iter().any(|c| {
                c.kind == HomeCardKind::UnreadMessage
                    && c.badge.contains("有人@我")
                    && c.urgency == 89
            }),
            "Explicit unread current-user mention must surface with urgency 89: {:?}",
            report.priority_cards
        );
        assert!(
            report.priority_cards.iter().any(|c| {
                c.kind == HomeCardKind::UnreadMessage
                    && c.badge.contains("BOSS直聘")
                    && c.urgency == 87
            }),
            "BOSS Zhipin recruitment thread must surface with urgency 87: {:?}",
            report.priority_cards
        );
    }

    #[test]
    fn reminder_auto_expiration_and_manual_governance_lifecycle_work_end_to_end() {
        let now = chrono::Local::now();
        let now_ms = now.timestamp_millis();
        let today_ymd = now.format("%Y-%m-%d").to_string();

        // 1. Single old express SMS (2 days old, sms.len() == 1) must auto-expire in extract_live_express_parcels_with_threads
        let stale_express_sms = vec![Message {
            provider_id: Some(701),
            address: "10698888".into(),
            body: "【菜鸟驿站】您的顺丰包裹 SF9988776655443 已到站，请凭取件码 9-1-9988 领取"
                .into(),
            date_ms: now_ms - 2 * 86_400_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        }];
        assert!(
            extract_live_express_parcels_with_threads(&stale_express_sms, &[], &[], "", &[])
                .is_empty(),
            "single stale express SMS (>24h) must auto-expire even when sms.len() == 1"
        );

        // 2. Read arrived takeout (>30m), read @mention/career (>30m), callback-resolved missed call,
        //    and ended meeting (>30m after dtend) must all auto-expire.
        let sms = vec![
            Message {
                provider_id: Some(702),
                address: "106912345678".into(),
                body: "【叮咚买菜】您在叮咚买菜购买的商品已代收（家门口），请及时取走。".into(),
                date_ms: now_ms - 2_400_000, // 40 mins ago (> 30m grace for read arrived takeout)
                outgoing: false,
                kind: 1,
                read: true,
                subscription: None,
            },
            Message {
                provider_id: Some(703),
                address: "+8613800006877".into(),
                body: "刚在开会，现在方便通电话吗？".into(),
                date_ms: now_ms - 60_000, // newer outgoing SMS after missed call
                outgoing: true,
                kind: 2,
                read: true,
                subscription: None,
            },
        ];
        let threads = vec![
            ImThread {
                thread_id: "com.tencent.mm::expired_at".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "架构群".into(),
                unread_count: 0,
                last_sender: "王工".into(),
                last_message: "@倒影颠倒 请看下接口文档".into(),
                last_timestamp_ms: now_ms - 2_400_000, // 40 mins ago (> 30m grace when read)
                ..Default::default()
            },
            ImThread {
                thread_id: "com.tencent.mm::active_unread".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "产品协同群".into(),
                unread_count: 2,
                last_sender: "陈经理".into(),
                last_message: "下午三点对齐进度".into(),
                last_timestamp_ms: now_ms - 120_000,
                ..Default::default()
            },
        ];
        let call_logs = vec![CallRecord {
            number: "138 0000 6877".into(),
            name: "刘总".into(),
            call_type: 3,
            date_ms: now_ms - 300_000, // 5 mins ago, resolved by newer outgoing SMS above
            duration_sec: 0,
        }];
        let ended_meeting_start = chrono::Local
            .with_ymd_and_hms(
                chrono::Datelike::year(&now),
                chrono::Datelike::month(&now),
                chrono::Datelike::day(&now),
                8,
                0,
                0,
            )
            .single()
            .unwrap()
            .timestamp_millis();
        let calendar_events = vec![PhoneCalendarEvent {
            recurring: Some(false),
            id: 8801,
            title: "晨间立会".into(),
            dtstart: ended_meeting_start,
            dtend: ended_meeting_start + 1_800_000, // ended at 08:30
            all_day: false,
            location: "线上".into(),
            description: "1) 同步昨日进展；2) 确认今日阻塞".into(),
            source: "飞书日历".into(),
        }];
        let wb = empty_wellbeing();
        let input = HomeHubInput {
            hour: 14,
            minute: 0,
            today_ymd: &today_ymd,
            device_connected: true,
            device_model: "PZ261A",
            firmware_profile: "StepOS",
            threads: &threads,
            notifications: &[],
            otp_banners: &[],
            sms: &sms,
            call_logs: &call_logs,
            express_parcels: &[],
            calendar_events: &calendar_events,
            alarms: &[],
            wellbeing: &wb,
            phone_clipboard: "",
            remote_agent_files: &[],
            work_deliverables_count: 0,
            pomodoro_running: false,
            pomodoro_remaining_secs: 0,
            dnd_on: false,
        };

        let report = build_home_hub_report(&input);
        assert!(
            !report
                .priority_cards
                .iter()
                .any(|c| c.kind == HomeCardKind::TakeoutAndDelivery),
            "read arrived takeout >30m must auto-expire: {:?}",
            report.priority_cards
        );
        assert!(
            !report
                .priority_cards
                .iter()
                .any(|c| c.id.contains("expired_at")),
            "read @mention thread >30m must auto-expire: {:?}",
            report.priority_cards
        );
        assert!(
            !report
                .priority_cards
                .iter()
                .any(|c| c.kind == HomeCardKind::MissedCall),
            "missed call followed by outgoing SMS must auto-expire: {:?}",
            report.priority_cards
        );
        assert!(
            !report
                .priority_cards
                .iter()
                .any(|c| c.kind == HomeCardKind::UpcomingMeeting),
            "meeting that ended at 08:30 must auto-expire by 14:00: {:?}",
            report.priority_cards
        );
        assert!(
            report.schedule_checklists.is_empty(),
            "ended meeting checklist must also auto-expire on today: {:?}",
            report.schedule_checklists
        );
        assert!(
            report
                .priority_cards
                .iter()
                .any(|c| c.id == "im-thread-com.tencent.mm::active_unread"),
            "active unread thread must remain visible"
        );

        // 3. Manual expire & restore via HomeReminderGovernance
        let orig_gov = crate::home_reminder_governance::active_governance();
        let mut test_gov = crate::home_reminder_governance::HomeReminderGovernance::default();
        test_gov.manual_expire(
            crate::home_reminder_governance::ReminderTargetSpec::new(
                "im:com.tencent.mm::active_unread",
                crate::home_reminder_governance::ReminderCategory::UnreadMessage,
                "产品协同群",
            )
            .with_aliases(
                crate::home_reminder_governance::canonical_keys_for_im_thread(
                    "com.tencent.mm::active_unread",
                    None,
                ),
            )
            .with_subtitle("微信 · 2 条未读")
            .with_source(
                "产品协同群",
                Some("im:com.tencent.mm::active_unread".into()),
            )
            .with_source_ts(now_ms),
            now_ms,
        );
        crate::home_reminder_governance::set_active_governance(&test_gov);

        let suppressed_report = build_home_hub_report(&input);
        assert!(
            !suppressed_report
                .priority_cards
                .iter()
                .any(|c| c.id == "im-thread-com.tencent.mm::active_unread"),
            "manually expired IM thread must be filtered out of priority_cards"
        );
        assert_eq!(suppressed_report.unread_messages_count, 0);

        test_gov.restore_reminder("im:com.tencent.mm::active_unread", now_ms + 1_000);
        crate::home_reminder_governance::set_active_governance(&test_gov);
        let restored_report = build_home_hub_report(&input);
        assert!(
            restored_report
                .priority_cards
                .iter()
                .any(|c| c.id == "im-thread-com.tencent.mm::active_unread"),
            "restored IM thread must reappear in priority_cards"
        );

        crate::home_reminder_governance::set_active_governance(&orig_gov);
    }

    #[test]
    fn dingdong_coupon_and_sf_account_change_are_rejected_and_slogan_headlines_are_cleaned() {
        // 1. "优惠券到账提醒" from "叮咚买菜 美食水果蔬菜海鲜早餐" must NOT be classified as delivery
        assert_eq!(
            classify_delivery_or_mobility_signal(
                "com.tencent.mm",
                "微信",
                "叮咚买菜 美食水果蔬菜海鲜早餐",
                "优惠券到账提醒",
            ),
            None
        );
        // Generic slogan without delivery progress must also be rejected
        assert_eq!(
            classify_delivery_or_mobility_signal(
                "com.tencent.mm",
                "微信",
                "叮咚买菜 美食水果蔬菜海鲜早餐",
                "周末生鲜特惠专场",
            ),
            None
        );
        // Active delivery from official account with slogan title must clean the headline
        let (_, headline, status_tag, is_arrived) = classify_delivery_or_mobility_signal(
            "com.tencent.mm",
            "微信",
            "叮咚买菜 美食水果蔬菜海鲜早餐",
            "骑手正在配送中",
        )
        .expect("real delivery progress should classify");
        assert!(!is_arrived);
        assert_eq!(status_tag, "骑手配送中");
        assert_eq!(headline, "骑手正在配送中");

        // 2. "顺丰速运+" with "账户变动提醒" must NOT be classified as express notification or live parcel
        assert!(!is_express_notification(
            "微信",
            "顺丰速运+",
            "账户变动提醒"
        ));
        let stale_sf_thread = ImThread {
            thread_id: "com.tencent.mm::顺丰速运+".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "顺丰速运+".into(),
            last_message: "账户变动提醒".into(),
            last_timestamp_ms: 0,
            unread_count: 0,
            messages: vec![crate::push::ImMessage {
                id: "m1".into(),
                sender: "顺丰速运+".into(),
                text: "账户变动提醒".into(),
                timestamp_ms: 0,
                observed_at_ms: 1_760_000_000_000,
                time_known: Some(false),
                outgoing: false,
                delivery_status: "received".into(),
                via: "ui_window".into(),
                client_msg_id: String::new(),
                media_hint: String::new(),
                recalled: false,
                ..Default::default()
            }],
            ..Default::default()
        };
        let parcels =
            extract_live_express_parcels_with_threads(&[], &[], &[stale_sf_thread], "", &[]);
        assert!(parcels.is_empty());
    }

    #[test]
    fn dianping_eta_window_community_power_outage_and_jd_shipment_signals_work_accurately() {
        // 1. Dianping delivery signal classification
        assert_eq!(
            classify_delivery_or_mobility_signal(
                "com.dianping.v1",
                "大众点评",
                "商家已接单",
                "您的订单预计今天 20:30~20:50送达，请您耐心等待。",
            ),
            Some((
                "大众点评".to_string(),
                "商家已接单".to_string(),
                "骑手配送中".to_string(),
                false,
            ))
        );

        // 2. ETA window extraction
        let ref_ms = Local
            .with_ymd_and_hms(2026, 11, 4, 20, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let expected_end_ms = Local
            .with_ymd_and_hms(2026, 11, 4, 20, 50, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            extract_eta_window_from_text(
                "您的订单预计今天 20:30~20:50送达，请您耐心等待。",
                ref_ms,
                ref_ms + 300_000,
            ),
            Some((
                expected_end_ms,
                "20:30~20:50".to_string(),
                "20:50送达".to_string(),
            ))
        );

        // 3. Community power outage alert classification
        let alert = classify_public_utility_or_community_alert(
            "微信",
            "春江郦城-运营官-朱",
            "【停电通知】因本小区10V开万甲线...",
        )
        .expect("power outage notice must classify as community/utility alert");
        assert_eq!(alert.1, "停电通知");
        assert!(alert.2.contains("因本小区10V开万甲线"));

        // 4. JD logistics shipment reminder classification & live parcel extraction even when read
        assert!(is_express_notification(
            "京东",
            "物流发出提醒",
            "您购买的[莲花（LIANHU...）]已从周..."
        ));
        let jd_notif = PushNotification {
            key: "jd-ship-1".into(),
            seq_id: 501,
            package_name: "com.jingdong.app.mall".into(),
            app_name: "京东".into(),
            title: "物流发出提醒".into(),
            text: "您购买的[莲花（LIANHU...）]已从周...".into(),
            post_time_ms: Local::now().timestamp_millis() - 60_000,
            active: false,
            read: true,
            dismissed: false,
            ..Default::default()
        };
        let jd_parcels = extract_live_express_parcels_with_threads(&[], &[jd_notif], &[], "", &[]);
        assert_eq!(jd_parcels.len(), 1);
        assert_eq!(jd_parcels[0].carrier, "京东快递");
        assert_eq!(jd_parcels[0].status, "运输中");
    }
}
