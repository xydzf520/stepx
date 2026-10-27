//! Multi-Source App Store Core Engine (`v0.2.52`).
//!
//! Aggregates 6 real upstream sources into a single deduplicated `StoreApp` per package
//! ("多源显示一个"):
//! 1. `StoreSource::Tencent` — 应用宝实时源 (`sj.qq.com` / `a.app.qq.com`)
//! 2. `StoreSource::Official` — 厂商官方直连 (绿联云、极空间、飞牛、群晖 Archive、Tailscale 等)
//! 3. `StoreSource::Global` — 国际应用源 (`ws75.aptoide.com` / 第三方应用分发)
//! 4. `StoreSource::GitHub` — GitHub Releases 开源直连
//! 5. `StoreSource::FDroid` — F-Droid 开源仓库
//! 6. `StoreSource::StepOs` — StepOS 商店源 (`api-app.stepaos.com`)

use crate::{apps::AppInfo, resources};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::HashMap,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StoreSource {
    Tencent,
    Official,
    Global,
    StepOs,
    GitHub,
    FDroid,
}

impl StoreSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tencent => "应用宝实时源",
            Self::Official => "厂商官方直连",
            Self::Global => "国际应用源",
            Self::StepOs => "StepOS 商店",
            Self::GitHub => "GitHub Releases",
            Self::FDroid => "F-Droid 开源源",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Tencent => "应用宝",
            Self::Official => "官方直连",
            Self::Global => "国际源",
            Self::StepOs => "StepOS",
            Self::GitHub => "GitHub",
            Self::FDroid => "F-Droid",
        }
    }

    pub fn priority(self) -> u8 {
        match self {
            Self::Official => 5,
            Self::Tencent | Self::Global => 4,
            Self::GitHub => 3,
            Self::FDroid => 2,
            Self::StepOs => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StoreCategory {
    All,
    Social,
    Media,
    Productivity,
    Tools,
    Lifestyle,
    Updatable,
    Installed,
}

impl StoreCategory {
    pub const ALL: [StoreCategory; 8] = [
        Self::All,
        Self::Social,
        Self::Media,
        Self::Productivity,
        Self::Tools,
        Self::Lifestyle,
        Self::Updatable,
        Self::Installed,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::All => "推荐应用",
            Self::Social => "社交通讯",
            Self::Media => "影音娱乐",
            Self::Productivity => "办公效率",
            Self::Tools => "实用工具",
            Self::Lifestyle => "生活出行",
            Self::Updatable => "可更新",
            Self::Installed => "已安装",
        }
    }
}

pub fn package_aliases(package: &str) -> &'static [&'static str] {
    match package {
        "com.twitter.android" => &["x", "tw", "twitter", "推特", "x.com", "马斯克"],
        "com.facebook.katana" => &["fb", "facebook", "脸书", "meta"],
        "com.facebook.orca" => &["messenger", "fb", "facebook", "meta", "脸书"],
        "com.instagram.android" => &["ins", "ig", "instagram", "insta", "照片墙", "meta", "facebook"],
        "org.telegram.messenger" | "org.telegram.messenger.web" => {
            &["tg", "telegram", "电报", "纸飞机"]
        }
        "com.whatsapp" => &["wa", "whatsapp", "meta", "facebook"],
        "com.google.android.youtube" => &["yt", "youtube", "油管", "ytb"],
        "com.openai.chatgpt" => &[
            "gpt", "chatgpt", "gpt4", "gpt4o", "o1", "o3", "openai", "ai", "人工智能",
        ],
        "com.anthropic.claude" => &[
            "claude", "anthropic", "sonnet", "opus", "haiku", "ai", "克劳德",
        ],
        "com.google.android.apps.bard" => &[
            "gemini", "bard", "google ai", "雙子座", "双子座", "ai",
        ],
        "ai.perplexity.app.android" => &[
            "perplexity", "pplx", "ai search", "ai搜索", "ai",
        ],
        "ai.x.grok" => &["grok", "xai", "x.ai", "马斯克ai", "ai"],
        "com.microsoft.copilot" => &[
            "copilot", "bing ai", "微软ai", "microsoft ai", "ai",
        ],
        "com.poe.android" => &["poe", "quora ai", "gpt", "claude", "ai"],
        "com.openai.sora" => &["sora", "openai video", "gpt", "ai视频", "ai"],
        "com.android.chrome" => &["chrome", "google chrome", "谷歌浏览器", "浏览器"],
        "com.microsoft.emmx" => &["edge", "microsoft edge", "微软浏览器", "浏览器"],
        "org.mozilla.firefox" => &["firefox", "火狐", "火狐浏览器", "浏览器"],
        "mark.via" => &["via", "via浏览器", "极简浏览器", "浏览器"],
        "com.discord" => &["dc", "discord", "语音"],
        "com.spotify.music" => &["spotify", "声破天", "音乐"],
        "com.reddit.frontpage" => &["reddit", "红迪"],
        "com.ugreen.pro" => &[
            "ugreen", "ugos", "ugos pro", "绿联", "绿联云", "绿联nas", "nas", "dxp",
        ],
        "com.ugreen.iot.connect" => &["ugreen", "绿联", "绿联智联", "iot", "nas"],
        "com.ugreen.nas" => &["ugreen", "绿联", "绿联私有云", "nas"],
        "com.trim.app" => &[
            "fnos", "feiniu", "trim", "飞牛", "飞牛云", "飞牛私有云", "飞牛nas", "nas",
        ],
        "com.zentraedi.zspace" => &["zspace", "zos", "极空间", "极空间私有云", "极空间nas", "极影视", "nas"],
        "com.synology.DSfile" => &["synology", "dsfile", "ds file", "群晖", "群晖nas", "nas"],
        "com.synology.dsfinder" => &["synology", "dsfinder", "ds finder", "群晖", "群晖管家", "nas"],
        "com.synology.uhidrive" | "com.synology.dsdrive" => &[
            "synology", "synology drive", "drive", "群晖", "群晖drive", "nas",
        ],
        "com.tailscale.ipn" => &["tailscale", "wireguard", "vpn", "异地组网", "内网穿透", "nas"],
        "com.zerotier.one" => &["zerotier", "异地组网", "内网穿透", "nas"],
        "app.alextran.immich" => &["immich", "相册", "nas"],
        "com.tencent.mm" => &["wechat", "weixin", "wx", "微信"],
        "com.tencent.mobileqq" => &["qq", "腾讯qq", "手机qq"],
        "com.tencent.tim" => &["tim", "腾讯tim", "办公qq"],
        "com.sina.weibo" => &["weibo", "wb", "微博", "新浪微博"],
        "com.zhihu.android" => &["zhihu", "知乎"],
        "com.coolapk.market" => &["coolapk", "酷安", "基安"],
        "com.douban.frodo" => &["douban", "豆瓣"],
        "com.baidu.tieba" => &["tieba", "贴吧", "百度贴吧"],
        "cn.soulapp.android" => &["soul", "社交"],
        "com.eg.android.AlipayGphone" => &["alipay", "zfb", "支付宝"],
        "tv.danmaku.bili" => &["bili", "bilibili", "b站", "哔哩哔哩"],
        "com.xingin.xhs" => &["xhs", "rednote", "小红书"],
        "com.ss.android.lark" => &["lark", "feishu", "飞书"],
        "com.alibaba.android.rimet" => &["dingtalk", "dingding", "钉钉"],
        "com.tencent.wework" => &["wecom", "wework", "企微", "企业微信"],
        "cn.wps.moffice_eng" => &["wps", "wps office", "文档", "office"],
        "com.tencent.wemeet.app" => &["wemeet", "voov", "腾讯会议", "会议"],
        "com.tencent.docs" => &["腾讯文档", "在线文档"],
        "notion.id" => &["notion", "笔记", "知识库"],
        "md.obsidian" => &["obsidian", "黑曜石", "markdown", "笔记"],
        "com.yuque.mobile.android.app" => &["yuque", "语雀", "知识库"],
        "com.baidu.netdisk" => &["baiduyun", "pan", "百度网盘", "百度云"],
        "com.alicloud.databox" => &["alipan", "aliyundrive", "阿里云盘", "阿里网盘"],
        "com.quark.browser" => &["quark", "夸克", "夸克网盘"],
        "com.chinamobile.mcloud" => &["mcloud", "中国移动云盘", "移动云盘", "和彩云"],
        "com.tencent.androidqqmail" => &["qqmail", "qq邮箱", "邮箱"],
        "com.netease.mail" => &["netease mail", "网易邮箱", "邮箱大师", "163邮箱"],
        "com.deepseek.chat" => &["deepseek", "深度求索", "ai", "大模型"],
        "com.larus.nova" => &["doubao", "豆包", "ai", "字节ai"],
        "com.moonshot.kimichat" => &["kimi", "moonshot", "月之暗面", "ai"],
        "com.autonavi.minimap" => &["amap", "gaode", "高德", "高德地图", "导航", "地图"],
        "com.baidu.BaiduMap" => &["baidumap", "百度地图", "导航", "地图"],
        "com.sankuai.meituan" => &["meituan", "美团", "外卖"],
        "me.ele" => &["eleme", "饿了么", "外卖"],
        "com.dianping.v1" => &["dianping", "大众点评", "点评"],
        "com.taobao.taobao" => &["taobao", "tb", "淘宝"],
        "com.jingdong.app.mall" => &["jd", "jingdong", "京东"],
        "com.xunmeng.pinduoduo" => &["pdd", "pinduoduo", "拼多多"],
        "com.taobao.idlefish" => &["xianyu", "闲鱼", "二手"],
        "com.shizhuang.duapp" => &["dewu", "poison", "得物", "毒"],
        "com.sdu.didi.psnger" => &["didi", "滴滴", "滴滴出行", "打车"],
        "com.MobileTicket" => &["12306", "铁路12306", "火车票", "高铁"],
        "ctrip.android.view" => &["ctrip", "携程", "携程旅行", "机票", "酒店"],
        "com.umetrip.android.msky.app" => &["umetrip", "航旅纵横", "航班", "值机"],
        "com.unionpay" => &["unionpay", "云闪付", "银联"],
        "com.gotokeep.keep" => &["keep", "健身", "运动"],
        "com.wudaokou.hippo" => &["hema", "盒马", "盒马鲜生"],
        "com.ss.android.ugc.aweme" => &["douyin", "tiktok", "dy", "抖音", "短视频"],
        "com.smile.gifmaker" => &["kuaishou", "ks", "快手", "短视频"],
        "com.netease.cloudmusic" => &["netease", "cloudmusic", "网易云", "网易云音乐", "音乐"],
        "com.tencent.qqmusic" => &["qqmusic", "qq音乐", "音乐"],
        "com.kugou.android" => &["kugou", "酷狗", "酷狗音乐", "音乐"],
        "com.luna.music" => &["qishui", "汽水", "汽水音乐", "抖音音乐"],
        "com.tencent.qqlive" => &["qqlive", "腾讯视频", "视频"],
        "com.qiyi.video" => &["iqiyi", "爱奇艺", "视频"],
        "com.youku.phone" => &["youku", "优酷", "优酷视频"],
        "com.ximalaya.ting.android" => &["ximalaya", "喜马拉雅", "听书", "播客"],
        "app.podcast.cosmos" => &["cosmos", "小宇宙", "播客", "podcast"],
        "com.tencent.weread" => &["weread", "微信读书", "阅读", "电子书"],
        "com.dragon.read" => &["fanqie", "番茄", "番茄小说", "番茄免费小说"],
        "org.localsend.localsend_app" => &["localsend", "局域网互传", "互传", "隔空投送"],
        "org.videolan.vlc" => &["vlc", "videolan", "播放器"],
        "org.jellyfin.mobile" => &["jellyfin", "emby", "plex", "家庭影院", "海报墙"],
        "com.termux" => &["termux", "terminal", "ssh", "linux", "终端"],
        "com.x8bit.bitwarden" => &["bitwarden", "密码", "密码管理"],
        "com.agilebits.onepassword" => &["1password", "onepassword", "密码", "密码管理"],
        "com.follow.clash" => &["flclash", "clash", "clash meta", "mihomo", "代理"],
        "org.kde.kdeconnect_tp" => &["kdeconnect", "kde connect", "互联", "跨屏"],
        "ru.zdevs.zarchiver" => &["zarchiver", "解压", "压缩包", "7z", "zip"],
        _ => &[],
    }
}

pub fn expand_upstream_query(query: &str) -> String {
    let q = query.trim().to_lowercase();
    match q.as_str() {
        "x" | "tw" | "推特" | "x.com" | "马斯克" => "twitter".to_string(),
        "fb" | "脸书" => "facebook".to_string(),
        "ins" | "ig" | "insta" | "照片墙" => "instagram".to_string(),
        "tg" | "电报" | "纸飞机" => "telegram".to_string(),
        "yt" | "ytb" | "油管" => "youtube".to_string(),
        "wa" => "whatsapp".to_string(),
        "dc" => "discord".to_string(),
        "gpt" | "gpt4" | "gpt4o" | "o1" | "o3" | "openai" | "人工智能" => {
            "chatgpt".to_string()
        }
        "克劳德" | "anthropic" | "sonnet" | "opus" | "haiku" => "claude".to_string(),
        "bard" | "google ai" | "雙子座" | "双子座" => "gemini".to_string(),
        "pplx" | "ai search" | "ai搜索" => "perplexity".to_string(),
        "xai" | "x.ai" | "马斯克ai" => "grok".to_string(),
        "bing ai" | "微软ai" | "microsoft ai" => "copilot".to_string(),
        "quora ai" => "poe".to_string(),
        "openai video" | "ai视频" => "sora".to_string(),
        "红迪" => "reddit".to_string(),
        "声破天" => "spotify".to_string(),
        "谷歌浏览器" => "chrome".to_string(),
        "wx" | "wechat" => "微信".to_string(),
        "zfb" | "alipay" => "支付宝".to_string(),
        "b站" | "bilibili" => "哔哩哔哩".to_string(),
        "xhs" | "rednote" => "小红书".to_string(),
        "dy" => "抖音".to_string(),
        "ks" => "快手".to_string(),
        "pdd" => "拼多多".to_string(),
        "jd" => "京东".to_string(),
        "tb" => "淘宝".to_string(),
        "企微" => "企业微信".to_string(),
        _ => query.trim().to_string(),
    }
}

