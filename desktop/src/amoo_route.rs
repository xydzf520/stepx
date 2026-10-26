//! Automatic routing for the single Amoo conversation.
//!
//! The user only talks; this module decides whether a message is ordinary
//! chat, a document (PPT/Word/Excel), a small app or game, an image or a back
//! screen pattern. Everything here is a pure function so it can be unit tested
//! and previewed live in the composer without side effects.
use crate::amoo_workspace::{Executor, Intent};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocKind {
    Ppt,
    Word,
    Excel,
    Any,
}
impl DocKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ppt => "生成 PPT",
            Self::Word => "生成 Word 文档",
            Self::Excel => "生成 Excel 表格",
            Self::Any => "生成文件",
        }
    }
}

/// Context of the current conversation used for one routing decision.
#[derive(Clone, Copy, Debug, Default)]
pub struct RouteCtx {
    /// One-shot route chosen explicitly (menu, Reroute). `Some(Auto)` = none.
    pub forced: Option<Intent>,
    /// `Task.route`: the executor that produced the latest artifact.
    pub sticky: Option<Intent>,
    /// Executor of the last step of the task.
    pub last: Option<Executor>,
}

/// What the attachments of this message look like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attached {
    pub office: bool,
    pub image: bool,
    pub kind: Option<DocKind>,
}
impl Attached {
    pub fn from_paths(files: &[PathBuf]) -> Self {
        let mut v = Self::default();
        for f in files {
            let ext = ext_of(f);
            match ext.as_str() {
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "heic" => v.image = true,
                "pptx" | "ppt" | "key" => {
                    v.office = true;
                    v.kind = merge_kind(v.kind, DocKind::Ppt);
                }
                "xlsx" | "xls" | "csv" | "tsv" | "numbers" => {
                    v.office = true;
                    v.kind = merge_kind(v.kind, DocKind::Excel);
                }
                "docx" | "doc" | "pages" | "rtf" => {
                    v.office = true;
                    v.kind = merge_kind(v.kind, DocKind::Word);
                }
                "pdf" | "md" | "txt" | "json" | "html" | "htm" => {
                    v.office = true;
                    v.kind = merge_kind(v.kind, DocKind::Any);
                }
                _ => {}
            }
        }
        v
    }
    /// Used by the live composer preview, which only knows "has files".
    pub fn unknown(has_files: bool) -> Self {
        Self {
            office: has_files,
            image: false,
            kind: None,
        }
    }
}
fn ext_of(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}
fn merge_kind(a: Option<DocKind>, b: DocKind) -> Option<DocKind> {
    match a {
        None => Some(b),
        Some(k) if k == b => Some(k),
        Some(_) => Some(DocKind::Any),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteDecision {
    /// Primary route (a combined documents-then-app request reports `Files`).
    pub intent: Intent,
    /// Ordered executors to run.
    pub steps: Vec<Executor>,
    /// Short human label for the inline chip, e.g. "生成 PPT". Empty for chat.
    pub label: &'static str,
    /// Route to remember for follow-up edits (`Task.route`). `None` = keep.
    pub sticky: Option<Intent>,
    /// The user explicitly asked for chat only; sticky routes are cleared.
    pub opt_out: bool,
    /// This is a follow-up edit continuing the previous artifact.
    pub followup: bool,
}
impl RouteDecision {
    fn chat(opt_out: bool) -> Self {
        Self {
            intent: Intent::Chat,
            steps: vec![Executor::Phone],
            label: "",
            sticky: None,
            opt_out,
            followup: false,
        }
    }
    fn single(intent: Intent, label: &'static str) -> Self {
        let (executor, sticky) = match intent {
            Intent::Files => (Executor::Files, Some(Intent::Files)),
            Intent::App => (Executor::App, Some(Intent::App)),
            Intent::Image => (Executor::Image, Some(Intent::Image)),
            Intent::Screen => (Executor::Screen, None),
            Intent::Chat | Intent::Auto => return Self::chat(false),
        };
        Self {
            intent,
            steps: vec![executor],
            label,
            sticky,
            opt_out: false,
            followup: false,
        }
    }
    fn files_then_app() -> Self {
        Self {
            intent: Intent::Files,
            steps: vec![Executor::Files, Executor::App],
            label: "先整理文件再做应用",
            sticky: Some(Intent::App),
            opt_out: false,
            followup: false,
        }
    }
    pub fn is_chat(&self) -> bool {
        self.steps == [Executor::Phone]
    }
}

/// Lowercase, full-width → half-width, and drop whitespace next to CJK text
/// ("生成 Word" → "生成word"). Spaces between Latin words are kept.
pub fn normalize(text: &str) -> String {
    let chars: Vec<char> = text
        .chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .flat_map(char::to_lowercase)
        .collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let prev = out.chars().last();
            let next = chars.get(i).copied();
            if let (Some(p), Some(n)) = (prev, next) {
                if !is_cjk(p) && !is_cjk(n) {
                    out.push(' ');
                }
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF)
}

fn ascii_word(term: &str) -> bool {
    term.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b' ' || b == b'\'')
}
/// End byte offsets of every occurrence; ASCII terms need word boundaries
/// ("app" does not match "apple", "word" does not match "password").
fn match_ends(t: &str, term: &str) -> Vec<usize> {
    let bytes = t.as_bytes();
    let boundary = ascii_word(term);
    t.match_indices(term)
        .filter(|(i, m)| {
            !boundary
                || ((*i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
                    && bytes
                        .get(i + m.len())
                        .is_none_or(|b| !b.is_ascii_alphanumeric()))
        })
        .map(|(i, m)| i + m.len())
        .collect()
}
fn has(t: &str, terms: &[&str]) -> bool {
    terms.iter().any(|w| !match_ends(t, w).is_empty())
}
fn last_end(t: &str, terms: &[&str]) -> Option<usize> {
    terms
        .iter()
        .filter_map(|w| match_ends(t, w).into_iter().max())
        .max()
}

const CHAT_ONLY: &[&str] = &[
    "只聊天",
    "只聊聊",
    "只是聊天",
    "普通聊天",
    "普通对话",
    "随便聊",
    "不生成文件",
    "不要生成文件",
    "别生成文件",
    "不用生成文件",
    "无需生成文件",
    "不需要生成文件",
    "不再生成文件",
    "不生成新文件",
    "不要生成新文件",
    "不再生成新文件",
    "不要生成任何",
    "只用文字",
    "只回答",
    "只回复",
    "直接回答",
    "直接告诉我",
    "不用做文件",
    "不要做文件",
    "不要做成",
    "不用做成",
    "不要执行",
    "just chat",
    "chat only",
    "no file",
    "don't generate",
    "do not generate",
    "text only",
];
const STRONG_QUESTION: &[&str] = &[
    "怎么做",
    "怎么制作",
    "如何制作",
    "如何做",
    "怎样做",
    "怎样制作",
    "怎么写",
    "如何写",
    "怎么弄",
    "如何弄",
    "怎么画",
    "如何画",
    "怎么修改",
    "如何修改",
    "怎么改",
    "如何改",
    "是什么",
    "什么意思",
    "为什么",
    "教我",
    "教程",
    "原理",
    "区别",
    "推荐",
    "介绍一下",
    "how to",
    "how do i",
    "what is",
    "why",
];
const WEAK_QUESTION: &[&str] = &[
    "怎么", "如何", "怎样", "吗", "?", "呢", "什么", "多少", "哪", "是否",
];
/// Polite shells such as "能不能…" / "…可以吗" wrap requests, not questions.
const POLITE: &[&str] = &[
    "帮我",
    "给我",
    "替我",
    "帮忙",
    "请",
    "麻烦",
    "能不能",
    "能否",
    "可不可以",
    "可以帮",
    "please",
    "can you",
    "could you",
];
const CREATE: &[&str] = &[
    "做",
    "写",
    "生成",
    "制作",
    "创建",
    "开发",
    "设计",
    "搭建",
    "编写",
    "整理成",
    "导出",
    "转成",
    "转换成",
    "输出",
    "起草",
    "撰写",
    "出一份",
    "来一份",
    "来个",
    "画",
    "create",
    "make",
    "generate",
    "build",
    "write",
    "draft",
    "export",
    "design",
    "draw",
];
const PPT: &[&str] = &[
    "ppt",
    "pptx",
    "幻灯片",
    "演示文稿",
    "slides",
    "slide deck",
    "keynote",
    "汇报材料",
    "路演",
];
const WORD: &[&str] = &[
    "word",
    "docx",
    "文档",
    "报告",
    "方案",
    "周报",
    "月报",
    "年报",
    "纪要",
    "合同",
    "简历",
    "说明书",
    "论文",
    "策划书",
    "计划书",
    "申请书",
    "report",
    "document",
    "memo",
];
const EXCEL: &[&str] = &[
    "excel",
    "xlsx",
    "xls",
    "csv",
    "表格",
    "报表",
    "台账",
    "透视表",
    "spreadsheet",
];
const GAME: &[&str] = &[
    "游戏",
    "game",
    "贪吃蛇",
    "2048",
    "俄罗斯方块",
    "打砖块",
    "五子棋",
    "象棋",
    "扫雷",
    "太空射击",
    "飞机大战",
    "消消乐",
    "跑酷",
];
const APP: &[&str] = &[
    "app",
    "应用",
    "网页",
    "h5",
    "看板",
    "仪表盘",
    "dashboard",
    "小工具",
    "工具",
    "计算器",
    "计时器",
    "番茄钟",
    "记账",
    "打卡",
    "小程序",
    "网站",
    "website",
    "webpage",
    "web page",
    "落地页",
    "landing page",
];
const IMAGE: &[&str] = &[
    "图片",
    "图像",
    "照片",
    "海报",
    "插画",
    "插图",
    "头像",
    "壁纸",
    "logo",
    "配图",
    "封面图",
    "表情包",
    "画一",
    "画个",
    "画张",
    "画幅",
    "画只",
    "画出",
    "生图",
    "image",
    "picture",
    "poster",
    "illustration",
    "wallpaper",
    "avatar",
];
const SCREEN: &[&str] = &["背屏", "灯阵", "点阵", "led屏", "背面屏"];
const SCREEN_SEND: &[&str] = &[
    "发送到背屏",
    "发到背屏",
    "显示到背屏",
    "显示在背屏",
    "投到背屏",
    "放到背屏",
];
const FILE_ACTION: &[&str] = &[
    "整理", "汇总", "分析", "核对", "计算", "合并", "提取", "修改", "转成", "转换", "生成", "统计",
    "对比", "清洗", "去重", "翻译", "润色", "排版", "改写", "批注", "修订",
];
const IMAGE_EDIT: &[&str] = &[
    "修图",
    "抠图",
    "换背景",
    "改成",
    "风格",
    "p图",
    "美化",
    "去水印",
    "上色",
];
const EDIT: &[&str] = &[
    "改",
    "调整",
    "换成",
    "换个",
    "换",
    "加一",
    "加个",
    "加上",
    "增加",
    "添加",
    "删",
    "去掉",
    "去除",
    "移除",
    "再来",
    "再做",
    "再加",
    "再生成",
    "再给",
    "继续",
    "优化",
    "润色",
    "修订",
    "放大",
    "缩小",
    "加粗",
    "替换",
    "更新",
    "补充",
    "重新",
    "大一点",
    "小一点",
    "居中",
    "对齐",
    "edit",
    "change",
    "make it",
];
const NOUN_ONLY_SUFFIX: &[&str] = &[
    "ppt",
    "pptx",
    "xlsx",
    "docx",
    "excel表格",
    "word文档",
    "演示文稿",
];

fn is_question(t: &str) -> bool {
    if has(t, STRONG_QUESTION) {
        return true;
    }
    let polite = has(&t.replace("请问", ""), POLITE);
    has(t, WEAK_QUESTION) && !polite
}

/// Split into clauses by punctuation and sequencing words, so
/// "生成汇报PPT，再做一个看板" yields two independent requests.
fn clauses(t: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut iter = t.char_indices().peekable();
    let mut prev: Option<char> = None;
    while let Some((i, c)) = iter.next() {
        let next = iter.peek().map(|(_, c)| *c);
        let cut = match c {
            ',' | ';' | '。' | '!' | '?' | '\n' | '；' | '，' => Some(c.len_utf8()),
            '然' if next == Some('后') => Some(0),
            '并' if prev != Some('合') => Some(0),
            '再' if prev != Some('不') => Some(0),
            '接' if next == Some('着') => Some(0),
            '以' if next == Some('及') => Some(0),
            '另' if next == Some('外') => Some(0),
            _ => None,
        };
        if let Some(skip) = cut {
            if i > start {
                out.push(&t[start..i]);
            }
            start = i + skip;
        }
        prev = Some(c);
    }
    if start < t.len() {
        out.push(&t[start..]);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Doc(DocKind),
    App,
    Image,
    Screen,
}
fn doc_kind(t: &str) -> Option<DocKind> {
    let mut kind = None;
    if has(t, PPT) {
        kind = merge_kind(kind, DocKind::Ppt);
    }
    if has(t, EXCEL) {
        kind = merge_kind(kind, DocKind::Excel);
    }
    if has(t, WORD) {
        kind = merge_kind(kind, DocKind::Word);
    }
    kind
}
/// A clause with a creation verb targets its head noun: in Chinese the
/// modified noun comes last ("AI 应用的报告" is a report, "报告看板" an app).
fn clause_target(c: &str) -> Option<Target> {
    if !has(c, CREATE) {
        return None;
    }
    let mut best: Option<(usize, Target)> = None;
    let mut consider = |end: Option<usize>, target: Target| {
        if let Some(end) = end {
            if best.is_none_or(|(b, _)| end > b) {
                best = Some((end, target));
            }
        }
    };
    let doc_end = [last_end(c, PPT), last_end(c, WORD), last_end(c, EXCEL)]
        .into_iter()
        .flatten()
        .max();
    consider(doc_end, Target::Doc(doc_kind(c).unwrap_or(DocKind::Any)));
    consider(last_end(c, APP).max(last_end(c, GAME)), Target::App);
    consider(last_end(c, IMAGE), Target::Image);
    consider(last_end(c, SCREEN), Target::Screen);
    best.map(|(_, t)| t)
}

fn app_label(t: &str) -> &'static str {
    if has(t, GAME) {
        "做小游戏"
    } else {
        "做小应用"
    }
}

fn sticky_route(ctx: &RouteCtx) -> Option<Intent> {
    ctx.sticky
        .filter(|i| matches!(i, Intent::Files | Intent::App | Intent::Image))
        .or(match ctx.last {
            Some(Executor::Files) => Some(Intent::Files),
            Some(Executor::App) => Some(Intent::App),
            Some(Executor::Image) => Some(Intent::Image),
            _ => None,
        })
}

/// Route for an explicitly chosen intent (menu choice or Reroute).
pub fn forced_route(intent: Intent, text: &str) -> RouteDecision {
    let t = normalize(text);
    match intent {
        Intent::Chat => RouteDecision::chat(true),
        Intent::Files => {
            RouteDecision::single(Intent::Files, doc_kind(&t).unwrap_or(DocKind::Any).label())
        }
        Intent::App => RouteDecision::single(Intent::App, app_label(&t)),
        Intent::Image => RouteDecision::single(Intent::Image, "生成图片"),
        Intent::Screen => RouteDecision::single(Intent::Screen, "背屏图案"),
        Intent::Auto => route_attached(text, Attached::default(), &RouteCtx::default()),
    }
}

/// Detect unambiguous instant phone-control commands typed in the main Amoo chat box
/// so they can be routed directly to the zero-config phone screen controller.
pub fn parse_instant_phone_control(text: &str) -> Option<(String, String)> {
    let t = text
        .trim()
        .trim_end_matches(|c: char| matches!(c, '。' | '！' | '!' | '.' | ' '));
    match t {
        "亮屏"
        | "点亮屏幕"
        | "唤醒屏幕"
        | "点亮手机"
        | "点亮手机屏幕"
        | "帮我点亮手机屏幕"
        | "帮我点亮屏幕"
        | "唤醒手机"
        | "唤醒手机屏幕"
        | "解锁手机"
        | "点亮并解锁"
        | "点亮并解锁手机" => Some(("当前屏幕".into(), "点亮屏幕".into())),
        "息屏" | "锁屏" | "关闭屏幕" | "锁定屏幕" | "把手机锁屏" | "手机息屏" | "手机锁屏" => {
            Some(("当前屏幕".into(), "锁屏".into()))
        }
        "截屏"
        | "截图"
        | "截取画面"
        | "截取手机画面"
        | "帮我截取手机画面"
        | "给手机截个屏"
        | "手机截屏"
        | "手机截图"
        | "刷新画面"
        | "刷新手机画面"
        | "看看手机屏幕"
        | "查看手机画面" => Some(("当前屏幕".into(), "截取手机画面".into())),
        "提取屏幕文字"
        | "提取手机屏幕文字"
        | "帮我提取手机屏幕文字"
        | "读取手机屏幕文字"
        | "识别手机屏幕文字" => Some(("当前屏幕".into(), "提取屏幕文字".into())),
        "手机静音" | "把手机静音" | "帮我把手机静音" => {
            Some(("当前屏幕".into(), "手机静音".into()))
        }
        "打开手机Wi-Fi设置"
        | "打开手机WiFi设置"
        | "打开手机 Wi-Fi 设置"
        | "打开 Wi-Fi 设置"
        | "打开WiFi设置" => Some(("设置".into(), "打开 Wi-Fi 设置".into())),
        "打开手机蓝牙设置" | "打开蓝牙设置" => {
            Some(("设置".into(), "打开蓝牙设置".into()))
        }
        "打开手机系统设置" | "打开系统设置" | "打开手机设置" => {
            Some(("设置".into(), "打开系统设置".into()))
        }
        "回到桌面" | "返回桌面" | "回桌面" | "回主页" | "回主屏幕" | "桌面" | "手机返回桌面"
        | "回到手机桌面" => Some(("当前屏幕".into(), "返回桌面".into())),
        "返回" | "返回上一页" | "返回上一级" | "后退" | "按返回键" | "手机返回" => {
            Some(("当前屏幕".into(), "返回上一页".into()))
        }
        "上滑" | "上滑屏幕" | "往上滑" | "手机上滑" => {
            Some(("当前屏幕".into(), "上滑屏幕".into()))
        }
        "下滑" | "下滑屏幕" | "往下滑" | "手机下滑" => {
            Some(("当前屏幕".into(), "下滑屏幕".into()))
        }
        "在手机上打开微信" | "打开手机微信" => {
            Some(("微信".into(), "打开微信".into()))
        }
        "在手机上打开支付宝" | "打开手机支付宝" => {
            Some(("支付宝".into(), "打开支付宝".into()))
        }
        "在手机上打开相机" | "打开手机相机" => {
            Some(("相机".into(), "打开相机".into()))
        }
        "在手机上打开相册" | "打开手机相册" => {
            Some(("相册".into(), "打开相册".into()))
        }
        _ => None,
    }
}

pub fn is_followup_edit(text: &str) -> bool {
    let t = normalize(text);
    !has(&t, CHAT_ONLY) && !is_question(&t) && has(&t, EDIT)
}

pub fn wants_files(text: &str, has_files: bool) -> bool {
    let d = route_attached(text, Attached::unknown(has_files), &RouteCtx::default());
    d.intent == Intent::Files || d.steps.contains(&Executor::Files)
}

pub fn wants_app(text: &str, has_files: bool) -> bool {
    let d = route_attached(text, Attached::unknown(has_files), &RouteCtx::default());
    d.intent == Intent::App || d.steps.contains(&Executor::App)
}

/// When `active` is `Intent::Files` or `Intent::App`, check whether the user's
/// new message should break out to `Intent::Chat` so normal follow-up
/// conversation isn't trapped in file/app generation.
pub fn breaks_active_route(active: Intent, text: &str, has_files: bool) -> bool {
    let t = normalize(text);
    if has(&t, CHAT_ONLY) || is_question(&t) {
        return true;
    }
    match active {
        Intent::Files | Intent::App => {
            !has_files
                && !wants_files(text, has_files)
                && !wants_app(text, has_files)
                && !is_followup_edit(text)
        }
        _ => false,
    }
}

/// Decide the route for one message. Pure: no I/O besides file extensions.
pub fn route_intent(text: &str, files: &[PathBuf], ctx: &RouteCtx) -> RouteDecision {
    route_attached(text, Attached::from_paths(files), ctx)
}

pub fn route_attached(text: &str, att: Attached, ctx: &RouteCtx) -> RouteDecision {
    if let Some(forced) = ctx.forced.filter(|i| *i != Intent::Auto) {
        return forced_route(forced, text);
    }
    let t = normalize(text);
    // 1. Explicit "chat only" wins and clears any sticky route.
    if has(&t, CHAT_ONLY) {
        return RouteDecision::chat(true);
    }
    // 2. Knowledge questions stay chat ("如何制作一个贪吃蛇小游戏？").
    let question = is_question(&t);
    if question {
        return RouteDecision::chat(false);
    }
    // 3. Explicit creation requests, clause by clause.
    let mut targets: Vec<Target> = clauses(&t).into_iter().filter_map(clause_target).collect();
    if targets.is_empty() {
        targets.extend(clause_target(&t));
    }
    let docs: Vec<DocKind> = targets
        .iter()
        .filter_map(|x| match x {
            Target::Doc(k) => Some(*k),
            _ => None,
        })
        .collect();
    let app = targets.contains(&Target::App);
    if app && (!docs.is_empty() || att.office) {
        return RouteDecision::files_then_app();
    }
    if app {
        return RouteDecision::single(Intent::App, app_label(&t));
    }
    if !docs.is_empty() {
        let kind = docs
            .iter()
            .skip(1)
            .fold(Some(docs[0]), |acc, k| merge_kind(acc, *k))
            .unwrap_or(DocKind::Any);
        return RouteDecision::single(Intent::Files, kind.label());
    }
    if targets.contains(&Target::Screen) || (has(&t, SCREEN) && has(&t, SCREEN_SEND)) {
        return RouteDecision::single(Intent::Screen, "背屏图案");
    }
    if targets.contains(&Target::Image) {
        return RouteDecision::single(Intent::Image, "生成图片");
    }
    // 4. Attachments plus a processing verb.
    if att.office && has(&t, FILE_ACTION) {
        let kind = doc_kind(&t).or(att.kind).unwrap_or(DocKind::Any);
        let label = if kind == DocKind::Any {
            "整理文件"
        } else {
            kind.label()
        };
        return RouteDecision::single(Intent::Files, label);
    }
    if att.image && !att.office && has(&t, IMAGE_EDIT) {
        return RouteDecision::single(Intent::Image, "生成图片");
    }
    // 5. Follow-up edits continue the previous artifact ("改成蓝色").
    if let Some(sticky) = sticky_route(ctx) {
        if has(&t, EDIT) {
            let label = match sticky {
                Intent::App => "继续修改应用",
                Intent::Image => "继续修改图片",
                _ => "继续修改文件",
            };
            let mut d = RouteDecision::single(sticky, label);
            d.followup = true;
            return d;
        }
    }
    // 6. Terse noun-only requests such as "季度汇报PPT".
    if t.chars().count() <= 20 && NOUN_ONLY_SUFFIX.iter().any(|s| t.ends_with(s)) {
        return RouteDecision::single(Intent::Files, doc_kind(&t).unwrap_or(DocKind::Any).label());
    }
    RouteDecision::chat(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalize_joins_cjk_and_latin_and_folds_width() {
        assert_eq!(normalize("生成 Word 报告"), "生成word报告");
        assert_eq!(normalize("ＰＰＴ　做一个"), "ppt做一个");
        assert_eq!(normalize("Make a web page"), "make a web page");
        assert_eq!(normalize("2048 怎么做？"), "2048怎么做?");
    }
    #[test]
    fn clauses_split_sequenced_requests() {
        assert_eq!(
            clauses("分析销售表格,生成汇报ppt,再做一个手机看板"),
            vec!["分析销售表格", "生成汇报ppt", "再做一个手机看板"]
        );
        assert_eq!(clauses("合并表格"), vec!["合并表格"]);
    }
    #[test]
    fn head_noun_decides_mixed_clauses() {
        let ctx = RouteCtx::default();
        assert_eq!(
            route_intent("写一份关于 AI 应用的报告", &[], &ctx).steps,
            vec![Executor::Files]
        );
        assert_eq!(
            route_intent("做一个背屏控制器可交互网页", &[], &ctx).steps,
            vec![Executor::App]
        );
    }
    #[test]
    fn instant_phone_control_commands_are_detected_cleanly() {
        assert_eq!(
            parse_instant_phone_control("帮我点亮手机屏幕"),
            Some(("当前屏幕".into(), "点亮屏幕".into()))
        );
        assert_eq!(
            parse_instant_phone_control("提取手机屏幕文字"),
            Some(("当前屏幕".into(), "提取屏幕文字".into()))
        );
        assert_eq!(
            parse_instant_phone_control("在手机上打开微信"),
            Some(("微信".into(), "打开微信".into()))
        );
        assert_eq!(parse_instant_phone_control("7 乘 8 等于多少？"), None);
    }
}
