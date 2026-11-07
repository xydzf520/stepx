//! Phone-to-desktop IM Chat Hub & full phone push notification center client.
//!
//! Supports dual-stack transport over same-Wi-Fi or USB:
//! 1. Fast local TCP stream via `adb forward tcp:0 localabstract:stepx_push_v1`
//!    (zero JVM fork overhead, < 15ms round-trip latency).
//! 2. Seamless fallback to `content call --uri content://com.stepx.companion.app.push`
//!    with Base64-encoded JSON payloads.

use crate::adb::Adb;
use anyhow::{ensure, Context, Result};
use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const PUSH_SOCKET_NAME: &str = "localabstract:stepx_push_v1";
const PUSH_PROVIDER_URI: &str = "content://com.stepx.companion.app.push";
const LISTENER_COMPONENT: &str =
    "com.stepx.companion.app/com.stepx.companion.app.PushNotificationService";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ImChannel {
    #[default]
    All,
    WeChat,
    WeCom,
    DingTalk,
    Feishu,
    QqTim,
    GlobalIm,
    SocialDm,
    Sms,
}

impl ImChannel {
    pub const ALL: [ImChannel; 8] = [
        Self::All,
        Self::WeChat,
        Self::WeCom,
        Self::DingTalk,
        Self::Feishu,
        Self::QqTim,
        Self::GlobalIm,
        Self::Sms,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::WeChat => "微信",
            Self::WeCom => "企微",
            Self::DingTalk => "钉钉",
            Self::Feishu => "飞书",
            Self::QqTim => "QQ/TIM",
            Self::GlobalIm => "更多社交",
            Self::SocialDm => "社媒私信",
            Self::Sms => "短信",
        }
    }

    pub fn badge(self) -> &'static str {
        match self {
            Self::All => "全",
            Self::WeChat => "微",
            Self::WeCom => "企",
            Self::DingTalk => "钉",
            Self::Feishu => "飞",
            Self::QqTim => "Q",
            Self::GlobalIm => "IM",
            Self::SocialDm => "私",
            Self::Sms => "短",
        }
    }

    pub fn from_package(pkg: &str) -> Self {
        let trimmed = pkg.trim();
        if trimmed.is_empty() {
            return Self::All;
        }
        match trimmed {
            "com.tencent.mm" => Self::WeChat,
            "com.tencent.wework" => Self::WeCom,
            "com.alibaba.android.rimet" => Self::DingTalk,
            "com.ss.android.lark" | "com.ss.android.lark.kami" | "com.larksuite.suite" => {
                Self::Feishu
            }
            "com.tencent.mobileqq" | "com.tencent.tim" | "com.tencent.qqlite" => Self::QqTim,
            // Social, Community, Career & Recruitment, E-Commerce & Customer Service -> SocialDm
            "com.xingin.xhs"
            | "com.ss.android.ugc.aweme"
            | "com.ss.android.ugc.aweme.lite"
            | "com.smile.gifmaker"
            | "com.kuaishou.nebula"
            | "com.sina.weibo"
            | "com.sina.weibog3"
            | "com.weico.international"
            | "tv.danmaku.bili"
            | "com.bilibili.app.in"
            | "com.zhihu.android"
            | "com.douban.frodo"
            | "com.baidu.tieba"
            | "cn.soulapp.android"
            | "com.immomo.momo"
            | "com.p1.mobile.putong"
            | "com.mihoyo.hyperion"
            | "com.max.xiaoheihe"
            | "com.taptap"
            | "com.hupu.games"
            | "com.coolapk.market"
            | "com.instagram.android"
            | "com.instagram.barcelona"
            | "com.twitter.android"
            | "com.facebook.katana"
            | "com.reddit.frontpage"
            | "com.hpbr.bosszhipin"
            | "com.taou.maimai"
            | "com.zhaopin.social"
            | "com.job.android"
            | "com.lietou.mishu"
            | "com.lietpin.android"
            | "com.lietpin.android.LPActivity"
            | "com.linkedin.android"
            | "com.wuba"
            | "com.yupao.android"
            | "com.shixiseng.activity"
            | "com.taobao.taobao"
            | "com.tmall.wireless"
            | "com.taobao.idlefish"
            | "com.jingdong.app.mall"
            | "com.xunmeng.pinduoduo"
            | "com.shizhuang.duapp"
            | "com.achievo.vipshop"
            | "com.alibaba.wireless"
            | "com.wuba.zhuanzhuan"
            | "com.ss.android.merchant"
            | "com.xiaomi.shop"
            | "com.vmall.client" => Self::SocialDm,
            // Global IM, Delivery, Travel, Finance, Workspace, Mail & Dev -> GlobalIm
            "org.telegram.messenger"
            | "org.telegram.messenger.web"
            | "org.thunderdog.challegram"
            | "org.telegram.plus"
            | "tw.nekomimi.nekogram"
            | "com.whatsapp"
            | "com.whatsapp.w4b"
            | "com.Slack"
            | "com.slack"
            | "com.discord"
            | "com.microsoft.teams"
            | "org.thoughtcrime.securesms"
            | "com.facebook.orca"
            | "jp.naver.line.android"
            | "com.kakao.talk"
            | "com.viber.voip"
            | "com.sankuai.meituan"
            | "com.sankuai.meituan.takeoutnew"
            | "com.dianping.v1"
            | "me.ele"
            | "com.yaya.zone"
            | "com.wudaokou.hippo"
            | "cn.samsclub.app"
            | "com.pupumall.customer"
            | "com.sf.activity"
            | "com.cainiao.wireless"
            | "com.Cainiao.wireless"
            | "com.jd.express"
            | "com.fcbox.hiveconsumer"
            | "com.zto.consumer"
            | "com.yto.eyto"
            | "com.yunda.express"
            | "com.jtexpress.client"
            | "com.sdu.didi.psnger"
            | "com.autonavi.minimap"
            | "com.baidu.BaiduMap"
            | "com.jingyao.easybike"
            | "cn.caocaokeji.user"
            | "com.t3go.passenger"
            | "com.MobileTicket"
            | "com.umetrip.android.msky.app"
            | "ctrip.android.view"
            | "com.Qunar"
            | "com.taobao.trip"
            | "com.tongcheng.android"
            | "com.htinns"
            | "com.eg.android.AlipayGphone"
            | "com.unionpay"
            | "cmb.pb"
            | "com.cmbchina.ccd.pluto.cmbActivity"
            | "com.icbc"
            | "com.chinamworld.main"
            | "com.android.bankabc"
            | "com.chinamworld.bocmbci"
            | "com.bankcomm.Bankcomm"
            | "com.pingan.paces.ccms"
            | "com.citiccard.mobilebank"
            | "com.cmbc.cc.mbank"
            | "com.spdbccc.app"
            | "com.cebbank.mobile.cemb"
            | "com.cib.cibmb"
            | "com.tencent.wemeet.app"
            | "us.zoom.videomeetings"
            | "com.tencent.docs"
            | "cn.wps.moffice_eng"
            | "chuxin.shimo.shimowendang"
            | "com.yuque.mobile.android.app"
            | "notion.id"
            | "com.alicloud.databox"
            | "com.baidu.netdisk"
            | "com.quark.browser"
            | "com.tencent.androidqqmail"
            | "com.netease.mobimail"
            | "cn.cj.pe"
            | "com.alibaba.cloudmail"
            | "com.google.android.gm"
            | "com.microsoft.office.outlook"
            | "com.readdle.spark"
            | "com.github.android" => Self::GlobalIm,
            "com.android.mms"
            | "com.google.android.apps.messaging"
            | "com.samsung.android.messaging"
            | "com.android.messaging"
            | "com.miui.mms"
            | "sms"
            | "telephony" => Self::Sms,
            _ => {
                let lower = trimmed.to_ascii_lowercase();
                if lower.contains("tencent.mm") {
                    Self::WeChat
                } else if lower.contains("wework") {
                    Self::WeCom
                } else if lower.contains("rimet") || lower.contains("dingtalk") {
                    Self::DingTalk
                } else if lower.contains("lark") || lower.contains("feishu") {
                    Self::Feishu
                } else if lower.contains("mobileqq") || lower.contains("tencent.tim") {
                    Self::QqTim
                } else if lower.contains("mms")
                    || lower.contains("messaging")
                    || lower == "sms"
                    || lower.contains("telephony")
                {
                    Self::Sms
                } else if lower.contains("xhs")
                    || lower.contains("aweme")
                    || lower.contains("idlefish")
                    || lower.contains("weibo")
                    || lower.contains("bili")
                    || lower.contains("zhihu")
                    || lower.contains("taobao")
                    || lower.contains("tmall")
                    || lower.contains("jingdong")
                    || lower.contains("pinduoduo")
                    || lower.contains("bosszhipin")
                    || lower.contains("maimai")
                    || lower.contains("lietpin")
                    || lower.contains("zhaopin")
                {
                    Self::SocialDm
                } else {
                    Self::GlobalIm
                }
            }
        }
    }

    pub fn matches_package(self, pkg: &str) -> bool {
        if is_excluded_im_package(pkg) {
            return false;
        }
        if self == Self::All {
            return true;
        }
        let detected = Self::from_package(pkg);
        if detected == self {
            return true;
        }
        // When filtering via the 8-tab bar (`ImChannel::ALL`), include `SocialDm` under `GlobalIm`
        // unless the caller specifically selected `SocialDm`.
        self == Self::GlobalIm && detected == Self::SocialDm
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PushCategory {
    #[default]
    All,
    Im,
    Social,
    Finance,
    DeliveryTravel,
    SystemSecurity,
    Other,
}

impl PushCategory {
    pub const ALL: [PushCategory; 7] = [
        Self::All,
        Self::Im,
        Self::Social,
        Self::Finance,
        Self::DeliveryTravel,
        Self::SystemSecurity,
        Self::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Im => "聊天",
            Self::Social => "社交",
            Self::Finance => "账单",
            Self::DeliveryTravel => "物流",
            Self::SystemSecurity => "系统",
            Self::Other => "其他",
        }
    }

    pub fn from_str_code(code: &str) -> Self {
        match code.trim().to_ascii_lowercase().as_str() {
            "" | "all" => Self::All,
            "im" | "chat" | "message" | "msg" => Self::Im,
            "social" => Self::Social,
            "finance" | "bill" | "payment" => Self::Finance,
            "delivery_travel" | "delivery" | "travel" | "logistics" => Self::DeliveryTravel,
            "system" | "system_security" | "security" | "otp" => Self::SystemSecurity,
            _ => Self::Other,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Im => "im",
            Self::Social => "social",
            Self::Finance => "finance",
            Self::DeliveryTravel => "delivery_travel",
            Self::SystemSecurity => "system",
            Self::Other => "other",
        }
    }

    pub fn matches(self, cat: &str) -> bool {
        self == Self::All || Self::from_str_code(cat) == self
    }

    pub fn from_notification(pkg: &str, title: &str, text: &str, can_reply: bool) -> Self {
        let p = pkg.trim();
        let t = title.trim();
        let body = text.trim();

        // 1. Core IM & Workspace / Mail / Dev apps -> Im (except WeChat Pay receipt notice)
        if matches!(
            p,
            "com.tencent.mm"
                | "com.tencent.wework"
                | "com.alibaba.android.rimet"
                | "com.ss.android.lark"
                | "com.ss.android.lark.kami"
                | "com.larksuite.suite"
                | "com.tencent.mobileqq"
                | "com.tencent.tim"
                | "com.tencent.qqlite"
                | "org.telegram.messenger"
                | "org.telegram.messenger.web"
                | "org.thunderdog.challegram"
                | "org.telegram.plus"
                | "tw.nekomimi.nekogram"
                | "com.whatsapp"
                | "com.whatsapp.w4b"
                | "com.discord"
                | "com.Slack"
                | "com.slack"
                | "org.thoughtcrime.securesms"
                | "jp.naver.line.android"
                | "com.facebook.orca"
                | "com.microsoft.teams"
                | "com.kakao.talk"
                | "com.viber.voip"
                | "com.tencent.wemeet.app"
                | "us.zoom.videomeetings"
                | "com.tencent.docs"
                | "cn.wps.moffice_eng"
                | "chuxin.shimo.shimowendang"
                | "com.yuque.mobile.android.app"
                | "notion.id"
                | "com.alicloud.databox"
                | "com.baidu.netdisk"
                | "com.quark.browser"
                | "com.tencent.androidqqmail"
                | "com.netease.mobimail"
                | "cn.cj.pe"
                | "com.alibaba.cloudmail"
                | "com.google.android.gm"
                | "com.microsoft.office.outlook"
                | "com.readdle.spark"
                | "com.github.android"
        ) {
            if p == "com.tencent.mm" && t.contains("微信支付") {
                return Self::Finance;
            }
            return Self::Im;
        }

        // 2. Finance, Payment & Banking apps -> Finance
        if matches!(
            p,
            "com.eg.android.AlipayGphone"
                | "com.unionpay"
                | "cmb.pb"
                | "com.cmbchina.ccd.pluto.cmbActivity"
                | "com.icbc"
                | "com.chinamworld.main"
                | "com.android.bankabc"
                | "com.chinamworld.bocmbci"
                | "com.bankcomm.Bankcomm"
                | "com.pingan.paces.ccms"
                | "com.citiccard.mobilebank"
                | "com.cmbc.cc.mbank"
                | "com.spdbccc.app"
                | "com.cebbank.mobile.cemb"
                | "com.cib.cibmb"
        ) {
            return Self::Finance;
        }

        // 3. Food Delivery, Grocery, Express & Travel apps -> DeliveryTravel
        if matches!(
            p,
            "com.sankuai.meituan"
                | "com.sankuai.meituan.takeoutnew"
                | "com.dianping.v1"
                | "me.ele"
                | "com.yaya.zone"
                | "com.wudaokou.hippo"
                | "cn.samsclub.app"
                | "com.pupumall.customer"
                | "com.sf.activity"
                | "com.cainiao.wireless"
                | "com.Cainiao.wireless"
                | "com.jd.express"
                | "com.fcbox.hiveconsumer"
                | "com.zto.consumer"
                | "com.yto.eyto"
                | "com.yunda.express"
                | "com.jtexpress.client"
                | "com.sdu.didi.psnger"
                | "com.autonavi.minimap"
                | "com.baidu.BaiduMap"
                | "com.jingyao.easybike"
                | "cn.caocaokeji.user"
                | "com.t3go.passenger"
                | "com.MobileTicket"
                | "com.umetrip.android.msky.app"
                | "ctrip.android.view"
                | "com.Qunar"
                | "com.taobao.trip"
                | "com.tongcheng.android"
                | "com.htinns"
        ) {
            return Self::DeliveryTravel;
        }

        // 4. Social/Community, Career & Recruitment, E-Commerce & Customer Service -> Social (or DeliveryTravel for shipping updates, or Im if replyable)
        if is_mainstream_message_app(p) || is_career_or_recruitment_signal(p, "", t, body) {
            if [
                "已发货",
                "派送中",
                "派件",
                "已签收",
                "待取件",
                "取件码",
                "快件",
                "包裹已到",
            ]
            .iter()
            .any(|kw| t.contains(kw) || body.contains(kw))
            {
                return Self::DeliveryTravel;
            }
            if can_reply || is_career_or_recruitment_signal(p, "", t, body) {
                return Self::Im;
            }
            return Self::Social;
        }

        // 5. Fallback keyword checks for non-mainstream packages
        if extract_otp_code(&format!("{t} {body}")).is_some()
            || ["验证码", "安全验证", "动态码", "登录提醒", "设备登录"]
                .iter()
                .any(|kw| t.contains(kw) || body.contains(kw))
        {
            return Self::SystemSecurity;
        }
        if [
            "付款成功",
            "支付成功",
            "扣款",
            "到账",
            "账单",
            "还款",
            "信用卡",
            "余额变动",
        ]
        .iter()
        .any(|kw| t.contains(kw) || body.contains(kw))
        {
            return Self::Finance;
        }
        if [
            "外卖",
            "骑手",
            "已送达",
            "快件",
            "包裹",
            "派送",
            "已签收",
            "取件码",
            "驿站",
            "丰巢",
            "菜鸟",
            "顺丰",
            "航班",
            "登机",
            "检票",
            "高铁",
        ]
        .iter()
        .any(|kw| t.contains(kw) || body.contains(kw))
        {
            return Self::DeliveryTravel;
        }
        if can_reply {
            return Self::Im;
        }
        if p == "android"
            || p.starts_with("com.android.")
            || p.starts_with("com.google.android.")
            || p.starts_with("com.stepos.")
        {
            return Self::SystemSecurity;
        }
        Self::Other
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReplyMode {
    #[default]
    Auto,
    RemoteInputOnly,
    UiBridgeFallback,
}

impl ReplyMode {
    pub const ALL: [ReplyMode; 3] = [Self::Auto, Self::RemoteInputOnly, Self::UiBridgeFallback];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "后台发送（不切换手机 App）",
            Self::RemoteInputOnly => "仅通知栏静默秒回",
            Self::UiBridgeFallback => "无障碍看屏发送",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::RemoteInputOnly => "remote_only",
            Self::UiBridgeFallback => "ui_bridge",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ImMessage {
    pub id: String,
    pub sender: String,
    pub text: String,
    #[serde(alias = "timestampMs", alias = "timestamp")]
    pub timestamp_ms: i64,
    #[serde(alias = "observedAtMs")]
    pub observed_at_ms: i64,
    #[serde(alias = "timeKnown")]
    pub time_known: Option<bool>,
    pub recalled: bool,
    pub outgoing: bool,
    #[serde(alias = "deliveryStatus", alias = "status")]
    pub delivery_status: String,
    pub via: String,
    #[serde(alias = "mediaHint")]
    pub media_hint: String,
    #[serde(alias = "clientMsgId")]
    pub client_msg_id: String,
    #[serde(alias = "mediaB64", default)]
    pub media_b64: String,
    #[serde(skip)]
    pub media_path: Option<PathBuf>,
}

impl ImMessage {
    /// Legacy UI captures used the time of capture as the message timestamp.
    /// Only explicit provenance can make such a timestamp safe for period reports.
    pub fn occurred_at_ms(&self) -> Option<i64> {
        let known = self.time_known.unwrap_or(!self.via.starts_with("ui_"));
        (known && self.timestamp_ms > 0).then_some(self.timestamp_ms)
    }

    pub fn same_delivery(&self, other: &Self) -> bool {
        (!self.id.is_empty() && self.id == other.id)
            || (!self.client_msg_id.is_empty()
                && (self.client_msg_id == other.client_msg_id || self.client_msg_id == other.id))
            || (!other.client_msg_id.is_empty() && other.client_msg_id == self.id)
    }
}

pub const OTP_TTL_MS: i64 = 15 * 60 * 1000;

pub fn is_native_sms_package_or_thread(package_name: &str, thread_id: &str) -> bool {
    let pkg = package_name.trim().to_ascii_lowercase();
    let tid = thread_id.trim().to_ascii_lowercase();
    matches!(
        pkg.as_str(),
        "com.android.mms"
            | "com.google.android.apps.messaging"
            | "com.samsung.android.messaging"
            | "com.android.messaging"
            | "com.miui.mms"
    ) || tid.starts_with("com.android.mms::")
        || tid.starts_with("com.google.android.apps.messaging::")
        || tid.starts_with("com.samsung.android.messaging::")
        || tid.starts_with("com.android.messaging::")
        || tid.starts_with("com.miui.mms::")
}

/// Preserve local sends across snapshots, replacing them by correlated phone records.
/// When the phone reports `unchanged == true` for a delta poll (`since_revision > 0`),
/// preserve the previous threads, notifications, and OTP banners while keeping updated device state.
pub fn merge_local_replies(next: &mut PushSnapshot, previous: &PushSnapshot) {
    // A slower message snapshot must not roll back the independent call monitor.
    if previous.live_call.observed_ms > next.live_call.observed_ms {
        next.live_call = previous.live_call.clone();
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let was_unchanged = next.unchanged && next.threads.is_empty();
    if next.unchanged {
        if next.revision == 0 {
            next.revision = previous.revision;
        }
        if next.threads.is_empty() {
            next.threads = previous.threads.clone();
        }
        if next.notifications.is_empty() {
            next.notifications = previous.notifications.clone();
        }
        if next.otp_banners.is_empty() {
            next.otp_banners = previous.otp_banners.clone();
        }
        next.unchanged = false;
    }
    next.threads
        .retain(|t| !is_native_sms_package_or_thread(&t.package_name, &t.thread_id));
    next.otp_banners.retain(|b| {
        if b.is_expired(now_ms) {
            return false;
        }
        if !b.notification_key.is_empty() {
            if let Some(n) = next
                .notifications
                .iter()
                .find(|n| n.key == b.notification_key)
            {
                if !n.active || n.dismissed || n.read {
                    return false;
                }
            }
        }
        true
    });
    if was_unchanged {
        return;
    }
    for old in &previous.threads {
        if is_native_sms_package_or_thread(&old.package_name, &old.thread_id) {
            continue;
        }
        if let Some(thread) = next
            .threads
            .iter_mut()
            .find(|t| threads_belong_to_same_conversation(t, old))
        {
            preserve_thread_identity_and_senders(thread, old);
            for message in &old.messages {
                if !message.outgoing || message.delivery_status == "failed" {
                    continue;
                }
                let is_local_or_recorded = !message.client_msg_id.is_empty()
                    || message.id.starts_with("stepx-")
                    || message.id.starts_with("out_")
                    || message.via.starts_with("ui_")
                    || message.via.starts_with("accessibility");
                if !is_local_or_recorded {
                    continue;
                }
                // A notification refresh is not a deletion receipt. Keep confirmed
                // local replies and captured outgoing messages regardless of age.
                // Explicit clears/deletes are applied through preference tombstones.
                let already_present = thread.messages.iter().any(|m| {
                    m.same_delivery(message)
                        || (m.outgoing
                            && messages_match_content(&m.text, &message.text)
                            && (m.timestamp_ms <= 0
                                || message.timestamp_ms <= 0
                                || (m.timestamp_ms - message.timestamp_ms).abs() <= 180_000))
                });
                if !already_present {
                    thread.messages.push(message.clone());
                }
            }
            deduplicate_thread_messages(&mut thread.messages, &thread.title);
            recompute_thread_summary(thread);
            continue;
        }

        // Only carry over a thread missing from `next` if it has an actively in-flight local reply
        // (`delivery_status == "sending"`) within the last 15 seconds. Never resurrect deleted threads!
        let has_in_flight_send = old.messages.iter().any(|m| {
            m.outgoing
                && !m.client_msg_id.is_empty()
                && m.id == m.client_msg_id
                && m.delivery_status == "sending"
                && (m.timestamp_ms <= 0
                    || (now_ms > 0 && (now_ms - m.timestamp_ms).abs() <= 15_000))
        });
        if !has_in_flight_send {
            continue;
        }
        let mut carried = old.clone();
        carried
            .messages
            .retain(|m| !(m.outgoing && m.delivery_status == "failed"));
        deduplicate_thread_messages(&mut carried.messages, &carried.title);
        recompute_thread_summary(&mut carried);
        if !carried.messages.is_empty() || !carried.last_message.is_empty() {
            next.threads.push(carried);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ImThread {
    #[serde(alias = "threadId", alias = "id")]
    pub thread_id: String,
    #[serde(alias = "packageName")]
    pub package_name: String,
    #[serde(alias = "appName")]
    pub app_name: String,
    pub title: String,
    #[serde(alias = "isGroup", alias = "group")]
    pub is_group: bool,
    #[serde(alias = "unreadCount")]
    pub unread_count: usize,
    #[serde(alias = "lastSender")]
    pub last_sender: String,
    #[serde(alias = "lastMessage", alias = "lastText")]
    pub last_message: String,
    #[serde(alias = "lastTimestampMs", alias = "updatedAtMs")]
    pub last_timestamp_ms: i64,
    #[serde(alias = "canRemoteReply", alias = "canReply")]
    pub can_remote_reply: bool,
    #[serde(alias = "canUiReply")]
    pub can_ui_reply: bool,
    #[serde(alias = "hasContentIntent")]
    pub has_content_intent: bool,
    #[serde(alias = "notificationKey", alias = "lastKey")]
    pub notification_key: String,
    pub messages: Vec<ImMessage>,
    #[serde(default)]
    pub avatar_b64: String,
    #[serde(default)]
    pub sender_avatars: HashMap<String, String>,
    #[serde(skip)]
    pub avatar_path: Option<PathBuf>,
    #[serde(skip)]
    pub sender_avatar_paths: HashMap<String, PathBuf>,
}

impl ImThread {
    /// Notifications are previews, never evidence of complete bidirectional history.
    pub fn notification_history_only(&self) -> bool {
        !self.messages.is_empty()
            && self.messages.iter().all(|m| {
                !m.outgoing
                    && matches!(
                        m.via.as_str(),
                        "notification" | "notification_fallback" | ""
                    )
            })
    }

    pub fn channel(&self) -> ImChannel {
        ImChannel::from_package(&self.package_name)
    }

    pub fn can_reply(&self) -> bool {
        self.can_remote_reply || self.can_ui_reply
    }
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PushNotification {
    pub key: String,
    #[serde(alias = "seqId", alias = "id")]
    pub seq_id: u64,
    #[serde(alias = "packageName")]
    pub package_name: String,
    #[serde(alias = "appName")]
    pub app_name: String,
    pub category: String,
    pub title: String,
    pub sender: String,
    pub text: String,
    #[serde(alias = "subText")]
    pub sub_text: String,
    #[serde(alias = "postTimeMs", alias = "timestampMs")]
    pub post_time_ms: i64,
    #[serde(alias = "isOngoing", alias = "ongoing")]
    pub is_ongoing: bool,
    #[serde(default, alias = "clearable", alias = "isClearable")]
    pub is_clearable: bool,
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default, alias = "removedTimeMs")]
    pub removed_time_ms: i64,
    #[serde(alias = "isGroup", alias = "group")]
    pub is_group: bool,
    pub dismissed: bool,
    pub read: bool,
    #[serde(alias = "canReply")]
    pub can_reply: bool,
    #[serde(alias = "threadId")]
    pub thread_id: String,
    #[serde(alias = "otpCode")]
    pub otp_code: Option<String>,
    #[serde(alias = "actionLabels", alias = "actions")]
    pub action_labels: Vec<String>,
    #[serde(alias = "urlHint")]
    pub url_hint: Option<String>,
    #[serde(default)]
    pub avatar_b64: String,
    #[serde(default)]
    pub sender_avatars: HashMap<String, String>,
    #[serde(default, alias = "progressCurrent")]
    pub progress_current: i32,
    #[serde(default, alias = "progressMax")]
    pub progress_max: i32,
    #[serde(default, alias = "progressIndeterminate")]
    pub progress_indeterminate: bool,
}

impl Default for PushNotification {
    fn default() -> Self {
        Self {
            key: String::new(),
            seq_id: 0,
            package_name: String::new(),
            app_name: String::new(),
            category: String::new(),
            title: String::new(),
            sender: String::new(),
            text: String::new(),
            sub_text: String::new(),
            post_time_ms: 0,
            is_ongoing: false,
            is_clearable: false,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: false,
            thread_id: String::new(),
            otp_code: None,
            action_labels: Vec::new(),
            url_hint: None,
            avatar_b64: String::new(),
            sender_avatars: HashMap::new(),
            progress_current: 0,
            progress_max: 0,
            progress_indeterminate: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct RawOtpBanner {
    code: String,
    #[serde(alias = "sourceApp", alias = "appName", alias = "app_name")]
    source_app: String,
    title: String,
    #[serde(alias = "rawText", alias = "text")]
    raw_text: String,
    #[serde(alias = "timestampMs", alias = "postTimeMs", alias = "post_time_ms")]
    timestamp_ms: i64,
    #[serde(alias = "expiresAtMs", alias = "expires_at_ms")]
    expires_at_ms: i64,
    #[serde(alias = "notificationKey", alias = "key")]
    notification_key: String,
}

impl From<RawOtpBanner> for OtpBanner {
    fn from(raw: RawOtpBanner) -> Self {
        let mut timestamp_ms = raw.timestamp_ms;
        if raw.expires_at_ms > 0 {
            let implied_start = (raw.expires_at_ms - OTP_TTL_MS).max(1);
            if timestamp_ms <= 0 || implied_start < timestamp_ms {
                timestamp_ms = implied_start;
            }
        }
        Self {
            code: raw.code,
            source_app: raw.source_app,
            title: raw.title,
            raw_text: raw.raw_text,
            timestamp_ms,
            notification_key: raw.notification_key,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(from = "RawOtpBanner")]
pub struct OtpBanner {
    pub code: String,
    pub source_app: String,
    pub title: String,
    pub raw_text: String,
    pub timestamp_ms: i64,
    pub notification_key: String,
}

impl OtpBanner {
    pub fn is_expired(&self, now_ms: i64) -> bool {
        self.timestamp_ms > 0
            && now_ms > self.timestamp_ms
            && (now_ms - self.timestamp_ms) > OTP_TTL_MS
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PushCoverage {
    #[serde(alias = "observedSinceMs")]
    pub observed_since_ms: i64,
    #[serde(alias = "lastObservedMs")]
    pub last_observed_ms: i64,
    #[serde(alias = "lastGapMs")]
    pub last_gap_ms: i64,
    #[serde(alias = "historyTruncated")]
    pub history_truncated: bool,
    #[serde(alias = "persistenceError")]
    pub persistence_error: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PushSnapshot {
    #[serde(alias = "batteryState")]
    pub battery_state: Option<crate::battery::BatteryState>,
    pub live_call: crate::phone::LiveCall,
    pub revision: u64,
    pub coverage: PushCoverage,
    pub unchanged: bool,
    #[serde(alias = "listenerConnected")]
    pub listener_connected: bool,
    #[serde(alias = "accessibilityConnected")]
    pub accessibility_connected: bool,
    #[serde(alias = "wifiAdbEnabled")]
    pub wifi_adb_enabled: bool,
    #[serde(alias = "screenInteractive")]
    pub screen_interactive: bool,
    #[serde(alias = "keyguardLocked")]
    pub keyguard_locked: bool,
    #[serde(alias = "humanTouchRecent")]
    pub human_touch_recent: bool,
    #[serde(alias = "transportMode")]
    pub transport_mode: String,
    pub threads: Vec<ImThread>,
    pub notifications: Vec<PushNotification>,
    #[serde(alias = "otpBanners")]
    pub otp_banners: Vec<OtpBanner>,
}

pub fn fetch_live_call(adb: &Adb, serial: &str) -> Result<crate::phone::LiveCall> {
    serde_json::from_value(call_push_provider(adb, serial, json!({"op":"call_state"}))?)
        .context("手机未返回可用的通话状态")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ReplyReceipt {
    pub ok: bool,
    #[serde(alias = "threadId")]
    pub thread_id: String,
    #[serde(alias = "clientMsgId")]
    pub client_msg_id: String,
    pub via: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default, alias = "deliveryStatus")]
    pub delivery_status: String,
    #[serde(alias = "error")]
    pub message: String,
    #[serde(alias = "timestampMs")]
    pub timestamp_ms: i64,
}

const SOCKET_MISS_TTL: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug)]
struct ForwardCacheEntry {
    port: Option<u16>,
    updated_at: Instant,
}

fn forward_cache() -> &'static Mutex<HashMap<String, ForwardCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, ForwardCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn invalidate_cached_port(serial: &str) {
    if let Ok(mut guard) = forward_cache().lock() {
        guard.remove(serial);
    }
}

fn mark_socket_unavailable(serial: &str) {
    if let Ok(mut guard) = forward_cache().lock() {
        guard.insert(
            serial.to_string(),
            ForwardCacheEntry {
                port: None,
                updated_at: Instant::now(),
            },
        );
    }
}

fn is_socket_recently_unavailable(serial: &str, now: Instant) -> bool {
    if let Ok(guard) = forward_cache().lock() {
        if let Some(entry) = guard.get(serial) {
            if entry.port.is_none()
                && now.saturating_duration_since(entry.updated_at) < SOCKET_MISS_TTL
            {
                return true;
            }
        }
    }
    false
}

fn obtain_forwarded_port(adb: &Adb, serial: &str, force_refresh: bool) -> Result<(u16, bool)> {
    if !force_refresh {
        if let Ok(guard) = forward_cache().lock() {
            if let Some(entry) = guard.get(serial) {
                if let Some(port) = entry.port {
                    // Re-establish forward if the laptop likely slept or the port is older than 5 minutes.
                    if entry.updated_at.elapsed() <= Duration::from_secs(300) {
                        return Ok((port, true));
                    }
                }
            }
        }
    }

    let is_wireless = crate::adb::is_wireless_transport(serial);
    let out = adb.run_timeout(
        &["-s", serial, "forward", "tcp:0", PUSH_SOCKET_NAME],
        Duration::from_millis(if is_wireless { 4000 } else { 2500 }),
    )?;
    let port: u16 = out
        .lines()
        .find_map(|line| line.trim().parse::<u16>().ok())
        .context("无法解析 adb forward 分配的本地端口")?;

    if let Ok(mut guard) = forward_cache().lock() {
        guard.insert(
            serial.to_string(),
            ForwardCacheEntry {
                port: Some(port),
                updated_at: Instant::now(),
            },
        );
    }
    Ok((port, false))
}

fn socket_read_timeout(request: &Value, serial: &str) -> Duration {
    let is_wireless = crate::adb::is_wireless_transport(serial);
    match request["op"].as_str() {
        Some("reply_im") => Duration::from_millis(25000),
        Some("sync_window" | "sync_im_list" | "open_push") => {
            Duration::from_millis(if is_wireless { 12000 } else { 10000 })
        }
        _ => Duration::from_millis(if is_wireless { 4000 } else { 1800 }),
    }
}

#[derive(Debug)]
struct UncertainPushWrite;
impl std::fmt::Display for UncertainPushWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "手机可能已执行操作，但响应未核实；为避免重复发送，请先查看手机"
        )
    }
}
impl std::error::Error for UncertainPushWrite {}

fn uncertain_write_error(error: anyhow::Error, request: &Value) -> anyhow::Error {
    if replay_safe(request) {
        error
    } else {
        error.context(UncertainPushWrite)
    }
}

fn try_call_local_socket(adb: &Adb, serial: &str, request: &Value) -> Result<Value> {
    if is_socket_recently_unavailable(serial, Instant::now()) {
        anyhow::bail!("本地推送流连接不可用（冷却期内直接回退）");
    }
    let mut payload = serde_json::to_vec(request)?;
    ensure!(payload.len() <= 32768, "推送请求过长");
    payload.push(b'\n');
    let read_timeout = socket_read_timeout(request, serial);

    for attempt in 0..2 {
        let force_refresh = attempt > 0;
        let (port, from_cache) = match obtain_forwarded_port(adb, serial, force_refresh) {
            Ok(res) => res,
            Err(e) => {
                mark_socket_unavailable(serial);
                return Err(e);
            }
        };

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut write_started = false;
        let io_result = (|| -> Result<Value> {
            let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
                .context("连接本地推送流端口超时")?;
            let _ = stream.set_nodelay(true);
            stream.set_write_timeout(Some(Duration::from_millis(1500)))?;
            stream.set_read_timeout(Some(read_timeout))?;
            write_started = true; // Even a partial write may dispatch a mutation.
            stream.write_all(&payload)?;
            stream.flush()?;

            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            let n = reader.read_line(&mut line)?;
            ensure!(n > 0, "本地推送流已断开 (EOF)");
            let mut value: Value =
                serde_json::from_str(line.trim()).context("解析推送流 JSON 失败")?;
            if let Some(obj) = value.as_object_mut() {
                obj.entry("transport_mode".to_string())
                    .or_insert_with(|| Value::String("local_socket".to_string()));
            }
            Ok(value)
        })();

        match io_result {
            Ok(val) => {
                if let Ok(mut guard) = forward_cache().lock() {
                    if let Some(entry) = guard.get_mut(serial) {
                        entry.port = Some(port);
                        entry.updated_at = Instant::now();
                    }
                }
                return Ok(val);
            }
            Err(err) => {
                if write_started && !replay_safe(request) {
                    invalidate_cached_port(serial);
                    return Err(err.context(UncertainPushWrite));
                }
                if !from_cache || attempt == 1 {
                    mark_socket_unavailable(serial);
                    return Err(err);
                }
                invalidate_cached_port(serial);
            }
        }
    }
    anyhow::bail!("本地推送流连接不可用")
}

/// Dual-stack RPC transport to the phone's `PushNotificationService` / `PushControlProvider`.
///
/// 1. First attempts fast local TCP stream via cached `adb forward tcp:0 localabstract:stepx_push_v1`
///    (automatic port cache per serial, auto-reconnect on broken pipe).
/// 2. Falls back seamlessly to `content call --uri content://com.stepx.companion.app.push --method request --arg <base64>`
///    only when the local socket transport fails.
fn replay_safe(request: &Value) -> bool {
    matches!(
        request["op"].as_str(),
        Some(
            "snapshot"
                | "call_state"
                | "status"
                | "ping"
                | "thread_detail"
                | "sync_window"
                | "sync_im_list"
                | "delete_thread"
                | "mark_thread_read"
                | "clear_thread_messages"
                | "delete_message"
        )
    )
}

fn socket_supported(request: &Value) -> bool {
    replay_safe(request)
        || matches!(
            request["op"].as_str(),
            Some(
                "reply_im"
                    | "dismiss_push"
                    | "dismiss_all_push"
                    | "open_push"
                    | "post_live_capsule"
                    | "dismiss_live_capsule"
                    | "clear_live_capsules"
            )
        )
}

fn request_is_read_only(request: &Value) -> bool {
    matches!(request["op"].as_str(), Some("snapshot" | "call_state" | "status" | "ping" | "thread_detail" | "phone_digest"))
        && request["allow_ui_fallback"] != true
}

#[derive(Clone, Default)]
struct MessageReadHealth { succeeded: Option<Instant>, failed: Option<Instant> }
fn message_read_health() -> &'static std::sync::Mutex<std::collections::HashMap<String, MessageReadHealth>> {
    static HEALTH: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, MessageReadHealth>>> = std::sync::OnceLock::new();
    HEALTH.get_or_init(Default::default)
}
fn observe_message_read(serial: &str, started: Instant, success: bool) {
    let mut map = message_read_health().lock().unwrap_or_else(|error| error.into_inner());
    let health = map.entry(serial.into()).or_default();
    if success { health.succeeded = Some(Instant::now()); health.failed = None; }
    else if health.succeeded.is_none_or(|at| at < started) { health.failed = Some(Instant::now()); }
}
pub fn message_data_stalled(device: &crate::device::Device) -> bool {
    let map = message_read_health().lock().unwrap_or_else(|error| error.into_inner());
    let records: Vec<_> = device.all_known_transports().iter().chain(std::iter::once(&device.serial)).filter_map(|target| map.get(target)).collect();
    let success = records.iter().filter_map(|health| health.succeeded).max();
    records.iter().filter_map(|health| health.failed).max().is_some_and(|failed| success.is_none_or(|at| at < failed))
}

pub fn call_push_provider(adb: &Adb, serial: &str, request: Value) -> Result<Value> {
    let started = Instant::now();
    let message_read = request["op"] == "snapshot";
    let read_only = request_is_read_only(&request);
    let _mutation = if !read_only { Some(crate::device::protect_transport_mutation(serial)?) } else { None };
    let result = call_push_provider_once(adb, serial, &request);
    if message_read && (result.is_ok() || result.as_ref().is_err_and(crate::adb::is_data_transport_failure)) {
        observe_message_read(serial, started, result.is_ok());
    }
    if !read_only || !crate::adb::is_wireless_transport(serial)
        || !result.as_ref().is_err_and(crate::adb::is_data_transport_failure) { return result; }
    // A listed device and a cached forward do not prove that data still flows.
    // Only verified reads may recover and repeat; sends and unknown operations
    // return their original uncertain receipt without resetting/replaying them.
    crate::device::invalidate_verified_transport(serial);
    invalidate_cached_port(serial);
    if let Some(recovered) = crate::device::recover_transport(adb, serial, Duration::from_secs(6)) {
        invalidate_cached_port(&recovered);
        let result = call_push_provider_once(adb, &recovered, &request);
        if message_read && result.is_ok() { observe_message_read(serial, started, true); observe_message_read(&recovered, started, true); }
        return result;
    }
    result
}

fn call_push_provider_once(adb: &Adb, serial: &str, request: &Value) -> Result<Value> {
    let active_serial = crate::device::resolve_online_transport(adb, serial, "", "", &[])
        .unwrap_or_else(|_| serial.to_string());
    let serial = active_serial.as_str();
    if socket_supported(&request) {
        let socket_result = try_call_local_socket(adb, serial, &request);
        if let Err(error) = &socket_result {
            if error.downcast_ref::<UncertainPushWrite>().is_some() {
                return socket_result;
            }
        }
        if let Ok(response) = socket_result {
            ensure!(
                response.get("ok").and_then(Value::as_bool).unwrap_or(true),
                "{}",
                response["error"].as_str().unwrap_or("手机推送服务返回错误")
            );
            return Ok(response);
        }
    }

    let encoded = STANDARD.encode(serde_json::to_vec(request)?);
    ensure!(encoded.len() <= 32768, "推送请求过长");
    let call_timeout = socket_read_timeout(&request, serial).max(Duration::from_secs(8));
    let result = adb
        .run_timeout(
            &[
                "-s",
                serial,
                "shell",
                "content",
                "call",
                "--user",
                "0",
                "--uri",
                PUSH_PROVIDER_URI,
                "--method",
                "request",
                "--arg",
                &encoded,
            ],
            call_timeout,
        )
        .map_err(|error| uncertain_write_error(error, &request))?;
    let parse_response = || -> Result<Value> {
        let raw = result
            .split("result=")
            .nth(1)
            .context("手机没有返回推送服务响应，请安装或更新新版 StepX伴侣")?
            .split(|c: char| !c.is_ascii_alphanumeric() && !"+/=".contains(c))
            .next()
            .unwrap_or("");
        let value: Value =
            serde_json::from_slice(&STANDARD.decode(raw).context("推送响应 Base64 解码失败")?)
                .context("推送响应 JSON 解析失败")?;
        Ok(value)
    };
    let mut value = parse_response().map_err(|error| uncertain_write_error(error, &request))?;
    ensure!(
        value.get("ok").and_then(Value::as_bool).unwrap_or(true),
        "{}",
        value["error"].as_str().unwrap_or("手机推送操作未完成")
    );
    if let Some(obj) = value.as_object_mut() {
        obj.entry("transport_mode".to_string())
            .or_insert_with(|| Value::String("content_provider".to_string()));
    }
    Ok(value)
}

pub fn ensure_apex_bridge_ready(adb: &Adb, serial: &str) {
    let _ = adb.script(
        serial,
        "settings put global hidden_api_policy 1 >/dev/null 2>&1 || true; \
         APK_PATH=$(pm path com.stepx.companion.app 2>/dev/null | head -n 1 | sed 's/^package://'); \
         if [ -n \"$APK_PATH\" ] && { [ ! -r /data/local/tmp/stepx-companion-bridge.apk ] || [ \"$APK_PATH\" -nt /data/local/tmp/stepx-companion-bridge.apk ]; }; then \
           rm -f /data/local/tmp/stepx-companion-bridge.apk && cp \"$APK_PATH\" /data/local/tmp/stepx-companion-bridge.apk && chmod 0444 /data/local/tmp/stepx-companion-bridge.apk; \
         fi",
    );
}

/// Ensures `PushNotificationService` is granted notification listener access on the phone.
pub fn ensure_notification_listener(adb: &Adb, serial: &str) -> Result<bool> {
    ensure_apex_bridge_ready(adb, serial);
    let mut enabled = adb
        .script(
            serial,
            &format!(
                "cmd notification allow_listener {LISTENER_COMPONENT} >/dev/null 2>&1 || true; settings --user 0 get secure enabled_notification_listeners"
            ),
        )
        .unwrap_or_default();
    let mut listeners: Vec<&str> = enabled
        .trim()
        .split(':')
        .filter(|s| !s.is_empty() && *s != "null")
        .collect();
    if !listeners.contains(&LISTENER_COMPONENT) {
        listeners.push(LISTENER_COMPONENT);
        let joined = listeners.join(":");
        if adb
            .shell(
                serial,
                &[
                    "settings",
                    "--user",
                    "0",
                    "put",
                    "secure",
                    "enabled_notification_listeners",
                    &joined,
                ],
            )
            .is_ok()
        {
            enabled = joined;
        }
    }

    if let Ok(status) = call_push_provider(adb, serial, json!({ "op": "status" })) {
        if let Some(connected) = status
            .get("listener_connected")
            .or_else(|| status.get("listenerConnected"))
            .and_then(Value::as_bool)
        {
            return Ok(connected);
        }
        return Ok(true);
    }

    Ok(enabled.contains(LISTENER_COMPONENT))
}

pub fn mainstream_message_app_name(package: &str) -> Option<&'static str> {
    match package.trim() {
        // 1. IM & Social DM
        "com.tencent.mm" => Some("微信"),
        "com.tencent.wework" => Some("企业微信"),
        "com.alibaba.android.rimet" => Some("钉钉"),
        "com.ss.android.lark" | "com.ss.android.lark.kami" | "com.larksuite.suite" => Some("飞书"),
        "com.tencent.mobileqq" => Some("QQ"),
        "com.tencent.tim" => Some("TIM"),
        "com.tencent.qqlite" => Some("QQ轻聊"),
        "com.xingin.xhs" => Some("小红书"),
        "com.ss.android.ugc.aweme" | "com.ss.android.ugc.aweme.lite" => Some("抖音"),
        "com.smile.gifmaker" | "com.kuaishou.nebula" => Some("快手"),
        "com.sina.weibo" | "com.sina.weibog3" | "com.weico.international" => Some("微博"),
        "tv.danmaku.bili" | "com.bilibili.app.in" => Some("哔哩哔哩"),
        "com.zhihu.android" => Some("知乎"),
        "com.douban.frodo" => Some("豆瓣"),
        "com.baidu.tieba" => Some("百度贴吧"),
        "cn.soulapp.android" => Some("Soul"),
        "com.immomo.momo" => Some("陌陌"),
        "com.p1.mobile.putong" => Some("探探"),
        "com.mihoyo.hyperion" => Some("米游社"),
        "com.max.xiaoheihe" => Some("小黑盒"),
        "com.taptap" => Some("TapTap"),
        "com.hupu.games" => Some("虎扑"),
        "com.coolapk.market" => Some("酷安"),
        "org.telegram.messenger"
        | "org.telegram.messenger.web"
        | "org.thunderdog.challegram"
        | "org.telegram.plus"
        | "tw.nekomimi.nekogram" => Some("Telegram"),
        "com.whatsapp" | "com.whatsapp.w4b" => Some("WhatsApp"),
        "com.discord" => Some("Discord"),
        "com.Slack" | "com.slack" => Some("Slack"),
        "org.thoughtcrime.securesms" => Some("Signal"),
        "jp.naver.line.android" => Some("LINE"),
        "com.facebook.orca" => Some("Messenger"),
        "com.microsoft.teams" => Some("Teams"),
        "com.kakao.talk" => Some("KakaoTalk"),
        "com.viber.voip" => Some("Viber"),
        "com.instagram.android" | "com.instagram.barcelona" => Some("Instagram"),
        "com.twitter.android" => Some("X"),
        "com.facebook.katana" => Some("Facebook"),
        "com.reddit.frontpage" => Some("Reddit"),

        // 2. Career & Recruitment
        "com.hpbr.bosszhipin" => Some("BOSS直聘"),
        "com.taou.maimai" => Some("脉脉"),
        "com.zhaopin.social" => Some("智联招聘"),
        "com.job.android" => Some("前程无忧"),
        "com.lietou.mishu" | "com.lietpin.android" | "com.lietpin.android.LPActivity" => {
            Some("猎聘")
        }
        "com.linkedin.android" => Some("领英"),
        "com.wuba" => Some("58同城"),
        "com.yupao.android" => Some("鱼泡直聘"),
        "com.shixiseng.activity" => Some("实习僧"),

        // 3. E-commerce & Customer Service
        "com.taobao.taobao" => Some("淘宝"),
        "com.tmall.wireless" => Some("天猫"),
        "com.taobao.idlefish" => Some("闲鱼"),
        "com.jingdong.app.mall" => Some("京东"),
        "com.xunmeng.pinduoduo" => Some("拼多多"),
        "com.shizhuang.duapp" => Some("得物"),
        "com.achievo.vipshop" => Some("唯品会"),
        "com.alibaba.wireless" => Some("1688"),
        "com.wuba.zhuanzhuan" => Some("转转"),
        "com.ss.android.merchant" => Some("抖店"),
        "com.xiaomi.shop" => Some("小米商城"),
        "com.vmall.client" => Some("华为商城"),

        // 4. Food Delivery, Grocery, Express & Travel
        "com.sankuai.meituan" => Some("美团"),
        "com.sankuai.meituan.takeoutnew" => Some("美团外卖"),
        "com.dianping.v1" => Some("大众点评"),
        "me.ele" => Some("饿了么"),
        "com.yaya.zone" => Some("叮咚买菜"),
        "com.wudaokou.hippo" => Some("盒马"),
        "cn.samsclub.app" => Some("山姆"),
        "com.pupumall.customer" => Some("朴朴超市"),
        "com.sf.activity" => Some("顺丰速运"),
        "com.cainiao.wireless" | "com.Cainiao.wireless" => Some("菜鸟"),
        "com.jd.express" => Some("京东快递"),
        "com.fcbox.hiveconsumer" => Some("丰巢"),
        "com.zto.consumer" => Some("中通快递"),
        "com.yto.eyto" => Some("圆通速递"),
        "com.yunda.express" => Some("韵达快递"),
        "com.jtexpress.client" => Some("极兔速递"),
        "com.sdu.didi.psnger" => Some("滴滴出行"),
        "com.autonavi.minimap" => Some("高德地图"),
        "com.baidu.BaiduMap" => Some("百度地图"),
        "com.jingyao.easybike" => Some("哈啰"),
        "cn.caocaokeji.user" => Some("曹操出行"),
        "com.t3go.passenger" => Some("T3出行"),
        "com.MobileTicket" => Some("铁路12306"),
        "com.umetrip.android.msky.app" => Some("航旅纵横"),
        "ctrip.android.view" => Some("携程"),
        "com.Qunar" => Some("去哪儿"),
        "com.taobao.trip" => Some("飞猪"),
        "com.tongcheng.android" => Some("同程旅行"),
        "com.htinns" => Some("华住会"),

        // 5. Finance, Payment & Banking
        "com.eg.android.AlipayGphone" => Some("支付宝"),
        "com.unionpay" => Some("云闪付"),
        "cmb.pb" => Some("招商银行"),
        "com.cmbchina.ccd.pluto.cmbActivity" => Some("掌上生活"),
        "com.icbc" => Some("工商银行"),
        "com.chinamworld.main" => Some("建设银行"),
        "com.android.bankabc" => Some("农业银行"),
        "com.chinamworld.bocmbci" => Some("中国银行"),
        "com.bankcomm.Bankcomm" => Some("交通银行"),
        "com.pingan.paces.ccms" => Some("平安口袋银行"),
        "com.citiccard.mobilebank" => Some("中信银行"),
        "com.cmbc.cc.mbank" => Some("民生银行"),
        "com.spdbccc.app" => Some("浦发银行"),
        "com.cebbank.mobile.cemb" => Some("光大银行"),
        "com.cib.cibmb" => Some("兴业银行"),

        // 6. Workspace, Mail & Dev
        "com.tencent.wemeet.app" => Some("腾讯会议"),
        "us.zoom.videomeetings" => Some("Zoom"),
        "com.tencent.docs" => Some("腾讯文档"),
        "cn.wps.moffice_eng" => Some("WPS"),
        "chuxin.shimo.shimowendang" => Some("石墨文档"),
        "com.yuque.mobile.android.app" => Some("语雀"),
        "notion.id" => Some("Notion"),
        "com.alicloud.databox" => Some("阿里云盘"),
        "com.baidu.netdisk" => Some("百度网盘"),
        "com.quark.browser" => Some("夸克"),
        "com.tencent.androidqqmail" => Some("QQ邮箱"),
        "com.netease.mobimail" => Some("网易邮箱大师"),
        "cn.cj.pe" => Some("139邮箱"),
        "com.alibaba.cloudmail" => Some("阿里邮箱"),
        "com.google.android.gm" => Some("Gmail"),
        "com.microsoft.office.outlook" => Some("Outlook"),
        "com.readdle.spark" => Some("Spark"),
        "com.github.android" => Some("GitHub"),
        _ => None,
    }
}

pub fn is_mainstream_message_app(package: &str) -> bool {
    mainstream_message_app_name(package).is_some()
}

pub fn is_excluded_im_package(pkg: &str) -> bool {
    let trimmed = pkg.trim();
    let lower = trimmed.to_ascii_lowercase();
    matches!(
        trimmed,
        "com.android.shell"
            | "android"
            | "com.android.systemui"
            | "com.android.settings"
            | "com.android.vending"
            | "com.android.launcher3"
            | "com.stepos.ailauncher"
            | "com.stepos.safe"
            | "com.stepx.companion"
            | "com.stepx.companion.app"
            | "com.android.server.telecom"
            | "com.android.incallui"
            | "com.android.dialer"
            | "com.google.android.dialer"
            | "com.android.phone"
            | "com.android.alarmclock"
            | "com.android.deskclock"
            | "com.google.android.deskclock"
            | "com.sec.android.app.clockpackage"
            | "com.oplus.alarmclock"
            | "com.coloros.alarmclock"
            | "com.huawei.deskclock"
            | "com.xiaomi.deskclock"
            | "com.android.calendar"
            | "com.android.providers.calendar"
            | "com.android.calculator2"
            | "com.android.soundrecorder"
            | "com.android.providers.downloads"
            | "com.android.providers.media"
    ) || lower.contains("launcher")
        || lower.contains("alarmclock")
        || lower.contains("deskclock")
        || lower.contains("telecom")
        || lower.contains("incallui")
        || lower.ends_with(".clock")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveCountdownTimer {
    pub key: String,
    pub package_name: String,
    pub app_name: String,
    pub title: String,
    pub total_secs: u32,
    pub remaining_secs: u32,
    pub remaining_label: String,
    pub mm_ss_label: String,
    pub progress_pct: u8,
    pub is_pomodoro: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhoneLiveCapsuleRequest {
    pub tag: String,
    pub pill_text: String,
    pub title: String,
    pub subtitle: String,
    pub badge_text: String,
    /// StepOS `notification.live.type`: `1` = Running/Status, `6` = Takeover/Action, `11` = Timer/Countdown
    pub live_type: i32,
    pub countdown_secs: Option<u64>,
    pub ongoing: bool,
}

impl Default for PhoneLiveCapsuleRequest {
    fn default() -> Self {
        Self {
            tag: "stepx_live_capsule".to_string(),
            pill_text: String::new(),
            title: String::new(),
            subtitle: String::new(),
            badge_text: String::new(),
            live_type: 1,
            countdown_secs: None,
            ongoing: true,
        }
    }
}

pub fn post_phone_live_capsule(
    adb: &Adb,
    serial: &str,
    req: &PhoneLiveCapsuleRequest,
) -> Result<()> {
    let tag = if req.tag.trim().is_empty() {
        "stepx_live_capsule"
    } else {
        req.tag.trim()
    };
    let countdown_secs = req.countdown_secs.unwrap_or(0);
    let effective_live_type = if countdown_secs > 0 {
        11
    } else if req.live_type > 0 {
        req.live_type
    } else {
        1
    };
    let payload = json!({
        "op": "post_live_capsule",
        "tag": tag,
        "pillText": req.pill_text.trim(),
        "pill_text": req.pill_text.trim(),
        "title": req.title.trim(),
        "subtitle": req.subtitle.trim(),
        "badgeText": req.badge_text.trim(),
        "badge_text": req.badge_text.trim(),
        "liveType": effective_live_type,
        "live_type": effective_live_type,
        "countdownSecs": countdown_secs,
        "countdown_secs": countdown_secs,
        "ongoing": req.ongoing,
    });
    if call_push_provider(adb, serial, payload).is_ok() {
        return Ok(());
    }
    let title = match (req.pill_text.trim().is_empty(), req.title.trim().is_empty()) {
        (false, false) => format!("{} · {}", req.pill_text.trim(), req.title.trim()),
        (false, true) => req.pill_text.trim().to_string(),
        (true, false) => req.title.trim().to_string(),
        (true, true) => "StepX 实况胶囊".to_string(),
    };
    let body = if req.subtitle.trim().is_empty() {
        req.badge_text.trim()
    } else {
        req.subtitle.trim()
    };
    let _ = adb.shell(
        serial,
        &[
            "cmd",
            "notification",
            "post",
            "-S",
            "bigtext",
            "-t",
            &title,
            tag,
            body,
        ],
    )?;
    Ok(())
}

pub fn dismiss_phone_live_capsule(adb: &Adb, serial: &str, tag: &str) -> Result<()> {
    let clean_tag = if tag.trim().is_empty() {
        "stepx_live_capsule"
    } else {
        tag.trim()
    };
    let _ = call_push_provider(
        adb,
        serial,
        json!({
            "op": "dismiss_live_capsule",
            "tag": clean_tag,
        }),
    );
    let _ = adb.script(
        serial,
        &format!("cmd notification cancel {clean_tag} >/dev/null 2>&1 || true"),
    );
    Ok(())
}

pub fn clear_all_phone_live_capsules(adb: &Adb, serial: &str) -> Result<()> {
    let _ = call_push_provider(
        adb,
        serial,
        json!({
            "op": "clear_live_capsules",
        }),
    );
    let _ = adb.script(
        serial,
        "for t in stepx_live_capsule stepx_pomodoro stepx_pickup stepx_status stepx_schedule stepx_amoo stepx_transfer; do cmd notification cancel \"$t\" >/dev/null 2>&1 || true; done",
    );
    Ok(())
}

pub fn is_timer_or_countdown_notification(
    pkg: &str,
    app_name: &str,
    title: &str,
    text: &str,
) -> bool {
    let lower_pkg = pkg.trim().to_ascii_lowercase();
    let app = app_name.trim();
    let t = title.trim();
    let body = text.trim();
    let combined = format!("{t} {body}");
    let is_clock_pkg = lower_pkg.contains("alarmclock")
        || lower_pkg.contains("deskclock")
        || lower_pkg.ends_with(".clock")
        || app == "时钟"
        || app == "计时器";
    let has_timer_word = ["倒计时", "计时器", "秒表", "定时器", "番茄钟", "计时"]
        .iter()
        .any(|kw| t.contains(kw) || app.contains(kw));
    let has_remaining_cue =
        body.contains("还剩") || body.contains("剩余") || t.contains("还剩") || t.contains("剩余");
    let has_timer_kw = has_timer_word || has_remaining_cue;
    if is_clock_pkg && has_timer_kw {
        return true;
    }
    has_timer_word && (has_remaining_cue || parse_duration_seconds_from_text(&combined).is_some())
}

pub fn parse_duration_seconds_from_text(raw: &str) -> Option<u32> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // 1. Check Chinese duration units: X小时 / X时, Y分钟 / Y分, Z秒
    let mut total_secs: u32 = 0;
    let mut matched_unit = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let mut val: u32 = 0;
            while i < chars.len() && chars[i].is_ascii_digit() {
                val = val
                    .saturating_mul(10)
                    .saturating_add(chars[i].to_digit(10).unwrap_or(0));
                i += 1;
            }
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let rest: String = chars[i..].iter().collect();
            if rest.starts_with("小时") || rest.starts_with("时") {
                total_secs = total_secs.saturating_add(val.saturating_mul(3600));
                matched_unit = true;
                i += if rest.starts_with("小时") { 2 } else { 1 };
                continue;
            } else if rest.starts_with("分钟") || rest.starts_with("分") {
                total_secs = total_secs.saturating_add(val.saturating_mul(60));
                matched_unit = true;
                i += if rest.starts_with("分钟") { 2 } else { 1 };
                continue;
            } else if rest.starts_with('秒') {
                total_secs = total_secs.saturating_add(val);
                matched_unit = true;
                i += 1;
                continue;
            }
        } else {
            i += 1;
        }
    }
    if matched_unit && total_secs > 0 {
        return Some(total_secs);
    }

    // 2. Check MM:SS or HH:MM:SS token
    for tok in s.split_whitespace() {
        let clean = tok.trim_matches(|c: char| !c.is_ascii_digit() && c != ':');
        if !clean.contains(':') {
            continue;
        }
        let parts: Vec<&str> = clean.split(':').collect();
        if parts.len() == 2 {
            if let (Ok(m), Ok(sec)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                if sec < 60 && (m > 0 || sec > 0) {
                    return Some(m * 60 + sec);
                }
            }
        } else if parts.len() == 3 {
            if let (Ok(h), Ok(m), Ok(sec)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
            ) {
                if m < 60 && sec < 60 && (h > 0 || m > 0 || sec > 0) {
                    return Some(h * 3600 + m * 60 + sec);
                }
            }
        }
    }
    None
}

pub fn format_countdown_remaining_cn(remaining_secs: u32) -> String {
    if remaining_secs >= 3600 {
        let h = remaining_secs / 3600;
        let m = (remaining_secs % 3600).div_ceil(60);
        if m == 0 {
            format!("还剩 {h} 小时")
        } else {
            format!("还剩 {h} 小时 {m} 分钟")
        }
    } else if remaining_secs >= 60 {
        let m = remaining_secs.div_ceil(60);
        format!("还剩 {m} 分钟")
    } else {
        format!("还剩 {} 秒", remaining_secs.max(1))
    }
}

pub fn format_countdown_mm_ss(remaining_secs: u32) -> String {
    if remaining_secs >= 3600 {
        format!(
            "{:02}:{:02}:{:02}",
            remaining_secs / 3600,
            (remaining_secs % 3600) / 60,
            remaining_secs % 60
        )
    } else {
        format!("{:02}:{:02}", remaining_secs / 60, remaining_secs % 60)
    }
}

pub fn extract_live_countdown_from_notification(
    n: &PushNotification,
    now_ms: i64,
) -> Option<LiveCountdownTimer> {
    if n.dismissed || n.removed_time_ms > 0 {
        return None;
    }
    if n.title.contains("结束")
        || n.title.contains("已停止")
        || n.text.contains("计时结束")
        || n.text.contains("已停止")
    {
        return None;
    }
    if !is_timer_or_countdown_notification(&n.package_name, &n.app_name, &n.title, &n.text) {
        return None;
    }
    let reported_rem_secs = parse_duration_seconds_from_text(&n.text)
        .or_else(|| parse_duration_seconds_from_text(&n.sub_text))
        .or_else(|| parse_duration_seconds_from_text(&n.title))?;
    if reported_rem_secs == 0 {
        return None;
    }
    let elapsed_since_post_secs = if n.post_time_ms > 0 && now_ms > n.post_time_ms {
        ((now_ms - n.post_time_ms) / 1000).max(0) as u32
    } else {
        0
    };
    let remaining_secs = if elapsed_since_post_secs < reported_rem_secs {
        reported_rem_secs - elapsed_since_post_secs
    } else if n.active && elapsed_since_post_secs <= reported_rem_secs.saturating_add(90) {
        // Active ongoing clock notification within 90s grace window of last minute tick
        1
    } else {
        return None;
    };
    let total_secs = parse_duration_seconds_from_text(&n.title)
        .unwrap_or(reported_rem_secs)
        .max(remaining_secs)
        .max(1);
    let elapsed = total_secs.saturating_sub(remaining_secs);
    let progress_pct = ((elapsed as u64 * 100) / (total_secs as u64)).clamp(4, 99) as u8;
    let title = if n.title.trim().is_empty() {
        "手机倒计时".to_string()
    } else {
        n.title.trim().to_string()
    };
    let app_name = if n.app_name.trim().is_empty() {
        "时钟".to_string()
    } else {
        n.app_name.trim().to_string()
    };
    Some(LiveCountdownTimer {
        key: n.key.clone(),
        package_name: n.package_name.clone(),
        app_name,
        title,
        total_secs,
        remaining_secs,
        remaining_label: format_countdown_remaining_cn(remaining_secs),
        mm_ss_label: format_countdown_mm_ss(remaining_secs),
        progress_pct,
        is_pomodoro: false,
    })
}

pub fn extract_active_countdown_timers(
    snapshot: &PushSnapshot,
    now_ms: i64,
) -> Vec<LiveCountdownTimer> {
    let mut out: Vec<LiveCountdownTimer> = Vec::new();
    let mut seen_titles = std::collections::HashSet::new();
    for n in &snapshot.notifications {
        if let Some(timer) = extract_live_countdown_from_notification(n, now_ms) {
            let dedup_key = format!("{}::{}", timer.package_name, timer.title);
            if seen_titles.insert(dedup_key) {
                out.push(timer);
            }
        }
    }
    for thread in &snapshot.threads {
        if !is_timer_or_countdown_notification(
            &thread.package_name,
            &thread.app_name,
            &thread.title,
            &thread.last_message,
        ) {
            continue;
        }
        let key = if !thread.notification_key.trim().is_empty() {
            thread.notification_key.clone()
        } else {
            thread.thread_id.clone()
        };
        if out.iter().any(|t| !key.is_empty() && t.key == key) {
            continue;
        }
        let synth = PushNotification {
            key,
            package_name: thread.package_name.clone(),
            app_name: thread.app_name.clone(),
            title: thread.title.clone(),
            text: thread.last_message.clone(),
            post_time_ms: thread.last_timestamp_ms,
            active: true,
            ..Default::default()
        };
        if let Some(timer) = extract_live_countdown_from_notification(&synth, now_ms) {
            let dedup_key = format!("{}::{}", timer.package_name, timer.title);
            if seen_titles.insert(dedup_key) {
                out.push(timer);
            }
        }
    }
    out
}

fn is_non_im_system_package(package_name: &str) -> bool {
    is_excluded_im_package(package_name)
}

pub fn is_system_or_lockscreen_title(title: &str) -> bool {
    let t = title.trim();
    if t.contains("锁屏")
        || t.contains("输入密码")
        || t.contains("密码验证")
        || t.contains("上滑解锁")
    {
        return true;
    }
    matches!(
        t,
        "输入密码"
            | "请输入密码"
            | "锁屏"
            | "锁屏界面"
            | "系统桌面"
            | "桌面"
            | "选择输入法"
            | "更改键盘"
            | "USB 调试"
            | "无线调试"
            | "USB 文件传输"
            | "正在通过 USB"
            | "StepX伴侣"
            | "权限请求"
    )
}

pub fn title_specificity_rank(title: &str) -> u8 {
    let t = title.trim();
    if t.is_empty() || is_system_or_lockscreen_title(t) {
        return 0;
    }
    if matches!(
        t,
        "群聊"
            | "微信"
            | "微信用户"
            | "WeChat"
            | "企业微信"
            | "钉钉"
            | "飞书"
            | "QQ"
            | "TIM"
            | "消息"
            | "聊天"
            | "会话"
            | "对话"
            | "未知会话"
            | "未知联系人"
    ) || t.eq_ignore_ascii_case("group chat")
    {
        return 1;
    }
    if let Some(rest) = t.strip_prefix("群聊") {
        let rest = rest.trim();
        let inner = rest
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .or_else(|| rest.strip_prefix('（').and_then(|s| s.strip_suffix('）')));
        if let Some(digits) = inner {
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                return 1;
            }
        }
        if rest.starts_with('·') || rest.starts_with('(') || rest.starts_with('（') {
            return 2;
        }
    }
    3
}

pub fn is_generic_im_title(title: &str) -> bool {
    title_specificity_rank(title) < 3
}

pub fn is_generic_sender_name(sender: &str, thread_title: &str) -> bool {
    let s = sender.trim();
    let title = thread_title.trim();
    s.is_empty()
        || is_generic_im_title(s)
        || matches!(
            s,
            "群成员" | "未知" | "未知用户" | "对方" | "用户" | "微信用户" | "qq用户" | "QQ用户"
        )
        || (!title.is_empty() && s == title && (is_generic_im_title(title) || title.contains('群')))
}

pub fn normalize_conversation_title(raw: &str) -> String {
    let mut s = raw.replace('\u{00A0}', " ").trim().to_string();
    for _ in 0..2 {
        let trimmed = s.trim_start();
        if let Some(after_open) = trimmed
            .strip_prefix('[')
            .or_else(|| trimmed.strip_prefix('【'))
            .or_else(|| trimmed.strip_prefix('('))
            .or_else(|| trimmed.strip_prefix('（'))
        {
            if let Some(close_idx) = after_open.find([']', '】', ')', '）']) {
                let badge = &after_open[..close_idx];
                if badge.chars().any(|c| c.is_ascii_digit())
                    && (badge.contains('条')
                        || badge.contains("未读")
                        || badge.contains("通知")
                        || badge.contains("消息"))
                {
                    s = after_open[close_idx..]
                        .trim_start_matches([']', '】', ')', '）'])
                        .trim()
                        .to_string();
                    continue;
                }
            }
        }
        break;
    }
    for _ in 0..2 {
        let trimmed = s.trim_end();
        if let Some(open_idx) = trimmed.rfind(['(', '（', '[']) {
            let suffix = &trimmed[open_idx..];
            let inner = suffix
                .trim_start_matches(['(', '（', '['])
                .trim_end_matches([')', '）', ']'])
                .trim();
            let is_pure_digits = !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit());
            let is_count_badge = suffix.chars().any(|c| c.is_ascii_digit())
                && (suffix.contains('条')
                    || suffix.contains("未读")
                    || suffix.contains("新消息")
                    || suffix.contains('人')
                    || suffix.contains("成员")
                    || is_pure_digits);
            if is_count_badge {
                let prefix = trimmed[..open_idx].trim();
                if !prefix.is_empty() && !is_generic_im_title(prefix) {
                    s = prefix.to_string();
                    continue;
                }
            }
        }
        break;
    }
    if s.contains("[表情]") {
        let without_placeholder = s.replace("[表情]", "").trim().to_string();
        if !without_placeholder.is_empty() {
            s = without_placeholder;
        }
    }
    s.trim().to_string()
}

pub fn strip_enterprise_at_suffix(raw: &str) -> String {
    let mut s = raw
        .trim()
        .trim_end_matches("...")
        .trim_end_matches('…')
        .trim()
        .to_string();
    if let Some(at_pos) = s.find(['@', '＠']) {
        let before_raw = &s[..at_pos];
        let prefix = before_raw.trim_end();
        let after_at = s[at_pos..].trim_start_matches(['@', '＠']).trim();
        let has_space_before_at =
            before_raw.ends_with(char::is_whitespace) || prefix.contains(char::is_whitespace);
        let after_has_cjk_or_non_ascii = after_at.chars().any(|c| !c.is_ascii());
        if prefix.chars().count() >= 2 && (has_space_before_at || after_has_cjk_or_non_ascii) {
            s = prefix
                .trim_end_matches("...")
                .trim_end_matches('…')
                .trim()
                .to_string();
        }
    }
    s
}

fn has_enterprise_at_suffix(raw: &str) -> bool {
    let trimmed = raw
        .trim()
        .trim_end_matches("...")
        .trim_end_matches('…')
        .trim();
    strip_enterprise_at_suffix(trimmed) != trimmed
}

fn has_trailing_truncation_ellipsis(raw: &str) -> bool {
    let t = raw.trim();
    t.ends_with('…') || t.ends_with("...")
}

pub fn canonical_conversation_title_key(raw: &str) -> String {
    let norm = clean_message_display_text(&normalize_conversation_title(raw));
    let stripped = strip_enterprise_at_suffix(&norm);
    let mut base = String::with_capacity(stripped.len());
    let mut chars = stripped.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            let mut inner = String::new();
            let mut closed = false;
            while let Some(&nc) = chars.peek() {
                chars.next();
                if nc == ']' {
                    closed = true;
                    break;
                }
                inner.push(nc);
            }
            let char_cnt = inner.chars().count();
            if closed && (1..=4).contains(&char_cnt) && !inner.chars().any(|ch| ch.is_ascii_digit())
            {
                continue;
            }
            base.push('[');
            base.push_str(&inner);
            if closed {
                base.push(']');
            }
        } else {
            base.push(c);
        }
    }
    let trimmed = base
        .trim()
        .trim_end_matches("...")
        .trim_end_matches('…')
        .trim();
    if trimmed.chars().count() >= 2 {
        trimmed.to_string()
    } else {
        stripped
    }
}

