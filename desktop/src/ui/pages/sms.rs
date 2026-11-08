use super::super::{
    icons::UiIcon, studio_state::StudioAction, theme::*, widgets::*, Action, Companion, Page,
    PushAction, F_IM_REPLY, F_SMS_BODY, F_SMS_SEARCH, F_SMS_TO,
};
use chrono::{Local, TimeZone};
use facloud_ui::{
    assets::asset_cache::AssetSource, color::ColorU, elements::*, event::DispatchedEvent,
    fonts::Weight, platform::Cursor, units::Pixels, AppContext, Element,
};
use pathfinder_geometry::vector::{vec2f, Vector2F};
use stepx_companion::push::{
    demo_push_snapshot, extract_first_url, extract_otp_code, ImChannel, ImMessage, ImThread,
    PushSnapshot,
};

// Message identity must survive history inserts and refreshes.
pub(in crate::ui) fn im_detail_key(t: &ImThread, idx: usize) -> String {
    let m = &t.messages[idx];
    let identity = if m.id.is_empty() {
        serde_json::to_string(&(m.timestamp_ms, &m.sender, m.outgoing, &m.text)).unwrap_or_default()
    } else {
        m.id.clone()
    };
    format!(
        "msg_detail::{}",
        serde_json::to_string(&(&t.thread_id, identity)).unwrap_or_default()
    )
}

pub(crate) fn sanitize_ui_display_text(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let mut cleaned = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            if let Some(rel_end) = chars[i..].iter().position(|&c| c == ']') {
                let bracket_content: String = chars[i + 1..i + rel_end].iter().collect();
                let lower = bracket_content.to_lowercase();
                if bracket_content.contains("队列消息")
                    || bracket_content.contains("压测")
                    || bracket_content.contains("自测探针")
                    || lower.contains("stepx_e2e")
                    || lower.contains("e2e_probe")
                {
                    i += rel_end + 1;
                    while i < chars.len() && chars[i] == ' ' {
                        i += 1;
                    }
                    continue;
                }
            }
        }
        let c = chars[i];
        let code = c as u32;
        // Strip supplementary-plane emojis/pictographs (>= 0x1F000) and variation selectors / ZWJ
        // that render as black tofu boxes (`☒`) in standard CJK fonts while keeping BMP symbols.
        let is_tofu_emoji = code >= 0x1F000
            || matches!(
                code,
                0x200D | 0xFE0E | 0xFE0F | 0xFFFC | 0xFFFD | 0xE0020..=0xE007F
            );
        if !is_tofu_emoji {
            cleaned.push(c);
        }
        i += 1;
    }
    cleaned
        .lines()
        .map(|line| {
            let mut prev_space = false;
            let mut out_line = String::with_capacity(line.len());
            for ch in line.trim().chars() {
                if ch == ' ' || ch == '\t' {
                    if !prev_space {
                        out_line.push(' ');
                    }
                    prev_space = true;
                } else {
                    prev_space = false;
                    out_line.push(ch);
                }
            }
            out_line
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

pub(crate) fn humanize_wechat_emojis(text: &str) -> String {
    if !text.contains('[') {
        return text.to_string();
    }
    const EMOJI_MAP: &[(&str, &str)] = &[
        ("[破涕为笑]", "😂"),
        ("[笑哭]", "😂"),
        ("[呲牙]", "😁"),
        ("[微笑]", "🙂"),
        ("[捂脸]", "🤦"),
        ("[偷笑]", "🤭"),
        ("[合十]", "🙏"),
        ("[抱拳]", "🙏"),
        ("[爱心]", "❤️"),
        ("[加油]", "💪"),
        ("[流泪]", "😢"),
        ("[大哭]", "😢"),
        ("[惊讶]", "😮"),
        ("[调皮]", "😜"),
        ("[可爱]", "🥰"),
        ("[发呆]", "😳"),
        ("[得意]", "😎"),
        ("[害羞]", "😊"),
        ("[愉快]", "😊"),
        ("[旺柴]", "🐶"),
        ("[狗头]", "🐶"),
        ("[庆祝]", "🎉"),
        ("[礼物]", "🎁"),
        ("[咖啡]", "☕"),
        ("[太阳]", "☀️"),
        ("[拥抱]", "🤗"),
        ("[握手]", "🤝"),
        ("[机智]", "🤓"),
        ("[鼓掌]", "👏"),
        ("[笑]", "😄"),
        ("[强]", "👍"),
        ("[赞]", "👍"),
        ("[玫瑰]", "🌹"),
        ("[OK]", "👌"),
        ("[ok]", "👌"),
        ("[汗]", "😅"),
        ("[色]", "😍"),
        ("[耶]", "✌️"),
    ];
    let mut out = text.to_string();
    for (token, emoji) in EMOJI_MAP {
        if out.contains(token) {
            out = out.replace(token, emoji);
        }
    }
    out
}

pub(crate) fn format_im_message_content(
    m: &ImMessage,
    privacy_mask: bool,
) -> (String, Option<&'static str>, Option<String>) {
    let recalled_badge = m.recalled.then_some("[已撤回]");
    if privacy_mask && !m.outgoing {
        let masked = "[防窥模式已开启，已隐藏消息内容]";
        let body = if m.recalled {
            format!("[已撤回] {masked}")
        } else {
            masked.to_string()
        };
        return (body, recalled_badge, None);
    }

    let hint_clean = sanitize_ui_display_text(m.media_hint.trim());
    let txt_clean = sanitize_ui_display_text(m.text.trim());
    let hint = hint_clean.as_str();
    let txt = txt_clean.as_str();
    let media_badge = (!hint.is_empty()).then(|| hint.to_string());

    let base_text = if !hint.is_empty() {
        if txt.is_empty() || txt == hint {
            hint.to_string()
        } else if txt.starts_with(hint) {
            txt_clean.clone()
        } else {
            format!("{hint} {txt_clean}")
        }
    } else if txt.is_empty() && m.recalled {
        "[消息已撤回]".to_string()
    } else {
        txt_clean.clone()
    };

    let display_text = if m.recalled
        && !base_text.contains("[已撤回]")
        && !base_text.contains("消息已撤回")
        && !base_text.contains("撤回了一条消息")
    {
        if base_text.trim().is_empty() {
            "[已撤回]".to_string()
        } else {
            format!("[已撤回] {base_text}")
        }
    } else {
        base_text
    };

    (display_text, recalled_badge, media_badge)
}

pub(crate) fn im_thread_has_visible_content(t: &ImThread) -> bool {
    !t.last_message.trim().is_empty()
        || t.messages
            .iter()
            .any(|m| !m.text.trim().is_empty() || !m.media_hint.trim().is_empty() || m.recalled)
}

pub(crate) fn im_message_matches_search(m: &ImMessage, search_q: &str) -> bool {
    let q = search_q.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    m.text.to_lowercase().contains(&q)
        || m.media_hint.to_lowercase().contains(&q)
        || (m.recalled && "[已撤回] 消息已撤回".contains(&q))
}

#[derive(Default, Clone)]
struct SmsScrollMetricsState {
    thread_key: String,
    msg_count: usize,
    content_height: f32,
    viewport_height: f32,
}

pub(in crate::ui) fn reset_message_scroll() {
    SMS_SCROLL_STATE.with(|cell| *cell.borrow_mut() = SmsScrollMetricsState::default());
}

thread_local! {
    static SMS_SCROLL_STATE: std::cell::RefCell<SmsScrollMetricsState> = Default::default();
}

struct SmsContentMeasure {
    child: El,
    measured_height: std::rc::Rc<std::cell::Cell<f32>>,
}

impl Element for SmsContentMeasure {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        let size = self.child.layout(c, ctx, app);
        self.measured_height.set(size.y());
        size
    }
    fn after_layout(&mut self, c: &mut AfterLayoutContext, a: &AppContext) {
        self.child.after_layout(c, a)
    }
    fn paint(&mut self, o: Vector2F, c: &mut PaintContext, a: &AppContext) {
        self.child.paint(o, c, a)
    }
    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }
    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
    fn dispatch_event(
        &mut self,
        e: &DispatchedEvent,
        c: &mut EventContext,
        a: &AppContext,
    ) -> bool {
        self.child.dispatch_event(e, c, a)
    }
}

struct SmsViewportFollow {
    child: El,
    scroll: ClippedScrollStateHandle,
    measured_content: std::rc::Rc<std::cell::Cell<f32>>,
    thread_key: String,
    msg_count: usize,
}

impl Element for SmsViewportFollow {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        let before_offset = self.scroll.scroll_start().as_f32();
        let size = self.child.layout(c, ctx, app);
        let after_child_offset = self.scroll.scroll_start().as_f32();
        let new_content = self.measured_content.get();
        let new_viewport = size.y();
        let mut target_scroll: Option<f32> = None;
        SMS_SCROLL_STATE.with(|cell| {
            let mut st = cell.borrow_mut();
            let thread_changed = st.thread_key != self.thread_key;
            let count_grew = !thread_changed && self.msg_count > st.msg_count;
            let old_max = (st.content_height - st.viewport_height).max(0.0);
            let new_max = (new_content - new_viewport).max(0.0);
            let dims_changed = (st.content_height - new_content).abs() > 0.5
                || (st.viewport_height - new_viewport).abs() > 0.5;
            let was_near_bottom = old_max <= 1.0
                || (old_max - before_offset) <= 96.0
                || before_offset >= new_max - 1.0;

            if new_content <= new_viewport + 1.0 {
                if after_child_offset.abs() > 0.1 || before_offset.abs() > 0.1 {
                    target_scroll = Some(0.0);
                }
            } else if thread_changed
                || (count_grew && was_near_bottom)
                || before_offset > new_max
                || (dims_changed && was_near_bottom)
            {
                target_scroll = Some(new_max);
            }

            st.thread_key = self.thread_key.clone();
            st.msg_count = self.msg_count;
            st.content_height = new_content;
            st.viewport_height = new_viewport;
        });
        if let Some(target) = target_scroll {
            if (target - after_child_offset).abs() > 0.5 {
                self.scroll.scroll_to(Pixels::new(target));
                return self.child.layout(c, ctx, app);
            }
        }
        size
    }
    fn after_layout(&mut self, c: &mut AfterLayoutContext, a: &AppContext) {
        self.child.after_layout(c, a)
    }
    fn paint(&mut self, o: Vector2F, c: &mut PaintContext, a: &AppContext) {
        self.child.paint(o, c, a)
    }
    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }
    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
    fn dispatch_event(
        &mut self,
        e: &DispatchedEvent,
        c: &mut EventContext,
        a: &AppContext,
    ) -> bool {
        self.child.dispatch_event(e, c, a)
    }
}

struct DisplaySmsThread {
    name: String,
    num: String,
    time: String,
    last_ts: u64,
    preview: String,
    last_body: String,
    unread: bool,
    badge: String,
    count: usize,
    otp_code: Option<String>,
}

fn norm_addr(s: &str) -> String {
    stepx_companion::sms::normalize_address(s)
}

pub(in super::super) fn extract_pickup_code(text: &str) -> Option<String> {
    stepx_companion::home_hub::extract_pickup_code(text)
}

pub(in super::super) fn extract_verification_code(text: &str) -> Option<String> {
    extract_otp_code(text).or_else(|| extract_pickup_code(text))
}

pub(crate) fn extract_sms_bracket_brand(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let start = trimmed.find('【')?;
    let after_start = &trimmed[start + '【'.len_utf8()..];
    let end = after_start.find('】')?;
    let candidate = after_start[..end].trim();
    let count = candidate.chars().count();
    if (2..=12).contains(&count)
        && !candidate.contains("队列")
        && !candidate.contains("压测")
        && !candidate.contains("探针")
    {
        Some(candidate.to_string())
    } else {
        None
    }
}

fn format_sms_time(ms: i64) -> String {
    if ms <= 0 {
        return "时间未知".into();
    }
    if let Some(dt) = Local.timestamp_millis_opt(ms).single() {
        dt.format("%m-%d %H:%M").to_string()
    } else {
        "时间未知".into()
    }
}

fn resolve_sender_info(
    addr: &str,
    contacts: &[stepx_companion::device::ContactEntry],
) -> (String, String) {
    let norm = norm_addr(addr);
    if let Some(c) = contacts.iter().find(|c| norm_addr(&c.number) == norm) {
        let first = c.name.trim().chars().next();
        let badge = match first {
            Some(ch) if ch != '+' && !ch.is_ascii_digit() => ch.to_string(),
            _ => "短".to_string(),
        };
        return (c.name.clone(), badge);
    }
    let label = if addr.is_empty() {
        "未知号码"
    } else {
        addr
    };
    (label.into(), "短".to_string())
}

fn im_avatar_style(package_name: &str, app_name: &str) -> (UiIcon, ColorU, ColorU) {
    let pkg = package_name.trim().to_ascii_lowercase();
    let app = app_name.trim();
    let app_lower = app.to_ascii_lowercase();

    let ch = if pkg == "com.android.mms" || app == "短信" {
        ImChannel::Sms
    } else {
        ImChannel::from_package(&pkg)
    };
    match ch {
        ImChannel::WeChat => {
            return (
                UiIcon::MessageCircle,
                rgb(0x07, 0xC1, 0x60),
                ColorU::white(),
            )
        }
        ImChannel::WeCom => return (UiIcon::Briefcase, rgb(0x2B, 0x6C, 0xB0), ColorU::white()),
        ImChannel::DingTalk => return (UiIcon::Bolt, rgb(0x00, 0x89, 0xFF), ColorU::white()),
        ImChannel::Feishu => return (UiIcon::Send, rgb(0x33, 0x70, 0xFF), ColorU::white()),
        ImChannel::QqTim => return (UiIcon::Chat, rgb(0x02, 0x84, 0xC7), ColorU::white()),
        ImChannel::Sms => return (UiIcon::Sms, rgb(0x10, 0xB9, 0x81), ColorU::white()),
        _ => {}
    }

    if pkg.contains("telegram") || app_lower.contains("telegram") {
        (UiIcon::Send, rgb(0x02, 0x84, 0xC7), ColorU::white())
    } else if pkg.contains("whatsapp") || app_lower.contains("whatsapp") {
        (UiIcon::Chat, rgb(0x16, 0xA3, 0x4A), ColorU::white())
    } else if pkg.contains("taobao")
        || pkg.contains("tmall")
        || app.contains("淘宝")
        || app.contains("天猫")
    {
        (UiIcon::Package, rgb(0xFF, 0x50, 0x00), ColorU::white())
    } else if pkg.contains("jingdong") || pkg.contains("jd.") || app.contains("京东") {
        (UiIcon::Package, rgb(0xE1, 0x25, 0x1B), ColorU::white())
    } else if pkg.contains("xunmeng") || pkg.contains("pinduoduo") || app.contains("拼多多") {
        (UiIcon::Package, rgb(0xE0, 0x2E, 0x24), ColorU::white())
    } else if pkg.contains("shizhuang")
        || pkg.contains("achievo")
        || app.contains("得物")
        || app.contains("唯品会")
    {
        (UiIcon::Package, rgb(0xDB, 0x27, 0x77), ColorU::white())
    } else if pkg.contains("sankuai")
        || pkg.contains("meituan")
        || pkg.contains("dianping")
        || app.contains("美团")
        || app.contains("大众点评")
    {
        (UiIcon::Coffee, rgb(0xFF, 0xB0, 0x00), ColorU::white())
    } else if pkg.contains("ele") || app.contains("饿了么") {
        (UiIcon::Coffee, rgb(0x00, 0x95, 0xFF), ColorU::white())
    } else if pkg.contains("yaya")
        || pkg.contains("hippo")
        || pkg.contains("samsclub")
        || pkg.contains("pupumall")
        || app.contains("叮咚")
        || app.contains("盒马")
        || app.contains("山姆")
        || app.contains("朴朴")
    {
        (UiIcon::Package, rgb(0x05, 0x96, 0x69), ColorU::white())
    } else if pkg.contains("cainiao")
        || pkg.contains("sf-express")
        || pkg.contains("fcbox")
        || app.contains("菜鸟")
        || app.contains("顺丰")
        || app.contains("丰巢")
        || app.contains("快递")
    {
        (UiIcon::Package, rgb(0xF5, 0x9E, 0x0B), ColorU::white())
    } else if pkg.contains("sdu.didi")
        || pkg.contains("autonavi")
        || pkg.contains("MobileTicket")
        || pkg.contains("umetrip")
        || pkg.contains("ctrip")
        || app.contains("滴滴")
        || app.contains("高德")
        || app.contains("12306")
        || app.contains("航旅")
        || app.contains("携程")
    {
        (UiIcon::Activity, rgb(0x02, 0x84, 0xC7), ColorU::white())
    } else if pkg.contains("alipay") || app.contains("支付宝") {
        (UiIcon::CreditCard, rgb(0x16, 0x77, 0xFF), ColorU::white())
    } else if pkg.contains("unionpay")
        || pkg.contains("cmbchina")
        || pkg.contains("icbc")
        || pkg.contains("ccb")
        || pkg.contains("abchina")
        || pkg.contains("chinamworld")
        || pkg.contains("bankcomm")
        || pkg.contains("pingan")
        || app.contains("云闪付")
        || app.contains("银行")
    {
        (UiIcon::CreditCard, rgb(0xDC, 0x26, 0x26), ColorU::white())
    } else if pkg.contains("wemeet")
        || pkg.contains("tencent.docs")
        || pkg.contains("wps")
        || app.contains("腾讯会议")
        || app.contains("腾讯文档")
        || app_lower.contains("wps")
    {
        (UiIcon::Briefcase, rgb(0x25, 0x63, 0xEB), ColorU::white())
    } else if pkg.contains("qqmail")
        || pkg.contains("netease.mail")
        || pkg.contains("android.gm")
        || pkg.contains("outlook")
        || app.contains("邮箱")
        || app_lower.contains("gmail")
        || app_lower.contains("outlook")
    {
        (UiIcon::Send, rgb(0x3B, 0x82, 0xF6), ColorU::white())
    } else if pkg.contains("github") || app_lower.contains("github") {
        (UiIcon::Cpu, rgb(0x33, 0x41, 0x55), ColorU::white())
    } else if app.contains("剪贴") || pkg.contains("clipboard") {
        (UiIcon::Clipboard, rgb(0x25, 0x63, 0xEB), ColorU::white())
    } else if app.contains("更新") || pkg.contains("companion.store") {
        (UiIcon::Download, rgb(0x7C, 0x3A, 0xED), ColorU::white())
    } else if app.contains("来电") || pkg.contains("dialer") {
        (UiIcon::Phone, rgb(0xEF, 0x44, 0x44), ColorU::white())
    } else if app.contains("日程")
        || app.contains("日历")
        || app.contains("闹钟")
        || pkg.contains("calendar")
        || pkg.contains("deskclock")
    {
        (UiIcon::Clock, rgb(0xF5, 0x9E, 0x0B), ColorU::white())
    } else if pkg.contains("stepx") || app_lower.contains("stepx") {
        (UiIcon::Sparkles, rgb(0x63, 0x66, 0xF1), ColorU::white())
    } else if ch == ImChannel::SocialDm {
        (UiIcon::Chat, rgb(0xE1, 0x1D, 0x48), ColorU::white())
    } else {
        (UiIcon::Chat, rgb(0x63, 0x66, 0xF1), ColorU::white())
    }
}