pub fn score_app_relevance(app: &StoreApp, query: &str) -> u32 {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return 1;
    }
    let name_lower = app.name.to_lowercase();
    let pkg_lower = app.package.to_lowercase();
    let aliases = package_aliases(&app.package);

    if q.chars().count() == 1 {
        if name_lower == q
            || aliases.contains(&q.as_str())
            || name_lower
                .split(|c: char| !c.is_alphanumeric())
                .any(|tok| !tok.is_empty() && tok == q)
        {
            return 1000;
        }
        return 0;
    }

    if name_lower == q || pkg_lower == q {
        return 1100;
    }
    if aliases.first().is_some_and(|a| a.to_lowercase() == q)
        || (aliases.iter().any(|a| a.to_lowercase() == q)
            && (name_lower.contains(&q) || pkg_lower.contains(&q)))
    {
        return 1050;
    }
    if aliases.iter().any(|a| a.to_lowercase() == q) {
        return 1000;
    }
    if name_lower.starts_with(&q) || aliases.iter().any(|a| a.to_lowercase().starts_with(&q)) {
        return 850;
    }
    if name_lower.contains(&q) || aliases.iter().any(|a| a.to_lowercase().contains(&q)) {
        return 700;
    }
    if pkg_lower.contains(&q) {
        return 450;
    }
    if app.developer.to_lowercase().contains(&q) {
        return 350;
    }
    if app.tagline.to_lowercase().contains(&q) || app.category_label.to_lowercase().contains(&q) {
        return 200;
    }
    0
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreSourceOffer {
    pub source: StoreSource,
    pub version_name: String,
    pub version_code: u64,
    pub size_bytes: u64,
    pub download_url: String,
    pub updated_at: String,
    pub md5: String,
    pub changelog: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalVersion {
    pub version_name: String,
    pub version_code: u64,
    pub size_bytes: u64,
    pub size_label: String,
    pub release_date: String,
    pub source: StoreSource,
    pub download_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedAppItem {
    pub package: String,
    pub name: String,
    pub version_name: String,
    pub size_bytes: u64,
    pub icon_url: String,
    pub cached_icon: Option<PathBuf>,
    pub developer: String,
    pub tagline: String,
    pub source: StoreSource,
    pub download_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoreApp {
    pub package: String,
    pub name: String,
    pub developer: String,
    pub category: StoreCategory,
    pub category_label: String,
    pub tagline: String,
    pub description: String,
    pub changelog: String,
    pub icon_url: String,
    pub cached_icon: Option<PathBuf>,
    pub screenshots: Vec<String>,
    pub cached_screenshots: Vec<PathBuf>,
    pub rating: Option<f32>,
    pub download_count_label: String,
    pub icp_number: String,
    pub featured: bool,
    pub allow_system_update: bool,
    pub sources: Vec<StoreSourceOffer>,
    pub selected_source_idx: usize,
    pub historical_versions: Vec<HistoricalVersion>,
    pub developer_apps: Vec<RelatedAppItem>,
    pub detail_loaded: bool,
}

impl StoreApp {
    pub fn active_offer(&self) -> Option<&StoreSourceOffer> {
        self.sources
            .get(self.selected_source_idx)
            .or_else(|| self.sources.first())
    }

    pub fn primary_source(&self) -> StoreSource {
        self.active_offer()
            .map(|o| o.source)
            .unwrap_or(StoreSource::Tencent)
    }

    pub fn version_name(&self) -> &str {
        self.active_offer()
            .map(|o| o.version_name.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("最新版")
    }

    pub fn version_code(&self) -> u64 {
        self.active_offer().map(|o| o.version_code).unwrap_or(0)
    }

    pub fn size_bytes(&self) -> u64 {
        self.active_offer().map(|o| o.size_bytes).unwrap_or(0)
    }

    pub fn download_url(&self) -> &str {
        self.active_offer()
            .map(|o| o.download_url.as_str())
            .unwrap_or("")
    }

    pub fn updated_at(&self) -> &str {
        self.active_offer()
            .map(|o| o.updated_at.as_str())
            .unwrap_or("")
    }

    pub fn matches_query(&self, query: &str) -> bool {
        if query.trim().is_empty() {
            return true;
        }
        score_app_relevance(self, query) > 0
    }
}

pub fn category_sub_tags(cat: StoreCategory) -> &'static [&'static str] {
    match cat {
        StoreCategory::All => &[
            "装机必备", "AI 助手", "NAS 私有云", "视频影视", "音乐听书", "协同办公",
        ],
        StoreCategory::Social => &["即时通讯", "社区论坛", "兴趣同好", "语音交友", "国际社交"],
        StoreCategory::Media => &[
            "视频影视", "音乐电台", "短视频直播", "小说听书", "图像摄影", "本地播放",
        ],
        StoreCategory::Productivity => &[
            "协同办公", "AI 助手", "文档笔记", "网盘存储", "教育学习", "邮箱扫描",
        ],
        StoreCategory::Tools => &[
            "系统工具", "NAS 私有云", "浏览器", "输入法", "文件解压", "安全网络",
        ],
        StoreCategory::Lifestyle => &[
            "网购商城", "外卖美食", "地图导航", "打车差旅", "金融理财", "生活服务",
        ],
        StoreCategory::Updatable | StoreCategory::Installed => &[],
    }
}

pub fn app_matches_sub_tag(app: &StoreApp, sub_tag: &str) -> bool {
    let tag = sub_tag.trim();
    if tag.is_empty() || tag == "全部" {
        return true;
    }
    if app.category_label.trim() == tag {
        return true;
    }
    let hay = format!(
        "{} {} {} {} {} {}",
        app.category_label,
        app.name,
        app.tagline,
        app.description,
        app.package,
        package_aliases(&app.package).join(" "),
    )
    .to_lowercase();

    let has_any = |kws: &[&str]| kws.iter().any(|k| hay.contains(&k.to_lowercase()));

    match tag {
        "装机必备" => {
            app.featured
                || app.category_label.contains("必备")
                || has_any(&["装机必备", "国民级"])
        }
        "AI 助手" => has_any(&[
            "大模型", "智能助手", "ai 助手", "ai助手", "人工智能助手", "deepseek", "豆包", "doubao",
            "kimi", "moonshot", "文心", "yiyan", "元宝", "yuanbao", "千问", "通义", "qwen",
            "tongyi", "智谱", "清言", "chatglm", "星火", "spark", "chatgpt", "openai",
            "claude", "gemini", "bard", "perplexity", "grok", "copilot", "poe", "sora",
            "anthropic", "xai", "天工", "tiangong", "纳米ai", "即梦", "可灵", "海螺", "minimax",
            "秘塔", "metaso", "跃问", "stepfun", "深度求索", "月之暗面",
        ]),
        "NAS 私有云" => has_any(&[
            "nas", "私有云", "绿联", "极空间", "群晖", "飞牛", "synology", "nextcloud",
            "immich", "jellyfin", "ugreen", "zspace", "fnos", "feiniu", "dsfile", "dsfinder",
            "tailscale", "zerotier", "家庭数据中心", "威联通", "qnap", "铁威马", "海康智存",
        ]),
        "视频影视" => has_any(&[
            "视频", "影视", "电影", "电视剧", "追剧", "动漫", "番剧", "腾讯视频", "爱奇艺",
            "优酷", "哔哩哔哩", "bilibili", "芒果tv", "央视频", "咪咕视频", "搜狐视频",
            "好看视频", "西瓜视频", "韩剧", "弹幕", "影院", "netflix", "youtube",
        ]),
        "音乐听书" => has_any(&[
            "音乐", "听歌", "听书", "电台", "播客", "k歌", "歌词", "无损", "网易云音乐",
            "qq音乐", "酷狗", "酷我", "汽水音乐", "喜马拉雅", "小宇宙", "微信读书", "番茄畅听",
            "懒人听书", "全民k歌", "唱吧", "spotify", "apple music", "咪咕音乐",
        ]),
        "音乐电台" => has_any(&[
            "音乐", "听歌", "电台", "播客", "k歌", "歌词", "无损", "网易云音乐", "qq音乐",
            "酷狗", "酷我", "汽水音乐", "喜马拉雅", "小宇宙", "全民k歌", "唱吧", "spotify",
            "apple music", "咪咕音乐", "猫耳", "荔枝", "蜻蜓",
        ]),
        "短视频直播" => has_any(&[
            "短视频", "直播", "抖音", "快手", "斗鱼", "虎牙", "视频号", "映客", "花椒",
            "火山", "tiktok", "twitch", "微视", "yy",
        ]),
        "小说听书" => has_any(&[
            "小说", "听书", "阅读", "读书", "漫画", "电子书", "名著", "有声", "微信读书",
            "番茄免费小说", "番茄小说", "七猫", "起点", "晋江", "快看漫画", "喜马拉雅",
            "番茄畅听", "书旗", "掌阅", "漫客",
        ]),
        "图像摄影" => has_any(&[
            "相机", "摄影", "修图", "剪辑", "相册", "美颜", "滤镜", "拍照", "拼图", "剪映",
            "美图秀秀", "醒图", "轻颜", "b612", "必剪", "快影", "wink", "黄油相机",
            "snapseed", "lightroom", "photoshop", "canva", "稿定", "immich",
        ]),
        "本地播放" => has_any(&[
            "播放器", "本地播放", "解码", "投屏", "字幕", "家庭影院", "海报墙", "vlc",
            "jellyfin", "emby", "plex", "kodi", "mx player", "nplayer", "mpv", "乐播投屏",
            "暴风影音", "恒星播放器", "oplayer",
        ]),
        "社交通讯" => {
            app.category == StoreCategory::Social
                || has_any(&[
                    "社交", "通讯", "聊天", "社区", "论坛", "微博", "微信", "qq", "小红书",
                    "知乎", "贴吧", "豆瓣", "酷安", "soul", "telegram", "twitter", "facebook",
                    "instagram", "whatsapp", "discord", "陌陌", "探探",
                ])
        }
        "即时通讯" => has_any(&[
            "即时通讯", "聊天", "通话", "群聊", "消息", "微信", "qq", "tim", "telegram",
            "whatsapp", "messenger", "discord", "line", "signal", "易信", "连信", "米聊",
        ]),
        "社区论坛" => has_any(&[
            "社区", "论坛", "问答", "热搜", "微博", "知乎", "小红书", "酷安", "贴吧", "虎扑",
            "豆瓣", "nga", "v2ex", "雪球", "懂球帝", "汽车之家", "懂车帝", "taptap", "reddit",
        ]),
        "兴趣同好" => has_any(&[
            "兴趣", "同好", "次元", "圈子", "手办", "潮玩", "贴吧", "豆瓣", "虎扑", "小红书",
            "lofter", "半次元", "米游社", "小黑盒", "taptap", "森空岛", "库街区", "王者营地",
            "千岛",
        ]),
        "语音交友" => has_any(&[
            "语音", "交友", "恋爱", "相亲", "派对", "扩列", "连麦", "陪玩", "同城", "soul",
            "陌陌", "探探", "tt语音", "oopz", "kook", "discord", "伊对", "他趣", "世纪佳缘",
            "珍爱", "比心", "会玩",
        ]),
        "国际社交" => {
            (app.category == StoreCategory::Social && app.primary_source() == StoreSource::Global)
                || has_any(&[
                    "国际社交", "全球社交", "海外社交", "twitter", "facebook", "instagram",
                    "telegram", "whatsapp", "discord", "reddit", "messenger", "snapchat",
                    "threads", "linkedin", "line", "kakao", "pinterest",
                ])
        }
        "协同办公" => has_any(&[
            "协同", "办公", "会议", "打卡", "考勤", "审批", "工作台", "企业", "飞书", "钉钉",
            "企业微信", "腾讯会议", "zoom", "teams", "slack", "welink", "向日葵", "todesk",
            "远程桌面",
        ]),
        "文档笔记" => has_any(&[
            "文档", "表格", "笔记", "便签", "知识库", "思维导图", "大纲", "写作", "markdown",
            "office", "wps", "腾讯文档", "石墨文档", "语雀", "notion", "obsidian", "印象笔记",
            "有道云笔记", "flomo", "幕布", "xmind", "敬业签", "滴答清单", "notability",
            "goodnotes", "word", "excel", "ppt",
        ]),
        "网盘存储" => has_any(&[
            "网盘", "云盘", "云存储", "备份", "传输", "同步盘", "百度网盘", "阿里云盘", "夸克",
            "中国移动云盘", "天翼云盘", "123云盘", "迅雷云盘", "微云", "onedrive",
            "google drive", "dropbox", "pikpak", "坚果云",
        ]),
        "教育学习" => has_any(&[
            "学习", "教育", "词典", "翻译", "背单词", "外语", "英语", "考公", "考研", "驾考",
            "题库", "网课", "作业", "课程", "百词斩", "多邻国", "不背单词", "墨墨背单词",
            "有道词典", "欧路词典", "deepl", "腾讯翻译", "学习通", "mooc", "粉笔", "驾考宝典",
            "猿辅导", "作业帮", "小猿", "宝宝巴士", "洪恩",
        ]),
        "邮箱扫描" => has_any(&[
            "邮箱", "邮件", "扫描", "pdf", "名片", "传真", "打印", "ocr", "取字", "qq邮箱",
            "网易邮箱", "邮箱大师", "139邮箱", "阿里邮箱", "outlook", "gmail", "全能扫描王",
            "扫描全能王", "白描", "adobe", "foxit", "福昕",
        ]),
        "系统工具" => has_any(&[
            "系统", "工具", "管家", "清理", "加速", "终端", "连接", "跑分", "检测", "框架",
            "root", "adb", "shell", "termux", "kde", "安兔兔", "鲁大师", "换机", "克隆",
            "手机搬家", "电量", "壁纸", "桌面", "启动器", "shizuku", "scene", "腾讯手机管家",
        ]),
        "浏览器" => has_any(&[
            "浏览器", "搜索", "上网", "网页", "内核", "chrome", "edge", "firefox", "via",
            "夸克", "uc浏览器", "qq浏览器", "百度", "必应", "bing", "opera", "brave", "kiwi",
            "x浏览器", "alook",
        ]),
        "输入法" => has_any(&[
            "输入法", "键盘", "打字", "五笔", "拼音", "语音输入", "手写", "搜狗输入法",
            "讯飞输入法", "百度输入法", "微信输入法", "qq输入法", "gboard", "同文输入法",
            "章鱼输入法", "kk键盘",
        ]),
        "文件解压" => has_any(&[
            "文件", "解压", "压缩", "互传", "快传", "管理器", "归档", "zip", "7z", "rar",
            "zarchiver", "mt管理器", "es文件", "solid explorer", "localsend", "快牙",
            "茄子快传", "互传", "闪传", "ftp",
        ]),
        "安全网络" => has_any(&[
            "安全", "网络", "wifi", "测速", "密码", "代理", "组网", "内网穿透", "vpn",
            "防火墙", "杀毒", "反诈", "验证器", "计算器", "天气", "日历", "万年历", "时钟",
            "闹钟", "手电筒", "指南针", "录音", "bitwarden", "1password", "keepass", "clash",
            "flclash", "tailscale", "zerotier", "国家反诈中心", "wifi万能钥匙", "speedtest",
            "全球网测", "墨迹天气", "彩云天气",
        ]),
        "网购外卖" => has_any(&[
            "网购", "购物", "商城", "电商", "外卖", "美食", "买菜", "超市", "二手", "团购",
            "淘宝", "京东", "拼多多", "闲鱼", "得物", "美团", "饿了么", "大众点评", "盒马",
            "唯品会", "天猫", "苏宁", "当当", "转转", "叮咚买菜", "朴朴", "山姆", "瑞幸",
            "星巴克", "肯德基", "麦当劳",
        ]),
        "网购商城" => has_any(&[
            "网购", "购物", "商城", "电商", "二手", "潮牌", "特卖", "百货", "淘宝", "京东",
            "拼多多", "闲鱼", "得物", "唯品会", "天猫", "阿里巴巴", "1688", "苏宁", "当当",
            "转转", "小米商城", "华为商城", "多抓鱼",
        ]),
        "外卖美食" => has_any(&[
            "外卖", "美食", "买菜", "生鲜", "咖啡", "奶茶", "点餐", "菜谱", "餐饮", "超市",
            "团购", "美团", "饿了么", "大众点评", "盒马", "叮咚买菜", "朴朴", "山姆",
            "下厨房", "瑞幸", "星巴克", "库迪", "喜茶", "霸王茶姬", "蜜雪冰城", "肯德基",
            "麦当劳", "必胜客", "海底捞",
        ]),
        "地图出行" => has_any(&[
            "地图", "导航", "打车", "出行", "公交", "地铁", "火车票", "高铁", "机票", "航班",
            "酒店", "旅游", "顺风车", "代驾", "租车", "高德地图", "百度地图", "腾讯地图",
            "滴滴", "12306", "携程", "航旅纵横", "飞猪", "去哪儿", "同程", "哈啰", "嘀嗒",
            "曹操", "t3出行", "奥维互动地图",
        ]),
        "地图导航" => has_any(&[
            "地图", "导航", "路况", "公交", "地铁", "卫星", "街景", "违章", "停车", "充电桩",
            "高德地图", "百度地图", "腾讯地图", "奥维互动地图", "车来了", "掌上公交",
            "交管12123", "e充电", "特来电",
        ]),
        "打车差旅" => has_any(&[
            "打车", "出行", "火车票", "高铁", "机票", "航班", "值机", "酒店", "民宿", "旅游",
            "差旅", "顺风车", "代驾", "租车", "单车", "滴滴", "12306", "携程", "航旅纵横",
            "飞猪", "去哪儿", "同程", "哈啰", "嘀嗒", "曹操", "t3出行", "花小猪", "一嗨租车",
            "神州租车", "华住会", "途家", "马蜂窝",
        ]),
        "金融理财" => has_any(&[
            "金融", "理财", "支付", "银行", "钱包", "信用卡", "股票", "证券", "基金", "记账",
            "征信", "社保", "公积金", "税务", "发票", "支付宝", "云闪付", "数字人民币",
            "招商银行", "工商银行", "建设银行", "农业银行", "中国银行", "交通银行",
            "邮储银行", "平安口袋银行", "同花顺", "东方财富", "天天基金", "雪球", "挖财",
            "鲨鱼记账", "随手记", "个人所得税",
        ]),
        "生活服务" => has_any(&[
            "生活", "服务", "运动", "健身", "健康", "医疗", "挂号", "快递", "物流", "招聘",
            "求职", "租房", "买房", "家政", "政务", "运营商", "水电", "宠物", "母婴", "汽车",
            "keep", "菜鸟", "顺丰", "京东快递", "boss直聘", "智联招聘", "前程无忧", "脉脉",
            "贝壳找房", "安居客", "自如", "58同城", "天眼查", "企查查", "中国移动",
            "中国联通", "中国电信", "网上国网", "丁香医生", "薄荷健康", "美柚",
        ]),
        _ => hay.contains(&tag.to_lowercase()),
    }
}

pub fn infer_category(name: &str, package: &str, summary: &str) -> (StoreCategory, String) {
    let hay = format!(
        "{} {} {} {}",
        name,
        package,
        summary,
        package_aliases(package).join(" ")
    )
    .to_lowercase();
    let has_any = |kws: &[&str]| kws.iter().any(|k| hay.contains(k));

    if has_any(&[
        "chatgpt",
        "openai",
        "claude",
        "anthropic",
        "gemini",
        "bard",
        "perplexity",
        "grok",
        "copilot",
        "poe",
        "sora",
        "deepseek",
        "kimi",
        "doubao",
        "豆包",
        "大模型",
        "智能助手",
        "ai chat",
        "ai assistant",
    ]) {
        return (StoreCategory::Productivity, "AI 助手".to_string());
    }
    if has_any(&[
        "nas",
        "synology",
        "ugreen",
        "zspace",
        "feiniu",
        "fnos",
        "nextcloud",
        "immich",
        "tailscale",
        "zerotier",
        "绿联",
        "极空间",
        "群晖",
        "飞牛",
    ]) {
        return (StoreCategory::Tools, "NAS 私有云".to_string());
    }
    if has_any(&[
        "browser",
        "chrome",
        "firefox",
        "edge",
        "opera",
        "brave",
        "webview",
        "浏览器",
    ]) {
        return (StoreCategory::Tools, "浏览器".to_string());
    }
    if has_any(&[
        "termux",
        "terminal",
        "ssh",
        "adb",
        "shizuku",
        "kdeconnect",
        "syncthing",
        "keepass",
        "bitwarden",
        "password",
        "authenticator",
        "vpn",
        "wireguard",
        "clash",
        "proxy",
        "zarchiver",
        "localsend",
        "file manager",
        "archive",
        "unzip",
    ]) {
        return (StoreCategory::Tools, "系统工具".to_string());
    }
    if has_any(&[
        "vlc",
        "jellyfin",
        "plex",
        "emby",
        "kodi",
        "mpv",
        "youtube",
        "netflix",
        "spotify",
        "music",
        "video",
        "player",
        "podcast",
        "audio",
        "camera",
        "photo",
        "gallery",
        "短视频",
        "音乐",
        "视频",
        "影视",
    ]) {
        return (StoreCategory::Media, "视频影视".to_string());
    }
    if has_any(&[
        "notion",
        "obsidian",
        "markdown",
        "note",
        "office",
        "wps",
        "word",
        "excel",
        "pdf",
        "mail",
        "gmail",
        "outlook",
        "calendar",
        "todo",
        "slack",
        "zoom",
        "teams",
        "lark",
        "dingtalk",
        "文档",
        "笔记",
        "办公",
    ]) {
        return (StoreCategory::Productivity, "文档笔记".to_string());
    }
    if has_any(&[
        "twitter",
        "facebook",
        "instagram",
        "telegram",
        "whatsapp",
        "discord",
        "reddit",
        "messenger",
        "signal",
        "mastodon",
        "wechat",
        "chat",
        "social",
        "forum",
        "社交",
        "聊天",
    ]) {
        return (StoreCategory::Social, "即时通讯".to_string());
    }
    if has_any(&[
        "map",
        "navigation",
        "shop",
        "shopping",
        "delivery",
        "travel",
        "hotel",
        "flight",
        "bank",
        "wallet",
        "alipay",
        "fitness",
        "weather",
        "地图",
        "外卖",
        "购物",
    ]) {
        return (StoreCategory::Lifestyle, "生活服务".to_string());
    }
    (StoreCategory::Tools, "实用工具".to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreInstallState {
    NotInstalled,
    Installed {
        installed_version: String,
        installed_code: u64,
        can_uninstall: bool,
    },
    Updatable {
        installed_version: String,
        installed_code: u64,
        latest_version: String,
        latest_code: u64,
        can_uninstall: bool,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StoreAppDetailPayload {
    pub package: String,
    pub developer: String,
    pub description: String,
    pub changelog: String,
    pub icp_number: String,
    pub rating: Option<f32>,
    pub download_count_label: String,
    pub icon_url: String,
    pub cached_icon: Option<PathBuf>,
    pub screenshots: Vec<String>,
    pub cached_screenshots: Vec<PathBuf>,
    pub extra_sources: Vec<StoreSourceOffer>,
    pub historical_versions: Vec<HistoricalVersion>,
    pub developer_apps: Vec<RelatedAppItem>,
}

fn parse_version_segments(ver: &str) -> Vec<u64> {
    let trimmed = ver
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['+', ' '])
        .next()
        .unwrap_or("");
    let mut nums = Vec::new();
    let mut cur = String::new();
    for ch in trimmed.chars() {
        if ch.is_ascii_digit() {
            cur.push(ch);
        } else if !cur.is_empty() {
            if let Ok(n) = cur.parse::<u64>() {
                nums.push(n);
            }
            cur.clear();
        }
    }
    if !cur.is_empty() {
        if let Ok(n) = cur.parse::<u64>() {
            nums.push(n);
        }
    }
    nums
}

pub fn compare_android_versions(installed_ver: &str, store_ver: &str) -> Ordering {
    let a = parse_version_segments(installed_ver);
    let b = parse_version_segments(store_ver);
    if a.is_empty() || b.is_empty() {
        return Ordering::Equal;
    }
    let max_len = a.len().max(b.len());
    for i in 0..max_len {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        match av.cmp(&bv) {
            Ordering::Equal => continue,
            non_eq => return non_eq,
        }
    }
    Ordering::Equal
}

pub fn is_eligible_for_store_update(installed: &AppInfo, store: &StoreApp) -> bool {
    if !installed.enabled {
        return false;
    }
    let pkg = installed.package.as_str();
    let is_core_rom = pkg == "android"
        || (pkg.starts_with("com.android.") && pkg != "com.android.chrome")
        || pkg.starts_with("android.")
        || pkg.starts_with("vendor.")
        || pkg.starts_with("com.qti.")
        || pkg.starts_with("com.qualcomm.")
        || pkg.starts_with("com.stepos.")
        || pkg.starts_with("com.stepx.");
    if is_core_rom {
        return false;
    }
    if installed.system && !installed.updated_system && !store.allow_system_update {
        return false;
    }
    true
}

pub fn check_store_install_state(installed_apps: &[AppInfo], store: &StoreApp) -> StoreInstallState {
    let Some(installed) = installed_apps.iter().find(|a| a.package == store.package) else {
        return StoreInstallState::NotInstalled;
    };
    let can_uninstall = !installed.system || installed.updated_system;
    let installed_version = if installed.version_name.trim().is_empty() {
        format!("{}", installed.version_code)
    } else {
        installed.version_name.trim().to_string()
    };
    let installed_code = installed.version_code;
    let latest_version = store.version_name().trim().to_string();
    let latest_code = store.version_code();

    if is_eligible_for_store_update(installed, store) {
        let newer_by_code =
            latest_code > 0 && installed_code > 0 && latest_code > installed_code;
        let newer_by_semver = !latest_version.is_empty()
            && latest_version != "最新版"
            && compare_android_versions(&installed_version, &latest_version) == Ordering::Less;
        if newer_by_code || newer_by_semver {
            return StoreInstallState::Updatable {
                installed_version,
                installed_code,
                latest_version,
                latest_code,
                can_uninstall,
            };
        }
    }

    StoreInstallState::Installed {
        installed_version,
        installed_code,
        can_uninstall,
    }
}

pub fn merge_and_deduplicate_apps(apps: Vec<StoreApp>) -> Vec<StoreApp> {
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, StoreApp> = HashMap::new();

    for mut incoming in apps {
        let pkg = incoming.package.trim().to_string();
        if pkg.is_empty() {
            continue;
        }
        incoming.package = pkg.clone();
        if let Some(existing) = map.get_mut(&pkg) {
            if existing.name.trim().is_empty() && !incoming.name.trim().is_empty() {
                existing.name = incoming.name;
            }
            if (existing.developer.trim().is_empty() || existing.developer == "未知开发者")
                && !incoming.developer.trim().is_empty()
            {
                existing.developer = incoming.developer;
            }
            if existing.tagline.trim().is_empty() && !incoming.tagline.trim().is_empty() {
                existing.tagline = incoming.tagline;
            }
            if existing.description.len() < incoming.description.len() {
                existing.description = incoming.description;
            }
            if existing.changelog.trim().is_empty() && !incoming.changelog.trim().is_empty() {
                existing.changelog = incoming.changelog;
            }
            if (existing.icon_url.trim().is_empty()
                || existing.icon_url.contains("sj.qq.com/appdetail"))
                && !incoming.icon_url.trim().is_empty()
                && !incoming.icon_url.contains("sj.qq.com/appdetail")
            {
                existing.icon_url = incoming.icon_url;
            }
            if existing.cached_icon.is_none() && incoming.cached_icon.is_some() {
                existing.cached_icon = incoming.cached_icon;
            }
            if existing.screenshots.is_empty() && !incoming.screenshots.is_empty() {
                existing.screenshots = incoming.screenshots;
            }
            if existing.cached_screenshots.is_empty() && !incoming.cached_screenshots.is_empty() {
                existing.cached_screenshots = incoming.cached_screenshots;
            }
            if existing.rating.is_none() && incoming.rating.is_some() {
                existing.rating = incoming.rating;
            }
            if existing.download_count_label.trim().is_empty()
                && !incoming.download_count_label.trim().is_empty()
            {
                existing.download_count_label = incoming.download_count_label;
            }
            if existing.icp_number.trim().is_empty() && !incoming.icp_number.trim().is_empty() {
                existing.icp_number = incoming.icp_number;
            }
            existing.featured |= incoming.featured;
            existing.allow_system_update |= incoming.allow_system_update;
            existing.detail_loaded |= incoming.detail_loaded;

            for offer in incoming.sources {
                if let Some(cur) = existing.sources.iter_mut().find(|s| s.source == offer.source) {
                    let cur_is_weak_url = cur.download_url.is_empty()
                        || cur.download_url.contains("sj.qq.com/appdetail")
                        || cur.download_url.contains("sjy.00008")
                        || cur.download_url.contains("sjy.00009");
                    let offer_is_direct_apk = !offer.download_url.is_empty()
                        && !offer.download_url.contains("sj.qq.com/appdetail")
                        && !offer.download_url.contains("sjy.00008")
                        && !offer.download_url.contains("sjy.00009")
                        && (offer.download_url.to_lowercase().contains(".apk")
                            || offer.download_url.contains("d.apkpure.com/b/APK/"));
                    let newer = compare_android_versions(&cur.version_name, &offer.version_name)
                        == Ordering::Less
                        || (cur.download_url.is_empty() && !offer.download_url.is_empty())
                        || (cur_is_weak_url && offer_is_direct_apk);
                    if newer {
                        *cur = offer;
                    }
                } else {
                    existing.sources.push(offer);
                }
            }
            existing.sources.sort_by(|a, b| {
                compare_android_versions(&b.version_name, &a.version_name)
                    .then_with(|| b.source.priority().cmp(&a.source.priority()))
            });
            if existing.selected_source_idx >= existing.sources.len() {
                existing.selected_source_idx = 0;
            }

            for hist in incoming.historical_versions {
                if !existing
                    .historical_versions
                    .iter()
                    .any(|h| h.version_name == hist.version_name)
                {
                    existing.historical_versions.push(hist);
                }
            }
            for dev_app in incoming.developer_apps {
                if dev_app.package != existing.package
                    && !existing
                        .developer_apps
                        .iter()
                        .any(|d| d.package == dev_app.package)
                {
                    existing.developer_apps.push(dev_app);
                }
            }
        } else {
            incoming.sources.sort_by(|a, b| {
                compare_android_versions(&b.version_name, &a.version_name)
                    .then_with(|| b.source.priority().cmp(&a.source.priority()))
            });
            order.push(pkg.clone());
            map.insert(pkg, incoming);
        }
    }

    order.into_iter().filter_map(|pkg| map.remove(&pkg)).collect()
}

fn store_cache_root() -> PathBuf {
    if let Some(home) = resources::user_home() {
        PathBuf::from(home)
            .join(".cache")
            .join("stepx-companion")
            .join("app-store")
    } else {
        std::env::temp_dir().join("stepx-companion-app-store")
    }
}

fn sanitize_cache_key(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn is_valid_raster_bytes(head: &[u8]) -> bool {
    if head.len() < 12 {
        return false;
    }
    let trimmed = head.trim_ascii_start();
    if trimmed.starts_with(b"<svg")
        || trimmed.starts_with(b"<!DOCTYPE")
        || trimmed.starts_with(b"<!doctype")
        || trimmed.starts_with(b"<html")
        || trimmed.starts_with(b"<HTML")
        || trimmed.starts_with(b"<?xml")
    {
        return false;
    }
    head.starts_with(b"\x89PNG")
        || head.starts_with(b"\xff\xd8\xff")
        || head.starts_with(b"GIF8")
        || (head.starts_with(b"RIFF") && &head[8..12] == b"WEBP")
}

fn is_valid_raster_file(path: &Path, min_bytes: u64) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() < min_bytes {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 32];
    let n = std::io::Read::read(&mut file, &mut head).unwrap_or(0);
    if is_valid_raster_bytes(&head[..n]) {
        true
    } else {
        let _ = fs::remove_file(path);
        false
    }
}

pub fn existing_cached_icon(package: &str) -> Option<PathBuf> {
    let path = store_cache_root()
        .join("icons")
        .join(format!("{}.png", sanitize_cache_key(package)));
    if is_valid_raster_file(&path, 128) {
        Some(path)
    } else {
        None
    }
}

pub fn existing_cached_screenshots(package: &str, count: usize) -> Vec<PathBuf> {
    let dir = store_cache_root().join("screenshots");
    let key = sanitize_cache_key(package);
    let mut out = Vec::new();
    for idx in 0..count {
        let p = dir.join(format!("{key}_{idx}.jpg"));
        if is_valid_raster_file(&p, 256) {
            out.push(p);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn build_curated_app(
    package: &str,
    name: &str,
    developer: &str,
    category: StoreCategory,
    category_label: &str,
    tagline: &str,
    description: &str,
    changelog: &str,
    icon_url: &str,
    screenshots: Vec<&str>,
    rating: f32,
    download_count_label: &str,
    icp_number: &str,
    featured: bool,
    allow_system_update: bool,
    sources: Vec<StoreSourceOffer>,
    historical_versions: Vec<HistoricalVersion>,
    developer_apps: Vec<RelatedAppItem>,
) -> StoreApp {
    let shot_urls: Vec<String> = screenshots.into_iter().map(String::from).collect();
    let cached_shots = existing_cached_screenshots(package, shot_urls.len());
    StoreApp {
        package: package.into(),
        name: name.into(),
        developer: developer.into(),
        category,
        category_label: category_label.into(),
        tagline: tagline.into(),
        description: description.into(),
        changelog: changelog.into(),
        icon_url: icon_url.into(),
        cached_icon: existing_cached_icon(package),
        screenshots: shot_urls,
        cached_screenshots: cached_shots,
        rating: Some(rating),
        download_count_label: download_count_label.into(),
        icp_number: icp_number.into(),
        featured,
        allow_system_update,
        sources,
        selected_source_idx: 0,
        historical_versions,
        developer_apps,
        detail_loaded: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn quick_curated_app(
    package: &str,
    name: &str,
    developer: &str,
    category: StoreCategory,
    tagline: &str,
    description: &str,
    icon_url: &str,
    rating: f32,
    download_count_label: &str,
    featured: bool,
    source: StoreSource,
    version_name: &str,
    size_bytes: u64,
    download_url: &str,
) -> StoreApp {
    build_curated_app(
        package,
        name,
        developer,
        category,
        category.label(),
        tagline,
        description,
        "1. 优化界面交互与启动响应速度；\n2. 修复已知问题并提升 Android 系统兼容性与稳定性。",
        icon_url,
        vec![],
        rating,
        download_count_label,
        "",
        featured,
        false,
        vec![StoreSourceOffer {
            source,
            version_name: version_name.into(),
            version_code: 0,
            size_bytes,
            download_url: download_url.into(),
            updated_at: "2026-04-10".into(),
            md5: String::new(),
            changelog: format!("{}应用分发", source.short_label()),
        }],
        vec![],
        vec![],
    )
}

pub fn curated_catalog() -> Vec<StoreApp> {
    let ugreen_related = vec![
        RelatedAppItem {
            package: "com.ugreen.iot.connect".into(),
            name: "绿联智联".into(),
            version_name: "1.5.2.0".into(),
            size_bytes: 58_412_032,
            icon_url: "https://pp.myapp.com/ma_icon/0/icon_54646330_1792983907/256".into(),
            cached_icon: existing_cached_icon("com.ugreen.iot.connect"),
            developer: "深圳市绿联科技股份有限公司".into(),
            tagline: "绿联智能穿戴、防丢器与音频外设全场景互联管理中心".into(),
            source: StoreSource::Tencent,
            download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/74C18FE05EA3EEB60F0983C7B0B131F2.apk?fsname=com.ugreen.iot.connect_1.5.2.0.apk".into(),
        },
        RelatedAppItem {
            package: "com.ugreen.nas".into(),
            name: "绿联私有云（旧代系统版）".into(),
            version_name: "5.2.0".into(),
            size_bytes: 112_400_000,
            icon_url: "https://pp.myapp.com/ma_icon/0/icon_54386980_1792032596/256".into(),
            cached_icon: existing_cached_icon("com.ugreen.nas"),
            developer: "深圳市绿联科技股份有限公司".into(),
            tagline: "适用于初代绿联 DH2100 / DH2600 / DX4600 非 UGOS Pro 固件机型".into(),
            source: StoreSource::Official,
            download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/70512A6D4341A411E59DFB763D40E8E5.apk?fsname=com.ugreen.nas_5.2.0.apk".into(),
        },
    ];

    let synology_related = vec![
        RelatedAppItem {
            package: "com.synology.dsdrive".into(),
            name: "Synology Drive".into(),
            version_name: "3.9.0-1117".into(),
            size_bytes: 76_521_000,
            icon_url: "https://pp.myapp.com/ma_icon/0/icon_52651842_1721000000/256".into(),
            cached_icon: existing_cached_icon("com.synology.dsdrive"),
            developer: "群晖科技 (Synology Inc.)".into(),
            tagline: "群晖私有云团队协作网盘与多端文件实时同步".into(),
            source: StoreSource::Official,
            download_url: "https://global.synologydownload.com/download/Mobile/Android-Drive/3.9.0-1117/Android-SynologyDrive.3.9.0-1117.cn.apk".into(),
        },
        RelatedAppItem {
            package: "com.synology.projectkailash".into(),
            name: "Synology Photos".into(),
            version_name: "2.4.2-666".into(),
            size_bytes: 84_120_000,
            icon_url: "https://pp.myapp.com/ma_icon/0/icon_54012011_1721000000/256".into(),
            cached_icon: existing_cached_icon("com.synology.projectkailash"),
            developer: "群晖科技 (Synology Inc.)".into(),
            tagline: "DSM 7.x AI 智能人脸识别相册与手机原图无感备份".into(),
            source: StoreSource::Official,
            download_url: "https://global.synologydownload.com/download/Mobile/Android-Photos/2.4.2-666/Android-SynologyPhotos.2.4.2-666.cn.apk".into(),
        },
    ];

    vec![
        // 1. 绿联云 (UGOS Pro)
        build_curated_app(
            "com.ugreen.pro",
            "绿联云",
            "深圳市绿联科技股份有限公司",
            StoreCategory::Tools,
            "实用工具",
            "绿联 NAS 私有云 UGOS Pro 官方全能客户端 · 相册备份 / 影视中心 / Docker 管理",
            "绿联云是专为绿联 NAS 私有云（DXP 系列及 UGOS Pro 系统）打造的全平台管理客户端。支持手机相册原图自动备份、AI 智能相册分类、影视中心海报墙与蓝光直链播放、远程安全访问（UGREENlink）、Docker 容器与虚拟机远程运维、SMB/WebDAV 挂载以及多家庭成员权限协同管理，彻底解决手机内置商店无法搜索安装绿联 NAS 的痛点。",
            "1. 新增系统硬盘温度与风扇状态实时看板；\n2. 优化 AI 相册人物与宠物聚类识别准确度；\n3. 提升影视中心杜比视界与高码率 4K HEVC 播放流畅度；\n4. 修复远程穿透连接在弱网切换下的重连延迟。",
            "https://pp.myapp.com/ma_icon/0/icon_54386980_1792032596/256",
            vec![
                "https://pp.myapp.com/ma_pic2/0/shot_54386980_1_1792032596/0",
                "https://pp.myapp.com/ma_pic2/0/shot_54386980_2_1792032596/0",
                "https://pp.myapp.com/ma_pic2/0/shot_54386980_3_1792032596/0",
                "https://pp.myapp.com/ma_pic2/0/shot_54386980_4_1792032596/0",
                "https://pp.myapp.com/ma_pic2/0/shot_54386980_5_1792032596/0",
            ],
            4.8,
            "320 万+",
            "粤ICP备12028978号-12A",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "1.20.12".into(),
                    version_code: 12012,
                    size_bytes: 298_609_339,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/BCD26A69568248B4F5911F0A4F6B9445.apk?fsname=com.ugreen.pro_1.20.12.apk".into(),
                    updated_at: "2026-04-10".into(),
                    md5: "bcd26a69568248b4f5911f0a4f6b9445".into(),
                    changelog: "绿联官方 UGOS Pro 1.20.12 正式版直连通道".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::Tencent,
                    version_name: "1.20.12".into(),
                    version_code: 12012,
                    size_bytes: 298_609_339,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/BCD26A69568248B4F5911F0A4F6B9445.apk?fsname=com.ugreen.pro_1.20.12.apk".into(),
                    updated_at: "2026-04-10".into(),
                    md5: "bcd26a69568248b4f5911f0a4f6b9445".into(),
                    changelog: "应用宝全量实时源 CDN 极速分发".into(),
                },
            ],
            vec![
                HistoricalVersion {
                    version_name: "1.20.10".into(),
                    version_code: 12010,
                    size_bytes: 298_609_339,
                    size_label: "284.8 MB".into(),
                    release_date: "2026-03-19".into(),
                    source: StoreSource::Tencent,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/BCD26A69568248B4F5911F0A4F6B9445.apk?fsname=com.ugreen.pro_1.20.12.apk".into(),
                },
                HistoricalVersion {
                    version_name: "1.19.0".into(),
                    version_code: 11900,
                    size_bytes: 298_609_339,
                    size_label: "284.8 MB".into(),
                    release_date: "2026-02-14".into(),
                    source: StoreSource::Official,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/BCD26A69568248B4F5911F0A4F6B9445.apk?fsname=com.ugreen.pro_1.20.12.apk".into(),
                },
            ],
            ugreen_related.clone(),
        ),
        // 2. 极空间私有云
        build_curated_app(
            "com.zentraedi.zspace",
            "极空间",
            "北京天顶星智能信息技术有限公司",
            StoreCategory::Tools,
            "实用工具",
            "极空间私有云 ZOS 官方客户端 · 极影视海报墙 / 极相册 / 闪电传",
            "极空间私有云官方手机客户端，集成了备受好评的「极影视」智能刮削海报墙与原盘蓝光播放、「极相册」家庭共享与 AI 足迹整理、「极音乐」无损流媒体播放以及团队闪电互传功能。支持无公网 IP 高速内网穿透直连。",
            "1. 极影视新增多音轨与外挂 ASS 特效字幕实时偏移调节；\n2. 极相册优化 LivePhoto 实况照片备份与回放体验；\n3. 修复部分 Android 16 机型后台自动备份唤醒问题。",
            "https://pp.myapp.com/ma_icon/0/icon_54067175_1789912976/256",
            vec![
                "https://pp.myapp.com/ma_pic2/0/shot_53953463_1_1773915035/0",
                "https://pp.myapp.com/ma_pic2/0/shot_53953463_2_1773915035/0",
                "https://pp.myapp.com/ma_pic2/0/shot_53953463_3_1773915035/0",
                "https://pp.myapp.com/ma_pic2/0/shot_53953463_4_1773915035/0",
            ],
            4.8,
            "260 万+",
            "京ICP备19045231号-3A",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "2.5.644".into(),
                    version_code: 205644,
                    size_bytes: 280_404_132,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/25CDBE43A2102967729608A17AC7080D.apk?fsname=com.zentraedi.zspace_2.5.644.apk".into(),
                    updated_at: "2026-04-02".into(),
                    md5: "25cdbe43a2102967729608a17ac7080d".into(),
                    changelog: "极空间官方直连稳定版 v2.5.644".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::Tencent,
                    version_name: "2.5.644".into(),
                    version_code: 205644,
                    size_bytes: 280_404_132,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/25CDBE43A2102967729608A17AC7080D.apk?fsname=com.zentraedi.zspace_2.5.644.apk".into(),
                    updated_at: "2026-04-02".into(),
                    md5: "25cdbe43a2102967729608a17ac7080d".into(),
                    changelog: "应用宝高速分发源".into(),
                },
            ],
            vec![
                HistoricalVersion {
                    version_name: "2.5.640".into(),
                    version_code: 205640,
                    size_bytes: 280_404_132,
                    size_label: "267.4 MB".into(),
                    release_date: "2026-03-10".into(),
                    source: StoreSource::Tencent,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/25CDBE43A2102967729608A17AC7080D.apk?fsname=com.zentraedi.zspace_2.5.644.apk".into(),
                },
                HistoricalVersion {
                    version_name: "2.5.620".into(),
                    version_code: 205620,
                    size_bytes: 280_404_132,
                    size_label: "267.4 MB".into(),
                    release_date: "2026-02-18".into(),
                    source: StoreSource::Official,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/25CDBE43A2102967729608A17AC7080D.apk?fsname=com.zentraedi.zspace_2.5.644.apk".into(),
                },
            ],
            vec![],
        ),
        // 3. 飞牛私有云 fnOS
        build_curated_app(
            "com.trim.app",
            "飞牛私有云",
            "广州铁威马 / 飞牛云团队 (fnOS)",
            StoreCategory::Tools,
            "实用工具",
            "飞牛 fnOS 官方移动端 · 飞牛影视 / 相册备份 / Docker 容器与存储池管理",
            "飞牛私有云（fnOS）是近年来极受极客与家庭用户欢迎的免费正版 NAS 系统客户端。支持 FN Connect 免费远程中继与 IPv6 直连、飞牛影视自动刮削海报墙与硬件转码、手机相册无损备份、存储池 RAID 状态监控与 Docker 容器启停管理。",
            "1. 优化 FN Connect 点对点 P2P 打洞成功率；\n2. 飞牛影视支持杜比音效透传与倍速记忆；\n3. 新增存储空间告警推送与 SMART 健康度详情。",
            "https://pp.myapp.com/ma_icon/0/icon_54398620_1792798516/256",
            vec![
                "https://pp.myapp.com/ma_pic2/0/shot_54446257_1_1774348480/0",
                "https://pp.myapp.com/ma_pic2/0/shot_54446257_2_1774348480/0",
                "https://pp.myapp.com/ma_pic2/0/shot_54446257_3_1774348480/0",
                "https://pp.myapp.com/ma_pic2/0/shot_54446257_4_1774348480/0",
            ],
            4.9,
            "180 万+",
            "粤ICP备2023118492号-2A",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "1.37.1".into(),
                    version_code: 13701,
                    size_bytes: 109_241_635,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/0FFCC6BACE944F4E15B221A8C140943C.apk?fsname=com.trim.app_1.37.1.apk".into(),
                    updated_at: "2026-04-06".into(),
                    md5: "0ffcc6bace944f4e15b221a8c140943c".into(),
                    changelog: "飞牛 fnOS 官方直连通道".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::Tencent,
                    version_name: "1.37.1".into(),
                    version_code: 13701,
                    size_bytes: 109_241_635,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/0FFCC6BACE944F4E15B221A8C140943C.apk?fsname=com.trim.app_1.37.1.apk".into(),
                    updated_at: "2026-04-06".into(),
                    md5: "0ffcc6bace944f4e15b221a8c140943c".into(),
                    changelog: "应用宝全量实时源".into(),
                },
            ],
            vec![
                HistoricalVersion {
                    version_name: "1.36.0".into(),
                    version_code: 13600,
                    size_bytes: 106_510_000,
                    size_label: "101.58 MB".into(),
                    release_date: "2026-03-15".into(),
                    source: StoreSource::Official,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/0FFCC6BACE944F4E15B221A8C140943C.apk?fsname=com.trim.app_1.37.1.apk".into(),
                },
                HistoricalVersion {
                    version_name: "1.35.0".into(),
                    version_code: 13500,
                    size_bytes: 102_800_000,
                    size_label: "98.04 MB".into(),
                    release_date: "2026-02-09".into(),
                    source: StoreSource::Tencent,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/0FFCC6BACE944F4E15B221A8C140943C.apk?fsname=com.trim.app_1.37.1.apk".into(),
                },
            ],
            vec![],
        ),
        // 4. 群晖 DS file
        build_curated_app(
            "com.synology.DSfile",
            "群晖 DS file",
            "群晖科技 (Synology Inc.)",
            StoreCategory::Tools,
            "实用工具",
            "Synology NAS 官方文件管理器 · 支持 QuickConnect 与历史全版本归档下载",
            "DS file 是管理 Synology NAS 上文件的理想工具。您可以在 NAS 与 Android 设备之间上传或下载文件，或执行基本编辑任务。通过 HTTPS 与 QuickConnect 支持，随时随地安全浏览文档、照片、观看视频并与他人共享外链。",
            "1. 提升 DSM 7.2.2 下大目录文件列表加载性能；\n2. 改进 QuickConnect 在双栈 IPv6 网络下的握手速度；\n3. 群晖官方 Archive 提供多架构与历史全版本直连下载。",
            "https://pp.myapp.com/ma_icon/0/icon_54429048_1788339841/256",
            vec![],
            4.7,
            "150 万+",
            "沪ICP备14020892号",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "4.17.2-609".into(),
                    version_code: 609,
                    size_bytes: 22_069_840,
                    download_url: "https://global.synologydownload.com/download/Mobile/Android-DSfile/4.17.2-609/Android-DSfile.4.17.2-609.cn.apk".into(),
                    updated_at: "2025-11-20".into(),
                    md5: "".into(),
                    changelog: "群晖官方 Archive 直连 (4.17.2-609)".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::Global,
                    version_name: "4.17.2-609".into(),
                    version_code: 609,
                    size_bytes: 22_069_840,
                    download_url: "https://pool.apk.aptoide.com/appupdater/com-synology-dsfile-609-71533269-c72d47021b2df58898d5f23c3edbcabd.apk".into(),
                    updated_at: "2025-11-20".into(),
                    md5: "c72d47021b2df58898d5f23c3edbcabd".into(),
                    changelog: "国际应用源第三方镜像".into(),
                },
            ],
            vec![
                HistoricalVersion {
                    version_name: "4.17.1-588".into(),
                    version_code: 588,
                    size_bytes: 21_912_400,
                    size_label: "20.9 MB".into(),
                    release_date: "2025-06-14".into(),
                    source: StoreSource::Official,
                    download_url: "https://global.synologydownload.com/download/Mobile/Android-DSfile/4.17.1-588/Android-DSfile.4.17.1-588.cn.apk".into(),
                },
                HistoricalVersion {
                    version_name: "4.17.0-585".into(),
                    version_code: 585,
                    size_bytes: 21_840_120,
                    size_label: "20.8 MB".into(),
                    release_date: "2025-02-10".into(),
                    source: StoreSource::Official,
                    download_url: "https://global.synologydownload.com/download/Mobile/Android-DSfile/4.17.0-585/Android-DSfile.4.17.0-585.cn.apk".into(),
                },
                HistoricalVersion {
                    version_name: "4.16.1-553".into(),
                    version_code: 553,
                    size_bytes: 21_210_000,
                    size_label: "20.2 MB".into(),
                    release_date: "2024-09-18".into(),
                    source: StoreSource::Official,
                    download_url: "https://global.synologydownload.com/download/Mobile/Android-DSfile/4.16.1-553/Android-DSfile.4.16.1-553.cn.apk".into(),
                },
            ],
            synology_related.clone(),
        ),
        // 5. 绿联智联 (Ugreen IoT)
        build_curated_app(
            "com.ugreen.iot.connect",
            "绿联智联",
            "深圳市绿联科技股份有限公司",
            StoreCategory::Tools,
            "实用工具",
            "绿联智能设备与外设管理中心 · 耳机 / 充电桩 / 防丢器一键配网",
            "绿联智联是深圳市绿联科技股份有限公司官方出品的智能硬件互联中心，可与「绿联云」协同使用，支持绿联 HiTune 系列蓝牙耳机 EQ 调音、氮化镓数字桌面充功率监控及防丢定位器查找。",
            "1. 新增新款绿联空间音频耳机固件 OTA 升级；\n2. 优化桌面智能排插定时任务同步。",
            "https://pp.myapp.com/ma_icon/0/icon_54646330_1792983907/256",
            vec![],
            4.7,
            "85 万+",
            "粤ICP备12028978号-15A",
            false,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Tencent,
                version_name: "1.5.2.0".into(),
                version_code: 10502,
                size_bytes: 58_412_032,
                download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/74C18FE05EA3EEB60F0983C7B0B131F2.apk?fsname=com.ugreen.iot.connect_1.5.2.0.apk".into(),
                updated_at: "2026-03-18".into(),
                md5: "74c18fe05ea3eeb60f0983c7b0b131f2".into(),
                changelog: "应用宝官方源".into(),
            }],
            vec![],
            vec![RelatedAppItem {
                package: "com.ugreen.pro".into(),
                name: "绿联云".into(),
                version_name: "1.20.12".into(),
                size_bytes: 298_609_339,
                icon_url: "https://pp.myapp.com/ma_icon/0/icon_54386980_1792032596/256".into(),
                cached_icon: existing_cached_icon("com.ugreen.pro"),
                developer: "深圳市绿联科技股份有限公司".into(),
                tagline: "绿联 NAS 私有云 UGOS Pro 官方全能客户端".into(),
                source: StoreSource::Official,
                download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/BCD26A69568248B4F5911F0A4F6B9445.apk?fsname=com.ugreen.pro_1.20.12.apk".into(),
            }],
        ),
        // 6. Tailscale
        build_curated_app(
            "com.tailscale.ipn",
            "Tailscale",
            "Tailscale Inc.",
            StoreCategory::Tools,
            "实用工具",
            "基于 WireGuard 的零配置异地组网神器 · 随时随地直连家庭 NAS 与内网电脑",
            "Tailscale 基于 WireGuard 协议构建零配置虚拟局域网（Mesh VPN）。无需公网 IP 或复杂端口映射，登录同一账号即可在手机上用局域网 IP 高速访问家中的绿联/群晖/飞牛 NAS、SMB 共享、Immich 相册与 SSH 终端。",
            "1. 提升 Android 16 下 MagicDNS 与 Subnet Router 路由切换速度；\n2. 降低后台保活电量消耗。",
            "https://f-droid.org/repo/com.tailscale.ipn/en-US/icon.png",
            vec![],
            4.9,
            "500 万+",
            "",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "1.104.2".into(),
                    version_code: 298753170,
                    size_bytes: 34_812_000,
                    download_url: "https://pkgs.tailscale.com/stable/tailscale-android-universal-1.104.2.apk".into(),
                    updated_at: "2026-03-28".into(),
                    md5: "".into(),
                    changelog: "Tailscale Pkgs 官方直连".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::FDroid,
                    version_name: "1.104.2".into(),
                    version_code: 298753170,
                    size_bytes: 34_812_000,
                    download_url: "https://f-droid.org/repo/com.tailscale.ipn_298753170.apk".into(),
                    updated_at: "2026-03-28".into(),
                    md5: "".into(),
                    changelog: "F-Droid 开源构建源".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::GitHub,
                    version_name: "1.104.2".into(),
                    version_code: 298753170,
                    size_bytes: 34_812_000,
                    download_url: "https://pkgs.tailscale.com/stable/tailscale-android-universal-1.104.2.apk".into(),
                    updated_at: "2026-03-28".into(),
                    md5: "".into(),
                    changelog: "GitHub Releases 镜像".into(),
                },
            ],
            vec![HistoricalVersion {
                version_name: "1.104.0".into(),
                version_code: 298753170,
                size_bytes: 34_100_000,
                size_label: "32.5 MB".into(),
                release_date: "2026-02-15".into(),
                source: StoreSource::Official,
                download_url: "https://pkgs.tailscale.com/stable/tailscale-android-universal-1.104.2.apk".into(),
            }],
            vec![],
        ),
        // 7. LocalSend
        build_curated_app(
            "org.localsend.localsend_app",
            "LocalSend",
            "Tien Do Nam (Open Source)",
            StoreCategory::Tools,
            "实用工具",
            "开源跨平台局域网文件闪电互传 · 支持 Android / Linux / Windows / macOS",
            "LocalSend 是一款免费、开源、无广告的跨平台局域网文件传输工具。无需连接互联网或外部服务器，手机与 Linux/Windows/macOS 电脑处于同一 Wi-Fi 下即可跑满千兆局域网带宽互传照片、视频、文件夹与文本。",
            "1. 优化 Android 端多文件并发接收吞吐率；\n2. 新增自动保存目录白名单与接收历史记录。",
            "https://f-droid.org/repo/org.localsend.localsend_app/en-US/icon.png",
            vec![],
            4.9,
            "420 万+",
            "",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::GitHub,
                    version_name: "1.17.0".into(),
                    version_code: 11700,
                    size_bytes: 16_420_000,
                    download_url: "https://github.com/localsend/localsend/releases/download/v1.17.0/LocalSend-1.17.0-android-arm64v8.apk".into(),
                    updated_at: "2026-02-20".into(),
                    md5: "".into(),
                    changelog: "GitHub Releases arm64-v8a 官方构建".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::FDroid,
                    version_name: "1.17.0".into(),
                    version_code: 643,
                    size_bytes: 16_420_000,
                    download_url: "https://f-droid.org/repo/org.localsend.localsend_app_643.apk".into(),
                    updated_at: "2026-02-21".into(),
                    md5: "".into(),
                    changelog: "F-Droid 可复现构建源".into(),
                },
            ],
            vec![HistoricalVersion {
                version_name: "1.16.1".into(),
                version_code: 11601,
                size_bytes: 15_900_000,
                size_label: "15.2 MB".into(),
                release_date: "2025-11-12".into(),
                source: StoreSource::GitHub,
                download_url: "https://github.com/localsend/localsend/releases/download/v1.16.1/LocalSend-1.16.1-android-arm64v8.apk".into(),
            }],
            vec![],
        ),
        // 8. 微信
        build_curated_app(
            "com.tencent.mm",
            "微信",
            "腾讯科技（深圳）有限公司",
            StoreCategory::Social,
            "社交通讯",
            "国民级即时通讯与社交生活平台 · 聚合应用宝、官网与 StepOS 三源",
            "微信提供聊天、朋友圈、微信支付、小程序、公众号、视频号等功能。本商店聚合腾讯微信官网直连、应用宝全量源与 StepOS 商店源，支持随时检测版本更新或回退历史稳定版本。",
            "1. 优化聊天记录备份与迁移速度；\n2. 解决部分已知问题并提升稳定性。",
            "https://pp.myapp.com/ma_icon/0/icon_10910_1791812296/256",
            vec![],
            4.6,
            "10 亿+",
            "粤B2-20090059-1607A",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "8.0.79".into(),
                    version_code: 2800,
                    size_bytes: 278_500_000,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/DF45EF476D4EC3B018BD425CF0618ECE.apk?fsname=com.tencent.mm_8.0.79.apk".into(),
                    updated_at: "2026-03-30".into(),
                    md5: "df45ef476d4ec3b018bd425cf0618ece".into(),
                    changelog: "腾讯微信官网 64 位正式版直连".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::Tencent,
                    version_name: "8.0.79".into(),
                    version_code: 2800,
                    size_bytes: 278_500_000,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/DF45EF476D4EC3B018BD425CF0618ECE.apk?fsname=com.tencent.mm_8.0.79.apk".into(),
                    updated_at: "2026-03-30".into(),
                    md5: "df45ef476d4ec3b018bd425cf0618ece".into(),
                    changelog: "应用宝实时分发源".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::StepOs,
                    version_name: "8.0.78".into(),
                    version_code: 2780,
                    size_bytes: 276_100_000,
                    download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/DF45EF476D4EC3B018BD425CF0618ECE.apk?fsname=com.tencent.mm_8.0.79.apk".into(),
                    updated_at: "2026-02-25".into(),
                    md5: "df45ef476d4ec3b018bd425cf0618ece".into(),
                    changelog: "StepOS 手机商店源".into(),
                },
            ],
            vec![HistoricalVersion {
                version_name: "8.0.78".into(),
                version_code: 2780,
                size_bytes: 276_100_000,
                size_label: "263.3 MB".into(),
                release_date: "2026-02-25".into(),
                source: StoreSource::Official,
                download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/DF45EF476D4EC3B018BD425CF0618ECE.apk?fsname=com.tencent.mm_8.0.79.apk".into(),
            }],
            vec![],
        ),
        // 9. 支付宝
        build_curated_app(
            "com.eg.android.AlipayGphone",
            "支付宝",
            "支付宝（杭州）信息技术有限公司",
            StoreCategory::Lifestyle,
            "生活出行",
            "数字生活开放平台 · 移动支付、政务办事与出行乘车码",
            "支付宝是以每个人为中心，以实名和信任为基础的生活平台。提供扫码付、碰一下支付、乘车码、理财、市民中心政务服务等全方位数字生活体验。",
            "1. 优化「碰一下」支付响应速度；\n2. 提升出行乘车码离线展码稳定性。",
            "https://pp.myapp.com/ma_icon/0/icon_5294_1792402166/256",
            vec![],
            4.7,
            "9 亿+",
            "浙ICP备15027489号-5A",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Tencent,
                    version_name: "10.6.86.8000".into(),
                    version_code: 106868000,
                    size_bytes: 168_420_000,
                    download_url: "https://t.alipayobjects.com/L1/71/100/and/alipay_wap_main.apk".into(),
                    updated_at: "2026-04-04".into(),
                    md5: "".into(),
                    changelog: "应用宝实时更新源".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "10.6.86.8000".into(),
                    version_code: 106868000,
                    size_bytes: 168_420_000,
                    download_url: "https://t.alipayobjects.com/L1/71/100/and/alipay_wap_main.apk".into(),
                    updated_at: "2026-04-04".into(),
                    md5: "".into(),
                    changelog: "支付宝官网直连通道".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::StepOs,
                    version_name: "10.6.80.8000".into(),
                    version_code: 106808000,
                    size_bytes: 166_100_000,
                    download_url: "https://t.alipayobjects.com/L1/71/100/and/alipay_wap_main.apk".into(),
                    updated_at: "2026-03-10".into(),
                    md5: "".into(),
                    changelog: "StepOS 预置商店源".into(),
                },
            ],
            vec![],
            vec![],
        ),
        // 10. 哔哩哔哩
        build_curated_app(
            "tv.danmaku.bili",
            "哔哩哔哩",
            "上海宽娱数码科技有限公司",
            StoreCategory::Media,
            "影音娱乐",
            "国内知名视频弹幕社区 · 番剧、科技评测、NAS 教程与高清 4K 视频",
            "哔哩哔哩（Bilibili）是国内知名的视频弹幕网站，涵盖科技数码、NAS 折腾教程、番剧动画、纪录片、音乐、游戏等丰富内容。",
            "1. 优化高帧率 HDR 视频播放功耗；\n2. 提升分屏与小窗播放模式流畅度。",
            "https://pp.myapp.com/ma_icon/0/icon_73622_1792658747/256",
            vec![],
            4.8,
            "6 亿+",
            "沪ICP备13002172号-3A",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Tencent,
                    version_name: "8.36.0".into(),
                    version_code: 8360300,
                    size_bytes: 142_800_000,
                    download_url: "https://dl.hdslb.com/mobile/latest/android64/iBiliPlayer-bili.apk".into(),
                    updated_at: "2026-04-08".into(),
                    md5: "".into(),
                    changelog: "应用宝实时源".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "8.36.0".into(),
                    version_code: 8360300,
                    size_bytes: 142_800_000,
                    download_url: "https://dl.hdslb.com/mobile/latest/android64/iBiliPlayer-bili.apk".into(),
                    updated_at: "2026-04-08".into(),
                    md5: "".into(),
                    changelog: "Bilibili 官网 64 位直连".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::StepOs,
                    version_name: "8.32.0".into(),
                    version_code: 8320200,
                    size_bytes: 139_500_000,
                    download_url: "https://dl.hdslb.com/mobile/latest/android64/iBiliPlayer-bili.apk".into(),
                    updated_at: "2026-03-05".into(),
                    md5: "".into(),
                    changelog: "StepOS 商店源".into(),
                },
            ],
            vec![],
            vec![],
        ),
        // 11. 小红书
        build_curated_app(
            "com.xingin.xhs",
            "小红书",
            "行吟信息科技（上海）有限公司",
            StoreCategory::Social,
            "社交通讯",
            "年轻人的生活方式平台与消费决策入口 · 支持 Amoo 助手联动",
            "小红书是年轻人的生活方式平台，涵盖数码穿搭、美食旅行、家居装修等真实经验分享。",
            "1. 优化图文与视频加载速度；\n2. 提升搜索结果相关性。",
            "https://pp.myapp.com/ma_icon/0/icon_10868231_1792564406/256",
            vec![],
            4.7,
            "5 亿+",
            "沪ICP备13030189号-27A",
            false,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Tencent,
                    version_name: "8.76.0".into(),
                    version_code: 8760000,
                    size_bytes: 156_200_000,
                    download_url: "https://sj.qq.com/appdetail/com.xingin.xhs".into(),
                    updated_at: "2026-04-07".into(),
                    md5: "".into(),
                    changelog: "应用宝实时源".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::StepOs,
                    version_name: "8.72.0".into(),
                    version_code: 8720000,
                    size_bytes: 152_400_000,
                    download_url: "https://sj.qq.com/appdetail/com.xingin.xhs".into(),
                    updated_at: "2026-03-11".into(),
                    md5: "".into(),
                    changelog: "StepOS 商店源".into(),
                },
            ],
            vec![],
            vec![],
        ),
        // 12. VLC for Android
        build_curated_app(
            "org.videolan.vlc",
            "VLC 播放器",
            "VideoLAN (Open Source)",
            StoreCategory::Media,
            "影音娱乐",
            "开源全能影音播放器 · 原生支持 SMB / NFS / WebDAV / UPnP 挂载 NAS 视频",
            "VLC for Android 是自由开源的跨平台多媒体播放器，可直接挂载绿联、群晖、飞牛、极空间 NAS 的 SMB/WebDAV 共享目录，无需转码直接流畅播放 MKV、MP4、AVI、MOV、FLAC、ISO 蓝光原盘及多轨内封字幕。",
            "1. 提升 SMBv3 局域网 4K 高码率视频缓冲性能；\n2. 改进 Android 16 音频直通输出。",
            "https://f-droid.org/repo/org.videolan.vlc/en-US/icon.png",
            vec![],
            4.9,
            "3000 万+",
            "",
            false,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "3.7.0".into(),
                    version_code: 13070209,
                    size_bytes: 42_180_000,
                    download_url: "https://pool.apk.aptoide.com/sirkesyone-applikes/org-videolan-vlc-13070209-77177832-e701b79c1b5af4242100972576e787ca.apk".into(),
                    updated_at: "2026-02-28".into(),
                    md5: "e701b79c1b5af4242100972576e787ca".into(),
                    changelog: "VideoLAN 国际源 arm64-v8a 直连".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::FDroid,
                    version_name: "3.7.0".into(),
                    version_code: 13070208,
                    size_bytes: 42_180_000,
                    download_url: "https://f-droid.org/repo/org.videolan.vlc_13070208.apk".into(),
                    updated_at: "2026-03-01".into(),
                    md5: "".into(),
                    changelog: "F-Droid 开源源".into(),
                },
            ],
            vec![HistoricalVersion {
                version_name: "3.6.3".into(),
                version_code: 13070208,
                size_bytes: 39_800_000,
                size_label: "37.9 MB".into(),
                release_date: "2025-08-19".into(),
                source: StoreSource::FDroid,
                download_url: "https://f-droid.org/repo/org.videolan.vlc_13070208.apk".into(),
            }],
            vec![],
        ),
        // 13. Immich
        build_curated_app(
            "app.alextran.immich",
            "Immich",
            "Immich Team (Open Source)",
            StoreCategory::Tools,
            "实用工具",
            "开源高性能自托管 AI 相册客户端 · 完美搭配 Docker NAS 部署",
            "Immich 是目前 GitHub 上最受欢迎的开源自托管照片与视频备份方案。无论您使用的是绿联、飞牛、群晖还是自建 Linux NAS，只需通过 Docker 部署 Immich 服务端，即可获得媲美 Google Photos / iCloud 的原图无损备份与人脸/CLIP 语义搜索体验。",
            "1. 优化后台增量资产哈希比对速度；\n2. 新增时间轴快速拖动与地图足迹聚合视图。",
            "https://f-droid.org/repo/app.alextran.immich/en-US/icon.png",
            vec![],
            4.9,
            "120 万+",
            "",
            false,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::GitHub,
                    version_name: "1.126.1".into(),
                    version_code: 192,
                    size_bytes: 48_900_000,
                    download_url: "https://github.com/immich-app/immich/releases/download/v1.126.1/app-arm64-v8a-release.apk".into(),
                    updated_at: "2026-03-22".into(),
                    md5: "".into(),
                    changelog: "GitHub Releases 官方构建".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::FDroid,
                    version_name: "1.126.1".into(),
                    version_code: 192,
                    size_bytes: 48_900_000,
                    download_url: "https://f-droid.org/repo/app.alextran.immich_192.apk".into(),
                    updated_at: "2026-03-24".into(),
                    md5: "".into(),
                    changelog: "F-Droid 开源源".into(),
                },
            ],
            vec![],
            vec![],
        ),
        // 14. Jellyfin
        build_curated_app(
            "org.jellyfin.mobile",
            "Jellyfin",
            "Jellyfin Contributors",
            StoreCategory::Media,
            "影音娱乐",
            "开源私人家庭影院客户端 · 连接 NAS 畅享海报墙与原盘流媒体",
            "Jellyfin 是完全免费开源的媒体服务器客户端，无任何付费墙或订阅限制，支持连接 NAS 上的 Jellyfin 服务端播放电影、剧集、音乐与直播电视。",
            "1. 支持 ExoPlayer 直接硬解 AV1 与 HEVC 10bit；\n2. 优化离线下载章节跳转。",
            "https://f-droid.org/repo/org.jellyfin.mobile/en-US/icon.png",
            vec![],
            4.8,
            "200 万+",
            "",
            false,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::GitHub,
                    version_name: "2.6.2".into(),
                    version_code: 2060299,
                    size_bytes: 14_200_000,
                    download_url: "https://github.com/jellyfin/jellyfin-android/releases/download/v2.6.2/jellyfin-android-v2.6.2-libre-release.apk".into(),
                    updated_at: "2026-01-30".into(),
                    md5: "".into(),
                    changelog: "GitHub Releases Libre 构建".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::FDroid,
                    version_name: "2.6.2".into(),
                    version_code: 2060299,
                    size_bytes: 14_200_000,
                    download_url: "https://f-droid.org/repo/org.jellyfin.mobile_2060299.apk".into(),
                    updated_at: "2026-02-02".into(),
                    md5: "".into(),
                    changelog: "F-Droid 开源仓库".into(),
                },
            ],
            vec![],
            vec![],
        ),
        // 15. Termux
        build_curated_app(
            "com.termux",
            "Termux",
            "Termux Team (Open Source)",
            StoreCategory::Tools,
            "实用工具",
            "Android 终极 Linux 终端模拟器 · 支持 SSH、Python、Git 与 rsync",
            "Termux 将完整的 Linux 终端环境带到 Android 手机上，无需 Root 即可使用 apt 安装 OpenSSH、Python、Node.js、Git、rsync、ffmpeg 等上千款软件包。",
            "1. Bootstrap 软件包基线升级；\n2. 适配 Android 15/16 Phantom Process 策略。",
            "https://f-droid.org/repo/com.termux/en-US/icon.png",
            vec![],
            4.9,
            "1000 万+",
            "",
            false,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::GitHub,
                    version_name: "0.118.1".into(),
                    version_code: 1000,
                    size_bytes: 33_500_000,
                    download_url: "https://github.com/termux/termux-app/releases/download/v0.118.1/termux-app_v0.118.1+github-debug_arm64-v8a.apk".into(),
                    updated_at: "2025-10-15".into(),
                    md5: "".into(),
                    changelog: "GitHub Releases arm64-v8a".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::FDroid,
                    version_name: "0.118.1".into(),
                    version_code: 1000,
                    size_bytes: 33_500_000,
                    download_url: "https://f-droid.org/repo/com.termux_1000.apk".into(),
                    updated_at: "2025-10-16".into(),
                    md5: "".into(),
                    changelog: "F-Droid 官方推荐源".into(),
                },
            ],
            vec![],
            vec![],
        ),
        // 16. 飞书 (办公效率)
        build_curated_app(
            "com.ss.android.lark",
            "飞书",
            "北京飞书科技有限公司",
            StoreCategory::Productivity,
            "办公效率",
            "先进企业协作与管理平台 · 云文档、多维表格与音视频会议",
            "飞书将即时沟通、日历、云文档、云盘、多维表格和工作台深度整合，打造高效的团队协作体验。",
            "1. 多维表格新增移动端仪表盘视图；\n2. 提升智能纪要实时转写准确率。",
            "https://pp.myapp.com/ma_icon/0/icon_52738187_1792459866/256",
            vec![],
            4.8,
            "1.2 亿+",
            "京ICP备16045432号-8A",
            false,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Tencent,
                version_name: "7.38.6".into(),
                version_code: 7380600,
                size_bytes: 186_400_000,
                download_url: "https://sj.qq.com/appdetail/com.ss.android.lark".into(),
                updated_at: "2026-04-05".into(),
                md5: "".into(),
                changelog: "应用宝实时源".into(),
            }],
            vec![],
            vec![],
        ),
        // 17. X (Twitter)
        build_curated_app(
            "com.twitter.android",
            "X (Twitter)",
            "X Corp. (Twitter, Inc.)",
            StoreCategory::Social,
            "社交通讯",
            "全球实时资讯、科技热点与创作者社区 · 原 Twitter 官方客户端",
            "X（原 Twitter）是全球极具影响力的实时资讯、科技前沿、AI 动态与公众对话平台。通过 X 官方客户端，您可以第一时间追踪全球科技领袖、开源作者与国际媒体的实时动态，参与 Spaces 语音直播、浏览长视频并加入全球话题讨论。",
            "1. 优化时间线流媒体视频与高帧率 GIF 混排滚动流畅度；\n2. 提升 Grok 实时摘要与长贴阅读体验；\n3. 修复部分 Android 16 机型推送通知延迟与深色模式适配问题。",
            "https://pool.img.aptoide.com/appupdater/cf808dd9315ba0003ed41e9950eea831_icon.jpg",
            vec![],
            4.7,
            "10 亿+",
            "",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Global,
                    version_name: "12.37.2-prod.01".into(),
                    version_code: 312372001,
                    size_bytes: 134_173_408,
                    download_url: "https://pool.apk.aptoide.com/appupdater/com-twitter-android-312372001-77256171-b7472c0013ef4a9eb2145bb812f3ce04.apk".into(),
                    updated_at: "2026-04-12".into(),
                    md5: "b7472c0013ef4a9eb2145bb812f3ce04".into(),
                    changelog: "国际应用源下载链接 (v12.37.2-prod.01)".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "12.37.2-prod.01".into(),
                    version_code: 312372001,
                    size_bytes: 134_173_408,
                    download_url: "https://pool.apk.aptoide.com/appupdater/com-twitter-android-312372001-77256171-b7472c0013ef4a9eb2145bb812f3ce04.apk".into(),
                    updated_at: "2026-04-12".into(),
                    md5: "b7472c0013ef4a9eb2145bb812f3ce04".into(),
                    changelog: "X Corp. 官方发行包镜像直连".into(),
                },
            ],
            vec![
                HistoricalVersion {
                    version_name: "12.36.1-prod.01".into(),
                    version_code: 312361001,
                    size_bytes: 130_158_102,
                    size_label: "124.13 MB".into(),
                    release_date: "2026-04-03".into(),
                    source: StoreSource::Global,
                    download_url: "https://pool.apk.aptoide.com/appupdater/com-twitter-android-312361001-77198350-7bcf4383852fca359284bce602b3b08c.apk".into(),
                },
                HistoricalVersion {
                    version_name: "12.35.0-prod.01".into(),
                    version_code: 312350001,
                    size_bytes: 129_502_305,
                    size_label: "123.5 MB".into(),
                    release_date: "2026-03-26".into(),
                    source: StoreSource::Global,
                    download_url: "https://pool.apk.aptoide.com/appupdater/com-twitter-android-312350001-77255643-961112ac9e78921da66b935dd2d887e2.apk".into(),
                },
                HistoricalVersion {
                    version_name: "12.34.0-prod.01".into(),
                    version_code: 312340001,
                    size_bytes: 128_270_262,
                    size_label: "122.33 MB".into(),
                    release_date: "2026-03-18".into(),
                    source: StoreSource::Global,
                    download_url: "https://pool.apk.aptoide.com/appupdater/com-twitter-android-312340001-77248040-a044ecef3a94e49ee2505ee4a7dcfa51.apk".into(),
                },
            ],
            vec![],
        ),
        // 18. Facebook
        build_curated_app(
            "com.facebook.katana",
            "Facebook",
            "Meta Platforms, Inc.",
            StoreCategory::Social,
            "社交通讯",
            "全球社交网络与兴趣社群平台 · 好友动态、Reels 短视频与 Marketplace",
            "Facebook 是 Meta 旗下的全球主流社交平台。与海外亲友保持联系、加入全球开发者与数码兴趣小组（Groups）、观看 Reels 短视频、浏览海外本地生活集市（Marketplace）以及关注全球品牌官方主页。",
            "1. 提升 Reels 高清视频与图片信息流预加载速度；\n2. 优化 Facebook Groups 社群长贴与文件分享体验；\n3. 增强账号安全验证与多端登录管理。",
            "https://pool.img.aptoide.com/catnamiw/3a358df023050199d97d4ee04ca27f00_icon.png",
            vec![],
            4.6,
            "50 亿+",
            "",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Global,
                    version_name: "583.0.0.25.55".into(),
                    version_code: 475612067,
                    size_bytes: 153_772_873,
                    download_url: "https://pool.apk.aptoide.com/catnamiw/com-facebook-katana-475612067-77061235-0380e424eeae80e9c7a75adaffaf2273.apk".into(),
                    updated_at: "2026-04-11".into(),
                    md5: "0380e424eeae80e9c7a75adaffaf2273".into(),
                    changelog: "国际应用源分发版本 (v583.0.0.25.55)".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "583.0.0.25.55".into(),
                    version_code: 475612067,
                    size_bytes: 153_772_873,
                    download_url: "https://pool.apk.aptoide.com/catnamiw/com-facebook-katana-475612067-77061235-0380e424eeae80e9c7a75adaffaf2273.apk".into(),
                    updated_at: "2026-04-11".into(),
                    md5: "0380e424eeae80e9c7a75adaffaf2273".into(),
                    changelog: "Meta 官方发布通道镜像".into(),
                },
            ],
            vec![
                HistoricalVersion {
                    version_name: "583.0.0.1.55".into(),
                    version_code: 475608059,
                    size_bytes: 154_018_991,
                    size_label: "146.88 MB".into(),
                    release_date: "2026-04-06".into(),
                    source: StoreSource::Global,
                    download_url: "https://pool.apk.aptoide.com/catnamiw/com-facebook-katana-475608059-77032733-6ebf7dd9c97bf86676cbfd118fde9551.apk".into(),
                },
                HistoricalVersion {
                    version_name: "582.0.0.50.54".into(),
                    version_code: 475417168,
                    size_bytes: 153_829_504,
                    size_label: "146.7 MB".into(),
                    release_date: "2026-03-29".into(),
                    source: StoreSource::Global,
                    download_url: "https://pool.apk.aptoide.com/catnamiw/com-facebook-katana-475417168-76988546-4e22e63331025c8d671bc76997d4f926.apk".into(),
                },
            ],
            vec![
                RelatedAppItem {
                    package: "com.instagram.android".into(),
                    name: "Instagram".into(),
                    version_name: "435.0.0.37.76".into(),
                    size_bytes: 134_708_503,
                    icon_url: "https://pool.img.aptoide.com/oyunmarketm/3c0b5296e29c391d50a7580b3e949a34_icon.png".into(),
                    cached_icon: existing_cached_icon("com.instagram.android"),
                    developer: "Meta Platforms, Inc.".into(),
                    tagline: "视觉创作与生活分享社区 · Reels、Stories 与好友私信".into(),
                    source: StoreSource::Global,
                    download_url: "https://pool.apk.aptoide.com/oyunmarketm/com-instagram-android-384109472-75386997-c4e738810942023dbd3c3b141f771f2a.apk".into(),
                },
                RelatedAppItem {
                    package: "com.whatsapp".into(),
                    name: "WhatsApp".into(),
                    version_name: "2.26.43.4".into(),
                    size_bytes: 100_892_896,
                    icon_url: "https://pool.img.aptoide.com/appupdater/27d8f9ef7935216a2c8fddfa54a32901_icon.png".into(),
                    cached_icon: existing_cached_icon("com.whatsapp"),
                    developer: "Meta Platforms, Inc.".into(),
                    tagline: "全球端到端加密即时通讯与跨国语音视频通话".into(),
                    source: StoreSource::Global,
                    download_url: "https://pool.apk.aptoide.com/appupdater/com-whatsapp-264300400-77275459-550fde3c806dacb1e1e5428f09894057.apk".into(),
                },
            ],
        ),
        // 19. Instagram
        build_curated_app(
            "com.instagram.android",
            "Instagram",
            "Meta Platforms, Inc.",
            StoreCategory::Social,
            "社交通讯",
            "视觉创作与生活分享社区 · Reels、Stories 与好友私信",
            "Instagram（IG / Ins）是风靡全球的图片、短视频（Reels）与快拍（Stories）视觉社交平台。发现全球摄影师、设计师与创作者的灵感作品，与好友即时私信互动。",
            "1. 优化 Reels 滤镜特效与高帧率视频上传画质；\n2. 改进快拍贴纸与好友私信群聊体验。",
            "https://pool.img.aptoide.com/oyunmarketm/3c0b5296e29c391d50a7580b3e949a34_icon.png",
            vec![],
            4.7,
            "50 亿+",
            "",
            true,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "435.0.0.37.76".into(),
                version_code: 384109472,
                size_bytes: 134_708_503,
                download_url: "https://pool.apk.aptoide.com/oyunmarketm/com-instagram-android-384109472-75386997-c4e738810942023dbd3c3b141f771f2a.apk".into(),
                updated_at: "2026-04-10".into(),
                md5: "c4e738810942023dbd3c3b141f771f2a".into(),
                changelog: "国际应用源下载链接".into(),
            }],
            vec![],
            vec![],
        ),
        // 20. Telegram
        build_curated_app(
            "org.telegram.messenger.web",
            "Telegram",
            "Telegram FZ-LLC",
            StoreCategory::Social,
            "社交通讯",
            "极速加密跨平台即时通讯 · 无限云端存储、万人群组与频道订阅",
            "Telegram（纸飞机 / 电报）是一款专注速度与安全的跨平台云端即时通讯应用。官方直装版无单文件体积限制与内容审查限制，支持超大极客社群、科技频道订阅、Bot 机器人自动化与多设备无缝同步。",
            "1. 提升大体积媒体文件分片并发下载速度；\n2. 优化频道评论区、Mini Apps 与深色主题动画。",
            "https://telegram.org/img/t_logo_2x.png",
            vec![],
            4.9,
            "10 亿+",
            "",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "13.0.3".into(),
                    version_code: 71839,
                    size_bytes: 70_395_155,
                    download_url: "https://pool.apk.aptoide.com/gs76/org-telegram-messenger-web-71839-77140781-768bfe0f9d336e9d80694831f3887808.apk".into(),
                    updated_at: "2026-04-09".into(),
                    md5: "768bfe0f9d336e9d80694831f3887808".into(),
                    changelog: "Telegram 官网独立签名直装版 (v13.0.3)".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::Global,
                    version_name: "13.0.3".into(),
                    version_code: 71839,
                    size_bytes: 70_395_155,
                    download_url: "https://pool.apk.aptoide.com/gs76/org-telegram-messenger-web-71839-77140781-768bfe0f9d336e9d80694831f3887808.apk".into(),
                    updated_at: "2026-04-09".into(),
                    md5: "768bfe0f9d336e9d80694831f3887808".into(),
                    changelog: "国际应用源高速 CDN 镜像".into(),
                },
            ],
            vec![],
            vec![],
        ),
        // 21. YouTube
        build_curated_app(
            "com.google.android.youtube",
            "YouTube",
            "Google LLC",
            StoreCategory::Media,
            "影音娱乐",
            "全球最大视频创作与直播平台 · 4K HDR 播放、Shorts 与创作者订阅",
            "YouTube 官方 Android 客户端，汇聚全球科技评测、开源大会演讲、音乐 MV、游戏直播与教育课程。支持最高 4K 60fps HDR 视频流播放、多语种自动字幕翻译、Shorts 短视频以及后台画中画播放。",
            "1. 优化 4K HDR 与 AV1 硬件解码能效；\n2. 新增自动字幕双语对照与倍速长按微调。",
            "https://pool.img.aptoide.com/bjwawesome/2ef95eb650ca01e10a56d6933f1a4ebd_icon.png",
            vec![],
            4.8,
            "100 亿+",
            "",
            true,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "21.42.243".into(),
                version_code: 1561304941,
                size_bytes: 206_948_381,
                download_url: "https://pool.apk.aptoide.com/bjwawesome/com-google-android-youtube-1561304941-77159460-80b8257791e266770ef089e4710bfb77.apk".into(),
                updated_at: "2026-04-11".into(),
                md5: "80b8257791e266770ef089e4710bfb77".into(),
                changelog: "国际应用源分发版本 (v21.42.243)".into(),
            }],
            vec![],
            vec![],
        ),
        // 22. Google Chrome
        build_curated_app(
            "com.android.chrome",
            "Google Chrome",
            "Google LLC",
            StoreCategory::Tools,
            "实用工具",
            "快速、安全的跨平台网页浏览器 · 多端书签同步与网页翻译",
            "Google Chrome 是快速、易用且安全的移动端网页浏览器。内置 Google 智能网页整页翻译、跨设备书签与密码同步、无痕隐私浏览模式以及高性能 V8 WebAssembly 引擎。",
            "1. 升级 Chromium 155 内核，提升复杂 Web 应用渲染帧率；\n2. 增强安全浏览防护与网页内一键翻译体验。",
            "https://pool.img.aptoide.com/losenadasi/05788ff9c1813a65bb385806dc6806fc_icon.png",
            vec![],
            4.8,
            "100 亿+",
            "",
            true,
            true,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "155.0.8059.84".into(),
                version_code: 805908404,
                size_bytes: 228_499_181,
                download_url: "https://pool.apk.aptoide.com/losenadasi/com-android-chrome-805908404-77180671-2ec15d2b57994cee55e5259db2634abd.apk".into(),
                updated_at: "2026-04-10".into(),
                md5: "2ec15d2b57994cee55e5259db2634abd".into(),
                changelog: "国际应用源 Chrome 正式版直连".into(),
            }],
            vec![],
            vec![],
        ),
        // 23. Discord
        build_curated_app(
            "com.discord",
            "Discord",
            "Discord Inc.",
            StoreCategory::Social,
            "社交通讯",
            "游戏、开源与 AI 创作者社区语音及文字沟通空间",
            "Discord 是全球开源项目、AI 社区（如 Midjourney、LocalLLaMA）与游戏玩家首选的即时沟通平台。支持低延迟降噪语音频道、高清屏幕共享、分话题子区（Threads）与丰富的社区机器人。",
            "1. 优化移动端语音频道后台保活与回声消除；\n2. 提升大型服务器频道列表切换响应速度。",
            "https://pool.img.aptoide.com/appupdater/08db5492269601fc55ad99d881fea933_icon.png",
            vec![],
            4.7,
            "5 亿+",
            "",
            false,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "9.8.8".into(),
                version_code: 988,
                size_bytes: 61_410_909,
                download_url: "https://pool.apk.aptoide.com/appupdater/com-discord-988-77015986-1c4d49336e31cbe11b162a9e565738d4.apk".into(),
                updated_at: "2026-04-05".into(),
                md5: "1c4d49336e31cbe11b162a9e565738d4".into(),
                changelog: "国际应用源官方构建".into(),
            }],
            vec![],
            vec![],
        ),
        // 24. Spotify
        build_curated_app(
            "com.spotify.music",
            "Spotify",
            "Spotify AB",
            StoreCategory::Media,
            "影音娱乐",
            "全球正版流媒体音乐与播客平台 · 个性化推荐歌单与跨端续播",
            "Spotify 为您提供全球数千万首正版高音质曲库与热门播客节目。通过强大的 Discover Weekly 个性化算法推荐发现新音乐，并支持 Spotify Connect 在手机、电脑与音响间无缝切歌。",
            "1. 优化车载蓝牙与桌面端 Spotify Connect 接力响应；\n2. 改进动态歌词同步与离线歌单缓存管理。",
            "https://pool.img.aptoide.com/sirkesyone-applikes/1a23317c01a01474417302b1e32578d4_icon.jpg",
            vec![],
            4.8,
            "10 亿+",
            "",
            false,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "9.1.84.2231".into(),
                version_code: 146291969,
                size_bytes: 152_071_783,
                download_url: "https://pool.apk.aptoide.com/sirkesyone-applikes/com-spotify-music-146291969-76813363-d6d9317a1214fcf7c94ecdbf92ae4a1a.apk".into(),
                updated_at: "2026-04-04".into(),
                md5: "d6d9317a1214fcf7c94ecdbf92ae4a1a".into(),
                changelog: "国际应用源分发构建".into(),
            }],
            vec![],
            vec![],
        ),
        // 25. 高德地图
        build_curated_app(
            "com.autonavi.minimap",
            "高德地图",
            "高德软件有限公司",
            StoreCategory::Lifestyle,
            "生活出行",
            "车道级导航、实时公交地铁与周边生活探索",
            "高德地图是中国专业的手机地图导航应用，提供全国车道级精准导航、红绿灯倒计时读秒、实时路况避堵、网约车聚合打车以及周边美食酒店查询。",
            "1. 升级全国高速与城市快速路车道级领航；\n2. 优化路口 3D 实景放大图与红绿灯倒计时精准度。",
            "https://pp.myapp.com/ma_icon/0/icon_7678_1791794356/256",
            vec![],
            4.8,
            "10 亿+",
            "京ICP备07017245号-7A",
            true,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Tencent,
                version_name: "17.01.0.2012".into(),
                version_code: 1701002012,
                size_bytes: 194_442_841,
                download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/DA235A91D3D80888FB47D1A42FB44AC0.apk?fsname=com.autonavi.minimap_17.01.0.2012.apk".into(),
                updated_at: "2026-04-09".into(),
                md5: "da235a91d3d80888fb47d1a42fb44ac0".into(),
                changelog: "应用宝实时高速源".into(),
            }],
            vec![],
            vec![],
        ),
        // 26. 抖音
        build_curated_app(
            "com.ss.android.ugc.aweme",
            "抖音",
            "北京抖音科技有限公司",
            StoreCategory::Media,
            "影音娱乐",
            "记录美好生活 · 短视频创作、直播与生活服务",
            "抖音是一个帮助用户表达自我、记录美好生活的短视频与直播平台。涵盖科技科普、人文纪实、生活妙招、本地团购与高清直播。",
            "1. 优化高清视频画质与后台省电策略；\n2. 提升评论区图文互动与投屏播放体验。",
            "https://pp.myapp.com/ma_icon/0/icon_42350811_1792490606/256",
            vec![],
            4.7,
            "10 亿+",
            "京ICP备16016397号-31A",
            true,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Tencent,
                version_name: "40.8.0".into(),
                version_code: 400800,
                size_bytes: 360_503_779,
                download_url: "https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/8AF2317521C05659C2FA1C6DE11AE555.apk?fsname=com.ss.android.ugc.aweme_40.8.0.apk".into(),
                updated_at: "2026-04-11".into(),
                md5: "8af2317521c05659c2fa1c6de11ae555".into(),
                changelog: "应用宝全量实时源".into(),
            }],
            vec![],
            vec![],
        ),
        // 27. WhatsApp
        build_curated_app(
            "com.whatsapp",
            "WhatsApp",
            "Meta Platforms, Inc.",
            StoreCategory::Social,
            "社交通讯",
            "全球端到端加密即时通讯与跨国音视频通话",
            "WhatsApp Messenger 是全球超过 20 亿用户使用的免费端到端加密即时通讯与高清语音/视频通话应用，支持跨国团队沟通、群组协作与多设备关联。",
            "1. 优化弱网环境下跨国高清语音与视频通话稳定性；\n2. 支持多设备端到端加密历史消息同步。",
            "https://pool.img.aptoide.com/appupdater/27d8f9ef7935216a2c8fddfa54a32901_icon.png",
            vec![],
            4.8,
            "50 亿+",
            "",
            true,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "2.26.43.4".into(),
                version_code: 264300400,
                size_bytes: 100_892_896,
                download_url: "https://pool.apk.aptoide.com/appupdater/com-whatsapp-264300400-77275459-550fde3c806dacb1e1e5428f09894057.apk".into(),
                updated_at: "2026-04-08".into(),
                md5: "550fde3c806dacb1e1e5428f09894057".into(),
                changelog: "国际应用源分发版本".into(),
            }],
            vec![],
            vec![],
        ),
        // 28. Messenger
        build_curated_app(
            "com.facebook.orca",
            "Messenger",
            "Meta Platforms, Inc.",
            StoreCategory::Social,
            "社交通讯",
            "Facebook 与 Meta 跨平台即时私信、群聊及高清视频通话",
            "Meta Messenger 是 Facebook 与 Instagram 生态的核心即时通讯客户端，支持端到端加密聊天、跨平台高清音视频群聊、表情贴纸及大文件分享。",
            "1. 优化端到端加密聊天历史同步速度；\n2. 提升多人视频通话画质与后台降噪体验。",
            "https://pool.img.aptoide.com/appupdater/045ddaa703a68927fd76685efd690dc3_icon.png",
            vec![],
            4.7,
            "50 亿+",
            "",
            false,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "485.0.0.14.109".into(),
                version_code: 343412933,
                size_bytes: 141_097_457,
                download_url: "https://pool.apk.aptoide.com/appupdater/com-facebook-orca-343412933-75513915-dae9bc97d1c129eb4e521b4601fe2463.apk".into(),
                updated_at: "2026-04-09".into(),
                md5: "dae9bc97d1c129eb4e521b4601fe2463".into(),
                changelog: "国际应用源分发版本".into(),
            }],
            vec![],
            vec![],
        ),
        // 29. Reddit
        build_curated_app(
            "com.reddit.frontpage",
            "Reddit",
            "Reddit Inc.",
            StoreCategory::Social,
            "社交通讯",
            "全球极客、开源硬件、SelfHosted 与 AI 深度讨论社区",
            "Reddit 是汇聚全球千万兴趣板块（Subreddits）的深度讨论社区，涵盖 r/SelfHosted、r/HomeLab、r/LocalLLaMA、r/Android 等全球极客一手经验与问答。",
            "1. 优化长帖嵌套评论折叠与代码块高亮渲染；\n2. 提升图片与视频流预加载速度。",
            "https://pool.img.aptoide.com/appupdater/05c2271c0e114965490bee7962608507_icon.png",
            vec![],
            4.7,
            "5 亿+",
            "",
            false,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "2026.42.0".into(),
                version_code: 2642021,
                size_bytes: 115_742_064,
                download_url: "https://pool.apk.aptoide.com/appupdater/com-reddit-frontpage-2642021-77158216-115159ad0389c7389465150c59ae6287.apk".into(),
                updated_at: "2026-04-10".into(),
                md5: "115159ad0389c7389465150c59ae6287".into(),
                changelog: "国际应用源官方构建".into(),
            }],
            vec![],
            vec![],
        ),
        // --- Social (社交通讯扩充，共 17 款) ---
        quick_curated_app(
            "com.tencent.mobileqq",
            "QQ",
            "深圳市腾讯计算机系统有限公司",
            StoreCategory::Social,
            "国民级跨平台即时通讯 · 群文件高速传输、频道社区与高清音视频",
            "腾讯 QQ 基于 NT 全新架构重构，支持超大群聊、百兆群文件不限速共享、屏幕共享与多端历史消息秒级同步。",
            "https://sj.qq.com/appdetail/com.tencent.mobileqq",
            4.6,
            "10 亿+",
            true,
            StoreSource::Tencent,
            "9.1.50",
            318_420_000,
            "https://sj.qq.com/appdetail/com.tencent.mobileqq",
        ),
        quick_curated_app(
            "com.tencent.tim",
            "TIM",
            "深圳市腾讯计算机系统有限公司",
            StoreCategory::Social,
            "腾讯轻量纯净办公版 QQ · 无广告干扰、专注团队沟通与在线文档",
            "TIM 是腾讯出品的轻量化高效沟通协作客户端，完整保留 QQ 好友与群聊生态，界面清爽无冗余娱乐插件，内置在线文档与日程协同。",
            "https://sj.qq.com/appdetail/com.tencent.tim",
            4.8,
            "1.5 亿+",
            false,
            StoreSource::Tencent,
            "4.0.95",
            146_200_000,
            "https://sj.qq.com/appdetail/com.tencent.tim",
        ),
        quick_curated_app(
            "com.sina.weibo",
            "微博",
            "微梦创科网络科技（中国）有限公司",
            StoreCategory::Social,
            "随时随地发现新鲜事 · 实时热搜榜单、数码大V动态与超话社区",
            "新浪微博汇聚全网实时热点新闻、数码科技博主首发评测、名人大V动态与兴趣超话互动讨论。",
            "https://sj.qq.com/appdetail/com.sina.weibo",
            4.5,
            "8 亿+",
            false,
            StoreSource::Tencent,
            "15.3.2",
            162_800_000,
            "https://sj.qq.com/appdetail/com.sina.weibo",
        ),
        quick_curated_app(
            "com.zhihu.android",
            "知乎",
            "智者四海（北京）技术有限公司",
            StoreCategory::Social,
            "有问题，就会有答案 · 科技数码深度问答、专业科普与行业专栏",
            "知乎是中文互联网高质量的问答社区和创作者聚集的原创内容平台，涵盖人工智能、硬件评测、软件工程、职场与科学科普深度解析。",
            "https://sj.qq.com/appdetail/com.zhihu.android",
            4.6,
            "4 亿+",
            false,
            StoreSource::Tencent,
            "10.42.0",
            118_500_000,
            "https://sj.qq.com/appdetail/com.zhihu.android",
        ),
        quick_curated_app(
            "com.coolapk.market",
            "酷安",
            "北京酷安网络科技有限公司",
            StoreCategory::Social,
            "发现科技新生活 · 数码极客、Android 玩机教程与真实硬件评测社区",
            "酷安是国内知名的数码极客与 Android 爱好者讨论社区，汇聚海量手机平板真实机主评测、NAS 折腾心得、桌面搭配与开源好软推荐。",
            "https://sj.qq.com/appdetail/com.coolapk.market",
            4.8,
            "8000 万+",
            true,
            StoreSource::Tencent,
            "15.1.2",
            86_400_000,
            "https://sj.qq.com/appdetail/com.coolapk.market",
        ),
        quick_curated_app(
            "com.douban.frodo",
            "豆瓣",
            "北京豆网科技有限公司",
            StoreCategory::Social,
            "书影音权威评分参考与兴趣小组深度交流精神角落",
            "豆瓣提供图书、电影、音乐的真实大众评分与深度长评，以及数万个垂直兴趣小组（豆瓣小组）的生活交流。",
            "https://sj.qq.com/appdetail/com.douban.frodo",
            4.8,
            "2 亿+",
            false,
            StoreSource::Tencent,
            "7.92.0",
            124_300_000,
            "https://sj.qq.com/appdetail/com.douban.frodo",
        ),
        quick_curated_app(
            "com.baidu.tieba",
            "百度贴吧",
            "百度在线网络技术（北京）有限公司",
            StoreCategory::Social,
            "全球潮流兴趣同好社区 · 图拉丁吧、显卡吧、NAS 吧与游戏攻略聚集地",
            "百度贴吧是以兴趣主题聚合志同道合者的互动平台，涵盖装机硬件、数码外设、单机游戏、体育赛事等海量主题吧。",
            "https://sj.qq.com/appdetail/com.baidu.tieba",
            4.5,
            "6 亿+",
            false,
            StoreSource::Tencent,
            "12.78.1.0",
            132_900_000,
            "https://sj.qq.com/appdetail/com.baidu.tieba",
        ),
        quick_curated_app(
            "cn.soulapp.android",
            "Soul",
            "上海任意门科技有限公司",
            StoreCategory::Social,
            "年轻人的兴趣社交元宇宙 · 瞬间广场、群聊派对与同好匹配",
            "Soul 是基于兴趣图谱建立关系的年轻社交平台，支持无压力生活记录、多人群聊语音派对与多元文化讨论。",
            "https://sj.qq.com/appdetail/cn.soulapp.android",
            4.6,
            "2 亿+",
            false,
            StoreSource::Tencent,
            "5.52.0",
            148_600_000,
            "https://sj.qq.com/appdetail/cn.soulapp.android",
        ),
        // --- Media (影音娱乐扩充，共 18 款) ---
        quick_curated_app(
            "com.netease.cloudmusic",
            "网易云音乐",
            "杭州网易云音乐科技有限公司",
            StoreCategory::Media,
            "超 3 亿音乐爱好者聚集地 · 日推歌单、无损超清母带与乐评社区",
            "网易云音乐专注发现与分享，提供私人雷达每日推荐、海量正版无损曲库、精彩乐评社区、播客电台及跨设备歌词同步。",
            "https://sj.qq.com/appdetail/com.netease.cloudmusic",
            4.8,
            "8 亿+",
            true,
            StoreSource::Tencent,
            "9.2.60",
            198_400_000,
            "https://sj.qq.com/appdetail/com.netease.cloudmusic",
        ),
        quick_curated_app(
            "com.tencent.qqmusic",
            "QQ音乐",
            "深圳市腾讯计算机系统有限公司",
            StoreCategory::Media,
            "千万正版高品曲库 · 臻品母带音质、听歌识曲与车载音响互联",
            "QQ音乐拥有海量正版音乐版权，支持银河音效、臻品全景声、动态伴唱歌词以及手机与电脑端歌单无缝同步。",
            "https://sj.qq.com/appdetail/com.tencent.qqmusic",
            4.7,
            "9 亿+",
            true,
            StoreSource::Tencent,
            "14.3.0.8",
            212_600_000,
            "https://sj.qq.com/appdetail/com.tencent.qqmusic",
        ),
        quick_curated_app(
            "com.luna.music",
            "汽水音乐",
            "北京微播视界科技有限公司",
            StoreCategory::Media,
            "抖音出品官方音乐 App · 懂你的潮流单曲推荐与抖音收藏同步",
            "汽水音乐是抖音官方出品的音乐客户端，支持一键同步抖音收藏音乐、上下滑动沉浸式切歌与高音质电台发现。",
            "https://sj.qq.com/appdetail/com.luna.music",
            4.8,
            "3 亿+",
            false,
            StoreSource::Tencent,
            "13.8.0",
            126_400_000,
            "https://sj.qq.com/appdetail/com.luna.music",
        ),
        quick_curated_app(
            "com.kugou.android",
            "酷狗音乐",
            "广州酷狗计算机科技有限公司",
            StoreCategory::Media,
            "经典正版数字音乐平台 · 蝰蛇音效、逐字精准歌词与HiFi专区",
            "酷狗音乐提供丰富的正版数字曲库、专业蝰蛇音效定制、K歌伴奏以及无损高解析度音频播放。",
            "https://sj.qq.com/appdetail/com.kugou.android",
            4.6,
            "8 亿+",
            false,
            StoreSource::Tencent,
            "20.1.6",
            184_200_000,
            "https://sj.qq.com/appdetail/com.kugou.android",
        ),
        quick_curated_app(
            "com.smile.gifmaker",
            "快手",
            "北京快手科技有限公司",
            StoreCategory::Media,
            "拥抱每一种生活 · 国民级短视频创作、赛事直播与人间烟火社区",
            "快手是记录与分享真实生活的短视频与直播社区，涵盖非遗手艺、乡村生活、体育电竞直播与实用百货经验。",
            "https://sj.qq.com/appdetail/com.smile.gifmaker",
            4.6,
            "9 亿+",
            false,
            StoreSource::Tencent,
            "13.2.40",
            152_800_000,
            "https://sj.qq.com/appdetail/com.smile.gifmaker",
        ),
        quick_curated_app(
            "com.tencent.qqlive",
            "腾讯视频",
            "深圳市腾讯计算机系统有限公司",
            StoreCategory::Media,
            "热播剧集、国漫番剧、NBA 体育赛事与臻彩视听 4K 影院",
            "腾讯视频提供海量正版院线大片、独播剧集、现象级国漫、纪录片以及 NBA/英超高清体育直播。",
            "https://sj.qq.com/appdetail/com.tencent.qqlive",
            4.6,
            "10 亿+",
            false,
            StoreSource::Tencent,
            "8.12.75",
            164_500_000,
            "https://sj.qq.com/appdetail/com.tencent.qqlive",
        ),
        quick_curated_app(
            "com.qiyi.video",
            "爱奇艺",
            "北京爱奇艺科技有限公司",
            StoreCategory::Media,
            "迷雾剧场独播大剧、热门综艺、院线电影与帧绮映画 4K 播放",
            "爱奇艺是高品质视频娱乐服务平台，汇聚迷雾剧场精品悬疑剧、自制综艺、动漫番剧与杜比全景声影院。",
            "https://sj.qq.com/appdetail/com.qiyi.video",
            4.6,
            "9 亿+",
            false,
            StoreSource::Tencent,
            "16.3.5",
            158_900_000,
            "https://sj.qq.com/appdetail/com.qiyi.video",
        ),
        quick_curated_app(
            "com.youku.phone",
            "优酷视频",
            "优酷信息技术（北京）有限公司",
            StoreCategory::Media,
            "这世界很酷 · 白夜剧场、经典港剧、人文文化与高清影视",
            "优酷视频涵盖白夜剧场独播悬疑剧、TVB 经典港剧修复版、羽毛球/斯诺克赛事直播与帧享超高清影院。",
            "https://sj.qq.com/appdetail/com.youku.phone",
            4.5,
            "7 亿+",
            false,
            StoreSource::Tencent,
            "11.1.32",
            142_300_000,
            "https://sj.qq.com/appdetail/com.youku.phone",
        ),
        quick_curated_app(
            "com.ximalaya.ting.android",
            "喜马拉雅",
            "上海喜马拉雅科技有限公司",
            StoreCategory::Media,
            "听书、有声剧、大师课、历史人文播客与助眠白噪音随身听",
            "喜马拉雅是知名音频分享平台，提供海量畅销有声小说、相声评书、历史商业精品课、儿童故事及通勤播客。",
            "https://sj.qq.com/appdetail/com.ximalaya.ting.android",
            4.7,
            "6 亿+",
            false,
            StoreSource::Tencent,
            "9.3.18",
            146_700_000,
            "https://sj.qq.com/appdetail/com.ximalaya.ting.android",
        ),
        quick_curated_app(
            "io.iftech.android.box",
            "小宇宙",
            "宁波追光网络科技有限公司",
            StoreCategory::Media,
            "纯粹高口碑中文播客客户端 · 时间戳高光笔记、单集搜索与深度对谈",
            "小宇宙是专为中文播客听众打造的高品质播客 App，支持单集精细搜索、听众高光时刻共鸣标记与无干扰沉浸收听。",
            "https://sj.qq.com/appdetail/io.iftech.android.box",
            4.9,
            "3500 万+",
            true,
            StoreSource::Tencent,
            "2.82.0",
            68_400_000,
            "https://sj.qq.com/appdetail/io.iftech.android.box",
        ),
        quick_curated_app(
            "com.tencent.weread",
            "微信读书",
            "深圳市腾讯计算机系统有限公司",
            StoreCategory::Media,
            "让阅读不再孤独 · 海量正版出版好书、EPUB/PDF 本地导入与 AI 速读",
            "微信读书提供海量正版出版书籍与网文佳作，支持导入本地 EPUB/PDF/TXT 电子书、跨端云同步书架与划线笔记。",
            "https://sj.qq.com/appdetail/com.tencent.weread",
            4.9,
            "3 亿+",
            true,
            StoreSource::Tencent,
            "8.3.2",
            116_800_000,
            "https://sj.qq.com/appdetail/com.tencent.weread",
        ),
        quick_curated_app(
            "com.dragon.read",
            "番茄免费小说",
            "北京臻鼎科技有限公司",
            StoreCategory::Media,
            "字节跳动旗下正版免费电子书阅读器 · 海量全本小说与真人多角色听书",
            "番茄免费小说提供海量正版免费网络文学与出版物，支持 AI 多情感语音朗读、护眼排版定制与离线全本缓存。",
            "https://sj.qq.com/appdetail/com.dragon.read",
            4.7,
            "5 亿+",
            false,
            StoreSource::Tencent,
            "6.6.5.32",
            104_200_000,
            "https://sj.qq.com/appdetail/com.dragon.read",
        ),
        // --- Productivity (办公效率扩充，共 17 款) ---
        quick_curated_app(
            "com.alibaba.android.rimet",
            "钉钉",
            "钉钉（中国）信息技术有限公司",
            StoreCategory::Productivity,
            "阿里巴巴出品企业级数字化协同办公平台 · 视频会议、闪记与 OA 审批",
            "钉钉提供企业即时通讯、高清音视频会议、钉钉文档、智能人事考勤、项目管理与企业 AI 助理。",
            "https://sj.qq.com/appdetail/com.alibaba.android.rimet",
            4.6,
            "7 亿+",
            true,
            StoreSource::Tencent,
            "7.6.35",
            224_600_000,
            "https://sj.qq.com/appdetail/com.alibaba.android.rimet",
        ),
        quick_curated_app(
            "com.tencent.wework",
            "企业微信",
            "深圳市腾讯计算机系统有限公司",
            StoreCategory::Productivity,
            "连接微信生态的专业办公管理工具 · 企业邮箱、微盘、日程与客户联系",
            "企业微信具有与微信一致的沟通体验，全方位连接微信生态，内置日程、微文档、微盘、会议与企业邮箱。",
            "https://sj.qq.com/appdetail/com.tencent.wework",
            4.7,
            "5 亿+",
            true,
            StoreSource::Tencent,
            "4.1.32",
            246_800_000,
            "https://sj.qq.com/appdetail/com.tencent.wework",
        ),
        quick_curated_app(
            "cn.wps.moffice_eng",
            "WPS Office",
            "珠海金山办公软件有限公司",
            StoreCategory::Productivity,
            "全功能移动办公套件 · 原生支持 Word / Excel / PPT / PDF 编辑与云同步",
            "WPS Office 完美兼容 Office 各类文档格式，集文字、表格、演示、PDF 阅读批注、OCR 扫描与多端云文档同步于一体。",
            "https://sj.qq.com/appdetail/cn.wps.moffice_eng",
            4.7,
            "10 亿+",
            true,
            StoreSource::Tencent,
            "14.12.0",
            172_400_000,
            "https://sj.qq.com/appdetail/cn.wps.moffice_eng",
        ),
        quick_curated_app(
            "com.tencent.wemeet.app",
            "腾讯会议",
            "深圳市腾讯计算机系统有限公司",
            StoreCategory::Productivity,
            "高清流畅云视频会议 · AI 降噪、实时字幕转写、屏幕共享与会议录制",
            "腾讯会议提供跨平台高清低延迟音视频会议服务，支持无线投屏、协作白板、AI 智能会议纪要与万人网络研讨会。",
            "https://sj.qq.com/appdetail/com.tencent.wemeet.app",
            4.8,
            "5 亿+",
            true,
            StoreSource::Tencent,
            "3.32.0",
            168_900_000,
            "https://sj.qq.com/appdetail/com.tencent.wemeet.app",
        ),
        quick_curated_app(
            "com.tencent.docs",
            "腾讯文档",
            "深圳市腾讯计算机系统有限公司",
            StoreCategory::Productivity,
            "多人实时协作在线文档 · 在线表格、收集表、思维导图与微信/QQ一键共享",
            "腾讯文档是一款可多人同时编辑的在线文档工具，支持在线 Word/Excel/PPT、智能表格、收集表与多端实时保存。",
            "https://sj.qq.com/appdetail/com.tencent.docs",
            4.8,
            "3 亿+",
            false,
            StoreSource::Tencent,
            "3.9.10",
            138_200_000,
            "https://sj.qq.com/appdetail/com.tencent.docs",
        ),
        // --- Global AI Flagship Apps (8 款国际顶级 AI 助手 · 装机必备) ---
        build_curated_app(
            "com.openai.chatgpt",
            "ChatGPT",
            "OpenAI",
            StoreCategory::Productivity,
            "AI 助手",
            "OpenAI 官方移动端旗舰 AI 助手 · GPT-4o 实时语音、视觉识别与 o1/o3 深度推理 · 装机必备",
            "ChatGPT 是 OpenAI 出品的全球标杆级人工智能助手。官方 Android 客户端支持 GPT-4o 多模态图文解析、超低延迟高级语音对话模式（Advanced Voice Mode）、o1/o3 复杂逻辑与代码推理、DALL·E 图像生成以及跨设备历史对话实时同步。",
            "1. 升级 GPT-4o 与 o3-mini 推理响应速度；\n2. 优化高级实时语音通话与相机屏幕共享体验。",
            "https://avatars.githubusercontent.com/u/14957082?s=200&v=4",
            vec![],
            4.9,
            "5 亿+",
            "",
            true,
            false,
            vec![
                StoreSourceOffer {
                    source: StoreSource::Global,
                    version_name: "1.2024.289".into(),
                    version_code: 397,
                    size_bytes: 52_151_206,
                    download_url: "https://d.apkpure.com/b/APK/com.openai.chatgpt?versionCode=397".into(),
                    updated_at: "2026-04-12".into(),
                    md5: "".into(),
                    changelog: "国际应用源 OpenAI 下载链接 (v1.2024.289)".into(),
                },
                StoreSourceOffer {
                    source: StoreSource::Official,
                    version_name: "1.2024.285".into(),
                    version_code: 393,
                    size_bytes: 52_151_206,
                    download_url: "https://d.apkpure.com/b/APK/com.openai.chatgpt?versionCode=393".into(),
                    updated_at: "2026-04-01".into(),
                    md5: "".into(),
                    changelog: "OpenAI 官方稳定归档通道 (versionCode 393)".into(),
                },
            ],
            vec![HistoricalVersion {
                version_name: "1.2024.285".into(),
                version_code: 393,
                size_bytes: 52_151_206,
                size_label: "49.7 MB".into(),
                release_date: "2026-04-01".into(),
                source: StoreSource::Official,
                download_url: "https://d.apkpure.com/b/APK/com.openai.chatgpt?versionCode=393".into(),
            }],
            vec![],
        ),
        build_curated_app(
            "com.anthropic.claude",
            "Claude",
            "Anthropic PBC",
            StoreCategory::Productivity,
            "AI 助手",
            "Anthropic 官方 AI 助手 · Claude Sonnet / Opus 顶级代码架构、长文创作与 Artifacts · 装机必备",
            "Claude 是 Anthropic 打造的旗舰级 AI 生产力与编程伙伴。支持 Claude 3.7 / 4 Sonnet 与 Opus 深度推理模型，擅长复杂软件工程代码编写、超长 PDF/代码库精准解析、移动端实时预览 Artifacts 交互组件与多模态图表分析。",
            "1. 新增移动端扩展思考（Extended Thinking）透明推理流；\n2. 优化代码块语法高亮与 Artifacts 实时渲染。",
            "https://pool.img.aptoide.com/8818freak/7959868d3cc002019cf3a1507ee3d476_icon.png",
            vec![],
            4.9,
            "1 亿+",
            "",
            true,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "1.261023.20".into(),
                version_code: 26102320,
                size_bytes: 51_540_913,
                download_url: "https://pool.apk.aptoide.com/8818freak/com-anthropic-claude-26102320-77253781-367ff60a0c62b20bfbcd7b040548dff4.apk".into(),
                updated_at: "2026-04-12".into(),
                md5: "367ff60a0c62b20bfbcd7b040548dff4".into(),
                changelog: "国际应用源 Anthropic 官方正版签名直连".into(),
            }],
            vec![],
            vec![],
        ),
        build_curated_app(
            "com.google.android.apps.bard",
            "Google Gemini",
            "Google LLC",
            StoreCategory::Productivity,
            "AI 助手",
            "Google 官方原生多模态 AI 助手 · Gemini Live 实时对话、百万上下文与全生态协同 · 装机必备",
            "Google Gemini（原 Bard）是 Google 倾力打造的原生多模态人工智能助手。支持 Gemini Pro / Ultra 超长上下文理解、Gemini Live 自然连续语音探讨、拍照识物解题以及与 Gmail、Google Docs、YouTube 的深度智能联动。",
            "1. 提升 Gemini Live 多语种语音对话自然度；\n2. 增强深度研究（Deep Research）报告生成能力。",
            "https://pool.img.aptoide.com/appupdater/2c4e2d188f5df936d954b9228a90d1f3_icon.png",
            vec![],
            4.8,
            "5 亿+",
            "",
            true,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "1.0.995038376".into(),
                version_code: 345,
                size_bytes: 5_351_117,
                download_url: "https://pool.apk.aptoide.com/appupdater/com-google-android-apps-bard-345-77185245-05d309c43261a8a8faed020c99a1ba35.apk".into(),
                updated_at: "2026-04-11".into(),
                md5: "05d309c43261a8a8faed020c99a1ba35".into(),
                changelog: "国际应用源 Google 下载链接".into(),
            }],
            vec![],
            vec![],
        ),
        build_curated_app(
            "ai.perplexity.app.android",
            "Perplexity AI",
            "PerplexityAI",
            StoreCategory::Productivity,
            "AI 助手",
            "新一代 AI 实时答案引擎 · 全网权威信源引用、Pro Search 深度调研与多模型切换 · 装机必备",
            "Perplexity AI 将实时全网深度检索与顶级大语言模型（GPT-4o、Claude Sonnet、DeepSeek R1、Sonar）深度结合。每一个回答均附带可溯源的权威网页与学术论文引文角标，彻底告别大模型幻觉。",
            "1. 升级 Deep Research 深度研究模式，支持百篇信源交叉核验；\n2. 优化语音提问与追问交互流畅度。",
            "https://pool.img.aptoide.com/appupdater/9fe3540e7c1ba97e43658100b42b0857_icon.png",
            vec![],
            4.9,
            "1 亿+",
            "",
            true,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "2.102.0".into(),
                version_code: 260671,
                size_bytes: 98_191_374,
                download_url: "https://pool.apk.aptoide.com/appupdater/ai-perplexity-app-android-260671-77223912-2ab349b601e91e8042ed504f755cfa19.apk".into(),
                updated_at: "2026-04-11".into(),
                md5: "2ab349b601e91e8042ed504f755cfa19".into(),
                changelog: "国际应用源 Perplexity 下载链接".into(),
            }],
            vec![],
            vec![],
        ),
        build_curated_app(
            "ai.x.grok",
            "Grok",
            "xAI",
            StoreCategory::Productivity,
            "AI 助手",
            "马斯克 xAI 出品旗舰大模型 · Grok 3 深度思考、DeepSearch 与 X 平台实时情报 · 装机必备",
            "Grok 是埃隆·马斯克旗下 xAI 团队打造的独立 AI 助手客户端。独家接入 X（Twitter）全球实时信息流与 DeepSearch 引擎，具备极强的数理逻辑推理（Think 模式）、实时热点追踪、幽默犀利对话与高逼真 Aurora 图像生成能力。",
            "1. 新增 Grok Think 深度推理链路可视化与 DeepSearch；\n2. 优化多模态相机识图与语音模式延迟。",
            "https://pool.img.aptoide.com/appupdater/a6e4ed22c7e97f2859e914a04d279cb3_icon.png",
            vec![],
            4.9,
            "1 亿+",
            "",
            true,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "1.2.51-release.00".into(),
                version_code: 11251200,
                size_bytes: 54_287_632,
                download_url: "https://pool.apk.aptoide.com/appupdater/ai-x-grok-11251200-77279753-3b6e07135431707bf6dacb168126fe9d.apk".into(),
                updated_at: "2026-04-12".into(),
                md5: "3b6e07135431707bf6dacb168126fe9d".into(),
                changelog: "国际应用源 xAI 下载链接".into(),
            }],
            vec![],
            vec![],
        ),
        build_curated_app(
            "com.microsoft.copilot",
            "Microsoft Copilot",
            "Microsoft Corporation",
            StoreCategory::Productivity,
            "AI 助手",
            "微软官方全能 AI 生产力伴侣 · GPT-4o 驱动、Copilot Voice 语音与 Office 办公协同 · 装机必备",
            "Microsoft Copilot 是微软推出的日常 AI 生产力助手，融合最新 OpenAI 模型与必应实时联网搜索。支持自然流畅的 Copilot Voice 语音交谈、Copilot Vision 视觉辅助、商务邮件公文撰写及 DALL·E 3 创意画图。",
            "1. 升级 Copilot Daily 每日资讯语音播报与 Think Deeper 深度推理；\n2. 优化长文档摘要与表格整理体验。",
            "https://pool.img.aptoide.com/sirkesyone-applikes/55f82c62e0df7ff8a824835cf23f5428_icon.png",
            vec![],
            4.8,
            "2 亿+",
            "",
            true,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "30.0.440922003".into(),
                version_code: 440922003,
                size_bytes: 45_897_737,
                download_url: "https://pool.apk.aptoide.com/sirkesyone-applikes/com-microsoft-copilot-440922003-76812735-60a87c03c9cadff9761635f868a27f3a.apk".into(),
                updated_at: "2026-04-09".into(),
                md5: "60a87c03c9cadff9761635f868a27f3a".into(),
                changelog: "国际应用源微软下载链接".into(),
            }],
            vec![],
            vec![],
        ),
        build_curated_app(
            "com.poe.android",
            "Poe - Fast AI Chat",
            "Quora, Inc.",
            StoreCategory::Productivity,
            "AI 助手",
            "Quora 出品一站式全球大模型聚合平台 · 同屏畅聊 GPT-4o / Claude / Gemini / DeepSeek · 装机必备",
            "Poe（Platform for Open Exploration）由知名问答社区 Quora 打造，让您在一个客户端内无缝切换并对比 OpenAI GPT-4o/o3、Anthropic Claude 3.7 Sonnet、Google Gemini Pro、DeepSeek-R1、Llama 及 Flux 绘图等全球顶尖 AI 模型。",
            "1. 支持同一对话流内一键切换不同厂商大模型对比回答；\n2. 优化自定义提示词机器人（Custom Bots）加载速度。",
            "https://pool.img.aptoide.com/appupdater/044153e6cb41157b336f90ce55cb436c_icon.png",
            vec![],
            4.8,
            "5000 万+",
            "",
            true,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "a3.77.2".into(),
                version_code: 3077006,
                size_bytes: 28_923_648,
                download_url: "https://pool.apk.aptoide.com/appupdater/com-poe-android-3077006-76666619-b2bc38c7bff9f2017fd58107940a2088.apk".into(),
                updated_at: "2026-04-08".into(),
                md5: "b2bc38c7bff9f2017fd58107940a2088".into(),
                changelog: "国际应用源 Quora 下载链接".into(),
            }],
            vec![],
            vec![],
        ),
        build_curated_app(
            "com.openai.sora",
            "Sora by OpenAI",
            "OpenAI",
            StoreCategory::Productivity,
            "AI 助手",
            "OpenAI 官方 AI 视频生成与创意工坊客户端 · 文本/图片生成电影级高清视频 · 装机必备",
            "Sora by OpenAI 是 OpenAI 推出的旗舰级生成式视频创作工具。支持通过自然语言提示词或参考图像生成具备真实物理世界规律的高清视频片段，提供 Storyboard 分镜编辑、Remix 风格重绘与全球创作者灵感精选。",
            "1. 优化移动端视频生成队列推送与高清预览播放器；\n2. 新增分镜关键帧时间轴微调能力。",
            "https://pool.img.aptoide.com/appupdater/f0b9b6143f600423e5babc6da5fc2667_icon.png",
            vec![],
            4.8,
            "3000 万+",
            "",
            true,
            false,
            vec![StoreSourceOffer {
                source: StoreSource::Global,
                version_name: "1.2026.071".into(),
                version_code: 2607100,
                size_bytes: 29_237_238,
                download_url: "https://pool.apk.aptoide.com/appupdater/com-openai-sora-2607100-73938065-9b40bd10cf4fb8fc34da6b9224d56477.apk".into(),
                updated_at: "2026-04-07".into(),
                md5: "9b40bd10cf4fb8fc34da6b9224d56477".into(),
                changelog: "国际应用源 OpenAI 下载链接".into(),
            }],
            vec![],
            vec![],
        ),
        quick_curated_app(
            "com.deepseek.chat",
            "DeepSeek",
            "杭州深度求索人工智能基础技术研究有限公司",
            StoreCategory::Productivity,
            "深度求索官方 AI 助手 · DeepSeek-R1 深度思考推理与联网搜索",
            "DeepSeek 官方移动端应用，搭载 DeepSeek-V3 与 R1 推理大模型，支持代码编写、数学逻辑推导、长文档分析与实时联网检索。",
            "https://sj.qq.com/appdetail/com.deepseek.chat",
            4.9,
            "2 亿+",
            true,
            StoreSource::Tencent,
            "1.4.2",
            42_600_000,
            "https://sj.qq.com/appdetail/com.deepseek.chat",
        ),
        quick_curated_app(
            "com.larus.nova",
            "豆包",
            "北京春田知韵科技有限公司",
            StoreCategory::Productivity,
            "字节跳动出品全能 AI 助手 · 实时语音通话、拍照解题、AI 写作与绘图",
            "豆包是您的随身智能助手，支持自然流畅的实时语音对话、网页摘要、多语种翻译、文档解读与灵感创作。",
            "https://sj.qq.com/appdetail/com.larus.nova",
            4.8,
            "3 亿+",
            true,
            StoreSource::Tencent,
            "8.1.0",
            112_400_000,
            "https://sj.qq.com/appdetail/com.larus.nova",
        ),
        quick_curated_app(
            "com.moonshot.kimichat",
            "Kimi 智能助手",
            "北京月之暗面科技有限公司",
            StoreCategory::Productivity,
            "超长上下文 AI 生产力助手 · 20 万字长文档精读、学术论文解析与深度搜索",
            "Kimi 擅长处理超长文本与复杂资料整理，支持一次性上传多份 PDF/Word/财报进行对比分析、精准提取核心结论。",
            "https://sj.qq.com/appdetail/com.moonshot.kimichat",
            4.8,
            "1.2 亿+",
            false,
            StoreSource::Tencent,
            "2.3.0",
            64_800_000,
            "https://sj.qq.com/appdetail/com.moonshot.kimichat",
        ),
        quick_curated_app(
            "md.obsidian",
            "Obsidian",
            "Dynalist Inc.",
            StoreCategory::Productivity,
            "本地优先的双链 Markdown 知识库 · 兼容 WebDAV / Syncthing 与 NAS 同步",
            "Obsidian 是强大的本地 Markdown 个人知识库与第二大脑工具，纯文本存储完全自主可控，拥有丰富的社区插件生态与关系图谱视图。",
            "https://pool.img.aptoide.com/appupdater/obsidian_icon.png",
            4.9,
            "2000 万+",
            true,
            StoreSource::GitHub,
            "1.8.9",
            24_600_000,
            "https://github.com/obsidianmd/obsidian-releases/releases/download/v1.8.9/Obsidian-1.8.9.apk",
        ),
        quick_curated_app(
            "notion.id",
            "Notion",
            "Notion Labs, Inc.",
            StoreCategory::Productivity,
            "All-in-One 模块化笔记、知识库、任务看板与团队项目管理系统",
            "Notion 将个人笔记、团队 Wiki 知识库、关系型数据库与敏捷项目看板融为一体，支持多端实时同步。",
            "https://pool.img.aptoide.com/appupdater/51f4f33f9f1322a35340c8cb8f553593_icon.png",
            4.7,
            "1 亿+",
            false,
            StoreSource::Global,
            "0.6.2810",
            39_565_551,
            "https://pool.apk.aptoide.com/appupdater/com-cron-calendar-81-75278244-b11552ae07ec0f1324c71687cd6edd91.apk",
        ),
        quick_curated_app(
            "com.yuque.mobile.android.app",
            "语雀",
            "支付宝（杭州）信息技术有限公司",
            StoreCategory::Productivity,
            "蚂蚁集团出品专业云端知识库 · 结构化技术文档与个人花园",
            "语雀提供体系化的目录知识库管理、流畅的 Markdown 编辑体验与小记速记功能，深受开发者与产品团队喜爱。",
            "https://sj.qq.com/appdetail/com.yuque.mobile.android.app",
            4.8,
            "3000 万+",
            false,
            StoreSource::Tencent,
            "3.12.0",
            82_500_000,
            "https://sj.qq.com/appdetail/com.yuque.mobile.android.app",
        ),
        quick_curated_app(
            "com.baidu.netdisk",
            "百度网盘",
            "北京度友科技有限公司",
            StoreCategory::Productivity,
            "超大容量个人云存储 · 手机相册微信文件备份、在线解压与倍速播放",
            "百度网盘为您提供文件的网络备份、同步和分享服务，支持绿联/群晖/极空间 NAS 双向云同步挂载。",
            "https://sj.qq.com/appdetail/com.baidu.netdisk",
            4.6,
            "8 亿+",
            false,
            StoreSource::Tencent,
            "12.22.4",
            192_300_000,
            "https://sj.qq.com/appdetail/com.baidu.netdisk",
        ),
        quick_curated_app(
            "com.alicloud.databox",
            "阿里云盘",
            "杭州短趣网络传媒技术有限公司",
            StoreCategory::Productivity,
            "阿里巴巴出品高速个人云盘 · 原画影视播放、开放 WebDAV 生态与资料备份",
            "阿里云盘主打高速上传下载体验，支持高清视频在线云播、文档实时预览以及与主流 NAS 系统的云盘直连同步。",
            "https://sj.qq.com/appdetail/com.alicloud.databox",
            4.7,
            "3 亿+",
            false,
            StoreSource::Tencent,
            "6.8.2",
            154_200_000,
            "https://sj.qq.com/appdetail/com.alicloud.databox",
        ),
        quick_curated_app(
            "com.quark.browser",
            "夸克",
            "优视科技（中国）有限公司",
            StoreCategory::Productivity,
            "极简 AI 搜索、夸克网盘大文件高速传输与相机文档扫描全能王",
            "夸克集成了无广告极简浏览器、超大空间夸克网盘、PDF 格式转换以及高精度试卷/票据扫描工具。",
            "https://sj.qq.com/appdetail/com.quark.browser",
            4.7,
            "5 亿+",
            false,
            StoreSource::Tencent,
            "7.8.5",
            136_400_000,
            "https://sj.qq.com/appdetail/com.quark.browser",
        ),
        quick_curated_app(
            "com.tencent.androidqqmail",
            "QQ邮箱",
            "深圳市腾讯计算机系统有限公司",
            StoreCategory::Productivity,
            "多账号全能邮件客户端 · 支持 QQ/Gmail/Outlook/企业邮箱与文件中转站",
            "QQ邮箱全面支持 IMAP/Exchange 通用协议，一个客户端管理国内外所有工作与个人邮箱，内置超大附件中转站。",
            "https://sj.qq.com/appdetail/com.tencent.androidqqmail",
            4.7,
            "4 亿+",
            false,
            StoreSource::Tencent,
            "6.6.8",
            122_100_000,
            "https://sj.qq.com/appdetail/com.tencent.androidqqmail",
        ),
        quick_curated_app(
            "com.netease.mobimail",
            "网易邮箱大师",
            "杭州网易质云科技有限公司",
            StoreCategory::Productivity,
            "专业全平台邮件管理客户端 · 支持 163/126/Gmail/校园与企业邮极速推送",
            "网易邮箱大师支持全球超过 100 万种邮箱域名一键免配置登录，提供毫秒级新邮件推送与多端签名同步。",
            "https://sj.qq.com/appdetail/com.netease.mobimail",
            4.7,
            "2 亿+",
            false,
            StoreSource::Tencent,
            "7.24.2",
            114_800_000,
            "https://sj.qq.com/appdetail/com.netease.mobimail",
        ),
        // --- Tools (实用工具扩充，共 19 款) ---
        quick_curated_app(
            "com.synology.dsdrive",
            "Synology Drive",
            "群晖科技 (Synology Inc.)",
            StoreCategory::Tools,
            "群晖 NAS 私有云企业网盘与个人多端文件双向实时同步客户端",
            "Synology Drive 让您随时随地访问和管理群晖 NAS 中的个人与团队共享文件，支持手机文件夹双向同步、离线访问与协同办公文档。",
            "https://pp.myapp.com/ma_icon/0/icon_52651842_1721000000/256",
            4.8,
            "120 万+",
            true,
            StoreSource::Official,
            "3.9.0-1117",
            76_521_000,
            "https://global.synologydownload.com/download/Mobile/Android-Drive/3.9.0-1117/Android-SynologyDrive.3.9.0-1117.cn.apk",
        ),
        quick_curated_app(
            "com.synology.projectkailash",
            "Synology Photos",
            "群晖科技 (Synology Inc.)",
            StoreCategory::Tools,
            "群晖 DSM 7.x 官方智能相册 · 手机原图无感备份、人脸与地理位置聚类",
            "Synology Photos 是群晖 DSM 7 系统专属相册管理应用，支持后台增量原图备份、家庭共享空间（Shared Space）与时间线快速检索。",
            "https://pp.myapp.com/ma_icon/0/icon_54012011_1721000000/256",
            4.8,
            "110 万+",
            false,
            StoreSource::Official,
            "2.4.2-666",
            84_120_000,
            "https://global.synologydownload.com/download/Mobile/Android-Photos/2.4.2-666/Android-SynologyPhotos.2.4.2-666.cn.apk",
        ),
        quick_curated_app(
            "com.microsoft.emmx",
            "Microsoft Edge",
            "Microsoft Corporation",
            StoreCategory::Tools,
            "微软跨平台旗舰浏览器 · 扩展插件支持、Copilot 侧边栏与桌面端防追踪同步",
            "Microsoft Edge 移动端支持安装篡改猴（Tampermonkey）、uBlock Origin 等浏览器扩展，并与电脑端 Edge 实时同步收藏夹、密码及标签页。",
            "https://sj.qq.com/appdetail/com.microsoft.emmx",
            4.8,
            "5 亿+",
            true,
            StoreSource::Tencent,
            "134.0.3124.68",
            174_200_000,
            "https://sj.qq.com/appdetail/com.microsoft.emmx",
        ),
        quick_curated_app(
            "org.mozilla.firefox",
            "Firefox 浏览器",
            "Mozilla Foundation",
            StoreCategory::Tools,
            "开源独立隐私保护浏览器 · 丰富扩展附加组件生态与跨设备标签发送",
            "Mozilla Firefox 采用独立 GeckoView 渲染引擎，支持自由安装海量桌面级扩展插件，严格拦截第三方跨站追踪器。",
            "https://pool.img.aptoide.com/appupdater/1b96934d2148bc9c11e3d699f98b83ae_icon.png",
            4.8,
            "5 亿+",
            false,
            StoreSource::Global,
            "159.0",
            68_569_153,
            "https://pool.apk.aptoide.com/appupdater/org-mozilla-firefox-2016191887-77236801-c46f4e7a6ba66e3d8bafacf2b8bf5a86.apk",
        ),
        quick_curated_app(
            "mark.via",
            "Via 浏览器",
            "上海知up网络科技有限公司",
            StoreCategory::Tools,
            "不足 2MB 的极客极简浏览器 · 毫秒级冷启动、自定义广告拦截与油猴脚本",
            "Via 浏览器专为追求纯粹极速体验的极客设计，体积超小、零后台驻留、零新闻推送，内置强大的自定义 CSS/JS 脚本与规则拦截。",
            "https://sj.qq.com/appdetail/mark.via",
            4.9,
            "3000 万+",
            true,
            StoreSource::Tencent,
            "6.1.0",
            2_180_000,
            "https://sj.qq.com/appdetail/mark.via",
        ),
        quick_curated_app(
            "com.x8bit.bitwarden",
            "Bitwarden",
            "Bitwarden Inc. (Open Source)",
            StoreCategory::Tools,
            "开源跨平台密码管理器 · 支持官方云端与 NAS Vaultwarden 自托管服务端",
            "Bitwarden 是安全审计透明的开源密码与通行密钥（Passkey）管理工具，完美支持在绿联/飞牛/群晖 Docker 中自建 Vaultwarden 私有密码库。",
            "https://pool.img.aptoide.com/appupdater/5b37085e5661d10ae4a8fcae828b483a_icon.png",
            4.9,
            "1500 万+",
            true,
            StoreSource::Global,
            "2026.10.0",
            21_567_997,
            "https://pool.apk.aptoide.com/appupdater/com-x8bit-bitwarden-21955-77144603-5d642e220b15f7dc9ae93af0613fba6e.apk",
        ),
        quick_curated_app(
            "org.kde.kdeconnect_tp",
            "KDE Connect",
            "KDE Community (Open Source)",
            StoreCategory::Tools,
            "开源 Linux / 桌面跨设备无线协同神器 · 剪贴板共享、远程触控板与通知同步",
            "KDE Connect 实现 Android 手机与 Linux（GNOME GSConnect / KDE）及 Windows 电脑的深度局域网互联，支持无线文件投送、多媒体控制与远程鼠标键盘。",
            "https://f-droid.org/repo/org.kde.kdeconnect_tp/en-US/icon.png",
            4.9,
            "800 万+",
            false,
            StoreSource::FDroid,
            "1.32.5",
            18_900_000,
            "https://f-droid.org/repo/org.kde.kdeconnect_tp_13205.apk",
        ),
        quick_curated_app(
            "com.follow.clash",
            "FlClash",
            "chen08209 (Open Source)",
            StoreCategory::Tools,
            "基于 ClashMeta 内核的多平台开源网络代理与分流诊断工具",
            "FlClash 采用 Material You 现代设计语言，内置 Clash.Meta (Mihomo) 高性能内核，支持多端配置同步、智能规则分流与低功耗后台运行。",
            "https://f-droid.org/repo/com.follow.clash/en-US/icon.png",
            4.9,
            "600 万+",
            true,
            StoreSource::GitHub,
            "0.8.80",
            38_200_000,
            "https://github.com/chen08209/FlClash/releases/download/v0.8.80/FlClash-0.8.80-android-arm64-v8a.apk",
        ),
        quick_curated_app(
            "com.zerotier.one",
            "ZeroTier One",
            "ZeroTier, Inc.",
            StoreCategory::Tools,
            "全球软件定义虚拟二层局域网 SD-WAN · P2P 异地组网直连家庭内网与服务器",
            "ZeroTier One 可将分布在不同网络下的手机、电脑与 NAS 组建在同一个虚拟以太网交换机下，支持自建 Moon 节点加速点对点直连。",
            "https://pool.img.aptoide.com/appupdater/755386a1b4478ec21f12a87c497cca53_icon.png",
            4.8,
            "500 万+",
            false,
            StoreSource::Global,
            "1.16.0-1",
            13_508_490,
            "https://pool.apk.aptoide.com/appupdater/com-zerotier-one-193-71762287-535d419f75c7b7ee317a9d0650c76f56.apk",
        ),
        quick_curated_app(
            "moe.shizuku.privileged.api",
            "Shizuku",
            "RikkaApps (Open Source)",
            StoreCategory::Tools,
            "免 Root 调用系统高权限 ADB API 框架 · 玩机冻结、文件管理与自动化必备",
            "Shizuku 让普通应用无需 Root 即可通过无线调试或电脑 ADB 授权直接调用 Android 系统级 API，大幅提升操作效率与安全性。",
            "https://github.com/RikkaApps/Shizuku/raw/master/manager/src/main/ic_launcher-playstore.png",
            4.9,
            "1200 万+",
            true,
            StoreSource::GitHub,
            "13.5.4",
            4_120_000,
            "https://github.com/RikkaApps/Shizuku/releases/download/v13.5.4/shizuku-v13.5.4.r1049.0e53409-release.apk",
        ),
        quick_curated_app(
            "ru.zdevs.zarchiver",
            "ZArchiver",
            "ZDevs",
            StoreCategory::Tools,
            "专业高速归档解压缩管理器 · 支持 7z / ZIP / RAR / TAR / ZSTD 与分卷加密解压",
            "ZArchiver 是 Android 平台上口碑极佳的免费解压缩与文件管理工具，支持多线程极速解压、带密码分卷包处理及 APK/OBB 查看。",
            "https://pool.img.aptoide.com/catappult/0c79669028e26dd575985f4dee66f609_icon.png",
            4.9,
            "1 亿+",
            false,
            StoreSource::Global,
            "1.0.10",
            5_608_239,
            "https://pool.apk.aptoide.com/catappult/ru-zdevs-zarchiver-101036-71757041-1ee5ec7bc9aab1da18d898fc5044899a.apk",
        ),
        // --- Lifestyle (生活出行扩充，共 16 款) ---
        quick_curated_app(
            "com.sankuai.meituan",
            "美团",
            "北京三快在线科技有限公司",
            StoreCategory::Lifestyle,
            "美好生活小帮手 · 外卖闪购、团购美食、酒店机票与共享单车一站式服务",
            "美团为您提供美食团购、美团外卖、闪电买菜买药、酒店旅游预订、电影演出选座及打车骑行等全方位本地生活服务。",
            "https://sj.qq.com/appdetail/com.sankuai.meituan",
            4.7,
            "10 亿+",
            true,
            StoreSource::Tencent,
            "12.28.404",
            118_600_000,
            "https://sj.qq.com/appdetail/com.sankuai.meituan",
        ),
        quick_curated_app(
            "com.taobao.taobao",
            "淘宝",
            "淘宝（中国）软件有限公司",
            StoreCategory::Lifestyle,
            "万能的淘宝 · 国民级综合网购商城、百亿补贴、物流实时追踪与直播购物",
            "手机淘宝汇聚全球海量数码百货、服饰家居与特色好物，支持拍照识图找同款、菜鸟裹裹物流实时推送与极速退换货服务。",
            "https://sj.qq.com/appdetail/com.taobao.taobao",
            4.6,
            "10 亿+",
            true,
            StoreSource::Tencent,
            "10.42.10",
            142_800_000,
            "https://sj.qq.com/appdetail/com.taobao.taobao",
        ),
        quick_curated_app(
            "com.jingdong.app.mall",
            "京东",
            "北京京东世纪贸易有限公司",
            StoreCategory::Lifestyle,
            "多快好省 · 正品 3C 数码家电自营、211 限时达次日达与京东秒送",
            "京东 APP 主打正品自营 3C 数码、电脑配件、NAS 硬盘及家电百货，依托京东物流提供极速当日达/次日达与上门换新服务。",
            "https://sj.qq.com/appdetail/com.jingdong.app.mall",
            4.7,
            "9 亿+",
            true,
            StoreSource::Tencent,
            "13.6.2",
            126_400_000,
            "https://sj.qq.com/appdetail/com.jingdong.app.mall",
        ),
        quick_curated_app(
            "com.xunmeng.pinduoduo",
            "拼多多",
            "上海寻梦信息技术有限公司",
            StoreCategory::Lifestyle,
            "多实惠，多乐趣 · 百亿补贴官方正品数码家电与源头产地生鲜直发",
            "拼多多通过社交拼团与源头供应链直连，提供百亿补贴正品手机数码、日用百货与农副生鲜好物。",
            "https://sj.qq.com/appdetail/com.xunmeng.pinduoduo",
            4.6,
            "9 亿+",
            false,
            StoreSource::Tencent,
            "7.36.0",
            92_400_000,
            "https://sj.qq.com/appdetail/com.xunmeng.pinduoduo",
        ),
        quick_curated_app(
            "com.taobao.idlefish",
            "闲鱼",
            "杭州闲鱼信息科技有限公司",
            StoreCategory::Lifestyle,
            "阿里巴巴旗下闲置交易社区 · 数码硬件流转、复古好物与同城面交",
            "闲鱼是极客升级换代显卡、手机、NAS 硬件与摄影器材的高频闲置流转平台，支持支付宝担保交易与验货宝服务。",
            "https://sj.qq.com/appdetail/com.taobao.idlefish",
            4.7,
            "5 亿+",
            false,
            StoreSource::Tencent,
            "7.18.60",
            128_500_000,
            "https://sj.qq.com/appdetail/com.taobao.idlefish",
        ),
        quick_curated_app(
            "com.MobileTicket",
            "铁路12306",
            "中国铁道科学研究院集团有限公司",
            StoreCategory::Lifestyle,
            "中国铁路官方购票出行平台 · 高铁候补抢票、在线选座、刷证进站与延误提醒",
            "铁路12306 是中国铁路唯一官方手机购票应用，无任何捆绑加速包，支持官方候补购票、静音车厢选座、高铁外卖订餐及电子发票开具。",
            "https://sj.qq.com/appdetail/com.MobileTicket",
            4.8,
            "8 亿+",
            true,
            StoreSource::Tencent,
            "5.8.2.12",
            106_400_000,
            "https://sj.qq.com/appdetail/com.MobileTicket",
        ),
        quick_curated_app(
            "com.sdu.didi.psnger",
            "滴滴出行",
            "北京小桔科技有限公司",
            StoreCategory::Lifestyle,
            "一站式移动出行平台 · 快车专车、出租车、顺风车与代驾接送机服务",
            "滴滴出行提供快车、礼帽专车、豪华车、顺风车与安全代驾服务，支持行程分享与背屏取件/车牌提醒联动。",
            "https://sj.qq.com/appdetail/com.sdu.didi.psnger",
            4.7,
            "6 亿+",
            false,
            StoreSource::Tencent,
            "7.0.22",
            134_200_000,
            "https://sj.qq.com/appdetail/com.sdu.didi.psnger",
        ),
        quick_curated_app(
            "com.baidu.BaiduMap",
            "百度地图",
            "北京百度网讯科技有限公司",
            StoreCategory::Lifestyle,
            "AI 车道级安全导航、真城市沉浸三维地图、红绿灯倒计时与跨城出行规划",
            "百度地图搭载文心交通大模型，提供全国车道级导航、实时公交地铁到站预测、智能停车位推荐与新能源充电桩地图。",
            "https://sj.qq.com/appdetail/com.baidu.BaiduMap",
            4.7,
            "8 亿+",
            false,
            StoreSource::Tencent,
            "20.8.0",
            182_600_000,
            "https://sj.qq.com/appdetail/com.baidu.BaiduMap",
        ),
        quick_curated_app(
            "ctrip.android.view",
            "携程旅行",
            "上海携程商务有限公司",
            StoreCategory::Lifestyle,
            "一站式差旅与度假预订平台 · 全球特价机票、酒店民宿、火车票与景点门票",
            "携程旅行提供国内外酒店预订、机票比价、高铁抢票、租车接送机及全球签证攻略，商旅报销凭证一键导出。",
            "https://sj.qq.com/appdetail/ctrip.android.view",
            4.7,
            "5 亿+",
            false,
            StoreSource::Tencent,
            "8.74.2",
            124_800_000,
            "https://sj.qq.com/appdetail/ctrip.android.view",
        ),
        quick_curated_app(
            "me.ele",
            "饿了么",
            "上海拉扎斯信息科技有限公司",
            StoreCategory::Lifestyle,
            "品质美食外卖、生鲜果蔬、超市百货与 24 小时买药准时送达",
            "饿了么连接周边品质餐厅、大型商超与连锁药房，提供准时宝配送保障与丰盛下午茶夜宵服务。",
            "https://sj.qq.com/appdetail/me.ele",
            4.6,
            "6 亿+",
            false,
            StoreSource::Tencent,
            "11.18.58",
            98_600_000,
            "https://sj.qq.com/appdetail/me.ele",
        ),
        quick_curated_app(
            "com.dianping.v1",
            "大众点评",
            "上海汉涛信息咨询有限公司",
            StoreCategory::Lifestyle,
            "发现城市品质好店 · 必吃榜餐厅参考、真实探店评价、团购优惠与休闲玩乐",
            "大众点评汇聚全国城市美食必吃榜、真实食客评价、周末展览演出及丽人休闲团购指南。",
            "https://sj.qq.com/appdetail/com.dianping.v1",
            4.7,
            "5 亿+",
            false,
            StoreSource::Tencent,
            "11.26.12",
            112_900_000,
            "https://sj.qq.com/appdetail/com.dianping.v1",
        ),
        quick_curated_app(
            "com.unionpay",
            "云闪付",
            "中国银联股份有限公司",
            StoreCategory::Lifestyle,
            "银联官方银行卡管理与惠民支付平台 · 跨行转账 0 手续费、全行账单查还",
            "云闪付由中国银联与各大商业银行共同打造，支持一键绑定管理名下所有银行卡、实时查余额、跨行免手续费还款及政府消费券领取。",
            "https://sj.qq.com/appdetail/com.unionpay",
            4.8,
            "5 亿+",
            false,
            StoreSource::Tencent,
            "10.1.8",
            116_400_000,
            "https://sj.qq.com/appdetail/com.unionpay",
        ),
        quick_curated_app(
            "com.tmri.app.main",
            "交管12123",
            "公安部交通管理科学研究所",
            StoreCategory::Lifestyle,
            "全国公安交管官方互联网服务平台 · 电子驾驶证、违章查缴、选号与年检预约",
            "交管12123 是官方交通安全综合服务管理客户端，支持申领电子驾驶证、机动车违法处理、期满换证及六年免检标志申领。",
            "https://sj.qq.com/appdetail/com.tmri.app.main",
            4.8,
            "5 亿+",
            false,
            StoreSource::Tencent,
            "3.1.8",
            94_200_000,
            "https://sj.qq.com/appdetail/com.tmri.app.main",
        ),
        quick_curated_app(
            "com.gotokeep.keep",
            "Keep",
            "北京卡路里科技有限公司",
            StoreCategory::Lifestyle,
            "移动健身教练与运动社区 · 居家跟练课程、户外跑步骑行轨迹与饮食热量记录",
            "Keep 提供量身定制的健身训练计划、专业视频跟练、户外跑步里程记录及智能运动手表数据同步。",
            "https://sj.qq.com/appdetail/com.gotokeep.keep",
            4.7,
            "3 亿+",
            false,
            StoreSource::Tencent,
            "8.16.0",
            138_400_000,
            "https://sj.qq.com/appdetail/com.gotokeep.keep",
        ),
    ]
}