pub fn normalize_scraped_media_or_voice_text(text: &str) -> String {
    let t = text.trim();
    if t.is_empty() {
        return String::new();
    }
    match t {
        "图片" => return "[图片]".to_string(),
        "动画表情" | "微信动画表情" => return "[动画表情]".to_string(),
        "视频" => return "[视频]".to_string(),
        "小程序卡片" | "微信小程序" => return "[小程序]".to_string(),
        _ => {}
    }
    if t.contains("语音") {
        let mut s = t.replace("未播放", "");
        if s.starts_with("语音") || s.ends_with("语音") {
            s = s.replace("语音", "");
            let has_unit = s.contains('秒')
                || s.contains('"')
                || s.contains('″')
                || s.contains('\'')
                || s.contains('”');
            let cleaned: String = s
                .chars()
                .filter(|c| !c.is_whitespace() && !matches!(*c, '秒' | '"' | '″' | '\'' | '”'))
                .collect();
            if has_unit
                && (1..=3).contains(&cleaned.len())
                && cleaned.chars().all(|c| c.is_ascii_digit())
            {
                return format!("[语音] {}\"", cleaned);
            }
        }
    }
    t.to_string()
}

fn strip_internal_probe_tags(raw: &str) -> String {
    let mut s = raw.to_string();
    for marker in [
        "[文件传输助手队列消息",
        "[文件传输助手后台回复测试",
        "[文件传输助手息屏回复测试",
        "[StepX队列消息",
        "[测试队列消息",
    ] {
        while let Some(pos) = s.find(marker) {
            let prefix_end = s[..pos].trim_end_matches([' ', '\t', '\u{00A0}']).len();
            if let Some(rel_close) = s[pos..].find(']') {
                let tail_start = pos + rel_close + 1;
                let mut next = String::with_capacity(s.len());
                next.push_str(&s[..prefix_end]);
                next.push_str(&s[tail_start..]);
                s = next;
            } else {
                s.truncate(prefix_end);
                break;
            }
        }
    }
    s
}

/// Strips internal E2E probe suffixes (such as ` [文件传输助手队列消息 #1]`) while keeping the
/// actual message prefix, and removes unrenderable supplementary-plane emoji/tofu glyphs (`>= U+1F000`)
/// so titles and previews never render black `☒` boxes.
pub fn clean_message_display_text(text: &str) -> String {
    let stripped = strip_internal_probe_tags(text);
    let mut out = String::with_capacity(stripped.len());
    for c in stripped.chars() {
        let cp = c as u32;
        if cp >= 0x1F000
            || (0xFE00..=0xFE0F).contains(&cp)
            || cp == 0x200D
            || cp == 0xFFFD
            || cp == 0x2612
        {
            continue;
        }
        out.push(c);
    }
    normalize_scraped_media_or_voice_text(out.trim())
}