fn push_notification_source_label(package_name: &str, app_name: &str, title: &str) -> String {
    let pkg = package_name.trim().to_ascii_lowercase();
    let app = app_name.trim();
    let ttl = title.trim();
    let combined = format!("{pkg} {app} {ttl}");

    if pkg == "com.android.mms" || app == "短信" {
        return "短信".into();
    }
    let ch = ImChannel::from_package(&pkg);
    match ch {
        ImChannel::WeChat => return "微信".into(),
        ImChannel::WeCom => return "企业微信".into(),
        ImChannel::DingTalk => return "钉钉".into(),
        ImChannel::Feishu => return "飞书".into(),
        ImChannel::QqTim => return "QQ".into(),
        ImChannel::Sms => return "短信".into(),
        ImChannel::SocialDm => {
            if pkg.contains("xingin") || combined.contains("小红书") {
                return "小红书".into();
            } else if pkg.contains("aweme") || combined.contains("抖音") {
                return "抖音".into();
            } else if pkg.contains("gifshow") || combined.contains("快手") {
                return "快手".into();
            } else if pkg.contains("weibo") || combined.contains("微博") {
                return "微博".into();
            } else if pkg.contains("bili") || combined.contains("哔哩") || combined.contains("B站")
            {
                return "哔哩哔哩".into();
            } else if pkg.contains("zhihu") || combined.contains("知乎") {
                return "知乎".into();
            } else if pkg.contains("coolapk") || combined.contains("酷安") {
                return "酷安".into();
            }
        }
        _ => {}
    }

    for (kw, label) in [
        ("淘宝", "淘宝"),
        ("taobao", "淘宝"),
        ("天猫", "天猫"),
        ("tmall", "天猫"),
        ("京东", "京东"),
        ("jingdong", "京东"),
        ("拼多多", "拼多多"),
        ("xunmeng", "拼多多"),
        ("得物", "得物"),
        ("唯品会", "唯品会"),
        ("美团", "美团"),
        ("sankuai", "美团"),
        ("饿了么", "饿了么"),
        ("ele", "饿了么"),
        ("大众点评", "大众点评"),
        ("dianping", "大众点评"),
        ("叮咚买菜", "叮咚买菜"),
        ("yaya", "叮咚买菜"),
        ("盒马", "盒马"),
        ("hippo", "盒马"),
        ("山姆", "山姆"),
        ("朴朴", "朴朴"),
        ("顺丰", "顺丰速运"),
        ("sf-express", "顺丰速运"),
        ("菜鸟", "菜鸟"),
        ("cainiao", "菜鸟"),
        ("丰巢", "丰巢"),
        ("fcbox", "丰巢"),
        ("滴滴", "滴滴出行"),
        ("sdu.didi", "滴滴出行"),
        ("高德", "高德地图"),
        ("autonavi", "高德地图"),
        ("12306", "铁路12306"),
        ("MobileTicket", "铁路12306"),
        ("航旅纵横", "航旅纵横"),
        ("umetrip", "航旅纵横"),
        ("携程", "携程旅行"),
        ("ctrip", "携程旅行"),
        ("支付宝", "支付宝"),
        ("alipay", "支付宝"),
        ("云闪付", "云闪付"),
        ("unionpay", "云闪付"),
        ("招商银行", "招商银行"),
        ("cmbchina", "招商银行"),
        ("工商银行", "工商银行"),
        ("建设银行", "建设银行"),
        ("农业银行", "农业银行"),
        ("中国银行", "中国银行"),
        ("交通银行", "交通银行"),
        ("平安银行", "平安银行"),
        ("腾讯会议", "腾讯会议"),
        ("wemeet", "腾讯会议"),
        ("腾讯文档", "腾讯文档"),
        ("WPS", "WPS"),
        ("wps", "WPS"),
        ("QQ邮箱", "QQ邮箱"),
        ("qqmail", "QQ邮箱"),
        ("网易邮箱", "网易邮箱"),
        ("Gmail", "Gmail"),
        ("Outlook", "Outlook"),
        ("GitHub", "GitHub"),
        ("github", "GitHub"),
        ("剪贴板", "跨端剪贴"),
        ("应用更新", "应用更新"),
        ("未接来电", "未接来电"),
        ("日程", "日程提醒"),
        ("闹钟", "闹钟提醒"),
    ] {
        if combined.contains(kw) {
            return label.into();
        }
    }

    if !app.is_empty() && app != "StepX" && app != "通知" {
        let chars: Vec<char> = app.chars().collect();
        if chars.len() > 4 && !app.is_ascii() {
            return chars.into_iter().take(4).collect();
        }
        return app.to_string();
    }
    if pkg.contains("stepx") {
        return "StepX 伴侣".into();
    }
    "通知".into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImEntityKind {
    Express,
    FoodTea,
    Finance,
    Service,
    Group,
    Contact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImMediaKind {
    Image,
    Sticker,
    Video,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImRichModalKind {
    File,
    MiniProgram,
    Location,
    RedPacket,
    Transfer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImServiceNoticeData {
    pub kind: ImEntityKind,
    pub category_badge: &'static str,
    pub headline: String,
    pub kv_rows: Vec<(String, String)>,
    pub highlight_code: Option<(String, bool)>,
}

pub(crate) fn detect_im_rich_modal(
    text: &str,
    media_hint: &str,
) -> Option<(ImRichModalKind, String)> {
    let combined = if media_hint.trim().is_empty() {
        sanitize_ui_display_text(text)
    } else if text.trim().is_empty() || text.trim() == media_hint.trim() {
        sanitize_ui_display_text(media_hint)
    } else if text.trim().starts_with(media_hint.trim()) {
        sanitize_ui_display_text(text)
    } else {
        sanitize_ui_display_text(&format!("{} {}", media_hint.trim(), text.trim()))
    };
    for (token, kind, default_label) in [
        ("[文件]", ImRichModalKind::File, "聊天文件"),
        ("[小程序]", ImRichModalKind::MiniProgram, "微信小程序卡片"),
        ("[位置]", ImRichModalKind::Location, "位置共享卡片"),
        (
            "[微信红包]",
            ImRichModalKind::RedPacket,
            "大吉大利，恭喜发财",
        ),
        ("[红包]", ImRichModalKind::RedPacket, "大吉大利，恭喜发财"),
        ("[转账]", ImRichModalKind::Transfer, "微信转账凭证"),
    ] {
        if combined.contains(token) {
            let caption = combined
                .replace(token, "")
                .replace("小程序卡片", "")
                .replace("[已撤回]", "")
                .trim()
                .trim_start_matches(':')
                .trim_start_matches('：')
                .trim()
                .to_string();
            let display_caption = if caption.is_empty() {
                default_label.to_string()
            } else {
                caption
            };
            return Some((kind, display_caption));
        }
    }
    let stripped = combined.replace("[已撤回]", "").trim().to_string();
    if stripped == "小程序卡片"
        || stripped.starts_with("小程序卡片 ")
        || stripped.starts_with("小程序卡片:")
        || stripped.starts_with("小程序卡片：")
    {
        let caption = stripped
            .replacen("小程序卡片", "", 1)
            .trim()
            .trim_start_matches(':')
            .trim_start_matches('：')
            .trim()
            .to_string();
        let display_caption = if caption.is_empty() {
            "微信小程序卡片".to_string()
        } else {
            caption
        };
        return Some((ImRichModalKind::MiniProgram, display_caption));
    }
    None
}

pub(crate) fn detect_im_service_notice(
    sender_or_title: &str,
    raw_text: &str,
) -> Option<ImServiceNoticeData> {
    let cleaned = sanitize_ui_display_text(raw_text);
    if cleaned.is_empty() {
        return None;
    }
    let entity = classify_im_entity(sender_or_title, &cleaned, false, &[]);
    let notice_keywords = [
        "账户变动提醒",
        "包裹派件提醒",
        "包裹揽件提醒",
        "包裹签收提醒",
        "快件已送达",
        "取餐提醒",
        "订单配送通知",
        "订单完成通知",
        "停水通知",
        "停电通知",
        "缴费提醒",
        "交易提醒",
        "收款到账",
        "微信支付凭证",
        "已支付¥",
        "已支付￥",
        "取件码",
        "取餐码",
    ];
    let has_notice_kw = notice_keywords.iter().any(|kw| cleaned.contains(kw));
    let lines: Vec<&str> = cleaned
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let kv_count = lines
        .iter()
        .filter(|l| l.contains('：') || l.contains(": "))
        .count();

    let is_service_entity = matches!(
        entity,
        ImEntityKind::Express
            | ImEntityKind::FoodTea
            | ImEntityKind::Finance
            | ImEntityKind::Service
    );
    if !is_service_entity && !has_notice_kw && kv_count < 2 {
        return None;
    }
    // Avoid turning casual 1-line chat from a contact into a service card unless it has explicit service keywords
    if !is_service_entity && !has_notice_kw {
        return None;
    }

    let effective_kind = if matches!(
        entity,
        ImEntityKind::Express
            | ImEntityKind::FoodTea
            | ImEntityKind::Finance
            | ImEntityKind::Service
    ) {
        // Check if message text strongly overrides the category (e.g., food vs express)
        let text_kind = classify_im_entity("", &cleaned, false, &[]);
        if matches!(
            text_kind,
            ImEntityKind::Express
                | ImEntityKind::FoodTea
                | ImEntityKind::Finance
                | ImEntityKind::Service
        ) && text_kind != entity
        {
            text_kind
        } else {
            entity
        }
    } else if cleaned.contains("取餐") || cleaned.contains("茶") || cleaned.contains("咖啡") {
        ImEntityKind::FoodTea
    } else if cleaned.contains("包裹")
        || cleaned.contains("快件")
        || cleaned.contains("取件")
        || cleaned.contains("派件")
    {
        ImEntityKind::Express
    } else if cleaned.contains("支付")
        || cleaned.contains("账户变动")
        || cleaned.contains("¥")
        || cleaned.contains("￥")
    {
        ImEntityKind::Finance
    } else {
        ImEntityKind::Service
    };

    let category_badge = match effective_kind {
        ImEntityKind::Express => "物流服务通知",
        ImEntityKind::FoodTea => "门店取餐通知",
        ImEntityKind::Finance => "金融账单凭证",
        _ => "服务号提醒",
    };

    let mut headline = String::new();
    let mut kv_rows: Vec<(String, String)> = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        if idx == 0 && !line.contains('：') && !line.contains(": ") && line.chars().count() <= 24 {
            headline = (*line).to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('：').or_else(|| line.split_once(": ")) {
            let k_trim = k.trim();
            let v_trim = v.trim();
            if !k_trim.is_empty() && !v_trim.is_empty() && k_trim.chars().count() <= 12 {
                kv_rows.push((k_trim.to_string(), v_trim.to_string()));
                continue;
            }
        }
        if headline.is_empty() {
            headline = (*line).to_string();
        } else if kv_rows.len() < 4 {
            kv_rows.push(("详情".to_string(), (*line).to_string()));
        }
    }

    if headline.is_empty() {
        headline = cleaned.chars().take(28).collect();
    }
    if kv_rows.is_empty() {
        let sender_clean = sanitize_ui_display_text(sender_or_title);
        if !sender_clean.is_empty() {
            kv_rows.push(("来源".to_string(), sender_clean));
        }
        if cleaned != headline {
            kv_rows.push(("摘要".to_string(), cleaned.clone()));
        }
    }

    let highlight_code = extract_pickup_code(&cleaned)
        .map(|c| (c, true))
        .or_else(|| {
            extract_verification_code(&cleaned).map(|c| {
                let pickup = is_pickup_code_text(&c, &cleaned);
                (c, pickup)
            })
        });

    Some(ImServiceNoticeData {
        kind: effective_kind,
        category_badge,
        headline,
        kv_rows,
        highlight_code,
    })
}

pub(crate) fn extract_thread_value_signals(t: &ImThread) -> Vec<(ImEntityKind, String, String)> {
    let mut signals: Vec<(ImEntityKind, String, String)> = Vec::new();
    let thread_title_clean = sanitize_ui_display_text(&t.title);

    let classify_single = |sender_raw: &str,
                           text_raw: &str|
     -> Option<(ImEntityKind, String, String)> {
        let text_clean = sanitize_ui_display_text(text_raw);
        if text_clean.is_empty()
            || text_clean == "[图片]"
            || text_clean == "[语音]"
            || text_clean == "[动画表情]"
            || text_clean == "[表情]"
            || stepx_companion::home_hub::is_completed_express_text(&text_clean)
        {
            return None;
        }
        let sender_clean = sanitize_ui_display_text(sender_raw);
        let effective_entity_name = if !sender_clean.is_empty()
            && sender_clean != "我"
            && !stepx_companion::push::is_generic_sender_name(&sender_clean, &thread_title_clean)
        {
            sender_clean
        } else {
            thread_title_clean.clone()
        };
        if effective_entity_name.is_empty() {
            return None;
        }

        let sender_kind = classify_im_entity(&effective_entity_name, "", false, &[]);
        let text_kind = classify_im_entity("", &text_clean, false, &[]);
        let combined_kind = classify_im_entity(&effective_entity_name, &text_clean, false, &[]);

        // Guard against cross-talk: if sender is an Express brand (e.g. 顺丰速运+)
        // and the message text is clearly a FoodTea message (e.g. 取餐提醒), do NOT misattribute
        // the FoodTea message to the Express brand!
        if matches!(
            sender_kind,
            ImEntityKind::Express
                | ImEntityKind::FoodTea
                | ImEntityKind::Finance
                | ImEntityKind::Service
        ) && matches!(
            text_kind,
            ImEntityKind::Express
                | ImEntityKind::FoodTea
                | ImEntityKind::Finance
                | ImEntityKind::Service
        ) && sender_kind != text_kind
        {
            return None;
        }

        if matches!(
            combined_kind,
            ImEntityKind::Express
                | ImEntityKind::FoodTea
                | ImEntityKind::Finance
                | ImEntityKind::Service
        ) {
            Some((combined_kind, effective_entity_name, text_clean))
        } else {
            None
        }
    };

    for m in t.messages.iter().rev().take(6) {
        if m.outgoing || m.recalled {
            continue;
        }
        if let Some((kind, entity_name, snip)) = classify_single(&m.sender, &m.text) {
            if !signals
                .iter()
                .any(|(_, existing_name, _)| existing_name == &entity_name)
            {
                signals.push((kind, entity_name, snip));
            }
        }
    }

    if signals.is_empty() && !t.last_message.trim().is_empty() {
        if let Some(sig) = classify_single(&t.last_sender, &t.last_message) {
            signals.push(sig);
        }
    }

    signals
}

const CONTACT_AVATAR_PALETTE: [ColorU; 8] = [
    rgb(0x4F, 0x46, 0xE5), // Indigo
    rgb(0x08, 0x91, 0xB2), // Cyan
    rgb(0x0D, 0x94, 0x88), // Teal
    rgb(0x7C, 0x3A, 0xED), // Violet
    rgb(0xEA, 0x58, 0x0C), // Orange
    rgb(0xDB, 0x27, 0x77), // Pink
    rgb(0x25, 0x63, 0xEB), // Blue
    rgb(0x16, 0xA3, 0x4A), // Emerald
];

fn contact_avatar_color(name: &str) -> ColorU {
    let mut hash: u32 = 2166136261;
    for b in name.trim().bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(16777619);
    }
    CONTACT_AVATAR_PALETTE[(hash as usize) % CONTACT_AVATAR_PALETTE.len()]
}

#[allow(dead_code)]
fn clean_contact_initial(name: &str) -> String {
    let trimmed = name
        .trim()
        .trim_start_matches("+86 ")
        .trim_start_matches("+86-")
        .trim_start_matches("+86")
        .trim();
    let stripped_bracket = if let Some(rest) = trimmed.strip_prefix('【') {
        rest.split('】').next().unwrap_or(rest).trim()
    } else if let Some(rest) = trimmed.strip_prefix("我是") {
        let r = rest.trim();
        if r.is_empty() {
            trimmed
        } else {
            r
        }
    } else {
        trimmed
    };
    for ch in stripped_bracket.chars() {
        if !ch.is_whitespace()
            && !matches!(
                ch,
                '[' | ']' | '【' | '】' | '(' | ')' | '（' | '）' | '-' | '_' | '•' | '·' | '+'
            )
            && !ch.is_ascii_digit()
        {
            return ch.to_uppercase().to_string();
        }
    }
    if stripped_bracket.chars().any(|c| c.is_ascii_digit()) {
        "短".to_string()
    } else {
        "友".to_string()
    }
}

fn classify_im_entity(
    title: &str,
    sample_text: &str,
    is_group: bool,
    messages: &[ImMessage],
) -> ImEntityKind {
    let t_low = title.to_lowercase();
    let s_low = sample_text.to_lowercase();
    let combined = format!("{t_low} {s_low}");

    let express_kws = [
        "圆通速递",
        "圆通",
        "顺丰",
        "京东快递",
        "菜鸟",
        "丰巢",
        "中通",
        "申通",
        "韵达",
        "极兔",
        "ems",
        "邮政",
        "快递",
        "驿站",
        "快件",
        "德邦",
        "取件码",
    ];
    if express_kws.iter().any(|kw| t_low.contains(kw))
        || (!is_group
            && (t_low.contains("服务通知") || t_low.contains("通知"))
            && express_kws.iter().any(|kw| combined.contains(kw)))
    {
        return ImEntityKind::Express;
    }

    let food_kws = [
        "叮咚买菜",
        "盒马",
        "山姆",
        "朴朴",
        "霸王茶姬",
        "瑞幸",
        "喜茶",
        "星巴克",
        "奈雪",
        "库迪",
        "美团",
        "饿了么",
        "麦当劳",
        "肯德基",
        "取餐",
        "茶百道",
        "古茗",
        "蜜雪冰城",
    ];
    if food_kws.iter().any(|kw| t_low.contains(kw))
        || (!is_group
            && (t_low.contains("服务") || t_low.contains("提醒"))
            && food_kws.iter().any(|kw| combined.contains(kw)))
    {
        return ImEntityKind::FoodTea;
    }

    let finance_kws = [
        "微信支付",
        "支付宝",
        "银行",
        "账单",
        "工资条",
        "收款",
        "理财",
        "信用卡",
        "云闪付",
    ];
    if finance_kws.iter().any(|kw| t_low.contains(kw)) {
        return ImEntityKind::Finance;
    }

    let service_kws = [
        "服务号",
        "粤海水务",
        "水务",
        "供电",
        "燃气",
        "物业",
        "公众号",
        "通知",
        "政务",
        "12306",
        "中国移动",
        "中国联通",
        "中国电信",
    ];
    if service_kws.iter().any(|kw| t_low.contains(kw)) {
        return ImEntityKind::Service;
    }

    let mut distinct_senders: Vec<&str> = Vec::new();
    for m in messages {
        let s = m.sender.trim();
        if !m.outgoing
            && !s.is_empty()
            && s != "我"
            && !stepx_companion::push::is_generic_sender_name(s, title)
            && !distinct_senders.contains(&s)
        {
            distinct_senders.push(s);
        }
    }
    if is_group
        || title.trim().starts_with("群聊")
        || title.contains('•')
        || distinct_senders.len() >= 2
    {
        return ImEntityKind::Group;
    }

    ImEntityKind::Contact
}

fn detect_im_media_kind(text: &str, media_hint: &str) -> Option<(ImMediaKind, String)> {
    let t = text.trim();
    let h = media_hint.trim();
    let t_clean = t.replace("[已撤回]", "");
    let t_clean = t_clean.trim();
    let h_clean = h.replace("[已撤回]", "");
    let h_clean = h_clean.trim();
    for s in [t_clean, h_clean] {
        match s {
            "图片" | "[图片]" if t_clean.is_empty() || matches!(t_clean, "图片" | "[图片]") =>
            {
                return Some((ImMediaKind::Image, String::new()));
            }
            "动画表情" | "微信动画表情" | "[动画表情]" | "[表情]"
                if t_clean.is_empty()
                    || matches!(
                        t_clean,
                        "动画表情" | "微信动画表情" | "[动画表情]" | "[表情]"
                    ) =>
            {
                return Some((ImMediaKind::Sticker, String::new()));
            }
            "视频" | "[视频]" if t_clean.is_empty() || matches!(t_clean, "视频" | "[视频]") =>
            {
                return Some((ImMediaKind::Video, String::new()));
            }
            _ => {}
        }
    }
    let combined = if h.is_empty() {
        t.to_string()
    } else if t.is_empty() || t == h {
        h.to_string()
    } else if t.starts_with(h) {
        t.to_string()
    } else if t.chars().count() <= 24 {
        format!("{h} {t}")
    } else {
        t.to_string()
    };
    for (token, kind) in [
        ("[动画表情]", ImMediaKind::Sticker),
        ("[表情]", ImMediaKind::Sticker),
        ("[图片]", ImMediaKind::Image),
        ("[视频]", ImMediaKind::Video),
    ] {
        let stripped = combined.replace("[已撤回]", "").trim().to_string();
        if stripped == token || (stripped.starts_with(token) && stripped.chars().count() <= 28) {
            let caption = stripped.replacen(token, "", 1).trim().to_string();
            return Some((kind, humanize_wechat_emojis(&caption)));
        }
    }
    None
}

fn extract_voice_duration_secs(text: &str, media_hint: &str) -> Option<u32> {
    let t = text.trim();
    let h = media_hint.trim();
    let has_voice_tag = t.contains("[语音]")
        || h.contains("[语音]")
        || t.contains("[Voice]")
        || h.contains("[Voice]")
        || t.contains("语音消息")
        || h.contains("语音消息");
    let looks_like_unbracketed_voice = [t, h].into_iter().any(|s| {
        let s = s.replace("[已撤回]", "");
        let s = s.trim();
        (s.starts_with("语音") || s.starts_with("未播放语音"))
            && s.chars().any(|c| c.is_ascii_digit())
            && (s.contains('秒') || s.contains('"') || s.contains('″') || s.contains('”'))
    });
    let looks_like_bare_quote_sec = !t.is_empty()
        && t.len() <= 8
        && t.chars().take_while(|c| c.is_ascii_digit()).count() >= 1
        && t.trim_start_matches(|c: char| c.is_ascii_digit() || c.is_whitespace())
            .chars()
            .all(|c| matches!(c, '"' | '″' | '\'' | '”' | 's' | '秒'));
    if !has_voice_tag && !looks_like_unbracketed_voice && !looks_like_bare_quote_sec {
        return None;
    }
    let combined = format!("{h} {t}");
    let mut num_str = String::new();
    for ch in combined.chars() {
        if ch.is_ascii_digit() {
            num_str.push(ch);
        } else if !num_str.is_empty() {
            break;
        }
    }
    let secs = num_str.parse::<u32>().ok().unwrap_or(8).clamp(1, 600);
    Some(secs)
}

pub(crate) fn is_pickup_code_text(code: &str, text: &str) -> bool {
    extract_pickup_code(text).is_some_and(|found| found.eq_ignore_ascii_case(code.trim()))
}

fn compact_thread_title(title: &str, max_chars: usize) -> String {
    let canonical = stepx_companion::push::canonical_conversation_title_key(title);
    let sanitized = sanitize_ui_display_text(&canonical);
    let trimmed = sanitized.trim();
    let stripped = trimmed
        .strip_prefix("+86 ")
        .or_else(|| trimmed.strip_prefix("+86-"))
        .or_else(|| trimmed.strip_prefix("+86"))
        .unwrap_or(trimmed)
        .trim();
    let max_units = if max_chars <= 15 {
        9.8
    } else {
        max_chars as f32
    };
    let mut used = 0.0_f32;
    let mut out = String::new();
    let mut truncated = false;
    for ch in stripped.chars() {
        let w = if ch.is_ascii() || matches!(ch, '•' | '·' | ' ') {
            0.55
        } else {
            1.0
        };
        if used + w > max_units {
            truncated = true;
            break;
        }
        used += w;
        out.push(ch);
    }
    if truncated {
        format!("{}…", out.trim_end())
    } else {
        stripped.to_string()
    }
}

fn mix_color(a: ColorU, b: ColorU, t: f32) -> ColorU {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    if a.a == 0 {
        return ColorU::new(b.r, b.g, b.b, mix(0, b.a));
    }
    if b.a == 0 {
        return ColorU::new(a.r, a.g, a.b, mix(a.a, 0));
    }
    ColorU::new(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), mix(a.a, b.a))
}

impl Companion {
    /// Keep text width and row height identical before/after hover and selection.
    fn message_row_actions_slot(&self, child: El) -> El {
        ConstrainedBox::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(child)
                .finish(),
        )
        .with_width(56.)
        .with_height(20.)
        .finish()
    }

    pub(in crate::ui) fn msg_inline_svg_action_btn(
        &self,
        id: impl Into<String>,
        icon: UiIcon,
        active: bool,
        action: Action,
    ) -> El {
        let p = self.p();
        let id = id.into();
        let state = self.mouse_state(format!("btn:{id}"));
        let btn = Hoverable::new(state, move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            let t = if self.settings.value.reduced_motion {
                if hovered {
                    1.0
                } else {
                    0.0
                }
            } else {
                mouse.hover_progress().clamp(0.0, 1.0)
            };
            let (bg, border, fg) = if active {
                let bg = if mouse.is_clicked() {
                    p.hover
                } else {
                    mix_color(p.accent_soft, p.hover, t * 0.5)
                };
                (bg, p.accent, p.accent)
            } else {
                let bg = if mouse.is_clicked() {
                    p.accent_soft
                } else {
                    mix_color(p.surface, p.hover, t)
                };
                let border = if mouse.is_clicked() {
                    p.border_strong
                } else {
                    mix_color(p.border, p.border_strong, t)
                };
                let fg = if mouse.is_clicked() || hovered {
                    p.text
                } else {
                    mix_color(p.text2, p.text, t)
                };
                (bg, border, fg)
            };
            ConstrainedBox::new(
                Container::new(Align::new(self.content_icon(icon, 13.0, fg)).finish())
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.0)))
                    .finish(),
            )
            .with_width(20.0)
            .with_height(20.0)
            .finish()
        })
        .without_hover_animation()
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish();
        #[cfg(feature = "ui-capture")]
        let btn = crate::ui::scenario_check::control(id, btn);
        btn
    }

    pub(in super::super) fn sms_actions(&self) -> Vec<El> {
        vec![self.button_icon(
            "sms-new",
            "新建短信",
            UiIcon::Plus,
            Action::NewSms,
            Btn::Primary,
        )]
    }

    fn is_unread_native_sms(&self, m: &stepx_companion::sms::Message, now_ms: u64) -> bool {
        if m.kind != 1 || m.read || m.outgoing {
            return false;
        }
        if m.date_ms <= 0 || now_ms.saturating_sub(m.date_ms as u64) > 12 * 3600 * 1000 {
            return false;
        }
        let norm = norm_addr(&m.address);
        if self.read_sms_addresses.contains(&norm) {
            return false;
        }
        let sender = if m.address.trim().is_empty() {
            "未知号码"
        } else {
            m.address.trim()
        };
        let sms_id = format!("sms:{}:{}", sender, m.date_ms);
        !self.notification_center.read_ids.contains(&sms_id)
    }

    pub(in super::super) fn comm_mode_bar(&self) -> El {
        let p = self.p();
        let now_ms = Local::now().timestamp_millis().max(0) as u64;
        let im_unread: usize = self
            .push
            .snapshot
            .threads
            .iter()
            .filter(|t| !Self::is_excluded_im_package(&t.package_name))
            .map(|t| t.unread_count)
            .sum();
        let sms_unread: usize = self
            .sms
            .iter()
            .filter(|m| self.is_unread_native_sms(m, now_ms))
            .count()
            + self
                .push
                .prefs
                .manual_unread_targets
                .iter()
                .filter(|k| k.starts_with("sms:"))
                .count();
        let chat_unread = im_unread + sms_unread;
        let effective_notifs = self.effective_push_notifications();
        let extra_push_unread: usize = effective_notifs
            .iter()
            .filter(|n| {
                if n.dismissed || n.read {
                    return false;
                }
                let raw_tid = n
                    .key
                    .strip_prefix("im-thread:")
                    .unwrap_or(n.thread_id.as_str());
                let covered_by_im = self.push.snapshot.threads.iter().any(|t| {
                    !Self::is_excluded_im_package(&t.package_name)
                        && t.unread_count > 0
                        && ((!raw_tid.is_empty() && t.thread_id == raw_tid)
                            || (!t.notification_key.is_empty()
                                && (t.notification_key == n.key
                                    || format!("push-notif:{}", t.notification_key) == n.key)))
                });
                if covered_by_im {
                    return false;
                }
                let covered_by_sms = (n.package_name == "com.android.mms"
                    || n.key.starts_with("sms:")
                    || n.thread_id.starts_with("sms:"))
                    && self.sms.iter().any(|m| {
                        self.is_unread_native_sms(m, now_ms)
                            && (format!("sms:{}:{}", m.address.trim(), m.date_ms) == n.key
                                || format!("sms:{}", m.address.trim()) == n.thread_id
                                || (!n.text.trim().is_empty() && m.body.trim() == n.text.trim()))
                    });
                !covered_by_sms
            })
            .count();

        let mode_seg = self.segmented(
            "comm-mode-bar",
            vec![
                (
                    "会话".into(),
                    self.page == Page::Sms,
                    Action::Page(Page::Sms),
                ),
                (
                    "推送".into(),
                    self.page == Page::Notifications,
                    Action::Page(Page::Notifications),
                ),
            ],
        );
        let total_unread = chat_unread + extra_push_unread;
        let summary_badge = self.pill_badge(
            if total_unread > 0 {
                format!("{total_unread} 未读")
            } else {
                "已读".into()
            },
            if total_unread > 0 {
                p.accent
            } else {
                p.success_text
            },
            if total_unread > 0 {
                p.accent_soft
            } else {
                p.success_soft
            },
        );
        let left_bar = self.hstack(S2, vec![mode_seg, summary_badge]);

        let mut right_badges: Vec<El> = Vec::new();
        if self.page == Page::Sms {
            right_badges.push(self.button_icon(
                "comm-mark-msg-read",
                "全部已读",
                UiIcon::CheckAll,
                Action::Push(PushAction::MarkAllMessagesRead),
                Btn::Ghost,
            ));
            right_badges.push(self.button_icon(
                "comm-sync-msg",
                "同步",
                UiIcon::Refresh,
                Action::Push(PushAction::Refresh),
                Btn::Ghost,
            ));
        } else if self.page == Page::Notifications {
            right_badges.push(self.button_icon(
                "comm-mark-notif-read",
                "全部已读",
                UiIcon::CheckAll,
                Action::Push(PushAction::MarkAllNotificationsRead),
                Btn::Ghost,
            ));
            right_badges.push(self.button_icon(
                "comm-sync-notif",
                "同步",
                UiIcon::Refresh,
                Action::Push(PushAction::Refresh),
                Btn::Ghost,
            ));
        }
        if self.push.prefs.privacy_mask {
            right_badges.push(self.button_icon(
                "comm-toggle-privacy",
                "弹窗防窥中",
                UiIcon::Shield,
                Action::Push(PushAction::TogglePrivacyMask),
                Btn::Ghost,
            ));
        }

        Container::new(self.between(left_bar, self.hstack(S2, right_badges)))
            .with_horizontal_padding(S5)
            .with_vertical_padding(S2 + 2.)
            .with_background_color(if p.dark { p.sidebar } else { p.bg })
            .with_border(Border::bottom(1.).with_border_color(p.border))
            .finish()
    }

    fn im_channel_pill(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        active: bool,
        action: Action,
    ) -> El {
        let p = self.p();
        let id = id.into();
        let state = self.mouse_state(format!("im-ch-pill:{id}"));
        let label = label.into();
        let font = self.font;
        let pill = Hoverable::new(state, move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            let t = if self.settings.value.reduced_motion {
                if hovered {
                    1.0
                } else {
                    0.0
                }
            } else {
                mouse.hover_progress().clamp(0.0, 1.0)
            };
            let (bg, border, fg, weight) = if active {
                let bg = if mouse.is_clicked() {
                    p.hover
                } else {
                    mix_color(p.accent_soft, p.hover, t * 0.5)
                };
                (bg, p.accent, p.text, Weight::Semibold)
            } else {
                let bg = if mouse.is_clicked() {
                    p.accent_soft
                } else {
                    mix_color(p.surface, p.hover, t)
                };
                let border = if mouse.is_clicked() {
                    p.border_strong
                } else {
                    mix_color(p.border, p.border_strong, t)
                };
                let fg = if mouse.is_clicked() || hovered {
                    p.text
                } else {
                    mix_color(p.text2, p.text, t)
                };
                (bg, border, fg, Weight::Normal)
            };
            Container::new(
                facloud_ui::elements::Text::new(label.clone(), font, MICRO)
                    .with_color(fg)
                    .with_style(facloud_ui::fonts::Properties::default().weight(weight))
                    .soft_wrap(false)
                    .finish(),
            )
            .with_horizontal_padding(8.)
            .with_vertical_padding(3.)
            .with_background_color(bg)
            .with_border(Border::all(1.).with_border_color(border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
        })
        .without_hover_animation()
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish();
        #[cfg(feature = "ui-capture")]
        let pill = crate::ui::scenario_check::control(format!("im-pill-{id}"), pill);
        pill
    }

    fn sms_stream_scroll(&self, thread_key: String, msg_count: usize, content: El) -> El {
        let measured = std::rc::Rc::new(std::cell::Cell::new(0.0));
        let measured_inner = SmsContentMeasure {
            child: content,
            measured_height: measured.clone(),
        }
        .finish();
        SmsViewportFollow {
            child: self.pane_scroll(self.sms_messages_scroll.clone(), measured_inner),
            scroll: self.sms_messages_scroll.clone(),
            measured_content: measured,
            thread_key,
            msg_count,
        }
        .finish()
    }

    fn render_im_entity_avatar(
        &self,
        package_name: &str,
        app_label: &str,
        title: &str,
        sample_text: &str,
        is_group: bool,
        messages: &[ImMessage],
        size: f32,
    ) -> El {
        let p = self.p();
        let clean_title = sanitize_ui_display_text(title);
        let lookup_title = if clean_title.is_empty() {
            title
        } else {
            clean_title.as_str()
        };
        let kind = classify_im_entity(lookup_title, sample_text, is_group, messages);
        let radius = (size * 0.28).round().max(8.);
        let icon_sz = (size * 0.50).round().max(16.);
        let fg = ColorU::white();

        let real_avatar_path =
            stepx_companion::push::cached_thread_avatar_path(package_name, title)
                .or_else(|| {
                    stepx_companion::push::cached_thread_avatar_path(package_name, lookup_title)
                })
                .or_else(|| {
                    (!is_group)
                        .then(|| stepx_companion::push::cached_sender_avatar_path(lookup_title))
                        .flatten()
                })
                .filter(|path| cached_is_file(path) && !cached_image_is_black(path));

        let main_box: El = if let Some(avatar_path) = real_avatar_path {
            let img = Image::new(
                AssetSource::LocalFile {
                    path: avatar_path.to_string_lossy().into_owned(),
                },
                CacheOption::BySize,
            )
            .cover()
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
            .finish();
            ConstrainedBox::new(
                Container::new(img)
                    .with_background_color(p.surface)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
                    .finish(),
            )
            .with_width(size)
            .with_height(size)
            .finish()
        } else {
            match kind {
                ImEntityKind::Express => ConstrainedBox::new(
                    Container::new(
                        Align::new(self.content_icon(UiIcon::Package, icon_sz, fg)).finish(),
                    )
                    .with_background_color(rgb(0xF5, 0x9E, 0x0B))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
                    .finish(),
                )
                .with_width(size)
                .with_height(size)
                .finish(),
                ImEntityKind::FoodTea => ConstrainedBox::new(
                    Container::new(
                        Align::new(self.content_icon(UiIcon::Coffee, icon_sz, fg)).finish(),
                    )
                    .with_background_color(rgb(0xEC, 0x48, 0x99))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
                    .finish(),
                )
                .with_width(size)
                .with_height(size)
                .finish(),
                ImEntityKind::Finance => ConstrainedBox::new(
                    Container::new(
                        Align::new(self.content_icon(UiIcon::Shield, icon_sz, fg)).finish(),
                    )
                    .with_background_color(rgb(0x05, 0x96, 0x69))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
                    .finish(),
                )
                .with_width(size)
                .with_height(size)
                .finish(),
                ImEntityKind::Service => ConstrainedBox::new(
                    Container::new(
                        Align::new(self.content_icon(UiIcon::Bell, icon_sz, fg)).finish(),
                    )
                    .with_background_color(rgb(0x3B, 0x82, 0xF6))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
                    .finish(),
                )
                .with_width(size)
                .with_height(size)
                .finish(),
                ImEntityKind::Group => {
                    let bg = contact_avatar_color(lookup_title);
                    ConstrainedBox::new(
                        Container::new(
                            Align::new(self.content_icon(
                                UiIcon::Users,
                                (size * 0.50).round(),
                                ColorU::white(),
                            ))
                            .finish(),
                        )
                        .with_background_color(bg)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
                        .finish(),
                    )
                    .with_width(size)
                    .with_height(size)
                    .finish()
                }
                ImEntityKind::Contact => {
                    let bg = contact_avatar_color(lookup_title);
                    let trimmed = lookup_title.trim();
                    let is_sms_or_code = package_name == "com.android.mms"
                        || app_label == "短信"
                        || ImChannel::from_package(package_name) == ImChannel::Sms
                        || trimmed.chars().any(|c| c.is_ascii_digit());
                    let avatar_icon = if trimmed.starts_with("106")
                        || trimmed.starts_with("95")
                        || trimmed.starts_with("100")
                        || trimmed.chars().any(|c| c.is_ascii_digit())
                        || (is_sms_or_code && (trimmed.contains("短信") || trimmed == "未知号码"))
                    {
                        self.content_icon(UiIcon::Sms, (size * 0.46).round(), ColorU::white())
                    } else {
                        self.content_icon(UiIcon::User, (size * 0.48).round(), ColorU::white())
                    };
                    ConstrainedBox::new(
                        Container::new(Align::new(avatar_icon).finish())
                            .with_background_color(bg)
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
                            .finish(),
                    )
                    .with_width(size)
                    .with_height(size)
                    .finish()
                }
            }
        };

        let (ch_icon, ch_bg, ch_fg) = im_avatar_style(package_name, app_label);
        let badge_sz = 16.0;
        let corner_badge = ConstrainedBox::new(
            Container::new(Align::new(self.content_icon(ch_icon, 9.5, ch_fg)).finish())
                .with_background_color(ch_bg)
                .with_border(Border::all(1.5).with_border_color(p.bg))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(badge_sz * 0.5)))
                .finish(),
        )
        .with_width(badge_sz)
        .with_height(badge_sz)
        .finish();

        Stack::new()
            .with_child(main_box)
            .with_positioned_child(
                corner_badge,
                OffsetPositioning::offset_from_parent(
                    vec2f(2.0, 2.0),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::BottomRight,
                    ChildAnchor::BottomRight,
                ),
            )
            .finish()
    }

    fn render_chat_sender_avatar(&self, t: &ImThread, sender: &str, size: f32) -> El {
        if !t.is_group {
            return self.render_sender_avatar(sender, "", size);
        }
        let p = self.p();
        // Notification largeIcon is usually the group image, not the speaker.
        // Only use a source-provided, member-specific image different from the group image.
        if t.sender_avatars
            .get(sender)
            .is_some_and(|b64| !b64.is_empty() && b64 != &t.avatar_b64)
        {
            if let Some(path) = t
                .sender_avatar_paths
                .get(sender)
                .filter(|path| Some(*path) != t.avatar_path.as_ref())
            {
                return ConstrainedBox::new(
                    Image::new(
                        AssetSource::LocalFile {
                            path: path.to_string_lossy().into_owned(),
                        },
                        CacheOption::BySize,
                    )
                    .cover()
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)))
                    .finish(),
                )
                .with_width(size)
                .with_height(size)
                .finish();
            }
        }
        let initial = sender
            .chars()
            .find(|c| !c.is_whitespace())
            .unwrap_or('人')
            .to_string();
        ConstrainedBox::new(
            Container::new(
                Align::new(self.line_w(initial, BODY, p.text2, Weight::Medium)).finish(),
            )
            .with_background_color(p.tag_bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)))
            .finish(),
        )
        .with_width(size)
        .with_height(size)
        .finish()
    }

    fn render_sender_avatar(&self, sender_name: &str, sample_text: &str, size: f32) -> El {
        let p = self.p();
        let radius = (size * 0.32).round().max(8.);
        let clean_sender = sanitize_ui_display_text(sender_name);
        let lookup_sender = if clean_sender.is_empty() {
            sender_name.trim()
        } else {
            clean_sender.trim()
        };
        if lookup_sender == "我" {
            return ConstrainedBox::new(
                Container::new(
                    Align::new(self.content_icon(UiIcon::User, (size * 0.50).round(), p.accent_fg))
                        .finish(),
                )
                .with_background_color(p.accent)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
                .finish(),
            )
            .with_width(size)
            .with_height(size)
            .finish();
        }

        if let Some(avatar_path) = stepx_companion::push::cached_sender_avatar_path(sender_name)
            .or_else(|| stepx_companion::push::cached_sender_avatar_path(lookup_sender))
            .or_else(|| {
                stepx_companion::push::cached_thread_avatar_path("com.tencent.mm", lookup_sender)
            })
            .filter(|path| cached_is_file(path) && !cached_image_is_black(path))
        {
            let img = Image::new(
                AssetSource::LocalFile {
                    path: avatar_path.to_string_lossy().into_owned(),
                },
                CacheOption::BySize,
            )
            .cover()
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
            .finish();
            return ConstrainedBox::new(
                Container::new(img)
                    .with_background_color(p.surface)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
                    .finish(),
            )
            .with_width(size)
            .with_height(size)
            .finish();
        }

        let kind = classify_im_entity(lookup_sender, sample_text, false, &[]);
        let (ic, bg) = match kind {
            ImEntityKind::Express => (UiIcon::Package, rgb(0xF5, 0x9E, 0x0B)),
            ImEntityKind::FoodTea => (UiIcon::Coffee, rgb(0xEC, 0x48, 0x99)),
            ImEntityKind::Finance => (UiIcon::Shield, rgb(0x05, 0x96, 0x69)),
            ImEntityKind::Service => (UiIcon::Bell, rgb(0x3B, 0x82, 0xF6)),
            ImEntityKind::Group => (UiIcon::Users, contact_avatar_color(lookup_sender)),
            ImEntityKind::Contact => (
                if lookup_sender.chars().any(|c| c.is_ascii_digit()) {
                    UiIcon::Sms
                } else {
                    UiIcon::User
                },
                contact_avatar_color(lookup_sender),
            ),
        };
        let fg = ColorU::white();

        ConstrainedBox::new(
            Container::new(Align::new(self.content_icon(ic, (size * 0.48).round(), fg)).finish())
                .with_background_color(bg)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
                .finish(),
        )
        .with_width(size)
        .with_height(size)
        .finish()
    }

    fn render_im_media_card(
        &self,
        idx: usize,
        m: &ImMessage,
        kind: ImMediaKind,
        caption: &str,
        sender_name: &str,
        t: &ImThread,
    ) -> El {
        let p = self.p();
        let local_preview_path = stepx_companion::push::cached_im_media_path(&t.thread_id, &m.id)
            .map(|path| path.to_string_lossy().into_owned());

        let (card_w, card_h, badge_label, icon_kind) = match kind {
            ImMediaKind::Sticker => {
                if local_preview_path.is_some() {
                    (168.0, 136.0, "动画表情", UiIcon::Sparkles)
                } else {
                    (226.0, 96.0, "动画表情", UiIcon::Sparkles)
                }
            }
            ImMediaKind::Video => (236.0, 148.0, "聊天视频", UiIcon::Video),
            ImMediaKind::Image => (236.0, 148.0, "聊天图片", UiIcon::Image),
        };

        let detail_key = im_detail_key(t, idx);
        let open_action = if let Some(ref thumb_path) = local_preview_path {
            Action::Studio(StudioAction::OpenPath(std::path::PathBuf::from(thumb_path)))
        } else {
            Action::Push(PushAction::SelectNotification(detail_key.clone()))
        };

        let visual_canvas = if let Some(ref thumb_path) = local_preview_path {
            let img_builder = Image::new(
                AssetSource::LocalFile {
                    path: thumb_path.clone(),
                },
                CacheOption::BySize,
            );
            let img = match kind {
                ImMediaKind::Sticker => img_builder.contain().finish(),
                ImMediaKind::Image | ImMediaKind::Video => img_builder
                    .cover()
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
                    .finish(),
            };
            let raw_box = ConstrainedBox::new(
                Container::new(img)
                    .with_uniform_padding(if kind == ImMediaKind::Sticker {
                        6.0
                    } else {
                        2.0
                    })
                    .with_background_color(p.surface)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
                    .finish(),
            )
            .with_width(card_w)
            .with_height(card_h)
            .finish();
            let canvas_state = self.mouse_state(format!("im-media-canvas:{}:{idx}", t.thread_id));
            let click_act = open_action.clone();
            Hoverable::new(canvas_state, move |_| raw_box)
                .without_hover_animation()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(click_act.clone());
                })
                .finish()
        } else {
            let labels = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(3.)
                .with_children(vec![
                    self.line_w(badge_label, CAPTION, p.text, Weight::Medium),
                    self.line("暂无预览", MICRO, p.text3),
                ])
                .finish();
            let preview = ConstrainedBox::new(
                Container::new(self.hstack(
                    10.,
                    vec![self.content_icon(icon_kind, 22., p.text2), labels],
                ))
                .with_uniform_padding(10.)
                .finish(),
            )
            .with_width(144.)
            .finish();
            let canvas_state = self.mouse_state(format!("im-media-canvas:{}:{idx}", t.thread_id));
            let click_act = open_action.clone();
            Hoverable::new(canvas_state, move |_| preview)
                .without_hover_animation()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(click_act.clone()))
                .finish()
        };

        let _ = sender_name;
        let mut rows: Vec<El> = vec![visual_canvas];
        if !caption.is_empty() {
            rows.push(self.chat_bubble_text_node(
                &format!("{badge_label}：{caption}"),
                CAPTION,
                p.text,
                card_w,
            ));
        } else if local_preview_path.is_some() && kind == ImMediaKind::Sticker {
            rows.push(self.pill_badge("动画表情", p.accent, p.accent_soft));
        }

        let card_col = Flex::column()
            .with_spacing(6.0)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_children(rows)
            .finish();
        let card = Container::new(card_col)
            .with_uniform_padding(8.0)
            .with_background_color(if p.dark {
                rgb(0x1E, 0x1E, 0x24)
            } else {
                rgb(0xF8, 0xFA, 0xFC)
            })
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .finish();
        #[cfg(feature = "ui-capture")]
        let card = crate::ui::scenario_check::control(format!("im-media-card-{idx}"), card);
        card
    }

    fn render_im_voice_card(
        &self,
        idx: usize,
        secs: u32,
        sender_name: &str,
        outgoing: bool,
        t: &ImThread,
    ) -> El {
        let p = self.p();
        let play_bg = if outgoing {
            ColorU::white()
        } else {
            rgb(0x07, 0xC1, 0x60)
        };
        let play_fg = if outgoing { p.accent } else { ColorU::white() };
        let play_circle = ConstrainedBox::new(
            Container::new(Align::new(self.content_icon(UiIcon::Play, 14., play_fg)).finish())
                .with_background_color(play_bg)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(14.)))
                .finish(),
        )
        .with_width(28.)
        .with_height(28.)
        .finish();

        const WAVE_HEIGHTS: [f32; 14] = [
            6.0, 12.0, 18.0, 10.0, 20.0, 14.0, 8.0, 19.0, 15.0, 11.0, 17.0, 9.0, 13.0, 7.0,
        ];
        let bar_color = if outgoing {
            ColorU::white()
        } else {
            rgb(0x07, 0xC1, 0x60)
        };
        let mut wave_bars: Vec<El> = Vec::with_capacity(14);
        for (b_i, &base_h) in WAVE_HEIGHTS.iter().enumerate() {
            let h = if (idx + b_i) % 3 == 0 {
                (base_h * 0.85).max(5.0)
            } else {
                base_h
            };
            wave_bars.push(
                ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background_color(bar_color)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(1.5)))
                        .finish(),
                )
                .with_width(3.0)
                .with_height(h)
                .finish(),
            );
        }
        let waveform = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(2.5)
            .with_children(wave_bars)
            .finish();

        let dur_label = self.mono(
            format!("{secs}\""),
            SUBHEAD,
            if outgoing { ColorU::white() } else { p.text },
        );

        let top_player_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(10.0)
            .with_child(play_circle)
            .with_child(waveform)
            .with_child(dur_label)
            .with_child(self.dot(rgb(0x10, 0xB9, 0x81)))
            .finish();

        let voice_id = format!("im-voice-{}-{idx}", t.thread_id);
        let phone_key = if !t.notification_key.is_empty() {
            t.notification_key.clone()
        } else {
            t.thread_id.clone()
        };

        let bottom_actions = self.wrap(vec![
            self.button_icon(
                format!("im-voice-play-{idx}"),
                format!("播放语音 ({secs}s)"),
                UiIcon::Speaker,
                Action::Speak(
                    voice_id,
                    format!(
                        "来自 {} 的 {} 秒微信语音消息，可在手机端收听原声",
                        sender_name, secs
                    ),
                ),
                Btn::Secondary,
            ),
            self.button_icon(
                format!("im-voice-phone-{idx}"),
                "手机听原声",
                UiIcon::Smartphone,
                Action::Push(PushAction::OpenOnPhone(phone_key)),
                Btn::Ghost,
            ),
            self.button_icon(
                format!("im-voice-ai-{idx}"),
                "AI 转写/回复",
                UiIcon::Sparkles,
                Action::SendToAmoo(format!(
                    "请帮我跟进并拟定回复：{} 在「{}」发来了一条 {} 秒的语音消息",
                    sender_name, t.title, secs
                )),
                Btn::Secondary,
            ),
        ]);

        let card_col = Flex::column()
            .with_spacing(6.0)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_children(vec![
                top_player_row,
                self.line(
                    format!("语音消息 ({secs}秒) · 支持桌面播报、手机收听"),
                    MICRO,
                    if outgoing { p.accent_fg } else { p.text2 },
                ),
                bottom_actions,
            ])
            .finish();
        let card = Container::new(card_col)
            .with_uniform_padding(8.0)
            .with_background_color(if outgoing {
                mix_color(p.accent, ColorU::black(), 0.12)
            } else if p.dark {
                rgb(0x1E, 0x24, 0x22)
            } else {
                rgb(0xF0, 0xFD, 0xF4)
            })
            .with_border(Border::all(1.).with_border_color(if outgoing {
                p.accent
            } else {
                rgb(0x86, 0xEF, 0xAC)
            }))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .finish();
        #[cfg(feature = "ui-capture")]
        let card = crate::ui::scenario_check::control(format!("im-voice-card-{idx}"), card);
        card
    }

    fn render_im_rich_modal_card(
        &self,
        idx: usize,
        kind: ImRichModalKind,
        caption: &str,
        _sender_name: &str,
        t: &ImThread,
    ) -> El {
        let p = self.p();
        let (icon, badge_text, tint, tint_soft, open_label) = match kind {
            ImRichModalKind::File => (
                UiIcon::File,
                "文件卡片",
                rgb(0x25, 0x63, 0xEB),
                if p.dark {
                    rgb(0x17, 0x25, 0x54)
                } else {
                    rgb(0xDB, 0xEA, 0xFE)
                },
                "手机打开文件",
            ),
            ImRichModalKind::MiniProgram => (
                UiIcon::Apps,
                "小程序卡片",
                rgb(0x7C, 0x3A, 0xED),
                if p.dark {
                    rgb(0x2E, 0x10, 0x65)
                } else {
                    rgb(0xED, 0xE9, 0xFE)
                },
                "手机打开小程序",
            ),
            ImRichModalKind::Location => (
                UiIcon::Pin,
                "位置共享",
                rgb(0x05, 0x96, 0x69),
                if p.dark {
                    rgb(0x06, 0x4E, 0x3B)
                } else {
                    rgb(0xD1, 0xFA, 0xE5)
                },
                "手机查看位置",
            ),
            ImRichModalKind::RedPacket => (
                UiIcon::Gift,
                "微信红包",
                rgb(0xE1, 0x1D, 0x48),
                if p.dark {
                    rgb(0x4C, 0x05, 0x19)
                } else {
                    rgb(0xFF, 0xE4, 0xE6)
                },
                "手机拆开红包",
            ),
            ImRichModalKind::Transfer => (
                UiIcon::CreditCard,
                "微信转账",
                rgb(0xD9, 0x77, 0x06),
                if p.dark {
                    rgb(0x45, 0x1A, 0x03)
                } else {
                    rgb(0xFE, 0xF3, 0xC7)
                },
                "手机确认收款",
            ),
        };

        let icon_box = ConstrainedBox::new(
            Container::new(Align::new(self.content_icon(icon, 18.0, tint)).finish())
                .with_background_color(tint_soft)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.0)))
                .finish(),
        )
        .with_width(36.0)
        .with_height(36.0)
        .finish();

        let phone_key = if !t.notification_key.is_empty() {
            t.notification_key.clone()
        } else {
            t.thread_id.clone()
        };

        let top_row = self.hstack(
            10.0,
            vec![
                icon_box,
                Flex::column()
                    .with_spacing(3.0)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_child(self.pill_badge_icon(badge_text, icon, tint, tint_soft))
                    .with_child(self.chat_bubble_text_node(caption, BODY, p.text, 260.0))
                    .finish(),
            ],
        );

        let actions = self.wrap(vec![
            self.button_icon(
                format!("im-modal-phone-{idx}"),
                open_label,
                UiIcon::Smartphone,
                Action::Push(PushAction::OpenOnPhone(phone_key)),
                Btn::Secondary,
            ),
            self.button_icon(
                format!("im-modal-detail-{idx}"),
                "查看详情",
                UiIcon::Search,
                Action::Push(PushAction::SelectNotification(im_detail_key(t, idx))),
                Btn::Ghost,
            ),
        ]);

        Container::new(
            Flex::column()
                .with_spacing(8.0)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_children(vec![top_row, actions])
                .finish(),
        )
        .with_uniform_padding(10.0)
        .with_background_color(p.surface)
        .with_border(Border::all(1.0).with_border_color(tint))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.0)))
        .finish()
    }

    fn render_im_service_notice_card(
        &self,
        idx: usize,
        notice: &ImServiceNoticeData,
        sender_name: &str,
        t: &ImThread,
    ) -> El {
        let p = self.p();
        let _ = sender_name;
        let (icon, tint, tint_soft) = match notice.kind {
            ImEntityKind::Express => (
                UiIcon::Package,
                rgb(0xF5, 0x9E, 0x0B),
                if p.dark {
                    rgb(0x3B, 0x2A, 0x10)
                } else {
                    rgb(0xFE, 0xF3, 0xC7)
                },
            ),
            ImEntityKind::FoodTea => (
                UiIcon::Coffee,
                rgb(0xEC, 0x48, 0x99),
                if p.dark {
                    rgb(0x3D, 0x18, 0x2B)
                } else {
                    rgb(0xFC, 0xE7, 0xF3)
                },
            ),
            ImEntityKind::Finance => (
                UiIcon::CreditCard,
                rgb(0x05, 0x96, 0x69),
                if p.dark {
                    rgb(0x0E, 0x31, 0x26)
                } else {
                    rgb(0xD1, 0xFA, 0xE5)
                },
            ),
            _ => (
                UiIcon::Bell,
                rgb(0x3B, 0x82, 0xF6),
                if p.dark {
                    rgb(0x17, 0x25, 0x54)
                } else {
                    rgb(0xDB, 0xEA, 0xFE)
                },
            ),
        };

        let mut header_left = vec![
            self.pill_badge_icon(notice.category_badge, icon, tint, tint_soft),
            self.chat_bubble_text_node(&notice.headline, BODY, p.text, 240.0),
        ];
        if let Some((code, pickup)) = &notice.highlight_code {
            header_left.push(self.pill_badge_icon(
                if *pickup {
                    format!("取件码 {code}")
                } else {
                    format!("凭据码 {code}")
                },
                if *pickup {
                    UiIcon::Package
                } else {
                    UiIcon::Key
                },
                tint,
                tint_soft,
            ));
        }

        let mut card_rows: Vec<El> = vec![self.wrap(header_left)];
        if !notice.kv_rows.is_empty() {
            for (k, v) in notice.kv_rows.iter().take(4) {
                card_rows.push(
                    Flex::row()
                        .with_spacing(8.0)
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_child(
                            ConstrainedBox::new(self.line_w(
                                format!("{k}："),
                                CAPTION,
                                p.text3,
                                Weight::Medium,
                            ))
                            .with_width(68.0)
                            .finish(),
                        )
                        .with_child(self.chat_bubble_text_node(v, CAPTION, p.text, 250.0))
                        .finish(),
                );
            }
        }

        let mut card_btns: Vec<El> = Vec::new();
        if let Some((code, pickup)) = &notice.highlight_code {
            card_btns.push(self.button_icon(
                format!("im-srv-copy-{idx}"),
                if *pickup {
                    format!("复制取件码 {code}")
                } else {
                    format!("复制 {code}")
                },
                UiIcon::Copy,
                Action::CopyText(code.clone(), format!("已复制 {code}")),
                Btn::Primary,
            ));
            if *pickup {
                card_btns.push(self.button_icon(
                    format!("im-srv-backscreen-{idx}"),
                    "投送背屏",
                    UiIcon::Matrix,
                    Action::AmooPushPickupCode(code.clone()),
                    Btn::Secondary,
                ));
            }
        }
        let phone_key = if !t.notification_key.is_empty() {
            t.notification_key.clone()
        } else {
            t.thread_id.clone()
        };
        card_btns.push(self.button_icon(
            format!("im-srv-detail-{idx}"),
            "查看详情",
            UiIcon::Search,
            Action::Push(PushAction::SelectNotification(im_detail_key(t, idx))),
            Btn::Secondary,
        ));
        card_btns.push(self.button_icon(
            format!("im-srv-phone-{idx}"),
            "手机查看",
            UiIcon::Smartphone,
            Action::Push(PushAction::OpenOnPhone(phone_key)),
            Btn::Ghost,
        ));
        card_rows.push(self.wrap(card_btns));

        Container::new(
            Flex::column()
                .with_spacing(6.0)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_children(card_rows)
                .finish(),
        )
        .with_uniform_padding(10.0)
        .with_background_color(p.surface)
        .with_border(Border::all(1.0).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.0)))
        .finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_im_message_detail_card(
        &self,
        idx: usize,
        sender_name: &str,
        ts: &str,
        full_text: &str,
        via: &str,
        _outgoing: bool,
        t: &ImThread,
    ) -> El {
        let p = self.p();
        let via_label = match via {
            "ui_captured" | "accessibility" => "应用页面已读取内容",
            "clipboard" => "手机跨端剪贴板",
            "reply" | "desktop" => "桌面直接发送",
            _ => "通知栏实时推送",
        };
        let header_meta = if ts.is_empty() {
            format!("消息详情 · {sender_name} · {via_label}")
        } else {
            format!("消息详情 · {sender_name} · {ts} · {via_label}")
        };
        let phone_key = if !t.notification_key.is_empty() {
            t.notification_key.clone()
        } else {
            t.thread_id.clone()
        };
        let detail_key = im_detail_key(t, idx);

        let mut action_btns = vec![
            self.button_icon(
                format!("im-detail-copy-{idx}"),
                "复制全文",
                UiIcon::Copy,
                Action::CopyText(full_text.to_string(), "已复制完整消息".into()),
                Btn::Secondary,
            ),
            self.button_icon(
                format!("im-detail-quote-{idx}"),
                "引用回复",
                UiIcon::Reply,
                Action::Push(PushAction::QuoteReply {
                    is_im: true,
                    sender: sender_name.to_string(),
                    text: full_text.to_string(),
                }),
                Btn::Secondary,
            ),
            self.button_icon(
                format!("im-detail-sync-{idx}"),
                "读取窗口",
                UiIcon::Refresh,
                Action::Push(PushAction::SyncActivePhoneChatWindow),
                Btn::Secondary,
            ),
            self.button_icon(
                format!("im-detail-phone-{idx}"),
                "手机打开",
                UiIcon::Smartphone,
                Action::Push(PushAction::OpenOnPhone(phone_key)),
                Btn::Ghost,
            ),
            self.button_icon(
                format!("im-detail-close-{idx}"),
                "收起",
                UiIcon::Close,
                Action::Push(PushAction::SelectNotification(detail_key)),
                Btn::Ghost,
            ),
        ];
        if let Some(code) = extract_verification_code(full_text) {
            let pickup = is_pickup_code_text(&code, full_text);
            action_btns.insert(
                0,
                self.button_icon(
                    format!("im-detail-otp-{idx}"),
                    if pickup {
                        format!("复制取件码 {code}")
                    } else {
                        format!("复制验证码 {code}")
                    },
                    if pickup { UiIcon::Package } else { UiIcon::Key },
                    Action::CopyText(code.clone(), format!("已复制 {code}")),
                    Btn::Primary,
                ),
            );
        }

        let card = Container::new(
            Flex::column()
                .with_spacing(8.0)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(self.line_w(header_meta, MICRO, p.accent, Weight::Semibold))
                .with_child(
                    ConstrainedBox::new(self.pane_scroll(
                        self.sms_detail_scroll.clone(),
                        self.chat_bubble_text_node(full_text, BODY, p.text, 460.0),
                    ))
                    .with_max_height(240.0)
                    .finish(),
                )
                .with_child(self.wrap(action_btns))
                .finish(),
        )
        .with_uniform_padding(12.0)
        .with_background_color(p.surface)
        .with_border(Border::all(1.0).with_border_color(p.accent))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.0)))
        .finish();

        card
    }

    fn render_quote_preview_bar(&self, is_sms: bool) -> Option<El> {
        let (ref q_sender, ref q_text, q_is_sms) = *self.push.reply_quote.as_ref()?;
        if q_is_sms != is_sms {
            return None;
        }
        let p = self.p();
        let clean_sender = sanitize_ui_display_text(q_sender);
        let clean_text: String = sanitize_ui_display_text(q_text)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let snippet = if clean_text.chars().count() > 54 {
            format!("{}…", clean_text.chars().take(54).collect::<String>())
        } else {
            clean_text
        };
        let label_prefix = if clean_sender.is_empty() {
            "引用消息：".to_string()
        } else {
            format!("引用 {clean_sender}：")
        };
        let accent_bar = ConstrainedBox::new(
            Container::new(Empty::new().finish())
                .with_background_color(p.accent)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(1.5)))
                .finish(),
        )
        .with_width(3.0)
        .with_height(18.0)
        .finish();
        let left = self.hstack(
            6.0,
            vec![
                accent_bar,
                self.content_icon(UiIcon::Quote, 13.0, p.accent),
                self.line_w(label_prefix, MICRO, p.accent, Weight::Semibold),
                self.line(snippet, MICRO, p.text2),
            ],
        );
        let close_id = if is_sms {
            "sms-clear-quote"
        } else {
            "im-clear-quote"
        };
        let close_btn = self.msg_inline_svg_action_btn(
            close_id,
            UiIcon::Close,
            false,
            Action::Push(PushAction::ClearQuoteReply),
        );
        Some(
            Container::new(self.between(left, close_btn))
                .with_horizontal_padding(10.0)
                .with_vertical_padding(5.0)
                .with_background_color(p.surface)
                .with_border(Border::all(1.0).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.0)))
                .finish(),
        )
    }

    fn chat_bubble_radius(outgoing: bool, grouped_with_prev: bool) -> CornerRadius {
        let mut r = CornerRadius::with_all(Radius::Pixels(14.0));
        if !grouped_with_prev {
            if outgoing {
                r.merge(CornerRadius::with_top_right(Radius::Pixels(4.0)));
            } else {
                r.merge(CornerRadius::with_top_left(Radius::Pixels(4.0)));
            }
        }
        r
    }

    fn chat_bubble_text_node(
        &self,
        text: &str,
        font_size: f32,
        color: ColorU,
        max_text_width: f32,
    ) -> El {
        // Respect the actual conversation width, including narrow windows. Text
        // width estimates cannot decide whether wrapping is safe for this layout.
        ConstrainedBox::new(
            facloud_ui::elements::Text::new(text.to_string(), self.font, font_size)
                .with_color(color)
                .with_line_height_ratio(1.42)
                .soft_wrap(true)
                .finish(),
        )
        .with_max_width(max_text_width)
        .finish()
    }

    fn im_thread_more_button(&self, t: &ImThread) -> El {
        let p = self.p();
        let target = crate::ui::context_menu::Target::ImThread {
            thread_id: t.thread_id.clone(),
            package_name: t.package_name.clone(),
            title: t.title.clone(),
            last_message: t.last_message.clone(),
            notification_key: t.notification_key.clone(),
            otp_code: extract_verification_code(&t.last_message),
        };
        let button = Hoverable::new(self.mouse_state("im-thread-more"), move |mouse| {
            ConstrainedBox::new(
                Container::new(
                    Align::new(self.content_icon(UiIcon::MoreHorizontal, 18., p.text2)).finish(),
                )
                .with_background_color(if mouse.is_mouse_over_element() {
                    p.hover
                } else {
                    p.bg
                })
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish(),
            )
            .with_width(32.)
            .with_height(32.)
            .finish()
        })
        .without_hover_animation()
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(target.clone(), at))
        })
        .finish();
        #[cfg(feature = "ui-capture")]
        let button = crate::ui::scenario_check::control("im-thread-more".into(), button);
        button
    }

    fn im_thread_dialog(&self, t: &ImThread, _snapshot: &PushSnapshot) -> El {
        let p = self.p();
        let app_label = if t.app_name.is_empty() {
            t.channel().label()
        } else {
            t.app_name.as_str()
        };

        let is_msg_outgoing = crate::ui::message_window::is_outgoing;
        let follow_latest = SMS_SCROLL_STATE.with(|cell| {
            let st = cell.borrow();
            st.thread_key != format!("im:{}", t.thread_id)
                || st.content_height
                    - st.viewport_height
                    - self.sms_messages_scroll.scroll_start().as_f32()
                    <= 96.
        });
        let visible = self
            .push
            .message_window
            .borrow_mut()
            .visible_range(t, follow_latest);

        let mut participants: Vec<String> = Vec::new();
        for m in t.messages.iter().rev().take(80) {
            if participants.len() >= 5 {
                break;
            }
            let s = sanitize_ui_display_text(m.sender.trim());
            if !is_msg_outgoing(m)
                && !s.is_empty()
                && !matches!(s.as_str(), "我" | "Me" | "me" | "自己")
                && !stepx_companion::push::is_generic_sender_name(&s, &t.title)
                && !participants.iter().any(|p| p == &s)
            {
                participants.push(s);
            }
        }
        let last_sender_clean = sanitize_ui_display_text(t.last_sender.trim());
        if participants.is_empty()
            && !last_sender_clean.is_empty()
            && !matches!(last_sender_clean.as_str(), "我" | "Me" | "me" | "自己")
            && !stepx_companion::push::is_generic_sender_name(&last_sender_clean, &t.title)
        {
            participants.push(last_sender_clean);
        }

        let clean_thread_title = sanitize_ui_display_text(&t.title);
        let display_group_title = if stepx_companion::push::is_generic_im_title(&clean_thread_title)
            && !participants.is_empty()
        {
            format!(
                "{}（{}）",
                if clean_thread_title.trim().is_empty() {
                    "群聊"
                } else {
                    clean_thread_title.trim()
                },
                participants.join("、")
            )
        } else if clean_thread_title.trim().is_empty() {
            format!("{app_label}会话")
        } else {
            clean_thread_title
        };

        let header_avatar = self.render_im_entity_avatar(
            &t.package_name,
            app_label,
            &display_group_title,
            &t.last_message,
            t.is_group,
            &t.messages,
            38.0,
        );

        let im_pin_key =
            stepx_companion::desktop_notify::NotifyPreferences::im_target_key(&t.thread_id);
        let im_pinned = self.push.prefs.is_pinned(&im_pin_key);
        let mut title_pills = Vec::new();
        if im_pinned {
            title_pills.push(self.pill_badge_icon("已置顶", UiIcon::Pin, p.accent, p.accent_soft));
        }
        if t.unread_count > 0 {
            title_pills.push(self.pill_badge(
                format!("{} 条未读", t.unread_count),
                ColorU::white(),
                p.danger,
            ));
        }
        let title_row = self.between(
            self.line_w(&display_group_title, SUBHEAD, p.text, Weight::Semibold),
            self.hstack(6., title_pills),
        );

        let latest_im_otp = t
            .messages
            .iter()
            .rev()
            .find_map(|m| {
                extract_verification_code(&m.text)
                    .map(|code| (code.clone(), is_pickup_code_text(&code, &m.text)))
            })
            .or_else(|| {
                extract_verification_code(&t.last_message)
                    .map(|code| (code.clone(), is_pickup_code_text(&code, &t.last_message)))
            });

        let mut top_hdr_btns = Vec::new();
        if let Some((code, pickup)) = &latest_im_otp {
            top_hdr_btns.push(self.button_icon(
                "im-hdr-copy-otp",
                if *pickup {
                    format!("复制取件码: {code}")
                } else {
                    format!("复制验证码: {code}")
                },
                if *pickup {
                    UiIcon::Package
                } else {
                    UiIcon::Copy
                },
                Action::CopyText(
                    code.clone(),
                    if *pickup {
                        format!("已复制取件码 {code}")
                    } else {
                        format!("已复制验证码 {code}")
                    },
                ),
                Btn::Primary,
            ));
        }
        top_hdr_btns.push(self.icon_action(
            "im-hdr-sync-window",
            "读取手机当前会话窗口",
            UiIcon::Refresh,
            vec![Action::Push(PushAction::SyncActivePhoneChatWindow)],
            false,
        ));
        top_hdr_btns.push(self.icon_action(
            "im-hdr-open-phone",
            "投屏打开手机",
            UiIcon::Monitor,
            vec![Action::Mirror],
            false,
        ));

        let common_hdr_btns = vec![self.im_thread_more_button(t)];

        let subtitle_text = {
            let mut parts = vec![if t.is_group {
                format!("{app_label}群聊")
            } else {
                app_label.to_string()
            }];
            if t.last_timestamp_ms > 0 {
                parts.push(format!(
                    "最近活跃 {}",
                    super::format_time(t.last_timestamp_ms)
                ));
            }
            parts.push(format!("{} 条已同步", t.messages.len()));
            parts.join(" · ")
        };

        let mut all_hdr_btns = top_hdr_btns;
        all_hdr_btns.extend(common_hdr_btns);

        let top_header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(10.)
            .with_child(header_avatar)
            .with_child(
                Expanded::new(
                    1.,
                    self.vstack(
                        2.,
                        vec![title_row, self.line(subtitle_text, MICRO, p.text3)],
                    ),
                )
                .finish(),
            )
            .with_child(self.hstack(4., all_hdr_btns))
            .finish();

        let mut header_rows = vec![top_header_row];
        if t.notification_history_only() {
            header_rows.push(self.line(
                "仅通知记录，不含手机发出的消息；“读取窗口”可补充手机当前可见内容。",
                MICRO,
                p.text2,
            ));
        }
        let dialog_header = Container::new(self.vstack(6., header_rows))
            .with_horizontal_padding(S5)
            .with_vertical_padding(12.)
            .with_background_color(p.bg)
            .with_border(Border::bottom(1.).with_border_color(p.border))
            .finish();

        let mut msg_items: Vec<El> = Vec::new();
        {
            // Always reserve this row; new arrivals never insert controls above the reader.
            let mut history_actions = Vec::new();
            if visible.start > 0 {
                let first = crate::ui::message_window::message_key(
                    &t.messages[visible.start.saturating_sub(80)],
                );
                history_actions.push(self.button(
                    "im-history-older",
                    "更早消息",
                    Action::Push(PushAction::ImHistoryPage {
                        thread_id: t.thread_id.clone(),
                        first: Some(first),
                    }),
                    Btn::Ghost,
                ));
            }
            history_actions.push(self.line(
                format!(
                    "{}–{} / {} 条",
                    if t.messages.is_empty() {
                        0
                    } else {
                        visible.start + 1
                    },
                    visible.end,
                    t.messages.len()
                ),
                MICRO,
                p.text3,
            ));
            if visible.end < t.messages.len() {
                history_actions.push(self.button(
                    "im-history-newer",
                    "后续消息",
                    Action::Push(PushAction::ImHistoryPage {
                        thread_id: t.thread_id.clone(),
                        first: Some(crate::ui::message_window::message_key(
                            &t.messages[visible.end],
                        )),
                    }),
                    Btn::Ghost,
                ));
                history_actions.push(self.button(
                    "im-history-latest",
                    "回到最新",
                    Action::Push(PushAction::ImHistoryPage {
                        thread_id: t.thread_id.clone(),
                        first: None,
                    }),
                    Btn::Secondary,
                ));
            }
            msg_items.push(
                ConstrainedBox::new(self.hstack(8., history_actions))
                    .with_height(30.)
                    .finish(),
            );
        }

        for idx in visible.clone() {
            let m = &t.messages[idx];
            let is_out = is_msg_outgoing(m);
            let resolved_sender = sanitize_ui_display_text(crate::ui::message_window::sender(t, m));
            let ts = m
                .occurred_at_ms()
                .map(super::format_time)
                .unwrap_or_default();
            let grouped_with_prev = idx > visible.start
                && crate::ui::message_window::same_burst(t, &t.messages[idx - 1], m);

            let (raw_msg_text, recalled_badge, raw_media_badge) =
                format_im_message_content(m, false);
            let msg_text = humanize_wechat_emojis(&raw_msg_text);
            let copy_payload = if raw_msg_text.trim().is_empty() {
                sanitize_ui_display_text(&m.text)
            } else {
                raw_msg_text.clone()
            };

            let voice_secs = extract_voice_duration_secs(&m.text, &m.media_hint);
            let media_info = if voice_secs.is_some() {
                None
            } else {
                detect_im_media_kind(&m.text, &m.media_hint)
            };
            let rich_modal_info = if voice_secs.is_some() || media_info.is_some() {
                None
            } else {
                detect_im_rich_modal(&m.text, &m.media_hint)
            };
            let media_badge =
                if voice_secs.is_none() && media_info.is_none() && rich_modal_info.is_none() {
                    raw_media_badge
                } else {
                    None
                };

            let top_gap = if idx == visible.start {
                0.0
            } else if grouped_with_prev {
                4.0
            } else {
                14.0
            };

            if is_out {
                let status_suffix = match m.delivery_status.as_str() {
                    "sending" => Some("发送中"),
                    "failed" => Some("发送失败"),
                    _ => None,
                };
                let meta = match (ts.is_empty(), status_suffix) {
                    (true, Some(st)) => st.to_string(),
                    (false, Some(st)) => format!("{ts} · {st}"),
                    (false, None) => ts.clone(),
                    (true, None) => String::new(),
                };
                let mut bubble_children = Vec::new();
                if recalled_badge.is_some() || media_badge.is_some() {
                    let mut badges = Vec::new();
                    if let Some(rb) = recalled_badge {
                        badges.push(self.pill_badge(rb, p.warning, p.warning_soft));
                    }
                    if let Some(mb) = &media_badge {
                        badges.push(self.pill_badge(mb, p.accent, p.accent_soft));
                    }
                    bubble_children.push(self.hstack(6., badges));
                }
                if let Some(secs) = voice_secs {
                    bubble_children.push(self.render_im_voice_card(idx, secs, "我", true, t));
                } else if let Some((mkind, mcap)) = &media_info {
                    bubble_children.push(self.render_im_media_card(idx, m, *mkind, mcap, "我", t));
                } else if let Some((rkind, rcap)) = &rich_modal_info {
                    bubble_children
                        .push(self.render_im_rich_modal_card(idx, *rkind, rcap, "我", t));
                } else {
                    bubble_children.push(self.chat_bubble_text_node(
                        &msg_text,
                        BODY,
                        if m.recalled { p.text2 } else { p.accent_fg },
                        440.0,
                    ));
                }
                let bubble_inner_col = Flex::column()
                    .with_spacing(4.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_children(bubble_children)
                    .finish();
                let is_standalone_card = media_info.is_some() || rich_modal_info.is_some();
                let bubble_box = if is_standalone_card {
                    bubble_inner_col
                } else {
                    Container::new(bubble_inner_col)
                        .with_horizontal_padding(14.)
                        .with_vertical_padding(9.)
                        .with_background_color(if m.recalled { p.surface } else { p.accent })
                        .with_corner_radius(Self::chat_bubble_radius(true, grouped_with_prev))
                        .finish()
                };
                let out_key = format!("im-msg-out:{}", im_detail_key(t, idx));
                let retry_id = format!("im-out-retry-{idx}");
                let mut col_stack = Flex::column()
                    .with_spacing(3.)
                    .with_cross_axis_alignment(CrossAxisAlignment::End)
                    .with_child(self.hstack(
                        6.,
                        vec![
                            self.msg_inline_svg_action_btn(
                                format!("im-msg-info-{idx}"),
                                UiIcon::MoreHorizontal,
                                false,
                                Action::Push(PushAction::SelectNotification(im_detail_key(t, idx))),
                            ),
                            Shrinkable::new(1., bubble_box).finish(),
                        ],
                    ));

                {
                    // Stable metadata and action row; hover never changes its dimensions.
                    let mut meta_children = Vec::new();
                    if !meta.is_empty() && (!grouped_with_prev || !m.delivery_status.is_empty()) {
                        meta_children.push(
                            Container::new(self.mono(
                                meta,
                                MICRO,
                                if m.delivery_status == "failed" {
                                    p.danger
                                } else {
                                    p.text3
                                },
                            ))
                            .with_padding_right(2.)
                            .finish(),
                        );
                    }
                    if m.delivery_status == "failed" {
                        meta_children.push(self.msg_inline_svg_action_btn(
                            retry_id,
                            UiIcon::Refresh,
                            false,
                            Action::Push(PushAction::QuickReplyPhrase {
                                thread_id: t.thread_id.clone(),
                                package_name: t.package_name.clone(),
                                title: t.title.clone(),
                                text: copy_payload.clone(),
                            }),
                        ));
                    }
                    if !meta_children.is_empty() {
                        col_stack = col_stack.with_child(
                            ConstrainedBox::new(
                                Flex::row()
                                    .with_spacing(5.)
                                    .with_main_axis_alignment(MainAxisAlignment::End)
                                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                    .with_children(meta_children)
                                    .finish(),
                            )
                            .with_height(18.0)
                            .finish(),
                        );
                    }
                }

                let right_avatar_slot = if !grouped_with_prev {
                    self.render_sender_avatar("我", "", 30.0)
                } else {
                    ConstrainedBox::new(Empty::new().finish())
                        .with_width(30.0)
                        .with_height(1.0)
                        .finish()
                };
                let out_row = Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::End)
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_spacing(8.)
                        .with_child(Shrinkable::new(1., col_stack.finish()).finish())
                        .with_child(right_avatar_slot)
                        .finish(),
                )
                .with_padding_top(top_gap)
                .finish();
                let state = self.mouse_state(out_key);
                let txt_dbl = copy_payload.clone();
                let otp_rc = extract_verification_code(&copy_payload);
                let tid_rc = t.thread_id.clone();
                let pkg_rc = t.package_name.clone();
                let title_rc = t.title.clone();
                let mid_rc = m.id.clone();
                let txt_rc = copy_payload.clone();
                let nkey_rc = t.notification_key.clone();
                let out_bubble_el = Hoverable::new(state, move |_| out_row)
                    .without_hover_animation()
                    .with_defer_events_to_children()
                    .on_double_click(move |ctx, _, _| {
                        if let Some(code) = extract_verification_code(&txt_dbl) {
                            let pickup = is_pickup_code_text(&code, &txt_dbl);
                            ctx.dispatch_typed_action(Action::CopyText(
                                code.clone(),
                                if pickup {
                                    format!("已复制取件码 {code}")
                                } else {
                                    format!("已复制验证码 {code}")
                                },
                            ));
                        } else {
                            ctx.dispatch_typed_action(Action::CopyText(
                                txt_dbl.clone(),
                                "已复制消息内容".into(),
                            ));
                        }
                    })
                    .on_right_click(move |ctx, _, at| {
                        ctx.dispatch_typed_action(Action::ContextOpen(
                            crate::ui::context_menu::Target::ImMessage {
                                thread_id: tid_rc.clone(),
                                package_name: pkg_rc.clone(),
                                title: title_rc.clone(),
                                msg_id: mid_rc.clone(),
                                sender: "我".into(),
                                text: txt_rc.clone(),
                                notification_key: nkey_rc.clone(),
                                otp_code: otp_rc.clone(),
                            },
                            at,
                        ));
                    })
                    .finish();
                #[cfg(feature = "ui-capture")]
                let out_bubble_el = crate::ui::scenario_check::control(
                    format!("im-msg-bubble-{idx}"),
                    out_bubble_el,
                );
                msg_items.push(out_bubble_el);
            } else {
                let sender_name = resolved_sender.as_str();
                let meta = if ts.is_empty() {
                    sender_name.to_string()
                } else {
                    format!("{sender_name} · {ts}")
                };

                let in_key = format!("im-msg-in:{}", im_detail_key(t, idx));
                let otp_id = format!("im-msg-otp-{idx}");
                let url_id = format!("im-msg-url-{idx}");
                let backscreen_id = format!("im-msg-backscreen-{idx}");
                let detected_code = extract_verification_code(&m.text);
                let detected_url = extract_first_url(&m.text);

                let mut meta_items = vec![self.line_w(meta, MICRO, p.text3, Weight::Medium)];
                if let Some(rb) = recalled_badge {
                    meta_items.push(self.pill_badge(rb, p.warning, p.warning_soft));
                }
                if let Some(mb) = &media_badge {
                    meta_items.push(self.pill_badge(mb, p.accent, p.accent_soft));
                }
                if let Some(code) = detected_code {
                    let pickup = is_pickup_code_text(&code, &m.text);
                    meta_items.push(self.msg_inline_svg_action_btn(
                        otp_id,
                        if pickup {
                            UiIcon::Package
                        } else {
                            UiIcon::Copy
                        },
                        true,
                        Action::CopyText(
                            code.clone(),
                            if pickup {
                                format!("已复制取件码 {code}")
                            } else {
                                format!("已复制验证码 {code}")
                            },
                        ),
                    ));
                    if pickup {
                        meta_items.push(self.msg_inline_svg_action_btn(
                            backscreen_id,
                            UiIcon::Matrix,
                            false,
                            Action::AmooPushPickupCode(code),
                        ));
                    }
                }
                if let Some(url) = detected_url {
                    meta_items.push(self.msg_inline_svg_action_btn(
                        url_id,
                        UiIcon::ExternalLink,
                        false,
                        Action::StudioOpenUrl(url),
                    ));
                }
                let service_notice =
                    if voice_secs.is_some() || media_info.is_some() || rich_modal_info.is_some() {
                        None
                    } else {
                        detect_im_service_notice(sender_name, &m.text)
                    };

                let mut bubble_inner_items: Vec<El> = Vec::new();
                if let Some(secs) = voice_secs {
                    bubble_inner_items.push(self.render_im_voice_card(
                        idx,
                        secs,
                        sender_name,
                        false,
                        t,
                    ));
                } else if let Some((mkind, mcap)) = &media_info {
                    bubble_inner_items.push(self.render_im_media_card(
                        idx,
                        m,
                        *mkind,
                        mcap,
                        sender_name,
                        t,
                    ));
                } else if let Some((rkind, rcap)) = &rich_modal_info {
                    bubble_inner_items.push(self.render_im_rich_modal_card(
                        idx,
                        *rkind,
                        rcap,
                        sender_name,
                        t,
                    ));
                } else if let Some(ref notice) = service_notice {
                    bubble_inner_items.push(self.render_im_service_notice_card(
                        idx,
                        notice,
                        sender_name,
                        t,
                    ));
                } else {
                    bubble_inner_items.push(self.chat_bubble_text_node(
                        &msg_text,
                        BODY,
                        if m.recalled { p.text2 } else { p.text },
                        440.0,
                    ));
                }

                let bubble_inner_col = Flex::column()
                    .with_spacing(6.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_children(bubble_inner_items)
                    .finish();

                let is_standalone_card =
                    media_info.is_some() || rich_modal_info.is_some() || service_notice.is_some();
                let bubble_box = if is_standalone_card {
                    bubble_inner_col
                } else {
                    Container::new(bubble_inner_col)
                        .with_horizontal_padding(14.)
                        .with_vertical_padding(9.)
                        .with_background_color(if m.recalled { p.surface } else { p.bg })
                        .with_border(Border::all(1.).with_border_color(p.border))
                        .with_corner_radius(Self::chat_bubble_radius(false, grouped_with_prev))
                        .finish()
                };

                let mut msg_col = Flex::column()
                    .with_spacing(3.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start);
                if !grouped_with_prev || m.recalled || media_badge.is_some() {
                    // Explicit grouping only; hover never changes geometry.
                    let meta_header_fixed = ConstrainedBox::new(
                        Container::new(
                            Flex::row()
                                .with_spacing(5.)
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_children(meta_items)
                                .finish(),
                        )
                        .with_padding_left(2.)
                        .finish(),
                    )
                    .with_height(18.0)
                    .finish();
                    msg_col = msg_col.with_child(meta_header_fixed);
                }
                msg_col = msg_col.with_child(self.hstack(
                    6.,
                    vec![
                        Shrinkable::new(1., bubble_box).finish(),
                        self.msg_inline_svg_action_btn(
                            format!("im-msg-info-{idx}"),
                            UiIcon::MoreHorizontal,
                            false,
                            Action::Push(PushAction::SelectNotification(im_detail_key(t, idx))),
                        ),
                    ],
                ));

                let left_avatar_slot = if !grouped_with_prev {
                    self.render_chat_sender_avatar(t, sender_name, 30.0)
                } else {
                    ConstrainedBox::new(Empty::new().finish())
                        .with_width(30.0)
                        .with_height(1.0)
                        .finish()
                };
                let in_el = Container::new(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_spacing(8.)
                        .with_child(left_avatar_slot)
                        .with_child(Shrinkable::new(1., msg_col.finish()).finish())
                        .finish(),
                )
                .with_padding_top(top_gap)
                .finish();
                let state = self.mouse_state(in_key);
                let txt_dbl = copy_payload.clone();
                let otp_rc = extract_verification_code(&m.text);
                let tid_rc = t.thread_id.clone();
                let pkg_rc = t.package_name.clone();
                let title_rc = t.title.clone();
                let mid_rc = m.id.clone();
                let sender_rc = sender_name.to_string();
                let txt_rc = txt_dbl.clone();
                let nkey_rc = t.notification_key.clone();
                let bubble_row = Hoverable::new(state, move |_| in_el)
                    .without_hover_animation()
                    .with_defer_events_to_children()
                    .on_double_click(move |ctx, _, _| {
                        if let Some(code) = extract_verification_code(&txt_dbl) {
                            let pickup = is_pickup_code_text(&code, &txt_dbl);
                            ctx.dispatch_typed_action(Action::CopyText(
                                code.clone(),
                                if pickup {
                                    format!("已复制取件码 {code}")
                                } else {
                                    format!("已复制验证码 {code}")
                                },
                            ));
                            return;
                        }
                        ctx.dispatch_typed_action(Action::CopyText(
                            txt_dbl.clone(),
                            "已复制消息内容".into(),
                        ));
                    })
                    .on_right_click(move |ctx, _, at| {
                        ctx.dispatch_typed_action(Action::ContextOpen(
                            crate::ui::context_menu::Target::ImMessage {
                                thread_id: tid_rc.clone(),
                                package_name: pkg_rc.clone(),
                                title: title_rc.clone(),
                                msg_id: mid_rc.clone(),
                                sender: sender_rc.clone(),
                                text: txt_rc.clone(),
                                notification_key: nkey_rc.clone(),
                                otp_code: otp_rc.clone(),
                            },
                            at,
                        ));
                    })
                    .finish();
                #[cfg(feature = "ui-capture")]
                let bubble_row =
                    crate::ui::scenario_check::control(format!("im-msg-bubble-{idx}"), bubble_row);
                msg_items.push(bubble_row);
            }
        }

        let chat_canvas_bg = if p.dark {
            rgb(0x12, 0x13, 0x17)
        } else {
            rgb(0xF6, 0xF7, 0xF9)
        };
        let stream_scroll = Container::new(
            self.sms_stream_scroll(
                format!("im:{}", t.thread_id),
                t.messages.len(),
                Container::new(self.vstack(0., msg_items))
                    .with_horizontal_padding(S5)
                    .with_padding_top(S4)
                    .with_padding_bottom(S5)
                    .finish(),
            ),
        )
        .with_background_color(chat_canvas_bg)
        .finish();

        // Most frames have no detail selected. Do not serialize every historic
        // message identity on every mouse move just to discover that fact.
        let selected_detail = self
            .push
            .selected_key
            .as_deref()
            .and_then(|key| key.strip_prefix("msg_detail::"))
            .and_then(|key| serde_json::from_str::<(String, String)>(key).ok())
            .filter(|(thread, _)| thread == &t.thread_id)
            .and_then(|(_, key)| {
                t.messages
                    .iter()
                    .enumerate()
                    .find(|(_, m)| crate::ui::message_window::has_key(m, &key))
            });
        let stream_scroll = if let Some((idx, m)) = selected_detail {
            let sender = if m.outgoing { "我" } else { &m.sender };
            let card = self.render_im_message_detail_card(
                idx,
                sender,
                &super::format_time(m.timestamp_ms),
                &m.text,
                &m.via,
                m.outgoing,
                t,
            );
            let card = ConstrainedBox::new(card).with_max_width(540.0).finish();
            #[cfg(feature = "ui-capture")]
            let card = crate::ui::scenario_check::control("im-message-detail-overlay".into(), card);
            Stack::new()
                .with_child(stream_scroll)
                .with_positioned_child(
                    card,
                    OffsetPositioning::offset_from_parent(
                        vec2f(-16.0, 16.0),
                        ParentOffsetBounds::ParentBySize,
                        ParentAnchor::TopRight,
                        ChildAnchor::TopRight,
                    ),
                )
                .finish()
        } else {
            stream_scroll
        };

        // Bottom Direct Reply Composer
        let quick_phrases = [
            ("收到，谢谢", "收到，谢谢"),
            ("稍后回电", "稍后回电"),
            ("正在开会，稍后联系", "正在开会，稍后联系"),
            ("快递请放门口/驿站", "快递请放门口/驿站"),
            ("我马上到", "我马上到"),
            ("好的，没问题", "好的，没问题"),
        ];
        let mut quick_chips: Vec<El> = quick_phrases
            .into_iter()
            .enumerate()
            .map(|(idx, (btn_label, phrase))| {
                self.im_channel_pill(
                    format!("im-qreply-{idx}"),
                    btn_label,
                    false,
                    Action::Push(PushAction::QuickReplyPhrase {
                        thread_id: t.thread_id.clone(),
                        package_name: t.package_name.clone(),
                        title: t.title.clone(),
                        text: phrase.to_string(),
                    }),
                )
            })
            .collect();

        quick_chips.push(self.im_channel_pill(
            "im-qreply-ai",
            "✦ 智能拟稿",
            true,
            Action::Push(PushAction::AiDraftReply {
                thread_id: t.thread_id.clone(),
                package_name: t.package_name.clone(),
                title: t.title.clone(),
            }),
        ));

        let status_node = if self.push.reply_in_flight {
            Some(self.hstack(
                6.,
                vec![
                    self.dot(p.warning),
                    self.line("后台静默发送中（可继续输入发送）…", MICRO, p.warning),
                ],
            ))
        } else {
            self.push.last_reply_status.as_ref().map(|st| {
                self.line(
                    st,
                    MICRO,
                    if st.starts_with("回复未完成") {
                        p.danger
                    } else {
                        p.text2
                    },
                )
            })
        };

        let quick_bar = self.wrap(quick_chips);

        let composer_input_row = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(8.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Expanded::new(
                        1.,
                        self.input(F_IM_REPLY, "输入回复内容，按回车发送…", None),
                    )
                    .finish(),
                )
                .with_child(self.button_icon(
                    "im-send-reply",
                    "发送",
                    UiIcon::Send,
                    Action::Push(PushAction::SendImReply),
                    Btn::Primary,
                ))
                .finish(),
        )
        .with_uniform_padding(8.)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let mut composer_rows = if t.can_remote_reply {
            vec![quick_bar]
        } else {
            Vec::new()
        };
        if let Some(quote_bar) = self.render_quote_preview_bar(false) {
            composer_rows.push(quote_bar);
        }
        if let Some(st) = status_node {
            composer_rows.push(st);
        }
        if t.can_remote_reply {
            composer_rows.push(composer_input_row);
        } else {
            composer_rows.push(self.between(
                self.line("此会话暂不支持后台回复", MICRO, p.text2),
                self.hstack(
                    8.,
                    vec![
                        self.button_icon(
                            "im-fallback-refresh",
                            "重试",
                            UiIcon::Refresh,
                            Action::Push(PushAction::Refresh),
                            Btn::Ghost,
                        ),
                        self.button_icon(
                            "im-fallback-open-app",
                            "投屏回复",
                            UiIcon::Monitor,
                            Action::Mirror,
                            Btn::Secondary,
                        ),
                    ],
                ),
            ));
            // Keep an existing draft available instead of silently discarding it.
            if !self.fields[F_IM_REPLY].text.trim().is_empty() {
                composer_rows.push(self.input(F_IM_REPLY, "回复草稿", None));
                composer_rows.push(self.button_icon(
                    "im-copy-draft",
                    "复制草稿",
                    UiIcon::Copy,
                    Action::CopyText(
                        self.fields[F_IM_REPLY].text.clone(),
                        "已复制回复草稿".into(),
                    ),
                    Btn::Ghost,
                ));
            }
        }

        let bottom_composer = Container::new(self.vstack(8., composer_rows))
            .with_horizontal_padding(S5)
            .with_padding_top(S3)
            .with_padding_bottom(S4)
            .with_background_color(p.bg)
            .with_border(Border::top(1.).with_border_color(p.border))
            .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(dialog_header)
            .with_child(Expanded::new(1., stream_scroll).finish())
            .with_child(bottom_composer)
            .finish()
    }

    pub(in super::super) fn sms_page(&self) -> El {
        let p = self.p();
        if !self.demo_mode && self.selected.is_none() {
            return Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.comm_mode_bar())
                .with_child(
                    Expanded::new(
                        1.,
                        self.empty(
                            "连接手机后查看消息与短信",
                            "连接后将同步微信、企业微信、钉钉、飞书会话以及手机短信。",
                            Some(self.button(
                                "sms-connect",
                                "连接设备",
                                Action::Page(Page::Connect),
                                Btn::Primary,
                            )),
                        ),
                    )
                    .finish(),
                )
                .finish();
        }

        let demo_fallback: PushSnapshot;
        let snapshot: &PushSnapshot = if self.demo_mode
            && self.push.snapshot.threads.is_empty()
            && self.push.snapshot.notifications.is_empty()
        {
            demo_fallback = demo_push_snapshot();
            &demo_fallback
        } else {
            &self.push.snapshot
        };

        let loaded = self.sms_loaded || self.demo_mode;
        let search_q = self.fields[F_SMS_SEARCH].text.trim().to_lowercase();

        // Build native SMS threads from the phone provider snapshot.
        let now_ms = Local::now().timestamp_millis().max(0) as u64;
        let mut display_threads: Vec<DisplaySmsThread> = Vec::new();
        for rt in self.sms_threads() {
            let (mut name, mut badge) = resolve_sender_info(&rt.address, &self.contacts);
            if name == rt.address
                || name
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '+' || c == '-' || c == ' ')
            {
                if let Some(brand) = extract_sms_bracket_brand(&rt.last_body) {
                    let short_num: String = rt.address.trim().chars().take(6).collect();
                    name = if short_num.chars().count() < rt.address.trim().chars().count() {
                        format!("{brand} · {short_num}…")
                    } else {
                        format!("{brand} · {short_num}")
                    };
                    if let Some(first_ch) = brand.chars().next() {
                        badge = first_ch.to_string();
                    }
                }
            }
            let norm = norm_addr(&rt.address);
            let addr_key = if norm.is_empty() {
                rt.address.trim().to_string()
            } else {
                norm.clone()
            };
            let last_is_outgoing = self
                .sms
                .iter()
                .filter(|m| norm_addr(&m.address) == norm)
                .max_by_key(|m| m.date_ms)
                .is_some_and(|m| m.outgoing);
            let raw_one_line: String = rt
                .last_body
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let one_line = if last_is_outgoing
                && !raw_one_line.is_empty()
                && !raw_one_line.starts_with("我:")
                && !raw_one_line.starts_with("我：")
            {
                format!("我: {raw_one_line}")
            } else {
                raw_one_line
            };
            let preview = if one_line.chars().count() > 38 {
                format!("{}…", one_line.chars().take(38).collect::<String>())
            } else {
                one_line
            };
            let otp_code = extract_verification_code(&rt.last_body);
            let sms_tkey =
                stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(&addr_key);
            let manual_unread = self.push.prefs.manual_unread_targets.contains(&sms_tkey);
            let unread = manual_unread
                || (rt.unread
                    && !self.read_sms_addresses.contains(&norm)
                    && rt.last_ms > 0
                    && now_ms.saturating_sub(rt.last_ms as u64) <= 12 * 3600 * 1000
                    && self.sms.iter().any(|m| {
                        norm_addr(&m.address) == norm && self.is_unread_native_sms(m, now_ms)
                    }));
            display_threads.push(DisplaySmsThread {
                name,
                num: rt.address,
                time: format_sms_time(rt.last_ms),
                last_ts: rt.last_ms.max(0) as u64,
                preview,
                last_body: rt.last_body,
                unread,
                badge,
                count: rt.count,
                otp_code,
            });
        }
        display_threads.sort_by(|a, b| {
            let ka = norm_addr(&a.num);
            let kb = norm_addr(&b.num);
            let pa = self.push.prefs.is_pinned(
                &stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                    if ka.is_empty() { a.num.trim() } else { &ka },
                ),
            );
            let pb = self.push.prefs.is_pinned(
                &stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                    if kb.is_empty() { b.num.trim() } else { &kb },
                ),
            );
            (pb, b.last_ts)
                .cmp(&(pa, a.last_ts))
                .then_with(|| ka.cmp(&kb))
        });

        // Filter IM threads from `snapshot.threads`
        let mut matching_im_threads: Vec<&ImThread> = if self.push.im_channel == ImChannel::Sms {
            Vec::new()
        } else {
            snapshot
                .threads
                .iter()
                .filter(|t| {
                    !stepx_companion::push::is_excluded_im_package(&t.package_name)
                        && !stepx_companion::push::is_scraped_ui_noise_thread(t)
                        && !stepx_companion::push::is_timer_or_countdown_notification(
                            &t.package_name,
                            &t.app_name,
                            &t.title,
                            &t.last_message,
                        )
                        && im_thread_has_visible_content(t)
                        && !t.thread_id.contains("stepx_e2e_test")
                        && !t.thread_id.contains("stepx_test_tag")
                })
                .filter(|t| t.channel() != ImChannel::Sms)
                .filter(|t| self.push.im_channel.matches_package(&t.package_name))
                .filter(|t| {
                    let is_selected = self
                        .push
                        .selected_im_thread
                        .as_deref()
                        .is_some_and(|sel| sel == t.thread_id);
                    if is_selected {
                        return true;
                    }
                    match self.sms_filter {
                        crate::ui::SmsFilter::All => true,
                        crate::ui::SmsFilter::Incoming => t.unread_count > 0,
                        crate::ui::SmsFilter::Outgoing => t.messages.iter().any(|m| m.outgoing),
                    }
                })
                .filter(|t| {
                    if search_q.is_empty() {
                        return true;
                    }
                    t.title.to_lowercase().contains(&search_q)
                        || t.app_name.to_lowercase().contains(&search_q)
                        || t.last_sender.to_lowercase().contains(&search_q)
                        || t.last_message.to_lowercase().contains(&search_q)
                        || t.messages
                            .iter()
                            .any(|m| im_message_matches_search(m, &search_q))
                })
                .collect()
        };
        matching_im_threads.sort_by(|a, b| {
            let pa = self.push.prefs.is_pinned(
                &stepx_companion::desktop_notify::NotifyPreferences::im_target_key(&a.thread_id),
            );
            let pb = self.push.prefs.is_pinned(
                &stepx_companion::desktop_notify::NotifyPreferences::im_target_key(&b.thread_id),
            );
            (pb, b.last_timestamp_ms)
                .cmp(&(pa, a.last_timestamp_ms))
                .then_with(|| a.thread_id.cmp(&b.thread_id))
        });

        let is_new_sms = self.selected_thread.as_deref() == Some("__new__");

        // Determine whether an IM thread or a native SMS thread is currently active in the right pane
        let is_notif_detail_selected = self
            .push
            .selected_im_thread
            .as_deref()
            .is_some_and(|s| s.starts_with("notif_detail::"));
        let active_im_thread: Option<&ImThread> =
            if is_new_sms || self.push.im_channel == ImChannel::Sms || is_notif_detail_selected {
                None
            } else if let Some(sel_id) = self.push.selected_im_thread.as_deref() {
                if sel_id.is_empty() {
                    if self.selected_thread.is_some() {
                        None
                    } else {
                        matching_im_threads.first().copied()
                    }
                } else {
                    matching_im_threads
                        .iter()
                        .copied()
                        .find(|t| t.thread_id == sel_id)
                        .or_else(|| {
                            if self.selected_thread.is_none() {
                                matching_im_threads.first().copied()
                            } else {
                                None
                            }
                        })
                }
            } else if self.selected_thread.is_none() || self.push.im_channel != ImChannel::All {
                matching_im_threads.first().copied()
            } else {
                None
            };

        let active_im_id = active_im_thread.map(|t| t.thread_id.as_str()).unwrap_or("");

        let active_addr = if is_new_sms {
            "__new__".to_string()
        } else if active_im_thread.is_some() || is_notif_detail_selected {
            String::new()
        } else if let Some(sel) = &self.selected_thread {
            sel.clone()
        } else if let Some(first) = display_threads.first() {
            first.num.clone()
        } else {
            String::new()
        };

        // Top Channel Bar (`ImChannel`)
        let im_threads_source: Vec<&ImThread> = snapshot
            .threads
            .iter()
            .filter(|t| {
                t.channel() != ImChannel::Sms
                    && !Self::is_excluded_im_package(&t.package_name)
                    && !stepx_companion::push::is_scraped_ui_noise_thread(t)
                    && !stepx_companion::push::is_timer_or_countdown_notification(
                        &t.package_name,
                        &t.app_name,
                        &t.title,
                        &t.last_message,
                    )
                    && im_thread_has_visible_content(t)
                    && !t.thread_id.contains("stepx_e2e_test")
                    && !t.thread_id.contains("stepx_test_tag")
                    && !t.title.trim_start().starts_with('{')
            })
            .collect();
        let effective_push_notifs = self.effective_push_notifications();
        let orphan_push_notifs: Vec<&stepx_companion::push::PushNotification> =
            effective_push_notifs
                .iter()
                .filter(|n| {
                    if n.dismissed
                        || n.key == "app-updates-summary"
                        || n.key.starts_with("app-update:")
                        || n.package_name == "com.stepos.appstore"
                        || n.package_name == "com.stepx.companion.app"
                        || (!n.active
                            && !n.key.starts_with("phone-")
                            && !n.key.starts_with("express:")
                            && !n.key.starts_with("missed-call:")
                            && !n.key.starts_with("calendar:")
                            && !n.key.starts_with("alarm:"))
                    {
                        return false;
                    }
                    if Self::is_excluded_im_package(&n.package_name)
                        || stepx_companion::push::is_timer_or_countdown_notification(
                            &n.package_name,
                            &n.app_name,
                            &n.title,
                            &n.text,
                        )
                        || n.title.trim_start().starts_with('{')
                    {
                        return false;
                    }
                    let raw_tid = n
                        .key
                        .strip_prefix("im-thread:")
                        .unwrap_or(n.thread_id.as_str());
                    let n_norm_title =
                        stepx_companion::push::normalize_conversation_title(&n.title);
                    let covered_by_im = im_threads_source.iter().any(|t| {
                        (!raw_tid.is_empty() && t.thread_id == raw_tid)
                            || (!t.notification_key.is_empty()
                                && (t.notification_key == n.key
                                    || format!("push-notif:{}", t.notification_key) == n.key))
                            || (t.package_name == n.package_name
                                && !n_norm_title.is_empty()
                                && stepx_companion::push::normalize_conversation_title(&t.title)
                                    == n_norm_title)
                    });
                    if covered_by_im {
                        return false;
                    }
                    let covered_by_sms = (n.package_name == "com.android.mms"
                        || ImChannel::from_package(&n.package_name) == ImChannel::Sms
                        || n.key.starts_with("sms:")
                        || n.thread_id.starts_with("sms:"))
                        && display_threads.iter().any(|d| {
                            format!("sms:{}", d.num.trim()) == n.thread_id
                                || n.key.starts_with(&format!("sms:{}:", d.num.trim()))
                                || (!n.text.trim().is_empty()
                                    && d.last_body.trim() == n.text.trim())
                                || (n.otp_code.is_some() && d.otp_code == n.otp_code)
                        });
                    !covered_by_sms
                })
                .collect();

        let has_channel_threads = |ch: ImChannel| -> bool {
            self.push.im_channel == ch
                || im_threads_source
                    .iter()
                    .any(|t| ch.matches_package(&t.package_name))
                || orphan_push_notifs
                    .iter()
                    .any(|n| ch.matches_package(&n.package_name))
        };
        let mut channel_tabs: Vec<(ImChannel, &'static str)> = vec![(ImChannel::All, "全部")];
        for (ch, label) in [
            (ImChannel::WeChat, "微信"),
            (ImChannel::WeCom, "企微"),
            (ImChannel::DingTalk, "钉钉"),
            (ImChannel::Feishu, "飞书"),
            (ImChannel::QqTim, "QQ"),
            (ImChannel::SocialDm, "社媒私信"),
            (ImChannel::GlobalIm, "更多"),
        ] {
            if has_channel_threads(ch) {
                channel_tabs.push((ch, label));
            }
        }
        channel_tabs.push((ImChannel::Sms, "短信"));
        let channel_pills: Vec<El> = channel_tabs
            .into_iter()
            .map(|(ch, label)| {
                self.im_channel_pill(
                    format!("ch-{ch:?}"),
                    label,
                    self.push.im_channel == ch,
                    Action::Push(PushAction::SelectImChannel(ch)),
                )
            })
            .collect();
        // Sort lightweight row identities before constructing avatars, text and
        // interaction widgets. Every source remains available to search and AI.
        let show_native_sms = matches!(self.push.im_channel, ImChannel::All | ImChannel::Sms);
        let thread_page_key = format!("message-threads:{:?}:{search_q}",self.push.im_channel);
        let mut thread_order: Vec<(bool,u64,String,u8,usize)> = Vec::new();
        for (i,thread) in matching_im_threads.iter().enumerate() {
            let pin_key = stepx_companion::desktop_notify::NotifyPreferences::im_target_key(&thread.thread_id);
            let timestamp = (thread.last_timestamp_ms > 0).then_some(thread.last_timestamp_ms)
                .or_else(||thread.messages.iter().rev().find_map(|m|(m.timestamp_ms>0).then_some(m.timestamp_ms))).unwrap_or(0).max(0) as u64;
            thread_order.push((self.push.prefs.is_pinned(&pin_key),timestamp,thread.thread_id.clone(),0,i));
        }
        if show_native_sms { for (i,thread) in display_threads.iter().enumerate() {
            let normalized = norm_addr(&thread.num);
            let address = if normalized.is_empty() {thread.num.trim()} else {normalized.as_str()};
            let pin_key = stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(address);
            thread_order.push((self.push.prefs.is_pinned(&pin_key),thread.last_ts,format!("sms:{}",thread.num),1,i));
        } }
        if self.push.im_channel != ImChannel::Sms { for (i,notification) in orphan_push_notifs.iter().enumerate() {
            if self.push.im_channel != ImChannel::All && !self.push.im_channel.matches_package(&notification.package_name) {continue;}
            if !search_q.is_empty() && !format!("{} {} {} {}",notification.title,notification.app_name,notification.sender,notification.text).to_lowercase().contains(&search_q) {continue;}
            thread_order.push((self.push.prefs.is_pinned(&format!("notif:{}",notification.key)),notification.post_time_ms.max(0) as u64,format!("push:{}",notification.key),2,i));
        } }
        thread_order.sort_by(|a,b|(b.0,b.1).cmp(&(a.0,a.1)).then_with(||a.2.cmp(&b.2)));
        let thread_total = thread_order.len();
        let visible_threads: std::collections::HashSet<_> = thread_order[self.list_page_range(&thread_page_key,thread_total,40)].iter().map(|row|(row.3,row.4)).collect();
        let mut combined_rows: Vec<(bool, u64, String, El)> = Vec::new();

        // 1. Render matching IM threads when channel != ImChannel::Sms
        for (i, t) in matching_im_threads.iter().enumerate() {
            if !visible_threads.contains(&(0,i)) {continue;}
            let active = !is_new_sms && t.thread_id == active_im_id;
            let im_pin_key =
                stepx_companion::desktop_notify::NotifyPreferences::im_target_key(&t.thread_id);
            let is_pinned = self.push.prefs.is_pinned(&im_pin_key);
            let app_label = if t.app_name.is_empty() {
                t.channel().label()
            } else {
                t.app_name.as_str()
            };

            let raw_title = if stepx_companion::push::is_generic_im_title(&t.title)
                && !t.last_sender.trim().is_empty()
                && t.last_sender.trim() != "我"
                && !stepx_companion::push::is_generic_im_title(&t.last_sender)
            {
                format!(
                    "{}（{}）",
                    if t.title.trim().is_empty() {
                        "群聊"
                    } else {
                        t.title.trim()
                    },
                    t.last_sender.trim()
                )
            } else {
                t.title.clone()
            };
            let avatar = self.render_im_entity_avatar(
                &t.package_name,
                app_label,
                &raw_title,
                &t.last_message,
                t.is_group,
                &t.messages,
                38.0,
            );
            let display_title = compact_thread_title(&raw_title, 15);
            let mut title_badges =
                vec![self.line_w(display_title, CAPTION, p.text, Weight::Semibold)];
            if is_pinned {
                title_badges.push(self.content_icon(UiIcon::Pin, 12.0, p.accent));
            }
            if self.push.im_channel == ImChannel::All {
                title_badges.push(self.pill_badge(app_label, p.text2, p.tag_bg));
            }

            let effective_last_msg = if let Some(last_m) = t.messages.last() {
                if last_m.recalled
                    || !last_m.media_hint.trim().is_empty()
                    || t.last_message.trim().is_empty()
                {
                    let (formatted, _, _) = format_im_message_content(last_m, false);
                    if !formatted.trim().is_empty() {
                        formatted
                    } else {
                        t.last_message.clone()
                    }
                } else {
                    t.last_message.clone()
                }
            } else {
                t.last_message.clone()
            };
            let last_is_outgoing =
                t.last_sender.trim() == "我" || t.messages.last().is_some_and(|m| m.outgoing);
            let raw_preview = if last_is_outgoing {
                let clean_msg = sanitize_ui_display_text(&effective_last_msg);
                if clean_msg.starts_with("我:") || clean_msg.starts_with("我：") {
                    clean_msg
                } else {
                    format!("我: {clean_msg}")
                }
            } else if !t.last_sender.is_empty()
                && t.last_sender != t.title
                && !effective_last_msg.starts_with(&t.last_sender)
            {
                format!(
                    "{}: {}",
                    sanitize_ui_display_text(&t.last_sender),
                    sanitize_ui_display_text(&effective_last_msg)
                )
            } else {
                sanitize_ui_display_text(&effective_last_msg)
            };
            let humanized_preview = humanize_wechat_emojis(&raw_preview);
            let single_line: String = humanized_preview
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let preview_str = if single_line.chars().count() > 36 {
                format!("{}…", single_line.chars().take(36).collect::<String>())
            } else {
                single_line
            };

            let row_key = format!("im-th:{}", t.thread_id);
            let pin_btn_id = format!("im-row-pin-{i}");
            let del_btn_id = format!("im-row-del-{i}");
            let row_hover_keys = [
                row_key.clone(),
                format!("btn:{pin_btn_id}"),
                format!("btn:{del_btn_id}"),
            ];
            let row_hovered = {
                let map = self.hover_states.borrow();
                row_hover_keys.iter().any(|k| {
                    map.get(k.as_str())
                        .is_some_and(|(_, h)| h.lock().unwrap().is_mouse_over_element())
                })
            };

            let show_unread = t.unread_count > 0;
            let preview_el = self.line_w(
                preview_str,
                CAPTION,
                if show_unread { p.text } else { p.text2 },
                if show_unread {
                    Weight::Medium
                } else {
                    Weight::Normal
                },
            );

            #[cfg(feature = "ui-capture")]
            let preview_el =
                crate::ui::scenario_check::control(format!("im-row-preview-{i}"), preview_el);
            let bottom_right = if row_hovered || active {
                let mut quick_btns = Vec::new();
                if show_unread {
                    quick_btns.push(self.dot(p.danger));
                }
                quick_btns.push(self.msg_inline_svg_action_btn(
                    pin_btn_id,
                    UiIcon::Pin,
                    is_pinned,
                    Action::Push(PushAction::TogglePinTarget(im_pin_key.clone())),
                ));
                quick_btns.push(self.msg_inline_svg_action_btn(
                    del_btn_id,
                    UiIcon::Trash,
                    false,
                    Action::Push(PushAction::DeleteImThread {
                        thread_id: t.thread_id.clone(),
                        package_name: t.package_name.clone(),
                        title: t.title.clone(),
                    }),
                ));
                self.hstack(4., quick_btns)
            } else if show_unread {
                self.dot(p.danger)
            } else {
                Empty::new().finish()
            };
            let bottom_row = ConstrainedBox::new(self.between(
                Clipped::new(preview_el).finish(),
                self.message_row_actions_slot(bottom_right),
            ))
            .with_height(20.0)
            .finish();

            let thread_ts = (t.last_timestamp_ms > 0)
                .then_some(t.last_timestamp_ms)
                .or_else(|| {
                    t.messages
                        .iter()
                        .rev()
                        .find_map(|m| (m.timestamp_ms > 0).then_some(m.timestamp_ms))
                })
                .map(super::format_time)
                .unwrap_or_default();
            let im_time_el = facloud_ui::elements::Text::new(thread_ts, self.mono_font, MICRO)
                .with_color(p.text3)
                .soft_wrap(false)
                .finish();

            let top_title_row = ConstrainedBox::new(
                self.between(
                    Clipped::new(
                        Flex::row()
                            .with_spacing(6.)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_constrain_horizontal_bounds_to_parent(true)
                            .with_children(title_badges)
                            .finish(),
                    )
                    .finish(),
                    im_time_el,
                ),
            )
            .with_height(20.0)
            .finish();

            let right_col = Flex::column()
                .with_spacing(4.)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(top_title_row)
                .with_child(bottom_row)
                .finish();

            let row_inner = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(12.)
                .with_child(avatar)
                .with_child(Expanded::new(1., right_col).finish())
                .finish();

            let state = self.mouse_state(row_key);
            let tid = t.thread_id.clone();
            let tid_dbl = t.thread_id.clone();
            let tid_rc = t.thread_id.clone();
            let pkg_rc = t.package_name.clone();
            let title_rc = t.title.clone();
            let last_msg_rc = t.last_message.clone();
            let last_msg_dbl = t.last_message.clone();
            let nkey_rc = t.notification_key.clone();
            let im_otp = t
                .messages
                .iter()
                .rev()
                .find_map(|m| extract_verification_code(&m.text))
                .or_else(|| extract_verification_code(&t.last_message));
            let im_otp_dbl = im_otp.clone();
            let row_el = Hoverable::new(state, move |mouse| {
                let hovered = mouse.is_mouse_over_element();
                let t = if self.settings.value.reduced_motion {
                    if hovered {
                        1.0
                    } else {
                        0.0
                    }
                } else {
                    mouse.hover_progress().clamp(0.0, 1.0)
                };
                let bg = if mouse.is_clicked() {
                    p.accent_soft
                } else if active {
                    mix_color(p.selected, p.hover, t * 0.5)
                } else {
                    mix_color(ColorU::transparent_black(), p.hover, t)
                };
                Container::new(row_inner)
                    .with_uniform_padding(12.)
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
            })
            .with_defer_events_to_children()
            .without_hover_animation()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(tid.clone())));
            })
            .on_double_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(
                    tid_dbl.clone(),
                )));
                if let Some(code) = &im_otp_dbl {
                    let pickup = is_pickup_code_text(code, &last_msg_dbl);
                    ctx.dispatch_typed_action(Action::CopyText(
                        code.clone(),
                        if pickup {
                            format!("已复制取件码 {code}")
                        } else {
                            format!("已复制验证码 {code}")
                        },
                    ));
                }
                ctx.dispatch_typed_action(Action::Focus(F_IM_REPLY));
            })
            .on_right_click(move |ctx, _, at| {
                ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(tid_rc.clone())));
                ctx.dispatch_typed_action(Action::ContextOpen(
                    crate::ui::context_menu::Target::ImThread {
                        thread_id: tid_rc.clone(),
                        package_name: pkg_rc.clone(),
                        title: title_rc.clone(),
                        last_message: last_msg_rc.clone(),
                        notification_key: nkey_rc.clone(),
                        otp_code: im_otp.clone(),
                    },
                    at,
                ));
            })
            .finish();
            #[cfg(feature = "ui-capture")]
            let row_el = crate::ui::scenario_check::control(
                format!("im-row-{i}"),
                crate::ui::scenario_check::control(format!("im-thread-{}", t.thread_id), row_el),
            );

            let ts_ms = (t.last_timestamp_ms > 0)
                .then_some(t.last_timestamp_ms)
                .or_else(|| {
                    t.messages
                        .iter()
                        .rev()
                        .find_map(|m| (m.timestamp_ms > 0).then_some(m.timestamp_ms))
                })
                .unwrap_or(0)
                .max(0) as u64;
            combined_rows.push((is_pinned, ts_ms, t.thread_id.clone(), row_el));
        }

        // 2. Render native SMS threads when channel is `All` or `Sms`
        if show_native_sms {
            for (i, t) in display_threads.iter().enumerate() {
                if !visible_threads.contains(&(1,i)) {continue;}
                let active = !is_new_sms
                    && active_im_thread.is_none()
                    && !active_addr.is_empty()
                    && norm_addr(&active_addr) == norm_addr(&t.num);
                let sms_norm = norm_addr(&t.num);
                let sms_addr_key = if sms_norm.is_empty() {
                    t.num.trim().to_string()
                } else {
                    sms_norm
                };
                let sms_pin_key =
                    stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                        &sms_addr_key,
                    );
                let is_pinned = self.push.prefs.is_pinned(&sms_pin_key);
                let display_name = compact_thread_title(&t.name, 14);
                let avatar = self.render_im_entity_avatar(
                    "com.android.mms",
                    "短信",
                    &t.name,
                    &t.last_body,
                    false,
                    &[],
                    38.0,
                );

                let row_key = format!("sms-th:{}", t.num);
                let pin_btn_id = format!("sms-row-pin-{i}");
                let del_btn_id = format!("sms-row-del-{i}");
                let row_hover_keys = [
                    row_key.clone(),
                    format!("btn:{pin_btn_id}"),
                    format!("btn:{del_btn_id}"),
                ];
                let row_hovered = {
                    let map = self.hover_states.borrow();
                    row_hover_keys.iter().any(|k| {
                        map.get(k.as_str())
                            .is_some_and(|(_, h)| h.lock().unwrap().is_mouse_over_element())
                    })
                };

                let show_unread = t.unread;
                let preview_line = self.line_w(
                    &t.preview,
                    CAPTION,
                    if show_unread { p.text } else { p.text2 },
                    if show_unread {
                        Weight::Medium
                    } else {
                        Weight::Normal
                    },
                );
                #[cfg(feature = "ui-capture")]
                let preview_line = crate::ui::scenario_check::control(
                    format!("sms-row-preview-{i}"),
                    preview_line,
                );
                let bottom_right = if row_hovered || active {
                    let mut quick_btns = Vec::new();
                    if show_unread {
                        quick_btns.push(self.dot(p.danger));
                    }
                    quick_btns.push(self.msg_inline_svg_action_btn(
                        pin_btn_id,
                        UiIcon::Pin,
                        is_pinned,
                        Action::Push(PushAction::TogglePinTarget(sms_pin_key.clone())),
                    ));
                    quick_btns.push(self.msg_inline_svg_action_btn(
                        del_btn_id,
                        UiIcon::Trash,
                        false,
                        Action::Push(PushAction::DeleteSmsThread {
                            address: t.num.clone(),
                            name: t.name.clone(),
                        }),
                    ));
                    self.hstack(4., quick_btns)
                } else if show_unread {
                    self.dot(p.danger)
                } else {
                    Empty::new().finish()
                };
                let bottom_row = ConstrainedBox::new(self.between(
                    Clipped::new(preview_line).finish(),
                    self.message_row_actions_slot(bottom_right),
                ))
                .with_height(20.0)
                .finish();

                let mut title_items =
                    vec![self.line_w(display_name, CAPTION, p.text, Weight::Semibold)];
                if is_pinned {
                    title_items.push(self.content_icon(UiIcon::Pin, 12.0, p.accent));
                }
                if let Some(code) = &t.otp_code {
                    let pickup = is_pickup_code_text(code, &t.preview);
                    title_items.push(self.pill_badge_icon(
                        if pickup {
                            format!("取件码 {code}")
                        } else {
                            format!("验证码 {code}")
                        },
                        if pickup { UiIcon::Package } else { UiIcon::Key },
                        p.warning,
                        p.warning_soft,
                    ));
                } else if self.push.im_channel == ImChannel::All {
                    title_items.push(self.pill_badge("短信", p.text2, p.tag_bg));
                }

                let sms_time_el =
                    facloud_ui::elements::Text::new(t.time.clone(), self.mono_font, MICRO)
                        .with_color(p.text2)
                        .soft_wrap(false)
                        .finish();

                let has_distinct_name = !t.name.trim().is_empty()
                    && t.name.trim() != t.num.trim()
                    && norm_addr(&t.name) != norm_addr(&t.num);

                let top_title_row = ConstrainedBox::new(
                    self.between(
                        Clipped::new(
                            Flex::row()
                                .with_spacing(6.)
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_constrain_horizontal_bounds_to_parent(true)
                                .with_children(title_items)
                                .finish(),
                        )
                        .finish(),
                        sms_time_el,
                    ),
                )
                .with_height(20.0)
                .finish();

                let mut right_col = Flex::column()
                    .with_spacing(3.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(top_title_row);
                if has_distinct_name {
                    right_col = right_col
                        .with_child(Clipped::new(self.mono(&t.num, CAPTION, p.text2)).finish());
                }
                let right_col = right_col.with_child(bottom_row).finish();

                let row_inner = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_spacing(12.)
                    .with_child(avatar)
                    .with_child(Expanded::new(1., right_col).finish())
                    .finish();

                let state = self.mouse_state(row_key);
                let num_str = t.num.clone();
                let num_dbl = t.num.clone();
                let otp_dbl = t.otp_code.clone();
                let body_dbl = t.last_body.clone();
                let num_rc = t.num.clone();
                let name_rc = t.name.clone();
                let body_rc = t.last_body.clone();
                let otp_rc = t.otp_code.clone();
                let row_el = Hoverable::new(state, move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    let t = if self.settings.value.reduced_motion {
                        if hovered {
                            1.0
                        } else {
                            0.0
                        }
                    } else {
                        mouse.hover_progress().clamp(0.0, 1.0)
                    };
                    let active_bg = if p.dark {
                        rgb(0x22, 0x22, 0x28)
                    } else {
                        rgb(0xF4, 0xF4, 0xF7)
                    };
                    let bg = if mouse.is_clicked() {
                        p.accent_soft
                    } else if active {
                        mix_color(active_bg, p.hover, t * 0.5)
                    } else {
                        mix_color(ColorU::transparent_black(), p.hover, t)
                    };
                    Container::new(row_inner)
                        .with_uniform_padding(12.)
                        .with_background_color(bg)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish()
                })
                .with_defer_events_to_children()
                .without_hover_animation()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(
                        String::new(),
                    )));
                    ctx.dispatch_typed_action(Action::SelectSmsThread(Some(num_str.clone())));
                })
                .on_double_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(
                        String::new(),
                    )));
                    ctx.dispatch_typed_action(Action::SelectSmsThread(Some(num_dbl.clone())));
                    if let Some(code) = &otp_dbl {
                        let pickup = is_pickup_code_text(code, &body_dbl);
                        ctx.dispatch_typed_action(Action::CopyText(
                            code.clone(),
                            if pickup {
                                format!("已复制取件码 {code}")
                            } else {
                                format!("已复制验证码 {code}")
                            },
                        ));
                    }
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(
                        String::new(),
                    )));
                    ctx.dispatch_typed_action(Action::SelectSmsThread(Some(num_rc.clone())));
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::SmsThread {
                            address: num_rc.clone(),
                            name: name_rc.clone(),
                            last_body: body_rc.clone(),
                            otp_code: otp_rc.clone(),
                        },
                        at,
                    ));
                })
                .finish();
                #[cfg(feature = "ui-capture")]
                let row_el = crate::ui::scenario_check::control(format!("sms-row-{i}"), row_el);
                combined_rows.push((is_pinned, t.last_ts, format!("sms:{}", t.num), row_el));
            }
        }

        // 3. Render unified pushed notifications & cross-device events not already covered by IM or SMS threads
        if self.push.im_channel != ImChannel::Sms {
            for (i, n) in orphan_push_notifs.iter().enumerate() {
                if !visible_threads.contains(&(2,i)) {continue;}
                if self.push.im_channel != ImChannel::All
                    && !self.push.im_channel.matches_package(&n.package_name)
                {
                    continue;
                }
                if !search_q.is_empty() {
                    let hay = format!("{} {} {} {}", n.title, n.app_name, n.sender, n.text)
                        .to_lowercase();
                    if !hay.contains(&search_q) {
                        continue;
                    }
                }
                let source_label =
                    push_notification_source_label(&n.package_name, &n.app_name, &n.title);
                let raw_title = if !n.title.trim().is_empty() {
                    n.title.trim().to_string()
                } else if !n.sender.trim().is_empty() {
                    n.sender.trim().to_string()
                } else {
                    source_label.clone()
                };
                let display_title = compact_thread_title(&raw_title, 15);
                let notif_pin_key = format!("notif:{}", n.key);
                let is_pinned = self.push.prefs.is_pinned(&notif_pin_key);
                let avatar = self.render_im_entity_avatar(
                    &n.package_name,
                    &source_label,
                    &raw_title,
                    &n.text,
                    false,
                    &[],
                    38.0,
                );
                let otp_code = n
                    .otp_code
                    .clone()
                    .or_else(|| extract_verification_code(&n.text));

                let mut title_badges =
                    vec![self.line_w(display_title, CAPTION, p.text, Weight::Semibold)];
                if is_pinned {
                    title_badges.push(self.content_icon(UiIcon::Pin, 12.0, p.accent));
                }
                if let Some(code) = &otp_code {
                    let pickup = is_pickup_code_text(code, &n.text);
                    title_badges.push(self.pill_badge_icon(
                        if pickup {
                            format!("取件码 {code}")
                        } else {
                            format!("验证码 {code}")
                        },
                        if pickup { UiIcon::Package } else { UiIcon::Key },
                        p.warning,
                        p.warning_soft,
                    ));
                } else if self.push.im_channel == ImChannel::All {
                    title_badges.push(self.pill_badge(&source_label, p.text2, p.tag_bg));
                }

                let raw_preview = if !n.text.trim().is_empty() {
                    sanitize_ui_display_text(&n.text)
                } else if !n.sub_text.trim().is_empty() {
                    sanitize_ui_display_text(&n.sub_text)
                } else {
                    source_label.clone()
                };
                let single_line: String = humanize_wechat_emojis(&raw_preview)
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let preview_str = if single_line.chars().count() > 36 {
                    format!("{}…", single_line.chars().take(36).collect::<String>())
                } else {
                    single_line
                };

                let row_key = format!("push-syn-th:{}", n.key);
                let pin_btn_id = format!("push-syn-pin-{i}");
                let del_btn_id = format!("push-syn-del-{i}");
                let row_hover_keys = [
                    row_key.clone(),
                    format!("btn:{pin_btn_id}"),
                    format!("btn:{del_btn_id}"),
                ];
                let row_hovered = {
                    let map = self.hover_states.borrow();
                    row_hover_keys.iter().any(|k| {
                        map.get(k.as_str())
                            .is_some_and(|(_, h)| h.lock().unwrap().is_mouse_over_element())
                    })
                };

                let show_unread = !n.read;
                let preview_el = self.line_w(
                    preview_str,
                    CAPTION,
                    if show_unread { p.text } else { p.text2 },
                    if show_unread {
                        Weight::Medium
                    } else {
                        Weight::Normal
                    },
                );

                #[cfg(feature = "ui-capture")]
                let preview_el =
                    crate::ui::scenario_check::control(format!("push-row-preview-{i}"), preview_el);
                let bottom_right = if row_hovered {
                    let mut quick_btns = Vec::new();
                    if show_unread {
                        quick_btns.push(self.dot(p.danger));
                    }
                    quick_btns.push(self.msg_inline_svg_action_btn(
                        pin_btn_id,
                        UiIcon::Pin,
                        is_pinned,
                        Action::Push(PushAction::TogglePinTarget(notif_pin_key.clone())),
                    ));
                    quick_btns.push(self.msg_inline_svg_action_btn(
                        del_btn_id,
                        UiIcon::Trash,
                        false,
                        Action::Push(PushAction::DismissNotification(n.key.clone())),
                    ));
                    self.hstack(4., quick_btns)
                } else if show_unread {
                    self.dot(p.danger)
                } else {
                    Empty::new().finish()
                };
                let bottom_row = ConstrainedBox::new(self.between(
                    Clipped::new(preview_el).finish(),
                    self.message_row_actions_slot(bottom_right),
                ))
                .with_height(20.0)
                .finish();

                let notif_ts = (n.post_time_ms > 0)
                    .then(|| super::format_time(n.post_time_ms))
                    .unwrap_or_default();
                let time_el = facloud_ui::elements::Text::new(notif_ts, self.mono_font, MICRO)
                    .with_color(p.text3)
                    .soft_wrap(false)
                    .finish();

                let top_title_row = ConstrainedBox::new(
                    self.between(
                        Clipped::new(
                            Flex::row()
                                .with_spacing(6.)
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_constrain_horizontal_bounds_to_parent(true)
                                .with_children(title_badges)
                                .finish(),
                        )
                        .finish(),
                        time_el,
                    ),
                )
                .with_height(20.0)
                .finish();

                let right_col = Flex::column()
                    .with_spacing(4.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(top_title_row)
                    .with_child(bottom_row)
                    .finish();

                let row_inner = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_spacing(12.)
                    .with_child(avatar)
                    .with_child(Expanded::new(1., right_col).finish())
                    .finish();

                let state = self.mouse_state(row_key);
                let nkey_click = n.key.clone();
                let nkey_dbl = n.key.clone();
                let ntext_dbl = n.text.clone();
                let otp_dbl = otp_code.clone();
                let active_notif = !is_new_sms
                    && self.push.selected_im_thread.as_deref()
                        == Some(&format!("notif_detail::{}", n.key));
                let row_el = Hoverable::new(state, move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    let t = if self.settings.value.reduced_motion {
                        if hovered {
                            1.0
                        } else {
                            0.0
                        }
                    } else {
                        mouse.hover_progress().clamp(0.0, 1.0)
                    };
                    let active_bg = if p.dark {
                        rgb(0x22, 0x22, 0x28)
                    } else {
                        rgb(0xF4, 0xF4, 0xF7)
                    };
                    let bg = if mouse.is_clicked() {
                        p.accent_soft
                    } else if active_notif {
                        mix_color(active_bg, p.hover, t * 0.5)
                    } else {
                        mix_color(ColorU::transparent_black(), p.hover, t)
                    };
                    Container::new(row_inner)
                        .with_uniform_padding(12.)
                        .with_background_color(bg)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish()
                })
                .with_defer_events_to_children()
                .without_hover_animation()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(format!(
                        "notif_detail::{nkey_click}"
                    ))));
                })
                .on_double_click(move |ctx, _, _| {
                    if let Some(code) = &otp_dbl {
                        let pickup = is_pickup_code_text(code, &ntext_dbl);
                        ctx.dispatch_typed_action(Action::CopyText(
                            code.clone(),
                            if pickup {
                                format!("已复制取件码 {code}")
                            } else {
                                format!("已复制验证码 {code}")
                            },
                        ));
                    } else {
                        ctx.dispatch_typed_action(Action::Push(PushAction::OpenOnPhone(
                            nkey_dbl.clone(),
                        )));
                    }
                })
                .finish();
                #[cfg(feature = "ui-capture")]
                let row_el =
                    crate::ui::scenario_check::control(format!("push-syn-row-{i}"), row_el);
                let ts_ms = n.post_time_ms.max(0) as u64;
                combined_rows.push((is_pinned, ts_ms, format!("push:{}", n.key), row_el));
            }
        }

        combined_rows.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)).then_with(|| a.2.cmp(&b.2)));
        let mut thread_rows: Vec<El> = combined_rows.into_iter().map(|(_, _, _, el)| el).collect();

        if show_native_sms {
            if let Some(error) = &self.sms_error {
                if thread_rows.is_empty() {
                    thread_rows.insert(
                        0,
                        self.empty(
                            "短信读取失败",
                            error.clone(),
                            Some(self.button(
                                "sms-read-retry",
                                "重试读取",
                                Action::ReadSms,
                                Btn::Secondary,
                            )),
                        ),
                    );
                }
            } else if !loaded && thread_rows.is_empty() {
                thread_rows.push(self.empty(
                    if self.busy {
                        "正在读取手机短信…"
                    } else {
                        "尚未读取短信"
                    },
                    "读取完成后显示会话与短信数量。",
                    if self.busy {
                        None
                    } else {
                        Some(self.button(
                            "sms-first-read",
                            "读取短信",
                            Action::ReadSms,
                            Btn::Secondary,
                        ))
                    },
                ));
            }
        }

        if thread_rows.is_empty() {
            let is_sms_only = self.push.im_channel == ImChannel::Sms;
            thread_rows.push(self.empty(
                if is_sms_only {
                    if self.sms.is_empty() {
                        "手机暂无短信"
                    } else {
                        "没有匹配的短信"
                    }
                } else {
                    "暂无消息"
                },
                if is_sms_only {
                    "收到或发送短信后将显示在这里。"
                } else {
                    "手机收到新消息或短信后将显示在这里。"
                },
                Some(self.button(
                    if is_sms_only {
                        "sms-empty-refresh"
                    } else {
                        "im-empty-sync-win"
                    },
                    "刷新",
                    if is_sms_only {
                        Action::ReadSms
                    } else {
                        Action::Push(PushAction::SyncActivePhoneChatWindow)
                    },
                    Btn::Secondary,
                )),
            ));
        }

        let left_scroll = self.pane_scroll(
            self.sms_threads_scroll.clone(),
            Container::new(self.vstack(2., thread_rows))
                .with_horizontal_padding(S2)
                .with_vertical_padding(S2)
                .finish(),
        );

        let search_and_new_row = self.input(F_SMS_SEARCH, "搜索消息、联系人或验证码…", None);

        let left_top_controls = vec![search_and_new_row, self.wrap(channel_pills), self.list_page_controls(&thread_page_key,thread_total,40)];

        // Top "重要" strip: up to 4 compact cards (验证码/取件码/外卖/@我)
        let top_code_bar: Option<El> = {
            struct SmartValueCard {
                icon: UiIcon,
                tint: ColorU,
                title: String,
                code: Option<String>,
                jump: Action,
                /// Dismiss key for pickup/express cards (shows「已取件」).
                done: Option<String>,
            }

            let now_val_ms = now_ms as i64;
            let code_not_dismissed = |c: &str| {
                let up = c.trim().to_uppercase();
                !up.is_empty() && !self.push.prefs.dismissed_pickup_codes.contains(&up)
            };

            let latest_otp = self
                .sms
                .iter()
                .filter(|m| {
                    !m.read
                        && !m.outgoing
                        && (m.date_ms <= 0
                            || now_val_ms <= 1_700_000_000_000
                            || now_val_ms.saturating_sub(m.date_ms) <= 900_000)
                })
                .find_map(|m| {
                    extract_verification_code(&m.body).filter(|code| {
                        !is_pickup_code_text(code, &m.body) && code_not_dismissed(code)
                    })
                })
                .or_else(|| {
                    display_threads
                        .iter()
                        .filter(|t| {
                            t.unread
                                && (t.last_ts == 0
                                    || now_val_ms <= 1_700_000_000_000
                                    || now_val_ms.saturating_sub(t.last_ts as i64) <= 900_000)
                        })
                        .find_map(|t| {
                            t.otp_code
                                .as_ref()
                                .filter(|c| {
                                    !is_pickup_code_text(c, &t.last_body) && code_not_dismissed(c)
                                })
                                .cloned()
                        })
                })
                .or_else(|| {
                    snapshot
                        .otp_banners
                        .iter()
                        .filter(|b| {
                            !is_pickup_code_text(&b.code, &b.raw_text)
                                && code_not_dismissed(&b.code)
                                && (b.timestamp_ms <= 0
                                    || now_val_ms <= 1_700_000_000_000
                                    || now_val_ms.saturating_sub(b.timestamp_ms) <= 900_000)
                        })
                        .map(|b| b.code.clone())
                        .next()
                })
                .or_else(|| {
                    effective_push_notifs
                        .iter()
                        .filter(|n| {
                            !n.dismissed
                                && !n.read
                                && (n.post_time_ms <= 0
                                    || now_val_ms <= 1_700_000_000_000
                                    || now_val_ms.saturating_sub(n.post_time_ms) <= 900_000)
                        })
                        .find_map(|n| {
                            n.otp_code
                                .clone()
                                .or_else(|| extract_verification_code(&n.text))
                                .filter(|code| {
                                    !is_pickup_code_text(code, &n.text) && code_not_dismissed(code)
                                })
                        })
                });

            let latest_pickup = self
                .sms
                .iter()
                .filter(|m| {
                    !m.read
                        && !m.outgoing
                        && !stepx_companion::home_hub::is_completed_express_text(&m.body)
                        && (m.date_ms <= 0
                            || now_val_ms <= 1_700_000_000_000
                            || now_val_ms.saturating_sub(m.date_ms) <= 86_400_000)
                })
                .find_map(|m| extract_pickup_code(&m.body).filter(|c| code_not_dismissed(c)))
                .or_else(|| {
                    display_threads
                        .iter()
                        .filter(|t| {
                            t.unread
                                && !stepx_companion::home_hub::is_completed_express_text(
                                    &t.last_body,
                                )
                                && (t.last_ts == 0
                                    || now_val_ms <= 1_700_000_000_000
                                    || now_val_ms.saturating_sub(t.last_ts as i64) <= 86_400_000)
                        })
                        .find_map(|t| {
                            extract_pickup_code(&t.last_body).filter(|c| code_not_dismissed(c))
                        })
                })
                .or_else(|| {
                    snapshot
                        .otp_banners
                        .iter()
                        .filter(|b| {
                            is_pickup_code_text(&b.code, &b.raw_text)
                                && code_not_dismissed(&b.code)
                                && !stepx_companion::home_hub::is_completed_express_text(
                                    &b.raw_text,
                                )
                                && (b.timestamp_ms <= 0
                                    || now_val_ms <= 1_700_000_000_000
                                    || now_val_ms.saturating_sub(b.timestamp_ms) <= 86_400_000)
                        })
                        .map(|b| b.code.clone())
                        .next()
                })
                .or_else(|| {
                    effective_push_notifs
                        .iter()
                        .filter(|n| {
                            !n.dismissed
                                && !n.read
                                && !stepx_companion::home_hub::is_completed_express_text(&n.text)
                                && (n.post_time_ms <= 0
                                    || now_val_ms <= 1_700_000_000_000
                                    || now_val_ms.saturating_sub(n.post_time_ms) <= 86_400_000)
                        })
                        .find_map(|n| {
                            extract_pickup_code(&n.text).filter(|c| code_not_dismissed(c))
                        })
                })
                .or_else(|| {
                    im_threads_source
                        .iter()
                        .filter(|t| {
                            t.unread_count > 0
                                && (t.last_timestamp_ms <= 0
                                    || now_val_ms <= 1_700_000_000_000
                                    || now_val_ms.saturating_sub(t.last_timestamp_ms) <= 86_400_000)
                        })
                        .find_map(|t| {
                            t.messages
                                .iter()
                                .rev()
                                .filter(|m| {
                                    !stepx_companion::home_hub::is_completed_express_text(&m.text)
                                })
                                .find_map(|m| {
                                    extract_pickup_code(&m.text).filter(|c| code_not_dismissed(c))
                                })
                                .or_else(|| {
                                    (!stepx_companion::home_hub::is_completed_express_text(
                                        &t.last_message,
                                    ))
                                    .then(|| extract_pickup_code(&t.last_message))
                                    .flatten()
                                    .filter(|c| code_not_dismissed(c))
                                })
                        })
                })
                .or_else(|| {
                    self.express_parcels
                        .iter()
                        .filter(|parcel| {
                            crate::ui::notification_center_state::is_real_pickup_code_label(
                                &parcel.pickup_code,
                            ) && code_not_dismissed(&parcel.pickup_code)
                        })
                        .map(|parcel| parcel.pickup_code.trim().to_string())
                        .next()
                });

            let mut cards: Vec<SmartValueCard> = Vec::new();
            let amber = rgb(0xF5, 0x9E, 0x0B);
            let rose = rgb(0xEC, 0x48, 0x99);
            let emerald = rgb(0x05, 0x96, 0x69);

            if let Some(pickup) = &latest_pickup {
                let jump = display_threads
                    .iter()
                    .find(|d| extract_pickup_code(&d.last_body).as_deref() == Some(pickup.as_str()))
                    .map(|d| Action::SelectSmsThread(Some(d.num.clone())))
                    .or_else(|| {
                        im_threads_source
                            .iter()
                            .find(|t| {
                                t.last_message.contains(pickup)
                                    || t.messages.iter().any(|m| m.text.contains(pickup))
                            })
                            .map(|t| Action::Push(PushAction::SelectImThread(t.thread_id.clone())))
                    })
                    .or_else(|| {
                        effective_push_notifs
                            .iter()
                            .find(|n| {
                                n.otp_code.as_deref() == Some(pickup.as_str())
                                    || n.text.contains(pickup.as_str())
                                    || n.title.contains(pickup.as_str())
                            })
                            .map(|n| {
                                Action::Push(PushAction::SelectImThread(format!(
                                    "notif_detail::{}",
                                    n.key
                                )))
                            })
                    })
                    .unwrap_or_else(|| {
                        Action::Push(PushAction::SelectImThread(
                            "notif_detail::phone-clipboard".into(),
                        ))
                    });
                cards.push(SmartValueCard {
                    icon: UiIcon::Package,
                    tint: amber,
                    title: format!("取件码 {pickup}"),
                    code: Some(pickup.clone()),
                    jump,
                    done: Some(pickup.clone()),
                });
            }

            if let Some(otp) = &latest_otp {
                let jump = display_threads
                    .iter()
                    .find(|d| d.otp_code.as_deref() == Some(otp.as_str()))
                    .map(|d| Action::SelectSmsThread(Some(d.num.clone())))
                    .or_else(|| {
                        im_threads_source
                            .iter()
                            .find(|t| {
                                t.last_message.contains(otp)
                                    || t.messages.iter().any(|m| m.text.contains(otp))
                            })
                            .map(|t| Action::Push(PushAction::SelectImThread(t.thread_id.clone())))
                    })
                    .or_else(|| {
                        effective_push_notifs
                            .iter()
                            .find(|n| {
                                n.otp_code.as_deref() == Some(otp.as_str())
                                    || n.text.contains(otp.as_str())
                                    || n.title.contains(otp.as_str())
                            })
                            .map(|n| {
                                Action::Push(PushAction::SelectImThread(format!(
                                    "notif_detail::{}",
                                    n.key
                                )))
                            })
                    })
                    .unwrap_or_else(|| {
                        Action::Push(PushAction::SelectImThread(
                            "notif_detail::phone-clipboard".into(),
                        ))
                    });
                cards.push(SmartValueCard {
                    icon: UiIcon::Key,
                    tint: p.accent,
                    title: format!("验证码 {otp}"),
                    code: Some(otp.clone()),
                    jump,
                    done: None,
                });
            }

            let phone_clip_val = self.back_screen.stepos_suite.phone_clipboard.trim();
            if !phone_clip_val.is_empty() && cards.len() < 4 {
                let short_clip: String = phone_clip_val.chars().take(12).collect();
                let clip_nkey = effective_push_notifs
                    .iter()
                    .find(|n| n.key.starts_with("phone-clip"))
                    .map(|n| n.key.clone())
                    .unwrap_or_else(|| "phone-clipboard".to_string());
                cards.push(SmartValueCard {
                    icon: UiIcon::Copy,
                    tint: p.blue,
                    title: format!("剪贴板: {short_clip}"),
                    code: Some(phone_clip_val.to_string()),
                    jump: Action::Push(PushAction::SelectImThread(format!(
                        "notif_detail::{clip_nkey}"
                    ))),
                    done: None,
                });
            }

            // Live capsules: 外卖/买菜, 有人@我, 出行/打车/航旅, 职场/日程
            for cap in self.collect_top_bar_live_capsules() {
                if cards.len() >= 4 {
                    break;
                }
                if !matches!(
                    cap.kind,
                    crate::ui::shell::TopBarCapsuleKind::InstantDelivery
                        | crate::ui::shell::TopBarCapsuleKind::DirectMention
                        | crate::ui::shell::TopBarCapsuleKind::TravelMobility
                        | crate::ui::shell::TopBarCapsuleKind::CareerRecruitment
                        | crate::ui::shell::TopBarCapsuleKind::ScheduleOrAlarm
                ) {
                    continue;
                }
                if cards.iter().any(|c| {
                    c.title == cap.title
                        || c.title.contains(&cap.title)
                        || cap.title.contains(&c.title)
                }) {
                    continue;
                }
                let (tint, _) = self.live_capsule_kind_tint(cap.kind);
                cards.push(SmartValueCard {
                    icon: cap.icon,
                    tint,
                    title: cap.title.clone(),
                    code: cap.copy_value.clone(),
                    jump: cap.primary_action.clone(),
                    done: None,
                });
            }

            // Also scan unread SMS threads for arrived grocery / delivery signals (e.g. 【叮咚买菜】) not yet in cards
            for d in &display_threads {
                if cards.len() >= 4 {
                    break;
                }
                if !d.unread {
                    continue;
                }
                if let Some((brand, headline, _detail, _is_arrived)) =
                    stepx_companion::home_hub::classify_delivery_or_mobility_signal(
                        "",
                        "",
                        &d.num,
                        &d.last_body,
                    )
                {
                    if extract_pickup_code(&d.last_body).is_some() {
                        continue; // already covered by latest_pickup
                    }
                    if !code_not_dismissed(&headline) {
                        continue;
                    }
                    if cards.iter().any(|c| {
                        c.title.contains(&brand)
                            || c.title.contains(&headline)
                            || headline.contains(&c.title)
                    }) {
                        continue;
                    }
                    let is_grocery = brand.contains("叮咚")
                        || brand.contains("盒马")
                        || brand.contains("山姆")
                        || brand.contains("朴朴");
                    cards.push(SmartValueCard {
                        icon: UiIcon::Package,
                        tint: if is_grocery { emerald } else { amber },
                        title: headline.clone(),
                        code: None,
                        jump: Action::SelectSmsThread(Some(d.num.clone())),
                        done: Some(headline.clone()),
                    });
                }
            }

            for t in &im_threads_source {
                if cards.len() >= 4 {
                    break;
                }
                let effective_ts = t.last_timestamp_ms.max(
                    t.messages
                        .iter()
                        .map(|m| m.timestamp_ms.max(m.observed_at_ms))
                        .max()
                        .unwrap_or(0),
                );
                let im_item_id = format!("im-thread:{}", t.thread_id);
                if t.unread_count == 0
                    || effective_ts <= 0
                    || self.notification_center.read_ids.contains(&im_item_id)
                    || self.notification_center.dismissed_ids.contains(&im_item_id)
                    || (now_val_ms > 1_700_000_000_000
                        && now_val_ms.saturating_sub(effective_ts) > 43_200_000)
                {
                    continue;
                }
                for (kind, entity_name, raw_snip) in extract_thread_value_signals(t) {
                    if cards.len() >= 4 {
                        break;
                    }
                    let compact_entity = compact_thread_title(&entity_name, 10);

                    match kind {
                        ImEntityKind::Express => {
                            if !cards.iter().any(|c| c.title.contains(&compact_entity)) {
                                let code = extract_pickup_code(&raw_snip);
                                if code.as_deref().is_some_and(|c| !code_not_dismissed(c))
                                    || !code_not_dismissed(&compact_entity)
                                {
                                    continue;
                                }
                                cards.push(SmartValueCard {
                                    icon: UiIcon::Package,
                                    tint: amber,
                                    done: Some(
                                        code.clone().unwrap_or_else(|| compact_entity.clone()),
                                    ),
                                    title: compact_entity,
                                    code: code.clone(),
                                    jump: Action::Push(PushAction::SelectImThread(
                                        t.thread_id.clone(),
                                    )),
                                });
                            }
                        }
                        ImEntityKind::FoodTea => {
                            if !cards.iter().any(|c| c.title.contains(&compact_entity)) {
                                cards.push(SmartValueCard {
                                    icon: UiIcon::Coffee,
                                    tint: rose,
                                    done: None,
                                    title: compact_entity,
                                    code: None,
                                    jump: Action::Push(PushAction::SelectImThread(
                                        t.thread_id.clone(),
                                    )),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }

            if !cards.is_empty() {
                let mut card_els: Vec<El> =
                    vec![self.line_w("重要", CAPTION, p.text, Weight::Semibold)];
                for (c_idx, card) in cards.into_iter().take(4).enumerate() {
                    let action_btn = if let Some(code) = &card.code {
                        self.button_icon(
                            format!("sms-val-copy-{c_idx}"),
                            "复制",
                            UiIcon::Copy,
                            Action::CopyText(code.clone(), format!("已复制 {}", code)),
                            Btn::Ghost,
                        )
                    } else {
                        self.button_icon(
                            format!("sms-val-open-{c_idx}"),
                            "打开",
                            UiIcon::ChevronRight,
                            card.jump.clone(),
                            Btn::Ghost,
                        )
                    };
                    let action_btn = if let Some(key) = &card.done {
                        self.hstack(
                            4.,
                            vec![
                                action_btn,
                                self.button_icon(
                                    format!("sms-val-done-{c_idx}"),
                                    "已取件",
                                    UiIcon::Check,
                                    Action::Push(PushAction::DismissPickupCode(key.clone())),
                                    Btn::Ghost,
                                ),
                            ],
                        )
                    } else {
                        action_btn
                    };
                    let card_inner = self.between(
                        Expanded::new(
                            1.,
                            Clipped::new(self.hstack(
                                6.,
                                vec![
                                    self.content_icon(card.icon, 14., card.tint),
                                    self.line_w(
                                        &compact_thread_title(&card.title, 12),
                                        CAPTION,
                                        p.text,
                                        Weight::Semibold,
                                    ),
                                ],
                            ))
                            .finish(),
                        )
                        .finish(),
                        action_btn,
                    );
                    let jump_act = card.jump.clone();
                    let tint_c = card.tint;
                    let state = self.mouse_state(format!("sms-val-card-st:{c_idx}"));
                    let card_box = Hoverable::new(state, move |mouse| {
                        let hovered = mouse.is_mouse_over_element();
                        let border_c = if hovered || mouse.is_clicked() {
                            tint_c
                        } else {
                            p.border
                        };
                        Container::new(card_inner)
                            .with_horizontal_padding(8.)
                            .with_vertical_padding(3.)
                            .with_background_color(p.bg)
                            .with_border(Border::all(1.).with_border_color(border_c))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                            .finish()
                    })
                    .with_defer_events_to_children()
                    .without_hover_animation()
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(jump_act.clone());
                    })
                    .finish();
                    #[cfg(feature = "ui-capture")]
                    let card_box = crate::ui::scenario_check::control(
                        format!("sms-value-card-{c_idx}"),
                        card_box,
                    );
                    card_els.push(Expanded::new(1.0, card_box).finish());
                }

                let cards_row = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(8.0)
                    .with_children(card_els)
                    .finish();

                let hub = Container::new(cards_row)
                    .with_horizontal_padding(S5)
                    .with_vertical_padding(6.)
                    .with_background_color(p.surface)
                    .with_border(Border::bottom(1.).with_border_color(p.border))
                    .finish();
                #[cfg(feature = "ui-capture")]
                let hub = crate::ui::scenario_check::control("sms-ai-value-hub".to_string(), hub);
                Some(hub)
            } else {
                None
            }
        };

        let left_pane = self.fixed_w(
            LIST_PANE_W + 24.,
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(
                        Container::new(self.vstack(8., left_top_controls))
                            .with_horizontal_padding(S4)
                            .with_vertical_padding(S3)
                            .with_border(Border::bottom(1.).with_border_color(p.border))
                            .finish(),
                    )
                    .with_child(Expanded::new(1., left_scroll).finish())
                    .finish(),
            )
            .with_background_color(if p.dark { p.sidebar } else { p.bg })
            .with_border(Border::right(1.).with_border_color(p.border))
            .finish(),
        );

        // If a notification / clipboard row is selected in the left list, synthesize a detail thread view in the right pane
        if active_im_thread.is_none() {
            if let Some(nkey) = self
                .push
                .selected_im_thread
                .as_deref()
                .and_then(|s| s.strip_prefix("notif_detail::"))
            {
                let synthesized = effective_push_notifs
                    .iter()
                    .find(|n| {
                        n.key == nkey
                            || (nkey == "phone-clipboard" && n.key.starts_with("phone-clip"))
                    })
                    .map(|n| {
                        let app_label = if n.app_name.trim().is_empty() {
                            stepx_companion::push::ImChannel::from_package(&n.package_name)
                                .label()
                                .to_string()
                        } else {
                            n.app_name.clone()
                        };
                        let title = if n.title.trim().is_empty() {
                            app_label.clone()
                        } else {
                            n.title.clone()
                        };
                        let ts_ms = n.post_time_ms.max(0);
                        ImThread {
                            thread_id: format!("notif_detail::{}", n.key),
                            package_name: n.package_name.clone(),
                            app_name: app_label.clone(),
                            title: title.clone(),
                            last_message: n.text.clone(),
                            last_sender: title.clone(),
                            last_timestamp_ms: ts_ms,
                            unread_count: usize::from(!n.read),
                            can_ui_reply: n.can_reply,
                            notification_key: n.key.clone(),
                            is_group: false,
                            messages: vec![stepx_companion::push::ImMessage {
                                id: n.key.clone(),
                                sender: title,
                                text: n.text.clone(),
                                timestamp_ms: ts_ms,
                                outgoing: false,
                                via: if n.key.starts_with("phone-clip") {
                                    "clipboard".into()
                                } else {
                                    "notification".into()
                                },
                                ..Default::default()
                            }],
                            ..Default::default()
                        }
                    })
                    .or_else(|| {
                        let clip = self.back_screen.stepos_suite.phone_clipboard.trim();
                        (!clip.is_empty()).then(|| ImThread {
                            thread_id: "notif_detail::phone-clipboard".into(),
                            package_name: "com.stepx.clipboard".into(),
                            app_name: "跨端剪贴".into(),
                            title: "手机剪贴板已同步".into(),
                            last_message: clip.to_string(),
                            last_sender: "手机剪贴板".into(),
                            last_timestamp_ms: 0,
                            unread_count: 0,
                            can_ui_reply: false,
                            notification_key: "phone-clipboard".into(),
                            is_group: false,
                            messages: vec![stepx_companion::push::ImMessage {
                                id: "phone-clipboard".into(),
                                sender: "手机剪贴板".into(),
                                text: clip.to_string(),
                                timestamp_ms: 0,
                                outgoing: false,
                                via: "clipboard".into(),
                                ..Default::default()
                            }],
                            ..Default::default()
                        })
                    });
                if let Some(synth_thread) = synthesized {
                    let right_pane = self.im_thread_dialog(&synth_thread, snapshot);
                    let split = Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_child(left_pane)
                        .with_child(Expanded::new(1., right_pane).finish())
                        .finish();
                    let mut page_col = Flex::column()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_child(self.comm_mode_bar());
                    if let Some(bar) = top_code_bar {
                        page_col = page_col.with_child(bar);
                    }
                    return page_col
                        .with_child(Expanded::new(1., split).finish())
                        .finish();
                }
            }
        }

        // If an IM thread is selected, render the IM chat bubble dialog & direct reply composer
        if let Some(im_thread) = active_im_thread {
            let right_pane = self.im_thread_dialog(im_thread, snapshot);
            let split = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(left_pane)
                .with_child(Expanded::new(1., right_pane).finish())
                .finish();
            let mut page_col = Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.comm_mode_bar());
            if let Some(bar) = top_code_bar {
                page_col = page_col.with_child(bar);
            }
            return page_col
                .with_child(Expanded::new(1., split).finish())
                .finish();
        }

        // Otherwise render the native SMS conversation & SMS composer unchanged
        let selected_thread_info = display_threads
            .iter()
            .find(|t| norm_addr(&t.num) == norm_addr(&active_addr));

        let (peer_name, peer_num) = if is_new_sms {
            let typed = self.fields[F_SMS_TO].text.trim();
            (
                "新建短信会话".to_string(),
                if typed.is_empty() {
                    String::new()
                } else {
                    typed.to_string()
                },
            )
        } else if let Some(info) = selected_thread_info {
            (info.name.clone(), info.num.clone())
        } else if active_addr.is_empty() {
            ("短信会话".into(), String::new())
        } else {
            let (n, _) = resolve_sender_info(&active_addr, &self.contacts);
            (n, active_addr.clone())
        };

        let sims = self
            .capabilities
            .as_ref()
            .map(|c| c.subscriptions.as_slice())
            .unwrap_or(&[]);
        let selected_sim = self
            .selected_sim
            .or_else(|| self.capabilities.as_ref().map(|c| c.default_subscription));
        let real_sim_badge = sims
            .iter()
            .find(|s| Some(s.id) == selected_sim)
            .map(|s| {
                format!(
                    "卡 {} · {}",
                    super::phone::sim_slot_number(sims, s.id),
                    stepx_companion::bridge::friendly_carrier_name(&s.label)
                )
            })
            .unwrap_or_else(|| "请选择可用 SIM".into());
        let can_send = self.capabilities.as_ref().is_some_and(|c| c.send_sms)
            && sims.iter().any(|s| Some(s.id) == selected_sim);
        let has_recipient = !peer_num.trim().is_empty();
        let is_composing_without_history =
            is_new_sms || (selected_thread_info.is_none() && has_recipient);
        let real_msgs = self.thread_messages(&active_addr);
        let latest_sms_otp = real_msgs.iter().rev().find_map(|m| {
            extract_verification_code(&m.body)
                .map(|code| (code.clone(), is_pickup_code_text(&code, &m.body)))
        });

        let sms_hdr_avatar = self.render_im_entity_avatar(
            "com.android.mms",
            "短信",
            &peer_name,
            real_msgs.last().map(|m| m.body.as_str()).unwrap_or(""),
            false,
            &[],
            34.0,
        );
        let sms_norm = norm_addr(&peer_num);
        let sms_addr_key = if sms_norm.is_empty() {
            peer_num.trim().to_string()
        } else {
            sms_norm
        };
        let sms_pin_key =
            stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(&sms_addr_key);
        let sms_pinned = has_recipient
            && !is_composing_without_history
            && self.push.prefs.is_pinned(&sms_pin_key);
        let mut peer_labels = vec![
            sms_hdr_avatar,
            self.line_w(&peer_name, CAPTION, p.text, Weight::Semibold),
        ];
        if sms_pinned {
            peer_labels.push(self.pill_badge_icon("已置顶", UiIcon::Pin, p.accent, p.accent_soft));
        }
        if has_recipient {
            if peer_name != peer_num {
                peer_labels.push(self.mono(&peer_num, CAPTION, p.text2));
            }
            peer_labels.push(self.tag(&real_sim_badge, p.text2));
        }
        let mut top_peer_actions = Vec::new();
        if is_composing_without_history {
            top_peer_actions.push(self.button_icon(
                "sms-cancel-new",
                "取消新建",
                UiIcon::Close,
                Action::SelectSmsThread(None),
                Btn::Secondary,
            ));
        }
        if let Some((code, pickup)) = &latest_sms_otp {
            top_peer_actions.push(self.button_icon(
                "sms-hdr-copy-otp",
                if *pickup {
                    format!("复制取件码: {code}")
                } else {
                    format!("复制验证码: {code}")
                },
                if *pickup {
                    UiIcon::Package
                } else {
                    UiIcon::Copy
                },
                Action::CopyText(
                    code.clone(),
                    if *pickup {
                        format!("已复制取件码 {code}")
                    } else {
                        format!("已复制验证码 {code}")
                    },
                ),
                Btn::Primary,
            ));
        }
        if has_recipient {
            top_peer_actions.push(self.button_icon(
                "sms-call-peer",
                "呼叫",
                UiIcon::Phone,
                Action::PhoneCall(peer_num.clone()),
                Btn::Secondary,
            ));
            top_peer_actions.push(self.button_icon(
                "sms-copy-num",
                "复制号码",
                UiIcon::Clipboard,
                Action::CopyText(peer_num.clone(), format!("已复制号码 {peer_num}")),
                Btn::Secondary,
            ));
        }

        let mut common_peer_actions = Vec::new();
        if has_recipient && !is_composing_without_history {
            common_peer_actions.push(self.button_icon(
                "sms-hdr-pin",
                if sms_pinned { "取消置顶" } else { "置顶" },
                UiIcon::Pin,
                Action::Push(PushAction::TogglePinTarget(sms_pin_key.clone())),
                if sms_pinned {
                    Btn::Secondary
                } else {
                    Btn::Ghost
                },
            ));
            let sms_is_unread = selected_thread_info.is_some_and(|i| i.unread)
                || self.push.prefs.manual_unread_targets.contains(&sms_pin_key);
            common_peer_actions.push(self.button_icon(
                "sms-hdr-toggle-read",
                if sms_is_unread {
                    "标为已读"
                } else {
                    "标为未读"
                },
                UiIcon::Check,
                Action::Push(PushAction::ToggleThreadRead {
                    is_im: false,
                    target_id: peer_num.clone(),
                }),
                Btn::Ghost,
            ));
            common_peer_actions.push(self.button_icon(
                "sms-hdr-clear-history",
                "清空记录",
                UiIcon::Trash,
                Action::Push(PushAction::ClearThreadMessages {
                    is_im: false,
                    target_id: peer_num.clone(),
                    package_name: "com.android.mms".into(),
                    title: peer_name.clone(),
                }),
                Btn::Ghost,
            ));
            common_peer_actions.push(self.button_icon(
                "sms-hdr-delete-thread",
                "删除对话",
                UiIcon::Trash,
                Action::Push(PushAction::DeleteSmsThread {
                    address: peer_num.clone(),
                    name: peer_name.clone(),
                }),
                Btn::Ghost,
            ));
        }

        // Right Conversation Detail Pane
        let mut all_sms_hdr_actions = common_peer_actions;
        if !all_sms_hdr_actions.is_empty() && !top_peer_actions.is_empty() {
            all_sms_hdr_actions.push(
                ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background_color(p.border)
                        .finish(),
                )
                .with_width(1.)
                .with_height(16.)
                .finish(),
            );
        }
        all_sms_hdr_actions.extend(top_peer_actions);

        let top_sms_row = self.between(
            self.hstack(10., peer_labels),
            self.hstack(4., all_sms_hdr_actions),
        );
        let subheader = Container::new(top_sms_row)
            .with_horizontal_padding(S5)
            .with_vertical_padding(12.)
            .with_background_color(p.bg)
            .with_border(Border::bottom(1.).with_border_color(p.border))
            .finish();

        let date_pill = |label: &str| -> El {
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_child(
                    Container::new(self.line(label, CAPTION, p.text2))
                        .with_horizontal_padding(10.)
                        .with_vertical_padding(2.)
                        .with_background_color(if p.dark {
                            rgb(0x22, 0x22, 0x27)
                        } else {
                            rgb(0xEF, 0xEF, 0xF2)
                        })
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
                        .finish(),
                )
                .finish()
        };

        let incoming_bubble = |idx: usize, txt: &str, meta: &str, date_ms: i64| -> El {
            let clean_txt = sanitize_ui_display_text(txt);
            let display_txt = if clean_txt.is_empty() {
                txt.to_string()
            } else {
                clean_txt
            };
            let b = Container::new(self.chat_bubble_text_node(&display_txt, BODY, p.text, 440.0))
                .with_horizontal_padding(14.)
                .with_vertical_padding(9.)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(Self::chat_bubble_radius(false, false))
                .finish();
            let in_key = format!("sms-bub-in:{idx}");
            let otp_id = format!("sms-in-otp-{idx}");
            let url_id = format!("sms-in-url-{idx}");
            let copy_id = format!("sms-in-copy-{idx}");
            let quote_id = format!("sms-msg-quote-{idx}");
            let del_id = format!("sms-in-del-{idx}");
            let backscreen_id = format!("sms-in-backscreen-{idx}");
            let mut meta_items: Vec<El> =
                vec![Container::new(self.mono(meta.to_string(), MICRO, p.text3))
                    .with_padding_left(2.)
                    .finish()];
            let detected_otp = extract_verification_code(txt);
            if let Some(code) = &detected_otp {
                let pickup = is_pickup_code_text(code, txt);
                meta_items.push(self.msg_inline_svg_action_btn(
                    otp_id,
                    if pickup {
                        UiIcon::Package
                    } else {
                        UiIcon::Copy
                    },
                    true,
                    Action::CopyText(
                        code.clone(),
                        if pickup {
                            format!("已复制取件码 {code}")
                        } else {
                            format!("已复制验证码 {code}")
                        },
                    ),
                ));
                if pickup {
                    meta_items.push(self.msg_inline_svg_action_btn(
                        backscreen_id,
                        UiIcon::Matrix,
                        false,
                        Action::AmooPushPickupCode(code.clone()),
                    ));
                }
            }
            if let Some(url) = extract_first_url(txt) {
                meta_items.push(self.msg_inline_svg_action_btn(
                    url_id,
                    UiIcon::ExternalLink,
                    false,
                    Action::StudioOpenUrl(url),
                ));
            }
            {
                meta_items.push(self.msg_inline_svg_action_btn(
                    copy_id,
                    UiIcon::Copy,
                    false,
                    Action::CopyText(display_txt.clone(), "已复制短信正文".into()),
                ));
                meta_items.push(self.msg_inline_svg_action_btn(
                    quote_id,
                    UiIcon::Reply,
                    false,
                    Action::Push(PushAction::QuoteReply {
                        is_im: false,
                        sender: peer_name.clone(),
                        text: display_txt.clone(),
                    }),
                ));
                meta_items.push(self.msg_inline_svg_action_btn(
                    del_id,
                    UiIcon::Trash,
                    false,
                    Action::Push(PushAction::DeleteSmsMessage {
                        address: peer_num.clone(),
                        date_ms,
                        body: display_txt.clone(),
                    }),
                ));
            }
            let meta_row_fixed = ConstrainedBox::new(
                Flex::row()
                    .with_spacing(5.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children(meta_items)
                    .finish(),
            )
            .with_height(18.0)
            .finish();
            let col = Flex::column()
                .with_spacing(3.)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(meta_row_fixed)
                .with_child(b)
                .finish();
            let sender_av = self.render_sender_avatar(&peer_name, txt, 30.0);
            let row_with_av = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(8.)
                .with_child(sender_av)
                .with_child(Shrinkable::new(1., col).finish())
                .finish();
            let state = self.mouse_state(in_key);
            let txt_dbl = display_txt.clone();
            let txt_rc = display_txt;
            let otp_rc = detected_otp;
            let addr_rc = peer_num.clone();
            let sender_rc = peer_name.clone();
            Hoverable::new(state, move |_| row_with_av)
                .with_defer_events_to_children()
                .without_hover_animation()
                .with_cursor(Cursor::PointingHand)
                .on_double_click(move |ctx, _, _| {
                    if let Some(code) = extract_verification_code(&txt_dbl) {
                        let pickup = is_pickup_code_text(&code, &txt_dbl);
                        ctx.dispatch_typed_action(Action::CopyText(
                            code.clone(),
                            if pickup {
                                format!("已复制取件码 {code}")
                            } else {
                                format!("已复制验证码 {code}")
                            },
                        ));
                    } else {
                        ctx.dispatch_typed_action(Action::CopyText(
                            txt_dbl.clone(),
                            "已复制短信全文".into(),
                        ));
                    }
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::SmsMessage {
                            address: addr_rc.clone(),
                            sender: sender_rc.clone(),
                            body: txt_rc.clone(),
                            date_ms,
                            otp_code: otp_rc.clone(),
                        },
                        at,
                    ));
                })
                .finish()
        };

        let outgoing_bubble = |idx: usize, txt: &str, meta: &str, date_ms: i64| -> El {
            let clean_txt = sanitize_ui_display_text(txt);
            let display_txt = if clean_txt.is_empty() {
                txt.to_string()
            } else {
                clean_txt
            };
            let b =
                Container::new(self.chat_bubble_text_node(&display_txt, BODY, p.accent_fg, 440.0))
                    .with_horizontal_padding(14.)
                    .with_vertical_padding(9.)
                    .with_background_color(p.accent)
                    .with_corner_radius(Self::chat_bubble_radius(true, false))
                    .finish();
            let out_key = format!("sms-bub-out:{idx}");
            let copy_id = format!("sms-out-copy-{idx}");
            let quote_id = format!("sms-msg-quote-{idx}");
            let del_id = format!("sms-out-del-{idx}");
            let mut meta_children =
                vec![Container::new(self.mono(meta.to_string(), MICRO, p.text3))
                    .with_padding_right(2.)
                    .finish()];
            {
                meta_children.push(self.msg_inline_svg_action_btn(
                    copy_id,
                    UiIcon::Copy,
                    false,
                    Action::CopyText(display_txt.clone(), "已复制短信正文".into()),
                ));
                meta_children.push(self.msg_inline_svg_action_btn(
                    quote_id,
                    UiIcon::Reply,
                    false,
                    Action::Push(PushAction::QuoteReply {
                        is_im: false,
                        sender: "我".into(),
                        text: display_txt.clone(),
                    }),
                ));
                meta_children.push(self.msg_inline_svg_action_btn(
                    del_id,
                    UiIcon::Trash,
                    false,
                    Action::Push(PushAction::DeleteSmsMessage {
                        address: peer_num.clone(),
                        date_ms,
                        body: display_txt.clone(),
                    }),
                ));
            }
            let meta_row_fixed = ConstrainedBox::new(
                Flex::row()
                    .with_spacing(5.)
                    .with_main_axis_alignment(MainAxisAlignment::End)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children(meta_children)
                    .finish(),
            )
            .with_height(18.0)
            .finish();
            let self_av = self.render_sender_avatar("我", "", 30.0);
            let row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(8.)
                .with_child(
                    Shrinkable::new(
                        1.,
                        Flex::column()
                            .with_spacing(3.)
                            .with_cross_axis_alignment(CrossAxisAlignment::End)
                            .with_child(b)
                            .with_child(meta_row_fixed)
                            .finish(),
                    )
                    .finish(),
                )
                .with_child(self_av)
                .finish();
            let state = self.mouse_state(out_key);
            let txt_dbl = display_txt.clone();
            let txt_rc = display_txt;
            let otp_rc = extract_verification_code(txt);
            let addr_rc = peer_num.clone();
            let sender_rc = peer_name.clone();
            Hoverable::new(state, move |_| row)
                .with_defer_events_to_children()
                .without_hover_animation()
                .with_cursor(Cursor::PointingHand)
                .on_double_click(move |ctx, _, _| {
                    if let Some(code) = extract_verification_code(&txt_dbl) {
                        let pickup = is_pickup_code_text(&code, &txt_dbl);
                        ctx.dispatch_typed_action(Action::CopyText(
                            code.clone(),
                            if pickup {
                                format!("已复制取件码 {code}")
                            } else {
                                format!("已复制验证码 {code}")
                            },
                        ));
                    } else {
                        ctx.dispatch_typed_action(Action::CopyText(
                            txt_dbl.clone(),
                            "已复制短信全文".into(),
                        ));
                    }
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::SmsMessage {
                            address: addr_rc.clone(),
                            sender: sender_rc.clone(),
                            body: txt_rc.clone(),
                            date_ms,
                            otp_code: otp_rc.clone(),
                        },
                        at,
                    ));
                })
                .finish()
        };

        let mut timeline_children: Vec<El> = Vec::new();

        if is_new_sms {
            timeline_children.push(date_pill("新建短信 · 输入号码与正文后确认发送"));
        } else if !active_addr.is_empty() && real_msgs.is_empty() {
            if has_recipient {
                timeline_children.push(date_pill(&format!(
                    "向 {peer_name} ({peer_num}) 发送新短信，在下方输入内容即可发送。"
                )));
            } else {
                timeline_children.push(date_pill("暂无短信，请选择会话或新建短信"));
            }
        }

        if !real_msgs.is_empty() {
            let page_key = format!("sms-history:{active_addr}");
            let newest_range = self.list_page_range(&page_key,real_msgs.len(),80);
            let visible = real_msgs.len()-newest_range.end..real_msgs.len()-newest_range.start;
            timeline_children.push(self.list_page_controls(&page_key,real_msgs.len(),80));
            for (idx, m) in real_msgs.iter().enumerate().skip(visible.start).take(visible.len()) {
                let ts = format_sms_time(m.date_ms);
                let sim = sims.iter().find(|s| Some(s.id) == m.subscription).map(|s| {
                    format!(
                        "卡 {} · {}",
                        super::phone::sim_slot_number(sims, s.id),
                        stepx_companion::bridge::friendly_carrier_name(&s.label)
                    )
                });
                let base_meta = match sim {
                    Some(sim) => format!("{ts} · {} · {sim}", m.status_label()),
                    None => format!("{ts} · {}", m.status_label()),
                };
                if m.outgoing {
                    let out_meta = format!("我 · {base_meta}");
                    timeline_children.push(outgoing_bubble(idx, &m.body, &out_meta, m.date_ms));
                } else {
                    timeline_children.push(incoming_bubble(idx, &m.body, &base_meta, m.date_ms));
                }
            }
        }

        let timeline = if active_addr.is_empty() && !is_new_sms {
            Container::new(
                Align::new(self.empty(
                    if self.sms_error.is_some() {
                        "短信暂时无法读取"
                    } else if !loaded {
                        "等待短信同步"
                    } else if self.sms.is_empty() {
                        "手机暂无短信"
                    } else {
                        "请选择一个会话"
                    },
                    if self.sms_error.is_some() {
                        "请重试读取手机短信。"
                    } else if !loaded {
                        "同步完成后可在左侧选择会话。"
                    } else {
                        "也可以点击“新建短信”填写收件人与内容。"
                    },
                    None,
                ))
                .finish(),
            )
            .with_uniform_padding(S5)
            .finish()
        } else {
            let chat_canvas_bg = if p.dark {
                rgb(0x12, 0x13, 0x17)
            } else {
                rgb(0xF6, 0xF7, 0xF9)
            };
            let timeline_inner = Container::new(self.vstack(14., timeline_children))
                .with_horizontal_padding(24.)
                .with_padding_top(16.)
                .with_padding_bottom(24.)
                .finish();
            Container::new(self.sms_stream_scroll(
                format!("sms:{active_addr}"),
                real_msgs.len(),
                timeline_inner,
            ))
            .with_background_color(chat_canvas_bg)
            .finish()
        };

        // Inline Send Safety Confirmation Bar + Composer
        let sim_label = &real_sim_badge;
        let next_sim = match sims.iter().position(|s| Some(s.id) == selected_sim) {
            Some(i) => sims.get((i + 1) % sims.len()).map(|s| s.id),
            None => sims.first().map(|s| s.id),
        };

        let mut composer_parts: Vec<El> = Vec::new();

        if let Some(plan) = &self.pending_sms_plan {
            let safety_banner = Container::new(self.between(
                self.hstack(
                    8.,
                    vec![
                        self.dot(p.warning),
                        self.line_w("敏感操作二次确认：", CAPTION, p.text, Weight::Semibold),
                        self.line("将通过", CAPTION, p.text2),
                        self.line_w(&plan.subscription_label, CAPTION, p.text, Weight::Semibold),
                        self.line("发送至", CAPTION, p.text2),
                        self.mono(&plan.number, CAPTION, p.text),
                        self.pill_badge(format!("共 {} 段", plan.parts), p.warning, p.warning_soft),
                    ],
                ),
                self.hstack(
                    8.,
                    vec![
                        self.button_icon(
                            "sms-cancel",
                            "取消",
                            UiIcon::Close,
                            Action::CancelDirectSms,
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "sms-confirm",
                            "确认立即发送",
                            UiIcon::Check,
                            Action::ConfirmDirectSms,
                            Btn::Primary,
                        ),
                    ],
                ),
            ))
            .with_horizontal_padding(14.)
            .with_vertical_padding(8.)
            .with_background_color(p.warning_soft)
            .with_border(Border::all(1.).with_border_color(p.warning))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish();
            composer_parts.push(safety_banner);
        } else if !can_send {
            composer_parts.push(self.hstack(
                6.,
                vec![
                    self.dot(p.text3),
                    self.line(
                        if self.capabilities.is_none() {
                            "正在确认手机短信发送能力…"
                        } else {
                            "请确认手机短信权限与可用 SIM 卡，或点击右侧「在手机上编辑」"
                        },
                        MICRO,
                        p.text2,
                    ),
                ],
            ));
        }

        if (is_composing_without_history || !has_recipient) && !self.contacts.is_empty() {
            let typed_to = self.fields[F_SMS_TO].text.trim().to_lowercase();
            let matching_contacts: Vec<_> = self
                .contacts
                .iter()
                .filter(|c| {
                    typed_to.is_empty()
                        || c.name.to_lowercase().contains(&typed_to)
                        || c.number.to_lowercase().contains(&typed_to)
                })
                .take(8)
                .collect();
            if !matching_contacts.is_empty() {
                let mut contact_chips: Vec<El> = vec![self.line("联系人速选：", MICRO, p.text2)];
                for (c_idx, c) in matching_contacts.into_iter().enumerate() {
                    let active_c = norm_addr(&self.fields[F_SMS_TO].text) == norm_addr(&c.number)
                        && !c.number.trim().is_empty();
                    let label = if c.name.trim().is_empty() || c.name.trim() == c.number.trim() {
                        c.number.clone()
                    } else {
                        format!("{} ({})", c.name, c.number)
                    };
                    contact_chips.push(self.button_icon(
                        format!("sms-contact-pick-{c_idx}"),
                        label,
                        UiIcon::User,
                        Action::PhoneSmsTo(c.number.clone()),
                        if active_c {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ));
                }
                composer_parts.push(self.wrap(contact_chips));
            }
        }

        let sim_btn = self.button(
            "sms-sim-switch",
            sim_label.clone(),
            Action::SelectSim(next_sim),
            Btn::Secondary,
        );

        let mut composer_row_children: Vec<El> = vec![sim_btn];
        if is_composing_without_history || !has_recipient {
            composer_row_children.push(self.input(F_SMS_TO, "输入收件人号码…", Some(160.)));
        }
        composer_row_children.push(
            Expanded::new(
                1.,
                self.input(F_SMS_BODY, "输入短信内容，按 Enter 或点击发送…", None),
            )
            .finish(),
        );
        composer_row_children.push(self.button_icon(
            "sms-send",
            if self.pending_sms_plan.is_some() {
                "确认发送"
            } else {
                "发送"
            },
            UiIcon::Send,
            if self.pending_sms_plan.is_some() {
                Action::ConfirmDirectSms
            } else {
                Action::PrepareDirectSms
            },
            Btn::Primary,
        ));

        let composer_box = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(8.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children(composer_row_children)
                .finish(),
        )
        .with_uniform_padding(8.)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let sms_quick_templates = [
            "收到，谢谢",
            "稍后回电",
            "正在开会，稍后联系",
            "快递请放门口/驿站",
            "我马上到",
            "好的，没问题",
        ];
        let mut sms_template_chips: Vec<El> = Vec::new();
        for (idx, tpl) in sms_quick_templates.into_iter().enumerate() {
            let active_tpl = self.fields[F_SMS_BODY].text.trim() == tpl;
            sms_template_chips.push(self.im_channel_pill(
                format!("sms-tpl-chip-{idx}"),
                tpl,
                active_tpl,
                Action::Push(PushAction::FillSmsTemplate(tpl.to_string())),
            ));
        }
        if !self.fields[F_SMS_BODY].text.trim().is_empty() {
            sms_template_chips.push(self.button_icon(
                "sms-tpl-clear",
                "清空输入",
                UiIcon::Trash,
                Action::ClearField(F_SMS_BODY),
                Btn::Ghost,
            ));
        }
        let sms_templates_bar = self.between(
            self.wrap(sms_template_chips),
            self.button_icon(
                "sms-draft-phone",
                "在手机上编辑",
                UiIcon::Smartphone,
                Action::DraftSms,
                Btn::Ghost,
            ),
        );

        composer_parts.push(sms_templates_bar);
        if let Some(quote_bar) = self.render_quote_preview_bar(true) {
            composer_parts.push(quote_bar);
        }
        if let Some(status) = &self.last_sms_status {
            composer_parts.push(self.between(
                self.text(status.summary_label(), CAPTION, p.text2),
                self.button_icon(
                    "sms-check-status",
                    "查询发送状态",
                    UiIcon::Refresh,
                    Action::CheckSmsStatus(status.id.clone()),
                    Btn::Secondary,
                ),
            ));
        }
        composer_parts.push(composer_box);
        let bottom_area = Container::new(self.vstack(8., composer_parts))
            .with_horizontal_padding(S5)
            .with_padding_top(S3)
            .with_padding_bottom(S4)
            .with_background_color(p.bg)
            .with_border(Border::top(1.).with_border_color(p.border))
            .finish();

        let mut right_pane = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(subheader)
            .with_child(Expanded::new(1., timeline).finish());
        if has_recipient || is_new_sms {
            right_pane = right_pane.with_child(bottom_area);
        }
        let right_pane = right_pane.finish();

        let split = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(left_pane)
            .with_child(Expanded::new(1., right_pane).finish())
            .finish();

        let mut page_col = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.comm_mode_bar());
        if let Some(bar) = top_code_bar {
            page_col = page_col.with_child(bar);
        }
        page_col
            .with_child(Expanded::new(1., self.sub_enter(split)).finish())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::format_im_message_content;
    use stepx_companion::push::ImMessage;

    #[test]
    fn message_detail_identity_survives_history_prepend() {
        let mut thread = stepx_companion::push::ImThread {
            thread_id: "chat".into(),
            messages: vec![ImMessage {
                id: "stable-id".into(),
                text: "body".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let before = super::im_detail_key(&thread, 0);
        thread.messages.insert(
            0,
            ImMessage {
                id: "older-id".into(),
                ..Default::default()
            },
        );
        assert_eq!(before, super::im_detail_key(&thread, 1));
        assert_ne!(before, super::im_detail_key(&thread, 0));
    }

    #[test]
    fn technical_dates_are_neither_pickup_codes_nor_verification_codes() {
        for text in [
            "CLI 版本1.4.12于2026-10-28发布，Code 编辑器更新日期10-28。",
            "凭此指南配置 VS Code 2026.1028，与快递无关。",
        ] {
            assert!(super::extract_pickup_code(text).is_none());
            assert!(super::extract_verification_code(text).is_none());
            assert!(!super::is_pickup_code_text("10-28", text));
        }
        let pickup = "菜鸟驿站的包裹已到，请凭取件码4-2-1088领取。";
        assert_eq!(
            super::extract_pickup_code(pickup).as_deref(),
            Some("4-2-1088")
        );
        assert!(super::is_pickup_code_text("4-2-1088", pickup));
        assert_eq!(
            super::extract_verification_code("您的登录验证码839201，5分钟内有效。").as_deref(),
            Some("839201")
        );
    }

    #[test]
    fn format_im_message_content_renders_media_hint_for_empty_and_non_empty_text() {
        let media_only = ImMessage {
            id: "m1".into(),
            sender: "Alice".into(),
            text: String::new(),
            media_hint: "[图片]".into(),
            ..Default::default()
        };
        let (txt, recalled, media) = format_im_message_content(&media_only, false);
        assert_eq!(txt, "[图片]");
        assert_eq!(recalled, None);
        assert_eq!(media.as_deref(), Some("[图片]"));

        let same_text = ImMessage {
            id: "m2".into(),
            sender: "Alice".into(),
            text: "[语音]".into(),
            media_hint: "[语音]".into(),
            ..Default::default()
        };
        let (txt2, recalled2, media2) = format_im_message_content(&same_text, false);
        assert_eq!(txt2, "[语音]");
        assert_eq!(recalled2, None);
        assert_eq!(media2.as_deref(), Some("[语音]"));

        let caption_with_media = ImMessage {
            id: "m3".into(),
            sender: "Bob".into(),
            text: "请查收现场照片".into(),
            media_hint: "[图片]".into(),
            ..Default::default()
        };
        let (txt3, recalled3, media3) = format_im_message_content(&caption_with_media, false);
        assert_eq!(txt3, "[图片] 请查收现场照片");
        assert_eq!(recalled3, None);
        assert_eq!(media3.as_deref(), Some("[图片]"));
    }

    #[test]
    fn format_im_message_content_renders_recalled_badge_and_respects_privacy_mask() {
        let recalled_msg = ImMessage {
            id: "r1".into(),
            sender: "Alice".into(),
            text: "刚才发错的文档".into(),
            recalled: true,
            ..Default::default()
        };
        let (txt, recalled, media) = format_im_message_content(&recalled_msg, false);
        assert_eq!(txt, "[已撤回] 刚才发错的文档");
        assert_eq!(recalled, Some("[已撤回]"));
        assert_eq!(media, None);

        let recalled_empty = ImMessage {
            id: "r2".into(),
            sender: "Alice".into(),
            text: String::new(),
            recalled: true,
            ..Default::default()
        };
        let (txt_empty, recalled_empty_badge, _) =
            format_im_message_content(&recalled_empty, false);
        assert!(txt_empty.contains("已撤回"));
        assert_eq!(recalled_empty_badge, Some("[已撤回]"));

        // Privacy mask hides incoming content while preserving recalled status
        let (masked_txt, masked_recalled, masked_media) =
            format_im_message_content(&recalled_msg, true);
        assert_eq!(masked_txt, "[已撤回] [防窥模式已开启，已隐藏消息内容]");
        assert_eq!(masked_recalled, Some("[已撤回]"));
        assert_eq!(masked_media, None);
    }

    #[test]
    fn media_only_and_recalled_only_threads_remain_visible_and_searchable() {
        use super::{im_message_matches_search, im_thread_has_visible_content};
        use stepx_companion::push::ImThread;

        let empty_thread = ImThread {
            thread_id: "t0".into(),
            last_message: String::new(),
            messages: vec![ImMessage::default()],
            ..Default::default()
        };
        assert!(!im_thread_has_visible_content(&empty_thread));

        let media_only_thread = ImThread {
            thread_id: "t1".into(),
            last_message: String::new(),
            messages: vec![ImMessage {
                media_hint: "[图片]".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(im_thread_has_visible_content(&media_only_thread));
        assert!(im_message_matches_search(
            &media_only_thread.messages[0],
            "图片"
        ));

        let recalled_only_thread = ImThread {
            thread_id: "t2".into(),
            last_message: String::new(),
            messages: vec![ImMessage {
                recalled: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(im_thread_has_visible_content(&recalled_only_thread));
        assert!(im_message_matches_search(
            &recalled_only_thread.messages[0],
            "撤回"
        ));
    }

    #[test]
    fn wechat_emoji_media_voice_and_entity_classification_work() {
        use super::{
            classify_im_entity, clean_contact_initial, detect_im_media_kind,
            extract_voice_duration_secs, humanize_wechat_emojis, ImEntityKind, ImMediaKind,
        };

        assert_eq!(humanize_wechat_emojis("收到[呲牙][强]"), "收到😁👍");
        assert_eq!(
            extract_voice_duration_secs("[语音] 20\"", "[语音]"),
            Some(20)
        );
        assert_eq!(extract_voice_duration_secs("[语音]", "[语音]"), Some(8));
        assert_eq!(
            detect_im_media_kind("[动画表情] 笑", "[动画表情]"),
            Some((ImMediaKind::Sticker, "笑".into()))
        );
        assert_eq!(
            classify_im_entity("圆通速递", "快件已送达", false, &[]),
            ImEntityKind::Express
        );
        assert_eq!(
            classify_im_entity("霸王茶姬", "取餐提醒", false, &[]),
            ImEntityKind::FoodTea
        );
        assert_eq!(
            classify_im_entity("微信支付", "已支付¥54.11", false, &[]),
            ImEntityKind::Finance
        );
        assert_eq!(
            classify_im_entity("粤海水务服务", "停水通知", false, &[]),
            ImEntityKind::Service
        );
        assert_eq!(clean_contact_initial("我是照相机先生"), "照");
        assert_eq!(clean_contact_initial("窝-StepX"), "窝");
    }

    #[test]
    fn sanitize_ui_display_text_and_value_hub_signals_prevent_tofu_and_crosstalk() {
        use super::{
            detect_im_rich_modal, detect_im_service_notice, extract_thread_value_signals,
            sanitize_ui_display_text, ImEntityKind, ImRichModalKind,
        };
        use stepx_companion::push::{ImMessage, ImThread};

        assert_eq!(
            sanitize_ui_display_text("⋆演示交流小组📱"),
            "⋆演示交流小组"
        );
        assert_eq!(
            sanitize_ui_display_text("[文件传输助手队列消息 #1] 第一条队列消息"),
            "第一条队列消息"
        );

        // Verify per-message AI Value Hub extraction never attributes 霸王茶姬's 取餐提醒 to 顺丰速运+
        let mixed_thread = ImThread {
            thread_id: "com.tencent.mm::服务通知".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "顺丰速运+".into(),
            last_sender: "顺丰速运+".into(),
            last_message: "取餐提醒".into(),
            messages: vec![
                ImMessage {
                    id: "sf-1".into(),
                    sender: "顺丰速运+".into(),
                    text: "包裹派件提醒：您的快件正在派送中".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "tea-1".into(),
                    sender: "霸王茶姬".into(),
                    text: "取餐提醒：伯牙绝弦已制作完成，取餐码 A088".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let signals = extract_thread_value_signals(&mixed_thread);
        assert_eq!(signals.len(), 2);
        assert_eq!(signals[0].0, ImEntityKind::FoodTea);
        assert_eq!(signals[0].1, "霸王茶姬");
        assert!(signals[0].2.contains("取餐提醒"));
        assert_eq!(signals[1].0, ImEntityKind::Express);
        assert_eq!(signals[1].1, "顺丰速运+");
        assert!(signals[1].2.contains("包裹派件提醒"));

        // Verify service notice card & multi-modal card detection
        let srv = detect_im_service_notice(
            "顺丰速运+",
            "包裹派件提醒\n运单号：SF123456789\n状态：派送中，取件码 4-2-1088",
        )
        .expect("should detect express service notice");
        assert_eq!(srv.kind, ImEntityKind::Express);
        assert_eq!(srv.headline, "包裹派件提醒");
        assert_eq!(srv.highlight_code, Some(("4-2-1088".to_string(), true)));

        assert_eq!(
            detect_im_rich_modal("[文件] 季度架构评审.pdf", "[文件]"),
            Some((ImRichModalKind::File, "季度架构评审.pdf".into()))
        );
        assert_eq!(
            detect_im_rich_modal("[小程序] 顺丰速运+", "[小程序]"),
            Some((ImRichModalKind::MiniProgram, "顺丰速运+".into()))
        );
    }

    #[test]
    fn push_notification_source_label_and_avatar_style_cover_mainstream_apps() {
        use super::{im_avatar_style, push_notification_source_label};

        assert_eq!(
            push_notification_source_label("com.xingin.xhs", "小红书", "收到一条新私信"),
            "小红书"
        );
        assert_eq!(
            push_notification_source_label("com.taobao.taobao", "手机淘宝", "您的订单已发货"),
            "淘宝"
        );
        assert_eq!(
            push_notification_source_label("com.sankuai.meituan", "美团外卖", "骑手正在配送"),
            "美团"
        );
        assert_eq!(
            push_notification_source_label("com.eg.android.AlipayGphone", "支付宝", "付款成功"),
            "支付宝"
        );
        assert_eq!(
            push_notification_source_label("com.stepx.companion.app", "跨端剪贴板", "已同步剪贴板"),
            "跨端剪贴"
        );
        let (_, xhs_bg, _) = im_avatar_style("com.xingin.xhs", "小红书");
        let (_, tb_bg, _) = im_avatar_style("com.taobao.taobao", "淘宝");
        assert_ne!(xhs_bg, tb_bg);
    }

    #[test]
    fn chat_bubble_and_wechat_noise_dedup_guards_work() {
        use super::{compact_thread_title, Companion};
        use stepx_companion::push::{
            canonical_conversation_title_key, is_scraped_ui_noise_thread, ImMessage, ImThread,
        };

        assert_eq!(
            canonical_conversation_title_key("示例团队 • 项目讨论[月亮]"),
            canonical_conversation_title_key("示例团队 • 项目讨论[表情][表情]")
        );
        assert!(is_scraped_ui_noise_thread(&ImThread {
            title: "通过朋友验证".into(),
            last_message: "不让他看我的朋友圈和状态".into(),
            ..Default::default()
        }));
        assert!(is_scraped_ui_noise_thread(&ImThread {
            title: "语音5秒未播放".into(),
            last_message: "[语音] 4\"".into(),
            ..Default::default()
        }));
        assert!(!is_scraped_ui_noise_thread(&ImThread {
            title: "桃桃".into(),
            last_message: "没时间".into(),
            messages: vec![ImMessage {
                text: "没时间".into(),
                ..Default::default()
            }],
            ..Default::default()
        }));
        assert_eq!(compact_thread_title("桃桃", 15), "桃桃");
        let _ = Companion::chat_bubble_radius(true, false);
        let _ = Companion::chat_bubble_radius(false, true);
    }

    #[test]
    fn wechat_search_pollution_and_long_welcome_sticker_misclassification_are_blocked() {
        use super::{detect_im_media_kind, ImMediaKind};
        use stepx_companion::push::{is_scraped_ui_noise_thread, ImMessage, ImThread};

        // 1. Long welcome message containing inline `[表情]` must NOT be classified as Sticker
        let welcome_text = "[表情] 刘磊BOB 欢迎加入STEPX探索基地 如果你是： √ 一直盯着STEPX Neo智能体手机新动态，想知道有什么新功能 √ 已经上手，想找人一起折腾、一起琢磨玩法";
        assert_eq!(detect_im_media_kind(welcome_text, "[动画表情]"), None);
        assert_eq!(
            detect_im_media_kind("[动画表情]", "[动画表情]"),
            Some((ImMediaKind::Sticker, String::new()))
        );

        // 2. Polluted WeChat global search / home screen thread (`微信 (董少华)` with search results) must be rejected
        let polluted_thread = ImThread {
            thread_id: "com.tencent.mm::微信".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "微信".into(),
            last_sender: "董少华".into(),
            last_message: "昵称: 不吃牛肉。".into(),
            is_group: true,
            messages: vec![
                ImMessage {
                    id: "ui_1".into(),
                    sender: "董少华".into(),
                    text: "你有1条消息未发送".into(),
                    via: "ui_captured".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "ui_2".into(),
                    sender: "我".into(),
                    text: "转发截图".into(),
                    outgoing: true,
                    via: "ui_captured".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "ui_3".into(),
                    sender: "董少华".into(),
                    text: "登录 Linux 电脑".into(),
                    via: "ui_captured".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "ui_4".into(),
                    sender: "董少华".into(),
                    text: "联系人".into(),
                    via: "ui_captured".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "ui_5".into(),
                    sender: "董少华".into(),
                    text: "牛坏人".into(),
                    via: "ui_captured".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "ui_6".into(),
                    sender: "董少华".into(),
                    text: "昵称: 不吃牛肉。".into(),
                    via: "ui_captured".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert!(is_scraped_ui_noise_thread(&polluted_thread));
    }
}
