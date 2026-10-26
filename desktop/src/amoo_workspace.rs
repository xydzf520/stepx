//! Task records shared by chat, desktop file work and local app creation.
//! Requests are routed automatically (see `amoo_route`); the chosen route is
//! recorded on the task so the UI can show it and the user can correct it.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Intent {
    /// Default: route automatically from the message text and attachments.
    #[default]
    Auto,
    /// Forced ordinary chat (e.g. "改为普通聊天").
    Chat,
    Files,
    Image,
    App,
    Screen,
}
impl Intent {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "自动",
            Self::Chat => "对话",
            Self::Files => "处理文件",
            Self::Image => "生成图片",
            Self::App => "制作应用",
            Self::Screen => "背屏创作",
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Executor {
    Phone,
    Files,
    Image,
    App,
    Screen,
}
impl Executor {
    pub fn label(self) -> &'static str {
        match self {
            Self::Phone => "手机 Amoo 回复",
            Self::Files => "处理资料并生成文件",
            Self::Image => "生成图片并接收原图",
            Self::App => "制作应用并运行检查",
            Self::Screen => "生成背屏点阵图案",
        }
    }
    pub fn source(self) -> &'static str {
        if self == Self::App {
            "手机 Amoo / 本机引擎 · 浏览器自检"
        } else {
            "手机 Amoo · 手机账号"
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Ready,
    Running,
    Done,
    Failed,
    Stopped,
    Unconfirmed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step {
    pub executor: Executor,
    pub phase: Phase,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Turn {
    pub user: bool,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Output {
    pub path: PathBuf,
    pub name: String,
    pub kind: String,
    pub lab_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub path: PathBuf,
    pub note: String,
    pub success: bool,
    pub device: String,
    pub account: Option<String>,
    pub cloud_url: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub device: Option<String>,
    pub account: Option<String>,
    pub session: Option<String>,
    pub draft: String,
    pub attachments: Vec<PathBuf>,
    #[serde(default)]
    pub submitted_files: Vec<PathBuf>,
    pub intent: Intent,
    pub workspace: Option<PathBuf>,
    pub turns: Vec<Turn>,
    pub steps: Vec<Step>,
    pub outputs: Vec<Output>,
    #[serde(default)]
    pub receipts: Vec<Receipt>,
    pub note: String,
    pub updated: i64,
    /// Route chosen automatically for the latest request, shown as an inline,
    /// correctable hint (e.g. "生成 PPT"). Empty for plain chat.
    #[serde(default)]
    pub route_note: String,
    /// Sticky route for follow-up edits ("改成蓝色" continues the last artifact).
    #[serde(default)]
    pub route: Option<Intent>,
}
impl Task {
    pub fn new(device: Option<String>, account: Option<String>) -> Self {
        Self {
            id: format!("task-{:032x}", rand::random::<u128>()),
            title: "新对话".into(),
            device,
            account,
            session: None,
            draft: String::new(),
            attachments: vec![],
            submitted_files: vec![],
            intent: Intent::Auto,
            workspace: None,
            turns: vec![],
            steps: vec![],
            outputs: vec![],
            receipts: vec![],
            note: String::new(),
            updated: chrono::Utc::now().timestamp(),
            route_note: String::new(),
            route: None,
        }
    }
    pub fn visible(&self, device: Option<&str>, account: Option<&str>) -> bool {
        if let Some(d) = self.device.as_deref() {
            if Some(d) != device {
                return false;
            }
        }
        self.account.as_deref().is_none_or(|a| Some(a) == account)
    }
    pub fn add_output(&mut self, output: Output) {
        if let Some(existing) = self.outputs.iter_mut().find(|v| v.path == output.path) {
            *existing = output;
        } else {
            self.outputs.push(output);
        }
    }
    /// Record an automatic or forced routing decision for the latest request.
    /// Chat clears the inline note; an explicit "chat only" also clears the
    /// sticky follow-up route, while ordinary chat keeps it for later edits.
    pub fn apply_route(&mut self, decision: &crate::amoo_route::RouteDecision) {
        self.route_note = decision.label.to_string();
        if decision.opt_out {
            self.route = None;
        } else if let Some(sticky) = decision.sticky {
            self.route = Some(sticky);
        }
    }
    pub fn meaningful(&self) -> bool {
        !self.turns.is_empty() || !self.outputs.is_empty() || self.session.is_some()
    }
}

pub fn clean_task_title(raw: &str) -> String {
    let mut s = raw
        .trim()
        .trim_start_matches("[仅普通对话，不启动本地文档或网页生成]")
        .trim()
        .lines()
        .next()
        .unwrap_or("")
        .trim();
    if let Some(rest) = s.strip_prefix("围绕我的手机日程/提醒「") {
        if let Some((inner, _)) = rest.split_once('」') {
            let inner = inner.trim();
            if !inner.is_empty() {
                return format!("日程：{inner}");
            }
        }
        s = rest.trim_end_matches('」').trim();
    } else if let Some(rest) = s.strip_prefix("请帮我分析这张来自手机相册的图片") {
        let rest = rest
            .trim()
            .trim_start_matches('（')
            .trim_start_matches('(')
            .trim_end_matches('）')
            .trim_end_matches(')')
            .trim();
        if rest.is_empty() {
            return "分析手机相册图片".into();
        }
        return format!("相册图片：{rest}")
            .chars()
            .take(22)
            .collect::<String>();
    }
    if s.is_empty() {
        "新对话".into()
    } else {
        let mut out: String = s.chars().take(24).collect();
        if s.chars().count() > 24 || out.ends_with("，2") {
            if let Some((prefix, tail)) = out.rsplit_once(['，', '、', ',']) {
                if !prefix.is_empty() && tail.chars().count() <= 2 {
                    out = prefix.to_string();
                }
            }
        }
        out
    }
}

pub fn is_automated_test_task(task: &Task) -> bool {
    const TEST_MARKERS: &[&str] = &[
        "只修改附件 Word：把项目预算从2580改为300",
        "这是 StepX Companion 统一任务验收",
        "这是 StepX Companion 对话验收",
        "这是 StepX Companion 聊天验收",
        "这是 StepX Companion 多附件验收",
        "这是第二条测试对话，请只回答 23 加 19",
        "验收办公场景：读取附件 CSV",
        "这是键盘发送验收。记住测试数 47",
        "请直接阅读这三个附件，不要创建任务或生成文件",
        "根据附件做三份可编辑文件：1. sales.xlsx",
        "图片生成时输入的新草稿，必须保留",
        "保留草稿TEST",
        "打开关于手机，查看 Android 版本，不修改设置",
        "STEPX_VERIFY",
    ];
    let matches_marker = |text: &str| TEST_MARKERS.iter().any(|m| text.contains(m));
    if matches_marker(&task.title) || matches_marker(&task.draft) {
        return true;
    }
    task.turns.iter().any(|t| matches_marker(&t.text))
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Journal {
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub deleted_sessions: Vec<String>,
}
impl Journal {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        ensure!(
            std::fs::metadata(path)?.len() < 16 * 1024 * 1024,
            "任务记录过大"
        );
        let mut value: Self = serde_json::from_slice(&std::fs::read(path)?)
            .context("任务记录无法读取，原文件已保留")?;
        let mut deleted_sessions = value.deleted_sessions;
        value.tasks.retain(|task| {
            if is_automated_test_task(task) {
                if let Some(key) = &task.session {
                    if !key.trim().is_empty() && !deleted_sessions.contains(key) {
                        deleted_sessions.push(key.clone());
                    }
                }
                return false;
            }
            let completely_empty = task.turns.is_empty()
                && task.outputs.is_empty()
                && task.session.is_none()
                && task.steps.is_empty()
                && task.submitted_files.is_empty()
                && task.attachments.is_empty()
                && task.draft.trim().is_empty();
            !completely_empty
        });
        value.deleted_sessions = deleted_sessions;
        // A closed process is never evidence that a remote task completed.
        for task in &mut value.tasks {
            while task.turns.last().is_some_and(|t| {
                !t.user && t.text.trim() == "No such file or directory (os error 2)"
            }) {
                task.turns.pop();
                if task.turns.last().is_some_and(|t| t.user) {
                    task.turns.pop();
                }
            }
            task.turns
                .retain(|t| t.text.trim() != "No such file or directory (os error 2)");
            task.steps
                .retain(|s| s.note.trim() != "No such file or directory (os error 2)");
            if task.note.trim() == "No such file or directory (os error 2)" {
                task.note.clear();
            }
            if task.turns.is_empty() {
                if let Some(first_out) = task.outputs.first() {
                    if !first_out.name.trim().is_empty() {
                        task.title = clean_task_title(&first_out.name);
                    }
                }
            }
            task.title = clean_task_title(&task.title);
            if matches!(task.title.as_str(), "新对话" | "新任务") {
                if let Some(first_user) = task.turns.iter().find(|t| t.user) {
                    task.title = clean_task_title(&first_user.text);
                } else if let Some(first_out) = task.outputs.first() {
                    if !first_out.name.trim().is_empty() {
                        task.title = clean_task_title(&first_out.name);
                    }
                }
            } else if let Some(first_user) = task.turns.iter().find(|t| t.user) {
                let better = clean_task_title(&first_user.text);
                if better.starts_with("日程：") || better.starts_with("相册图片：") {
                    task.title = better;
                }
            }
            // Routing is automatic. Older records stored `Chat` as the default
            // mode; a forced route is one-shot and never restored on launch.
            if task.intent == Intent::Chat
                || (task.draft.trim().is_empty() && task.attachments.is_empty())
            {
                task.intent = Intent::Auto;
            }
            for step in &mut task.steps {
                if step.phase == Phase::Running {
                    step.phase = Phase::Unconfirmed;
                    step.note = "上次执行已中断；手机任务需同步核对，不会自动重发".into();
                }
            }
        }
        Ok(value)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("任务目录无效")?;
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            file.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        use std::io::Write;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }
}

pub fn is_unconfirmed_placeholder(text: &str) -> bool {
    let trimmed = text.trim();
    matches!(
        trimmed,
        "已停止等待；手机任务可能仍继续，可同步查看结果"
            | "上次执行已中断；手机任务需同步核对，不会自动重发"
            | "手机连接中断，尚未确认回复完成。重连后同步任务记录；未自动重发。"
            | "桌面生成流程已中断；请核对当前结果后再继续"
    )
}
/// Live, side-effect-free preview of what Enter will do with the current draft.
/// Returns `None` for plain chat; otherwise the route and a short human label
/// (e.g. "生成 PPT", "做小游戏") for the composer hint chip.
/// SHARED CONTRACT: routing agent owns the implementation; UI only calls it.
pub fn preview_route(
    prompt: &str,
    has_files: bool,
    sticky: Option<Intent>,
) -> Option<(Intent, &'static str)> {
    if prompt.trim().is_empty() {
        return None;
    }
    let ctx = crate::amoo_route::RouteCtx {
        forced: None,
        sticky,
        last: None,
    };
    let d = crate::amoo_route::route_attached(
        prompt,
        crate::amoo_route::Attached::unknown(has_files),
        &ctx,
    );
    (!d.is_chat()).then_some((d.intent, d.label))
}

/// Primary automatic route of a message without conversation context.
pub fn detect_explicit_intent(prompt: &str, has_files: bool) -> Intent {
    crate::amoo_route::route_attached(
        prompt,
        crate::amoo_route::Attached::unknown(has_files),
        &crate::amoo_route::RouteCtx::default(),
    )
    .intent
}

/// Ordered executors for a message. `Intent::Auto` routes automatically;
/// any other intent is an explicit, forced route (`Chat` = chat only).
pub fn propose(prompt: &str, intent: Intent, attachments: bool) -> Vec<Executor> {
    propose_followup(prompt, intent, attachments, None)
}

/// Follow-up edits continue the last executor; questions stay ordinary chat.
pub fn propose_followup(
    prompt: &str,
    intent: Intent,
    attachments: bool,
    last: Option<Executor>,
) -> Vec<Executor> {
    let ctx = crate::amoo_route::RouteCtx {
        forced: Some(intent),
        sticky: None,
        last,
    };
    crate::amoo_route::route_attached(
        prompt,
        crate::amoo_route::Attached::unknown(attachments),
        &ctx,
    )
    .steps
}

/// Only text results from the accepted task are sent to the app model, bounded
/// in size. No arbitrary directory scan or executable contents are read.
pub fn app_context(task: &Task) -> Result<String> {
    let Some(root) = &task.workspace else {
        return Ok(String::new());
    };
    if !root.exists() {
        return Ok(String::new());
    }
    let Ok(root) = root.canonicalize() else {
        return Ok(String::new());
    };
    let mut text = String::new();
    for file in &task.outputs {
        if file.lab_id.is_some() {
            continue;
        }
        let extension = file
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let is_text = ["txt", "md", "csv", "json"].contains(&extension.as_str());
        let is_office = ["docx", "xlsx", "pptx"].contains(&extension.as_str());
        if !is_text && !is_office {
            continue;
        }
        let Ok(path) = file.path.canonicalize() else {
            continue;
        };
        ensure!(path.starts_with(&root), "成果不在此任务的资料目录中");
        let value = if is_office {
            let Ok(rel) = path.strip_prefix(&root) else {
                continue;
            };
            match crate::work::Workspace::open(&root)
                .and_then(|ws| ws.read_text(&rel.to_string_lossy()))
            {
                Ok(v) => v.chars().take(3000).collect::<String>(),
                Err(_) => continue,
            }
        } else {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            ensure!(meta.len() <= 1024 * 1024, "文本成果过大，请选择更小的资料");
            let Ok(v) = std::fs::read_to_string(path) else {
                continue;
            };
            v
        };
        let remaining = 24000usize.saturating_sub(text.chars().count());
        if remaining == 0 {
            break;
        }
        text.push_str(&format!(
            "\n资料 {}：\n{}\n",
            file.name,
            value.chars().take(remaining.min(12000)).collect::<String>()
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amoo_route::{route_intent, RouteCtx};
    fn auto(
        text: &str,
        files: &[&str],
        last: Option<Executor>,
        sticky: Option<Intent>,
    ) -> Vec<Executor> {
        let files = files.iter().map(PathBuf::from).collect::<Vec<_>>();
        route_intent(
            text,
            &files,
            &RouteCtx {
                forced: None,
                sticky,
                last,
            },
        )
        .steps
    }
    #[test]
    fn auto_is_default_and_explicit_deliverables_route_cleanly() {
        assert_eq!(Intent::default(), Intent::Auto);
        assert_eq!(Task::new(None, None).intent, Intent::Auto);
        assert_eq!(Intent::Auto.label(), "自动");
        // Without a previous artifact, edits and questions stay chat.
        for text in ["改成蓝色", "这张图片是什么", "今天天气怎么样"] {
            assert_eq!(
                propose_followup(text, Intent::default(), true, None),
                vec![Executor::Phone],
                "{text}"
            );
        }
        // Follow-up edits continue the previous executor; questions do not.
        for last in [Executor::Files, Executor::App, Executor::Image] {
            assert_eq!(
                propose_followup("改成蓝色", Intent::default(), false, Some(last)),
                vec![last]
            );
            for q in ["这张图片是什么", "今天天气怎么样"] {
                assert_eq!(
                    propose_followup(q, Intent::default(), false, Some(last)),
                    vec![Executor::Phone],
                    "{q}"
                );
            }
        }
        assert_eq!(
            propose_followup("分析这个表格", Intent::default(), true, None),
            vec![Executor::Files]
        );
        assert_eq!(
            propose_followup("生成 PPT", Intent::default(), false, None),
            vec![Executor::Files]
        );
        assert_eq!(
            propose_followup("做个游戏", Intent::default(), false, None),
            vec![Executor::App]
        );
    }
    #[test]
    fn detect_explicit_intent_covers_games_office_apps_and_chat() {
        // Games -> Intent::App
        for p in [
            "帮我做个游戏",
            "写个贪吃蛇小游戏",
            "做一个 2048",
            "做一个太空射击游戏",
            "写个俄罗斯方块",
        ] {
            assert_eq!(detect_explicit_intent(p, false), Intent::App, "{p}");
        }
        // Build App -> Intent::App
        for p in [
            "做个应用：番茄钟",
            "做一个App记录习惯打卡",
            "生成应用：记账本",
            "做一个背屏控制器可交互网页",
            "做个网页",
            "做个计算器",
        ] {
            assert_eq!(detect_explicit_intent(p, false), Intent::App, "{p}");
        }
        // Office -> Intent::Files
        for p in [
            "生成一份季度汇报 PPT",
            "导出 Word 文档报告.docx",
            "生成 Excel 表格汇总.xlsx",
            "整理成文档并生成报告文件",
            "帮我做个PPT",
            "生成一份 Word 报告",
            "写一份周报",
            "季度汇报PPT",
        ] {
            assert_eq!(detect_explicit_intent(p, false), Intent::Files, "{p}");
        }
        assert_eq!(
            detect_explicit_intent("帮我汇总这份资料", true),
            Intent::Files
        );
        // Images
        for p in ["画一只猫", "设计一张新品海报", "生成一张图片：海边日落"] {
            assert_eq!(detect_explicit_intent(p, false), Intent::Image, "{p}");
        }
        // Screen -> Intent::Screen
        assert_eq!(
            detect_explicit_intent("设计一个背屏图案并发送到背屏", false),
            Intent::Screen
        );
        // Questions & explicit chat-only -> Intent::Chat
        for p in [
            "如何制作一个贪吃蛇小游戏？",
            "2048 怎么做？",
            "这张图片是什么",
            "阅读附件，只用文字回答，不生成文件",
            "今天上海会下雨吗？",
            "这份表格讲了什么",
            "俄罗斯方块",
        ] {
            assert_eq!(detect_explicit_intent(p, true), Intent::Chat, "{p}");
        }
    }
    #[test]
    fn polite_shells_still_generate() {
        assert_eq!(
            auto("能不能帮我做个PPT", &[], None, None),
            vec![Executor::Files]
        );
        assert_eq!(
            auto("帮我做个游戏可以吗", &[], None, None),
            vec![Executor::App]
        );
        assert_eq!(
            auto("可以帮我写一份周报吗？", &[], None, None),
            vec![Executor::Files]
        );
        assert_eq!(auto("麻烦画一只猫", &[], None, None), vec![Executor::Image]);
        assert_eq!(
            preview_route("能不能帮我做个PPT", false, None),
            Some((Intent::Files, "生成 PPT"))
        );
        assert_eq!(
            preview_route("帮我做个游戏可以吗", false, None),
            Some((Intent::App, "做小游戏"))
        );
    }
    #[test]
    fn knowledge_questions_stay_chat() {
        for p in [
            "如何制作一个贪吃蛇小游戏？",
            "2048 怎么做？",
            "这张图片是什么",
            "PPT 和 Keynote 有什么区别？",
            "你能做PPT吗",
        ] {
            assert_eq!(auto(p, &[], None, None), vec![Executor::Phone], "{p}");
            assert_eq!(preview_route(p, false, None), None, "{p}");
        }
        assert_eq!(
            auto("这张图片是什么", &["photo.png"], None, None),
            vec![Executor::Phone]
        );
    }
    #[test]
    fn preview_labels_are_short_and_plain() {
        let cases = [
            ("生成一份季度汇报 PPT", "生成 PPT"),
            ("生成一份 Word 报告", "生成 Word 文档"),
            ("做一个带公式的 Excel 表格", "生成 Excel 表格"),
            ("写个贪吃蛇小游戏", "做小游戏"),
            ("做个番茄钟应用", "做小应用"),
            ("画一只猫", "生成图片"),
            (
                "分析销售表格，生成汇报 PPT，再做一个手机看板",
                "先整理文件再做应用",
            ),
            ("设计一个背屏图案并发送到背屏", "背屏图案"),
        ];
        for (p, label) in cases {
            assert_eq!(
                preview_route(p, false, None).map(|v| v.1),
                Some(label),
                "{p}"
            );
        }
        assert_eq!(preview_route("今天聊聊天", false, None), None);
        assert_eq!(preview_route("", true, None), None);
    }
    #[test]
    fn sticky_route_continues_short_edits() {
        for sticky in [Intent::Files, Intent::App, Intent::Image] {
            let expected = match sticky {
                Intent::Files => Executor::Files,
                Intent::App => Executor::App,
                _ => Executor::Image,
            };
            for p in ["改成蓝色", "第三页加个图表", "标题再大一点"] {
                assert_eq!(
                    auto(p, &[], Some(Executor::Phone), Some(sticky)),
                    vec![expected],
                    "{p}"
                );
            }
            // Questions are not edits.
            assert_eq!(
                auto("第三页讲了什么？", &[], None, Some(sticky)),
                vec![Executor::Phone]
            );
            assert_eq!(auto("谢谢", &[], None, Some(sticky)), vec![Executor::Phone]);
        }
        assert_eq!(
            preview_route("改成蓝色", false, Some(Intent::App)),
            Some((Intent::App, "继续修改应用"))
        );
        // A new explicit request beats the sticky route.
        assert_eq!(
            auto("再做一个贪吃蛇游戏", &[], None, Some(Intent::Files)),
            vec![Executor::App]
        );
    }
    #[test]
    fn chat_only_overrides_and_clears_sticky() {
        let ctx = RouteCtx {
            forced: None,
            sticky: Some(Intent::Files),
            last: Some(Executor::Files),
        };
        for p in [
            "只聊天，改成蓝色是什么感觉",
            "不要生成文件，直接告诉我第三页怎么改",
            "现在继续普通对话，不再生成文件。刚才结果表中 B 项的金额是多少？",
            "帮我做个PPT的大纲，只用文字回答",
        ] {
            let d = route_intent(p, &[], &ctx);
            assert_eq!(d.steps, vec![Executor::Phone], "{p}");
            assert!(d.opt_out, "{p}");
            let mut task = Task::new(None, None);
            task.route = Some(Intent::Files);
            task.route_note = "生成 PPT".into();
            task.apply_route(&d);
            assert_eq!(task.route, None);
            assert!(task.route_note.is_empty());
        }
    }
    #[test]
    fn attachments_with_actions_become_files() {
        assert_eq!(
            auto("帮我整理一下", &["销售.xlsx"], None, None),
            vec![Executor::Files]
        );
        let d = route_intent(
            "汇总这几份数据",
            &[PathBuf::from("a.csv"), PathBuf::from("b.xlsx")],
            &RouteCtx::default(),
        );
        assert_eq!(d.label, "生成 Excel 表格");
        assert_eq!(
            auto("分析这份合同的风险", &["合同.docx"], None, None),
            vec![Executor::Files]
        );
        // Attachment + pure question stays chat; images alone stay chat.
        assert_eq!(
            auto("这份表格合计多少？", &["a.csv"], None, None),
            vec![Executor::Phone]
        );
        assert_eq!(
            auto("请查看这张照片", &["a.png"], None, None),
            vec![Executor::Phone]
        );
        // Office attachment + app request digests the files first.
        assert_eq!(
            auto("根据附件做一个手机看板", &["sales.xlsx"], None, None),
            vec![Executor::Files, Executor::App]
        );
    }
    #[test]
    fn reroute_updates_route_note_and_sticky() {
        let text = "帮我做个PPT";
        let mut task = Task::new(None, None);
        let first = route_intent(text, &[], &RouteCtx::default());
        task.apply_route(&first);
        assert_eq!(task.route_note, "生成 PPT");
        assert_eq!(task.route, Some(Intent::Files));
        // "改为制作应用"
        let app = route_intent(
            text,
            &[],
            &RouteCtx {
                forced: Some(Intent::App),
                ..Default::default()
            },
        );
        assert_eq!(app.steps, vec![Executor::App]);
        task.apply_route(&app);
        assert_eq!(task.route_note, "做小应用");
        assert_eq!(task.route, Some(Intent::App));
        // "改为普通聊天"
        let chat = route_intent(
            text,
            &[],
            &RouteCtx {
                forced: Some(Intent::Chat),
                ..Default::default()
            },
        );
        assert_eq!(chat.steps, vec![Executor::Phone]);
        task.apply_route(&chat);
        assert!(task.route_note.is_empty());
        assert_eq!(task.route, None);
        // Ordinary chat keeps the sticky route for later edits.
        task.route = Some(Intent::Files);
        task.apply_route(&route_intent("今天天气怎么样", &[], &RouteCtx::default()));
        assert_eq!(task.route, Some(Intent::Files));
        assert!(task.route_note.is_empty());
    }
    #[test]
    fn migrate_legacy_chat_default_to_auto_without_changing_draft_or_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        let mut task = Task::new(None, None);
        task.intent = Intent::Chat;
        task.draft = "生成一个游戏".into();
        let mut forced = Task::new(None, None);
        forced.intent = Intent::Files;
        forced.draft = "整理表格".into();
        Journal {
            tasks: vec![task, forced],
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        let journal = Journal::load(&path).unwrap();
        assert_eq!(journal.tasks[0].intent, Intent::Auto);
        assert_eq!(journal.tasks[0].draft, "生成一个游戏");
        assert_eq!(journal.tasks[1].intent, Intent::Files);
    }
    #[test]
    fn combined_request_has_ordered_real_executors() {
        assert_eq!(
            propose(
                "分析销售表格，生成汇报 PPT，再做一个手机看板",
                Intent::Auto,
                true
            ),
            vec![Executor::Files, Executor::App]
        );
    }
    #[test]
    fn ordinary_questions_do_not_start_local_harness() {
        for p in ["如何制作一个游戏？", "今天上海会下雨吗？", "这张图片是什么"]
        {
            assert_eq!(propose(p, Intent::Auto, true), vec![Executor::Phone]);
            assert_eq!(propose(p, Intent::Chat, true), vec![Executor::Phone]);
        }
    }
    #[test]
    fn explicit_choice_and_explicit_intent_work() {
        assert_eq!(
            propose("今天聊聊天", Intent::Auto, false),
            vec![Executor::Phone]
        );
        assert_eq!(
            propose("做个贪吃蛇小游戏", Intent::Auto, false),
            vec![Executor::App]
        );
        // Forced chat beats an explicit deliverable.
        assert_eq!(
            propose("做个贪吃蛇小游戏", Intent::Chat, false),
            vec![Executor::Phone]
        );
        assert_eq!(
            propose("修改这一项", Intent::App, false),
            vec![Executor::App]
        );
        assert_eq!(
            propose("随便写点", Intent::Files, false),
            vec![Executor::Files]
        );
    }
    #[test]
    fn task_records_keep_drafts_outputs_and_do_not_resume_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tasks.json");
        let mut t = Task::new(Some("phone-a".into()), Some("account-a".into()));
        t.draft = "中文草稿".into();
        t.attachments.push(PathBuf::from("照片.png"));
        t.steps.push(Step {
            executor: Executor::App,
            phase: Phase::Running,
            note: String::new(),
        });
        let journal = Journal {
            tasks: vec![t],
            ..Default::default()
        };
        journal.save(&path).unwrap();
        let saved = Journal::load(&path).unwrap();
        assert_eq!(saved.tasks[0].draft, "中文草稿");
        assert_eq!(saved.tasks[0].attachments, journal.tasks[0].attachments);
        assert_eq!(saved.tasks[0].steps[0].phase, Phase::Unconfirmed);
        assert!(!saved.tasks[0].visible(Some("phone-b"), Some("account-a")));
        assert!(!saved.tasks[0].visible(Some("phone-a"), Some("account-b")));
    }
    #[test]
    fn app_context_only_contains_this_tasks_text_outputs() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path().join("report.md");
        std::fs::write(&p, "实际核对结果").unwrap();
        std::fs::write(root.path().join("private.txt"), "不应上传").unwrap();
        let mut t = Task::new(None, None);
        t.workspace = Some(root.path().into());
        t.add_output(Output {
            path: p,
            name: "report.md".into(),
            kind: "document".into(),
            lab_id: None,
        });
        let result = app_context(&t).unwrap();
        assert!(result.contains("实际核对结果"));
        assert!(!result.contains("不应上传"));
    }
    #[test]
    fn followup_edit_keeps_task_executor_but_questions_do_not() {
        assert_eq!(
            propose_followup("把按钮改成蓝色", Intent::Auto, false, Some(Executor::App)),
            vec![Executor::App]
        );
        assert_eq!(
            propose_followup(
                "增加一列累计金额",
                Intent::Auto,
                false,
                Some(Executor::Files)
            ),
            vec![Executor::Files]
        );
        assert_eq!(
            propose_followup(
                "这个应用如何修改？",
                Intent::Auto,
                false,
                Some(Executor::App)
            ),
            vec![Executor::Phone]
        );
        assert_eq!(
            propose_followup("改成蓝色", Intent::Auto, false, Some(Executor::Phone)),
            vec![Executor::Phone]
        );
    }
    #[test]
    fn accepted_materials_survive_draft_clearing() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path().join("tasks.json");
        let mut t = Task::new(None, None);
        t.submitted_files.push("input.csv".into());
        t.attachments.clear();
        Journal {
            tasks: vec![t],
            ..Default::default()
        }
        .save(&p)
        .unwrap();
        assert_eq!(
            Journal::load(&p).unwrap().tasks[0].submitted_files,
            vec![PathBuf::from("input.csv")]
        );
    }
    #[test]
    fn context_rejects_files_outside_selected_task_directory() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("outside.md");
        std::fs::write(&file, "outside").unwrap();
        let mut t = Task::new(None, None);
        t.workspace = Some(root.path().into());
        t.add_output(Output {
            path: file,
            name: "outside.md".into(),
            kind: "text".into(),
            lab_id: None,
        });
        assert!(app_context(&t).is_err());
    }
    #[test]
    fn test_app_context_with_nonexistent_workspace_and_app_output_succeeds() {
        let root = tempfile::tempdir().unwrap();
        let missing_ws = root.path().join("nonexistent-task-dir");
        let mut t = Task::new(None, None);
        t.workspace = Some(missing_ws);
        t.add_output(Output {
            path: root.path().join("starter-neon-snake/round-1/index.html"),
            name: "霓虹贪吃蛇大作战".into(),
            kind: "应用 · 已检查".into(),
            lab_id: Some("starter-neon-snake".into()),
        });
        assert_eq!(app_context(&t).unwrap(), "");
    }
}