fn im_avatar_cache_dir() -> PathBuf {
    let base = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base
        .join(".cache")
        .join("stepx-companion")
        .join("im-avatars");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn sanitize_avatar_key(raw: &str) -> String {
    let mut out = String::new();
    let mut last_underscore = false;
    for c in raw.trim().chars() {
        if c.is_alphanumeric() || c == '-' || c == '+' {
            out.push(c);
            last_underscore = false;
        } else if !last_underscore && !out.is_empty() {
            out.push('_');
            last_underscore = true;
        }
        if out.chars().count() >= 48 {
            break;
        }
    }
    out.trim_matches('_').to_string()
}

fn fnv1a_hash64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn avatar_registry() -> &'static Mutex<HashMap<String, PathBuf>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut map = HashMap::new();
        let dir = im_avatar_cache_dir();
        let index_path = dir.join("index.json");
        if let Ok(raw) = std::fs::read_to_string(&index_path) {
            if let Ok(index_map) = serde_json::from_str::<HashMap<String, String>>(&raw) {
                for (k, rel) in index_map {
                    let p = dir.join(&rel);
                    if p.is_file() {
                        map.insert(k, p);
                    }
                }
            }
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("png") || !path.is_file() {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    map.entry(stem.to_string()).or_insert_with(|| path.clone());
                    if let Some((prefix, hash_part)) = stem.rsplit_once('_') {
                        if hash_part.len() == 16
                            && hash_part.chars().all(|c| c.is_ascii_hexdigit())
                            && !prefix.is_empty()
                        {
                            map.entry(prefix.to_string())
                                .or_insert_with(|| path.clone());
                        }
                    }
                }
            }
        }
        Mutex::new(map)
    })
}

fn persist_avatar_registry_index(map: &HashMap<String, PathBuf>, dir: &std::path::Path) {
    let mut serializable: HashMap<String, String> = HashMap::with_capacity(map.len());
    for (k, path) in map {
        if let Some(fname) = path.file_name().and_then(|f| f.to_str()) {
            serializable.insert(k.clone(), fname.to_string());
        }
    }
    if let Ok(json_bytes) = serde_json::to_vec(&serializable) {
        let tmp = dir.join(".index.json.tmp");
        let dst = dir.join("index.json");
        if std::fs::write(&tmp, &json_bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &dst);
        }
    }
}

fn decode_base64_png_bytes(b64: &str) -> Option<Vec<u8>> {
    let trimmed = b64.trim();
    if trimmed.is_empty() {
        return None;
    }
    let payload = if let Some((_, tail)) = trimmed.split_once(";base64,") {
        tail.trim()
    } else {
        trimmed
    };
    let clean: String = payload
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    if clean.is_empty() {
        return None;
    }
    let bytes = STANDARD
        .decode(&clean)
        .or_else(|_| STANDARD_NO_PAD.decode(clean.trim_end_matches('=')))
        .or_else(|_| URL_SAFE.decode(&clean))
        .or_else(|_| URL_SAFE_NO_PAD.decode(clean.trim_end_matches('=')))
        .ok()?;
    // Validate PNG magic header `\x89PNG\r\n\x1a\n`
    if bytes.len() < 8 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    Some(bytes)
}

pub fn decode_and_cache_avatar_b64(key_hint: &str, b64: &str) -> Option<PathBuf> {
    let bytes = decode_base64_png_bytes(b64)?;
    let hint = key_hint.trim();
    let dir = im_avatar_cache_dir();
    let hash = fnv1a_hash64(&bytes) ^ fnv1a_hash64(hint.as_bytes());
    let safe_key = sanitize_avatar_key(hint);
    let file_name = if safe_key.is_empty() {
        format!("avatar_{hash:016x}.png")
    } else {
        format!("{safe_key}_{hash:016x}.png")
    };
    let final_path = dir.join(&file_name);
    if !final_path.is_file() {
        let tmp_path = dir.join(format!(".tmp_{}_{}", std::process::id(), file_name));
        if std::fs::write(&tmp_path, &bytes).is_ok() {
            if std::fs::rename(&tmp_path, &final_path).is_err() {
                let _ = std::fs::write(&final_path, &bytes);
                let _ = std::fs::remove_file(&tmp_path);
            }
        } else {
            let _ = std::fs::write(&final_path, &bytes);
        }
    }
    if !safe_key.is_empty() {
        let alias_path = dir.join(format!("{safe_key}.png"));
        if !alias_path.is_file() {
            let _ = std::fs::write(&alias_path, &bytes);
        }
    }
    if let Ok(mut guard) = avatar_registry().lock() {
        if !hint.is_empty() {
            guard.insert(hint.to_string(), final_path.clone());
        }
        if !safe_key.is_empty() {
            guard.insert(safe_key, final_path.clone());
        }
        if let Some((pkg, raw_title)) = hint.split_once("::") {
            let norm_title = normalize_conversation_title(raw_title);
            if !norm_title.is_empty() {
                guard.insert(
                    format!("{}::{}", pkg.trim(), norm_title),
                    final_path.clone(),
                );
                guard.insert(norm_title.clone(), final_path.clone());
                let safe_title = sanitize_avatar_key(&norm_title);
                if !safe_title.is_empty() {
                    guard.insert(safe_title, final_path.clone());
                }
            }
            if !raw_title.trim().is_empty() {
                guard.insert(raw_title.trim().to_string(), final_path.clone());
            }
        } else {
            let norm = normalize_conversation_title(hint);
            if !norm.is_empty() {
                guard.insert(norm.clone(), final_path.clone());
                let safe_norm = sanitize_avatar_key(&norm);
                if !safe_norm.is_empty() {
                    guard.insert(safe_norm, final_path.clone());
                }
            }
        }
        persist_avatar_registry_index(&guard, &dir);
    }
    Some(final_path)
}

pub fn cached_thread_avatar_path(package_name: &str, title: &str) -> Option<PathBuf> {
    let pkg = package_name.trim();
    let raw_title = title.trim();
    let norm_title = normalize_conversation_title(raw_title);
    let clean_title = clean_message_display_text(&norm_title);
    let candidates = [
        format!("{pkg}::{raw_title}"),
        format!("{pkg}::{norm_title}"),
        format!("{pkg}::{clean_title}"),
        norm_title.clone(),
        clean_title.clone(),
        raw_title.to_string(),
        sanitize_avatar_key(&format!("{pkg}::{norm_title}")),
        sanitize_avatar_key(&norm_title),
    ];
    if let Ok(guard) = avatar_registry().lock() {
        for cand in &candidates {
            if !cand.is_empty() {
                if let Some(p) = guard.get(cand) {
                    if p.is_file() {
                        return Some(p.clone());
                    }
                }
            }
        }
    }
    let dir = im_avatar_cache_dir();
    for key in [
        sanitize_avatar_key(&format!("{pkg}::{norm_title}")),
        sanitize_avatar_key(&norm_title),
        sanitize_avatar_key(raw_title),
    ] {
        if !key.is_empty() {
            let direct = dir.join(format!("{key}.png"));
            if direct.is_file() {
                return Some(direct);
            }
        }
    }
    None
}

pub fn cached_sender_avatar_path(sender: &str) -> Option<PathBuf> {
    let raw = sender.trim();
    if raw.is_empty() {
        return None;
    }
    let norm = normalize_conversation_title(raw);
    let clean = clean_message_display_text(&norm);
    let safe = sanitize_avatar_key(&norm);
    if let Ok(guard) = avatar_registry().lock() {
        for cand in [raw, norm.as_str(), clean.as_str(), safe.as_str()] {
            if !cand.is_empty() {
                if let Some(p) = guard.get(cand) {
                    if p.is_file() {
                        return Some(p.clone());
                    }
                }
            }
        }
    }
    if !safe.is_empty() {
        let direct = im_avatar_cache_dir().join(format!("{safe}.png"));
        if direct.is_file() {
            return Some(direct);
        }
    }
    None
}

fn im_media_cache_dir() -> PathBuf {
    let base = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base
        .join(".cache")
        .join("stepx-companion")
        .join("im-media-v2");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn media_registry() -> &'static Mutex<HashMap<String, PathBuf>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut map = HashMap::new();
        let dir = im_media_cache_dir();
        let index_path = dir.join("index.json");
        if let Ok(raw) = std::fs::read_to_string(&index_path) {
            if let Ok(index_map) = serde_json::from_str::<HashMap<String, String>>(&raw) {
                for (k, rel) in index_map {
                    let p = dir.join(&rel);
                    if p.is_file() {
                        map.insert(k, p);
                    }
                }
            }
        }
        Mutex::new(map)
    })
}

pub fn decode_and_cache_im_media_b64(key_hint: &str, b64: &str) -> Option<PathBuf> {
    let trimmed = b64.trim();
    if trimmed.is_empty() {
        return None;
    }
    let payload = if let Some((_, tail)) = trimmed.split_once(";base64,") {
        tail.trim()
    } else {
        trimmed
    };
    let clean: String = payload
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    if clean.is_empty() {
        return None;
    }
    let bytes = STANDARD
        .decode(&clean)
        .or_else(|_| STANDARD_NO_PAD.decode(clean.trim_end_matches('=')))
        .or_else(|_| URL_SAFE.decode(&clean))
        .or_else(|_| URL_SAFE_NO_PAD.decode(clean.trim_end_matches('=')))
        .ok()?;
    if bytes.len() < 64 {
        return None;
    }
    let ext = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if bytes.starts_with(&[b'G', b'I', b'F']) {
        "gif"
    } else if bytes.starts_with(&[b'R', b'I', b'F', b'F']) {
        "webp"
    } else {
        "jpg"
    };
    let hint = key_hint.trim();
    let dir = im_media_cache_dir();
    let hash = fnv1a_hash64(&bytes) ^ fnv1a_hash64(hint.as_bytes());
    let file_name = format!("media_{hash:016x}.{ext}");
    let final_path = dir.join(&file_name);
    if !final_path.is_file() {
        let tmp_path = dir.join(format!(".tmp_{}_{}", std::process::id(), file_name));
        if std::fs::write(&tmp_path, &bytes).is_ok() {
            if std::fs::rename(&tmp_path, &final_path).is_err() {
                let _ = std::fs::write(&final_path, &bytes);
                let _ = std::fs::remove_file(&tmp_path);
            }
        } else {
            let _ = std::fs::write(&final_path, &bytes);
        }
    }
    if let Ok(mut guard) = media_registry().lock() {
        if !hint.is_empty() && guard.get(hint) != Some(&final_path) {
            guard.insert(hint.to_string(), final_path.clone());
            persist_avatar_registry_index(&guard, &dir);
        }
    }
    Some(final_path)
}

pub fn cached_im_media_path(thread_id: &str, msg_id: &str) -> Option<PathBuf> {
    let tid = thread_id.trim();
    let mid = msg_id.trim();
    if tid.is_empty() || mid.is_empty() {
        return None;
    }
    // Message IDs are only unique inside a conversation. Never look up an ID alone
    // or a lossy sanitized alias, and never consult the legacy guessed-image cache.
    media_registry()
        .lock()
        .ok()?
        .get(&format!("{tid}::{mid}"))
        .filter(|path| path.is_file())
        .cloned()
}

pub fn verified_message_media_path(thread_id: &str, message: &ImMessage) -> Option<PathBuf> {
    if thread_id.trim().is_empty() || message.id.trim().is_empty() {
        return None;
    }
    if !message.media_b64.trim().is_empty() {
        return decode_and_cache_im_media_b64(
            &format!("{}::{}", thread_id.trim(), message.id.trim()),
            &message.media_b64,
        );
    }
    cached_im_media_path(thread_id, &message.id)
}

fn populate_thread_avatars(thread: &mut ImThread) {
    if !thread.avatar_b64.trim().is_empty() {
        let key_hint = if thread.package_name.trim().is_empty() {
            thread.title.clone()
        } else {
            format!("{}::{}", thread.package_name.trim(), thread.title.trim())
        };
        if let Some(path) = decode_and_cache_avatar_b64(&key_hint, &thread.avatar_b64) {
            thread.avatar_path = Some(path);
        }
    }
    if thread.avatar_path.is_none() {
        thread.avatar_path = cached_thread_avatar_path(&thread.package_name, &thread.title)
            .or_else(|| {
                if !thread.is_group {
                    cached_sender_avatar_path(&thread.title)
                } else {
                    None
                }
            });
    }
    for (sender, b64) in &thread.sender_avatars {
        if !b64.trim().is_empty() {
            if let Some(path) = decode_and_cache_avatar_b64(sender, b64) {
                thread
                    .sender_avatar_paths
                    .insert(sender.trim().to_string(), path.clone());
                let norm = normalize_conversation_title(sender);
                if !norm.is_empty() {
                    thread.sender_avatar_paths.insert(norm, path);
                }
            }
        }
    }
    for msg in &mut thread.messages {
        let s = msg.sender.trim();
        if !s.is_empty() && !thread.sender_avatar_paths.contains_key(s) {
            if let Some(path) = cached_sender_avatar_path(s) {
                thread.sender_avatar_paths.insert(s.to_string(), path);
            }
        }
        msg.media_path = verified_message_media_path(&thread.thread_id, msg);
    }
    if !thread.is_group && thread.avatar_path.is_none() {
        if let Some(path) = thread
            .sender_avatar_paths
            .get(thread.title.trim())
            .cloned()
            .or_else(|| cached_sender_avatar_path(&thread.title))
        {
            thread.avatar_path = Some(path);
        }
    }
}

pub fn split_sender_and_body(raw_text: &str) -> (Option<String>, String) {
    let mut cleaned = raw_text.replace('\u{00A0}', " ").trim().to_string();
    if let Some(after_open) = cleaned
        .strip_prefix('[')
        .or_else(|| cleaned.strip_prefix('【'))
        .or_else(|| cleaned.strip_prefix('('))
        .or_else(|| cleaned.strip_prefix('（'))
    {
        if let Some(close_idx) = after_open.find([']', '】', ')', '）']) {
            let badge = &after_open[..close_idx];
            if badge.chars().any(|c| c.is_ascii_digit())
                && (badge.contains('条') || badge.contains("未读") || badge.contains("消息"))
            {
                cleaned = after_open[close_idx..]
                    .trim_start_matches([']', '】', ')', '）'])
                    .trim()
                    .to_string();
            }
        }
    }
    if let Some(colon_pos) = cleaned.find([':', '：']) {
        let candidate_sender = cleaned[..colon_pos].trim();
        let rest = cleaned[colon_pos..].trim_start_matches([':', '：']).trim();
        if !candidate_sender.is_empty()
            && !rest.is_empty()
            && candidate_sender.chars().count() <= 24
            && !candidate_sender.contains('\n')
            && !candidate_sender.starts_with("http")
            && !rest.starts_with("//")
            && !rest.starts_with("http://")
            && !rest.starts_with("https://")
            && !candidate_sender.contains(['，', '。', '！', '？', '；', '#'])
        {
            return (Some(candidate_sender.to_string()), rest.to_string());
        }
    }
    (None, cleaned)
}

pub fn infer_media_hint_from_text(text: &str) -> String {
    let (_, body) = split_sender_and_body(text);
    let raw_candidate = if body.is_empty() {
        text.trim()
    } else {
        body.trim()
    };
    let candidate_owned = normalize_scraped_media_or_voice_text(raw_candidate);
    let candidate = candidate_owned.trim();
    if candidate.starts_with("[语音]") {
        return candidate.to_string();
    }
    for tag in ["[图片]", "[动画表情]", "[语音]", "[视频]", "[小程序]"] {
        if candidate == tag || (candidate.starts_with(tag) && candidate.chars().count() <= 28) {
            return tag.to_string();
        }
    }
    String::new()
}

fn strip_truncation_ellipsis(s: &str) -> Option<&str> {
    let t = s.trim_end();
    let prefix = t
        .strip_suffix("...")
        .or_else(|| t.strip_suffix("…"))
        .map(str::trim_end)?;
    if prefix.chars().count() >= 12 {
        Some(prefix)
    } else {
        None
    }
}

pub fn messages_match_content(a: &str, b: &str) -> bool {
    let ta_norm = normalize_scraped_media_or_voice_text(a.trim());
    let tb_norm = normalize_scraped_media_or_voice_text(b.trim());
    let ta = ta_norm.trim();
    let tb = tb_norm.trim();
    if ta.is_empty() || tb.is_empty() {
        return false;
    }
    if ta == tb {
        return true;
    }
    let (_, raw_body_a) = split_sender_and_body(ta);
    let (_, raw_body_b) = split_sender_and_body(tb);
    let body_a = normalize_scraped_media_or_voice_text(&raw_body_a);
    let body_b = normalize_scraped_media_or_voice_text(&raw_body_b);
    if !body_a.is_empty() && body_a == body_b {
        return true;
    }
    if let Some(pref_a) = strip_truncation_ellipsis(&body_a) {
        if body_b.chars().count() > pref_a.chars().count() && body_b.starts_with(pref_a) {
            return true;
        }
    }
    if let Some(pref_b) = strip_truncation_ellipsis(&body_b) {
        if body_a.chars().count() > pref_b.chars().count() && body_a.starts_with(pref_b) {
            return true;
        }
    }
    (body_a == "[语音]" && body_b.starts_with("[语音] "))
        || (body_b == "[语音]" && body_a.starts_with("[语音] "))
}

fn is_untimed_ui_preview_or_capture(m: &ImMessage) -> bool {
    m.via == "ui_list_preview"
}

fn can_unify_outgoing_with_preview_or_capture(a: &ImMessage, b: &ImMessage) -> bool {
    let a_out = a.outgoing || a.sender.trim() == "我";
    let b_out = b.outgoing || b.sender.trim() == "我";
    if a_out == b_out {
        return true;
    }
    (a_out && is_untimed_ui_preview_or_capture(b)) || (b_out && is_untimed_ui_preview_or_capture(a))
}

fn enrich_message_pair(target: &mut ImMessage, donor: &ImMessage, thread_title: &str) {
    let target_is_out = target.outgoing || target.sender.trim() == "我";
    let donor_is_out = donor.outgoing || donor.sender.trim() == "我";
    if (target_is_out || donor_is_out) && can_unify_outgoing_with_preview_or_capture(target, donor)
    {
        target.outgoing = true;
        target.sender = "我".to_string();
        if target.delivery_status.is_empty() || target.delivery_status == "received" {
            target.delivery_status = if donor_is_out
                && !donor.delivery_status.is_empty()
                && donor.delivery_status != "received"
            {
                donor.delivery_status.clone()
            } else {
                "sent".to_string()
            };
        }
    } else if !target.outgoing
        && is_generic_sender_name(&target.sender, thread_title)
        && !is_generic_sender_name(&donor.sender, thread_title)
    {
        target.sender = donor.sender.clone();
    } else if target.sender.trim().is_empty() && !donor.sender.trim().is_empty() {
        target.sender = donor.sender.clone();
    }
    if target.text.trim() == "[语音]" && donor.text.trim().starts_with("[语音] ") {
        target.text = donor.text.trim().to_string();
    } else if let Some(pref) = strip_truncation_ellipsis(&target.text) {
        let (_, donor_body) = split_sender_and_body(&donor.text);
        let candidate = if donor_body.is_empty() {
            donor.text.trim()
        } else {
            donor_body.trim()
        };
        if candidate.chars().count() > pref.chars().count() && candidate.starts_with(pref) {
            target.text = candidate.to_string();
        }
    }
    if target.media_hint.trim().is_empty() && !donor.media_hint.trim().is_empty() {
        target.media_hint = donor.media_hint.clone();
    } else if target.media_hint.trim() == "[语音]" && donor.media_hint.trim().starts_with("[语音] ")
    {
        target.media_hint = donor.media_hint.clone();
    }
    if target.media_hint.trim().is_empty() {
        let hint = infer_media_hint_from_text(&target.text);
        if !hint.is_empty() {
            target.media_hint = hint;
        }
    }
    if target.media_b64.trim().is_empty() && !donor.media_b64.trim().is_empty() {
        target.media_b64 = donor.media_b64.clone();
    }
    if target.media_path.is_none() && donor.media_path.is_some() {
        target.media_path = donor.media_path.clone();
    }
    if target.occurred_at_ms().is_none() && donor.occurred_at_ms().is_some() {
        target.timestamp_ms = donor.timestamp_ms;
        target.time_known = Some(true);
    } else if target.timestamp_ms <= 0 && donor.timestamp_ms > 0 {
        target.timestamp_ms = donor.timestamp_ms;
        if donor.time_known == Some(true) {
            target.time_known = Some(true);
        }
    }
    if target.observed_at_ms <= 0 && donor.observed_at_ms > 0 {
        target.observed_at_ms = donor.observed_at_ms;
    }
    if target.client_msg_id.is_empty() && !donor.client_msg_id.is_empty() {
        target.client_msg_id = donor.client_msg_id.clone();
    } else if target.delivery_status == "failed"
        && donor.delivery_status != "failed"
        && !donor.client_msg_id.is_empty()
    {
        target.client_msg_id = donor.client_msg_id.clone();
    }
    if (target.id.is_empty()
        || target.id == target.client_msg_id
        || target.delivery_status == "failed")
        && !donor.id.is_empty()
        && (donor.id != donor.client_msg_id || target.delivery_status == "failed")
    {
        target.id = donor.id.clone();
    }
    if target.outgoing {
        let target_rank = match target.delivery_status.as_str() {
            "sent" => 4,
            "observed" => 3,
            "sending" => 2,
            "unknown" => 1,
            _ => 0,
        };
        let donor_rank = match donor.delivery_status.as_str() {
            "sent" => 4,
            "observed" => 3,
            "sending" => 2,
            "unknown" => 1,
            _ => 0,
        };
        if donor_rank > target_rank {
            target.delivery_status = donor.delivery_status.clone();
        }
        if donor.timestamp_ms > target.timestamp_ms {
            target.timestamp_ms = donor.timestamp_ms;
        }
        if (target.via.is_empty() || target.via.starts_with("ui_"))
            && !donor.via.is_empty()
            && !donor.via.starts_with("ui_")
        {
            target.via = donor.via.clone();
        }
    }
}

fn should_collapse_messages(a: &ImMessage, b: &ImMessage, thread_title: &str) -> bool {
    if a.same_delivery(b) {
        return true;
    }
    // Equal bodies or nearby timestamps do not identify a sender or a media item.
    if a.sender.trim() != b.sender.trim()
        && !is_generic_sender_name(&a.sender, thread_title)
        && !is_generic_sender_name(&b.sender, thread_title)
    {
        // A conversation-list preview has no reliable sender. A captured bubble does.
        if a.via != "ui_list_preview" && b.via != "ui_list_preview" {
            return false;
        }
    }
    let is_visual_media = |m: &ImMessage| {
        ["[图片]", "[动画表情]", "[视频]"]
            .iter()
            .any(|hint| m.media_hint.trim().starts_with(hint) || m.text.trim().starts_with(hint))
    };
    if is_visual_media(a) || is_visual_media(b) {
        return false;
    }
    if !a.media_b64.trim().is_empty()
        && !b.media_b64.trim().is_empty()
        && a.media_b64.trim() != b.media_b64.trim()
    {
        return false;
    }
    if let (Some(pa), Some(pb)) = (&a.media_path, &b.media_path) {
        if pa != pb {
            return false;
        }
    }
    if !can_unify_outgoing_with_preview_or_capture(a, b)
        || !messages_match_content(&a.text, &b.text)
    {
        return false;
    }
    if a.outgoing && b.outgoing {
        if a.delivery_status == "failed" || b.delivery_status == "failed" {
            return true;
        }
        if matches!(a.delivery_status.as_str(), "sending" | "unknown")
            && matches!(b.delivery_status.as_str(), "sending" | "unknown")
        {
            return true;
        }
    }
    let has_distinct_client_ids = !a.client_msg_id.is_empty()
        && !b.client_msg_id.is_empty()
        && a.client_msg_id != b.client_msg_id;
    if has_distinct_client_ids {
        return false;
    }
    if a.occurred_at_ms().is_none()
        || b.occurred_at_ms().is_none()
        || a.via.starts_with("ui_")
        || b.via.starts_with("ui_")
        || a.client_msg_id.is_empty()
        || b.client_msg_id.is_empty()
    {
        if a.timestamp_ms > 0 && b.timestamp_ms > 0 {
            return (a.timestamp_ms - b.timestamp_ms).abs() <= 180_000
                || a.occurred_at_ms().is_none()
                || b.occurred_at_ms().is_none();
        }
        return true;
    }
    false
}

pub fn deduplicate_thread_messages(messages: &mut Vec<ImMessage>, thread_title: &str) {
    for m in messages.iter_mut() {
        m.text = clean_message_display_text(&m.text);
        m.sender = clean_message_display_text(&m.sender);
        if m.sender == "我" && !m.outgoing {
            m.outgoing = true;
            if m.delivery_status.is_empty() || m.delivery_status == "received" {
                m.delivery_status = "sent".into();
            }
        }
    }
    messages.retain(|m| {
        !is_bogus_captured_view_text(&m.text)
            && (!m.text.trim().is_empty() || !m.media_hint.trim().is_empty())
    });

    for m in messages.iter_mut() {
        if !m.outgoing && is_generic_sender_name(&m.sender, thread_title) {
            let (parsed_sender, parsed_body) = split_sender_and_body(&m.text);
            if let Some(s) = parsed_sender {
                if s.trim() == "我" && !parsed_body.is_empty() {
                    m.outgoing = true;
                    m.sender = "我".into();
                    m.text = clean_message_display_text(&parsed_body);
                    if m.delivery_status.is_empty() || m.delivery_status == "received" {
                        m.delivery_status = "sent".into();
                    }
                } else if !is_generic_sender_name(&s, thread_title) && !parsed_body.is_empty() {
                    m.sender = clean_message_display_text(&s);
                    m.text = clean_message_display_text(&parsed_body);
                }
            }
        }
        if m.media_hint.trim().is_empty() {
            let hint = infer_media_hint_from_text(&m.text);
            if !hint.is_empty() {
                m.media_hint = hint;
            }
        } else if m.media_hint.trim() == "[语音]" && m.text.trim().starts_with("[语音] ") {
            m.media_hint = m.text.trim().to_string();
        }
    }

    // If any non-failed outgoing message has the same text as a failed outgoing message,
    // the retry/send superseded the failed bubble; otherwise keep at most the single latest failed bubble.
    let active_outgoing_texts: Vec<String> = messages
        .iter()
        .filter(|m| m.outgoing && m.delivery_status != "failed")
        .map(|m| m.text.trim().to_string())
        .collect();
    messages.retain(|m| {
        if m.outgoing && m.delivery_status == "failed" {
            return !active_outgoing_texts.iter().any(|t| t == m.text.trim());
        }
        true
    });

    let last_failed_idx = messages
        .iter()
        .rposition(|m| m.outgoing && m.delivery_status == "failed");
    let mut idx = 0;
    messages.retain(|m| {
        let cur = idx;
        idx += 1;
        if m.outgoing && m.delivery_status == "failed" {
            return Some(cur) == last_failed_idx;
        }
        true
    });

    let mut deduped: Vec<ImMessage> = Vec::with_capacity(messages.len());
    for msg in messages.drain(..) {
        if let Some(existing) = deduped
            .iter_mut()
            .rev()
            .find(|existing| should_collapse_messages(existing, &msg, thread_title))
        {
            enrich_message_pair(existing, &msg, thread_title);
        } else {
            deduped.push(msg);
        }
    }

    // Fill in missing timestamps for UI-captured messages flanked by messages with known timestamps.
    for m in &mut deduped {
        if m.occurred_at_ms().is_none() {
            m.timestamp_ms = 0;
        }
    }
    let mut last_known_any_ts: Option<i64> = None;
    for m in &mut deduped {
        if let Some(ts) = m.occurred_at_ms() {
            last_known_any_ts = Some(ts);
        } else if m.timestamp_ms <= 0 {
            if let Some(ts) = last_known_any_ts {
                let assigned = if m.outgoing { ts } else { ts.saturating_add(1) };
                m.timestamp_ms = assigned;
                last_known_any_ts = Some(assigned);
            }
        }
    }
    let mut next_known_ts: Option<(i64, bool)> = None;
    for m in deduped.iter_mut().rev() {
        if let Some(ts) = m.occurred_at_ms() {
            next_known_ts = Some((ts, m.outgoing));
        } else if m.timestamp_ms <= 0 {
            if let Some((ts, next_is_outgoing)) = next_known_ts {
                m.timestamp_ms = if !m.outgoing && next_is_outgoing {
                    ts.saturating_sub(1)
                } else {
                    ts
                };
            }
        }
    }

    // Sort chronologically (stable sort preserves screen order when timestamps are equal).
    deduped.sort_by_key(|m| m.timestamp_ms);

    *messages = deduped;
}

pub fn recompute_thread_summary(thread: &mut ImThread) {
    thread.title = clean_message_display_text(&thread.title);
    thread.last_message = clean_message_display_text(&thread.last_message);
    thread.last_sender = clean_message_display_text(&thread.last_sender);

    if !is_generic_sender_name(&thread.last_sender, &thread.title) && thread.last_sender != "我" {
        for m in thread.messages.iter_mut().rev() {
            if !m.outgoing
                && is_generic_sender_name(&m.sender, &thread.title)
                && messages_match_content(&m.text, &thread.last_message)
            {
                m.sender = thread.last_sender.clone();
                break;
            }
        }
    }
    if let Some(last) = thread
        .messages
        .iter()
        .rev()
        .find(|m| !(m.outgoing && m.delivery_status == "failed") && !m.text.trim().is_empty())
    {
        thread.last_message = clean_message_display_text(&last.text);
        if last.outgoing {
            thread.last_sender = "我".into();
        } else if !is_generic_sender_name(&last.sender, &thread.title) {
            thread.last_sender = last.sender.clone();
        } else if thread.last_sender == "我" || thread.last_sender.trim().is_empty() {
            thread.last_sender = last.sender.clone();
        }
        if last.timestamp_ms > thread.last_timestamp_ms {
            thread.last_timestamp_ms = last.timestamp_ms;
        }
    }
    if let Some(max_msg_ts) = thread
        .messages
        .iter()
        .map(|m| m.timestamp_ms.max(m.observed_at_ms))
        .max()
    {
        if max_msg_ts > thread.last_timestamp_ms {
            thread.last_timestamp_ms = max_msg_ts;
        }
    }
    let tail_is_outgoing = thread.messages.last().map(|m| m.outgoing).unwrap_or(false);
    for m in &mut thread.messages {
        if m.timestamp_ms <= 0 && m.observed_at_ms > 0 {
            m.timestamp_ms = m.observed_at_ms;
        } else if m.timestamp_ms <= 0 && thread.last_timestamp_ms > 0 {
            m.timestamp_ms = if !m.outgoing && tail_is_outgoing {
                thread.last_timestamp_ms.saturating_sub(1)
            } else {
                thread.last_timestamp_ms
            };
        }
    }
}

fn title_cleanliness_preference(title: &str) -> u8 {
    let has_at = has_enterprise_at_suffix(title);
    let truncated = has_trailing_truncation_ellipsis(title);
    match (has_at, truncated) {
        (false, false) => 4,
        (true, false) => 3,
        (false, true) => 2,
        (true, true) => 1,
    }
}

fn preserve_thread_identity_and_senders(target: &mut ImThread, source: &ImThread) {
    let source_clean = clean_message_display_text(&normalize_conversation_title(&source.title));
    let target_clean = clean_message_display_text(&normalize_conversation_title(&target.title));
    let source_rank = title_specificity_rank(&source_clean);
    let target_rank = title_specificity_rank(&target_clean);
    let source_pref = title_cleanliness_preference(&source.title);
    let target_pref = title_cleanliness_preference(&target.title);
    if source_rank > target_rank
        || (source_rank == target_rank && source_rank >= 3 && source_pref > target_pref)
    {
        target.title = if has_enterprise_at_suffix(&source_clean)
            && has_trailing_truncation_ellipsis(&source_clean)
        {
            strip_enterprise_at_suffix(&source_clean)
        } else {
            source_clean
        };
        if !source.thread_id.trim().is_empty() && !is_generic_im_title(&source.title) {
            target.thread_id = source.thread_id.clone();
        }
    } else if target_rank >= 3 && !target_clean.is_empty() {
        target.title = if has_enterprise_at_suffix(&target_clean)
            && has_trailing_truncation_ellipsis(&target_clean)
        {
            strip_enterprise_at_suffix(&target_clean)
        } else {
            target_clean
        };
    }
    target.is_group = target.is_group
        || source.is_group
        || source.title.contains("群")
        || target.title.contains("群");
    target.can_remote_reply = target.can_remote_reply || source.can_remote_reply;
    target.can_ui_reply = target.can_ui_reply || source.can_ui_reply;
    target.has_content_intent = target.has_content_intent || source.has_content_intent;
    if (target.notification_key.trim().is_empty()
        || target.notification_key.starts_with("ui_window::"))
        && !source.notification_key.trim().is_empty()
        && !source.notification_key.starts_with("ui_window::")
    {
        target.notification_key = source.notification_key.clone();
    } else if target.notification_key.trim().is_empty()
        && !source.notification_key.trim().is_empty()
    {
        target.notification_key = source.notification_key.clone();
    }
    if (target.app_name.trim().is_empty() || target.app_name == target.package_name)
        && !source.app_name.trim().is_empty()
    {
        target.app_name = source.app_name.clone();
    }
    if is_generic_sender_name(&target.last_sender, &target.title)
        && !is_generic_sender_name(&source.last_sender, &target.title)
    {
        target.last_sender = source.last_sender.clone();
    }
    target.last_timestamp_ms = target.last_timestamp_ms.max(source.last_timestamp_ms);

    if target.avatar_b64.trim().is_empty() && !source.avatar_b64.trim().is_empty() {
        target.avatar_b64 = source.avatar_b64.clone();
    }
    if target.avatar_path.is_none() && source.avatar_path.is_some() {
        target.avatar_path = source.avatar_path.clone();
    }
    for (k, v) in &source.sender_avatars {
        if !v.trim().is_empty() {
            target
                .sender_avatars
                .entry(k.clone())
                .or_insert_with(|| v.clone());
        }
    }
    for (k, v) in &source.sender_avatar_paths {
        target
            .sender_avatar_paths
            .entry(k.clone())
            .or_insert_with(|| v.clone());
    }

    let title_snapshot = target.title.clone();
    for tm in &mut target.messages {
        if let Some(sm) = source
            .messages
            .iter()
            .find(|sm| should_collapse_messages(tm, sm, &title_snapshot))
        {
            enrich_message_pair(tm, sm, &title_snapshot);
        }
    }
}

/// Merges `incoming` (such as a screen-captured window or notification thread) into `existing`
/// without allowing generic titles (`"群聊"`), generic senders (`"微信用户"`), or missing timestamps
/// to overwrite known metadata.
pub fn merge_thread_preserving_metadata(existing: &mut ImThread, mut incoming: ImThread) {
    preserve_thread_identity_and_senders(existing, &incoming);
    preserve_thread_identity_and_senders(&mut incoming, existing);

    existing.unread_count = existing.unread_count.max(incoming.unread_count);
    let incoming_has_real_key = !incoming.notification_key.trim().is_empty()
        && !incoming.notification_key.starts_with("ui_window::");
    let existing_has_real_key = !existing.notification_key.trim().is_empty()
        && !existing.notification_key.starts_with("ui_window::");
    if incoming_has_real_key
        && (!existing_has_real_key || incoming.last_timestamp_ms >= existing.last_timestamp_ms)
    {
        existing.notification_key = incoming.notification_key.clone();
    } else if existing.notification_key.trim().is_empty()
        && !incoming.notification_key.trim().is_empty()
    {
        existing.notification_key = incoming.notification_key.clone();
    }

    let incoming_is_screen_order = incoming
        .messages
        .iter()
        .any(|m| m.via.starts_with("ui_") || m.time_known == Some(false));

    if incoming_is_screen_order {
        // Interleave existing messages and incoming screen messages using matched anchors
        // so historical messages before the screen snapshot stay before it, intervening messages
        // stay between their anchors, and newly sent local outgoing messages after all screen
        // messages remain at the bottom.
        let e_len = existing.messages.len();
        let i_len = incoming.messages.len();
        let mut matched_e_to_i: Vec<Option<usize>> = vec![None; e_len];
        let mut matched_i_used: Vec<bool> = vec![false; i_len];

        for (e_idx, em) in existing.messages.iter().enumerate() {
            if let Some(i_idx) = (0..i_len).find(|&i_idx| {
                !matched_i_used[i_idx]
                    && should_collapse_messages(em, &incoming.messages[i_idx], &existing.title)
            }) {
                matched_e_to_i[e_idx] = Some(i_idx);
                matched_i_used[i_idx] = true;
                enrich_message_pair(&mut incoming.messages[i_idx], em, &existing.title);
            }
        }

        let mut before_each_i: Vec<Vec<ImMessage>> = vec![Vec::new(); i_len];
        let mut after_each_i: Vec<Vec<ImMessage>> = vec![Vec::new(); i_len];
        let mut after_all: Vec<ImMessage> = Vec::new();

        for (e_idx, em) in existing.messages.iter().enumerate() {
            if matched_e_to_i[e_idx].is_some() {
                continue;
            }
            let prev_anchor_i = (0..e_idx).rev().find_map(|k| matched_e_to_i[k]);
            let next_anchor_i = ((e_idx + 1)..e_len).find_map(|k| matched_e_to_i[k]);

            match (prev_anchor_i, next_anchor_i) {
                (None, Some(next_i)) => {
                    before_each_i[next_i].push(em.clone());
                }
                (Some(prev_i), Some(next_i)) => {
                    if prev_i < next_i {
                        after_each_i[prev_i].push(em.clone());
                    } else {
                        before_each_i[next_i].push(em.clone());
                    }
                }
                (Some(prev_i), None) => {
                    let em_ts = em.occurred_at_ms().unwrap_or(0);
                    let tail_has_newer_observation =
                        incoming.messages[(prev_i + 1)..].iter().any(|im| {
                            let im_ts = im.occurred_at_ms().unwrap_or(0).max(im.observed_at_ms);
                            em_ts > 0 && im_ts >= em_ts
                        });
                    if em.outgoing && em_ts > 0 && !tail_has_newer_observation {
                        after_all.push(em.clone());
                    } else {
                        after_each_i[prev_i].push(em.clone());
                    }
                }
                (None, None) => {
                    let em_ts = em.occurred_at_ms().unwrap_or(0);
                    let any_newer_in_incoming = incoming.messages.iter().any(|im| {
                        let im_ts = im.occurred_at_ms().unwrap_or(0).max(im.observed_at_ms);
                        em_ts > 0 && im_ts >= em_ts
                    });
                    if em.outgoing && em_ts > 0 && !any_newer_in_incoming {
                        after_all.push(em.clone());
                    } else if i_len > 0 {
                        before_each_i[0].push(em.clone());
                    } else {
                        after_all.push(em.clone());
                    }
                }
            }
        }

        let mut combined: Vec<ImMessage> = Vec::with_capacity(e_len + i_len);
        for (i_idx, im) in incoming.messages.into_iter().enumerate() {
            combined.append(&mut before_each_i[i_idx]);
            combined.push(im);
            combined.append(&mut after_each_i[i_idx]);
        }
        combined.append(&mut after_all);
        existing.messages = combined;
    } else {
        for im in incoming.messages {
            if let Some(em) = existing
                .messages
                .iter_mut()
                .find(|em| should_collapse_messages(em, &im, &existing.title))
            {
                enrich_message_pair(em, &im, &existing.title);
            } else {
                existing.messages.push(im);
            }
        }
    }

    deduplicate_thread_messages(&mut existing.messages, &existing.title);
    recompute_thread_summary(existing);
    populate_thread_avatars(existing);
}