fn run_python_json(script: &str, args: &[&str], timeout_secs: u64) -> Result<serde_json::Value> {
    let mut cmd = resources::python_command();
    cmd.arg("-c").arg(script);
    cmd.arg(timeout_secs.to_string());
    for a in args {
        cmd.arg(a);
    }
    let out = cmd.output().context("failed to execute python helper")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!("python helper failed: {}", stderr.trim()));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_str(trimmed).context("invalid json from python helper")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OnlineCatalogCache {
    pub updated_unix: u64,
    pub last_sync_label: String,
    pub apps: Vec<StoreApp>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SearchCacheEntry {
    pub updated_unix: u64,
    pub apps: Vec<StoreApp>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct DetailCacheEntry {
    pub updated_unix: u64,
    pub detail: StoreAppDetailPayload,
}

static MEMORY_ONLINE_CATALOG: OnceLock<Mutex<OnlineCatalogCache>> = OnceLock::new();
static MEMORY_SEARCH_CACHE: OnceLock<Mutex<HashMap<String, SearchCacheEntry>>> = OnceLock::new();
static MEMORY_DETAIL_CACHE: OnceLock<Mutex<HashMap<String, DetailCacheEntry>>> = OnceLock::new();

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn online_catalog_cache_file() -> PathBuf {
    store_cache_root().join("online_catalog_v1.json")
}

fn search_cache_file() -> PathBuf {
    store_cache_root().join("search_cache_v3.json")
}

fn detail_cache_file() -> PathBuf {
    store_cache_root().join("detail_cache_v3.json")
}

fn is_blacklisted_store_item(pkg: &str, name: &str) -> bool {
    let p = pkg.trim();
    let n = name.trim();
    p.is_empty()
        || n.is_empty()
        || p == "com.tencent.android.qqdownloader"
        || p.starts_with("com.tencent.pcgame.")
        || p.starts_with("com.tencent.tmgp.")
        || matches!(n, "安装快捷方式" | "应用宝" | "腾讯应用宝")
}

fn memory_online_catalog() -> &'static Mutex<OnlineCatalogCache> {
    MEMORY_ONLINE_CATALOG.get_or_init(|| {
        let path = online_catalog_cache_file();
        let mut cache = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<OnlineCatalogCache>(&s).ok())
            .unwrap_or_default();
        cache
            .apps
            .retain(|a| !is_blacklisted_store_item(&a.package, &a.name));
        Mutex::new(cache)
    })
}