fn threads_share_message_content(a: &ImThread, b: &ImThread) -> bool {
    let a_matches_b = a.messages.iter().any(|ma| {
        !ma.text.trim().is_empty()
            && (b
                .messages
                .iter()
                .any(|mb| messages_match_content(&ma.text, &mb.text))
                || messages_match_content(&ma.text, &b.last_message))
    });
    if a_matches_b {
        return true;
    }
    !a.last_message.trim().is_empty()
        && (messages_match_content(&a.last_message, &b.last_message)
            || b.messages
                .iter()
                .any(|mb| messages_match_content(&a.last_message, &mb.text)))
}

fn titles_match_canonical_or_truncated(raw_a: &str, raw_b: &str, ca: &str, cb: &str) -> bool {
    if ca.is_empty() || cb.is_empty() {
        return false;
    }
    if ca == cb {
        return true;
    }
    if has_trailing_truncation_ellipsis(raw_a) && ca.chars().count() >= 3 && cb.starts_with(ca) {
        return true;
    }
    if has_trailing_truncation_ellipsis(raw_b) && cb.chars().count() >= 3 && ca.starts_with(cb) {
        return true;
    }
    false
}

fn threads_belong_to_same_conversation(a: &ImThread, b: &ImThread) -> bool {
    let ta = clean_message_display_text(&normalize_conversation_title(&a.title));
    let tb = clean_message_display_text(&normalize_conversation_title(&b.title));
    let ca = canonical_conversation_title_key(&ta);
    let cb = canonical_conversation_title_key(&tb);
    let a_gen = is_generic_im_title(&ta);
    let b_gen = is_generic_im_title(&tb);
    let titles_match = titles_match_canonical_or_truncated(&a.title, &b.title, &ca, &cb);

    // When both threads have non-empty, non-generic titles and they differ (e.g. "顺丰速运+" vs "圆通速递"),
    // they NEVER belong to the same conversation even if WeChat reuses the same notification_key.
    if !ca.is_empty() && !cb.is_empty() && !a_gen && !b_gen && !titles_match {
        return false;
    }

    if a.thread_id == b.thread_id && !a.thread_id.is_empty() {
        return true;
    }
    if a.package_name.is_empty() || a.package_name != b.package_name {
        return false;
    }
    let a_has_real_key =
        !a.notification_key.trim().is_empty() && !a.notification_key.starts_with("ui_window::");
    let b_has_real_key =
        !b.notification_key.trim().is_empty() && !b.notification_key.starts_with("ui_window::");
    if a_has_real_key
        && b_has_real_key
        && a.notification_key == b.notification_key
        && (ca.is_empty() || cb.is_empty() || a_gen || b_gen || titles_match)
    {
        return true;
    }
    if titles_match {
        return true;
    }
    if a_gen || b_gen {
        // When at least one thread has a generic title (e.g. "群聊" or "群聊 · 郑-stepx")
        // and the other has a specific title (e.g. "StepX 通知自测群"), merge ONLY if they
        // share message content so unrelated conversations are never collapsed together.
        if threads_share_message_content(a, b) {
            return true;
        }
    }
    false
}

fn has_service_or_merchant_keywords(s: &str) -> bool {
    const SERVICE_KWS: [&str; 19] = [
        "提醒",
        "通知",
        "取餐",
        "派件",
        "签收",
        "配送",
        "订单",
        "账单",
        "凭证",
        "速运",
        "速递",
        "快递",
        "买菜",
        "茶姬",
        "取件",
        "快件",
        "驿站",
        "公众号",
        "服务通知",
    ];
    SERVICE_KWS.iter().any(|kw| s.contains(kw))
}

fn split_polluted_service_threads(threads: &mut Vec<ImThread>) {
    let mut result: Vec<ImThread> = Vec::with_capacity(threads.len());
    for mut thread in threads.drain(..) {
        thread.title = clean_message_display_text(&normalize_conversation_title(&thread.title));
        let norm_title = thread.title.clone();
        let is_explicit_group_title = norm_title.contains("群")
            || norm_title.contains("群聊")
            || norm_title.contains('(')
            || norm_title.contains('（');
        let is_sms = is_sms_package_or_thread(&thread.package_name, &thread.thread_id)
            || sanitize_sms_number_candidate(&norm_title).is_some();

        // A structured conversation or reply handle is stronger evidence than words
        // such as “通知” or “订单” in a message. Splitting it invents another recipient
        // and loses the only background reply handle.
        let has_bound_notification = !thread.notification_key.trim().is_empty()
            && !thread.notification_key.starts_with("ui_window::")
            && thread.messages.iter().any(|m| m.via == "notification");
        if thread.is_group
            || thread.can_remote_reply
            || has_bound_notification
            || is_explicit_group_title
            || is_sms
        {
            result.push(thread);
            continue;
        }

        let is_service_hub = matches!(
            norm_title.as_str(),
            "服务通知" | "公众号" | "订阅号" | "服务号" | "小程序"
        );
        let mut distinct_incoming_senders = std::collections::HashSet::new();
        if !is_service_hub && !norm_title.is_empty() && !is_generic_im_title(&norm_title) {
            distinct_incoming_senders.insert(norm_title.clone());
        }
        let mut has_foreign_incoming = false;
        let mut any_service_kw = has_service_or_merchant_keywords(&norm_title);

        for m in &thread.messages {
            if m.outgoing {
                continue;
            }
            let s = clean_message_display_text(&normalize_conversation_title(&m.sender));
            if has_service_or_merchant_keywords(&s) {
                any_service_kw = true;
            }
            if !s.is_empty() && s != "我" && !is_generic_sender_name(&s, &norm_title) {
                distinct_incoming_senders.insert(s.clone());
                if s != norm_title {
                    has_foreign_incoming = true;
                }
            }
        }

        let should_split = (is_service_hub || any_service_kw)
            && (is_service_hub
                || (has_foreign_incoming
                    && (distinct_incoming_senders.len() >= 2 || any_service_kw)));

        if !should_split {
            result.push(thread);
            continue;
        }

        let orig_pkg = thread.package_name.clone();
        let orig_app = thread.app_name.clone();
        let orig_unread = thread.unread_count;
        let orig_ts = thread.last_timestamp_ms;
        let orig_can_ui_reply = thread.can_ui_reply;
        let orig_has_intent = thread.has_content_intent;
        let orig_sender_avatars = thread.sender_avatars.clone();
        let orig_sender_avatar_paths = thread.sender_avatar_paths.clone();

        let mut kept_messages: Vec<ImMessage> = Vec::new();
        let mut foreign_buckets: Vec<(String, Vec<ImMessage>)> = Vec::new();

        for mut msg in thread.messages.drain(..) {
            msg.text = clean_message_display_text(&msg.text);
            msg.sender = clean_message_display_text(&normalize_conversation_title(&msg.sender));
            let s = msg.sender.trim().to_string();
            let is_foreign = !msg.outgoing
                && !s.is_empty()
                && s != "我"
                && !is_generic_sender_name(&s, &norm_title)
                && (is_service_hub || s != norm_title);

            if is_foreign {
                if let Some((_, bucket)) = foreign_buckets.iter_mut().find(|(k, _)| *k == s) {
                    bucket.push(msg);
                } else {
                    foreign_buckets.push((s, vec![msg]));
                }
            } else {
                kept_messages.push(msg);
            }
        }

        if !kept_messages.is_empty() || foreign_buckets.is_empty() {
            thread.is_group = false;
            for m in &mut kept_messages {
                if m.timestamp_ms <= 0 && m.observed_at_ms > 0 {
                    m.timestamp_ms = m.observed_at_ms;
                }
            }
            thread.messages = kept_messages;
            thread.last_timestamp_ms = thread
                .messages
                .iter()
                .map(|m| m.timestamp_ms.max(m.observed_at_ms))
                .filter(|&ts| ts > 0)
                .max()
                .unwrap_or(orig_ts);
            thread.last_message.clear();
            thread.last_sender = thread.title.clone();
            if orig_unread > 0 {
                thread.unread_count = thread
                    .messages
                    .iter()
                    .filter(|m| !m.outgoing)
                    .count()
                    .max(1);
            }
            deduplicate_thread_messages(&mut thread.messages, &thread.title);
            recompute_thread_summary(&mut thread);
            populate_thread_avatars(&mut thread);
            result.push(thread);
        }

        for (foreign_sender, mut f_msgs) in foreign_buckets {
            for m in &mut f_msgs {
                if m.timestamp_ms <= 0 && m.observed_at_ms > 0 {
                    m.timestamp_ms = m.observed_at_ms;
                }
            }
            deduplicate_thread_messages(&mut f_msgs, &foreign_sender);
            let max_ts = f_msgs
                .iter()
                .map(|m| m.timestamp_ms.max(m.observed_at_ms))
                .filter(|&ts| ts > 0)
                .max()
                .unwrap_or(orig_ts);
            let unread = if orig_unread > 0 {
                f_msgs.iter().filter(|m| !m.outgoing).count().max(1)
            } else {
                0
            };
            let avatar_b64 = orig_sender_avatars
                .get(&foreign_sender)
                .cloned()
                .unwrap_or_default();
            let avatar_path = orig_sender_avatar_paths.get(&foreign_sender).cloned();
            let mut split_thread = ImThread {
                thread_id: format!("{orig_pkg}::{foreign_sender}"),
                package_name: orig_pkg.clone(),
                app_name: orig_app.clone(),
                title: foreign_sender.clone(),
                is_group: false,
                unread_count: unread,
                last_sender: foreign_sender.clone(),
                last_message: String::new(),
                last_timestamp_ms: max_ts,
                can_remote_reply: false,
                can_ui_reply: orig_can_ui_reply,
                has_content_intent: orig_has_intent,
                notification_key: String::new(),
                messages: f_msgs,
                avatar_b64,
                sender_avatars: HashMap::new(),
                avatar_path,
                sender_avatar_paths: HashMap::new(),
            };
            recompute_thread_summary(&mut split_thread);
            populate_thread_avatars(&mut split_thread);
            result.push(split_thread);
        }
    }
    *threads = result;
}

pub fn consolidate_im_threads(threads: &mut Vec<ImThread>, notifications: &[PushNotification]) {
    // 0. Split any polluted service/merchant threads before folding notifications.
    split_polluted_service_threads(threads);

    // 1. Fold active (or recent non-dismissed) IM & mainstream notifications into threads so lockscreen
    //    notification data (real group title, sender name, timestamp, RemoteInput handle, notification key)
    //    is always present.
    for n in notifications {
        if n.dismissed
            || n.removed_time_ms > 0
            || is_excluded_im_package(&n.package_name)
            || is_native_sms_package_or_thread(&n.package_name, &n.thread_id)
            || is_system_or_lockscreen_title(&n.title)
            || is_timer_or_countdown_notification(&n.package_name, &n.app_name, &n.title, &n.text)
        {
            continue;
        }
        let is_im = n.category.eq_ignore_ascii_case("im")
            || n.category.eq_ignore_ascii_case("social")
            || n.category.eq_ignore_ascii_case("delivery_travel")
            || n.category.eq_ignore_ascii_case("finance")
            || n.can_reply
            || !n.thread_id.trim().is_empty()
            || is_mainstream_message_app(&n.package_name)
            || is_career_or_recruitment_signal(&n.package_name, &n.app_name, &n.title, &n.text)
            || matches!(
                ImChannel::from_package(&n.package_name),
                ImChannel::WeChat
                    | ImChannel::WeCom
                    | ImChannel::DingTalk
                    | ImChannel::Feishu
                    | ImChannel::QqTim
                    | ImChannel::SocialDm
            );
        if !is_im {
            continue;
        }
        let clean_title = clean_message_display_text(&normalize_conversation_title(&n.title));
        if clean_title.is_empty() && n.text.trim().is_empty() {
            continue;
        }
        let (parsed_sender, parsed_body) = split_sender_and_body(&n.text);
        let effective_sender =
            if !n.sender.trim().is_empty() && !is_generic_sender_name(&n.sender, &clean_title) {
                clean_message_display_text(n.sender.trim())
            } else if let Some(s) = parsed_sender {
                clean_message_display_text(&s)
            } else {
                clean_title.clone()
            };
        let effective_body = clean_message_display_text(if parsed_body.is_empty() {
            n.text.trim()
        } else {
            &parsed_body
        });
        let tid = if !n.thread_id.trim().is_empty() && !is_generic_im_title(&n.title) {
            n.thread_id.trim().to_string()
        } else {
            format!("{}::{}", n.package_name, clean_title)
        };
        let notif_msg = ImMessage {
            id: if n.key.is_empty() {
                format!("notif-{}", n.post_time_ms)
            } else {
                format!("{}:{}", n.key, n.post_time_ms)
            },
            sender: effective_sender.clone(),
            text: effective_body.clone(),
            timestamp_ms: n.post_time_ms,
            observed_at_ms: n.post_time_ms,
            time_known: Some(n.post_time_ms > 0),
            recalled: false,
            outgoing: false,
            delivery_status: "received".into(),
            via: "notification".into(),
            media_hint: infer_media_hint_from_text(&effective_body),
            client_msg_id: String::new(),
            ..Default::default()
        };
        let mut notif_thread = ImThread {
            thread_id: tid,
            package_name: n.package_name.clone(),
            app_name: if n.app_name.is_empty()
                || n.app_name == n.package_name
                || n.app_name == "更多社交"
                || n.app_name == "社媒私信"
            {
                mainstream_message_app_name(&n.package_name)
                    .unwrap_or_else(|| ImChannel::from_package(&n.package_name).label())
                    .to_string()
            } else {
                n.app_name.clone()
            },
            title: clean_title.clone(),
            is_group: n.is_group
                || (!effective_sender.is_empty() && effective_sender != clean_title)
                || clean_title.contains("群"),
            unread_count: usize::from(!n.read && !n.dismissed),
            last_sender: effective_sender,
            last_message: effective_body.clone(),
            last_timestamp_ms: n.post_time_ms,
            can_remote_reply: n.can_reply,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: n.key.clone(),
            messages: if effective_body.is_empty() {
                Vec::new()
            } else {
                vec![notif_msg]
            },
            avatar_b64: n.avatar_b64.clone(),
            sender_avatars: n.sender_avatars.clone(),
            avatar_path: None,
            sender_avatar_paths: HashMap::new(),
        };

        if let Some(existing) = threads
            .iter_mut()
            .find(|t| threads_belong_to_same_conversation(t, &notif_thread))
        {
            if !n.active {
                continue;
            }
            if existing.unread_count == 0
                && notif_thread.last_timestamp_ms <= existing.last_timestamp_ms
            {
                notif_thread.unread_count = 0;
            }
            merge_thread_preserving_metadata(existing, notif_thread);
        } else {
            threads.push(notif_thread);
        }
    }

    // Split again in case folding notifications introduced a service hub thread.
    split_polluted_service_threads(threads);

    // 2. Consolidate duplicate or generic ("群聊") threads into specific threads that share conversation identity/messages.
    let mut consolidated: Vec<ImThread> = Vec::with_capacity(threads.len());
    // Process threads with highest title specificity first so generic threads merge into specific ones.
    threads.sort_by_key(|t| std::cmp::Reverse(title_specificity_rank(&t.title)));
    for mut thread in threads.drain(..) {
        if is_excluded_im_package(&thread.package_name)
            || is_native_sms_package_or_thread(&thread.package_name, &thread.thread_id)
            || is_system_or_lockscreen_title(&thread.title)
        {
            continue;
        }
        if thread.app_name.trim().is_empty()
            || thread.app_name == thread.package_name
            || thread.app_name == "更多社交"
            || thread.app_name == "社媒私信"
        {
            if let Some(name) = mainstream_message_app_name(&thread.package_name) {
                thread.app_name = name.to_string();
            } else if thread.app_name.trim().is_empty() {
                thread.app_name = ImChannel::from_package(&thread.package_name)
                    .label()
                    .to_string();
            }
        }
        deduplicate_thread_messages(&mut thread.messages, &thread.title);
        recompute_thread_summary(&mut thread);
        populate_thread_avatars(&mut thread);
        if let Some(existing) = consolidated
            .iter_mut()
            .find(|existing| threads_belong_to_same_conversation(existing, &thread))
        {
            merge_thread_preserving_metadata(existing, thread);
        } else {
            consolidated.push(thread);
        }
    }

    for thread in &mut consolidated {
        thread.messages.sort_by_key(|m| m.timestamp_ms);
        recompute_thread_summary(thread);
        populate_thread_avatars(thread);
    }

    consolidated.sort_by(|a, b| b.last_timestamp_ms.cmp(&a.last_timestamp_ms));
    *threads = consolidated;
}

pub fn merge_synced_window_into_snapshot(
    snapshot: &mut PushSnapshot,
    mut synced: ImThread,
) -> ImThread {
    deduplicate_thread_messages(&mut synced.messages, &synced.title);
    recompute_thread_summary(&mut synced);
    populate_thread_avatars(&mut synced);
    if let Some(existing) = snapshot
        .threads
        .iter_mut()
        .find(|existing| threads_belong_to_same_conversation(existing, &synced))
    {
        merge_thread_preserving_metadata(existing, synced);
        let merged = existing.clone();
        consolidate_im_threads(&mut snapshot.threads, &snapshot.notifications);
        snapshot
            .threads
            .iter()
            .find(|t| {
                t.thread_id == merged.thread_id || threads_belong_to_same_conversation(t, &merged)
            })
            .cloned()
            .unwrap_or(merged)
    } else {
        snapshot.threads.insert(0, synced.clone());
        consolidate_im_threads(&mut snapshot.threads, &snapshot.notifications);
        snapshot
            .threads
            .iter()
            .find(|t| {
                t.thread_id == synced.thread_id || threads_belong_to_same_conversation(t, &synced)
            })
            .cloned()
            .unwrap_or(synced)
    }
}

fn extract_dumpsys_extra_string(block: &str, key: &str) -> Option<String> {
    let lines: Vec<&str> = block.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let rest_opt = trimmed.strip_prefix(key).or_else(|| {
            let prefix = trimmed.get(..key.len())?;
            if prefix.eq_ignore_ascii_case(key) {
                trimmed.get(key.len()..)
            } else {
                None
            }
        });
        if let Some(rest) = rest_opt {
            let rest = rest.trim_start();
            if !rest.starts_with('=') {
                continue;
            }
            let rest = rest.trim_start_matches('=').trim();
            if rest == "null" || rest.is_empty() {
                return None;
            }
            // Format: `String (StepX 通知自测群)` or multi-line `String (line1\nline2)`
            if let Some(open_paren) = rest.find('(') {
                let prefix = rest[..open_paren].trim();
                if matches!(
                    prefix,
                    "String" | "SpannableString" | "CharSequence" | "String[]"
                ) {
                    if rest.ends_with(')') {
                        let inner = &rest[open_paren + 1..rest.len() - 1];
                        if !inner.trim().is_empty() && inner.trim() != "null" {
                            return Some(inner.trim().to_string());
                        }
                    } else {
                        let mut buf = rest[open_paren + 1..].to_string();
                        for next_line in lines.iter().skip(idx + 1).take(12) {
                            let nt = next_line.trim();
                            if nt.ends_with(')') && !nt.contains("=(") {
                                buf.push('\n');
                                buf.push_str(&nt[..nt.len() - 1]);
                                break;
                            }
                            if nt.contains('=') && (nt.starts_with("android.") || nt == "}") {
                                break;
                            }
                            buf.push('\n');
                            buf.push_str(nt);
                        }
                        if !buf.trim().is_empty() && buf.trim() != "null" {
                            return Some(buf.trim().to_string());
                        }
                    }
                }
            }
            return Some(rest.trim_matches('"').trim().to_string());
        }
    }
    None
}

fn extract_bundle_field(bundle: &str, field_prefix: &str) -> Option<String> {
    let start = bundle.find(field_prefix)? + field_prefix.len();
    let rest = &bundle[start..];
    const DELIMITERS: [&str; 8] = [
        ", time=",
        ", sender=",
        ", text=",
        ", sender_person=",
        ", uri=",
        ", type=",
        ", extras=",
        ", is_important=",
    ];
    let end = DELIMITERS
        .iter()
        .filter_map(|delim| rest.find(delim))
        .min()
        .unwrap_or(rest.len());
    let val = rest[..end].trim().trim_end_matches(['}', ']']).trim();
    if val.is_empty() || val == "null" {
        None
    } else {
        Some(val.to_string())
    }
}

fn extract_messaging_style_bundles(
    chunk: &str,
    default_sender: &str,
    default_time_ms: i64,
    key: &str,
) -> Vec<ImMessage> {
    let self_display_name =
        extract_dumpsys_extra_string(chunk, "android.selfDisplayName").unwrap_or_default();
    let self_name_trimmed = self_display_name.trim();
    let mut out = Vec::new();
    let mut cursor = chunk;
    while let Some(b_start) = cursor.find("Bundle[{") {
        let after_open = &cursor[b_start + "Bundle[{".len()..];
        let Some(b_end) = after_open.find("}]") else {
            break;
        };
        let bundle = &after_open[..b_end];
        cursor = &after_open[b_end + 2..];

        if !(bundle.contains("text=")
            && (bundle.contains("sender=")
                || bundle.contains("time=")
                || bundle.contains("sender_person=")))
        {
            continue;
        }
        let Some(msg_txt) = extract_bundle_field(bundle, "text=") else {
            continue;
        };
        let raw_sender = extract_bundle_field(bundle, "sender=");
        let raw_sender_person = extract_bundle_field(bundle, "sender_person=");
        let is_self_reply = match raw_sender.as_deref().map(str::trim) {
            Some(s) if !s.is_empty() => {
                matches!(s, "我" | "Me" | "me" | "自己")
                    || (!self_name_trimmed.is_empty() && s == self_name_trimmed)
            }
            _ => raw_sender_person.is_none(),
        };
        let msg_sender = if is_self_reply {
            "我".to_string()
        } else {
            raw_sender.unwrap_or_else(|| default_sender.to_string())
        };
        let msg_time = extract_bundle_field(bundle, "time=")
            .and_then(|s| {
                s.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<i64>()
                    .ok()
            })
            .filter(|&t| t > 1_000_000_000_000)
            .unwrap_or(default_time_ms);
        let norm_txt = clean_message_display_text(&msg_txt);
        let media_hint = infer_media_hint_from_text(&norm_txt);
        out.push(ImMessage {
            id: format!("{}:{}:{}", key, msg_time, out.len()),
            sender: msg_sender,
            text: norm_txt,
            timestamp_ms: msg_time,
            observed_at_ms: msg_time.max(default_time_ms),
            time_known: Some(msg_time > 0),
            recalled: false,
            outgoing: is_self_reply,
            delivery_status: if is_self_reply {
                "sent".into()
            } else {
                "received".into()
            },
            via: "notification".into(),
            media_hint,
            client_msg_id: String::new(),
            ..Default::default()
        });
    }
    out
}

fn extract_inbox_text_lines(chunk: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_text_lines = false;
    for line in chunk.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("android.textLines=") {
            in_text_lines = true;
            if let Some(open_bracket) = rest.find('[') {
                if rest.ends_with(']') && !rest.contains("[]") {
                    let inner = &rest[open_bracket + 1..rest.len() - 1];
                    for part in inner.split(", ") {
                        let p = part.trim();
                        if !p.is_empty() && p != "null" {
                            out.push(p.to_string());
                        }
                    }
                    in_text_lines = false;
                }
            }
            continue;
        }
        if in_text_lines {
            if let Some(colon_pos) = trimmed.find("]: ") {
                if trimmed.starts_with('[')
                    && trimmed[1..colon_pos].chars().all(|c| c.is_ascii_digit())
                {
                    let line_txt = trimmed[colon_pos + 3..].trim();
                    if !line_txt.is_empty() && line_txt != "null" {
                        out.push(line_txt.to_string());
                    }
                    continue;
                }
            }
            if trimmed.starts_with("android.") || trimmed == "}" {
                in_text_lines = false;
            }
        }
    }
    out
}

/// Parses `adb shell dumpsys notification --noredact` output into active notifications
/// and enriched multi-message IM threads so full conversation information is available
/// even while the phone is locked (`锁屏状态`).
pub fn parse_dumpsys_notifications(raw: &str) -> (Vec<PushNotification>, Vec<ImThread>) {
    let mut notifications = Vec::new();
    let mut threads: Vec<ImThread> = Vec::new();

    const ARCHIVE_MARKERS: [&str; 6] = [
        "\n  mSummaryByGroupKey:",
        "\n  Snoozed notifications:",
        "\n  mArchive",
        "\n  Notification listeners:",
        "\n  RankingHelper:",
        "\n  GroupHelper:",
    ];
    let cut_pos = ARCHIVE_MARKERS
        .iter()
        .filter_map(|marker| raw.find(marker))
        .min()
        .unwrap_or(raw.len());
    let active_section = &raw[..cut_pos];

    for chunk in active_section.split("NotificationRecord(").skip(1) {
        if chunk.contains("isCanceled=true") || chunk.contains("mIsCanceled=true") {
            continue;
        }
        let header_line = chunk.lines().next().unwrap_or("");
        let pkg = header_line
            .split_whitespace()
            .find_map(|tok| tok.strip_prefix("pkg="))
            .unwrap_or("")
            .trim();
        if pkg.is_empty() || is_excluded_im_package(pkg) {
            continue;
        }
        let key = header_line
            .split_whitespace()
            .find_map(|tok| tok.strip_prefix("key="))
            .map(|k| k.trim_end_matches(':').trim().to_string())
            .or_else(|| extract_dumpsys_extra_string(chunk, "key"))
            .unwrap_or_default();

        let title = extract_dumpsys_extra_string(chunk, "android.conversationTitle")
            .or_else(|| extract_dumpsys_extra_string(chunk, "android.title"))
            .unwrap_or_default();
        let text = extract_dumpsys_extra_string(chunk, "android.bigText")
            .or_else(|| extract_dumpsys_extra_string(chunk, "android.text"))
            .unwrap_or_default();
        let sub_text = extract_dumpsys_extra_string(chunk, "android.subText").unwrap_or_default();
        let ticker_text = extract_dumpsys_extra_string(chunk, "tickerText").unwrap_or_default();

        let mut post_time_ms = chunk
            .lines()
            .find_map(|line| {
                let t = line.trim();
                for prefix in [
                    "postTime=",
                    "when=",
                    "creationTime=",
                    "mUpdateTimeMs=",
                    "lastAudibleAlertMillis=",
                ] {
                    if let Some(pos) = t.find(prefix) {
                        let digits: String = t[pos + prefix.len()..]
                            .chars()
                            .take_while(|c| c.is_ascii_digit())
                            .collect();
                        if let Ok(val) = digits.parse::<i64>() {
                            if val > 1_000_000_000_000 {
                                return Some(val);
                            }
                        }
                    }
                }
                None
            })
            .unwrap_or(0);

        let can_reply = chunk.contains("RemoteInput")
            || chunk.contains("android.app.RemoteInput")
            || chunk.contains("key_text_reply")
            || chunk.contains("remoteInputs=");
        let has_content_intent = chunk
            .lines()
            .any(|l| l.trim().starts_with("contentIntent=") && !l.trim().ends_with("null"));

        let clean_title = normalize_conversation_title(&title);
        if is_system_or_lockscreen_title(&clean_title) {
            continue;
        }
        let effective_raw = if !text.is_empty() {
            text.as_str()
        } else {
            ticker_text.as_str()
        };
        let last_line_raw = effective_raw
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or(effective_raw);
        let (sender_opt, body_clean) = split_sender_and_body(last_line_raw);
        let (ticker_sender, ticker_body) = split_sender_and_body(&ticker_text);

        let mut sender = sender_opt
            .or(ticker_sender)
            .unwrap_or_else(|| clean_title.clone());
        let mut final_body = if !body_clean.is_empty() {
            body_clean
        } else {
            ticker_body
        };

        let mut extracted_msgs =
            extract_messaging_style_bundles(chunk, &sender, post_time_ms, &key);

        if extracted_msgs.is_empty() {
            let inbox_lines = extract_inbox_text_lines(chunk);
            for (idx, raw_line) in inbox_lines.into_iter().enumerate() {
                let (line_sender, line_body) = split_sender_and_body(&raw_line);
                if !line_body.is_empty() {
                    let media_hint = infer_media_hint_from_text(&line_body);
                    extracted_msgs.push(ImMessage {
                        id: format!("{}:{}:{}", key, post_time_ms, idx),
                        sender: line_sender.unwrap_or_else(|| sender.clone()),
                        text: line_body,
                        timestamp_ms: post_time_ms,
                        observed_at_ms: post_time_ms,
                        time_known: Some(post_time_ms > 0),
                        recalled: false,
                        outgoing: false,
                        delivery_status: "received".into(),
                        via: "notification".into(),
                        media_hint,
                        client_msg_id: String::new(),
                        ..Default::default()
                    });
                }
            }
        }

        if extracted_msgs.is_empty() && text.lines().count() > 1 {
            for (idx, raw_line) in text.lines().enumerate() {
                let raw_line = raw_line.trim();
                if raw_line.is_empty() {
                    continue;
                }
                let (line_sender, line_body) = split_sender_and_body(raw_line);
                if !line_body.is_empty() {
                    let media_hint = infer_media_hint_from_text(&line_body);
                    extracted_msgs.push(ImMessage {
                        id: format!("{}:{}:{}", key, post_time_ms, idx),
                        sender: line_sender.unwrap_or_else(|| sender.clone()),
                        text: line_body,
                        timestamp_ms: post_time_ms,
                        observed_at_ms: post_time_ms,
                        time_known: Some(post_time_ms > 0),
                        recalled: false,
                        outgoing: false,
                        delivery_status: "received".into(),
                        via: "notification".into(),
                        media_hint,
                        client_msg_id: String::new(),
                        ..Default::default()
                    });
                }
            }
        }

        if let Some(last_extracted) = extracted_msgs.last() {
            if final_body.is_empty() || last_extracted.outgoing {
                final_body = last_extracted.text.clone();
                // The summary body came from this structured message, so its
                // speaker must come from the same record rather than the group title.
                sender = last_extracted.sender.clone();
            }
            if last_extracted.outgoing {
                sender = "我".to_string();
            } else if is_generic_sender_name(&sender, &clean_title)
                && !is_generic_sender_name(&last_extracted.sender, &clean_title)
            {
                sender = last_extracted.sender.clone();
            }
            if last_extracted.timestamp_ms > post_time_ms {
                post_time_ms = last_extracted.timestamp_ms;
            }
        }

        if title.is_empty() && final_body.is_empty() && extracted_msgs.is_empty() {
            continue;
        }

        if extracted_msgs.is_empty() && !final_body.is_empty() {
            let media_hint = infer_media_hint_from_text(&final_body);
            extracted_msgs.push(ImMessage {
                id: format!("{}:{}", key, post_time_ms),
                sender: sender.clone(),
                text: final_body.clone(),
                timestamp_ms: post_time_ms,
                observed_at_ms: post_time_ms,
                time_known: Some(post_time_ms > 0),
                recalled: false,
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                media_hint,
                client_msg_id: String::new(),
                ..Default::default()
            });
        }

        let ch = ImChannel::from_package(pkg);
        let notif_cat = PushCategory::from_notification(pkg, &title, &final_body, can_reply);
        let is_im_notification = !is_excluded_im_package(pkg)
            && !is_native_sms_package_or_thread(pkg, "")
            && (notif_cat == PushCategory::Im
                || is_career_or_recruitment_signal(pkg, "", &title, &final_body)
                || can_reply
                || chunk.contains("MessagingStyle")
                || chunk.contains("category=msg"));
        let app_name = mainstream_message_app_name(pkg)
            .unwrap_or_else(|| ch.label())
            .to_string();
        let thread_id = if is_im_notification {
            format!(
                "{}::{}",
                pkg,
                if clean_title.is_empty() {
                    &sender
                } else {
                    &clean_title
                }
            )
        } else {
            String::new()
        };
        let is_group = chunk.contains("android.isGroupConversation=Boolean (true)")
            || (!sender.is_empty()
                && sender != "我"
                && !clean_title.is_empty()
                && sender != clean_title)
            || clean_title.contains("群");

        let notif = PushNotification {
            key: key.clone(),
            seq_id: 0,
            package_name: pkg.to_string(),
            app_name: app_name.clone(),
            category: PushCategory::from_notification(pkg, &title, &final_body, can_reply)
                .code()
                .to_string(),
            title: if clean_title.is_empty() {
                title.clone()
            } else {
                clean_title.clone()
            },
            sender: sender.clone(),
            text: final_body.clone(),
            sub_text,
            post_time_ms,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group,
            dismissed: false,
            read: false,
            can_reply,
            thread_id: thread_id.clone(),
            otp_code: extract_otp_code(&format!("{title} {final_body}")),
            action_labels: Vec::new(),
            url_hint: extract_first_url(&final_body),
            ..Default::default()
        };
        if !should_hide_notification(&notif) {
            notifications.push(notif);
        }

        if !is_im_notification
            || is_excluded_im_package(pkg)
            || is_timer_or_countdown_notification(pkg, &app_name, &title, &final_body)
        {
            continue;
        }

        let thread = ImThread {
            thread_id,
            package_name: pkg.to_string(),
            app_name,
            title: if clean_title.is_empty() {
                sender.clone()
            } else {
                clean_title
            },
            is_group,
            unread_count: 1,
            last_sender: sender,
            last_message: final_body,
            last_timestamp_ms: post_time_ms,
            can_remote_reply: can_reply,
            can_ui_reply: true,
            has_content_intent,
            notification_key: key,
            messages: extracted_msgs,
            ..Default::default()
        };
        if let Some(existing) = threads
            .iter_mut()
            .find(|t| threads_belong_to_same_conversation(t, &thread))
        {
            merge_thread_preserving_metadata(existing, thread);
        } else {
            threads.push(thread);
        }
    }

    let im_notifs: Vec<PushNotification> = notifications
        .iter()
        .filter(|n| !n.thread_id.trim().is_empty())
        .cloned()
        .collect();
    consolidate_im_threads(&mut threads, &im_notifs);
    (notifications, threads)
}

pub fn should_hide_notification(n: &PushNotification) -> bool {
    if n.removed_time_ms > 0 || n.dismissed {
        return true;
    }
    if !n.active {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if extract_live_countdown_from_notification(n, now_ms).is_none() {
            return true;
        }
    }
    let pkg = n.package_name.trim();
    if pkg == "com.android.shell"
        || n.key.contains("stepx_e2e_test")
        || n.key.contains("stepx_test_tag")
    {
        return true;
    }
    let title = n.title.trim();
    let text = n.text.trim();
    let sub_text = n.sub_text.trim();
    if title.is_empty() && text.is_empty() && sub_text.is_empty() {
        return true;
    }
    if text.is_empty() && sub_text.is_empty() && pkg == "android" {
        return true;
    }
    if pkg == "android" || pkg == "com.android.systemui" {
        const LOW_VALUE_BANNERS: [&str; 6] = [
            "USB 文件传输",
            "正在通过 USB",
            "USB 调试",
            "无线调试",
            "选择输入法",
            "更改键盘",
        ];
        if LOW_VALUE_BANNERS
            .iter()
            .any(|kw| title.contains(kw) || text.contains(kw))
        {
            return true;
        }
    }
    false
}

pub fn is_bogus_captured_view_text(text: &str) -> bool {
    let t = text.trim();
    matches!(
        t,
        "ImageView"
            | "TextView"
            | "EditText"
            | "FrameLayout"
            | "LinearLayout"
            | "RelativeLayout"
            | "credential"
            | "转文字"
            | "重发"
            | "撤回"
            | "引用"
            | "多选"
            | "拍一拍"
            | "转发截图"
            | "系统桌面"
            | "输入密码"
            | "登录 Linux 电脑"
            | "登录 Windows 微信"
            | "登录 Mac 微信"
            | "登录 iPad 微信"
            | "Windows 微信已登录"
            | "Mac 微信已登录"
            | "Linux 微信已登录"
            | "iPad 微信已登录"
            | "你有1条消息未发送"
            | "联系人"
            | "最常使用"
            | "搜索指定内容"
            | "更多联系人"
            | "更多群聊"
            | "发消息"
            | "音视频通话"
    ) || t.starts_with("[\\u0001-")
        || t.starts_with("[\u{0001}-")
        || t.contains("\\u4E00-\\u9FA5")
        || t.starts_with("昵称:")
        || t.starts_with("昵称：")
        || t.starts_with("微信号:")
        || t.starts_with("微信号：")
        || (t.starts_with("你有") && t.ends_with("条消息未发送"))
}