fn merge_curated_with_online(online_apps: &[StoreApp]) -> Vec<StoreApp> {
    let curated = curated_catalog();
    let curated_name_to_pkg: HashMap<String, String> = curated
        .iter()
        .map(|a| (a.name.trim().to_lowercase(), a.package.clone()))
        .collect();
    let mut combined = curated;
    for app in online_apps {
        if is_blacklisted_store_item(&app.package, &app.name) {
            continue;
        }
        if let Some(canon_pkg) = curated_name_to_pkg.get(&app.name.trim().to_lowercase()) {
            if canon_pkg != &app.package {
                continue;
            }
        }
        combined.push(app.clone());
    }
    let mut merged = merge_and_deduplicate_apps(combined);
    for app in &mut merged {
        if app.cached_icon.is_none() {
            app.cached_icon = existing_cached_icon(&app.package);
        }
    }
    merged
}

pub fn load_startup_catalog() -> (Vec<StoreApp>, Option<String>, usize, bool) {
    let Ok(mut guard) = memory_online_catalog().lock() else {
        let curated = curated_catalog();
        return (curated, None, 0, false);
    };
    if guard.apps.is_empty() {
        let path = online_catalog_cache_file();
        if let Ok(s) = fs::read_to_string(&path) {
            if let Ok(mut disk_cache) = serde_json::from_str::<OnlineCatalogCache>(&s) {
                disk_cache
                    .apps
                    .retain(|a| !is_blacklisted_store_item(&a.package, &a.name));
                if !disk_cache.apps.is_empty() {
                    *guard = disk_cache;
                }
            }
        }
    }
    let online_count = guard.apps.len();
    let has_multi_source = guard.apps.iter().any(|a| {
        a.sources
            .iter()
            .any(|s| matches!(s.source, StoreSource::Global | StoreSource::FDroid))
    });
    let is_fresh = guard.updated_unix > 0
        && now_unix_secs().saturating_sub(guard.updated_unix) < 12 * 3600
        && online_count >= 200
        && has_multi_source;
    let label = (!guard.last_sync_label.trim().is_empty()).then(|| guard.last_sync_label.clone());
    let merged = merge_curated_with_online(&guard.apps);
    (merged, label, online_count, is_fresh)
}