pub fn is_scraped_ui_noise_thread(t: &ImThread) -> bool {
    let title = t.title.trim();
    let msg = t.last_message.trim();
    if is_excluded_im_package(&t.package_name)
        || is_system_or_lockscreen_title(title)
        || is_bogus_captured_view_text(title)
        || is_bogus_captured_view_text(msg)
    {
        return true;
    }
    if matches!(
        title,
        "搜索 账号/手机号"
            | "搜索小程序 搜索栏"
            | "最近"
            | "导入与导出"
            | "搜索"
            | "发现"
            | "通讯录"
            | "我"
            | "通过朋友验证"
            | "申请添加朋友"
            | "验证申请"
            | "新的朋友"
            | "添加朋友"
            | "朋友权限"
            | "设置备注和标签"
            | "详细资料"
            | "社交资料"
            | "更多信息"
            | "聊天信息"
            | "聊天详情"
            | "群聊信息"
            | "朋友圈"
            | "视频号"
            | "看一看"
            | "搜一搜"
            | "个人信息"
            | "设置"
    ) || title.starts_with("[语音]")
        || (title.starts_with("语音") && (title.contains("未播放") || title.contains('秒')))
    {
        return true;
    }
    let norm_title = normalize_conversation_title(title);
    let base_title = norm_title
        .split(['(', '（'])
        .next()
        .map(str::trim)
        .unwrap_or(norm_title.as_str());
    let is_app_level_title = matches!(
        base_title,
        "微信"
            | "WeChat"
            | "Weixin"
            | "QQ"
            | "TIM"
            | "钉钉"
            | "飞书"
            | "企业微信"
            | "消息"
            | "聊天"
            | "联系人"
            | "通讯录"
            | "发现"
            | "我"
    );
    if is_app_level_title
        && (t.unread_count == 0
            || !t.can_remote_reply
            || t.notification_key.is_empty()
            || t.notification_key.starts_with("ui_window::")
            || t.messages
                .iter()
                .any(|m| m.via.starts_with("ui_") || m.time_known == Some(false)))
    {
        return true;
    }
    if t.messages.iter().any(|m| {
        let txt = m.text.trim();
        matches!(
            txt,
            "不让他看我的朋友圈和状态"
                | "不看他的朋友圈和状态"
                | "聊天、朋友圈、微信运动等"
                | "朋友权限"
                | "设置备注和标签"
                | "转发截图"
                | "登录 Linux 电脑"
                | "你有1条消息未发送"
        ) || txt.starts_with("昵称:")
            || txt.starts_with("昵称：")
    }) {
        return true;
    }
    if is_generic_im_title(title)
        && msg.contains("登录")
        && (msg.contains("电脑")
            || msg.contains("Mac")
            || msg.contains("Windows")
            || msg.contains("iPad")
            || msg.contains("平板")
            || msg.contains("网页版"))
    {
        return true;
    }
    let all_ui_scraped = !t.messages.is_empty()
        && t.messages
            .iter()
            .all(|m| m.via.starts_with("ui_") || m.time_known == Some(false));
    (t.last_timestamp_ms <= 0 || all_ui_scraped)
        && t.unread_count == 0
        && is_generic_im_title(title)
}

pub fn sanitize_im_thread_messages(t: &mut ImThread) {
    t.title = clean_message_display_text(&t.title);
    t.last_message = clean_message_display_text(&t.last_message);
    t.last_sender = clean_message_display_text(&t.last_sender);
    for m in &mut t.messages {
        m.text = clean_message_display_text(&m.text);
        m.sender = clean_message_display_text(&m.sender);
        if !m.media_hint.trim().is_empty()
            && !m.text.trim_start().starts_with("[语音]")
            && m.text.trim().chars().count() > 28
        {
            m.media_hint.clear();
        }
    }
    t.messages
        .retain(|m| !is_bogus_captured_view_text(&m.text) && !m.text.trim().is_empty());
    if is_bogus_captured_view_text(&t.last_message) || t.last_message.trim().is_empty() {
        if let Some(last) = t.messages.last() {
            t.last_message = last.text.clone();
            if !last.sender.trim().is_empty() {
                t.last_sender = last.sender.clone();
            }
        } else {
            t.last_message.clear();
        }
    }
    populate_thread_avatars(t);
}

fn enrich_snapshot(mut snapshot: PushSnapshot, default_transport: &str) -> PushSnapshot {
    if snapshot.transport_mode.is_empty() {
        snapshot.transport_mode = default_transport.to_string();
    }
    for t in &mut snapshot.threads {
        sanitize_im_thread_messages(t);
    }
    snapshot.threads.retain(|t| {
        !is_excluded_im_package(&t.package_name)
            && !is_native_sms_package_or_thread(&t.package_name, &t.thread_id)
            && !is_system_or_lockscreen_title(&t.title)
            && !is_scraped_ui_noise_thread(t)
            && !is_timer_or_countdown_notification(
                &t.package_name,
                &t.app_name,
                &t.title,
                &t.last_message,
            )
            && !(t.last_message.trim().is_empty()
                && t.messages.iter().all(|m| m.text.trim().is_empty()))
            && !t.thread_id.contains("stepx_e2e_test")
            && !t.thread_id.contains("stepx_test_tag")
    });
    snapshot
        .notifications
        .retain(|n| !should_hide_notification(n));
    snapshot
        .notifications
        .sort_by(|a, b| b.post_time_ms.cmp(&a.post_time_ms));
    let mut seen_notifs = std::collections::HashSet::new();
    snapshot.notifications.retain(|n| {
        seen_notifs.insert((
            n.package_name.clone(),
            n.title.trim().to_string(),
            n.text.trim().to_string(),
        ))
    });
    consolidate_im_threads(&mut snapshot.threads, &snapshot.notifications);
    for notif in &mut snapshot.notifications {
        if let Some(thread) = snapshot.threads.iter().find(|t| {
            t.package_name == notif.package_name
                && ((!notif.key.is_empty() && t.notification_key == notif.key)
                    || (!notif.thread_id.is_empty() && t.thread_id == notif.thread_id)
                    || (is_generic_im_title(&notif.title)
                        && messages_match_content(&notif.text, &t.last_message)))
        }) {
            if title_specificity_rank(&thread.title) > title_specificity_rank(&notif.title) {
                notif.title = thread.title.clone();
                notif.thread_id = thread.thread_id.clone();
            }
            if is_generic_sender_name(&notif.sender, &notif.title)
                && !is_generic_sender_name(&thread.last_sender, &thread.title)
            {
                notif.sender = thread.last_sender.clone();
            }
        }
    }
    let sys_now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let max_snap_ts = snapshot
        .coverage
        .last_observed_ms
        .max(
            snapshot
                .notifications
                .iter()
                .map(|n| n.post_time_ms)
                .max()
                .unwrap_or(0),
        )
        .max(
            snapshot
                .otp_banners
                .iter()
                .map(|b| b.timestamp_ms)
                .max()
                .unwrap_or(0),
        );
    let ref_now_ms = if snapshot.coverage.last_observed_ms > 0 || max_snap_ts >= 1_750_000_000_000 {
        sys_now_ms.max(max_snap_ts)
    } else {
        max_snap_ts
    };
    snapshot.otp_banners.retain(|b| {
        if b.notification_key.contains("stepx_e2e_test")
            || b.notification_key.contains("stepx_test_tag")
            || b.is_expired(ref_now_ms)
        {
            return false;
        }
        if !b.notification_key.is_empty() {
            if let Some(n) = snapshot
                .notifications
                .iter()
                .find(|n| n.key == b.notification_key)
            {
                if !n.active || n.dismissed || n.read {
                    return false;
                }
            }
        }
        true
    });
    for notif in &mut snapshot.notifications {
        if notif.app_name.trim().is_empty()
            || notif.app_name == notif.package_name
            || notif.app_name == "更多社交"
            || notif.app_name == "社媒私信"
        {
            if let Some(name) = mainstream_message_app_name(&notif.package_name) {
                notif.app_name = name.to_string();
            }
        }
        if notif.category.trim().is_empty()
            || notif.category.eq_ignore_ascii_case("other")
            || (notif.category.eq_ignore_ascii_case("system")
                && is_mainstream_message_app(&notif.package_name))
        {
            notif.category = PushCategory::from_notification(
                &notif.package_name,
                &notif.title,
                &notif.text,
                notif.can_reply,
            )
            .code()
            .to_string();
        }
        if notif.otp_code.is_none() {
            let combined = format!("{} {}", notif.title, notif.text);
            notif.otp_code = extract_otp_code(&combined);
        }
        if notif.url_hint.is_none() {
            notif.url_hint =
                extract_first_url(&notif.text).or_else(|| extract_first_url(&notif.sub_text));
        }
    }
    for notif in &snapshot.notifications {
        if !notif.active || notif.dismissed || notif.read {
            continue;
        }
        if notif.post_time_ms > 0
            && ref_now_ms > notif.post_time_ms
            && (ref_now_ms - notif.post_time_ms) > OTP_TTL_MS
        {
            continue;
        }
        if let Some(code) = &notif.otp_code {
            let source_app = if notif.app_name.is_empty() {
                notif.package_name.clone()
            } else {
                notif.app_name.clone()
            };
            let already_exists = snapshot.otp_banners.iter().any(|b| {
                b.code == *code
                    && ((!notif.key.is_empty() && b.notification_key == notif.key)
                        || b.source_app == source_app
                        || b.source_app == notif.package_name)
            });
            if !already_exists {
                snapshot.otp_banners.push(OtpBanner {
                    code: code.clone(),
                    source_app,
                    title: notif.title.clone(),
                    raw_text: notif.text.clone(),
                    timestamp_ms: notif.post_time_ms,
                    notification_key: notif.key.clone(),
                });
            }
        }
    }
    snapshot
}

static LAST_STATUSBAR_FALLBACK: Mutex<Option<Instant>> = Mutex::new(None);

pub fn should_run_statusbar_fallback(snapshot: &PushSnapshot) -> bool {
    !snapshot.unchanged && !snapshot.listener_connected
}

pub fn enrich_threads_with_im_media(_adb: &Adb, _serial: &str, threads: &mut [ImThread]) {
    // Only media attached to the exact message by the phone provider is evidence.
    // Recent gallery files and screenshots have no conversation/sender binding.
    for thread in threads {
        populate_thread_avatars(thread);
    }
}

pub fn fetch_snapshot(adb: &Adb, serial: &str, since_revision: u64) -> Result<PushSnapshot> {
    let value = call_push_provider(
        adb,
        serial,
        json!({
            "op": "snapshot",
            "since_revision": since_revision,
            "sinceRevision": since_revision,
            "rescan": since_revision == 0,
        }),
    )?;
    let transport = value
        .get("transport_mode")
        .and_then(Value::as_str)
        .unwrap_or("adb");
    let target = value.get("snapshot").unwrap_or(&value);
    let mut snapshot: PushSnapshot =
        serde_json::from_value(target.clone()).context("无法解析手机推送快照数据")?;
    if value.get("unchanged").and_then(Value::as_bool) == Some(true)
        || target.get("unchanged").and_then(Value::as_bool) == Some(true)
    {
        snapshot.unchanged = true;
    }
    let needs_statusbar_fallback = should_run_statusbar_fallback(&snapshot);
    let allow_dumpsys_enrichment = needs_statusbar_fallback && {
        let mut guard = LAST_STATUSBAR_FALLBACK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let allowed = guard.is_none_or(|t| t.elapsed() >= Duration::from_secs(60));
        if allowed {
            *guard = Some(Instant::now());
        }
        allowed
    };
    if allow_dumpsys_enrichment {
        if let Ok(raw_dumpsys) = adb.shell(serial, &["dumpsys", "notification", "--noredact"]) {
            let (dump_notifs, dump_threads) = parse_dumpsys_notifications(&raw_dumpsys);
            for dn in dump_notifs {
                if !snapshot.notifications.iter().any(|n| n.key == dn.key) {
                    snapshot.notifications.push(dn);
                }
            }
            for mut dt in dump_threads {
                if let Some(existing) = snapshot
                    .threads
                    .iter_mut()
                    .find(|t| threads_belong_to_same_conversation(t, &dt))
                {
                    if existing.unread_count == 0
                        && dt.last_timestamp_ms <= existing.last_timestamp_ms
                    {
                        dt.unread_count = 0;
                    }
                    merge_thread_preserving_metadata(existing, dt);
                } else {
                    snapshot.threads.push(dt);
                }
            }
        }
    }
    let mut enriched = enrich_snapshot(snapshot, transport);
    if !enriched.unchanged {
        enrich_threads_with_im_media(adb, serial, &mut enriched.threads);
    }
    Ok(enriched)
}

pub fn fetch_thread_detail(adb: &Adb, serial: &str, thread_id: &str) -> Result<ImThread> {
    let value = call_push_provider(
        adb,
        serial,
        json!({
            "op": "thread_detail",
            "thread_id": thread_id,
            "threadId": thread_id,
        }),
    )?;
    let target = value.get("thread").unwrap_or(&value);
    let mut thread: ImThread =
        serde_json::from_value(target.clone()).context("无法解析会话详情")?;
    sanitize_im_thread_messages(&mut thread);
    Ok(thread)
}

fn invoke_reply_im_once(
    adb: &Adb,
    serial: &str,
    thread_id: &str,
    package_name: &str,
    title: &str,
    text: &str,
    mode_code: &str,
    client_msg_id: &str,
) -> Result<ReplyReceipt> {
    let value = call_push_provider(
        adb,
        serial,
        json!({
            "op": "reply_im",
            "thread_id": thread_id,
            "threadId": thread_id,
            "package_name": package_name,
            "packageName": package_name,
            "title": title,
            "text": text,
            "mode": mode_code,
            "force": mode_code == "ui_force",
            "client_msg_id": client_msg_id,
            "clientMsgId": client_msg_id,
        }),
    )?;
    let target = value.get("receipt").unwrap_or(&value);
    let mut receipt: ReplyReceipt =
        serde_json::from_value(target.clone()).context("无法解析回复回执")?;
    ensure!(
        receipt.ok,
        "{}",
        if receipt.message.is_empty() {
            "手机未确认回复成功"
        } else {
            &receipt.message
        }
    );
    if receipt.thread_id.is_empty() {
        receipt.thread_id = thread_id.to_string();
    }
    if receipt.client_msg_id.is_empty() {
        receipt.client_msg_id = client_msg_id.to_string();
    }
    if receipt.via.is_empty() && !receipt.channel.is_empty() {
        receipt.via = receipt.channel.clone();
    }
    if receipt.via.is_empty() {
        receipt.via = mode_code.to_string();
    }
    if receipt.channel.is_empty() {
        receipt.channel = receipt.via.clone();
    }
    if receipt.delivery_status.is_empty() && receipt.ok {
        // A successful dispatch is not a delivery/read receipt from the peer.
        receipt.delivery_status = "sent".to_string();
    }
    Ok(receipt)
}

fn reply_thread_matches_target(
    thread: &ImThread,
    package_name: &str,
    thread_id: &str,
    title: &str,
) -> bool {
    thread.package_name == package_name
        && (thread.thread_id == thread_id
            || (!is_generic_im_title(title)
                && normalize_conversation_title(&thread.title) == title))
}

fn ensure_specific_ui_reply_target(title: &str, notif_key: &str) -> Result<()> {
    if !notif_key.trim().is_empty() && !notif_key.starts_with("ui_window::") {
        return Ok(());
    }
    ensure!(
        !is_generic_im_title(title),
        "无法确认具体回复会话，消息未通过看屏通道发送；请打开明确的联系人或群聊后重试"
    );
    Ok(())
}

fn resolve_reply_target_from_snapshot(
    adb: &Adb,
    serial: &str,
    package_name: &str,
    resolved_thread_id: &mut String,
    resolved_title: &mut String,
    resolved_notif_key: &mut String,
) {
    let Ok(snap) = fetch_snapshot(adb, serial, 0) else {
        return;
    };
    if let Some(matched) = snap.threads.iter().find(|thread| {
        reply_thread_matches_target(thread, package_name, resolved_thread_id, resolved_title)
    }) {
        if title_specificity_rank(&matched.title) > title_specificity_rank(resolved_title) {
            *resolved_title = normalize_conversation_title(&matched.title);
            *resolved_thread_id = matched.thread_id.clone();
        }
        if !matched.notification_key.trim().is_empty()
            && !matched.notification_key.starts_with("ui_window::")
        {
            *resolved_notif_key = matched.notification_key.clone();
        }
    }
    if resolved_notif_key.is_empty() {
        if let Some(notif) = snap
            .notifications
            .iter()
            .find(|n| {
                n.active
                    && n.package_name == package_name
                    && (n.key == *resolved_thread_id
                        || n.thread_id == *resolved_thread_id
                        || format!("{}::{}", n.package_name.trim(), n.title.trim())
                            == *resolved_thread_id
                        || (!resolved_title.trim().is_empty()
                            && normalize_conversation_title(&n.title)
                                == normalize_conversation_title(resolved_title)))
            })
            .or_else(|| {
                snap.notifications
                    .iter()
                    .find(|n| n.active && n.package_name == package_name)
            })
        {
            *resolved_notif_key = notif.key.clone();
            if title_specificity_rank(&notif.title) > title_specificity_rank(resolved_title) {
                *resolved_title = normalize_conversation_title(&notif.title);
                *resolved_thread_id = format!("{}::{}", package_name, resolved_title);
            }
        }
    }
}

fn parse_foreground_component_from_dumpsys(raw: &str) -> Option<String> {
    let mut fallback_systemui: Option<String> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if !(trimmed.contains("mCurrentFocus=")
            || trimmed.contains("mFocusedApp=")
            || trimmed.contains("topResumedActivity=")
            || trimmed.contains("mResumedActivity=")
            || trimmed.contains("ResumedActivity:"))
        {
            continue;
        }
        for tok in trimmed.split_whitespace() {
            if tok.contains('/') {
                let cleaned = tok
                    .trim_start_matches(['{', '(', '[', '='])
                    .trim_end_matches(['}', ')', ']', ',', ';']);
                let Some(slash_pos) = cleaned.find('/') else {
                    continue;
                };
                let pkg = cleaned[..slash_pos].trim();
                let cls = cleaned[slash_pos + 1..].trim();
                if pkg.contains('.')
                    && !cls.is_empty()
                    && pkg
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
                    && cls
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '$')
                {
                    let comp = format!("{pkg}/{cls}");
                    if pkg == "com.android.systemui" {
                        if fallback_systemui.is_none() {
                            fallback_systemui = Some(comp);
                        }
                    } else {
                        return Some(comp);
                    }
                }
            }
        }
    }
    fallback_systemui
}

fn parse_foreground_package_from_dumpsys(raw: &str) -> Option<String> {
    if let Some(comp) = parse_foreground_component_from_dumpsys(raw) {
        if let Some((pkg, _)) = comp.split_once('/') {
            return Some(pkg.to_string());
        }
    }
    for line in raw.lines() {
        let trimmed = line.trim();
        if !(trimmed.contains("mCurrentFocus=")
            || trimmed.contains("mFocusedApp=")
            || trimmed.contains("topResumedActivity=")
            || trimmed.contains("mResumedActivity=")
            || trimmed.contains("ResumedActivity:"))
        {
            continue;
        }
        for tok in trimmed.split_whitespace() {
            if let Some(slash_idx) = tok.find('/') {
                let candidate = tok[..slash_idx]
                    .trim_start_matches(['{', '(', '[', '='])
                    .trim();
                if candidate.contains('.')
                    && candidate
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
                {
                    return Some(candidate.to_string());
                }
            }
        }
    }
    None
}

fn detect_foreground_package_and_component(
    adb: &Adb,
    serial: &str,
) -> (Option<String>, Option<String>) {
    if let Ok(out) = adb.shell(serial, &["dumpsys", "window", "displays"]) {
        if let Some(comp) = parse_foreground_component_from_dumpsys(&out) {
            let pkg = comp.split_once('/').map(|(p, _)| p.to_string());
            return (pkg, Some(comp));
        }
        if let Some(pkg) = parse_foreground_package_from_dumpsys(&out) {
            return (Some(pkg), None);
        }
    }
    if let Ok(out) = adb.shell(serial, &["dumpsys", "activity", "activities"]) {
        if let Some(comp) = parse_foreground_component_from_dumpsys(&out) {
            let pkg = comp.split_once('/').map(|(p, _)| p.to_string());
            return (pkg, Some(comp));
        }
        if let Some(pkg) = parse_foreground_package_from_dumpsys(&out) {
            return (Some(pkg), None);
        }
    }
    (None, None)
}

fn detect_foreground_package(adb: &Adb, serial: &str) -> Option<String> {
    detect_foreground_package_and_component(adb, serial).0
}

fn is_display_state_off_via_dumpsys(adb: &Adb, serial: &str) -> bool {
    adb.shell(
        serial,
        &[
            "sh",
            "-c",
            "dumpsys display 2>/dev/null | grep -m 1 'mState='",
        ],
    )
    .map(|out| out.contains("mState=OFF"))
    .unwrap_or(false)
}

fn is_power_awake_via_dumpsys(adb: &Adb, serial: &str) -> bool {
    let Ok(out) = adb.shell(serial, &["dumpsys", "power"]) else {
        return true;
    };
    out.contains("mWakefulness=Awake") || out.contains("Display Power: state=ON")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PreSendDeviceState {
    was_screen_interactive: bool,
    was_power_awake: bool,
    was_display_off: bool,
    prev_foreground_pkg: Option<String>,
    prev_foreground_component: Option<String>,
    prev_brightness: Option<String>,
    prev_brightness_mode: Option<String>,
}

fn clean_setting_int_value(raw: &str) -> Option<String> {
    let trimmed = raw.lines().next().unwrap_or("").trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("null")
        || !trimmed.chars().all(|c| c.is_ascii_digit())
    {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn capture_pre_send_device_state_and_dim_if_sleeping(
    adb: &Adb,
    serial: &str,
) -> PreSendDeviceState {
    let was_power_awake = is_power_awake_via_dumpsys(adb, serial);
    let was_display_off = is_display_state_off_via_dumpsys(adb, serial);
    let was_screen_interactive = was_power_awake && !was_display_off;
    let (prev_foreground_pkg, prev_foreground_component) =
        detect_foreground_package_and_component(adb, serial);
    let mut prev_brightness = None;
    let mut prev_brightness_mode = None;

    if !was_screen_interactive {
        if let Ok(b) = adb.shell(serial, &["settings", "get", "system", "screen_brightness"]) {
            prev_brightness = clean_setting_int_value(&b);
        }
        if let Ok(m) = adb.shell(
            serial,
            &["settings", "get", "system", "screen_brightness_mode"],
        ) {
            prev_brightness_mode = clean_setting_int_value(&m);
        }
        let _ = adb.shell(
            serial,
            &["settings", "put", "system", "screen_brightness_mode", "0"],
        );
        let _ = adb.shell(
            serial,
            &["settings", "put", "system", "screen_brightness", "1"],
        );
    }

    PreSendDeviceState {
        was_screen_interactive,
        was_power_awake,
        was_display_off,
        prev_foreground_pkg,
        prev_foreground_component,
        prev_brightness,
        prev_brightness_mode,
    }
}

fn is_launcher_or_home_package(pkg: &str) -> bool {
    let lower = pkg.trim().to_ascii_lowercase();
    lower.is_empty()
        || lower.contains("launcher")
        || lower.contains("home")
        || lower == "com.android.systemui"
}

fn launch_reply_app(adb: &Adb, serial: &str, package_name: &str) -> Result<()> {
    let device = crate::device::Device::inspect(adb, serial)?;
    let target = if package_name == "com.tencent.mm" {
        "com.tencent.mm/.ui.LauncherUI"
    } else {
        package_name
    };
    device
        .open_app(adb, target)
        .context("未能打开目标聊天应用；尚未继续 UI 回复")
}

fn restore_previous_background_state(
    adb: &Adb,
    serial: &str,
    pre_state: &PreSendDeviceState,
    target_package: &str,
) {
    if pre_state.prev_foreground_pkg.as_deref() != Some(target_package) {
        let can_restore_component = match (
            pre_state.prev_foreground_pkg.as_deref(),
            pre_state.prev_foreground_component.as_deref(),
        ) {
            (Some(pkg), Some(comp)) if !is_launcher_or_home_package(pkg) && comp.contains('/') => {
                Some(comp)
            }
            _ => None,
        };
        if let Some(comp) = can_restore_component {
            let started = adb
                .shell(
                    serial,
                    &["am", "start", "--activity-brought-to-front", "-n", comp],
                )
                .map(|out| !out.contains("Error:") && !out.contains("Exception"))
                .unwrap_or(false);
            if !started {
                let _ = adb.shell(serial, &["input", "keyevent", "KEYCODE_HOME"]);
            }
        } else {
            let _ = adb.shell(serial, &["input", "keyevent", "KEYCODE_HOME"]);
        }
    }

    if !pre_state.was_screen_interactive {
        if let Some(b) = pre_state.prev_brightness.as_deref() {
            let _ = adb.shell(
                serial,
                &["settings", "put", "system", "screen_brightness", b],
            );
        }
        if let Some(m) = pre_state.prev_brightness_mode.as_deref() {
            let _ = adb.shell(
                serial,
                &["settings", "put", "system", "screen_brightness_mode", m],
            );
        }
        if pre_state.was_display_off && pre_state.was_power_awake {
            let _ = adb.shell(
                serial,
                &[
                    "sh",
                    "-c",
                    "sleep 0.2; cmd display power-reset 0; sleep 0.15; cmd display power-off 0",
                ],
            );
        } else {
            let _ = adb.shell(serial, &["input", "keyevent", "223"]);
        }
    }
}

fn is_sms_package_or_thread(package_name: &str, thread_id: &str) -> bool {
    let pkg_lower = package_name.trim().to_ascii_lowercase();
    pkg_lower == "com.android.mms"
        || pkg_lower == "com.google.android.apps.messaging"
        || pkg_lower.contains("mms")
        || pkg_lower.contains("messaging")
        || thread_id.trim().starts_with("sms:")
}

fn sanitize_sms_number_candidate(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let compact: String = trimmed
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '(' | ')' | '\u{00a0}'))
        .collect();
    let digits = compact.strip_prefix('+').unwrap_or(compact.as_str());
    if (3..=24).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit()) {
        return Some(compact);
    }
    if let (Some(open), Some(close)) = (trimmed.rfind('('), trimmed.rfind(')')) {
        if open + 1 < close {
            if let Some(inner) = sanitize_sms_number_candidate(&trimmed[open + 1..close]) {
                return Some(inner);
            }
        }
    }
    None
}