pub fn save_online_catalog_apps(incoming: &[StoreApp]) -> (Vec<StoreApp>, String, usize) {
    let sync_label = chrono::Local::now().format("%H:%M").to_string();
    let mut combined_online: Vec<StoreApp> = incoming
        .iter()
        .filter(|a| !is_blacklisted_store_item(&a.package, &a.name))
        .cloned()
        .collect();

    if let Ok(mut guard) = memory_online_catalog().lock() {
        combined_online.extend(
            guard
                .apps
                .iter()
                .filter(|a| !is_blacklisted_store_item(&a.package, &a.name))
                .cloned(),
        );
        let deduped_online = merge_and_deduplicate_apps(combined_online);
        let online_count = deduped_online.len();
        guard.updated_unix = now_unix_secs();
        guard.last_sync_label = sync_label.clone();
        guard.apps = deduped_online;

        let path = online_catalog_cache_file();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json_str) = serde_json::to_string(&*guard) {
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, json_str).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
        let full_merged = merge_curated_with_online(&guard.apps);
        return (full_merged, sync_label, online_count);
    }

    let deduped_online = merge_and_deduplicate_apps(combined_online);
    let online_count = deduped_online.len();
    let full_merged = merge_curated_with_online(&deduped_online);
    (full_merged, sync_label, online_count)
}

const PY_FETCH_ONLINE_CATALOG: &str = r#"
import sys, json, re, urllib.request, urllib.parse
from concurrent.futures import ThreadPoolExecutor

timeout = float(sys.argv[1]) if len(sys.argv) > 1 else 25.0
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"

CATEGORY_JOBS = [
    ("110005", "All", "装机必备"),
    ("200002", "Media", "视频影视"),
    ("200004", "Social", "即时通讯"),
    ("200009", "Lifestyle", "生活服务"),
    ("200006", "Productivity", "协同办公"),
    ("200003", "Media", "音乐电台"),
    ("200011", "Productivity", "AI 助手"),
    ("200007", "Productivity", "教育学习"),
    ("200005", "Media", "小说听书"),
    ("210056", "Tools", "浏览器"),
    ("200008", "Lifestyle", "金融理财"),
    ("310010", "Media", "图像摄影"),
    ("200012", "Tools", "系统工具"),
    ("200018", "Productivity", "文档笔记"),
    ("200014", "Productivity", "网盘存储"),
    ("200015", "Tools", "文件解压"),
    ("200013", "Tools", "输入法"),
    ("200016", "Tools", "安全网络"),
    ("200010", "Tools", "系统工具"),
    ("200017", "Tools", "系统工具"),
]

SEARCH_JOBS = [
    ("即时通讯 聊天", "Social", "即时通讯"),
    ("微信 QQ 聊天 通讯", "Social", "即时通讯"),
    ("微博 社区 论坛", "Social", "社区论坛"),
    ("知乎 小红书 酷安 问答", "Social", "社区论坛"),
    ("贴吧 虎扑 豆瓣 兴趣", "Social", "兴趣同好"),
    ("二次元 动漫社区 手办 同好", "Social", "兴趣同好"),
    ("语音 交友 派对", "Social", "语音交友"),
    ("恋爱 相亲 同城", "Social", "语音交友"),
    ("视频 影视 追剧", "Media", "视频影视"),
    ("电视剧 电影 动漫", "Media", "视频影视"),
    ("音乐 听歌 K歌", "Media", "音乐电台"),
    ("电台 播客 助眠 铃声", "Media", "音乐电台"),
    ("短视频 直播", "Media", "短视频直播"),
    ("弹幕 娱乐直播 短剧", "Media", "短视频直播"),
    ("小说 听书 阅读 漫画", "Media", "小说听书"),
    ("电子书 免费小说 有声书", "Media", "小说听书"),
    ("相机 修图 剪辑 相册", "Media", "图像摄影"),
    ("美颜 滤镜 拼图 视频制作", "Media", "图像摄影"),
    ("本地播放器 投屏", "Media", "本地播放"),
    ("视频解码 字幕 播放器", "Media", "本地播放"),
    ("协同办公 视频会议", "Productivity", "协同办公"),
    ("考勤 打卡 企业办公 远程桌面", "Productivity", "协同办公"),
    ("AI助手 大模型 智能写作", "Productivity", "AI 助手"),
    ("AI问答 智能助手 DeepSeek 豆包", "Productivity", "AI 助手"),
    ("文档 表格 笔记 便签", "Productivity", "文档笔记"),
    ("思维导图 知识库 待办 日程", "Productivity", "文档笔记"),
    ("网盘 云盘 备份", "Productivity", "网盘存储"),
    ("学习 词典 翻译 背单词", "Productivity", "教育学习"),
    ("考研 考公 驾考 题库 网课", "Productivity", "教育学习"),
    ("邮箱 扫描 PDF 思维导图", "Productivity", "邮箱扫描"),
    ("扫描王 邮件 名片识别 文字识别", "Productivity", "邮箱扫描"),
    ("系统工具 手机管家 清理", "Tools", "系统工具"),
    ("跑分 换机 桌面 壁纸 省电", "Tools", "系统工具"),
    ("私有云 NAS 远程控制", "Tools", "NAS 私有云"),
    ("浏览器 搜索", "Tools", "浏览器"),
    ("输入法 键盘", "Tools", "输入法"),
    ("文件管理 解压 互传", "Tools", "文件解压"),
    ("安全 WiFi 测速 密码 计算器 天气", "Tools", "安全网络"),
    ("日历 万年历 闹钟 指南针 录音", "Tools", "安全网络"),
    ("网购 商城 电商 二手", "Lifestyle", "网购商城"),
    ("特卖 百货 拼团 导购 返利", "Lifestyle", "网购商城"),
    ("外卖 美食 买菜 咖啡", "Lifestyle", "外卖美食"),
    ("菜谱 生鲜 超市 奶茶 点餐", "Lifestyle", "外卖美食"),
    ("地图 导航 公交 地铁", "Lifestyle", "地图导航"),
    ("路况 卫星地图 充电桩 违章", "Lifestyle", "地图导航"),
    ("打车 火车票 酒店 旅游", "Lifestyle", "打车差旅"),
    ("机票 航班 顺风车 租车 民宿", "Lifestyle", "打车差旅"),
    ("银行 支付 理财 记账", "Lifestyle", "金融理财"),
    ("股票 证券 基金 信用卡 社保", "Lifestyle", "金融理财"),
    ("运动 健康 快递 招聘 租房", "Lifestyle", "生活服务"),
    ("买房 医疗 挂号 家政 查企业", "Lifestyle", "生活服务"),
]

APTOIDE_JOBS = [
    ("https://ws75.aptoide.com/api/7/listApps?limit=50&offset=0&sort=downloads7d&aab=1", "Auto", "", True),
    ("https://ws75.aptoide.com/api/7/listApps?limit=50&offset=50&sort=downloads7d&aab=1", "Auto", "", False),
    ("https://ws75.aptoide.com/api/7/listApps?limit=50&offset=100&sort=downloads7d&aab=1", "Auto", "", False),
    ("https://ws75.aptoide.com/api/7/apps/search?query=ai&limit=25&aab=1", "Productivity", "AI 助手", False),
    ("https://ws75.aptoide.com/api/7/apps/search?query=chatgpt&limit=20&aab=1", "Productivity", "AI 助手", False),
    ("https://ws75.aptoide.com/api/7/apps/search?query=claude&limit=20&aab=1", "Productivity", "AI 助手", False),
    ("https://ws75.aptoide.com/api/7/apps/search?query=nas&limit=20&aab=1", "Tools", "NAS 私有云", False),
    ("https://ws75.aptoide.com/api/7/apps/search?query=browser&limit=25&aab=1", "Tools", "浏览器", False),
    ("https://ws75.aptoide.com/api/7/apps/search?query=music&limit=25&aab=1", "Media", "音乐电台", False),
    ("https://ws75.aptoide.com/api/7/apps/search?query=video&limit=25&aab=1", "Media", "视频影视", False),
    ("https://ws75.aptoide.com/api/7/apps/search?query=office&limit=25&aab=1", "Productivity", "协同办公", False),
    ("https://ws75.aptoide.com/api/7/apps/search?query=developer&limit=25&aab=1", "Auto", "", False),
    ("https://ws75.aptoide.com/api/7/apps/search?query=tools&limit=25&aab=1", "Auto", "", False),
]

FDROID_QUERIES = [
    ("termux", "Tools", "系统工具"),
    ("nextcloud", "Productivity", "网盘存储"),
    ("syncthing", "Tools", "文件解压"),
    ("keepass", "Tools", "安全网络"),
    ("markdown", "Productivity", "文档笔记"),
]

BAD_CARDS = {
    "yybn_searchresult_hotgame",
    "yybn_searchresult_reservegame",
    "YYB_HOME_SEARCH_CLOUD_GAME",
    "YYB_HOME_SEARCH_WECHAT_GAME",
    "YYB_HOME_APP_LIBRARY_CATEGORY",
}

def map_cat_sub(cate_new, hint_cat, hint_sub):
    c = (cate_new or "").strip()
    if hint_cat == "All":
        if c == "社交": return ("Social", "即时通讯", True)
        if c == "视频": return ("Media", "视频影视", True)
        if c == "音乐": return ("Media", "音乐电台", True)
        if c == "阅读": return ("Media", "小说听书", True)
        if c == "摄影": return ("Media", "图像摄影", True)
        if c == "办公": return ("Productivity", "协同办公", True)
        if c == "教育": return ("Productivity", "教育学习", True)
        if c == "购物": return ("Lifestyle", "网购商城", True)
        if c == "美食": return ("Lifestyle", "外卖美食", True)
        if c == "交通出行": return ("Lifestyle", "地图导航", True)
        if c == "旅游出行": return ("Lifestyle", "打车差旅", True)
        if c == "理财": return ("Lifestyle", "金融理财", True)
        if c in ("生活", "健康", "汽车", "母婴"): return ("Lifestyle", "生活服务", True)
        return ("Tools", "系统工具", True)
    if c == "社交":
        return ("Social", hint_sub if hint_cat == "Social" else "社区论坛", False)
    if c in ("视频", "音乐", "阅读", "摄影"):
        if hint_cat == "Media":
            return ("Media", hint_sub, False)
        if c == "视频": return ("Media", "视频影视", False)
        if c == "音乐": return ("Media", "音乐电台", False)
        if c == "阅读": return ("Media", "小说听书", False)
        return ("Media", "图像摄影", False)
    if c == "教育":
        return ("Productivity", "教育学习", False)
    if c == "办公":
        if hint_cat == "Productivity":
            return ("Productivity", hint_sub, False)
        if hint_sub in ("NAS 私有云", "文件解压"):
            return (hint_cat, hint_sub, False)
        return ("Productivity", "协同办公", False)
    if c in ("购物", "美食", "交通出行", "旅游出行", "理财", "生活", "健康", "汽车", "母婴"):
        if hint_cat == "Lifestyle":
            return ("Lifestyle", hint_sub, False)
        if c == "购物": return ("Lifestyle", "网购商城", False)
        if c == "美食": return ("Lifestyle", "外卖美食", False)
        if c == "交通出行": return ("Lifestyle", "地图导航", False)
        if c == "旅游出行": return ("Lifestyle", "打车差旅", False)
        if c == "理财": return ("Lifestyle", "金融理财", False)
        return ("Lifestyle", "生活服务", False)
    if hint_sub in (
        "AI 助手", "网盘存储", "邮箱扫描", "文档笔记", "协同办公",
        "本地播放", "NAS 私有云", "浏览器", "输入法", "文件解压", "安全网络", "系统工具",
    ):
        return (hint_cat, hint_sub, False)
    return ("Tools", "系统工具", False)

def run_tencent_job(job):
    url, hcat, hsub = job
    out = []
    try:
        req = urllib.request.Request(
            url,
            headers={"User-Agent": UA, "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8"},
        )
        with urllib.request.urlopen(req, timeout=min(timeout, 6.0)) as r:
            html = r.read().decode("utf-8", errors="ignore")
        m = re.search(r'<script[^>]*id="__NEXT_DATA__"[^>]*>(.*?)</script>', html, re.S)
        if not m:
            return out
        data = json.loads(m.group(1))
        comps = (
            data.get("props", {})
            .get("pageProps", {})
            .get("dynamicCardResponse", {})
            .get("data", {})
            .get("components", [])
        )
        for comp in comps:
            card_id = (comp.get("cardId") or comp.get("componentId") or "").strip()
            if card_id in BAD_CARDS:
                continue
            for it in ((comp.get("data") or {}).get("itemData") or []):
                if not isinstance(it, dict):
                    continue
                pkg = (it.get("pkg_name") or "").strip()
                name = (it.get("name") or "").strip()
                if not pkg or not name:
                    continue
                if (
                    pkg.startswith("com.tencent.pcgame.")
                    or pkg.startswith("com.tencent.tmgp.")
                    or pkg == "com.tencent.android.qqdownloader"
                ):
                    continue
                if name in ("安装快捷方式", "应用宝", "腾讯应用宝"):
                    continue
                if (
                    str(it.get("game_type") or "") == "7"
                    or str(it.get("is_game") or "") == "1"
                    or str(it.get("is_booking") or "") == "1"
                    or str(it.get("is_cloud_game") or "") == "1"
                ):
                    continue
                cate_new = (it.get("cate_name_new") or "").strip()
                if cate_new == "游戏辅助":
                    continue
                raw_dl = (it.get("download_url") or it.get("apk_url") or "").strip().replace("http://", "https://")
                raw_dl_low = raw_dl.lower()
                if any(raw_dl_low.endswith(ext) or (ext + "?") in raw_dl_low for ext in (".exe", ".dmg", ".msi")):
                    continue
                if "qqdownloader" in raw_dl_low:
                    raw_dl = ""

                ver = (it.get("version_name") or "").strip()
                md5 = (it.get("md_5") or "").strip()
                if raw_dl and (raw_dl_low.endswith(".apk") or ".apk?" in raw_dl_low):
                    dl_url = re.sub(r'sjy\.0000[89]/', 'sjy.00022/', raw_dl)
                elif len(md5) == 32 and ver:
                    dl_url = f"https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/{md5.upper()}.apk?fsname={pkg}_{ver}.apk"
                elif len(md5) == 32:
                    dl_url = f"https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/{md5.upper()}.apk?fsname={pkg}.apk"
                else:
                    dl_url = f"https://sj.qq.com/appdetail/{pkg}"

                sz = 0
                try:
                    sz = int(it.get("apk_size") or 0)
                except Exception:
                    sz = 0
                vcode = 0
                try:
                    vcode = int(it.get("version_code") or 0)
                except Exception:
                    vcode = 0
                rating = 0.0
                try:
                    rating = float(it.get("average_rating") or 0.0)
                except Exception:
                    rating = 0.0

                dev = (it.get("developer") or it.get("operator") or "").strip() or "开发者未提供"
                icon_u = (it.get("icon") or "").strip().replace("http://", "https://")
                if icon_u.endswith(".svg") or "yyb-icon" in icon_u:
                    icon_u = ""
                intro = (it.get("editor_intro") or it.get("show_text") or "").strip()
                desc = (it.get("description") or intro or "").strip()
                cat_name, sub_tag, is_feat = map_cat_sub(cate_new, hcat, hsub)

                out.append({
                    "source": "tencent",
                    "package": pkg,
                    "name": name,
                    "developer": dev,
                    "category": cat_name,
                    "category_label": sub_tag,
                    "tagline": intro,
                    "description": desc,
                    "icon_url": icon_u,
                    "rating": rating if rating > 0.0 else None,
                    "featured": is_feat,
                    "version_name": ver or "最新版",
                    "version_code": vcode,
                    "size_bytes": sz,
                    "download_url": dl_url,
                    "md5": md5,
                })
    except Exception:
        pass
    return out

def run_aptoide_job(job):
    url, hcat, hsub, is_feat = job
    out = []
    try:
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=min(timeout, 6.5)) as r:
            raw = r.read().decode("utf-8", errors="ignore")
        data = json.loads(raw)
        apps = (data.get("datalist") or {}).get("list") or []
        for it in apps:
            if not isinstance(it, dict):
                continue
            pkg = (it.get("package") or "").strip()
            name = (it.get("name") or "").strip()
            if not pkg or not name:
                continue
            file_obj = it.get("file") or {}
            ver = (file_obj.get("vername") or "").strip()
            vcode = 0
            try:
                vcode = int(file_obj.get("vercode") or 0)
            except Exception:
                vcode = 0
            sz = 0
            try:
                sz = int(file_obj.get("filesize") or it.get("size") or 0)
            except Exception:
                sz = 0
            md5 = (file_obj.get("md5sum") or "").strip()
            app_id = it.get("id")
            store_obj = it.get("store") or {}
            store_name = (store_obj.get("name") or "apps").strip()
            dev_obj = it.get("developer") or {}
            dev = (dev_obj.get("name") or store_name or "International Developer").strip()
            icon_u = (it.get("icon") or "").strip().replace("http://", "https://")
            dl_url = (file_obj.get("path") or "").strip().replace("http://", "https://")
            if not dl_url and store_name and vcode and app_id and md5:
                slug = re.sub(r'[^a-z0-9]+', '-', pkg.lower()).strip('-')
                dl_url = f"https://pool.apk.aptoide.com/{store_name}/{slug}-{vcode}-{app_id}-{md5}.apk"
            if not dl_url:
                continue
            rating = 0.0
            try:
                rating = float(((it.get("stats") or {}).get("rating") or {}).get("avg") or 0.0)
            except Exception:
                rating = 0.0
            out.append({
                "source": "global",
                "package": pkg,
                "name": name,
                "developer": dev,
                "category": hcat,
                "category_label": hsub,
                "tagline": "",
                "description": "",
                "icon_url": icon_u,
                "rating": rating if rating > 0.0 else None,
                "featured": is_feat,
                "version_name": ver or "最新版",
                "version_code": vcode,
                "size_bytes": sz,
                "download_url": dl_url,
                "md5": md5,
            })
    except Exception:
        pass
    return out

def run_fdroid_job(job):
    q, hcat, hsub = job
    out = []
    try:
        url = "https://search.f-droid.org/api/search_apps?q=" + urllib.parse.quote(q)
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=min(timeout, 6.5)) as r:
            raw = r.read().decode("utf-8", errors="ignore")
        data = json.loads(raw)
        for app in (data.get("apps") or [])[:10]:
            if not isinstance(app, dict):
                continue
            u = (app.get("url") or "").strip()
            pkg_m = re.search(r'/packages/([A-Za-z0-9_.]+)', u)
            if not pkg_m:
                continue
            pkg = pkg_m.group(1)
            name = (app.get("name") or pkg).strip()
            summary = (app.get("summary") or "").strip()
            icon_u = (app.get("icon") or "").strip().replace("http://", "https://")
            out.append({
                "source": "fdroid",
                "package": pkg,
                "name": name,
                "developer": "F-Droid Open Source",
                "category": hcat,
                "category_label": hsub,
                "tagline": summary,
                "description": summary,
                "icon_url": icon_u,
                "rating": None,
                "featured": False,
                "version_name": "最新版",
                "version_code": 0,
                "size_bytes": 0,
                "download_url": u,
                "md5": "",
            })
    except Exception:
        pass
    return out

tc_jobs = [("https://sj.qq.com/app/" + cid, c, s) for (cid, c, s) in CATEGORY_JOBS] + [
    ("https://sj.qq.com/search?q=" + urllib.parse.quote(q), c, s) for (q, c, s) in SEARCH_JOBS
]

by_pkg_src = {}

def merge_batch(batch):
    for item in batch:
        key = (item["package"], item.get("source", "tencent"))
        if key not in by_pkg_src:
            by_pkg_src[key] = item
        else:
            cur = by_pkg_src[key]
            cur["featured"] = cur["featured"] or item["featured"]
            if cur["category_label"] in ("", "系统工具", "协同办公", "生活服务") and item["category_label"] not in ("", "系统工具", "协同办公", "生活服务"):
                cur["category"] = item["category"]
                cur["category_label"] = item["category_label"]
            if not cur["icon_url"] and item["icon_url"]:
                cur["icon_url"] = item["icon_url"]
            if not cur["md5"] and item["md5"]:
                cur["md5"] = item["md5"]
                cur["download_url"] = item["download_url"]

with ThreadPoolExecutor(max_workers=20) as ex:
    futs = (
        [ex.submit(run_tencent_job, j) for j in tc_jobs]
        + [ex.submit(run_aptoide_job, j) for j in APTOIDE_JOBS]
        + [ex.submit(run_fdroid_job, j) for j in FDROID_QUERIES]
    )
    for fut in futs:
        try:
            merge_batch(fut.result(timeout=7.5))
        except Exception:
            pass

print(json.dumps(list(by_pkg_src.values()), ensure_ascii=False))
"#;

pub fn fetch_online_catalog_from_upstream() -> Result<(Vec<StoreApp>, String, usize)> {
    let val = run_python_json(PY_FETCH_ONLINE_CATALOG, &[], 35)?;
    let arr = val
        .as_array()
        .ok_or_else(|| anyhow!("上游应用市场返回格式无效"))?;
    let mut fetched = Vec::with_capacity(arr.len());
    for item in arr {
        let pkg = item
            .get("package")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if is_blacklisted_store_item(pkg, name) {
            continue;
        }
        let source = match item
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("tencent")
            .to_lowercase()
            .as_str()
        {
            "global" => StoreSource::Global,
            "fdroid" => StoreSource::FDroid,
            "github" => StoreSource::GitHub,
            "official" => StoreSource::Official,
            "stepos" => StoreSource::StepOs,
            _ => StoreSource::Tencent,
        };
        let desc = item
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let raw_cat = item
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let raw_cat_label = item
            .get("category_label")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let (inferred_cat, inferred_sub) = infer_category(name, pkg, &desc);
        let (category, category_label) = if matches!(source, StoreSource::Global | StoreSource::FDroid)
            || raw_cat.is_empty()
            || raw_cat == "Auto"
        {
            if inferred_sub != "系统工具" || raw_cat_label.is_empty() {
                (inferred_cat, inferred_sub)
            } else {
                let hint_cat = match raw_cat {
                    "Social" => StoreCategory::Social,
                    "Media" => StoreCategory::Media,
                    "Productivity" => StoreCategory::Productivity,
                    "Lifestyle" => StoreCategory::Lifestyle,
                    _ => inferred_cat,
                };
                (hint_cat, raw_cat_label.to_string())
            }
        } else {
            let explicit_cat = match raw_cat {
                "Social" => StoreCategory::Social,
                "Media" => StoreCategory::Media,
                "Productivity" => StoreCategory::Productivity,
                "Lifestyle" => StoreCategory::Lifestyle,
                _ => StoreCategory::Tools,
            };
            let explicit_label = if !raw_cat_label.is_empty() {
                raw_cat_label.to_string()
            } else {
                explicit_cat.label().to_string()
            };
            if explicit_label == "系统工具" && inferred_sub != "系统工具" {
                (inferred_cat, inferred_sub)
            } else {
                (explicit_cat, explicit_label)
            }
        };
        let default_dev = match source {
            StoreSource::Global => "International Developer",
            StoreSource::FDroid => "F-Droid Open Source",
            _ => "开发者未提供",
        };
        let dev = item
            .get("developer")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(default_dev)
            .trim()
            .to_string();
        let raw_tagline = item
            .get("tagline")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let tagline = if !raw_tagline.is_empty() {
            if raw_tagline.chars().count() > 44 {
                let s: String = raw_tagline.chars().take(44).collect();
                format!("{s}…")
            } else {
                raw_tagline.to_string()
            }
        } else if !desc.is_empty() {
            if desc.chars().count() > 44 {
                let s: String = desc.chars().take(44).collect();
                format!("{s}…")
            } else {
                desc.clone()
            }
        } else {
            format!("{dev} · {category_label}")
        };
        let icon_url = item
            .get("icon_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let rating = item
            .get("rating")
            .and_then(|v| v.as_f64())
            .map(|r| r as f32)
            .filter(|&r| r > 0.0 && r <= 5.0);
        let featured = item
            .get("featured")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let ver = item
            .get("version_name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("最新版")
            .trim()
            .to_string();
        let vcode = item.get("version_code").and_then(|v| v.as_u64()).unwrap_or(0);
        let size_bytes = item.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
        let dl = item
            .get("download_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let md5 = item
            .get("md5")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if dl.is_empty() {
            continue;
        }
        fetched.push(StoreApp {
            package: pkg.to_string(),
            name: name.to_string(),
            developer: dev,
            category,
            category_label,
            tagline,
            description: desc,
            changelog: String::new(),
            icon_url,
            cached_icon: existing_cached_icon(pkg),
            screenshots: Vec::new(),
            cached_screenshots: Vec::new(),
            rating,
            download_count_label: String::new(),
            icp_number: String::new(),
            featured,
            allow_system_update: false,
            sources: vec![StoreSourceOffer {
                source,
                version_name: ver,
                version_code: vcode,
                size_bytes,
                download_url: dl,
                updated_at: String::new(),
                md5,
                changelog: String::new(),
            }],
            selected_source_idx: 0,
            historical_versions: Vec::new(),
            developer_apps: Vec::new(),
            detail_loaded: false,
        });
    }
    if fetched.is_empty() {
        return Err(anyhow!("未从上游应用市场获取到在线应用目录"));
    }
    Ok(save_online_catalog_apps(&fetched))
}

fn memory_search_cache() -> &'static Mutex<HashMap<String, SearchCacheEntry>> {
    MEMORY_SEARCH_CACHE.get_or_init(|| {
        let path = search_cache_file();
        let map = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<HashMap<String, SearchCacheEntry>>(&s).ok())
            .unwrap_or_default();
        Mutex::new(map)
    })
}

fn memory_detail_cache() -> &'static Mutex<HashMap<String, DetailCacheEntry>> {
    MEMORY_DETAIL_CACHE.get_or_init(|| {
        let path = detail_cache_file();
        let map = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<HashMap<String, DetailCacheEntry>>(&s).ok())
            .unwrap_or_default();
        Mutex::new(map)
    })
}

fn save_search_cache(query: &str, apps: &[StoreApp]) {
    let key = query.trim().to_lowercase();
    if key.is_empty() {
        return;
    }
    if let Ok(mut guard) = memory_search_cache().lock() {
        guard.insert(
            key,
            SearchCacheEntry {
                updated_unix: now_unix_secs(),
                apps: apps.to_vec(),
            },
        );
        if guard.len() > 64 {
            let mut pairs: Vec<(String, u64)> =
                guard.iter().map(|(k, v)| (k.clone(), v.updated_unix)).collect();
            pairs.sort_by(|a, b| b.1.cmp(&a.1));
            let keep: std::collections::HashSet<String> =
                pairs.into_iter().take(64).map(|(k, _)| k).collect();
            guard.retain(|k, _| keep.contains(k));
        }
        let path = search_cache_file();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json_str) = serde_json::to_string(&*guard) {
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, json_str).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
    }
}

fn save_detail_cache(package: &str, detail: &StoreAppDetailPayload) {
    let key = package.trim().to_string();
    if key.is_empty() {
        return;
    }
    if let Ok(mut guard) = memory_detail_cache().lock() {
        guard.insert(
            key,
            DetailCacheEntry {
                updated_unix: now_unix_secs(),
                detail: detail.clone(),
            },
        );
        if guard.len() > 128 {
            let mut pairs: Vec<(String, u64)> =
                guard.iter().map(|(k, v)| (k.clone(), v.updated_unix)).collect();
            pairs.sort_by(|a, b| b.1.cmp(&a.1));
            let keep: std::collections::HashSet<String> =
                pairs.into_iter().take(128).map(|(k, _)| k).collect();
            guard.retain(|k, _| keep.contains(k));
        }
        let path = detail_cache_file();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json_str) = serde_json::to_string(&*guard) {
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, json_str).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
    }
}

fn curated_alias_matches(query: &str) -> Vec<StoreApp> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let short_q = q.chars().count() < 2;
    curated_catalog()
        .into_iter()
        .filter(|app| {
            package_aliases(&app.package).iter().any(|alias| {
                let alias_low = alias.to_lowercase();
                if short_q {
                    alias_low == q
                } else {
                    alias_low == q || alias_low.contains(&q) || q.contains(&alias_low)
                }
            }) || app.matches_query(&q)
        })
        .collect()
}

pub fn search_instant_local(query: &str) -> (Vec<StoreApp>, bool) {
    let q = query.trim();
    let full_local = load_startup_catalog().0;
    if q.is_empty() {
        return (full_local, true);
    }
    let key = q.to_lowercase();
    let mut results = curated_alias_matches(q);
    results.extend(full_local.into_iter().filter(|app| app.matches_query(q)));

    if let Ok(guard) = memory_search_cache().lock() {
        if let Some(entry) = guard.get(&key) {
            let is_fresh = now_unix_secs().saturating_sub(entry.updated_unix) < 6 * 3600
                && entry.apps.len() >= 24;
            results.extend(entry.apps.clone());
            let mut merged = merge_and_deduplicate_apps(results);
            for app in &mut merged {
                if app.cached_icon.is_none() {
                    app.cached_icon = existing_cached_icon(&app.package);
                }
            }
            merged.sort_by(|a, b| {
                score_app_relevance(b, q)
                    .cmp(&score_app_relevance(a, q))
                    .then_with(|| b.featured.cmp(&a.featured))
            });
            return (merged, is_fresh);
        }
    }

    let mut merged = merge_and_deduplicate_apps(results);
    for app in &mut merged {
        if app.cached_icon.is_none() {
            app.cached_icon = existing_cached_icon(&app.package);
        }
    }
    merged.sort_by(|a, b| {
        score_app_relevance(b, q)
            .cmp(&score_app_relevance(a, q))
            .then_with(|| b.featured.cmp(&a.featured))
    });
    (merged, false)
}

pub fn get_cached_app_detail(package: &str) -> Option<(StoreAppDetailPayload, bool)> {
    let key = package.trim();
    if key.is_empty() {
        return None;
    }
    let guard = memory_detail_cache().lock().ok()?;
    let entry = guard.get(key)?;
    let is_fresh = now_unix_secs().saturating_sub(entry.updated_unix) < 12 * 3600;
    let mut detail = entry.detail.clone();
    if detail.cached_icon.is_none() {
        detail.cached_icon = existing_cached_icon(key);
    }
    let cached_shots = existing_cached_screenshots(key, detail.screenshots.len());
    if !cached_shots.is_empty() {
        detail.cached_screenshots = cached_shots;
    }
    Some((detail, is_fresh))
}

const PY_MULTI_SEARCH: &str = r#"
import sys, json, re, urllib.request, urllib.parse
from concurrent.futures import ThreadPoolExecutor

timeout = float(sys.argv[1])
query = sys.argv[2]
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"

def get_text(url, t=6.0):
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8"})
    with urllib.request.urlopen(req, timeout=t) as r:
        return r.read().decode("utf-8", errors="ignore")

def search_tencent(q):
    out = []
    bad_cards = {
        "yybn_searchresult_hotgame",
        "yybn_searchresult_reservegame",
        "YYB_HOME_SEARCH_CLOUD_GAME",
        "YYB_HOME_SEARCH_WECHAT_GAME",
        "YYB_HOME_APP_LIBRARY_CATEGORY",
    }
    try:
        url = "https://sj.qq.com/search?q=" + urllib.parse.quote(q)
        html = get_text(url, min(timeout, 6.0))
        m = re.search(r'<script[^>]*id="__NEXT_DATA__"[^>]*>(.*?)</script>', html, re.S)
        if m:
            data = json.loads(m.group(1))
            comps = (
                data.get("props", {})
                .get("pageProps", {})
                .get("dynamicCardResponse", {})
                .get("data", {})
                .get("components", [])
            )
            for comp in comps:
                cid = (comp.get("cardId") or comp.get("componentId") or "").strip()
                if cid in bad_cards:
                    continue
                item_list = (comp.get("data") or {}).get("itemData") or []
                for it in item_list:
                    if not isinstance(it, dict):
                        continue
                    pkg = (it.get("pkg_name") or "").strip()
                    name = (it.get("name") or "").strip()
                    if (
                        not pkg
                        or not name
                        or pkg.startswith("com.tencent.pcgame.")
                        or pkg.startswith("com.tencent.tmgp.")
                        or pkg == "com.tencent.android.qqdownloader"
                        or name in ("安装快捷方式", "应用宝", "腾讯应用宝")
                    ):
                        continue
                    if (
                        str(it.get("game_type") or "") == "7"
                        or str(it.get("is_game") or "") == "1"
                        or str(it.get("is_booking") or "") == "1"
                    ):
                        continue
                    size_b = int(it.get("apk_size") or 0)
                    raw_dl = (it.get("download_url") or it.get("apk_url") or "").strip().replace("http://", "https://")
                    if any(raw_dl.lower().endswith(ext) or (ext + "?") in raw_dl.lower() for ext in (".exe", ".dmg", ".msi")):
                        continue
                    ver = (it.get("version_name") or "").strip()
                    md5 = (it.get("md_5") or "").strip()
                    if raw_dl and (raw_dl.lower().endswith(".apk") or ".apk?" in raw_dl.lower()) and "qqdownloader" not in raw_dl.lower():
                        dl = re.sub(r'sjy\.0000[89]/', 'sjy.00022/', raw_dl)
                    elif len(md5) == 32 and ver:
                        dl = f"https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/{md5.upper()}.apk?fsname={pkg}_{ver}.apk"
                    elif size_b > 0:
                        dl = "https://sj.qq.com/appdetail/" + pkg
                    else:
                        continue
                    dev = (it.get("developer") or "").strip()
                    icon = (it.get("icon") or "").strip().replace("http://", "https://")
                    desc = (it.get("description") or it.get("editor_intro") or "").strip()
                    out.append({
                        "source": "Tencent",
                        "package": pkg,
                        "name": name,
                        "version_name": ver or "最新版",
                        "version_code": int(it.get("version_code") or 0),
                        "size_bytes": size_b,
                        "developer": dev or "开发者未提供",
                        "icon_url": icon,
                        "download_url": dl,
                        "md5": md5,
                        "description": desc,
                        "category_label": (it.get("cate_name_new") or "").strip() or "应用宝",
                    })
    except Exception:
        pass
    return out