fn extract_sms_recipient_number(
    thread_id: &str,
    resolved_thread_id: &str,
    resolved_title: &str,
    title: &str,
) -> Option<String> {
    let candidates = [
        thread_id.trim().strip_prefix("sms:"),
        resolved_thread_id.trim().strip_prefix("sms:"),
        thread_id.trim().rsplit_once("::").map(|(_, tail)| tail),
        resolved_thread_id
            .trim()
            .rsplit_once("::")
            .map(|(_, tail)| tail),
        Some(resolved_title.trim()),
        Some(title.trim()),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Some(num) = sanitize_sms_number_candidate(cand) {
            return Some(num);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn reply_im(
    adb: &Adb,
    serial: &str,
    thread_id: &str,
    package_name: &str,
    title: &str,
    text: &str,
    mode: ReplyMode,
    client_msg_id: &str,
) -> Result<ReplyReceipt> {
    let trimmed = text.trim();
    ensure!(!trimmed.is_empty(), "回复内容不能为空");
    let effective_client_msg_id = if client_msg_id.trim().is_empty() {
        format!(
            "stepx_reply_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        )
    } else {
        client_msg_id.trim().to_string()
    };

    let mut resolved_thread_id = thread_id.trim().to_string();
    let mut resolved_title = normalize_conversation_title(title);
    let mut resolved_notif_key = String::new();
    let mut snapshot_resolved = false;

    if is_generic_im_title(&resolved_title) {
        resolve_reply_target_from_snapshot(
            adb,
            serial,
            package_name,
            &mut resolved_thread_id,
            &mut resolved_title,
            &mut resolved_notif_key,
        );
        snapshot_resolved = true;
    }
    if resolved_title.is_empty() {
        resolved_title = title.trim().to_string();
    }

    // Attempt 1: Fast direct call (uses RemoteInput immediately even on lockscreen when available).
    // When mode == ReplyMode::Auto, pass "remote_only" so Java never wakes the screen or launches
    // the app before we record PreSendDeviceState!
    let first_mode = "remote_only";
    match invoke_reply_im_once(
        adb,
        serial,
        &resolved_thread_id,
        package_name,
        &resolved_title,
        trimmed,
        first_mode,
        &effective_client_msg_id,
    ) {
        Ok(mut receipt) => {
            receipt.thread_id = thread_id.to_string();
            return Ok(receipt);
        }
        Err(first_err) => {
            // No fallback may replay an operation after a lost acknowledgement.
            if first_err.downcast_ref::<UncertainPushWrite>().is_some() {
                return Err(first_err);
            }
            if !snapshot_resolved {
                resolve_reply_target_from_snapshot(
                    adb,
                    serial,
                    package_name,
                    &mut resolved_thread_id,
                    &mut resolved_title,
                    &mut resolved_notif_key,
                );
            }

            // Direct 100% silent background SMS reply via SmsManager bridge when RemoteInput is absent.
            if is_sms_package_or_thread(package_name, thread_id)
                || is_sms_package_or_thread(package_name, &resolved_thread_id)
            {
                if let Some(sms_number) = extract_sms_recipient_number(
                    thread_id,
                    &resolved_thread_id,
                    &resolved_title,
                    title,
                ) {
                    let device = crate::device::Device {
                        transport: serial.to_string(),
                        serial: String::new(),
                        ..Default::default()
                    };
                    if let Ok(plan) =
                        crate::bridge::sms_prepare(adb, &device, &sms_number, trimmed, None)
                    {
                        if let Ok(status) = crate::bridge::sms_send(adb, &device, &plan) {
                            if matches!(status.state.as_str(), "sent" | "sending") {
                                let now_ms = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis() as i64)
                                    .unwrap_or(0);
                                return Ok(ReplyReceipt {
                                    ok: true,
                                    thread_id: thread_id.to_string(),
                                    client_msg_id: effective_client_msg_id,
                                    via: "sms_bridge".to_string(),
                                    channel: "sms_bridge".to_string(),
                                    delivery_status: if status.state == "sent" {
                                        "sent"
                                    } else {
                                        "unknown"
                                    }
                                    .to_string(),
                                    message: status.summary_label(),
                                    timestamp_ms: status.updated_ms.unwrap_or(now_ms),
                                });
                            }
                        }
                    }
                }
            }

            if mode != ReplyMode::UiBridgeFallback {
                return Err(
                    first_err.context("后台回复未完成；未唤醒手机或切换 App，也未改发到其他会话")
                );
            }
            ensure_specific_ui_reply_target(&resolved_title, &resolved_notif_key)?;

            let pre_state = capture_pre_send_device_state_and_dim_if_sleeping(adb, serial);
            let was_already_foreground = pre_state.was_screen_interactive
                && pre_state.prev_foreground_pkg.as_deref() == Some(package_name);

            // Attempt 2: Wake & unlock lockscreen/AOD (dimmed when previously asleep), open target
            // conversation via notification contentIntent (`open_push`) or the shared MCP-first
            // app launcher, send via UI, and restore background state on both Ok and Err.
            let attempt2_result: Result<ReplyReceipt> = (|| {
                ensure_apex_bridge_ready(adb, serial);
                if !(pre_state.was_power_awake && pre_state.was_display_off) {
                    let _ = crate::device::wake_and_unlock_transport(adb, serial, 600, 2100, 350);
                }
                let open_key = if !resolved_notif_key.is_empty() {
                    resolved_notif_key.as_str()
                } else {
                    resolved_thread_id.as_str()
                };
                if !open_key.is_empty() {
                    let _ = call_push_provider(
                        adb,
                        serial,
                        json!({
                            "op": "open_push",
                            "key": open_key,
                            "thread_id": resolved_thread_id,
                            "threadId": resolved_thread_id,
                            "title": resolved_title,
                            "package_name": package_name,
                            "packageName": package_name,
                        }),
                    );
                    std::thread::sleep(Duration::from_millis(650));
                }

                if detect_foreground_package(adb, serial).as_deref() != Some(package_name) {
                    launch_reply_app(adb, serial, package_name)?;
                    std::thread::sleep(Duration::from_millis(900));
                }

                // A failed send must keep its original recipient. Never retry
                // as an arbitrary foreground chat or a generic “group chat”.
                let mut receipt = invoke_reply_im_once(
                    adb,
                    serial,
                    &resolved_thread_id,
                    package_name,
                    &resolved_title,
                    trimmed,
                    "ui_force",
                    &effective_client_msg_id,
                )?;
                receipt.thread_id = thread_id.to_string();
                Ok(receipt)
            })();

            if !was_already_foreground {
                restore_previous_background_state(adb, serial, &pre_state, package_name);
            }
            attempt2_result
        }
    }
}

pub fn sync_active_chat_window(adb: &Adb, serial: &str) -> Result<Option<ImThread>> {
    sync_active_chat_window_for(adb, serial, None, None)
}

pub fn sync_active_chat_window_for(
    adb: &Adb,
    serial: &str,
    package_name: Option<&str>,
    title: Option<&str>,
) -> Result<Option<ImThread>> {
    let mut payload = json!({ "op": "sync_window" });
    if let Some(pkg) = package_name.map(str::trim).filter(|s| !s.is_empty()) {
        payload["package_name"] = json!(pkg);
        payload["packageName"] = json!(pkg);
    }
    if let Some(t) = title.map(str::trim).filter(|s| !s.is_empty()) {
        payload["title"] = json!(t);
    }
    let raw_synced: Option<ImThread> = call_push_provider(adb, serial, payload)
        .ok()
        .and_then(|value| {
            if let Some(thread_val) = value.get("thread") {
                if thread_val.is_null() {
                    return None;
                }
                serde_json::from_value::<ImThread>(thread_val.clone()).ok()
            } else if value.get("thread_id").is_some() || value.get("threadId").is_some() {
                serde_json::from_value::<ImThread>(value).ok()
            } else {
                None
            }
        })
        .map(|mut thread| {
            sanitize_im_thread_messages(&mut thread);
            thread
        })
        .filter(|thread| {
            package_name
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .is_none_or(|pkg| thread.package_name == pkg)
                && title
                    .map(normalize_conversation_title)
                    .filter(|t| !t.is_empty())
                    .is_none_or(|target| normalize_conversation_title(&thread.title) == target)
                && (!thread.thread_id.is_empty() || !thread.title.is_empty())
                && !is_non_im_system_package(&thread.package_name)
                && !is_system_or_lockscreen_title(&thread.title)
        });

    let mut snap = fetch_snapshot(adb, serial, 0).unwrap_or_default();
    if let Some(synced) = raw_synced {
        let merged = merge_synced_window_into_snapshot(&mut snap, synced);
        return Ok(Some(merged));
    }

    // When the phone is locked (`锁屏状态`) or the foreground is a system/lockscreen surface,
    // return the requested IM conversation (or most recent active conversation) from PushStore / notifications.
    if let Some(pkg) = package_name.map(str::trim).filter(|s| !s.is_empty()) {
        let norm_target = title.map(normalize_conversation_title).unwrap_or_default();
        if let Some(matched) = snap.threads.iter().find(|t| {
            t.package_name == pkg
                && (norm_target.is_empty() || normalize_conversation_title(&t.title) == norm_target)
        }) {
            return Ok(Some(matched.clone()));
        }
    }
    Ok(None)
}

pub fn dismiss_notification(adb: &Adb, serial: &str, key: &str) -> Result<()> {
    ensure!(!key.trim().is_empty(), "通知标识不能为空");
    let _ = call_push_provider(
        adb,
        serial,
        json!({
            "op": "dismiss_push",
            "key": key,
        }),
    )?;
    Ok(())
}

pub fn dismiss_all_notifications(adb: &Adb, serial: &str, category: Option<&str>) -> Result<usize> {
    let value = call_push_provider(
        adb,
        serial,
        json!({
            "op": "dismiss_all_push",
            "category": category.unwrap_or("all"),
        }),
    )?;
    let count = value
        .get("cleared")
        .or_else(|| value.get("count"))
        .or_else(|| value.get("dismissed"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    Ok(count)
}

pub fn open_notification_on_phone(adb: &Adb, serial: &str, key: &str) -> Result<()> {
    ensure!(!key.trim().is_empty(), "通知标识不能为空");
    let _ = call_push_provider(
        adb,
        serial,
        json!({
            "op": "open_push",
            "key": key,
        }),
    )?;
    Ok(())
}

pub fn delete_im_thread(
    adb: &Adb,
    serial: &str,
    thread_id: &str,
    package_name: &str,
    title: &str,
) -> Result<()> {
    let _ = call_push_provider(
        adb,
        serial,
        json!({
            "op": "delete_thread",
            "thread_id": thread_id,
            "threadId": thread_id,
            "package_name": package_name,
            "packageName": package_name,
            "title": title,
        }),
    )?;
    Ok(())
}

pub fn mark_im_thread_read(
    adb: &Adb,
    serial: &str,
    thread_id: &str,
    package_name: &str,
    title: &str,
    read: bool,
) -> Result<()> {
    let _ = call_push_provider(
        adb,
        serial,
        json!({
            "op": "mark_thread_read",
            "thread_id": thread_id,
            "threadId": thread_id,
            "package_name": package_name,
            "packageName": package_name,
            "title": title,
            "read": read,
        }),
    )?;
    Ok(())
}

pub fn clear_im_thread_messages(
    adb: &Adb,
    serial: &str,
    thread_id: &str,
    package_name: &str,
    title: &str,
) -> Result<()> {
    let _ = call_push_provider(
        adb,
        serial,
        json!({
            "op": "clear_thread_messages",
            "thread_id": thread_id,
            "threadId": thread_id,
            "package_name": package_name,
            "packageName": package_name,
            "title": title,
        }),
    )?;
    Ok(())
}

pub fn delete_im_message(
    adb: &Adb,
    serial: &str,
    thread_id: &str,
    msg_id: &str,
    text: &str,
) -> Result<()> {
    let _ = call_push_provider(
        adb,
        serial,
        json!({
            "op": "delete_message",
            "thread_id": thread_id,
            "threadId": thread_id,
            "msg_id": msg_id,
            "msgId": msg_id,
            "text": text,
        }),
    )?;
    Ok(())
}

pub fn sync_im_conversation_list(
    adb: &Adb,
    serial: &str,
    package_name: Option<&str>,
) -> Result<()> {
    let mut payload = json!({ "op": "sync_im_list" });
    if let Some(pkg) = package_name.map(str::trim).filter(|s| !s.is_empty()) {
        payload["package_name"] = json!(pkg);
        payload["packageName"] = json!(pkg);
    }
    let _ = call_push_provider(adb, serial, payload)?;
    Ok(())
}

/// A plain @handle is not evidence that the current account was mentioned.
/// Only explicit notification markers supplied by the source app qualify.
/// Broadcast mentions deliberately remain ordinary messages.
pub fn extract_direct_at_mention(text: &str) -> Option<String> {
    let text = text.trim();
    if contains_broadcast_mention(text) {
        return None;
    }
    ["[有人@我]", "【有人@我】", "[@我]", "【@我】"]
        .iter()
        .any(|marker| text.starts_with(marker))
        .then(|| "@我".to_string())
}

fn contains_broadcast_mention(text: &str) -> bool {
    let folded = text.replace('＠', "@").to_lowercase();
    ["@所有人", "@全体", "@全员", "@all", "@everyone", "@here"]
        .iter()
        .any(|marker| folded.contains(marker))
}

pub fn notification_mentions_current_user(n: &PushNotification) -> bool {
    if contains_broadcast_mention(&n.text)
        || contains_broadcast_mention(&n.sub_text)
        || contains_broadcast_mention(&n.title)
    {
        return false;
    }
    extract_direct_at_mention(&n.text).is_some()
        || extract_direct_at_mention(&n.title).is_some()
        || extract_direct_at_mention(&n.sub_text).is_some()
        // Metadata can contain the bare marker; never trust the old desktop-
        // synthesized "有人@我 · 重要消息推送" label as independent evidence.
        || matches!(n.sub_text.trim(), "有人@我" | "有人提到了你")
}

pub fn thread_mentions_current_user(t: &ImThread, notifications: &[PushNotification]) -> bool {
    if t.unread_count == 0
        || t.messages.last().is_some_and(|m| m.outgoing)
        || contains_broadcast_mention(&t.last_message)
    {
        return false;
    }
    extract_direct_at_mention(&t.last_message).is_some()
        || notifications.iter().any(|n| {
            n.package_name == t.package_name
                && n.active
                && !n.read
                && !n.dismissed
                && n.post_time_ms >= t.last_timestamp_ms
                && ((!t.notification_key.is_empty() && n.key == t.notification_key)
                    || (!t.thread_id.is_empty() && n.thread_id == t.thread_id))
                && notification_mentions_current_user(n)
        })
}

/// Detects career or recruitment communications (`BOSS直聘`, `脉脉`, `猎聘`, `智联招聘`, `前程无忧`, `领英`, `58同城`, `鱼泡直聘`, `实习僧`).
pub fn is_career_or_recruitment_signal(pkg: &str, app_name: &str, title: &str, text: &str) -> bool {
    let p = pkg.trim().to_ascii_lowercase();
    if matches!(
        p.as_str(),
        "com.hpbr.bosszhipin"
            | "com.taou.maimai"
            | "com.lietou.mishu"
            | "com.lietpin.android"
            | "com.lietpin.android.lpactivity"
            | "com.zhaopin.social"
            | "com.job.android"
            | "com.linkedin.android"
            | "com.wuba"
            | "com.yupao.android"
            | "com.shixiseng.activity"
    ) {
        return true;
    }
    let a = app_name.trim();
    let t = title.trim();
    let x = text.trim();
    [
        "BOSS直聘",
        "Boss直聘",
        "脉脉",
        "猎聘",
        "智联招聘",
        "前程无忧",
        "领英",
        "鱼泡直聘",
        "实习僧",
    ]
    .iter()
    .any(|kw| a.contains(kw) || t.contains(kw))
        || [
            "面试邀约",
            "发来新消息",
            "查看了你的简历",
            "对你感兴趣",
            "交换微信",
            "发来一份简历",
            "牛人",
            "招聘",
            "HR",
            "岗位",
        ]
        .iter()
        .any(|kw| {
            (a.contains("直聘")
                || t.contains("直聘")
                || x.contains("直聘")
                || x.contains("简历")
                || x.contains("面试"))
                && (t.contains(kw) || x.contains(kw))
        })
}

/// Extracts a 4 to 8 digit verification code (OTP) when verification keywords are present.
pub fn extract_otp_code(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let keywords = [
        "验证码",
        "校验码",
        "动态码",
        "确认码",
        "安全码",
        "授权码",
        "认证码",
        "登录码",
        "提取码",
        "verification",
        "passcode",
        "security code",
        "auth code",
        "one-time",
        "otp",
        "code is",
        "code:",
    ];

    let mut keyword_end_byte: Option<usize> = None;
    for kw in keywords {
        let mut offset = 0usize;
        while let Some(rel) = lower[offset..].find(kw) {
            let idx = offset + rel;
            let end = idx + kw.len();
            let boundary_ok = if kw.is_ascii() {
                let prev_ok = lower[..idx]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !c.is_ascii_alphabetic() && c != '_');
                let next_ok = !kw.ends_with(|c: char| c.is_ascii_alphabetic())
                    || lower[end..]
                        .chars()
                        .next()
                        .is_none_or(|c| !c.is_ascii_alphabetic() && c != '_');
                prev_ok && next_ok
            } else {
                true
            };
            if boundary_ok {
                keyword_end_byte = Some(match keyword_end_byte {
                    Some(prev) => prev.min(end),
                    None => end,
                });
                break;
            }
            offset = end;
        }
    }
    let kw_end = keyword_end_byte?;

    let candidates = collect_digit_candidates(trimmed);
    if candidates.is_empty() {
        return None;
    }

    // Prefer the first valid candidate appearing after the keyword, otherwise the closest one before it.
    if let Some((_, code)) = candidates.iter().find(|(start, _)| *start >= kw_end) {
        return Some(code.clone());
    }
    candidates.last().map(|(_, code)| code.clone())
}

fn collect_digit_candidates(text: &str) -> Vec<(usize, String)> {
    let bytes = text.as_bytes();
    let mut results = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let len = i - start;
            if (4..=8).contains(&len) {
                let prev_char = text[..start].chars().next_back();
                let next_char = text[i..].chars().next();
                let after_str = text[i..].trim_start();

                let prev_is_decimal = prev_char == Some('.')
                    && text[..start.saturating_sub(1)]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_ascii_digit());
                let next_is_decimal = next_char == Some('.')
                    && text[i + 1..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_digit());
                let boundary_invalid = matches!(prev_char, Some('-' | '/' | ':'))
                    || matches!(next_char, Some('-' | '/' | ':'))
                    || prev_is_decimal
                    || next_is_decimal;
                let unit_invalid = [
                    "元", "分钟", "秒", "小时", "年", "月", "日", "条", "人", "个", "%", "MB", "GB",
                ]
                .iter()
                .any(|unit| after_str.starts_with(unit));
                let tail_number_context = text[..start].trim_end().ends_with("尾号")
                    || text[..start].trim_end().ends_with("卡号")
                    || text[..start].trim_end().ends_with("账户");

                if !boundary_invalid && !unit_invalid && !tail_number_context {
                    results.push((start, text[start..i].to_string()));
                }
            }
        } else {
            i += 1;
        }
    }
    results
}

/// Extracts the first `https://` or `http://` URL from notification or chat text.
pub fn extract_first_url(text: &str) -> Option<String> {
    for scheme in ["https://", "http://"] {
        if let Some(start) = text.find(scheme) {
            let rest = &text[start..];
            let end = rest
                .find(|c: char| {
                    c.is_whitespace()
                        || matches!(
                            c,
                            ')' | ']'
                                | '>'
                                | '"'
                                | '\''
                                | '，'
                                | '。'
                                | '；'
                                | '！'
                                | '？'
                                | '、'
                                | '”'
                                | '’'
                                | '】'
                                | '》'
                                | '（'
                                | '）'
                        )
                })
                .unwrap_or(rest.len());
            let candidate = rest[..end].trim_end_matches(['.', ',', ';', ':', '!']);
            if candidate.len() > scheme.len() + 3 && candidate[scheme.len()..].contains('.') {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

/// Generates a clean, structured summary of unread IM conversations, OTP codes, and important
/// notifications for AI 简洁模式.
pub fn summarize_unread_for_ai(snapshot: &PushSnapshot) -> String {
    let unread_threads: Vec<&ImThread> = snapshot
        .threads
        .iter()
        .filter(|t| t.unread_count > 0)
        .collect();
    let total_unread_msgs: usize = unread_threads.iter().map(|t| t.unread_count).sum();

    let active_important_notifs: Vec<&PushNotification> = snapshot
        .notifications
        .iter()
        .filter(|n| !n.dismissed && !n.is_ongoing && n.category != "im")
        .collect();

    if unread_threads.is_empty()
        && snapshot.otp_banners.is_empty()
        && active_important_notifs.is_empty()
    {
        return "目前手机没有未读聊天消息或待处理的验证码/重要推送。".to_string();
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "手机实时消息汇总（共 {} 个未读会话 / {} 条未读消息，{} 条验证码/取件码）：",
        unread_threads.len(),
        total_unread_msgs,
        snapshot.otp_banners.len()
    ));

    if !unread_threads.is_empty() {
        lines.push("\n【未读聊天会话】".to_string());
        for (idx, thread) in unread_threads.iter().take(6).enumerate() {
            let ch = thread.channel().label();
            let reply_tag = if thread.can_remote_reply {
                "可秒回"
            } else if thread.can_ui_reply {
                "可看屏回"
            } else {
                "只读"
            };
            let preview = if !thread.last_sender.is_empty()
                && thread.last_sender != thread.title
                && !thread.last_message.starts_with(&thread.last_sender)
            {
                format!("{}: {}", thread.last_sender, thread.last_message)
            } else {
                thread.last_message.clone()
            };
            lines.push(format!(
                "{}. [{}] {}（{}条未读 · {}）：{}",
                idx + 1,
                ch,
                thread.title,
                thread.unread_count,
                reply_tag,
                preview
            ));
        }
    }

    if !snapshot.otp_banners.is_empty() {
        lines.push("\n【最新验证码与取件码】".to_string());
        for banner in snapshot.otp_banners.iter().take(3) {
            lines.push(format!(
                "- [验证码] `{}`（来自 {} · {}）",
                banner.code, banner.source_app, banner.title
            ));
        }
    }

    if !active_important_notifs.is_empty() {
        lines.push("\n【重要应用推送】".to_string());
        for notif in active_important_notifs.iter().take(4) {
            let cat = PushCategory::from_str_code(&notif.category).label();
            let app = if notif.app_name.is_empty() {
                &notif.package_name
            } else {
                &notif.app_name
            };
            lines.push(format!(
                "- [{} · {}] {}：{}",
                app, cat, notif.title, notif.text
            ));
        }
    }

    lines.join("\n")
}

/// Generates a contextual quick reply draft based on the latest incoming message in `thread`.
pub fn generate_smart_reply_draft(thread: &ImThread) -> String {
    generate_contextual_reply_draft(thread, None, None, false)
}

/// Generates a contextual quick reply draft incorporating live calendar, Pomodoro, and DND state.
pub fn generate_contextual_reply_draft(
    thread: &ImThread,
    active_event_title: Option<&str>,
    pomodoro_remaining_mins: Option<u32>,
    dnd_on: bool,
) -> String {
    let latest_incoming = thread
        .messages
        .iter()
        .rev()
        .find(|m| !m.outgoing && !m.recalled && !m.text.trim().is_empty())
        .map(|m| m.text.as_str())
        .or_else(|| {
            let fallback = thread.last_message.trim();
            if fallback.is_empty()
                || thread.last_sender == "我"
                || fallback.starts_with("我:")
                || fallback.starts_with("我：")
                || fallback.contains("撤回了一条消息")
            {
                None
            } else {
                Some(fallback)
            }
        })
        .unwrap_or("");

    let active_event = active_event_title
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(12).collect::<String>());
    let pomo_mins = pomodoro_remaining_mins.filter(|&m| m > 0);

    let text = latest_incoming.trim();
    if text.is_empty() {
        if let Some(ev) = active_event {
            return format!("正在参加「{ev}」，稍后第一时间回复你。");
        }
        if let Some(mins) = pomo_mins {
            return format!("正在专注工作（约 {mins} 分钟后结束），稍后联系你。");
        }
        if dnd_on {
            return "当前免打扰中，稍后联系。".to_string();
        }
        return "收到，稍后联系。".to_string();
    }

    let lower = text.to_ascii_lowercase();
    let stripped_greeting = text.trim_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '?' | '？' | '!' | '！' | '~' | '～' | '.' | '。' | ',' | '，'
            )
    });
    let lower_greeting = stripped_greeting.to_ascii_lowercase();

    let base = if matches!(
        stripped_greeting,
        "在吗"
            | "在不在"
            | "有人吗"
            | "你好"
            | "您好"
            | "哈喽"
            | "嗨"
            | "早"
            | "早安"
            | "早上好"
            | "下午好"
            | "晚上好"
            | "你好，在吗"
            | "您好，在吗"
    ) || matches!(lower_greeting.as_str(), "hi" | "hello" | "hey")
    {
        "你好，在的，请说。".to_string()
    } else if text.contains("发版")
        || text.contains("上线")
        || text.contains("部署")
        || text.contains("验收")
    {
        "收到，我这边已经验收就绪，可以按计划推进发版。".to_string()
    } else if lower.contains("pr")
        || lower.contains("review")
        || text.contains("联调")
        || text.contains("接口")
        || text.contains("代码")
        || text.contains("文档")
        || text.contains("看下")
        || text.contains("看一下")
    {
        "收到，我马上看下详细内容，确认后第一时间回复你。".to_string()
    } else if text.contains("会议")
        || text.contains("评审")
        || text.contains("开会")
        || text.contains("会议室")
        || text.contains("周会")
    {
        if text.contains("几点")
            || text.contains("什么时候")
            || text.contains("有空")
            || text.contains("方便")
        {
            "我核对一下今天日程，确认好时间后马上回复你。".to_string()
        } else {
            "好的，已记下时间和地点，我准时参加。".to_string()
        }
    } else if text.contains("告警")
        || text.contains("故障")
        || text.contains("异常")
        || text.contains("延迟")
        || text.contains("报错")
        || text.contains("崩溃")
        || text.contains("闪退")
    {
        "收到告警，我立即排查一下当前节点指标，稍后同步进展。".to_string()
    } else if text.contains("试用") || text.contains("安装包") || text.contains("体验") {
        "好的，我今天整理好最新安装包与使用说明发给您体验。".to_string()
    } else if text.contains("客户")
        || text.contains("方案")
        || text.contains("报价")
        || text.contains("合同")
        || text.contains("资料")
    {
        "好的，我今天整理好最新方案与说明资料发给您确认。".to_string()
    } else if text.contains("快递")
        || text.contains("外卖")
        || text.contains("放门口")
        || text.contains("前台")
        || text.contains("驿站")
        || text.contains("取件")
        || text.contains("包裹")
    {
        "好的，谢谢提醒，我稍后去取。".to_string()
    } else if text.contains("到哪")
        || text.contains("到了吗")
        || text.contains("几分钟到")
        || text.contains("出发了吗")
    {
        "我这边已经在路上了，稍后就到。".to_string()
    } else if text.contains("吃饭")
        || text.contains("午饭")
        || text.contains("晚饭")
        || text.contains("聚餐")
        || text.contains("喝咖啡")
    {
        "好的，稍等我忙完手头这点事就过去。".to_string()
    } else if (text.contains("谢谢")
        || text.contains("感谢")
        || text.contains("多谢")
        || text.contains("辛苦"))
        && !text.contains('?')
        && !text.contains('？')
        && !text.contains("吗")
    {
        "不客气，有问题随时联系。".to_string()
    } else if text.contains("进展")
        || text.contains("进度")
        || text.contains("支持")
        || text.contains("机型")
        || text.contains("功能")
    {
        "收到，目前进展顺利，稍后把详细说明发给你。".to_string()
    } else if text.contains('?')
        || text.contains('？')
        || text.contains("吗")
        || text.contains("哪些")
        || text.contains("怎么")
        || text.contains("如何")
    {
        "收到，我确认一下具体情况，稍后回复你。".to_string()
    } else {
        "收到，好的，我马上处理。".to_string()
    };

    let trimmed = base.trim_end_matches('。');
    if let Some(ev) = active_event {
        format!("{trimmed}（当前正在「{ev}」开会，会后立即跟进）。")
    } else if let Some(mins) = pomo_mins {
        format!("{trimmed}（当前正在专注中，约 {mins} 分钟后处理）。")
    } else if dnd_on {
        format!("{trimmed}（当前免打扰中，稍后详细回复）。")
    } else {
        base
    }
}

/// Rich, realistic Chinese demo data for `--capture-ui`, previews, and offline testing.
pub fn demo_push_snapshot() -> PushSnapshot {
    let now_ms: i64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(1_761_238_800_000);

    let threads = vec![
        ImThread {
            thread_id: "com.tencent.mm::产品研发群".to_string(),
            package_name: "com.tencent.mm".to_string(),
            app_name: "微信".to_string(),
            title: "产品研发群".to_string(),
            is_group: true,
            unread_count: 3,
            last_sender: "王敏".to_string(),
            last_message: "@全体 今晚 8 点发版前大家把推送中心与小窗秒回验收一下，有问题吗？"
                .to_string(),
            last_timestamp_ms: now_ms - 45_000,
            can_remote_reply: true,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.tencent.mm|4097|prod_group|10231".to_string(),
            messages: vec![
                ImMessage {
                    id: "msg_wx_1".to_string(),
                    sender: "陈亮".to_string(),
                    text: "同 Wi-Fi LocalSocket 长连接实测往返延迟只有 9ms，熄屏锁屏下很稳。"
                        .to_string(),
                    timestamp_ms: now_ms - 240_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
                },
                ImMessage {
                    id: "msg_wx_2".to_string(),
                    sender: "我".to_string(),
                    text: "桌面三端原生通知和置顶悬浮小窗也已经联调通过。".to_string(),
                    timestamp_ms: now_ms - 180_000,
                    outgoing: true,
                    delivery_status: "sent".to_string(),
                    via: "remote_input".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
                },
                ImMessage {
                    id: "msg_wx_3".to_string(),
                    sender: "林工".to_string(),
                    text: "安卓端 RemoteInput 句柄缓存池已加上，支持连续追发多条回复。".to_string(),
                    timestamp_ms: now_ms - 95_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
                },
                ImMessage {
                    id: "msg_wx_4".to_string(),
                    sender: "王敏".to_string(),
                    text: "@全体 今晚 8 点发版前大家把推送中心与小窗秒回验收一下，有问题吗？"
                        .to_string(),
                    timestamp_ms: now_ms - 45_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        ImThread {
            thread_id: "com.tencent.mm::张三".to_string(),
            package_name: "com.tencent.mm".to_string(),
            app_name: "微信".to_string(),
            title: "张三".to_string(),
            is_group: false,
            unread_count: 1,
            last_sender: "张三".to_string(),
            last_message: "下午 3 点的架构评审会改到 B302 会议室了，记得带测试机过来".to_string(),
            last_timestamp_ms: now_ms - 120_000,
            can_remote_reply: true,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.tencent.mm|4098|zhangsan|10231".to_string(),
            messages: vec![
                ImMessage {
                    id: "msg_zs_1".to_string(),
                    sender: "张三".to_string(),
                    text: "无线调试自愈重连那块今天下午一起过一下？".to_string(),
                    timestamp_ms: now_ms - 600_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
                },
                ImMessage {
                    id: "msg_zs_2".to_string(),
                    sender: "我".to_string(),
                    text: "好，文档我已经整理好了。".to_string(),
                    timestamp_ms: now_ms - 520_000,
                    outgoing: true,
                    delivery_status: "sent".to_string(),
                    via: "remote_input".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
                },
                ImMessage {
                    id: "msg_zs_3".to_string(),
                    sender: "张三".to_string(),
                    text: "下午 3 点的架构评审会改到 B302 会议室了，记得带测试机过来".to_string(),
                    timestamp_ms: now_ms - 120_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        ImThread {
            thread_id: "com.ss.android.lark::李四".to_string(),
            package_name: "com.ss.android.lark".to_string(),
            app_name: "飞书".to_string(),
            title: "李四".to_string(),
            is_group: false,
            unread_count: 2,
            last_sender: "李四".to_string(),
            last_message:
                "接口联调通过了，辛苦看下 PR #42：https://github.com/stepx/companion/pull/42"
                    .to_string(),
            last_timestamp_ms: now_ms - 195_000,
            can_remote_reply: true,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.ss.android.lark|2001|lisi|10245".to_string(),
            messages: vec![
                ImMessage {
                    id: "msg_ls_1".to_string(),
                    sender: "李四".to_string(),
                    text: "我把全量推送的 6 大智能分类和 OTP 提取器合进主分支了".to_string(),
                    timestamp_ms: now_ms - 260_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
                },
                ImMessage {
                    id: "msg_ls_2".to_string(),
                    sender: "李四".to_string(),
                    text:
                        "接口联调通过了，辛苦看下 PR #42：https://github.com/stepx/companion/pull/42"
                            .to_string(),
                    timestamp_ms: now_ms - 195_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        ImThread {
            thread_id: "com.alibaba.android.rimet::运维告警群".to_string(),
            package_name: "com.alibaba.android.rimet".to_string(),
            app_name: "钉钉".to_string(),
            title: "运维告警群".to_string(),
            is_group: true,
            unread_count: 1,
            last_sender: "告警机器人".to_string(),
            last_message:
                "[P2 紧急] 华东节点推送网关 P99 延迟升至 380ms，请值班同学确认".to_string(),
            last_timestamp_ms: now_ms - 310_000,
            can_remote_reply: true,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.alibaba.android.rimet|3011|ops_alert|10250".to_string(),
            messages: vec![ImMessage {
                id: "msg_dd_1".to_string(),
                sender: "告警机器人".to_string(),
                text: "[P2 紧急] 华东节点推送网关 P99 延迟升至 380ms，请值班同学确认".to_string(),
                timestamp_ms: now_ms - 310_000,
                outgoing: false,
                delivery_status: "received".to_string(),
                via: "notification".to_string(),
                media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
            }],
            ..Default::default()
        },
        ImThread {
            thread_id: "com.tencent.wework::客户王总".to_string(),
            package_name: "com.tencent.wework".to_string(),
            app_name: "企业微信".to_string(),
            title: "客户王总".to_string(),
            is_group: false,
            unread_count: 1,
            last_sender: "客户王总".to_string(),
            last_message: "这周能把新版桌面伴侣的试用包发我们团队体验一下吗？".to_string(),
            last_timestamp_ms: now_ms - 540_000,
            can_remote_reply: true,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.tencent.wework|5001|wangzong|10260".to_string(),
            messages: vec![ImMessage {
                id: "msg_wc_1".to_string(),
                sender: "客户王总".to_string(),
                text: "这周能把新版桌面伴侣的试用包发我们团队体验一下吗？".to_string(),
                timestamp_ms: now_ms - 540_000,
                outgoing: false,
                delivery_status: "received".to_string(),
                via: "notification".to_string(),
                media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
            }],
            ..Default::default()
        },
        ImThread {
            thread_id: "org.telegram.messenger::StepX OpenSource".to_string(),
            package_name: "org.telegram.messenger".to_string(),
            app_name: "Telegram".to_string(),
            title: "StepX OpenSource".to_string(),
            is_group: true,
            unread_count: 0,
            last_sender: "Alex".to_string(),
            last_message: "v0.2.3 cross-platform desktop notification center works great on Linux!"
                .to_string(),
            last_timestamp_ms: now_ms - 900_000,
            can_remote_reply: true,
            can_ui_reply: false,
            has_content_intent: true,
            notification_key: "0|org.telegram.messenger|6001|stepx_tg|10270".to_string(),
            messages: vec![ImMessage {
                id: "msg_tg_1".to_string(),
                sender: "Alex".to_string(),
                text: "v0.2.3 cross-platform desktop notification center works great on Linux!"
                    .to_string(),
                timestamp_ms: now_ms - 900_000,
                outgoing: false,
                delivery_status: "received".to_string(),
                via: "notification".to_string(),
                media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
            }],
            ..Default::default()
        },
        ImThread {
            thread_id: "com.xingin.xhs::数码测评君".to_string(),
            package_name: "com.xingin.xhs".to_string(),
            app_name: "小红书".to_string(),
            title: "数码测评君".to_string(),
            is_group: false,
            unread_count: 1,
            last_sender: "数码测评君".to_string(),
            last_message: "你好，看到你们做的同 Wi-Fi 锁屏秒回微信功能很酷，支持哪些安卓机型呀？"
                .to_string(),
            last_timestamp_ms: now_ms - 1_200_000,
            can_remote_reply: false,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.xingin.xhs|7001|xhs_dm|10280".to_string(),
            messages: vec![ImMessage {
                id: "msg_xhs_1".to_string(),
                sender: "数码测评君".to_string(),
                text: "你好，看到你们做的同 Wi-Fi 锁屏秒回微信功能很酷，支持哪些安卓机型呀？"
                    .to_string(),
                timestamp_ms: now_ms - 1_200_000,
                outgoing: false,
                delivery_status: "received".to_string(),
                via: "notification".to_string(),
                media_hint: String::new(),
                    client_msg_id: String::new(),
                    observed_at_ms: 0,
                    time_known: None,
                    recalled: false,
                    ..Default::default()
            }],
            ..Default::default()
        },
    ];

    let notifications = vec![
        PushNotification {
            key: "0|com.tencent.mm|4097|prod_group|10231".to_string(),
            seq_id: 108,
            package_name: "com.tencent.mm".to_string(),
            app_name: "微信".to_string(),
            category: "im".to_string(),
            title: "产品研发群".to_string(),
            sender: "王敏".to_string(),
            text: "@全体 今晚 8 点发版前大家把推送中心与小窗秒回验收一下，有问题吗？".to_string(),
            sub_text: "3条未读消息".to_string(),
            post_time_ms: now_ms - 45_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: true,
            dismissed: false,
            read: false,
            can_reply: true,
            thread_id: "com.tencent.mm::产品研发群".to_string(),
            otp_code: None,
            action_labels: vec!["回复".to_string(), "标为已读".to_string()],
            url_hint: None,
            ..Default::default()
        },
        PushNotification {
            key: "0|com.google.android.apps.messaging|8001|cmb_otp|10090".to_string(),
            seq_id: 107,
            package_name: "com.google.android.apps.messaging".to_string(),
            app_name: "短信".to_string(),
            category: "system".to_string(),
            title: "招商银行 (1069800095555)".to_string(),
            sender: "1069800095555".to_string(),
            text: "【招商银行】验证码 839201，5分钟内有效，用于登录掌上生活，请勿泄露。"
                .to_string(),
            sub_text: "刚刚 · 短信验证码".to_string(),
            post_time_ms: now_ms - 60_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: true,
            thread_id: "com.google.android.apps.messaging::1069800095555".to_string(),
            otp_code: Some("839201".to_string()),
            action_labels: vec!["复制验证码".to_string(), "回复".to_string()],
            url_hint: None,
            ..Default::default()
        },
        PushNotification {
            key: "0|com.tencent.mm|4098|zhangsan|10231".to_string(),
            seq_id: 106,
            package_name: "com.tencent.mm".to_string(),
            app_name: "微信".to_string(),
            category: "im".to_string(),
            title: "张三".to_string(),
            sender: "张三".to_string(),
            text: "下午 3 点的架构评审会改到 B302 会议室了，记得带测试机过来".to_string(),
            sub_text: String::new(),
            post_time_ms: now_ms - 120_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: true,
            thread_id: "com.tencent.mm::张三".to_string(),
            otp_code: None,
            action_labels: vec!["回复".to_string()],
            url_hint: None,
            ..Default::default()
        },
        PushNotification {
            key: "0|com.ss.android.lark|2001|lisi|10245".to_string(),
            seq_id: 105,
            package_name: "com.ss.android.lark".to_string(),
            app_name: "飞书".to_string(),
            category: "im".to_string(),
            title: "李四".to_string(),
            sender: "李四".to_string(),
            text: "接口联调通过了，辛苦看下 PR #42：https://github.com/stepx/companion/pull/42"
                .to_string(),
            sub_text: "研发协作".to_string(),
            post_time_ms: now_ms - 195_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: true,
            thread_id: "com.ss.android.lark::李四".to_string(),
            otp_code: None,
            action_labels: vec!["回复".to_string()],
            url_hint: Some("https://github.com/stepx/companion/pull/42".to_string()),
            ..Default::default()
        },
        PushNotification {
            key: "0|com.alibaba.android.rimet|3011|ops_alert|10250".to_string(),
            seq_id: 104,
            package_name: "com.alibaba.android.rimet".to_string(),
            app_name: "钉钉".to_string(),
            category: "im".to_string(),
            title: "运维告警群".to_string(),
            sender: "告警机器人".to_string(),
            text: "[P2 紧急] 华东节点推送网关 P99 延迟升至 380ms，请值班同学确认".to_string(),
            sub_text: "生产环境监控".to_string(),
            post_time_ms: now_ms - 310_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: true,
            dismissed: false,
            read: false,
            can_reply: true,
            thread_id: "com.alibaba.android.rimet::运维告警群".to_string(),
            otp_code: None,
            action_labels: vec!["回复".to_string()],
            url_hint: None,
            ..Default::default()
        },
        PushNotification {
            key: "0|com.cainiao.wireless|9001|pickup|10290".to_string(),
            seq_id: 103,
            package_name: "com.cainiao.wireless".to_string(),
            app_name: "菜鸟".to_string(),
            category: "delivery_travel".to_string(),
            title: "包裹到站待取提醒".to_string(),
            sender: "菜鸟驿站".to_string(),
            text: "您的顺丰包裹已到达博彦科技园菜鸟驿站，凭取件码 4-2-1088 及时领取。".to_string(),
            sub_text: "物流出行".to_string(),
            post_time_ms: now_ms - 420_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: false,
            thread_id: String::new(),
            otp_code: Some("4-2-1088".to_string()),
            action_labels: vec!["查看取件码".to_string()],
            url_hint: None,
            ..Default::default()
        },
        PushNotification {
            key: "0|com.tencent.wework|5001|wangzong|10260".to_string(),
            seq_id: 102,
            package_name: "com.tencent.wework".to_string(),
            app_name: "企业微信".to_string(),
            category: "im".to_string(),
            title: "客户王总".to_string(),
            sender: "客户王总".to_string(),
            text: "这周能把新版桌面伴侣的试用包发我们团队体验一下吗？".to_string(),
            sub_text: "外部联系人".to_string(),
            post_time_ms: now_ms - 540_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: true,
            thread_id: "com.tencent.wework::客户王总".to_string(),
            otp_code: None,
            action_labels: vec!["回复".to_string()],
            url_hint: None,
            ..Default::default()
        },
        PushNotification {
            key: "0|cmb.pb|9201|bill|10300".to_string(),
            seq_id: 101,
            package_name: "cmb.pb".to_string(),
            app_name: "招商银行".to_string(),
            category: "finance".to_string(),
            title: "信用卡消费提醒".to_string(),
            sender: "招商银行".to_string(),
            text: "您尾号 6219 的招行信用卡于 12:35 在【瑞幸咖啡】消费人民币 16.50 元。"
                .to_string(),
            sub_text: "交易与账单".to_string(),
            post_time_ms: now_ms - 780_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: false,
            thread_id: String::new(),
            otp_code: None,
            action_labels: vec!["查看账单".to_string()],
            url_hint: None,
            ..Default::default()
        },
        PushNotification {
            key: "0|com.xingin.xhs|7001|xhs_dm|10280".to_string(),
            seq_id: 100,
            package_name: "com.xingin.xhs".to_string(),
            app_name: "小红书".to_string(),
            category: "social".to_string(),
            title: "数码测评君 私信了你".to_string(),
            sender: "数码测评君".to_string(),
            text: "你好，看到你们做的同 Wi-Fi 锁屏秒回微信功能很酷，支持哪些安卓机型呀？"
                .to_string(),
            sub_text: "社交互动".to_string(),
            post_time_ms: now_ms - 1_200_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: false,
            thread_id: "com.xingin.xhs::数码测评君".to_string(),
            otp_code: None,
            action_labels: vec!["打开详情".to_string()],
            url_hint: None,
            ..Default::default()
        },
    ];

    let otp_banners = vec![
        OtpBanner {
            code: "839201".to_string(),
            source_app: "招商银行 · 短信".to_string(),
            title: "掌上生活登录验证码".to_string(),
            raw_text: "【招商银行】验证码 839201，5分钟内有效，用于登录掌上生活，请勿泄露。"
                .to_string(),
            timestamp_ms: now_ms - 60_000,
            notification_key: "0|com.google.android.apps.messaging|8001|cmb_otp|10090".to_string(),
        },
        OtpBanner {
            code: "4-2-1088".to_string(),
            source_app: "菜鸟驿站".to_string(),
            title: "博彦科技园驿站取件码".to_string(),
            raw_text: "您的顺丰包裹已到达博彦科技园菜鸟驿站，凭取件码 4-2-1088 及时领取。"
                .to_string(),
            timestamp_ms: now_ms - 420_000,
            notification_key: "0|com.cainiao.wireless|9001|pickup|10290".to_string(),
        },
    ];

    PushSnapshot {
        revision: 42,
        battery_state: None,
        live_call: Default::default(),
        coverage: PushCoverage::default(),
        unchanged: false,
        listener_connected: true,
        accessibility_connected: true,
        wifi_adb_enabled: true,
        screen_interactive: false,
        keyguard_locked: true,
        human_touch_recent: false,
        transport_mode: "同 Wi-Fi 极速流 (LocalSocket < 15ms)".to_string(),
        threads,
        notifications,
        otp_banners,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_recovery_never_replays_writes_or_ui_fallbacks() {
        for op in ["snapshot", "call_state", "status", "ping", "thread_detail", "phone_digest"] {
            assert!(request_is_read_only(&json!({"op":op})));
            assert!(!request_is_read_only(&json!({"op":op,"allow_ui_fallback":true})));
        }
        for op in ["reply_im", "open_push", "sync_window", "mark_read", "delete_thread", "set_clipboard", "unknown"] {
            assert!(!request_is_read_only(&json!({"op":op})));
        }
    }
    #[test]
    fn late_failed_snapshot_does_not_hide_a_newer_successful_data_read() {
        let target = "192.0.2.164:40999";
        let device = crate::device::Device { transport: target.into(), serial: "DATA164_HEALTH".into(), ..Default::default() };
        let old = Instant::now();
        observe_message_read(target, old, false);
        assert!(message_data_stalled(&device));
        observe_message_read(target, old, true);
        observe_message_read(target, old, false);
        assert!(!message_data_stalled(&device));
    }

    #[test]
    fn legacy_screen_capture_has_no_claimed_occurrence_time() {
        for via in ["ui_bridge", "ui_bridge_capture", "ui_capture"] {
            let m: ImMessage =
                serde_json::from_value(json!({"timestampMs": 12345, "via":via})).unwrap();
            assert_eq!(m.occurred_at_ms(), None);
        }
        let legacy_notification: ImMessage =
            serde_json::from_value(json!({"timestampMs":12345,"via":"notification"})).unwrap();
        assert_eq!(legacy_notification.occurred_at_ms(), Some(12345));
    }

    #[test]
    fn explicit_provenance_and_capture_coverage_survive_protocol_parse() {
        let m: ImMessage = serde_json::from_value(json!({"timestampMs":0,"observedAtMs":12345,"timeKnown":false,"via":"ui_capture","recalled":true})).unwrap();
        assert_eq!(m.observed_at_ms, 12345);
        assert!(m.recalled);
        assert_eq!(m.occurred_at_ms(), None);
        let sent: ImMessage =
            serde_json::from_value(json!({"timestampMs":12345,"timeKnown":true,"via":"ui_bridge"}))
                .unwrap();
        assert_eq!(sent.occurred_at_ms(), Some(12345));
        let snapshot: PushSnapshot = serde_json::from_value(json!({"coverage":{"observedSinceMs":100,"lastObservedMs":200,"lastGapMs":150,"historyTruncated":true,"persistenceError":true}})).unwrap();
        assert_eq!(snapshot.coverage.observed_since_ms, 100);
        assert_eq!(snapshot.coverage.last_gap_ms, 150);
        assert!(snapshot.coverage.history_truncated && snapshot.coverage.persistence_error);
    }

    #[test]
    fn extracts_chinese_and_english_otp_codes_accurately() {
        assert_eq!(
            extract_otp_code("【招商银行】验证码 839201，5分钟内有效，请勿泄露。"),
            Some("839201".to_string())
        );
        assert_eq!(
            extract_otp_code(
                "【招商银行】您尾号 6219 的储蓄卡正在登录，动态码：490218，请在 3 分钟内输入。"
            ),
            Some("490218".to_string())
        );
        assert_eq!(
            extract_otp_code("739104 is your GitHub verification code."),
            Some("739104".to_string())
        );
        assert_eq!(
            extract_otp_code("Your StepX OTP: 9812. Do not share it."),
            Some("9812".to_string())
        );
        // Normal bill or chat messages without OTP keywords must not false-trigger.
        assert_eq!(
            extract_otp_code("您尾号 6219 的信用卡消费人民币 1650 元"),
            None
        );
        assert_eq!(
            extract_otp_code("今晚 2026 年发版会议改到 14:30 开始"),
            None
        );
    }

    #[test]
    fn extracts_first_url_without_trailing_punctuation() {
        assert_eq!(
            extract_first_url(
                "接口联调通过了，辛苦看下 PR #42：https://github.com/stepx/companion/pull/42，谢谢！"
            ),
            Some("https://github.com/stepx/companion/pull/42".to_string())
        );
        assert_eq!(
            extract_first_url("点击链接入会 (https://meeting.feishu.cn/j/938271625)."),
            Some("https://meeting.feishu.cn/j/938271625".to_string())
        );
        assert_eq!(extract_first_url("普通聊天文本没有网址"), None);
    }

    #[test]
    fn classifies_im_channels_and_categories() {
        assert_eq!(ImChannel::from_package("com.tencent.mm"), ImChannel::WeChat);
        assert_eq!(
            ImChannel::from_package("com.tencent.wework"),
            ImChannel::WeCom
        );
        assert_eq!(
            ImChannel::from_package("com.alibaba.android.rimet"),
            ImChannel::DingTalk
        );
        assert_eq!(
            ImChannel::from_package("com.ss.android.lark"),
            ImChannel::Feishu
        );
        assert_eq!(
            ImChannel::from_package("com.tencent.mobileqq"),
            ImChannel::QqTim
        );
        assert_eq!(
            ImChannel::from_package("org.telegram.messenger"),
            ImChannel::GlobalIm
        );
        assert_eq!(
            ImChannel::from_package("com.xingin.xhs"),
            ImChannel::SocialDm
        );
        assert_eq!(
            ImChannel::from_package("com.google.android.apps.messaging"),
            ImChannel::Sms
        );

        assert!(ImChannel::All.matches_package("com.tencent.mm"));
        assert!(ImChannel::WeChat.matches_package("com.tencent.mm"));
        assert!(!ImChannel::WeChat.matches_package("com.ss.android.lark"));

        assert_eq!(PushCategory::from_str_code("im"), PushCategory::Im);
        assert_eq!(
            PushCategory::from_str_code("finance"),
            PushCategory::Finance
        );
        assert_eq!(
            PushCategory::from_str_code("delivery_travel"),
            PushCategory::DeliveryTravel
        );
        assert_eq!(
            PushCategory::from_str_code("system"),
            PushCategory::SystemSecurity
        );
        assert!(PushCategory::All.matches("finance"));
        assert!(PushCategory::Finance.matches("finance"));
        assert!(!PushCategory::Im.matches("finance"));
    }

    #[test]
    fn parses_camel_case_and_snake_case_push_snapshot_json() {
        let raw_json = json!({
            "revision": 99,
            "listenerConnected": true,
            "accessibilityConnected": false,
            "wifiAdbEnabled": true,
            "screenInteractive": false,
            "keyguardLocked": true,
            "humanTouchRecent": false,
            "transportMode": "local_socket",
            "threads": [{
                "threadId": "com.tencent.mm::测试群",
                "packageName": "com.tencent.mm",
                "appName": "微信",
                "title": "测试群",
                "isGroup": true,
                "unreadCount": 2,
                "lastSender": "小王",
                "lastMessage": "今晚发版准备好了吗？",
                "lastTimestampMs": 1700000000000i64,
                "canRemoteReply": true,
                "canUiReply": false,
                "hasContentIntent": true,
                "notificationKey": "k1",
                "messages": [{
                    "id": "m1",
                    "sender": "小王",
                    "text": "今晚发版准备好了吗？",
                    "timestampMs": 1700000000000i64,
                    "outgoing": false,
                    "deliveryStatus": "received",
                    "via": "notification"
                }]
            }],
            "notifications": [{
                "key": "k_otp",
                "seqId": 12,
                "packageName": "com.android.mms",
                "appName": "短信",
                "category": "system",
                "title": "招商银行",
                "sender": "95555",
                "text": "【招商银行】您的验证码 654321，请勿泄露。详情 https://cmbchina.com/info",
                "postTimeMs": 1700000001000i64,
                "isOngoing": false,
                "isClearable": true,
                "canReply": false
            }]
        });

        let parsed: PushSnapshot = serde_json::from_value(raw_json).expect("valid snapshot JSON");
        let enriched = enrich_snapshot(parsed, "local_socket");
        assert_eq!(enriched.revision, 99);
        assert!(enriched.listener_connected);
        assert_eq!(enriched.threads.len(), 1);
        assert_eq!(enriched.threads[0].title, "测试群");
        assert!(enriched.threads[0].can_remote_reply);
        assert_eq!(
            enriched.notifications[0].otp_code,
            Some("654321".to_string())
        );
        assert_eq!(
            enriched.notifications[0].url_hint,
            Some("https://cmbchina.com/info".to_string())
        );
        assert_eq!(enriched.otp_banners.len(), 1);
        assert_eq!(enriched.otp_banners[0].code, "654321");
    }

    #[test]
    fn generates_ai_summary_and_smart_reply_draft() {
        let demo = demo_push_snapshot();
        let summary = summarize_unread_for_ai(&demo);
        assert!(summary.contains("产品研发群"));
        assert!(summary.contains("张三"));
        assert!(summary.contains("839201"));
        assert!(summary.contains("4-2-1088"));

        let draft_release = generate_smart_reply_draft(&demo.threads[0]);
        assert!(draft_release.contains("发版") || draft_release.contains("验收"));

        let draft_pr = generate_smart_reply_draft(&demo.threads[2]);
        assert!(draft_pr.contains("看下") || draft_pr.contains("回复"));

        let client_non_wang = ImThread {
            thread_id: "com.tencent.mm::李客户".into(),
            title: "李客户".into(),
            last_sender: "李客户".into(),
            last_message: "麻烦发一下最新方案和报价".into(),
            ..Default::default()
        };
        let draft_client =
            generate_contextual_reply_draft(&client_non_wang, Some("架构评审会"), Some(15), true);
        assert!(!draft_client.contains("王总"));
        assert!(draft_client.contains("架构评审会"));
        let client_wang_proposal = ImThread {
            thread_id: "com.tencent.mm::王敏".into(),
            title: "王敏".into(),
            last_sender: "王敏".into(),
            last_message: "麻烦发一下最新方案和报价".into(),
            ..Default::default()
        };
        let draft_wang_proposal =
            generate_contextual_reply_draft(&client_wang_proposal, None, None, false);
        assert!(!draft_wang_proposal.contains("王总"));
        assert!(draft_wang_proposal.contains("方案"));

        let greeting_punct = ImThread {
            thread_id: "com.tencent.mm::同事A".into(),
            title: "同事A".into(),
            last_sender: "同事A".into(),
            last_message: "在吗?".into(),
            ..Default::default()
        };
        assert_eq!(
            generate_contextual_reply_draft(&greeting_punct, None, None, false),
            "你好，在的，请说。"
        );

        let general_question = ImThread {
            thread_id: "com.tencent.mm::朋友B".into(),
            title: "朋友B".into(),
            last_sender: "朋友B".into(),
            last_message: "这个配置参数怎么调整？".into(),
            ..Default::default()
        };
        assert_eq!(
            generate_contextual_reply_draft(&general_question, None, None, false),
            "收到，我确认一下具体情况，稍后回复你。"
        );

        let recalled_and_outgoing = ImThread {
            thread_id: "com.tencent.mm::同事C".into(),
            title: "同事C".into(),
            last_sender: "我".into(),
            last_message: "我: 好的".into(),
            messages: vec![
                ImMessage {
                    sender: "同事C".into(),
                    text: "在不在？".into(),
                    outgoing: false,
                    recalled: false,
                    ..Default::default()
                },
                ImMessage {
                    sender: "同事C".into(),
                    text: "撤回了一条消息".into(),
                    outgoing: false,
                    recalled: true,
                    ..Default::default()
                },
                ImMessage {
                    sender: "我".into(),
                    text: "好的".into(),
                    outgoing: true,
                    recalled: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            generate_contextual_reply_draft(&recalled_and_outgoing, None, None, false),
            "你好，在的，请说。"
        );
    }

    #[test]
    fn filters_and_deduplicates_notifications_and_non_im_threads() {
        let raw_json = json!({
            "revision": 100,
            "listenerConnected": true,
            "threads": [
                {
                    "threadId": "com.android.deskclock::闹钟",
                    "packageName": "com.android.deskclock",
                    "title": "闹钟提醒",
                    "lastMessage": "08:00 起床"
                },
                {
                    "threadId": "com.tencent.mm::empty",
                    "packageName": "com.tencent.mm",
                    "title": "空会话",
                    "lastMessage": "   ",
                    "messages": []
                },
                {
                    "threadId": "com.tencent.mm::stepx_e2e_test",
                    "packageName": "com.tencent.mm",
                    "title": "E2E",
                    "lastMessage": "test"
                },
                {
                    "threadId": "com.tencent.mm::real",
                    "packageName": "com.tencent.mm",
                    "title": "真实会话",
                    "lastMessage": "你好"
                }
            ],
            "notifications": [
                {
                    "key": "0|com.stepx.companion.app|1|a",
                    "packageName": "com.stepx.companion.app",
                    "title": "StepX 伴侣权限已就绪",
                    "text": "通知监听与无障碍已开启",
                    "postTimeMs": 1000,
                    "clearable": false,
                    "active": true
                },
                {
                    "key": "0|com.stepx.companion.app|2|b",
                    "packageName": "com.stepx.companion.app",
                    "title": "StepX 伴侣权限已就绪",
                    "text": "通知监听与无障碍已开启",
                    "postTimeMs": 3000,
                    "clearable": false,
                    "active": true
                },
                {
                    "key": "0|com.android.shell|99|stepx_e2e_test",
                    "packageName": "com.android.shell",
                    "title": "测试通知",
                    "text": "验证码 123456",
                    "postTimeMs": 4000,
                    "clearable": true,
                    "active": true
                },
                {
                    "key": "0|android|10|usb",
                    "packageName": "android",
                    "title": "USB 调试已连接",
                    "text": "轻触可停用 USB 调试",
                    "postTimeMs": 5000,
                    "clearable": false,
                    "active": true
                },
                {
                    "key": "0|com.tencent.mm|20|removed",
                    "packageName": "com.tencent.mm",
                    "title": "已移除",
                    "text": "旧消息",
                    "postTimeMs": 6000,
                    "clearable": true,
                    "active": false,
                    "removedTimeMs": 6001
                }
            ]
        });

        let parsed: PushSnapshot =
            serde_json::from_value(raw_json).expect("clearable + active must not collide");
        let enriched = enrich_snapshot(parsed, "local_socket");
        assert_eq!(enriched.threads.len(), 1);
        assert_eq!(enriched.threads[0].thread_id, "com.tencent.mm::real");
        assert_eq!(enriched.notifications.len(), 1);
        assert_eq!(enriched.notifications[0].post_time_ms, 3000);
        assert_eq!(enriched.notifications[0].title, "StepX 伴侣权限已就绪");
    }

    #[test]
    fn parse_foreground_component_from_dumpsys_extracts_exact_activity_and_skips_systemui_shade() {
        let dumpsys_window = r#"
  Window #1 Window{1a2b3c4 u0 NotificationShade}:
    mCurrentFocus=Window{1a2b3c4 u0 com.android.systemui/com.android.systemui.shade.NotificationShadeWindowView}
    mFocusedApp=ActivityRecord{5d6e7f8 u0 com.android.chrome/com.google.android.apps.chrome.Main t123}
"#;
        assert_eq!(
            parse_foreground_component_from_dumpsys(dumpsys_window).as_deref(),
            Some("com.android.chrome/com.google.android.apps.chrome.Main")
        );
        assert_eq!(
            parse_foreground_package_from_dumpsys(dumpsys_window).as_deref(),
            Some("com.android.chrome")
        );

        let dumpsys_activity = r#"
  topResumedActivity=ActivityRecord{9a8b7c6 u0 com.tencent.mm/.ui.LauncherUI t45}
"#;
        assert_eq!(
            parse_foreground_component_from_dumpsys(dumpsys_activity).as_deref(),
            Some("com.tencent.mm/.ui.LauncherUI")
        );
    }

    #[test]
    fn reply_target_never_uses_an_unrelated_window_or_a_sole_unmatched_thread() {
        for title in ["云啊", "我是照相机先生", "StepX 通知自测群"] {
            assert!(ensure_specific_ui_reply_target(title, "").is_ok());
        }
        for title in ["", "微信", "群聊"] {
            assert!(ensure_specific_ui_reply_target(title, "").is_err());
        }
        assert!(
            ensure_specific_ui_reply_target("小红书", "0|com.xingin.xhs|100|null|10200").is_ok()
        );
        let thread = ImThread {
            thread_id: "thread-specific".into(),
            package_name: "com.tencent.mm".into(),
            title: "明确的联系人".into(),
            ..Default::default()
        };
        assert!(!reply_thread_matches_target(&thread, "com.tencent.mm", "thread-unresolved", "微信"),
            "Even a sole cached thread cannot identify the recipient of an unrelated generic notice");
        assert!(!reply_thread_matches_target(
            &thread,
            "com.tencent.mobileqq",
            "thread-specific",
            "明确的联系人"
        ));
        assert!(reply_thread_matches_target(
            &thread,
            "com.tencent.mm",
            "thread-specific",
            "微信"
        ));
        assert!(reply_thread_matches_target(
            &thread,
            "com.tencent.mm",
            "alias",
            "明确的联系人"
        ));

        // SMS number extraction helper tests
        assert!(is_sms_package_or_thread(
            "com.android.mms",
            "com.android.mms::10086"
        ));
        assert!(is_sms_package_or_thread(
            "com.google.android.apps.messaging",
            "thread-1"
        ));
        assert!(is_sms_package_or_thread("com.unknown.app", "sms:10688123"));
        assert_eq!(
            extract_sms_recipient_number(
                "sms:106889900",
                "com.android.mms::服务号",
                "服务号",
                "服务号"
            )
            .as_deref(),
            Some("106889900")
        );
        assert_eq!(
            extract_sms_recipient_number(
                "com.android.mms::+8531888",
                "com.android.mms::+8531888",
                "+853 1888",
                "+853 1888"
            )
            .as_deref(),
            Some("+8531888")
        );
    }

    #[test]
    fn mainstream_apps_are_absorbed_into_threads_with_proper_app_names_and_otp_banners_merge() {
        let now_ms = chrono::Local::now().timestamp_millis();
        let raw_json = json!({
            "revision": 202,
            "listenerConnected": true,
            "otpBanners": [
                {
                    "code": "112233",
                    "sourceApp": "招商银行",
                    "title": "登录验证码",
                    "rawText": "您的验证码 112233",
                    "timestampMs": now_ms - 10_000,
                    "notificationKey": "0|cmb.pb|1|otp"
                }
            ],
            "threads": [],
            "notifications": [
                {
                    "key": "0|com.taobao.taobao|101|cs",
                    "packageName": "com.taobao.taobao",
                    "appName": "",
                    "category": "other",
                    "title": "旗舰店客服",
                    "text": "亲，您的包裹今天已经发出啦",
                    "postTimeMs": now_ms - 5_000,
                    "active": true
                },
                {
                    "key": "0|com.sankuai.meituan|102|rider",
                    "packageName": "com.sankuai.meituan",
                    "appName": "",
                    "category": "delivery_travel",
                    "title": "骑手张师傅",
                    "text": "外卖已放在前台外卖架，请及时取餐",
                    "postTimeMs": now_ms - 4_000,
                    "active": true
                },
                {
                    "key": "0|com.xingin.xhs|103|dm",
                    "packageName": "com.xingin.xhs",
                    "appName": "",
                    "category": "social",
                    "title": "摄影师阿林",
                    "text": "周末约拍机位发你啦",
                    "postTimeMs": now_ms - 3_000,
                    "active": true
                },
                {
                    "key": "0|com.eg.android.AlipayGphone|104|bill",
                    "packageName": "com.eg.android.AlipayGphone",
                    "appName": "",
                    "category": "finance",
                    "title": "付款成功通知",
                    "text": "您已向星巴克付款 32.00 元",
                    "postTimeMs": now_ms - 2_000,
                    "active": true
                },
                {
                    "key": "0|com.sf.activity|105|exp",
                    "packageName": "com.sf.activity",
                    "appName": "",
                    "category": "delivery_travel",
                    "title": "顺丰派件通知",
                    "text": "您的快件 SF1234567890 正在派送中",
                    "postTimeMs": now_ms - 1_500,
                    "active": true
                },
                {
                    "key": "0|com.netease.mobimail|106|mail",
                    "packageName": "com.netease.mobimail",
                    "appName": "",
                    "category": "im",
                    "title": "HR 面试确认函",
                    "text": "您的复试时间为明日下午 14:00，登录验证码 889900",
                    "postTimeMs": now_ms - 1_000,
                    "active": true
                }
            ]
        });

        let parsed: PushSnapshot = serde_json::from_value(raw_json).unwrap();
        let enriched = enrich_snapshot(parsed, "local_socket");

        for (pkg, expected_app_name) in [
            ("com.taobao.taobao", "淘宝"),
            ("com.sankuai.meituan", "美团"),
            ("com.xingin.xhs", "小红书"),
            ("com.eg.android.AlipayGphone", "支付宝"),
            ("com.sf.activity", "顺丰速运"),
            ("com.netease.mobimail", "网易邮箱大师"),
        ] {
            assert!(
                is_mainstream_message_app(pkg),
                "{pkg} must be in mainstream registry"
            );
            assert_eq!(mainstream_message_app_name(pkg), Some(expected_app_name));
            assert!(!is_excluded_im_package(pkg), "{pkg} must not be excluded");
            let thread = enriched
                .threads
                .iter()
                .find(|t| t.package_name == pkg)
                .unwrap_or_else(|| panic!("expected thread for {pkg}"));
            assert_eq!(thread.app_name, expected_app_name);
        }

        // Verify new OTP from notifications merged alongside existing otpBanners
        assert!(enriched.otp_banners.iter().any(|b| b.code == "112233"));
        assert!(enriched
            .otp_banners
            .iter()
            .any(|b| b.code == "889900" && b.source_app == "网易邮箱大师"));
    }
}

#[cfg(test)]
mod reply_transport_tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn lost_acknowledgement_never_replays_a_send_on_socket_provider_or_ui() {
        use std::io::BufRead;
        use std::os::unix::fs::PermissionsExt;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut request = String::new();
                        std::io::BufReader::new(stream)
                            .read_line(&mut request)
                            .unwrap();
                        // Simulate executing the send, then losing its response.
                        return serde_json::from_str::<Value>(&request).unwrap();
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    other => panic!("mock request was not received: {other:?}"),
                }
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("adb.log");
        let script = dir.path().join("adb");
        std::fs::write(&script, format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$*\" in *'forward tcp:0 localabstract:stepx_push_v1'*) echo {};; esac\n",log.display(),port)).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let result = reply_im(
            &Adb { path: script },
            "lost-acknowledgement-phone",
            "com.tencent.mm::Chosen recipient",
            "com.tencent.mm",
            "Chosen recipient",
            "single reply",
            ReplyMode::Auto,
            "lost-ack-id",
        );
        let error = result.unwrap_err();
        assert!(
            error.downcast_ref::<UncertainPushWrite>().is_some(),
            "{error:#}"
        );
        let request = server.join().unwrap();
        assert_eq!(request["client_msg_id"], "lost-ack-id");
        let commands = std::fs::read_to_string(log).unwrap();
        assert_eq!(
            commands
                .matches("forward tcp:0 localabstract:stepx_push_v1")
                .count(),
            1
        );
        assert!(
            !commands.contains("content call"),
            "A lost acknowledgement must not replay using content provider: {commands}"
        );
        assert!(
            !commands.contains("keyevent") && !commands.contains("monkey"),
            "Must not fall back to another UI send"
        );
    }

    #[test]
    fn sending_and_unknown_operations_never_retry_over_another_transport() {
        for op in [
            "reply_im",
            "open_notification",
            "dismiss_all",
            "future_write",
        ] {
            assert!(!replay_safe(&json!({"op":op})));
        }
        for op in ["snapshot", "call_state", "status", "ping", "thread_detail", "sync_window"] {
            assert!(replay_safe(&json!({"op":op})));
        }
    }

    #[test]
    fn write_operations_use_fast_local_socket_first() {
        for op in [
            "snapshot",
            "call_state",
            "status",
            "ping",
            "thread_detail",
            "sync_window",
            "reply_im",
            "dismiss_push",
            "dismiss_all_push",
            "open_push",
            "post_live_capsule",
            "dismiss_live_capsule",
            "clear_live_capsules",
        ] {
            assert!(socket_supported(&json!({"op": op})));
        }
        assert!(!socket_supported(&json!({"op": "unknown_op"})));
        assert_eq!(
            socket_read_timeout(&json!({"op": "reply_im"}), "STEPXTEST8DB89E4B8927"),
            Duration::from_millis(25000)
        );
        assert_eq!(
            socket_read_timeout(&json!({"op": "open_push"}), "STEPXTEST8DB89E4B8927"),
            Duration::from_millis(10000)
        );
        assert_eq!(
            socket_read_timeout(&json!({"op": "dismiss_push"}), "STEPXTEST8DB89E4B8927"),
            Duration::from_millis(1800)
        );
    }

    #[test]
    fn late_message_snapshot_cannot_resurrect_an_ended_call() {
        let previous = PushSnapshot { live_call: crate::phone::LiveCall {
            available: true, state: "idle".into(), observed_ms: 20_000, ..Default::default()
        }, ..Default::default() };
        let mut next = PushSnapshot { live_call: crate::phone::LiveCall {
            available: true, id: "ended-call".into(), state: "ringing".into(), observed_ms: 10_000,
            ..Default::default()
        }, ..Default::default() };
        merge_local_replies(&mut next, &previous);
        assert_eq!(next.live_call, previous.live_call);
        next.live_call = crate::phone::LiveCall {
            available: true, id: "new-call".into(), state: "ringing".into(), observed_ms: 30_000,
            ..Default::default()
        };
        merge_local_replies(&mut next, &previous);
        assert_eq!(next.live_call.id, "new-call");
    }

    #[cfg(unix)]
    #[test]
    fn failed_background_reply_never_opens_another_app_or_wakes_the_phone() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::os::unix::fs::PermissionsExt;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock push socket");
        let port = listener.local_addr().unwrap().port();
        let observed_ops = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let ops_clone = observed_ops.clone();

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_server = stop.clone();
        listener.set_nonblocking(true).unwrap();
        let server_handle = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            while !stop_server.load(std::sync::atomic::Ordering::Acquire)
                && Instant::now() < deadline
            {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let req: Value = serde_json::from_str(line.trim()).unwrap_or_default();
                let op = req["op"].as_str().unwrap_or("").to_string();
                let title = req["title"].as_str().unwrap_or("").to_string();
                let mode = req["mode"].as_str().unwrap_or("").to_string();
                ops_clone
                    .lock()
                    .unwrap()
                    .push(format!("{op}:{mode}:{title}"));

                let resp = match op.as_str() {
                    "reply_im" if mode == "auto" || mode == "remote_only" => json!({
                        "ok": false,
                        "error": "remote_input_unavailable_on_lockscreen"
                    }),
                    "snapshot" => json!({
                        "ok": true,
                        "snapshot": {
                            "revision": 7,
                            "threads": [{
                                "threadId": "com.tencent.mm::StepX 通知自测群",
                                "notificationKey": "0|com.tencent.mm|4097|null|10275",
                                "packageName": "com.tencent.mm",
                                "appName": "微信",
                                "title": "StepX 通知自测群",
                                "isGroup": true,
                                "lastSender": "邱-stepx",
                                "lastMessage": "发个测试",
                                "lastTimestampMs": 1775021164151_i64
                            }],
                            "notifications": []
                        }
                    }),
                    "open_push" => json!({ "ok": true }),
                    "sync_window" => json!({
                        "ok": true,
                        "thread": {
                            "threadId": "com.tencent.mm::群聊",
                            "packageName": "com.tencent.mm",
                            "appName": "微信",
                            "title": "群聊",
                            "isGroup": true,
                            "lastMessage": "发个测试"
                        }
                    }),
                    "reply_im" if mode == "ui_force" && title == "StepX 通知自测群" => json!({
                        "ok": false,
                        "error": "active_window_title_is_generic_group"
                    }),
                    "reply_im" if mode == "ui_force" && title == "群聊" => json!({
                        "ok": true,
                        "receipt": {
                            "ok": true,
                            "via": "ui_automation",
                            "threadId": "com.tencent.mm::群聊",
                            "clientMsgId": req["client_msg_id"].as_str().unwrap_or("client-lock-1")
                        }
                    }),
                    _ => json!({ "ok": true }),
                };

                let mut out = serde_json::to_vec(&resp).unwrap();
                out.push(b'\n');
                let _ = (&stream).write_all(&out);
            }
        });

        let dir = std::env::temp_dir().join(format!(
            "stepx_lock_reply_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let log_file = dir.join("adb.log");
        let adb_script = dir.join("fake_adb.sh");
        let script = format!(
            r#"#!/usr/bin/env bash
echo "$@" >> "{log}"
case "$*" in
  *"forward tcp:0 localabstract:stepx_push_v1"*)
    echo "{port}"
    exit 0
    ;;
  *"dumpsys notification --noredact"*)
    exit 0
    ;;
esac
if [ "$3" = "shell" ]; then
  shift 3
  SCRIPT="$*"
  dumpsys() {{
    if [ "$1" = "power" ]; then
      echo "mWakefulness=Dozing"
    elif [ "$1" = "window" ]; then
      echo "  KeyguardServiceDelegate"
      echo "    showing=true"
      echo "    showingAndNotOccluded=true"
      echo "    mShowingLockscreen=true"
    fi
  }}
  input() {{
    echo "input $*" >> "{log}"
  }}
  wm() {{
    echo "wm $*" >> "{log}"
  }}
  cmd() {{
    echo "cmd $*" >> "{log}"
  }}
  am() {{
    echo "am $*" >> "{log}"
  }}
  monkey() {{
    echo "monkey $*" >> "{log}"
  }}
  settings() {{
    if [ "$1" = "get" ] && [ "$3" = "screen_brightness" ]; then
      echo "128"
    elif [ "$1" = "get" ] && [ "$3" = "screen_brightness_mode" ]; then
      echo "1"
    else
      echo "settings $*" >> "{log}"
    fi
  }}
  sleep() {{
    :
  }}
  eval "$SCRIPT"
fi
exit 0
"#,
            log = log_file.display(),
            port = port
        );
        std::fs::write(&adb_script, script).unwrap();
        let mut perms = std::fs::metadata(&adb_script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&adb_script, perms).unwrap();

        let adb = Adb { path: adb_script };
        let result = reply_im(
            &adb,
            "lock-serial",
            "com.tencent.mm::StepX 通知自测群",
            "com.tencent.mm",
            "StepX 通知自测群",
            "收到",
            ReplyMode::Auto,
            "client-lock-1",
        );
        stop.store(true, std::sync::atomic::Ordering::Release);
        let _ = server_handle.join();
        assert!(
            result.is_err(),
            "An unverified generic foreground group must not receive the reply"
        );

        let adb_log = std::fs::read_to_string(&log_file).unwrap_or_default();
        assert!(
            !adb_log.contains("input keyevent")
                && !adb_log.contains("input swipe")
                && !adb_log.contains("settings put")
                && !adb_log.contains("monkey"),
            "automatic background reply must not wake, dim or switch phone apps: {adb_log}"
        );
        let ops = observed_ops.lock().unwrap().clone();
        assert!(
            ops.iter().any(|s| s.starts_with("reply_im:remote_only:")),
            "Attempt 1 in Auto mode must use remote_only so Java does not prematurely wake UI: {ops:?}"
        );
        assert!(
            !ops.iter().any(|s| s.starts_with("open_push:")),
            "background reply must not open a foreground conversation: {ops:?}"
        );
        assert!(
            !ops.iter().any(|s| s == "reply_im:ui_force:群聊"),
            "must never change the selected recipient to a generic foreground group: {ops:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod reply_merge_tests {
    use super::*;
    #[test]
    fn confirmed_outgoing_survives_old_notification_refresh_and_explicit_delete_does_not_return() {
        let sent = ImMessage {
            id: "stepx-sent".into(),
            client_msg_id: "stepx-sent".into(),
            outgoing: true,
            sender: "我".into(),
            text: "已确认的回复".into(),
            timestamp_ms: 1_700_000_000_000,
            delivery_status: "sent".into(),
            via: "reply".into(),
            ..Default::default()
        };
        let old = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "chat".into(),
                title: "联系人".into(),
                package_name: "com.tencent.mm".into(),
                messages: vec![sent.clone()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut next = old.clone();
        next.threads[0].messages.clear();
        for _ in 0..3 {
            merge_local_replies(&mut next, &old);
        }
        assert_eq!(next.threads[0].messages, vec![sent]);
        let mut deleted = PushSnapshot::default();
        merge_local_replies(&mut deleted, &old);
        assert!(deleted.threads.is_empty());
    }

    #[test]
    fn notification_preview_does_not_claim_bidirectional_history() {
        let mut thread = ImThread {
            messages: vec![ImMessage {
                via: "notification".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(thread.notification_history_only());
        thread.messages[0].outgoing = true;
        assert!(!thread.notification_history_only());
        thread.messages[0].outgoing = false;
        thread.messages[0].via = "ui_capture".into();
        assert!(!thread.notification_history_only());
    }

    #[test]
    fn phone_id_and_client_id_correlate_without_collapsing_repeated_text() {
        let local = ImMessage {
            id: "stepx-a".into(),
            client_msg_id: "stepx-a".into(),
            outgoing: true,
            text: "same body".into(),
            delivery_status: "sending".into(),
            ..Default::default()
        };
        let remote: ImMessage = serde_json::from_value(
            json!({"id":"phone-a","clientMsgId":"stepx-a","outgoing":true,"text":"same body"}),
        )
        .unwrap();
        let old = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "thread".into(),
                messages: vec![local.clone()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut next = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "thread".into(),
                messages: vec![remote.clone()],
                ..Default::default()
            }],
            ..Default::default()
        };
        merge_local_replies(&mut next, &old);
        assert_eq!(next.threads[0].messages.len(), 1);
        let mut distinct = local.clone();
        distinct.id = "stepx-b".into();
        distinct.client_msg_id = "stepx-b".into();
        assert!(!distinct.same_delivery(&remote));
        next.threads[0].messages.clear();
        merge_local_replies(&mut next, &old);
        assert_eq!(next.threads[0].messages, vec![local]);
        let mut empty = PushSnapshot::default();
        merge_local_replies(&mut empty, &old);
        assert_eq!(empty.threads.len(), 1);
    }

    #[test]
    fn unchanged_delta_snapshot_preserves_existing_threads_and_notifications() {
        let prev = demo_push_snapshot();
        let mut delta = PushSnapshot {
            revision: prev.revision,
            unchanged: true,
            listener_connected: true,
            screen_interactive: true,
            transport_mode: "local_socket".into(),
            ..Default::default()
        };
        merge_local_replies(&mut delta, &prev);
        assert!(!delta.unchanged);
        assert_eq!(delta.revision, prev.revision);
        assert_eq!(delta.threads, prev.threads);
        assert_eq!(delta.notifications, prev.notifications);
        assert_eq!(delta.otp_banners, prev.otp_banners);
        assert!(delta.screen_interactive);
    }

    #[test]
    fn socket_miss_cache_blocks_repeated_forward_within_ttl() {
        let serial = "TEST-SOCKET-MISS-SERIAL";
        let now = Instant::now();
        invalidate_cached_port(serial);
        assert!(!is_socket_recently_unavailable(serial, now));
        mark_socket_unavailable(serial);
        assert!(is_socket_recently_unavailable(
            serial,
            now + Duration::from_secs(5)
        ));
        assert!(!is_socket_recently_unavailable(
            serial,
            now + SOCKET_MISS_TTL + Duration::from_secs(1)
        ));
        invalidate_cached_port(serial);
    }

    #[test]
    fn failed_outgoing_replies_do_not_stack_or_persist_across_merges() {
        let incoming = ImMessage {
            id: "in-1".into(),
            sender: "邱-stepx".into(),
            text: "发个测试".into(),
            time_known: Some(true),
            timestamp_ms: 1775021164151,
            outgoing: false,
            delivery_status: "received".into(),
            ..Default::default()
        };
        let failed = ImMessage {
            id: "fail-1".into(),
            client_msg_id: "fail-1".into(),
            sender: "我".into(),
            text: "hi".into(),
            outgoing: true,
            delivery_status: "failed".into(),
            ..Default::default()
        };
        let old = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "com.tencent.mm::StepX 通知自测群".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "StepX 通知自测群".into(),
                last_sender: "我".into(),
                last_message: "hi".into(),
                messages: vec![incoming.clone(), failed.clone(), failed],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut next = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "com.tencent.mm::StepX 通知自测群".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "StepX 通知自测群".into(),
                last_sender: "邱-stepx".into(),
                last_message: "发个测试".into(),
                messages: vec![incoming],
                ..Default::default()
            }],
            ..Default::default()
        };
        merge_local_replies(&mut next, &old);
        assert_eq!(next.threads[0].messages.len(), 1);
        assert_eq!(next.threads[0].messages[0].text, "发个测试");
        assert_eq!(next.threads[0].last_message, "发个测试");
    }

    #[test]
    fn generic_group_window_capture_merges_into_specific_notification_thread_preserving_all_metadata(
    ) {
        let mut snap = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "com.tencent.mm::StepX 通知自测群".into(),
                notification_key: "0|com.tencent.mm|4097|null|10275".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "StepX 通知自测群(3)".into(),
                last_sender: "邱-stepx".into(),
                last_message: "发个测试".into(),
                last_timestamp_ms: 1775021164151,
                unread_count: 1,
                is_group: true,
                can_remote_reply: true,
                can_ui_reply: true,
                has_content_intent: true,
                messages: vec![ImMessage {
                    id: "notif-1".into(),
                    sender: "邱-stepx".into(),
                    text: "发个测试".into(),
                    timestamp_ms: 1775021164151,
                    time_known: Some(true),
                    outgoing: false,
                    delivery_status: "received".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let captured_window = ImThread {
            thread_id: "com.tencent.mm::群聊".into(),
            notification_key: "ui_window::com.tencent.mm::群聊".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "群聊".into(),
            last_sender: "微信用户".into(),
            last_message: "发个测试".into(),
            last_timestamp_ms: 1775021400000,
            unread_count: 0,
            is_group: true,
            can_remote_reply: false,
            can_ui_reply: true,
            messages: vec![
                ImMessage {
                    id: "win-0".into(),
                    sender: "群成员".into(),
                    text: "在的".into(),
                    timestamp_ms: 0,
                    time_known: Some(false),
                    outgoing: false,
                    delivery_status: "received".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "win-1".into(),
                    sender: "微信用户".into(),
                    text: "发个测试".into(),
                    timestamp_ms: 0,
                    time_known: Some(false),
                    outgoing: false,
                    delivery_status: "received".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let merged_res = merge_synced_window_into_snapshot(&mut snap, captured_window);
        assert_eq!(merged_res.thread_id, "com.tencent.mm::StepX 通知自测群");
        assert_eq!(snap.threads.len(), 1);
        let merged = &snap.threads[0];
        assert_eq!(merged.title, "StepX 通知自测群");
        assert!(merged.can_remote_reply);
        assert_eq!(merged.notification_key, "0|com.tencent.mm|4097|null|10275");
        assert_eq!(merged.messages.len(), 2);
        assert_eq!(merged.messages[0].text, "在的");
        assert_eq!(merged.messages[1].text, "发个测试");
        assert_eq!(merged.messages[1].sender, "邱-stepx");
        assert_eq!(merged.messages[1].time_known, Some(true));
        assert_eq!(merged.messages[1].timestamp_ms, 1775021164151);
        assert_eq!(merged.last_sender, "邱-stepx");
    }

    #[test]
    fn parse_dumpsys_notifications_extracts_lockscreen_group_messages_and_remote_input() {
        let dumpsys_sample = r#"
  NotificationRecord(0x0916d306: pkg=com.tencent.mm user=UserHandle{0} id=4097 tag=null importance=4 key=0|com.tencent.mm|4097|null|10275: Notification(channel= message_channel_new_id shortcut=message_44684824419@chatroom contentView=null vibrate=null sound=null defaults=0x0 flags=0x110 color=0x00000000 category=msg Vis=PRIVATE))
    uid=10275 userId=0
    opPkg=com.tencent.mm
    tickerText=StepX 通知自测群: 邱-stepx: 发个测试
    contentIntent=PendingIntent{eb5653d: PendingIntentRecord{8260e55 com.tencent.mm startActivity}}
    extras={
      android.title=String (StepX 通知自测群(2))
      android.ConversationTitle=String (StepX 通知自测群)
      android.isGroupConversation=Boolean (true)
      android.text=String ([2条]邱-stepx: 发个测试)
      android.bigText=String (卢: 在的
邱-stepx: 发个测试)
    }
    lastAudibleAlertMillis=1775021164173
    Action 0:
      mTitle=回复
      RemoteInput 0:
        mResultKey=key_text_reply
        mLabel=回复 StepX 通知自测群
"#;
        let (notifs, threads) = parse_dumpsys_notifications(dumpsys_sample);
        assert_eq!(notifs.len(), 1);
        assert_eq!(threads.len(), 1);
        let thread = &threads[0];
        assert_eq!(thread.package_name, "com.tencent.mm");
        assert_eq!(thread.title, "StepX 通知自测群");
        assert!(thread.is_group);
        assert!(thread.can_remote_reply);
        assert!(thread.has_content_intent);
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.messages[0].sender, "卢");
        assert_eq!(thread.messages[0].text, "在的");
        assert_eq!(thread.messages[1].sender, "邱-stepx");
        assert_eq!(thread.messages[1].text, "发个测试");
    }

    #[test]
    fn system_and_lockscreen_titles_are_filtered_out_of_snapshot_threads() {
        let snap = PushSnapshot {
            threads: vec![
                ImThread {
                    thread_id: "com.tencent.mm::锁屏界面".into(),
                    package_name: "com.tencent.mm".into(),
                    title: "锁屏界面".into(),
                    last_message: "上滑解锁".into(),
                    ..Default::default()
                },
                ImThread {
                    thread_id: "com.tencent.mm::StepX 通知自测群".into(),
                    package_name: "com.tencent.mm".into(),
                    title: "StepX 通知自测群".into(),
                    last_sender: "邱-stepx".into(),
                    last_message: "发个测试".into(),
                    messages: vec![ImMessage {
                        id: "1".into(),
                        sender: "邱-stepx".into(),
                        text: "发个测试".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let enriched = enrich_snapshot(snap, "local_socket");
        assert_eq!(enriched.threads.len(), 1);
        assert_eq!(enriched.threads[0].title, "StepX 通知自测群");
    }

    #[test]
    fn unrelated_wechat_threads_are_never_collapsed_when_generic_group_shares_no_messages() {
        let snap = PushSnapshot {
            threads: vec![
                ImThread {
                    thread_id: "com.tencent.mm::微信支付".into(),
                    package_name: "com.tencent.mm".into(),
                    app_name: "微信".into(),
                    title: "微信支付".into(),
                    is_group: false,
                    last_sender: "微信支付".into(),
                    last_message: "微信支付凭证".into(),
                    last_timestamp_ms: 1775021000000,
                    messages: vec![ImMessage {
                        id: "pay-1".into(),
                        sender: "微信支付".into(),
                        text: "微信支付凭证".into(),
                        timestamp_ms: 1775021000000,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ImThread {
                    thread_id: "com.tencent.mm::软路由".into(),
                    package_name: "com.tencent.mm".into(),
                    app_name: "微信".into(),
                    title: "软路由".into(),
                    is_group: true,
                    last_sender: "张工".into(),
                    last_message: "固件已更新".into(),
                    last_timestamp_ms: 1775021100000,
                    messages: vec![ImMessage {
                        id: "router-1".into(),
                        sender: "张工".into(),
                        text: "固件已更新".into(),
                        timestamp_ms: 1775021100000,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ImThread {
                    thread_id: "com.tencent.mm::群聊".into(),
                    package_name: "com.tencent.mm".into(),
                    app_name: "微信".into(),
                    title: "群聊".into(),
                    is_group: true,
                    last_sender: "卢".into(),
                    last_message: "1".into(),
                    last_timestamp_ms: 1775021200000,
                    messages: vec![ImMessage {
                        id: "grp-1".into(),
                        sender: "群成员".into(),
                        text: "卢: 1".into(),
                        timestamp_ms: 1775021200000,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let enriched = enrich_snapshot(snap, "local_socket");
        assert_eq!(
            enriched.threads.len(),
            3,
            "unrelated WeChat conversations must never be merged into each other"
        );
        let group_thread = enriched
            .threads
            .iter()
            .find(|t| t.thread_id == "com.tencent.mm::群聊")
            .expect("generic group thread must remain distinct");
        assert_eq!(group_thread.title, "群聊");
        assert_eq!(group_thread.messages[0].sender, "卢");
        assert_eq!(group_thread.messages[0].text, "1");
    }

    #[test]
    fn parse_dumpsys_notifications_handles_messaging_style_commas_inbox_lines_and_ignores_archive()
    {
        let dumpsys_sample = r#"
  Notification List:
    NotificationRecord(0x01: pkg=com.tencent.mm user=UserHandle{0} id=4097 tag=null importance=4 key=0|com.tencent.mm|4097|null|10275: Notification(channel=msg flags=0x10))
      contentIntent=PendingIntent{eb5653d: PendingIntentRecord{8260e55 com.tencent.mm startActivity}}
      extras={
        android.conversationTitle=String (研发群)
        android.isGroupConversation=Boolean (true)
        android.messages=Parcelable[Bundle[{sender=卢-stepx, text=你好,这是一条带逗号的消息, time=1775021160000}], Bundle[{sender=邱-stepx, text=收到,马上处理, time=1775021164000}]]
      }
      lastAudibleAlertMillis=1775021164000
      Action 0:
        RemoteInput 0:
          mResultKey=key_text_reply
    NotificationRecord(0x02: pkg=com.alibaba.android.rimet user=UserHandle{0} id=4098 tag=null importance=4 key=0|com.alibaba.android.rimet|4098|null|10276: Notification(channel=msg flags=0x10))
      extras={
        android.title=String (项目组)
        android.textLines=CharSequence[] (2):
          [0]: 王五: 下午三点开会
          [1]: 赵六: 好的收到
      }
      postTime=1775021200000
  mArchive:
    NotificationRecord(0x99: pkg=com.tencent.mm user=UserHandle{0} id=9999 tag=null importance=4 key=0|com.tencent.mm|9999|null|10275: Notification(channel=msg flags=0x10))
      extras={
        android.title=String (已划掉的历史会话)
        android.text=String (这条在mArchive里不应被当成活跃未读)
      }
      postTime=1775020000000
"#;
        let (notifs, threads) = parse_dumpsys_notifications(dumpsys_sample);
        assert_eq!(notifs.len(), 2, "must ignore mArchive notifications");
        assert_eq!(threads.len(), 2);

        let wechat = threads
            .iter()
            .find(|t| t.package_name == "com.tencent.mm")
            .unwrap();
        assert_eq!(wechat.title, "研发群");
        assert!(wechat.can_remote_reply);
        assert_eq!(wechat.messages.len(), 2);
        assert_eq!(wechat.messages[0].sender, "卢-stepx");
        assert_eq!(wechat.messages[0].text, "你好,这是一条带逗号的消息");
        assert_eq!(wechat.messages[0].timestamp_ms, 1775021160000);
        assert_eq!(wechat.messages[1].sender, "邱-stepx");
        assert_eq!(wechat.messages[1].text, "收到,马上处理");
        assert_eq!(wechat.messages[1].timestamp_ms, 1775021164000);

        let ding = threads
            .iter()
            .find(|t| t.package_name == "com.alibaba.android.rimet")
            .unwrap();
        assert_eq!(ding.title, "项目组");
        assert_eq!(ding.messages.len(), 2);
        assert_eq!(ding.messages[0].sender, "王五");
        assert_eq!(ding.messages[0].text, "下午三点开会");
        assert_eq!(ding.messages[1].sender, "赵六");
        assert_eq!(ding.messages[1].text, "好的收到");
    }

    #[test]
    fn deduplicate_thread_messages_replaces_failed_bubble_when_retry_is_sending_or_sent() {
        let mut msgs = vec![
            ImMessage {
                id: "in-1".into(),
                sender: "卢".into(),
                text: "1".into(),
                timestamp_ms: 1000,
                outgoing: false,
                delivery_status: "received".into(),
                ..Default::default()
            },
            ImMessage {
                id: "fail-1".into(),
                client_msg_id: "fail-1".into(),
                sender: "我".into(),
                text: "hi".into(),
                timestamp_ms: 1001,
                outgoing: true,
                delivery_status: "failed".into(),
                ..Default::default()
            },
            ImMessage {
                id: "fail-2".into(),
                client_msg_id: "fail-2".into(),
                sender: "我".into(),
                text: "hi".into(),
                timestamp_ms: 1002,
                outgoing: true,
                delivery_status: "failed".into(),
                ..Default::default()
            },
        ];
        deduplicate_thread_messages(&mut msgs, "StepX 通知自测群");
        let failed_count = msgs
            .iter()
            .filter(|m| m.outgoing && m.delivery_status == "failed")
            .count();
        assert_eq!(
            failed_count, 1,
            "multiple failed bubbles must collapse to 1"
        );

        // Now user retries "hi", adding a "sending" bubble
        msgs.push(ImMessage {
            id: "retry-3".into(),
            client_msg_id: "retry-3".into(),
            sender: "我".into(),
            text: "hi".into(),
            timestamp_ms: 1003,
            outgoing: true,
            delivery_status: "sending".into(),
            ..Default::default()
        });
        deduplicate_thread_messages(&mut msgs, "StepX 通知自测群");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].text, "hi");
        assert_eq!(msgs[1].delivery_status, "sending");
        assert_eq!(msgs[1].client_msg_id, "retry-3");
    }

    #[test]
    fn media_hints_and_voice_durations_are_preserved_across_dedup_and_consolidation() {
        assert!(is_generic_im_title("群聊 (邱-stepx、窝-StepX)"));
        assert!(is_generic_im_title("群聊 (窝-StepX)"));
        assert!(is_generic_im_title("群聊 · 邱-stepx、窝-StepX"));
        assert_eq!(infer_media_hint_from_text("[图片]"), "[图片]");
        assert_eq!(
            infer_media_hint_from_text("窝-StepX: [动画表情]"),
            "[动画表情]"
        );
        assert_eq!(infer_media_hint_from_text("[语音] 20\""), "[语音] 20\"");
        assert_eq!(infer_media_hint_from_text("[视频]"), "[视频]");
        assert_eq!(infer_media_hint_from_text("[小程序]"), "[小程序]");

        let mut msgs = vec![
            ImMessage {
                id: "n-1".into(),
                sender: "窝-StepX".into(),
                text: "[语音]".into(),
                timestamp_ms: 1775021164000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
            },
            ImMessage {
                id: "u-1".into(),
                sender: "窝-StepX".into(),
                text: "[语音] 20\"".into(),
                timestamp_ms: 0,
                time_known: Some(false),
                outgoing: false,
                delivery_status: "observed".into(),
                via: "ui_capture".into(),
                ..Default::default()
            },
        ];
        deduplicate_thread_messages(&mut msgs, "群聊 (窝-StepX)");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "[语音] 20\"");
        assert_eq!(msgs[0].media_hint, "[语音] 20\"");
        assert_eq!(msgs[0].timestamp_ms, 1775021164000);
    }

    #[test]
    fn shared_notification_key_never_merges_distinct_service_merchants() {
        let sf = ImThread {
            thread_id: "com.tencent.mm::顺丰速运+".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "顺丰速运+".into(),
            notification_key: "0|com.tencent.mm|4097|null|10275".into(),
            ..Default::default()
        };
        let yto = ImThread {
            thread_id: "com.tencent.mm::圆通速递".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "圆通速递".into(),
            notification_key: "0|com.tencent.mm|4097|null|10275".into(),
            ..Default::default()
        };
        assert!(
            !threads_belong_to_same_conversation(&sf, &yto),
            "顺丰速运+ and 圆通速递 must never merge even when sharing notification_key"
        );
    }

    #[test]
    fn consolidate_im_threads_splits_polluted_service_thread_into_four_merchants_and_cleans_probes()
    {
        let polluted = ImThread {
            thread_id: "com.tencent.mm::顺丰速运+".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "顺丰速运+".into(),
            is_group: false,
            unread_count: 4,
            last_sender: "霸王茶姬".into(),
            last_message: "霸王茶姬: 取餐提醒".into(),
            last_timestamp_ms: 4000,
            notification_key: "0|com.tencent.mm|4097|null|10275".into(),
            messages: vec![
                ImMessage {
                    id: "m1".into(),
                    sender: "顺丰速运+".into(),
                    text: "顺丰同城急送服务通知".into(),
                    timestamp_ms: 1000,
                    time_known: Some(true),
                    outgoing: false,
                    delivery_status: "received".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "m2".into(),
                    sender: "圆通速递".into(),
                    text: "快件派件通知".into(),
                    timestamp_ms: 2000,
                    time_known: Some(true),
                    outgoing: false,
                    delivery_status: "received".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "m3".into(),
                    sender: "叮咚买菜 美食水果蔬菜海鲜早餐".into(),
                    text: "订单配送通知".into(),
                    timestamp_ms: 3000,
                    time_known: Some(true),
                    outgoing: false,
                    delivery_status: "received".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "m4".into(),
                    sender: "霸王茶姬".into(),
                    text: "取餐提醒".into(),
                    timestamp_ms: 4000,
                    time_known: Some(true),
                    outgoing: false,
                    delivery_status: "received".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let mut threads = vec![polluted];
        consolidate_im_threads(&mut threads, &[]);
        assert_eq!(
            threads.len(),
            4,
            "polluted thread must split into 4 separate merchant threads: {:?}",
            threads.iter().map(|t| &t.title).collect::<Vec<_>>()
        );

        for expected_title in [
            "顺丰速运+",
            "圆通速递",
            "叮咚买菜 美食水果蔬菜海鲜早餐",
            "霸王茶姬",
        ] {
            let t = threads
                .iter()
                .find(|t| t.title == expected_title)
                .unwrap_or_else(|| panic!("missing split thread for {expected_title}"));
            assert_eq!(t.messages.len(), 1);
            assert_eq!(t.messages[0].sender, expected_title);
            assert!(!t.is_group);
        }

        assert_eq!(
            clean_message_display_text("我: 稍后同步进展 [文件传输助手队列消息 #1]"),
            "我: 稍后同步进展"
        );
        assert_eq!(
            clean_message_display_text("稍后同步进展 [文件传输助手队列消息 #1]"),
            "稍后同步进展"
        );
    }

    #[test]
    fn decodes_and_caches_avatar_png_on_disk_and_populates_thread_paths() {
        // Minimal valid 1x1 transparent PNG in Base64
        let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let path = decode_and_cache_avatar_b64("com.tencent.mm::顺丰速运+", png_b64)
            .expect("must decode valid PNG and write to cache");
        assert!(path.is_file());
        assert_eq!(
            cached_thread_avatar_path("com.tencent.mm", "顺丰速运+").as_ref(),
            Some(&path)
        );
        assert_eq!(cached_sender_avatar_path("顺丰速运+").as_ref(), Some(&path));
    }

    #[test]
    fn screen_capture_merge_preserves_local_outgoing_at_bottom_without_timestamp_inheritance() {
        let mut existing = ImThread {
            thread_id: "com.tencent.mm::文件传输助手".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "文件传输助手".into(),
            last_timestamp_ms: 5000,
            messages: vec![
                ImMessage {
                    id: "m_in_1".into(),
                    sender: "文件传输助手".into(),
                    text: "第一条历史消息".into(),
                    timestamp_ms: 1000,
                    time_known: Some(true),
                    outgoing: false,
                    delivery_status: "received".into(),
                    via: "notification".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "local_5000".into(),
                    sender: "我".into(),
                    text: "我刚发的本地回复".into(),
                    timestamp_ms: 5000,
                    time_known: Some(true),
                    outgoing: true,
                    delivery_status: "sent".into(),
                    via: "remote_input".into(),
                    client_msg_id: "stepx-5000".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        // Incoming screen capture has >= existing.messages.len() messages, with un-timestamped screen bubbles
        // and does not yet include the newly sent outgoing message.
        let incoming = ImThread {
            thread_id: "com.tencent.mm::文件传输助手".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "文件传输助手".into(),
            last_timestamp_ms: 0,
            messages: vec![
                ImMessage {
                    id: "cap_1".into(),
                    sender: "文件传输助手".into(),
                    text: "第一条历史消息".into(),
                    timestamp_ms: 0,
                    time_known: Some(false),
                    outgoing: false,
                    delivery_status: "observed".into(),
                    via: "ui_window".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "cap_2".into(),
                    sender: "文件传输助手".into(),
                    text: "屏幕上较早的另一条消息".into(),
                    timestamp_ms: 0,
                    time_known: Some(false),
                    outgoing: false,
                    delivery_status: "observed".into(),
                    via: "ui_window".into(),
                    ..Default::default()
                },
                ImMessage {
                    id: "cap_3".into(),
                    sender: "文件传输助手".into(),
                    text: "屏幕上较早的第三条消息".into(),
                    timestamp_ms: 0,
                    time_known: Some(false),
                    outgoing: false,
                    delivery_status: "observed".into(),
                    via: "ui_window".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        merge_thread_preserving_metadata(&mut existing, incoming);
        assert_eq!(existing.messages.len(), 4);
        let tail = existing.messages.last().expect("must have tail message");
        assert!(
            tail.outgoing,
            "local outgoing message must remain at the bottom"
        );
        assert_eq!(tail.text, "我刚发的本地回复");
        assert_eq!(existing.last_sender, "我");
        assert_eq!(existing.last_message, "我刚发的本地回复");
        for m in &existing.messages[..3] {
            assert!(
                m.timestamp_ms < tail.timestamp_ms,
                "un-timestamped screen message ({}) must not inherit outgoing timestamp ({})",
                m.timestamp_ms,
                tail.timestamp_ms
            );
        }
    }

    #[test]
    fn native_sms_threads_expired_otp_banners_and_deleted_sent_threads_are_pruned() {
        assert!(is_native_sms_package_or_thread("com.android.mms", ""));
        assert!(is_native_sms_package_or_thread(
            "",
            "com.android.mms::79983898"
        ));
        assert!(!is_native_sms_package_or_thread(
            "com.tencent.mm",
            "com.tencent.mm::张三"
        ));

        let now = chrono::Local::now().timestamp_millis();
        let expired = OtpBanner {
            code: "123456".into(),
            source_app: "短信".into(),
            title: "验证码".into(),
            raw_text: "验证码 123456".into(),
            timestamp_ms: now - OTP_TTL_MS - 1_000,
            notification_key: "k1".into(),
        };
        let fresh = OtpBanner {
            code: "654321".into(),
            source_app: "短信".into(),
            title: "验证码".into(),
            raw_text: "验证码 654321".into(),
            timestamp_ms: now - 60_000,
            notification_key: "k2".into(),
        };
        assert!(expired.is_expired(now));
        assert!(!fresh.is_expired(now));
        let json_expired: OtpBanner = serde_json::from_value(serde_json::json!({
            "code": "79983898",
            "appName": "短信",
            "title": "丰巢",
            "text": "凭取件码79983898取件",
            "expiresAtMs": now - 5_000,
            "key": "k_exp"
        }))
        .unwrap();
        assert!(
            json_expired.is_expired(now),
            "OtpBanner with past expiresAtMs in JSON must be expired"
        );

        let old = PushSnapshot {
            threads: vec![
                ImThread {
                    thread_id: "ai.x.grok.bot::马助理".into(),
                    package_name: "ai.x.grok.bot".into(),
                    messages: vec![ImMessage {
                        id: "stepx-sent-1".into(),
                        client_msg_id: "stepx-sent-1".into(),
                        text: "好的".into(),
                        timestamp_ms: now - 30_000,
                        outgoing: true,
                        delivery_status: "sent".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ImThread {
                    thread_id: "com.android.mms::79983898".into(),
                    package_name: "com.android.mms".into(),
                    ..Default::default()
                },
                ImThread {
                    thread_id: "com.tencent.mm::王五".into(),
                    package_name: "com.tencent.mm".into(),
                    title: "王五".into(),
                    messages: vec![
                        ImMessage {
                            id: "stepx-old-out".into(),
                            client_msg_id: "stepx-old-out".into(),
                            text: "两分钟前已确认发送的本地消息".into(),
                            timestamp_ms: now - 120_000,
                            outgoing: true,
                            delivery_status: "sent".into(),
                            ..Default::default()
                        },
                        ImMessage {
                            id: "ui_untimed_out".into(),
                            text: "无时间戳旧UI捕获消息".into(),
                            timestamp_ms: 0,
                            observed_at_ms: 0,
                            outgoing: true,
                            delivery_status: "observed".into(),
                            via: "ui_capture".into(),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut next = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "com.tencent.mm::王五".into(),
                package_name: "com.tencent.mm".into(),
                title: "王五".into(),
                messages: vec![ImMessage {
                    id: "in_1".into(),
                    sender: "王五".into(),
                    text: "最新一条消息".into(),
                    timestamp_ms: now - 5_000,
                    outgoing: false,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        merge_local_replies(&mut next, &old);
        assert_eq!(
            next.threads.len(),
            1,
            "deleted thread with already-sent reply and native SMS thread must not be resurrected"
        );
        assert_eq!(
            next.threads[0].messages.len(),
            3,
            "a notification-only refresh is not a deletion receipt for older outgoing evidence"
        );
    }

    #[test]
    fn mentions_require_current_user_evidence_and_current_conversation() {
        for body in [
            "@张三 请看一下",
            "＠张三 请看一下",
            "@我 请看一下",
            "@所有人",
            "@全体成员",
            "@all hello",
            "@everyone hello",
            "email@example.com",
            "引用：[有人@我] 这是什么意思",
            "[有人@我] @所有人 今天开会",
        ] {
            assert!(extract_direct_at_mention(body).is_none(), "{body}");
            let n = PushNotification {
                text: body.into(),
                sub_text: "有人@我 · 重要消息推送".into(),
                ..Default::default()
            };
            assert!(
                !notification_mentions_current_user(&n),
                "legacy synthetic label: {body}"
            );
        }
        let mut thread = ImThread {
            package_name: "com.tencent.mm".into(),
            thread_id: "group:current".into(),
            notification_key: "notification:current".into(),
            unread_count: 1,
            last_message: "请看一下".into(),
            last_timestamp_ms: 2000,
            ..Default::default()
        };
        let mut n = PushNotification {
            package_name: thread.package_name.clone(),
            thread_id: thread.thread_id.clone(),
            active: true,
            post_time_ms: 2000,
            sub_text: "有人@我".into(),
            ..Default::default()
        };
        assert!(thread_mentions_current_user(&thread, &[n.clone()]));
        n.thread_id = "group:other".into();
        assert!(!thread_mentions_current_user(&thread, &[n.clone()]));
        n.key = thread.notification_key.clone();
        assert!(thread_mentions_current_user(&thread, &[n.clone()]));
        n.post_time_ms = 1999;
        assert!(!thread_mentions_current_user(&thread, &[n.clone()]));
        n.post_time_ms = 2000;
        n.read = true;
        assert!(!thread_mentions_current_user(&thread, &[n.clone()]));
        n.read = false;
        n.package_name = "other.app".into();
        assert!(!thread_mentions_current_user(&thread, &[n.clone()]));
        n.package_name = thread.package_name.clone();
        thread.unread_count = 0;
        assert!(!thread_mentions_current_user(&thread, &[n.clone()]));
        thread.unread_count = 1;
        thread.messages.push(ImMessage {
            outgoing: true,
            ..Default::default()
        });
        assert!(!thread_mentions_current_user(&thread, &[n.clone()]));
        thread.messages.clear();
        thread.last_message = "@所有人".into();
        assert!(!thread_mentions_current_user(&thread, &[n]));
        thread.last_message = "[有人@我] 张三: 请看一下".into();
        assert!(thread_mentions_current_user(&thread, &[]));
    }

    #[test]
    fn direct_at_mention_and_career_recruitment_signals_are_recognized_without_false_positives() {
        assert_eq!(
            extract_direct_at_mention("@倒影颠倒 下午有空一起对一下StepX胶囊卡片吗？"),
            None
        );
        assert_eq!(
            extract_direct_at_mention("[有人@我] 张三: 请看下最新设计稿"),
            Some("@我".to_string())
        );
        assert_eq!(
            extract_direct_at_mention("各位同事 @所有人 今晚8点发版"),
            None
        );
        assert_eq!(
            extract_direct_at_mention("请联系邮箱 developer@stepx.ai 获取权限"),
            None
        );

        assert!(is_career_or_recruitment_signal(
            "com.hpbr.bosszhipin",
            "BOSS直聘",
            "王女士 · 字节跳动",
            "您好，看到您的简历非常匹配，方便聊聊吗？"
        ));
        assert!(is_career_or_recruitment_signal(
            "com.tencent.mm",
            "微信",
            "BOSS直聘通知",
            "有2位牛人查看了你的简历并发来面试邀约"
        ));
        assert!(!is_career_or_recruitment_signal(
            "com.tencent.mm",
            "微信",
            "产品研发群",
            "今天下午三点讨论发布计划"
        ));
        assert_eq!(
            ImChannel::from_package("com.hpbr.bosszhipin"),
            ImChannel::SocialDm
        );
        assert!(ImChannel::GlobalIm.matches_package("com.hpbr.bosszhipin"));
    }

    #[test]
    fn notification_conversation_keeps_sender_and_background_reply_handle() {
        for (group, can_reply, key) in [
            (true, false, ""),
            (false, true, ""),
            (false, false, "0|chat|1"),
        ] {
            let mut threads = vec![ImThread {
                thread_id: "chat.app::项目讨论".into(),
                package_name: "chat.app".into(),
                title: "项目讨论".into(),
                is_group: group,
                can_remote_reply: can_reply,
                notification_key: key.into(),
                messages: vec![ImMessage {
                    id: "event-1".into(),
                    sender: "王斌".into(),
                    text: "请看这条订单通知".into(),
                    via: "notification".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }];
            consolidate_im_threads(&mut threads, &[]);
            assert_eq!(threads.len(), 1);
            assert_eq!(threads[0].thread_id, "chat.app::项目讨论");
            assert_eq!(threads[0].title, "项目讨论");
            assert_eq!(threads[0].messages[0].sender, "王斌");
            assert_eq!(threads[0].can_remote_reply, can_reply);
            assert_eq!(threads[0].notification_key, key);
        }
    }

    #[test]
    fn split_polluted_service_threads_preserves_observed_at_ms_timestamp() {
        let obs_ts = 1_793_534_529_744i64;
        let mut threads = vec![ImThread {
            thread_id: "com.tencent.mm::服务通知".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "服务通知".into(),
            unread_count: 0,
            last_timestamp_ms: 0,
            messages: vec![ImMessage {
                id: "1".into(),
                sender: "顺丰速运+".into(),
                text: "账户变动提醒".into(),
                timestamp_ms: 0,
                observed_at_ms: obs_ts,
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
        }];
        split_polluted_service_threads(&mut threads);
        let sf = threads
            .iter()
            .find(|t| t.title == "顺丰速运+")
            .expect("split SF thread");
        assert_eq!(sf.last_timestamp_ms, obs_ts);
        assert_eq!(sf.messages[0].timestamp_ms, obs_ts);
    }

    #[test]
    fn enterprise_at_suffix_and_truncated_titles_unify_and_prefer_clean_contact_title() {
        assert_eq!(
            canonical_conversation_title_key("保险 3623 @中华财险"),
            canonical_conversation_title_key("保险 3623")
        );
        assert_eq!(
            canonical_conversation_title_key("保险 3623 @中华财…"),
            canonical_conversation_title_key("保险 3623")
        );
        assert_eq!(
            canonical_conversation_title_key("user@example.com"),
            "user@example.com"
        );

        let t_clean = ImThread {
            thread_id: "com.tencent.mm::保险 3623".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "保险 3623".into(),
            messages: vec![ImMessage {
                id: "m1".into(),
                sender: "保险 3623".into(),
                text: "您好，车险保单已发您".into(),
                timestamp_ms: 1000,
                time_known: Some(true),
                ..Default::default()
            }],
            ..Default::default()
        };
        let t_truncated_enterprise = ImThread {
            thread_id: "com.tencent.mm::保险 3623 @中华财…".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "保险 3623 @中华财…".into(),
            messages: vec![ImMessage {
                id: "m2".into(),
                sender: "我".into(),
                text: "收到，谢谢".into(),
                timestamp_ms: 2000,
                time_known: Some(true),
                outgoing: true,
                delivery_status: "sent".into(),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(threads_belong_to_same_conversation(
            &t_clean,
            &t_truncated_enterprise
        ));

        let mut threads = vec![t_truncated_enterprise, t_clean];
        consolidate_im_threads(&mut threads, &[]);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].title, "保险 3623");
        assert_eq!(threads[0].messages.len(), 2);
    }

    #[test]
    fn unbracketed_voice_and_media_labels_normalize_and_ui_action_labels_are_filtered() {
        assert_eq!(
            normalize_scraped_media_or_voice_text("语音6秒未播放"),
            "[语音] 6\""
        );
        assert_eq!(
            normalize_scraped_media_or_voice_text("语音 15 秒未播放"),
            "[语音] 15\""
        );
        assert_eq!(
            normalize_scraped_media_or_voice_text("未播放语音6秒"),
            "[语音] 6\""
        );
        assert_eq!(normalize_scraped_media_or_voice_text("图片"), "[图片]");
        assert_eq!(
            normalize_scraped_media_or_voice_text("动画表情"),
            "[动画表情]"
        );
        assert_eq!(
            normalize_scraped_media_or_voice_text("微信动画表情"),
            "[动画表情]"
        );
        assert_eq!(normalize_scraped_media_or_voice_text("视频"), "[视频]");
        assert_eq!(
            normalize_scraped_media_or_voice_text("小程序卡片"),
            "[小程序]"
        );
        assert_eq!(clean_message_display_text("语音6秒未播放"), "[语音] 6\"");

        for label in [
            "转文字",
            "重发",
            "撤回",
            "引用",
            "多选",
            "拍一拍",
            "转发截图",
            "系统桌面",
            "输入密码",
        ] {
            assert!(
                is_bogus_captured_view_text(label),
                "UI action/system label '{label}' must be rejected as bogus"
            );
        }

        let launcher_thread = ImThread {
            thread_id: "com.android.launcher3::系统桌面".into(),
            package_name: "com.android.launcher3".into(),
            title: "系统桌面".into(),
            ..Default::default()
        };
        let lockscreen_thread = ImThread {
            thread_id: "com.tencent.mm::输入密码".into(),
            package_name: "com.tencent.mm".into(),
            title: "输入密码".into(),
            ..Default::default()
        };
        assert!(is_scraped_ui_noise_thread(&launcher_thread));
        assert!(is_scraped_ui_noise_thread(&lockscreen_thread));
    }

    #[test]
    fn outgoing_preservation_and_forward_timestamp_ordering_work_across_preview_and_capture() {
        let msgs = vec![
            ImMessage {
                id: "preview_1".into(),
                sender: "保险 3623".into(),
                text: "好的我马上处理".into(),
                timestamp_ms: 0,
                time_known: Some(false),
                outgoing: false,
                delivery_status: "observed".into(),
                via: "ui_list_preview".into(),
                ..Default::default()
            },
            ImMessage {
                id: "out_1".into(),
                sender: "我".into(),
                text: "好的我马上处理".into(),
                timestamp_ms: 10_000,
                time_known: Some(true),
                outgoing: true,
                delivery_status: "sent".into(),
                via: "ui_bridge".into(),
                ..Default::default()
            },
            ImMessage {
                id: "in_after_out".into(),
                sender: "保险 3623".into(),
                text: "语音6秒未播放".into(),
                timestamp_ms: 0,
                time_known: Some(false),
                outgoing: false,
                delivery_status: "observed".into(),
                via: "ui_capture".into(),
                ..Default::default()
            },
        ];

        let mut thread = ImThread {
            thread_id: "com.tencent.mm::保险 3623".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "保险 3623".into(),
            messages: msgs,
            ..Default::default()
        };
        sanitize_im_thread_messages(&mut thread);
        deduplicate_thread_messages(&mut thread.messages, "保险 3623");
        let msgs = &thread.messages;
        assert_eq!(msgs.len(), 2);
        assert!(
            msgs[0].outgoing,
            "merged preview + outgoing message must preserve outgoing = true"
        );
        assert_eq!(msgs[0].sender, "我");
        assert_eq!(msgs[0].timestamp_ms, 10_000);
        assert_eq!(msgs[1].text, "[语音] 6\"");
        assert!(!msgs[1].outgoing);
        assert_eq!(
            msgs[1].timestamp_ms, 10_001,
            "incoming screen bubble after outgoing must get last_known_any_ts + 1"
        );
    }

    #[test]
    fn same_text_does_not_change_speaker_or_attach_unrelated_media() {
        let base = ImMessage {
            id: "a".into(),
            sender: "王斌".into(),
            text: "好的".into(),
            timestamp_ms: 1000,
            ..Default::default()
        };
        let mut other = base.clone();
        other.id = "b".into();
        other.sender = "张三".into();
        let mut mine = base.clone();
        mine.id = "c".into();
        mine.sender = "我".into();
        mine.outgoing = true;
        mine.via = "ui_capture".into();
        mine.time_known = Some(false);
        let mut rows = vec![base, other, mine];
        deduplicate_thread_messages(&mut rows, "项目群");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().filter(|m| m.outgoing).count(), 1);
        for row in &mut rows {
            row.text = "[图片]".into();
            row.media_hint = "[图片]".into();
        }
        let mut second_image = rows[0].clone();
        second_image.id = "another-image".into();
        rows.push(second_image);
        deduplicate_thread_messages(&mut rows, "项目群");
        assert_eq!(
            rows.len(),
            4,
            "media placeholders are not a message identity"
        );
    }

    #[test]
    fn media_cache_requires_exact_conversation_and_rejects_arbitrary_local_images() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&[0x35; 96]);
        let b64 = STANDARD.encode(png);
        let path = decode_and_cache_im_media_b64("media-owner-a::same-message-id", &b64).unwrap();
        assert!(cached_im_media_path("media-owner-a", "same-message-id").is_some());
        assert!(cached_im_media_path("media-owner-b", "same-message-id").is_none());
        let wrong = ImMessage {
            id: "unbound-id".into(),
            media_path: Some(path),
            ..Default::default()
        };
        assert!(verified_message_media_path("media-owner-b", &wrong).is_none());
    }

    #[test]
    fn media_b64_decodes_to_cache_and_survives_thread_merge() {
        let mut fake_png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        fake_png.extend_from_slice(&[0x42u8; 96]);
        let b64 = STANDARD.encode(&fake_png);

        let tid = "com.tencent.mm::StepX测试群";
        let mid = "ui_sticker_msg_1";
        let path = decode_and_cache_im_media_b64(&format!("{tid}::{mid}"), &b64)
            .expect("should decode and cache >=64 byte PNG media");
        assert!(path.is_file());
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
        assert_eq!(
            cached_im_media_path(tid, mid).as_deref(),
            Some(path.as_path())
        );

        let mut existing = ImThread {
            thread_id: tid.into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "StepX测试群".into(),
            is_group: true,
            messages: vec![ImMessage {
                id: "notif_sticker_1".into(),
                sender: "窝-StepX".into(),
                text: "[动画表情]".into(),
                media_hint: "[动画表情]".into(),
                timestamp_ms: 1_760_000_010_000,
                time_known: Some(true),
                via: "notification".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let incoming = ImThread {
            thread_id: tid.into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "StepX测试群".into(),
            is_group: true,
            messages: vec![ImMessage {
                id: mid.into(),
                sender: "窝-StepX".into(),
                text: "[动画表情]".into(),
                media_hint: "[动画表情]".into(),
                media_b64: b64.clone(),
                timestamp_ms: 0,
                time_known: Some(false),
                via: "ui_capture".into(),
                ..Default::default()
            }],
            ..Default::default()
        };

        merge_thread_preserving_metadata(&mut existing, incoming);
        assert_eq!(existing.messages.len(), 2);
        let original = existing
            .messages
            .iter()
            .find(|m| m.id == "notif_sticker_1")
            .unwrap();
        assert!(original.media_b64.is_empty() && original.media_path.is_none());
        let captured = existing.messages.iter().find(|m| m.id == mid).unwrap();
        assert_eq!(captured.media_b64, b64);
        assert!(captured.media_path.as_ref().is_some_and(|p| p.is_file()));
    }

    #[cfg(unix)]
    #[test]
    fn statusbar_fallback_and_unresolved_media_never_scan_unrelated_phone_photos() {
        use std::os::unix::fs::PermissionsExt;

        // 1. When listener_connected == true, dumpsys notification fallback must NEVER run,
        //    even if threads are empty or a thread has a generic title like "群聊".
        let connected_empty = PushSnapshot {
            listener_connected: true,
            unchanged: false,
            threads: Vec::new(),
            ..Default::default()
        };
        assert!(
            !should_run_statusbar_fallback(&connected_empty),
            "must not run dumpsys notification when listener_connected == true and threads is empty"
        );

        let connected_generic = PushSnapshot {
            listener_connected: true,
            unchanged: false,
            threads: vec![ImThread {
                thread_id: "com.tencent.mm::群聊".into(),
                package_name: "com.tencent.mm".into(),
                title: "群聊".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(
            !should_run_statusbar_fallback(&connected_generic),
            "must not run dumpsys notification when listener_connected == true even for generic title"
        );

        let disconnected = PushSnapshot {
            listener_connected: false,
            unchanged: false,
            ..Default::default()
        };
        assert!(
            should_run_statusbar_fallback(&disconnected),
            "should allow statusbar fallback only when !listener_connected && !unchanged"
        );

        // 2. Negative cache for unresolvable IM media placeholders (10-minute TTL):
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("media_adb.log");
        let script_path = dir.path().join("fake_adb.sh");
        let script = format!(
            "#!/usr/bin/env bash\necho \"$@\" >> \"{}\"\nexit 0\n",
            log_path.display()
        );
        std::fs::write(&script_path, script).unwrap();
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();

        let adb = Adb { path: script_path };
        let tid = "com.tencent.mm::负缓存测试群";
        let mid = "unresolved_img_msg_99";
        let mut threads = vec![ImThread {
            thread_id: tid.into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "负缓存测试群".into(),
            messages: vec![ImMessage {
                id: mid.into(),
                sender: "张三".into(),
                text: "[图片]".into(),
                media_hint: "[图片]".into(),
                ..Default::default()
            }],
            ..Default::default()
        }];

        enrich_threads_with_im_media(&adb, "test-serial", &mut threads);
        enrich_threads_with_im_media(&adb, "test-serial", &mut threads);
        assert!(threads[0].messages[0].media_path.is_none());
        assert!(
            std::fs::read_to_string(&log_path)
                .unwrap_or_default()
                .is_empty(),
            "missing message media must not scan gallery or screenshots"
        );
    }
}