def search_aptoide(q):
    out = []
    try:
        url = "https://ws75.aptoide.com/api/7/apps/search?query=" + urllib.parse.quote(q) + "&limit=40&aab=1"
        raw = get_text(url, min(timeout, 6.0))
        data = json.loads(raw)
        apps = (data.get("datalist") or {}).get("list") or []
        for app in apps[:40]:
            if not isinstance(app, dict):
                continue
            pkg = (app.get("package") or "").strip()
            name = (app.get("name") or "").strip()
            file_obj = app.get("file") or {}
            ver = (file_obj.get("vername") or "").strip()
            vcode = int(file_obj.get("vercode") or 0)
            size_b = int(file_obj.get("filesize") or app.get("size") or 0)
            dev_obj = app.get("developer") or {}
            store_obj = app.get("store") or {}
            store_name = (store_obj.get("name") or "apps").strip()
            dev = (dev_obj.get("name") or store_name or "International Developer").strip()
            icon = (app.get("icon") or "").strip().replace("http://", "https://")
            dl = (file_obj.get("path") or "").strip().replace("http://", "https://")
            md5 = (file_obj.get("md5sum") or "").strip()
            app_id = app.get("id")
            if not dl and store_name and vcode and app_id and md5:
                slug = re.sub(r'[^a-z0-9]+', '-', pkg.lower()).strip('-')
                dl = f"https://pool.apk.aptoide.com/{store_name}/{slug}-{vcode}-{app_id}-{md5}.apk"
            if pkg and name and dl:
                out.append({
                    "source": "Global",
                    "package": pkg,
                    "name": name,
                    "version_name": ver or "最新版",
                    "version_code": vcode,
                    "size_bytes": size_b,
                    "developer": dev,
                    "icon_url": icon,
                    "download_url": dl,
                    "md5": md5,
                    "description": "",
                    "category_label": "国际应用",
                })
    except Exception:
        pass
    return out

def search_fdroid(q):
    out = []
    try:
        f_url = "https://search.f-droid.org/api/search_apps?q=" + urllib.parse.quote(q)
        raw = get_text(f_url, min(timeout, 6.0))
        fdata = json.loads(raw)
        for app in (fdata.get("apps") or [])[:20]:
            pkg_m = re.search(r'/packages/([A-Za-z0-9_.]+)', app.get("url", ""))
            if pkg_m:
                pkg = pkg_m.group(1)
                out.append({
                    "source": "FDroid",
                    "package": pkg,
                    "name": app.get("name", pkg),
                    "version_name": "",
                    "version_code": 0,
                    "size_bytes": 0,
                    "developer": "F-Droid Open Source",
                    "icon_url": (app.get("icon") or "").replace("http://", "https://"),
                    "download_url": app.get("url", ""),
                    "md5": "",
                    "description": app.get("summary", ""),
                    "category_label": "开源应用",
                })
    except Exception:
        pass
    return out

raw_q = sys.argv[3].strip() if len(sys.argv) > 3 else ""
primary_q = query.split()[0] if " " in query else query
queries_ap = [primary_q]
if raw_q and raw_q.lower() != primary_q.lower():
    queries_ap.append(raw_q)

items = []
with ThreadPoolExecutor(max_workers=5) as ex:
    futs = [
        ex.submit(search_tencent, query),
        ex.submit(search_fdroid, primary_q),
    ]
    if raw_q and raw_q.lower() != query.lower():
        futs.append(ex.submit(search_tencent, raw_q))
    for aq in queries_ap:
        futs.append(ex.submit(search_aptoide, aq))
    for fut in futs:
        try:
            items.extend(fut.result(timeout=6.5))
        except Exception:
            pass

print(json.dumps(items, ensure_ascii=False))
"#;

pub fn search_multi_source(query: &str) -> Result<Vec<StoreApp>> {
    let q = query.trim();
    let full_local = load_startup_catalog().0;
    if q.is_empty() {
        return Ok(full_local);
    }
    let alias_matches = curated_alias_matches(q);
    let alias_pkgs: std::collections::HashSet<String> =
        alias_matches.iter().map(|a| a.package.clone()).collect();
    let mut results = alias_matches;
    results.extend(full_local.into_iter().filter(|app| app.matches_query(q)));

    let upstream_q = expand_upstream_query(q);
    if let Ok(val) = run_python_json(PY_MULTI_SEARCH, &[&upstream_q, q], 10) {
        if let Some(arr) = val.as_array() {
            for item in arr {
                let pkg = item
                    .get("package")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if pkg.is_empty() || name.is_empty() || is_blacklisted_store_item(pkg, name) {
                    continue;
                }
                let src = match item.get("source").and_then(|v| v.as_str()).unwrap_or("") {
                    "Global" => StoreSource::Global,
                    "FDroid" => StoreSource::FDroid,
                    "GitHub" => StoreSource::GitHub,
                    "StepOs" => StoreSource::StepOs,
                    "Official" => StoreSource::Official,
                    _ => StoreSource::Tencent,
                };
                let ver = item
                    .get("version_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let vcode = item.get("version_code").and_then(|v| v.as_u64()).unwrap_or(0);
                let size_bytes = item.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
                let dev = item
                    .get("developer")
                    .and_then(|v| v.as_str())
                    .unwrap_or("开发者未提供")
                    .to_string();
                let icon_url = item
                    .get("icon_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let dl = item
                    .get("download_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let md5 = item
                    .get("md5")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let desc = item
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let raw_cat_label = item
                    .get("category_label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let (inferred_cat, inferred_sub) = infer_category(name, pkg, &desc);
                let (category, cat_label) = match src {
                    StoreSource::Global | StoreSource::FDroid | StoreSource::GitHub => {
                        (inferred_cat, inferred_sub)
                    }
                    _ => {
                        if !raw_cat_label.is_empty()
                            && raw_cat_label != "应用宝"
                            && inferred_sub == "系统工具"
                        {
                            let c = match raw_cat_label {
                                "社交" | "社区" | "聊天" => StoreCategory::Social,
                                "视频" | "音乐" | "阅读" | "摄影" | "娱乐" => StoreCategory::Media,
                                "办公" | "教育" | "效率" | "商务" => StoreCategory::Productivity,
                                "购物" | "美食" | "交通出行" | "旅游出行" | "理财" | "生活"
                                | "健康" | "汽车" | "母婴" => StoreCategory::Lifestyle,
                                _ => inferred_cat,
                            };
                            (c, raw_cat_label.to_string())
                        } else {
                            (inferred_cat, inferred_sub)
                        }
                    }
                };
                let tagline = if desc.chars().count() > 44 {
                    let s: String = desc.chars().take(44).collect();
                    format!("{s}…")
                } else if !desc.is_empty() {
                    desc.clone()
                } else {
                    format!("{dev} · {cat_label}")
                };

                results.push(StoreApp {
                    package: pkg.to_string(),
                    name: name.to_string(),
                    developer: dev,
                    category,
                    category_label: cat_label,
                    tagline,
                    description: desc,
                    changelog: String::new(),
                    icon_url,
                    cached_icon: existing_cached_icon(pkg),
                    screenshots: Vec::new(),
                    cached_screenshots: Vec::new(),
                    rating: None,
                    download_count_label: String::new(),
                    icp_number: String::new(),
                    featured: false,
                    allow_system_update: false,
                    sources: vec![StoreSourceOffer {
                        source: src,
                        version_name: ver,
                        version_code: vcode,
                        size_bytes,
                        download_url: dl,
                        updated_at: String::new(),
                        md5,
                        changelog: String::new(),
                    }],
                    selected_source_idx: 0,
                    historical_versions: Vec::new(),
                    developer_apps: Vec::new(),
                    detail_loaded: false,
                });
            }
        }
    }

    let curated_name_to_pkg: HashMap<String, String> = curated_catalog()
        .into_iter()
        .map(|a| (a.name.to_lowercase(), a.package))
        .collect();
    let mut merged = merge_and_deduplicate_apps(results);
    merged.retain(|app| {
        if alias_pkgs.contains(&app.package) {
            return true;
        }
        if let Some(canon_pkg) = curated_name_to_pkg.get(&app.name.to_lowercase()) {
            if canon_pkg != &app.package {
                return false;
            }
        }
        score_app_relevance(app, q) > 0 || score_app_relevance(app, &upstream_q) > 0
    });
    merged.sort_by(|a, b| {
        score_app_relevance(b, q)
            .max(score_app_relevance(b, &upstream_q))
            .cmp(&score_app_relevance(a, q).max(score_app_relevance(a, &upstream_q)))
            .then_with(|| b.featured.cmp(&a.featured))
    });
    save_search_cache(q, &merged);
    Ok(merged)
}

pub fn expand_category_catalog_batch(
    seeds: &[String],
    target_cat: Option<StoreCategory>,
    target_sub: Option<String>,
) -> Result<(Vec<StoreApp>, String, usize)> {
    let curated_pkgs: std::collections::HashSet<String> =
        curated_catalog().into_iter().map(|a| a.package).collect();
    let mut results = Vec::new();
    let joined_seeds = seeds.join(" ");
    for seed in seeds {
        let s = seed.trim();
        if s.is_empty() {
            continue;
        }
        let upstream_q = expand_upstream_query(s);
        if let Ok(val) = run_python_json(PY_MULTI_SEARCH, &[&upstream_q, s], 10) {
            if let Some(arr) = val.as_array() {
                for item in arr {
                    let pkg = item
                        .get("package")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    if pkg.is_empty() || name.is_empty() || is_blacklisted_store_item(pkg, name) {
                        continue;
                    }
                    let src = match item.get("source").and_then(|v| v.as_str()).unwrap_or("") {
                        "Global" => StoreSource::Global,
                        "FDroid" => StoreSource::FDroid,
                        "GitHub" => StoreSource::GitHub,
                        "StepOs" => StoreSource::StepOs,
                        "Official" => StoreSource::Official,
                        _ => StoreSource::Tencent,
                    };
                    let ver = item
                        .get("version_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let vcode = item.get("version_code").and_then(|v| v.as_u64()).unwrap_or(0);
                    let size_bytes = item.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
                    let dev = item
                        .get("developer")
                        .and_then(|v| v.as_str())
                        .unwrap_or("开发者未提供")
                        .to_string();
                    let icon_url = item
                        .get("icon_url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let dl = item
                        .get("download_url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let md5 = item
                        .get("md5")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let desc = item
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let (inferred_cat, inferred_sub) = infer_category(name, pkg, &desc);
                    let category = if !curated_pkgs.contains(pkg) {
                        target_cat
                            .filter(|c| {
                                !matches!(
                                    c,
                                    StoreCategory::All
                                        | StoreCategory::Updatable
                                        | StoreCategory::Installed
                                )
                            })
                            .unwrap_or(inferred_cat)
                    } else {
                        inferred_cat
                    };
                    let cat_label = target_sub
                        .as_deref()
                        .filter(|sub| !sub.trim().is_empty() && *sub != "全部")
                        .map(|sub| sub.to_string())
                        .unwrap_or(inferred_sub);
                    let tagline = if desc.chars().count() > 44 {
                        let s_cut: String = desc.chars().take(44).collect();
                        format!("{s_cut}…")
                    } else if !desc.is_empty() {
                        desc.clone()
                    } else {
                        format!("{dev} · {cat_label}")
                    };
                    results.push(StoreApp {
                        package: pkg.to_string(),
                        name: name.to_string(),
                        developer: dev,
                        category,
                        category_label: cat_label,
                        tagline,
                        description: desc,
                        changelog: String::new(),
                        icon_url,
                        cached_icon: existing_cached_icon(pkg),
                        screenshots: Vec::new(),
                        cached_screenshots: Vec::new(),
                        rating: None,
                        download_count_label: String::new(),
                        icp_number: String::new(),
                        featured: false,
                        allow_system_update: false,
                        sources: vec![StoreSourceOffer {
                            source: src,
                            version_name: ver,
                            version_code: vcode,
                            size_bytes,
                            download_url: dl,
                            updated_at: String::new(),
                            md5,
                            changelog: String::new(),
                        }],
                        selected_source_idx: 0,
                        historical_versions: Vec::new(),
                        developer_apps: Vec::new(),
                        detail_loaded: false,
                    });
                }
            }
        }
    }
    let _ = joined_seeds;
    Ok(save_online_catalog_apps(&results))
}

const PY_FETCH_DETAIL: &str = r#"
import sys, json, re, urllib.request, urllib.parse
from concurrent.futures import ThreadPoolExecutor

timeout = float(sys.argv[1])
pkg = sys.argv[2]
dev_hint = sys.argv[3] if len(sys.argv) > 3 else ""
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"

def get_text(url, t=None):
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8"})
    with urllib.request.urlopen(req, timeout=(t or min(timeout, 4.5))) as r:
        return r.read().decode("utf-8", errors="ignore")

out = {
    "package": pkg,
    "developer": "",
    "description": "",
    "changelog": "",
    "icp_number": "",
    "icon_url": "",
    "screenshots": [],
    "extra_sources": [],
    "historical_versions": [],
    "developer_apps": [],
}

def find_sjqq_item(html, target_pkg):
    if not target_pkg or target_pkg == "com.tencent.android.qqdownloader":
        return None
    m_next = re.search(r'<script[^>]*id="__NEXT_DATA__"[^>]*>(.*?)</script>', html, re.S)
    if not m_next:
        return None
    try:
        data = json.loads(m_next.group(1))
        comps = (
            data.get("props", {})
            .get("pageProps", {})
            .get("dynamicCardResponse", {})
            .get("data", {})
            .get("components", [])
        )
        for comp in comps:
            for it in ((comp.get("data") or {}).get("itemData") or []):
                if isinstance(it, dict) and (it.get("pkg_name") or "").strip() == target_pkg:
                    nm = (it.get("name") or "").strip()
                    if nm and nm not in ("安装快捷方式", "应用宝"):
                        return it
    except Exception:
        pass
    return None

def task_tencent():
    res = {}
    try:
        dl_url = ""
        try:
            s_html = get_text("https://a.app.qq.com/o/simple.jsp?pkgname=" + urllib.parse.quote(pkg), 3.5)
            m_dl = re.search(r'https?://imtt\d*\.dd\.qq\.com/[^\s"\'<>]+\.apk[^\s"\'<>]*', s_html)
            if m_dl and "qqdownloader" not in m_dl.group(0):
                dl_url = re.sub(r'sjy\.0000[89]/', 'sjy.00022/', m_dl.group(0).replace("http://", "https://"))
        except Exception:
            pass

        d_html = get_text("https://sj.qq.com/appdetail/" + urllib.parse.quote(pkg), 4.0)
        it = find_sjqq_item(d_html, pkg)
        if it:
            raw_shots = (it.get("images") or it.get("snap_shots") or "")
            shot_list = raw_shots if isinstance(raw_shots, list) else str(raw_shots).split(",")
            seen_s = []
            for s in shot_list:
                s_https = str(s).strip().replace("http://", "https://")
                if s_https.startswith("https://") and "shot_5848_" not in s_https and s_https not in seen_s:
                    seen_s.append(s_https)
            res["screenshots"] = seen_s[:6]

            icon_u = (it.get("icon") or "").strip().replace("http://", "https://")
            if icon_u and not icon_u.endswith(".svg") and "yyb-icon" not in icon_u:
                res["icon_url"] = icon_u

            desc = (it.get("description") or it.get("editor_intro") or "").strip()
            if desc:
                res["description"] = desc.replace("\\n", "\n")
            chg = (it.get("new_feature") or "").strip()
            if chg:
                res["changelog"] = chg.replace("\\n", "\n")
            dev = (it.get("developer") or "").strip()
            if dev and dev != "深圳市腾讯计算机系统有限公司" or pkg.startswith("com.tencent."):
                res["developer"] = dev
            ver_name = (it.get("version_name") or "").strip()
            sz_b = int(it.get("apk_size") or 0)
            md5 = (it.get("md_5") or "").strip()
            if not dl_url:
                cand_dl = (it.get("download_url") or "").strip().replace("http://", "https://")
                if cand_dl and "qqdownloader" not in cand_dl:
                    dl_url = re.sub(r'sjy\.0000[89]/', 'sjy.00022/', cand_dl)
                elif md5 and ver_name:
                    dl_url = f"https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/{md5.upper()}.apk?fsname={pkg}_{ver_name}.apk"
            if dl_url and ver_name and ver_name != "9.2.9":
                res["offer"] = {
                    "source": "Tencent",
                    "version_name": ver_name,
                    "version_code": int(it.get("version_code") or 0),
                    "size_bytes": sz_b,
                    "download_url": dl_url,
                    "updated_at": "",
                    "md5": md5,
                    "changelog": "",
                }
    except Exception:
        pass
    return res

def task_aptoide_meta():
    res = {}
    try:
        raw = get_text("https://ws75.aptoide.com/api/7/app/getMeta?package_name=" + urllib.parse.quote(pkg) + "&aab=1", 4.5)
        data = (json.loads(raw).get("data") or {})
        if data:
            media = data.get("media") or {}
            desc = (media.get("description") or "").strip()
            news = (media.get("news") or "").strip()
            sshots = []
            for s in (media.get("sshots") or []):
                if isinstance(s, dict) and s.get("url"):
                    u = s["url"].replace("http://", "https://")
                    if u not in sshots:
                        sshots.append(u)
            dev = ((data.get("developer") or {}).get("name") or "").strip()
            icon = (data.get("icon") or "").strip().replace("http://", "https://")
            file_obj = data.get("file") or {}
            dl_path = (file_obj.get("path") or "").strip().replace("http://", "https://")
            ver_name = (file_obj.get("vername") or "").strip()
            ver_code = int(file_obj.get("vercode") or 0)
            fsize = int(file_obj.get("filesize") or data.get("size") or 0)
            md5 = (file_obj.get("md5sum") or "").strip()
            app_id = data.get("id")
            store_name = ((data.get("store") or {}).get("name") or "apps").strip()
            if not dl_path and store_name and ver_code and app_id and md5:
                slug = re.sub(r'[^a-z0-9]+', '-', pkg.lower()).strip('-')
                dl_path = f"https://pool.apk.aptoide.com/{store_name}/{slug}-{ver_code}-{app_id}-{md5}.apk"
            updated = (data.get("updated") or "")[:10]
            res["description"] = desc
            res["changelog"] = news
            res["developer"] = dev
            res["icon_url"] = icon
            res["screenshots"] = sshots[:6]
            if dl_path:
                res["offer"] = {
                    "source": "Global",
                    "version_name": ver_name or "最新版",
                    "version_code": ver_code,
                    "size_bytes": fsize,
                    "download_url": dl_path,
                    "updated_at": updated,
                    "md5": md5,
                    "changelog": news,
                }
    except Exception:
        pass
    return res

def task_aptoide_versions():
    hists = []
    try:
        raw = get_text("https://ws75.aptoide.com/api/7/listAppsVersions?package_name=" + urllib.parse.quote(pkg) + "&limit=6&aab=1", 4.5)
        items = (json.loads(raw).get("list") or [])
        for it in items[:6]:
            if not isinstance(it, dict):
                continue
            store_name = ((it.get("store") or {}).get("name") or "").strip()
            file_obj = it.get("file") or {}
            vname = (file_obj.get("vername") or "").strip()
            vcode = int(file_obj.get("vercode") or 0)
            fsize = int(file_obj.get("filesize") or it.get("size") or 0)
            md5 = (file_obj.get("md5sum") or "").strip()
            app_id = it.get("id")
            dl_path = (file_obj.get("path") or "").strip().replace("http://", "https://")
            if not dl_path and store_name and vcode and app_id and md5:
                slug = re.sub(r'[^a-z0-9]+', '-', pkg.lower()).strip('-')
                dl_path = f"https://pool.apk.aptoide.com/{store_name}/{slug}-{vcode}-{app_id}-{md5}.apk"
            if vname and dl_path:
                sz_lbl = f"{fsize / 1048576.0:.1f} MB" if fsize > 0 else "历史归档版"
                rel_date = (it.get("updated") or it.get("added") or "")[:10]
                hists.append({
                    "version_name": vname,
                    "version_code": vcode,
                    "size_bytes": fsize,
                    "size_label": sz_lbl,
                    "release_date": rel_date,
                    "source": "Global",
                    "download_url": dl_path,
                })
    except Exception:
        pass
    return hists

with ThreadPoolExecutor(max_workers=3) as ex:
    f_tc = ex.submit(task_tencent)
    f_am = ex.submit(task_aptoide_meta)
    f_av = ex.submit(task_aptoide_versions)

    tc = {}
    am = {}
    av = []
    try: tc = f_tc.result(timeout=5.0)
    except Exception: pass
    try: am = f_am.result(timeout=5.0)
    except Exception: pass
    try: av = f_av.result(timeout=5.0)
    except Exception: pass

    out["description"] = tc.get("description") or am.get("description") or ""
    out["changelog"] = tc.get("changelog") or am.get("changelog") or ""
    out["developer"] = tc.get("developer") or am.get("developer") or ""
    out["icon_url"] = tc.get("icon_url") or am.get("icon_url") or ""
    out["screenshots"] = tc.get("screenshots") or am.get("screenshots") or []
    if tc.get("offer"):
        out["extra_sources"].append(tc["offer"])
    if am.get("offer"):
        out["extra_sources"].append(am["offer"])
    out["historical_versions"].extend(av)

print(json.dumps(out, ensure_ascii=False))
"#;

pub fn fetch_app_full_detail(app: &StoreApp) -> Result<StoreAppDetailPayload> {
    let mut payload = StoreAppDetailPayload {
        package: app.package.clone(),
        developer: app.developer.clone(),
        description: app.description.clone(),
        changelog: app.changelog.clone(),
        icp_number: app.icp_number.clone(),
        rating: app.rating,
        download_count_label: app.download_count_label.clone(),
        icon_url: app.icon_url.clone(),
        cached_icon: existing_cached_icon(&app.package).or_else(|| app.cached_icon.clone()),
        screenshots: app.screenshots.clone(),
        cached_screenshots: existing_cached_screenshots(&app.package, app.screenshots.len()),
        extra_sources: Vec::new(),
        historical_versions: app.historical_versions.clone(),
        developer_apps: app.developer_apps.clone(),
    };

    // Populate same-developer apps from curated catalog first
    let dev_prefix: String = app.developer.chars().take(4).collect();
    if dev_prefix.chars().count() >= 2 {
        for other in curated_catalog() {
            if other.package != app.package
                && (other.developer == app.developer || other.developer.contains(&dev_prefix))
                && !payload
                    .developer_apps
                    .iter()
                    .any(|d| d.package == other.package)
            {
                payload.developer_apps.push(RelatedAppItem {
                    package: other.package.clone(),
                    name: other.name.clone(),
                    version_name: other.version_name().to_string(),
                    size_bytes: other.size_bytes(),
                    icon_url: other.icon_url.clone(),
                    cached_icon: other.cached_icon.clone(),
                    developer: other.developer.clone(),
                    tagline: other.tagline.clone(),
                    source: other.primary_source(),
                    download_url: other.download_url().to_string(),
                });
            }
        }
    }

    if let Ok(val) = run_python_json(PY_FETCH_DETAIL, &[&app.package, &app.developer], 8) {
        if let Some(desc) = val.get("description").and_then(|v| v.as_str()) {
            let trimmed = desc.trim();
            if trimmed.chars().count() > 20
                || (!trimmed.is_empty() && payload.description.trim().is_empty())
            {
                payload.description = trimmed.to_string();
            }
        }
        if let Some(chg) = val.get("changelog").and_then(|v| v.as_str()) {
            if !chg.trim().is_empty() {
                payload.changelog = chg.trim().to_string();
            }
        }
        if let Some(dev) = val.get("developer").and_then(|v| v.as_str()) {
            if !dev.trim().is_empty() {
                payload.developer = dev.trim().to_string();
            }
        }
        if let Some(icon) = val.get("icon_url").and_then(|v| v.as_str()) {
            if !icon.trim().is_empty() {
                payload.icon_url = icon.trim().to_string();
            }
        }
        if let Some(shots) = val.get("screenshots").and_then(|v| v.as_array()) {
            let live_shots: Vec<String> = shots
                .iter()
                .filter_map(|s| {
                    s.as_str()
                        .map(str::trim)
                        .filter(|u| !u.is_empty())
                        .map(String::from)
                })
                .collect();
            if !live_shots.is_empty() {
                payload.screenshots = live_shots;
            }
        }
        if let Some(extras) = val.get("extra_sources").and_then(|v| v.as_array()) {
            for ex in extras {
                if let Ok(offer) = serde_json::from_value::<StoreSourceOffer>(ex.clone()) {
                    if !offer.download_url.trim().is_empty()
                        && !payload
                            .extra_sources
                            .iter()
                            .any(|s| s.source == offer.source && s.version_name == offer.version_name)
                    {
                        payload.extra_sources.push(offer);
                    }
                }
            }
        }
        if let Some(hists) = val.get("historical_versions").and_then(|v| v.as_array()) {
            for h in hists {
                if let Ok(hist) = serde_json::from_value::<HistoricalVersion>(h.clone()) {
                    if !hist.version_name.is_empty()
                        && !payload
                            .historical_versions
                            .iter()
                            .any(|x| x.version_name == hist.version_name)
                    {
                        payload.historical_versions.push(hist);
                    }
                }
            }
        }
    }

    payload.cached_screenshots =
        existing_cached_screenshots(&app.package, payload.screenshots.len());
    save_detail_cache(&app.package, &payload);
    Ok(payload)
}

const PY_BATCH_CHECK_UPDATES: &str = r#"
import sys, json, re, urllib.request, urllib.parse
from concurrent.futures import ThreadPoolExecutor

timeout = float(sys.argv[1])
pkgs = json.loads(sys.argv[2])
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"

def check_one(pkg):
    if not pkg or pkg == "com.tencent.android.qqdownloader":
        return None
    try:
        url = "https://sj.qq.com/appdetail/" + urllib.parse.quote(pkg)
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=min(timeout, 3.2)) as r:
            html = r.read().decode("utf-8", errors="ignore")
        m_next = re.search(r'<script[^>]*id="__NEXT_DATA__"[^>]*>(.*?)</script>', html, re.S)
        if m_next:
            data = json.loads(m_next.group(1))
            comps = (
                data.get("props", {})
                .get("pageProps", {})
                .get("dynamicCardResponse", {})
                .get("data", {})
                .get("components", [])
            )
            for comp in comps:
                for it in ((comp.get("data") or {}).get("itemData") or []):
                    if not isinstance(it, dict):
                        continue
                    if (it.get("pkg_name") or "").strip() != pkg:
                        continue
                    name = (it.get("name") or "").strip()
                    ver = (it.get("version_name") or "").strip()
                    if not name or name in ("安装快捷方式", "应用宝") or not ver:
                        continue
                    dev = (it.get("developer") or "").strip()
                    sz = int(it.get("apk_size") or 0)
                    icon_u = (it.get("icon") or "").strip().replace("http://", "https://")
                    if icon_u.endswith(".svg") or "yyb-icon" in icon_u:
                        icon_u = ""
                    md5 = (it.get("md_5") or "").strip()
                    dl_u = (it.get("download_url") or "").strip().replace("http://", "https://")
                    if not dl_u and md5 and ver:
                        dl_u = f"https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/{md5.upper()}.apk?fsname={pkg}_{ver}.apk"
                    if dl_u and "qqdownloader" not in dl_u:
                        return {
                            "source": "Tencent",
                            "package": pkg,
                            "name": name,
                            "version_name": ver,
                            "size_bytes": sz,
                            "developer": dev or "开发者未提供",
                            "icon_url": icon_u,
                            "download_url": dl_u,
                        }
    except Exception:
        pass
    return None

results = []
with ThreadPoolExecutor(max_workers=8) as ex:
    futs = [ex.submit(check_one, pkg) for pkg in pkgs[:18]]
    for fut in futs:
        try:
            r = fut.result(timeout=3.8)
            if r:
                results.append(r)
        except Exception:
            pass

print(json.dumps(results, ensure_ascii=False))
"#;

pub fn check_updates_for_installed(
    installed: &[AppInfo],
    existing_catalog: &[StoreApp],
) -> Vec<StoreApp> {
    let mut out: Vec<StoreApp> = existing_catalog
        .iter()
        .filter(|a| {
            a.name != "安装快捷方式"
                && a.tagline != "从应用宝实时源匹配到的本机已装应用"
                && a.package != "com.tencent.android.qqdownloader"
        })
        .cloned()
        .collect();
    let mut installed_in_catalog: Vec<String> = Vec::new();

    for app in installed {
        if !app.in_default_scope() {
            continue;
        }
        if let Some(cat_app) = out.iter().find(|c| c.package == app.package) {
            if is_eligible_for_store_update(app, cat_app) {
                installed_in_catalog.push(app.package.clone());
            }
        }
    }

    if !installed_in_catalog.is_empty() {
        if let Ok(pkgs_json) = serde_json::to_string(&installed_in_catalog) {
            if let Ok(val) = run_python_json(PY_BATCH_CHECK_UPDATES, &[&pkgs_json], 5) {
                if let Some(arr) = val.as_array() {
                    for item in arr {
                        let pkg = item
                            .get("package")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if pkg.is_empty() {
                            continue;
                        }
                        let name = item
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if name.is_empty() || name == "安装快捷方式" || name == "应用宝" {
                            continue;
                        }
                        let ver = item
                            .get("version_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let dl = item
                            .get("download_url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if ver.is_empty() || dl.is_empty() || dl.contains("qqdownloader") {
                            continue;
                        }
                        let size_bytes =
                            item.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
                        let icon_url = item
                            .get("icon_url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        if let Some(existing) = out.iter_mut().find(|c| c.package == pkg) {
                            if !icon_url.is_empty()
                                && (existing.icon_url.is_empty()
                                    || existing.icon_url.contains("sj.qq.com/appdetail/"))
                            {
                                existing.icon_url = icon_url;
                            }
                            existing.sources.push(StoreSourceOffer {
                                source: StoreSource::Tencent,
                                version_name: ver,
                                version_code: 0,
                                size_bytes,
                                download_url: dl,
                                updated_at: String::new(),
                                md5: String::new(),
                                changelog: String::new(),
                            });
                        }
                    }
                }
            }
        }
    }

    merge_and_deduplicate_apps(out)
}

const PY_DOWNLOAD_ASSET: &str = r#"
import sys, os, re, json, urllib.request, urllib.parse

timeout = float(sys.argv[1])
url = sys.argv[2]
dest = sys.argv[3]
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"

def is_raster(b):
    if len(b) < 128:
        return False
    s = b[:64].lstrip()
    if s.startswith((b"<svg", b"<!DOCTYPE", b"<!doctype", b"<html", b"<HTML", b"<?xml")):
        return False
    return b.startswith(b"\x89PNG") or b.startswith(b"\xff\xd8\xff") or b.startswith(b"GIF8") or (b.startswith(b"RIFF") and b[8:12] == b"WEBP")

def resolve_icon_url(u):
    if "sj.qq.com/appdetail/" in u:
        pkg = u.split("sj.qq.com/appdetail/")[-1].split("?")[0].strip("/")
        try:
            req = urllib.request.Request(u, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=min(timeout, 3.5)) as r:
                html = r.read().decode("utf-8", errors="ignore")
            m = re.search(r'<script[^>]*id="__NEXT_DATA__"[^>]*>(.*?)</script>', html, re.S)
            if m:
                data = json.loads(m.group(1))
                comps = data.get("props", {}).get("pageProps", {}).get("dynamicCardResponse", {}).get("data", {}).get("components", [])
                for comp in comps:
                    for it in ((comp.get("data") or {}).get("itemData") or []):
                        if isinstance(it, dict) and (it.get("pkg_name") or "").strip() == pkg:
                            ic = (it.get("icon") or "").strip().replace("http://", "https://")
                            if ic and not ic.endswith(".svg") and "yyb-icon" not in ic:
                                return ic
        except Exception:
            pass
        try:
            ap_url = "https://ws75.aptoide.com/api/7/app/getMeta?package_name=" + urllib.parse.quote(pkg) + "&aab=1"
            req2 = urllib.request.Request(ap_url, headers={"User-Agent": UA})
            with urllib.request.urlopen(req2, timeout=min(timeout, 3.2)) as r2:
                adata = json.loads(r2.read().decode("utf-8", errors="ignore"))
                ic2 = ((adata.get("data") or {}).get("icon") or "").strip().replace("http://", "https://")
                if ic2:
                    return ic2
        except Exception:
            pass
        return ""
    return u

real_url = resolve_icon_url(url)
if not real_url:
    sys.exit(2)
os.makedirs(os.path.dirname(dest), exist_ok=True)
tmp = dest + ".part"
req = urllib.request.Request(real_url, headers={"User-Agent": UA})
with urllib.request.urlopen(req, timeout=timeout) as r:
    data = r.read()
if not is_raster(data):
    sys.exit(2)
with open(tmp, "wb") as f:
    f.write(data)
os.replace(tmp, dest)
"#;

const PY_BATCH_DOWNLOAD_ASSETS: &str = r#"
import sys, os, re, json, threading, urllib.request, urllib.parse
from concurrent.futures import ThreadPoolExecutor

timeout = float(sys.argv[1])
jobs = json.loads(sys.argv[2])
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"
print_lock = threading.Lock()

def is_raster(b, min_b):
    if len(b) < min_b or len(b) < 12:
        return False
    s = b[:64].lstrip()
    if s.startswith((b"<svg", b"<!DOCTYPE", b"<!doctype", b"<html", b"<HTML", b"<?xml")):
        return False
    return b.startswith(b"\x89PNG") or b.startswith(b"\xff\xd8\xff") or b.startswith(b"GIF8") or (b.startswith(b"RIFF") and b[8:12] == b"WEBP")

def resolve_icon_url(u, pkg_key):
    if "sj.qq.com/appdetail/" in u:
        pkg = u.split("sj.qq.com/appdetail/")[-1].split("?")[0].strip("/") or pkg_key
        try:
            req = urllib.request.Request(u, headers={"User-Agent": UA})
            with urllib.request.urlopen(req, timeout=min(timeout, 3.5)) as r:
                html = r.read().decode("utf-8", errors="ignore")
            m = re.search(r'<script[^>]*id="__NEXT_DATA__"[^>]*>(.*?)</script>', html, re.S)
            if m:
                data = json.loads(m.group(1))
                comps = data.get("props", {}).get("pageProps", {}).get("dynamicCardResponse", {}).get("data", {}).get("components", [])
                for comp in comps:
                    for it in ((comp.get("data") or {}).get("itemData") or []):
                        if isinstance(it, dict) and (it.get("pkg_name") or "").strip() == pkg:
                            ic = (it.get("icon") or "").strip().replace("http://", "https://")
                            if ic and not ic.endswith(".svg") and "yyb-icon" not in ic:
                                return ic
        except Exception:
            pass
        try:
            ap_url = "https://ws75.aptoide.com/api/7/app/getMeta?package_name=" + urllib.parse.quote(pkg) + "&aab=1"
            req2 = urllib.request.Request(ap_url, headers={"User-Agent": UA})
            with urllib.request.urlopen(req2, timeout=min(timeout, 3.2)) as r2:
                adata = json.loads(r2.read().decode("utf-8", errors="ignore"))
                ic2 = ((adata.get("data") or {}).get("icon") or "").strip().replace("http://", "https://")
                if ic2:
                    return ic2
        except Exception:
            pass
        return ""
    return u

def fetch_one(job):
    key = job.get("key", "")
    url = (job.get("url") or "").strip()
    dest = (job.get("dest") or "").strip()
    min_b = int(job.get("min_bytes") or 128)
    if not key or not url or not dest:
        return
    try:
        if os.path.isfile(dest) and os.path.getsize(dest) >= min_b:
            with open(dest, "rb") as rf:
                head = rf.read(64)
            if is_raster(head + b"\x00" * 128, min_b):
                with print_lock:
                    print(f"OK\t{key}\t{dest}", flush=True)
                return
            else:
                try: os.remove(dest)
                except Exception: pass
        real_url = resolve_icon_url(url, key)
        if not real_url:
            return
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        tmp = f"{dest}.{os.getpid()}.{threading.get_ident()}.part"
        req = urllib.request.Request(real_url, headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=timeout) as r:
            data = r.read()
        if not is_raster(data, min_b):
            return
        with open(tmp, "wb") as f:
            f.write(data)
        os.replace(tmp, dest)
        with print_lock:
            print(f"OK\t{key}\t{dest}", flush=True)
    except Exception:
        pass

with ThreadPoolExecutor(max_workers=8) as ex:
    list(ex.map(fetch_one, jobs))
"#;

fn download_small_asset(url: &str, dest: &Path) -> Option<PathBuf> {
    if is_valid_raster_file(dest, 128) {
        return Some(dest.to_path_buf());
    }
    let u = url.trim();
    if !u.starts_with("http://") && !u.starts_with("https://") {
        return None;
    }
    let dest_str = dest.to_string_lossy().into_owned();
    let status = resources::python_command()
        .args(["-c", PY_DOWNLOAD_ASSET, "10", u, &dest_str])
        .status()
        .ok()?;
    if status.success() && is_valid_raster_file(dest, 128) {
        Some(dest.to_path_buf())
    } else {
        None
    }
}

pub fn cache_store_icon(package: &str, icon_url: &str) -> Option<PathBuf> {
    let dest = store_cache_root()
        .join("icons")
        .join(format!("{}.png", sanitize_cache_key(package)));
    download_small_asset(icon_url, &dest)
}

pub fn cache_store_screenshot(package: &str, index: usize, url: &str) -> Option<PathBuf> {
    let dest = store_cache_root()
        .join("screenshots")
        .join(format!("{}_{}.jpg", sanitize_cache_key(package), index));
    download_small_asset(url, &dest)
}

pub fn stream_batch_icons<F: FnMut(String, PathBuf)>(items: &[(String, String)], mut on_cached: F) {
    let icons_dir = store_cache_root().join("icons");
    let mut pending_jobs = Vec::new();
    let mut seen_pkgs = std::collections::HashSet::new();

    for (pkg, url) in items {
        let p = pkg.trim();
        let u = url.trim();
        if p.is_empty() || !seen_pkgs.insert(p.to_string()) {
            continue;
        }
        if let Some(existing) = existing_cached_icon(p) {
            on_cached(p.to_string(), existing);
            continue;
        }
        if !u.starts_with("http://") && !u.starts_with("https://") {
            continue;
        }
        let dest = icons_dir.join(format!("{}.png", sanitize_cache_key(p)));
        pending_jobs.push(serde_json::json!({
            "key": p,
            "url": u,
            "dest": dest.to_string_lossy(),
            "min_bytes": 128,
        }));
    }

    if pending_jobs.is_empty() {
        return;
    }
    let Ok(jobs_json) = serde_json::to_string(&pending_jobs) else {
        return;
    };
    let Ok(mut child) = resources::python_command()
        .args(["-c", PY_BATCH_DOWNLOAD_ASSETS, "6", &jobs_json])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("OK\t") {
                if let Some((pkg_key, dest_str)) = rest.split_once('\t') {
                    let path = PathBuf::from(dest_str.trim());
                    if is_valid_raster_file(&path, 128) {
                        on_cached(pkg_key.trim().to_string(), path);
                    }
                }
            }
        }
    }
    let _ = child.wait();
}

pub fn stream_batch_screenshots<F: FnMut(String, usize, PathBuf)>(
    package: &str,
    urls: &[String],
    mut on_cached: F,
) {
    let pkg = package.trim();
    if pkg.is_empty() || urls.is_empty() {
        return;
    }
    let shots_dir = store_cache_root().join("screenshots");
    let sanitized = sanitize_cache_key(pkg);
    let mut pending_jobs = Vec::new();

    for (idx, url) in urls.iter().enumerate() {
        let dest = shots_dir.join(format!("{sanitized}_{idx}.jpg"));
        if is_valid_raster_file(&dest, 256) {
            on_cached(pkg.to_string(), idx, dest);
            continue;
        }
        let u = url.trim();
        if !u.starts_with("http://") && !u.starts_with("https://") {
            continue;
        }
        pending_jobs.push(serde_json::json!({
            "key": idx.to_string(),
            "url": u,
            "dest": dest.to_string_lossy(),
            "min_bytes": 256,
        }));
    }

    if pending_jobs.is_empty() {
        return;
    }
    let Ok(jobs_json) = serde_json::to_string(&pending_jobs) else {
        return;
    };
    let Ok(mut child) = resources::python_command()
        .args(["-c", PY_BATCH_DOWNLOAD_ASSETS, "8", &jobs_json])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("OK\t") {
                if let Some((idx_str, dest_str)) = rest.split_once('\t') {
                    if let Ok(idx) = idx_str.trim().parse::<usize>() {
                        let path = PathBuf::from(dest_str.trim());
                        if is_valid_raster_file(&path, 256) {
                            on_cached(pkg.to_string(), idx, path);
                        }
                    }
                }
            }
        }
    }
    let _ = child.wait();
}

const PY_RESOLVE_URL: &str = r#"
import sys, re, json, urllib.request, urllib.parse

timeout = float(sys.argv[1])
raw_url = sys.argv[2]
pkg = sys.argv[3]
UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"

def resolve_tencent(p):
    try:
        u = "https://a.app.qq.com/o/simple.jsp?pkgname=" + urllib.parse.quote(p)
        req = urllib.request.Request(u, headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=timeout) as r:
            html = r.read().decode("utf-8", errors="ignore")
        m = re.search(r'https?://imtt\d*\.dd\.qq\.com/[^\s"\'<>]+\.apk[^\s"\'<>]*', html)
        if m and "qqdownloader" not in m.group(0):
            return re.sub(r'sjy\.0000[89]/', 'sjy.00022/', m.group(0).replace("http://", "https://"))
    except Exception:
        pass
    try:
        d = "https://sj.qq.com/appdetail/" + urllib.parse.quote(p)
        req2 = urllib.request.Request(d, headers={"User-Agent": UA})
        with urllib.request.urlopen(req2, timeout=timeout) as r2:
            html2 = r2.read().decode("utf-8", errors="ignore")
        m_next = re.search(r'<script[^>]*id="__NEXT_DATA__"[^>]*>(.*?)</script>', html2, re.S)
        if m_next:
            data = json.loads(m_next.group(1))
            comps = data.get("props", {}).get("pageProps", {}).get("dynamicCardResponse", {}).get("data", {}).get("components", [])
            for comp in comps:
                for it in ((comp.get("data") or {}).get("itemData") or []):
                    if isinstance(it, dict) and (it.get("pkg_name") or "").strip() == p:
                        dl = (it.get("download_url") or "").strip().replace("http://", "https://")
                        if dl and "qqdownloader" not in dl:
                            return re.sub(r'sjy\.0000[89]/', 'sjy.00022/', dl)
                        md5 = (it.get("md_5") or "").strip()
                        ver = (it.get("version_name") or "").strip()
                        if md5 and ver:
                            return f"https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/{md5.upper()}.apk?fsname={p}_{ver}.apk"
    except Exception:
        pass
    return ""

def resolve_fdroid(p):
    try:
        u = "https://f-droid.org/api/v1/packages/" + urllib.parse.quote(p)
        req = urllib.request.Request(u, headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=timeout) as r:
            data = json.loads(r.read().decode("utf-8", errors="ignore"))
        vc = data.get("suggestedVersionCode")
        if not vc:
            pkgs = data.get("packages") or []
            if pkgs and isinstance(pkgs[0], dict):
                vc = pkgs[0].get("versionCode")
        if vc:
            return f"https://f-droid.org/repo/{p}_{vc}.apk"
    except Exception:
        pass
    return ""

if "f-droid.org" in raw_url and "/packages/" in raw_url:
    res = resolve_fdroid(pkg)
    print(res or raw_url)
elif "sj.qq.com/appdetail" in raw_url or not raw_url:
    res = resolve_tencent(pkg)
    print(res or raw_url)
else:
    print(re.sub(r'sjy\.0000[89]/', 'sjy.00022/', raw_url))
"#;

pub fn resolve_apk_download_url(raw_url: &str, package: &str) -> Result<String> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() && package.trim().is_empty() {
        return Err(anyhow!("下载链接为空"));
    }
    if !trimmed.is_empty()
        && !trimmed.contains("sj.qq.com/appdetail")
        && !(trimmed.contains("f-droid.org") && trimmed.contains("/packages/"))
    {
        return Ok(trimmed.to_string());
    }
    let out = resources::python_command()
        .args(["-c", PY_RESOLVE_URL, "10", trimmed, package])
        .output()
        .context("解析下载直链失败")?;
    let resolved = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if resolved.starts_with("http://") || resolved.starts_with("https://") {
        Ok(resolved)
    } else {
        Ok(trimmed.to_string())
    }
}

const PY_DOWNLOAD_APK_STREAM: &str = r#"
import sys, os, re, json, urllib.request, urllib.parse

url = (sys.argv[1] if len(sys.argv) > 1 else "").strip()
dest = sys.argv[2]
pkg = (sys.argv[3] if len(sys.argv) > 3 else "").strip()

UA_MOBILE = "Mozilla/5.0 (Linux; Android 16; PZ261A) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Safari/537.36"
UA_DESKTOP = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"

if not pkg:
    if "sj.qq.com/appdetail/" in url:
        pkg = url.split("sj.qq.com/appdetail/")[-1].split("?")[0].strip("/")
    elif "/packages/" in url:
        m_p = re.search(r'/packages/([A-Za-z0-9_.]+)', url)
        if m_p:
            pkg = m_p.group(1)

def get_text(u, t=6.0):
    req = urllib.request.Request(u, headers={"User-Agent": UA_DESKTOP, "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8"})
    with urllib.request.urlopen(req, timeout=t) as r:
        return r.read().decode("utf-8", errors="ignore")

def tencent_item_urls(it, target_pkg):
    out = []
    if not isinstance(it, dict):
        return out
    p = (it.get("pkg_name") or target_pkg or "").strip()
    raw_dl = (it.get("download_url") or it.get("apk_url") or "").strip().replace("http://", "https://")
    ver = (it.get("version_name") or "").strip()
    md5 = (it.get("md_5") or "").strip()
    if raw_dl and "qqdownloader" not in raw_dl.lower():
        fixed = re.sub(r'sjy\.0000[89]/', 'sjy.00022/', raw_dl)
        out.append(fixed)
        if fixed != raw_dl:
            out.append(raw_dl)
    if len(md5) == 32 and p:
        if ver:
            out.append(f"https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/{md5.upper()}.apk?fsname={p}_{ver}.apk")
        out.append(f"https://imtt.dd.qq.com/sjy.00022/sjy.00004/16891/apk/{md5.upper()}.apk?fsname={p}.apk")
    return out

def resolve_tencent_detail(p):
    out = []
    if not p:
        return out
    try:
        s_html = get_text("https://a.app.qq.com/o/simple.jsp?pkgname=" + urllib.parse.quote(p), 5.0)
        m = re.search(r'https?://imtt\d*\.dd\.qq\.com/[^\s"\'<>]+\.apk[^\s"\'<>]*', s_html)
        if m and "qqdownloader" not in m.group(0):
            u = m.group(0).replace("http://", "https://")
            fixed = re.sub(r'sjy\.0000[89]/', 'sjy.00022/', u)
            out.append(fixed)
            if fixed != u:
                out.append(u)
    except Exception:
        pass
    try:
        d_html = get_text("https://sj.qq.com/appdetail/" + urllib.parse.quote(p), 5.5)
        m_next = re.search(r'<script[^>]*id="__NEXT_DATA__"[^>]*>(.*?)</script>', d_html, re.S)
        if m_next:
            data = json.loads(m_next.group(1))
            comps = data.get("props", {}).get("pageProps", {}).get("dynamicCardResponse", {}).get("data", {}).get("components", [])
            for comp in comps:
                for it in ((comp.get("data") or {}).get("itemData") or []):
                    if isinstance(it, dict) and (it.get("pkg_name") or "").strip() == p:
                        out.extend(tencent_item_urls(it, p))
    except Exception:
        pass
    return out

def resolve_aptoide_meta(p):
    out = []
    if not p:
        return out
    try:
        raw = get_text("https://ws75.aptoide.com/api/7/app/getMeta?package_name=" + urllib.parse.quote(p) + "&aab=1", 6.0)
        data = (json.loads(raw).get("data") or {})
        if data:
            file_obj = data.get("file") or {}
            dl_path = (file_obj.get("path") or "").strip().replace("http://", "https://")
            if dl_path:
                out.append(dl_path)
            dl_alt = (file_obj.get("path_alt") or "").strip().replace("http://", "https://")
            if dl_alt:
                out.append(dl_alt)
            store_name = ((data.get("store") or {}).get("name") or "apps").strip()
            vcode = int(file_obj.get("vercode") or 0)
            app_id = data.get("id")
            md5 = (file_obj.get("md5sum") or "").strip()
            if store_name and vcode and app_id and md5:
                slug = re.sub(r'[^a-z0-9]+', '-', p.lower()).strip('-')
                out.append(f"https://pool.apk.aptoide.com/{store_name}/{slug}-{vcode}-{app_id}-{md5}.apk")
    except Exception:
        pass
    return out

def resolve_fdroid(p):
    out = []
    if not p:
        return out
    try:
        raw = get_text("https://f-droid.org/api/v1/packages/" + urllib.parse.quote(p), 6.0)
        data = json.loads(raw)
        vc = data.get("suggestedVersionCode")
        if vc:
            out.append(f"https://f-droid.org/repo/{p}_{vc}.apk")
        for item in (data.get("packages") or [])[:3]:
            if isinstance(item, dict) and item.get("versionCode"):
                u = f"https://f-droid.org/repo/{p}_{item['versionCode']}.apk"
                if u not in out:
                    out.append(u)
    except Exception:
        pass
    return out

def resolve_tencent_search(p):
    out = []
    if not p:
        return out
    try:
        html = get_text("https://sj.qq.com/search?q=" + urllib.parse.quote(p), 6.0)
        m_next = re.search(r'<script[^>]*id="__NEXT_DATA__"[^>]*>(.*?)</script>', html, re.S)
        if m_next:
            data = json.loads(m_next.group(1))
            comps = data.get("props", {}).get("pageProps", {}).get("dynamicCardResponse", {}).get("data", {}).get("components", [])
            exact = []
            fallback = []
            for comp in comps:
                for it in ((comp.get("data") or {}).get("itemData") or []):
                    if not isinstance(it, dict):
                        continue
                    it_pkg = (it.get("pkg_name") or "").strip()
                    if not it_pkg or it_pkg == "com.tencent.android.qqdownloader" or it_pkg.startswith("com.tencent.pcgame."):
                        continue
                    urls = tencent_item_urls(it, it_pkg)
                    if it_pkg == p:
                        exact.extend(urls)
                    elif not fallback and urls:
                        fallback.extend(urls)
            out.extend(exact)

    except Exception:
        pass
    return out

def resolve_aptoide_search(p):
    out = []
    if not p:
        return out
    try:
        raw = get_text("https://ws75.aptoide.com/api/7/apps/search?query=" + urllib.parse.quote(p) + "&limit=3&aab=1", 6.0)
        apps = ((json.loads(raw).get("datalist") or {}).get("list") or [])
        exact = []
        fallback = []
        for app in apps[:3]:
            if not isinstance(app, dict):
                continue
            ap_pkg = (app.get("package") or "").strip()
            file_obj = app.get("file") or {}
            dl = (file_obj.get("path") or "").strip().replace("http://", "https://")
            if not dl:
                store_name = ((app.get("store") or {}).get("name") or "apps").strip()
                vcode = int(file_obj.get("vercode") or 0)
                app_id = app.get("id")
                md5 = (file_obj.get("md5sum") or "").strip()
                if store_name and vcode and app_id and md5 and ap_pkg:
                    slug = re.sub(r'[^a-z0-9]+', '-', ap_pkg.lower()).strip('-')
                    dl = f"https://pool.apk.aptoide.com/{store_name}/{slug}-{vcode}-{app_id}-{md5}.apk"
            if dl:
                if ap_pkg == p:
                    exact.append(dl)
                else:
                    fallback.append(dl)
        out.extend(exact)

    except Exception:
        pass
    return out

def iter_candidates(initial_url, target_pkg):
    seen = set()
    def emit(u):
        u = (u or "").strip()
        if not u or u in seen:
            return None
        if not (u.startswith("http://") or u.startswith("https://")):
            return None
        seen.add(u)
        return u

    if "sjy.00008/" in initial_url or "sjy.00009/" in initial_url:
        c = emit(re.sub(r'sjy\.0000[89]/', 'sjy.00022/', initial_url))
        if c:
            yield c

    is_page_url = (
        "sj.qq.com/appdetail" in initial_url
        or ("f-droid.org" in initial_url and "/packages/" in initial_url)
        or "wandoujia.com/" in initial_url
    )
    if initial_url and not is_page_url:
        c = emit(initial_url)
        if c:
            yield c

    if not target_pkg:
        return

    # A pinned APK URL must not silently turn into the latest release or another store.
    if not is_page_url:
        return
    if "f-droid.org" in initial_url:
        resolvers = (resolve_fdroid,)
    elif "sj.qq.com" in initial_url:
        resolvers = (resolve_tencent_detail, resolve_tencent_search)
    else:
        resolvers = ()

    for resolver in resolvers:
        for cand in resolver(target_pkg):
            c = emit(cand)
            if c:
                yield c

os.makedirs(os.path.dirname(dest), exist_ok=True)
tmp = dest + ".part"
last_err = "未找到可用的 APK 下载源"

for cand_url in iter_candidates(url, pkg):
    try:
        req = urllib.request.Request(cand_url, headers={"User-Agent": UA_MOBILE})
        with urllib.request.urlopen(req, timeout=25) as r:
            first_chunk = r.read(131072)
            if len(first_chunk) < 4 or first_chunk[:4] != b"PK\x03\x04":
                last_err = f"下载源返回的不是有效 APK (ZIP) 安装包: {cand_url}"
                continue
            total = int(r.headers.get("Content-Length") or 0)
            downloaded = len(first_chunk)
            with open(tmp, "wb") as f:
                f.write(first_chunk)
                print(f"PROGRESS {downloaded} {total}", flush=True)
                while True:
                    chunk = r.read(131072)
                    if not chunk:
                        break
                    f.write(chunk)
                    downloaded += len(chunk)
                    print(f"PROGRESS {downloaded} {total}", flush=True)
        with open(tmp, "rb") as f:
            magic = f.read(4)
        if magic != b"PK\x03\x04":
            try:
                os.remove(tmp)
            except Exception:
                pass
            last_err = f"APK 校验失败: {cand_url}"
            continue
        os.replace(tmp, dest)
        print("DONE", flush=True)
        sys.exit(0)
    except Exception as e:
        last_err = f"{cand_url}: {e}"
        try:
            if os.path.exists(tmp):
                os.remove(tmp)
        except Exception:
            pass
        continue

sys.stderr.write(f"下载的文件不是有效的 APK (ZIP) 安装包 ({last_err})\n")
sys.exit(3)
"#;

pub fn download_apk_with_progress<F: FnMut(u64, u64)>(
    raw_url: &str,
    package: &str,
    version_name: &str,
    mut on_progress: F,
) -> Result<PathBuf> {
    let final_url = resolve_apk_download_url(raw_url, package)?;
    let apks_dir = store_cache_root().join("apks");
    fs::create_dir_all(&apks_dir).context("创建 APK 缓存目录失败")?;
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    final_url.hash(&mut hasher);
    let source_id = format!("{:016x}", hasher.finish());
    let dest = apks_dir.join(format!(
        "{}_{}_{}.apk",
        sanitize_cache_key(package),
        sanitize_cache_key(version_name),
        &source_id[..16]
    ));

    if dest.is_file() {
        let mut valid_apk = false;
        let mut file_len = 0u64;
        if let Ok(meta) = fs::metadata(&dest) {
            file_len = meta.len();
            if file_len > 65_536 {
                if let Ok(mut f) = fs::File::open(&dest) {
                    let mut magic = [0u8; 4];
                    if std::io::Read::read_exact(&mut f, &mut magic).is_ok()
                        && &magic == b"PK\x03\x04"
                    {
                        valid_apk = true;
                    }
                }
            }
        }
        if valid_apk && crate::device::check_store_apk(&dest, package).is_ok() {
            on_progress(file_len, file_len);
            return Ok(dest);
        } else {
            let _ = fs::remove_file(&dest);
        }
    }

    let dest_str = dest.to_string_lossy().into_owned();
    let mut child = resources::python_command()
        .args(["-c", PY_DOWNLOAD_APK_STREAM, &final_url, &dest_str, package])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("启动 APK 下载进程失败")?;

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("PROGRESS ") {
                let mut parts = rest.split_whitespace();
                if let (Some(dl_s), Some(tot_s)) = (parts.next(), parts.next()) {
                    if let (Ok(dl), Ok(tot)) = (dl_s.parse::<u64>(), tot_s.parse::<u64>()) {
                        on_progress(dl, tot);
                    }
                }
            }
        }
    }

    let output = child.wait_with_output().context("等待 APK 下载完成失败")?;
    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow!(if err_msg.is_empty() {
            "APK 下载失败".to_string()
        } else {
            err_msg
        }));
    }

    if !dest.is_file() {
        return Err(anyhow!("APK 下载完成后未找到目标文件"));
    }
    crate::device::check_store_apk(&dest, package)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_installed_app(
        package: &str,
        version_name: &str,
        version_code: u64,
        system: bool,
        updated_system: bool,
    ) -> AppInfo {
        AppInfo {
            package: package.into(),
            label: package.into(),
            version_name: version_name.into(),
            version_code,
            enabled: true,
            system,
            updated_system,
            launchable: true,
            ..Default::default()
        }
    }

    #[test]
    fn test_compare_android_versions() {
        assert_eq!(compare_android_versions("1.20.10", "1.21.0"), Ordering::Less);
        assert_eq!(compare_android_versions("1.21.0", "1.21.0"), Ordering::Equal);
        assert_eq!(
            compare_android_versions("4.17.1-588", "4.17.2-607"),
            Ordering::Less
        );
        assert_eq!(compare_android_versions("2.7.0", "2.6.32"), Ordering::Greater);
    }

    #[test]
    fn test_curated_catalog_contains_ugreen_and_nas_apps() {
        let catalog = curated_catalog();
        assert!(catalog.len() >= 12);

        let ugreen = catalog
            .iter()
            .find(|a| a.package == "com.ugreen.pro")
            .expect("绿联云 com.ugreen.pro must be in curated catalog");
        assert_eq!(ugreen.name, "绿联云");
        assert!(ugreen.sources.len() >= 2, "绿联云应包含多源下载通道");
        assert!(
            !ugreen.historical_versions.is_empty(),
            "绿联云应包含历史旧版本列表"
        );
        assert!(
            !ugreen.developer_apps.is_empty(),
            "绿联云应包含同开发者其他应用（绿联智联/绿联私有云）"
        );
        assert!(
            !ugreen.screenshots.is_empty(),
            "绿联云应包含应用预览截图 URL"
        );
    }

    #[test]
    fn test_multi_source_deduplication_merges_by_package() {
        let catalog = curated_catalog();
        let ugreen = catalog
            .iter()
            .find(|a| a.package == "com.ugreen.pro")
            .unwrap()
            .clone();
        let mut duplicate = ugreen.clone();
        duplicate.sources = vec![StoreSourceOffer {
            source: StoreSource::StepOs,
            version_name: "1.18.0".into(),
            version_code: 11800,
            size_bytes: 195_000_000,
            download_url: "https://example.com/ugreen-1.18.apk".into(),
            updated_at: "2026-01-10".into(),
            md5: String::new(),
            changelog: String::new(),
        }];

        let merged = merge_and_deduplicate_apps(vec![ugreen, duplicate]);
        assert_eq!(merged.len(), 1, "同一包名必须合并为一个 StoreApp（多源显示一个）");
        assert_eq!(merged[0].sources.len(), 3, "合并后应保留全部 3 个不同来源");
        assert_eq!(merged[0].version_name(), "1.20.12");
    }

    #[test]
    fn test_install_and_update_state_detection() {
        let catalog = curated_catalog();
        let ugreen = catalog
            .iter()
            .find(|a| a.package == "com.ugreen.pro")
            .unwrap();

        // 1. Not installed
        assert_eq!(
            check_store_install_state(&[], ugreen),
            StoreInstallState::NotInstalled
        );

        // 2. Installed older version -> Updatable
        let old_inst = vec![mock_installed_app(
            "com.ugreen.pro",
            "1.20.10",
            12010,
            false,
            false,
        )];
        match check_store_install_state(&old_inst, ugreen) {
            StoreInstallState::Updatable {
                installed_version,
                latest_version,
                can_uninstall,
                ..
            } => {
                assert_eq!(installed_version, "1.20.10");
                assert_eq!(latest_version, "1.20.12");
                assert!(can_uninstall);
            }
            other => panic!("expected Updatable, got {other:?}"),
        }

        // 3. Installed latest version -> Installed
        let cur_inst = vec![mock_installed_app(
            "com.ugreen.pro",
            "1.20.12",
            12012,
            false,
            false,
        )];
        match check_store_install_state(&cur_inst, ugreen) {
            StoreInstallState::Installed {
                installed_version,
                can_uninstall,
                ..
            } => {
                assert_eq!(installed_version, "1.20.12");
                assert!(can_uninstall);
            }
            other => panic!("expected Installed, got {other:?}"),
        }
    }

    #[test]
    fn test_global_social_catalog_and_relevance_search() {
        let catalog = curated_catalog();
        assert!(
            catalog.len() >= 80,
            "精选应用目录应包含至少 80 款跨分类应用, 实际: {}",
            catalog.len()
        );
        for cat in [
            StoreCategory::Social,
            StoreCategory::Media,
            StoreCategory::Productivity,
            StoreCategory::Tools,
            StoreCategory::Lifestyle,
        ] {
            let count = catalog.iter().filter(|a| a.category == cat).count();
            assert!(
                count >= 15,
                "分类 {:?} ({}) 应包含至少 15 款应用, 实际: {}",
                cat,
                cat.label(),
                count
            );
        }

        // Search "x" -> X (Twitter) (com.twitter.android) must be #1 and must NOT match com.xingin.xhs
        let mut x_matches: Vec<StoreApp> = catalog
            .iter()
            .filter(|a| a.matches_query("x"))
            .cloned()
            .collect();
        x_matches.sort_by(|a, b| {
            score_app_relevance(b, "x")
                .cmp(&score_app_relevance(a, "x"))
                .then_with(|| b.featured.cmp(&a.featured))
        });
        assert!(!x_matches.is_empty(), "搜索 'x' 必须返回 X (Twitter)");
        assert_eq!(x_matches[0].package, "com.twitter.android");
        assert!(
            !x_matches.iter().any(|a| a.package == "com.xingin.xhs"),
            "单字母 'x' 不应误匹配小红书 (com.xingin.xhs)"
        );

        // Search "facebook" -> Facebook (com.facebook.katana) must be #1
        let mut fb_matches: Vec<StoreApp> = catalog
            .iter()
            .filter(|a| a.matches_query("facebook"))
            .cloned()
            .collect();
        fb_matches.sort_by(|a, b| {
            score_app_relevance(b, "facebook")
                .cmp(&score_app_relevance(a, "facebook"))
                .then_with(|| b.featured.cmp(&a.featured))
        });
        assert!(!fb_matches.is_empty(), "搜索 'facebook' 必须返回 Facebook");
        assert_eq!(fb_matches[0].package, "com.facebook.katana");
        assert!(
            fb_matches[0].historical_versions.len() >= 2,
            "Facebook 应包含真实历史旧版本"
        );
        assert!(
            fb_matches[0]
                .developer_apps
                .iter()
                .any(|d| d.package == "com.instagram.android"),
            "Facebook 应包含同开发商应用 Instagram"
        );

        // Also test aliases ("推特", "脸书", "tg", "油管")
        assert!(x_matches[0].matches_query("推特"));
        assert!(fb_matches[0].matches_query("脸书"));
        assert_eq!(expand_upstream_query("x"), "twitter");
        assert_eq!(expand_upstream_query("fb"), "facebook");
    }

    #[test]
    fn test_category_sub_tags_and_matching() {
        let catalog = curated_catalog();
        assert!(!category_sub_tags(StoreCategory::All).is_empty());
        assert!(category_sub_tags(StoreCategory::Updatable).is_empty());
        assert!(category_sub_tags(StoreCategory::Installed).is_empty());

        let ugreen = catalog
            .iter()
            .find(|a| a.package == "com.ugreen.pro")
            .unwrap();
        assert!(app_matches_sub_tag(ugreen, ""));
        assert!(app_matches_sub_tag(ugreen, "全部"));
        assert!(app_matches_sub_tag(ugreen, "装机必备"));
        assert!(app_matches_sub_tag(ugreen, "NAS 私有云"));

        let deepseek = catalog
            .iter()
            .find(|a| a.package == "com.deepseek.chat")
            .unwrap();
        assert!(app_matches_sub_tag(deepseek, "AI 助手"));

        let twitter = catalog
            .iter()
            .find(|a| a.package == "com.twitter.android")
            .unwrap();
        assert!(app_matches_sub_tag(twitter, "国际社交"));
        assert!(app_matches_sub_tag(twitter, "社交通讯"));
    }

    #[test]
    fn test_online_catalog_fetch_and_persistent_cache() {
        let (mut merged, mut label, mut online_count, mut is_fresh) = load_startup_catalog();
        if !is_fresh || online_count < 500 {
            let (m, l, cnt) = fetch_online_catalog_from_upstream()
                .expect("fetch_online_catalog_from_upstream should succeed");
            merged = m;
            label = Some(l);
            online_count = cnt;
            is_fresh = load_startup_catalog().3;
        }
        assert!(
            online_count >= 500,
            "在线曲库应至少缓存 500 款真实应用, 实际: {online_count}"
        );
        assert!(
            merged.len() >= 580,
            "合并精选与在线曲库后总应用数应 >= 580, 实际: {}",
            merged.len()
        );
        assert!(is_fresh, "持久化缓存应处于新鲜状态");
        assert!(label.is_some(), "应包含最后同步时间标签");
        assert!(
            online_catalog_cache_file().is_file(),
            "online_catalog_v1.json 必须持久化到磁盘"
        );
        assert!(
            !merged
                .iter()
                .any(|a| a.package.starts_with("com.tencent.pcgame.")
                    || a.package == "com.tencent.android.qqdownloader"
                    || a.download_url().ends_with(".exe")),
            "在线曲库不得混入 PC .exe 游戏或应用宝自身"
        );

        for cat in [
            StoreCategory::Social,
            StoreCategory::Media,
            StoreCategory::Productivity,
            StoreCategory::Tools,
            StoreCategory::Lifestyle,
        ] {
            for &sub_tag in category_sub_tags(cat) {
                let matches_count = merged
                    .iter()
                    .filter(|a| a.category == cat && app_matches_sub_tag(a, sub_tag))
                    .count();
                assert!(
                    matches_count >= 3,
                    "分类 {:?} 子标签 '{}' 应至少包含 3 款应用, 实际: {}",
                    cat,
                    sub_tag,
                    matches_count
                );
            }
        }
    }
}
