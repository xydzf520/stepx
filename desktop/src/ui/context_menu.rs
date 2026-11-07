//! Conventional context actions, sharing the same validation as keyboard/buttons.
use super::{theme::*, widgets::*, *};
use facloud_ui::{color::ColorU, fonts::Weight, platform::Cursor, TypedActionView};

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
#[allow(non_upper_case_globals)]
impl icons::UiIcon {
    const FileText: Self = Self::ScanText;
    const Eye: Self = Self::Lock;
}

#[derive(Clone, Debug)]
pub(super) struct ContextMenuEntry {
    pub(super) label: String,
    pub(super) key: String,
    pub(super) action: Option<Action>,
    pub(super) icon: icons::UiIcon,
    pub(super) is_danger: bool,
    pub(super) separator_before: bool,
}

fn classify_context_entry(label: &str, action: Option<&Action>) -> (icons::UiIcon, bool, u8) {
    use icons::UiIcon;
    if label.starts_with("✓ 立即过期") || label.starts_with("🧹 一键清理") {
        return (UiIcon::Check, false, 0);
    }
    if label.starts_with("⏱ 稍后提醒") {
        return (UiIcon::Clock, false, 1);
    }
    if label.starts_with("⌛ ") {
        return (UiIcon::Clock, false, 2);
    }
    if label.starts_with("✓ ⚙ ") || label.starts_with("⚙ ") {
        return (UiIcon::Settings, false, 2);
    }
    if label.starts_with("🔕 屏蔽") {
        return (UiIcon::Ban, true, 3);
    }
    if label.starts_with("🗂 打开首页提醒与过期管理") {
        return (UiIcon::Layers, false, 4);
    }
    let is_otp = label.contains("复制验证码")
        || label.starts_with("复制码值")
        || matches!(action, Some(Action::Push(PushAction::CopyOtp(_))));
    if is_otp {
        return (UiIcon::Key, false, 0);
    }

    let is_danger = label.contains("删除")
        || label.contains("移至回收站")
        || label.contains("卸载")
        || label.contains("移除")
        || label.contains("清除")
        || label.contains("清空");
    if is_danger {
        return (UiIcon::Trash, true, 3);
    }

    if label.contains("剪切") {
        return (UiIcon::Edit, false, 1);
    }
    if label.contains("粘贴") {
        return (UiIcon::Clipboard, false, 2);
    }
    if label.contains("复制") {
        return (UiIcon::Copy, false, 2);
    }
    if label.starts_with("✓ ")
        || label.contains("全选")
        || label.contains("标为已读")
        || label.contains("标为未读")
        || label.contains("标记为已完成")
        || label.contains("标记完成")
        || label.contains("取消已完成")
        || label.contains("设为未完")
        || label.contains("启用此系统服务")
    {
        return (UiIcon::Check, false, 0);
    }
    if label.contains("设为过期") || label.contains("不再提醒") {
        return (UiIcon::Clock, false, 1);
    }
    if label.contains("恢复有效提醒") || label.contains("恢复今日闹钟提醒") {
        return (UiIcon::Restore, false, 1);
    }
    if label.contains("撤销") || label.contains("向左平移") || label.contains("返回上一级") {
        return (
            UiIcon::ArrowLeft,
            false,
            if label.contains("撤销") { 0 } else { 1 },
        );
    }
    if label.contains("重做") || label.contains("向右平移") {
        return (
            UiIcon::ArrowRight,
            false,
            if label.contains("重做") { 0 } else { 1 },
        );
    }
    if label.contains("向上平移") || label.contains("上滑解锁") {
        return (UiIcon::ArrowUp, false, 1);
    }
    if label.contains("向下平移") || label.contains("下拉手机通知栏") {
        return (UiIcon::ArrowDown, false, 1);
    }
    if label.contains("置顶") || label.contains("取消置顶") {
        return (UiIcon::Pin, false, 0);
    }
    if label.contains("引用并继续提问")
        || label.contains("切换至此对话")
        || label.contains("查看所在对话")
        || label.contains("仅聊天问答")
    {
        let group = if label.contains("切换至此对话") { 0 } else { 1 };
        return (UiIcon::Chat, false, group);
    }
    if label.contains("Amoo") || label.contains("自动识别") || label.contains("智能") {
        return (UiIcon::Sparkles, false, 1);
    }
    if label.contains("重新生成") || label.contains("重试") {
        return (UiIcon::Retry, false, 1);
    }
    if label.contains("朗读") || label.contains("声音：") {
        return (UiIcon::Speaker, false, 1);
    }
    if label.contains("重命名") || label.contains("修改") || label.contains("编辑") {
        return (UiIcon::Edit, false, 1);
    }
    if label.contains("文本文件") || label.contains("处理文件") {
        return (UiIcon::FileText, false, 2);
    }
    if label.contains("新建文件夹") || label.contains("新建") {
        let group = if label.contains("在此目录新建") { 2 } else { 0 };
        return (UiIcon::Plus, false, group);
    }
    if label.contains("日历")
        || label.contains("日程")
        || label.contains("顺延")
        || label.contains("克隆日程")
        || label.contains("快速添加：")
    {
        let group = if label.contains("快速添加：") { 0 } else { 1 };
        return (UiIcon::Calendar, false, group);
    }
    if label.contains("闹钟") || label.contains("推迟 10 分钟") || label.contains("时钟应用") {
        return (UiIcon::Clock, false, 1);
    }
    if label.contains("下载")
        || label.contains("保存到电脑")
        || label.contains("导出")
        || label.contains("另存")
        || label.contains("保存截图")
    {
        return (UiIcon::Download, false, 2);
    }
    if label.contains("上传") || label.contains("导入") {
        return (UiIcon::Upload, false, 2);
    }
    if label.contains("文件夹")
        || label.contains("文件管理器")
        || label.contains("目录")
        || label.contains("文件管理")
    {
        return (UiIcon::Folder, false, 0);
    }
    if label.contains("全屏") || label.contains("预览") {
        return (UiIcon::Maximize, false, 0);
    }
    if label.contains("旋转") || label.contains("刷新") || label.contains("同步") {
        return (UiIcon::Refresh, false, 1);
    }
    if label.contains("反色") {
        return (UiIcon::Contrast, false, 1);
    }
    if label.contains("背屏") || label.contains("像素") || label.contains("点阵") {
        let group = if label.contains("立即应用") || label.contains("关联到背屏") {
            0
        } else {
            1
        };
        return (UiIcon::Matrix, false, group);
    }
    if label.contains("还原") {
        return (UiIcon::Restore, false, 0);
    }
    if label.contains("拨打") || label.contains("回拨") || label.contains("拨号盘") {
        let group = if label.contains("立即拨打") || label.contains("填入拨号盘") {
            0
        } else {
            1
        };
        return (UiIcon::Phone, false, group);
    }
    if label.contains("短信") {
        return (UiIcon::Sms, false, 1);
    }
    if label.contains("收藏") || label.contains("默认连接设备") {
        let group = if label.contains("默认连接设备") { 0 } else { 1 };
        return (UiIcon::Star, false, group);
    }
    if label.contains("强行停止") || label.contains("停止") || label.contains("停用") {
        return (UiIcon::Ban, false, 1);
    }
    if label.contains("查看应用详情与权限") || matches!(action, Some(Action::InspectApp(_))) {
        return (UiIcon::Settings, false, 0);
    }
    if label.contains("设置") || label.contains("详情与权限") {
        return (UiIcon::Settings, false, 1);
    }
    if label.contains("截图") || label.contains("截取") {
        return (UiIcon::Camera, false, 2);
    }
    if label.contains("制作应用") || label.contains("小游戏") {
        return (UiIcon::Apps, false, 0);
    }
    if label.contains("生成图片") || label.contains("海报") || label.contains("看图") {
        return (UiIcon::Image, false, 0);
    }
    if label.contains("隐藏输入") || label.contains("显示输入") {
        return (UiIcon::Eye, false, 2);
    }
    if label.contains("回到手机桌面") {
        return (UiIcon::Home, false, 1);
    }
    if label.contains("投屏") {
        return (UiIcon::Monitor, false, 0);
    }
    if label.contains("存储空间") {
        return (UiIcon::Storage, false, 0);
    }
    if label.contains("扩容归档") {
        return (UiIcon::Archive, false, 0);
    }
    if label.contains("手机") || label.contains("连接此设备") || label.contains("连接设备") {
        let group = if label.contains("发送到手机") || label.contains("发送提醒到手机") {
            1
        } else {
            0
        };
        return (UiIcon::Smartphone, false, group);
    }
    if label.contains("打开") || label.contains("浏览器") {
        return (UiIcon::ExternalLink, false, 0);
    }
    (UiIcon::Tools, false, 1)
}

#[derive(Clone, Debug)]
pub(super) enum StudioWorkTarget {
    WorkspaceOutput {
        task_id: String,
        output_idx: usize,
        path: PathBuf,
        title: String,
        is_app: bool,
    },
    LocalArtifact {
        round_idx: usize,
        path: PathBuf,
        title: String,
    },
    CloudProject {
        source: u8,
        idx: usize,
        url: String,
        title: String,
    },
    SavedWork {
        idx: usize,
        url: String,
        title: String,
    },
}

#[derive(Clone, Debug)]
pub(super) enum OverviewContextKind {
    Device,
    Battery,
    Storage,
    ScreenPreview,
}

#[derive(Clone, Debug)]
pub(super) enum Target {
    Input(usize),
    Selection,
    File(String),
    LocalFile(PathBuf),
    LocalDirectory(String),
    WorkspaceIntent(bool),
    GalleryItem {
        idx: usize,
        remote_path: String,
    },
    ExpansionItem {
        id: String,
        local_path: Option<PathBuf>,
        phone_path: String,
        is_archived: bool,
    },
    WorkspaceOutput(usize),
    ChatAttachment {
        path: String,
        url: String,
        remove_idx: Option<usize>,
    },
    ChatMessage {
        assistant: bool,
        text: String,
        session_key: Option<String>,
        retry_text: Option<String>,
    },
    ChatSession {
        workspace_task_id: Option<String>,
        phone_session_idx: Option<usize>,
        title: String,
    },
    StudioWork(StudioWorkTarget),
    SmsThread {
        address: String,
        name: String,
        last_body: String,
        otp_code: Option<String>,
    },
    SmsMessage {
        address: String,
        sender: String,
        body: String,
        date_ms: i64,
        otp_code: Option<String>,
    },
    ImThread {
        thread_id: String,
        package_name: String,
        title: String,
        last_message: String,
        notification_key: String,
        otp_code: Option<String>,
    },
    ImMessage {
        thread_id: String,
        package_name: String,
        title: String,
        msg_id: String,
        sender: String,
        text: String,
        notification_key: String,
        otp_code: Option<String>,
    },
    PhoneEntry {
        name: String,
        number: String,
    },
    Notification(String),
    App(String),
    ScheduleEvent(i64),
    AlarmItem(i64),
    OverviewCard(OverviewContextKind),
    BackScreenPreset(stepx_companion::backscreen::Preset),
    BackScreenRecentImage {
        index: usize,
        path: String,
    },
    BackScreenApp {
        label: String,
        package: String,
    },
    ToolCard(ToolPage),
    ToolFile {
        path: String,
        operation: Option<ToolOperation>,
    },
    Screenshot(String),
    ServiceRow {
        name: String,
        pkg: String,
        enabled: bool,
    },
    ConnectDevice {
        id: String,
        model: String,
        online: bool,
        selected: bool,
    },
    RememberedDevice {
        serial: String,
        model: String,
        is_preferred: bool,
    },
    CopyableText {
        title: String,
        text: String,
    },
    CalendarDate {
        year: i32,
        month: u32,
        day: u32,
    },
    BackScreenCanvas,
    FilesDirectory,
    HomeReminder(pages::home::HomeReminderContextTarget),
    HomeModule(pages::home::HomeModuleId),
    HomeCanvas,
}
pub(super) struct Menu {
    pub(super) target: Target,
    pub(super) at: Vector2F,
    pub(super) text: Option<String>,
    pub(super) selected: Option<usize>,
}
impl Companion {
    pub(super) fn open_workspace_intent_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(anchor) = ctx.element_position_by_id("workspace-intent-anchor".to_string()) else {
            return;
        };
        let Some(size) = ctx
            .windows()
            .platform_window(ctx.window_id())
            .map(|w| w.size())
        else {
            return;
        };
        let (x, y) = intent_menu_position(
            anchor.origin().x(),
            anchor.origin().y(),
            anchor.height(),
            size.x(),
            size.y(),
        );
        #[cfg(feature = "ui-capture")]
        if std::env::var_os("STEPX_CHAT_FIRST_TEST").is_some() {
            eprintln!(
                "MENU_GEOMETRY anchor={anchor:?} client={}x{} above={}",
                size.x(),
                size.y(),
                y < anchor.origin().y()
            );
        }
        use stepx_companion::amoo_workspace::Intent;
        let selected = [
            Some(Intent::Auto),
            Some(Intent::Chat),
            Some(Intent::Files),
            Some(Intent::App),
            Some(Intent::Image),
            Some(Intent::Screen),
            None,
        ]
        .iter()
        .position(|i| *i == Some(self.workspace.intent))
        .unwrap_or(0);
        self.context_menu = Some(Menu {
            target: Target::WorkspaceIntent(y < anchor.origin().y()),
            at: Vector2F::new(x, y),
            text: None,
            selected: Some(selected),
        });
        ctx.notify();
    }
    pub(super) fn handle_context(&mut self, action: &Action, ctx: &mut ViewContext<Self>) -> bool {
        match action {
            Action::ContextOpen(target, at) => {
                match target {
                    Target::Input(i) => {
                        self.focus = Some(*i);
                        self.selected_text = None;
                    }
                    Target::File(path) => {
                        self.selected_file = self.files.iter().position(|f| &f.path == path);
                        self.focus = None;
                    }
                    Target::GalleryItem { idx, .. } => {
                        self.back_screen.gallery_selected = *idx;
                        self.focus = None;
                    }
                    Target::ExpansionItem { id, .. } => {
                        self.expansion.selected_media_id = Some(id.clone());
                        self.focus = None;
                    }
                    Target::WorkspaceOutput(idx) => {
                        self.workspace.selected_output = Some(*idx);
                        self.focus = None;
                    }
                    Target::Notification(key) => {
                        self.push.selected_key = Some(key.clone());
                        self.focus = None;
                    }
                    Target::CalendarDate { year, month, day } => {
                        self.back_screen.calendar_year = *year;
                        self.back_screen.calendar_month = *month;
                        self.back_screen.selected_calendar_day = *day;
                        self.focus = None;
                    }
                    _ => {
                        self.focus = None;
                    }
                }
                let mut menu = Menu {
                    target: target.clone(),
                    at: *at,
                    text: self.selected_text.clone(),
                    selected: None,
                };
                let count = self.context_entries(&menu).len();
                let window_size = ctx
                    .windows()
                    .platform_window(ctx.window_id())
                    .map(|w| w.size());
                let menu_w = match &menu.target {
                    Target::WorkspaceIntent(_) => 280.,
                    Target::CalendarDate { .. } => 264.,
                    Target::HomeReminder(_) | Target::HomeModule(_) | Target::HomeCanvas => 296.,
                    _ => 248.,
                };
                menu.at = clamp_menu_position_with_width(*at, count, window_size, menu_w);
                self.context_menu = Some(menu);
                ctx.notify();
                true
            }
            Action::ContextClose | Action::Escape if self.context_menu.is_some() => {
                self.context_menu = None;
                ctx.notify();
                true
            }
            Action::ContextNavigate(direction) => {
                if let Some(menu) = &self.context_menu {
                    let entries = self.context_entries(menu);
                    let count = entries.len();
                    if count > 0 {
                        let mut i = menu
                            .selected
                            .map(|i| i as isize)
                            .unwrap_or(if *direction > 0 { -1 } else { count as isize });
                        for _ in 0..count {
                            i = (i + *direction as isize).rem_euclid(count as isize);
                            if entries[i as usize].action.is_some() {
                                break;
                            }
                        }
                        self.context_menu.as_mut().unwrap().selected = Some(i as usize);
                    }
                }
                ctx.notify();
                true
            }
            Action::ContextActivate => {
                let next = self.context_menu.as_ref().and_then(|m| {
                    m.selected.and_then(|i| {
                        self.context_entries(m)
                            .get(i)
                            .and_then(|e| e.action.clone())
                    })
                });
                if let Some(next) = next {
                    self.run_context_action(&next, ctx);
                } else {
                    self.context_menu = None;
                }
                ctx.notify();
                true
            }
            Action::ContextRun(next) => {
                self.run_context_action(next, ctx);
                ctx.notify();
                true
            }
            _ => {
                if matches!(
                    action,
                    Action::Copy
                        | Action::CopyInput
                        | Action::CutInput
                        | Action::Paste
                        | Action::SelectAll
                        | Action::Insert(_)
                        | Action::UndoInput
                        | Action::RedoInput
                        | Action::Page(_)
                        | Action::CompactChat
                ) {
                    self.context_menu = None;
                }
                false
            }
        }
    }
    fn run_context_action(&mut self, next: &Action, ctx: &mut ViewContext<Self>) {
        let mut next = next.clone();
        if let Some(Menu {
            target: Target::File(path),
            ..
        }) = &self.context_menu
        {
            if let Some(i) = self.files.iter().position(|f| &f.path == path) {
                self.selected_file = Some(i);
                next = match next {
                    Action::ActivateFile(_) => Action::ActivateFile(i),
                    Action::DownloadFile(_) => Action::DownloadFile(i),
                    a => a,
                };
            } else if !self.files.is_empty()
                && !matches!(
                    next,
                    Action::OpenNewFolder
                        | Action::OpenNewTextFile
                        | Action::Upload
                        | Action::Restore
                        | Action::CopyText(..)
                )
            {
                self.context_menu = None;
                self.status = "文件列表已变化，请重新选择文件".into();
                ctx.notify();
                return;
            }
        }
        self.context_menu = None;
        self.handle_action(&next, ctx);
    }
    pub(super) fn context_entries(&self, menu: &Menu) -> Vec<ContextMenuEntry> {
        let mut raw_entries: Vec<(String, String, Option<Action>)> = Vec::new();
        let command = if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "Ctrl"
        };
        let copy_key = format!("{command}+C");
        let paste_key = format!("{command}+V");
        let cut_key = format!("{command}+X");
        let all_key = format!("{command}+A");
        let mut push = |label: &str, key: &str, action: Option<Action>| {
            raw_entries.push((label.to_string(), key.to_string(), action));
        };
        match &menu.target {
            Target::WorkspaceIntent(_) => {
                use stepx_companion::amoo_workspace::Intent;
                push(
                    "自动识别（推荐）",
                    "",
                    Some(Action::Workspace(WorkspaceAction::Intent(Intent::Auto))),
                );
                push(
                    "仅聊天问答",
                    "",
                    Some(Action::Workspace(WorkspaceAction::Intent(Intent::Chat))),
                );
                push(
                    "处理文件（PPT / Word / 表格）",
                    "",
                    Some(Action::Workspace(WorkspaceAction::Intent(Intent::Files))),
                );
                push(
                    "制作应用 / 小游戏",
                    "",
                    Some(Action::Workspace(WorkspaceAction::Intent(Intent::App))),
                );
                push(
                    "生成图片 / 海报",
                    "",
                    Some(Action::Workspace(WorkspaceAction::Intent(Intent::Image))),
                );
                push(
                    "生成背屏图案",
                    "",
                    Some(Action::Workspace(WorkspaceAction::Intent(Intent::Screen))),
                );
                push(
                    "操作手机屏幕",
                    "",
                    Some(Action::AmooUi(AmooUiAction::GuiMode(true))),
                );
            }
            Target::Input(i) => {
                let f = &self.fields[*i];
                let is_secret = *i == F_LAB_KEY || *i == F_PAIR_CODE;
                let selected = f.selection_range().is_some_and(|r| !r.is_empty())
                    && (!is_secret || self.show_secret);
                push(
                    "撤销",
                    "",
                    (!f.undo.is_empty()).then_some(Action::UndoInput),
                );
                push(
                    "重做",
                    "",
                    (!f.redo.is_empty()).then_some(Action::RedoInput),
                );
                push("剪切", &cut_key, selected.then_some(Action::CutInput));
                push("复制", &copy_key, selected.then_some(Action::CopyInput));
                if !selected && !f.text.is_empty() && (!is_secret || self.show_secret) {
                    push(
                        "复制全部内容",
                        "",
                        Some(Action::CopyText(f.text.clone(), "已复制输入框内容".into())),
                    );
                }
                push("粘贴", &paste_key, Some(Action::Paste));
                push(
                    "全选",
                    &all_key,
                    (!f.text.is_empty()).then_some(Action::SelectAll),
                );
                if is_secret {
                    push(
                        if self.show_secret {
                            "隐藏输入内容"
                        } else {
                            "显示输入内容"
                        },
                        "",
                        Some(Action::ToggleShowSecret),
                    );
                }
                if !f.text.is_empty() {
                    push("清空输入", "", Some(Action::ClearField(*i)));
                }
            }
            Target::Selection => {
                let text = menu.text.clone().unwrap_or_default();
                push(
                    "复制",
                    &copy_key,
                    (!text.is_empty())
                        .then(|| Action::CopyText(text.clone(), "已复制所选文字".into())),
                );
                push(
                    "发给 Amoo 处理",
                    "",
                    (!text.is_empty()).then(|| Action::SendToAmoo(text.clone())),
                );
                push(
                    "朗读所选文字",
                    "",
                    (!text.is_empty()).then(|| Action::Speak("selected-text".into(), text)),
                );
            }
            Target::File(path) => {
                let in_trash = self.directory.contains("/.stepx-trash")
                    || self.directory.contains("/.StepXTrash")
                    || path.contains("/.stepx-trash")
                    || path.contains("/.StepXTrash");
                if in_trash {
                    push("从回收站还原", "", Some(Action::Restore));
                }
                if let Some((i, f)) = self.files.iter().enumerate().find(|(_, f)| &f.path == path) {
                    push(
                        if f.directory {
                            "打开文件夹"
                        } else {
                            "用系统默认程序打开"
                        },
                        "Enter",
                        Some(Action::ActivateFile(i)),
                    );
                    if !f.directory {
                        push("下载到电脑…", "", Some(Action::DownloadFile(i)));
                        push(
                            "发送到 Amoo 分析",
                            "",
                            Some(Action::Expansion(
                                expansion_state::ExpansionAction::AnalyzeWithAmoo(f.path.clone()),
                            )),
                        );
                        if f.kind_badge() == "图片" {
                            push(
                                "设为背屏像素画",
                                "",
                                Some(Action::FilePushToBackScreen(f.path.clone())),
                            );
                        }
                    }
                    push(
                        "复制手机路径",
                        "",
                        Some(Action::CopyText(
                            f.path.clone(),
                            "已复制手机文件路径".into(),
                        )),
                    );
                    push("重命名…", "F2", Some(Action::OpenRename));
                    push("移至回收站…", "Delete", Some(Action::Trash));
                } else {
                    push(
                        "复制手机路径",
                        "",
                        Some(Action::CopyText(path.clone(), "已复制手机文件路径".into())),
                    );
                    push("重命名…", "F2", Some(Action::OpenRename));
                    push("移至回收站…", "Delete", Some(Action::Trash));
                }
                push("在此目录新建文件夹…", "", Some(Action::OpenNewFolder));
                push("在此目录新建文本文件…", "", Some(Action::OpenNewTextFile));
                push("上传电脑文件到此目录…", "", Some(Action::Upload));
            }
            Target::FilesDirectory => {
                push("新建文件夹…", "", Some(Action::OpenNewFolder));
                push("新建文本文件…", "", Some(Action::OpenNewTextFile));
                push("上传电脑文件到此目录…", "", Some(Action::Upload));
                if self.trashed.is_some()
                    && (self.directory.contains("/.stepx-trash")
                        || self.directory.contains("/.StepXTrash"))
                {
                    push("从回收站还原最近项", "", Some(Action::Restore));
                }
                push(
                    "刷新当前目录",
                    "",
                    Some(Action::Browse(self.directory.clone())),
                );
                push(
                    "复制当前目录路径",
                    "",
                    Some(Action::CopyText(
                        self.directory.clone(),
                        "已复制当前手机目录路径".into(),
                    )),
                );
            }
            Target::LocalFile(path) => {
                let path_str = path.display().to_string();
                push(
                    "用系统默认程序打开",
                    "Enter",
                    Some(Action::Studio(StudioAction::OpenPath(path.clone()))),
                );
                push(
                    "在文件夹中显示",
                    "",
                    Some(Action::RevealLocalPath(path.clone())),
                );
                push(
                    "发给 Amoo 分析",
                    "",
                    Some(Action::AmooDropFiles(vec![path.clone()])),
                );
                push(
                    "复制本地文件路径",
                    "",
                    Some(Action::CopyText(path_str, "已复制本地文件路径".into())),
                );
            }
            Target::LocalDirectory(path_str) => {
                let path = PathBuf::from(path_str);
                push(
                    "在系统文件管理器中打开",
                    "",
                    Some(Action::Studio(StudioAction::OpenPath(path.clone()))),
                );
                push(
                    "复制目录路径",
                    "",
                    Some(Action::CopyText(path_str.clone(), "已复制目录路径".into())),
                );
            }
            Target::GalleryItem { remote_path, .. } => {
                push(
                    "全屏沉浸看图",
                    "Space",
                    Some(Action::BackScreen(BackScreenAction::GalleryToggleLightbox)),
                );
                push(
                    "用系统看图软件打开",
                    "Enter",
                    Some(Action::BackScreen(BackScreenAction::GalleryOpenOnDesktop(
                        remote_path.clone(),
                    ))),
                );
                push(
                    "保存到电脑…",
                    "",
                    Some(Action::BackScreen(BackScreenAction::GallerySaveToDesktop(
                        remote_path.clone(),
                    ))),
                );
                push(
                    "顺时针旋转 90°",
                    "",
                    Some(Action::BackScreen(BackScreenAction::GalleryRotateRight(
                        remote_path.clone(),
                    ))),
                );
                push(
                    "发送给 Amoo 分析",
                    "",
                    Some(Action::BackScreen(BackScreenAction::GallerySendToAmoo(
                        remote_path.clone(),
                    ))),
                );
                push(
                    "设为背屏像素画",
                    "",
                    Some(Action::FilePushToBackScreen(remote_path.clone())),
                );
                push(
                    "复制手机路径",
                    "",
                    Some(Action::BackScreen(BackScreenAction::GalleryCopyPath(
                        remote_path.clone(),
                    ))),
                );
                push(
                    "删除照片",
                    "Delete",
                    Some(Action::BackScreen(BackScreenAction::GalleryDelete(
                        remote_path.clone(),
                    ))),
                );
            }
            Target::ExpansionItem {
                id,
                local_path,
                phone_path,
                is_archived,
            } => {
                push(
                    "用系统程序打开",
                    "Enter",
                    Some(Action::Expansion(expansion_state::ExpansionAction::Open(
                        id.clone(),
                    ))),
                );
                push(
                    "在文件夹中显示",
                    "",
                    Some(Action::Expansion(
                        expansion_state::ExpansionAction::RevealInFolder(id.clone()),
                    )),
                );
                if *is_archived {
                    push(
                        "还原回手机",
                        "",
                        Some(Action::Expansion(
                            expansion_state::ExpansionAction::Restore(id.clone()),
                        )),
                    );
                }
                push(
                    "发送给 Amoo 分析",
                    "",
                    Some(Action::Expansion(
                        expansion_state::ExpansionAction::AnalyzeWithAmoo(id.clone()),
                    )),
                );
                if let Some(lp) = local_path {
                    push(
                        "复制电脑本地路径",
                        "",
                        Some(Action::CopyText(
                            lp.display().to_string(),
                            "已复制电脑本地文件路径".into(),
                        )),
                    );
                }
                if !phone_path.is_empty() {
                    push(
                        "复制原手机路径",
                        "",
                        Some(Action::CopyText(
                            phone_path.clone(),
                            "已复制原手机路径".into(),
                        )),
                    );
                }
            }
            Target::WorkspaceOutput(idx) => {
                let output = self
                    .workspace_task()
                    .and_then(|t| t.outputs.get(*idx))
                    .cloned();
                push(
                    "用系统默认程序打开",
                    "Enter",
                    Some(Action::Workspace(WorkspaceAction::OpenOutput(*idx))),
                );
                push(
                    "在右侧预览",
                    "",
                    Some(Action::Workspace(WorkspaceAction::Preview(*idx))),
                );
                push(
                    "在文件夹中显示",
                    "",
                    Some(Action::Workspace(WorkspaceAction::RevealOutput(*idx))),
                );
                push(
                    "继续让 Amoo 修改",
                    "",
                    Some(Action::Workspace(WorkspaceAction::ContinueEdit(*idx))),
                );
                push(
                    "发送到手机",
                    "",
                    Some(Action::Workspace(WorkspaceAction::SendOutputToPhone(*idx))),
                );
                if let Some(out) = output {
                    push(
                        "复制文件路径",
                        "",
                        Some(Action::CopyText(
                            out.path.display().to_string(),
                            "已复制文件路径".into(),
                        )),
                    );
                }
            }
            Target::ChatAttachment {
                path,
                url,
                remove_idx,
            } => {
                if !path.is_empty() {
                    push(
                        "打开文件",
                        "",
                        Some(Action::Work(WorkAction::OpenPreview(path.clone()))),
                    );
                    push(
                        "在文件夹中显示",
                        "",
                        Some(Action::RevealLocalPath(PathBuf::from(path))),
                    );
                    push(
                        "复制文件路径",
                        "",
                        Some(Action::CopyText(path.clone(), "已复制文件路径".into())),
                    );
                } else if !url.is_empty() {
                    push(
                        "下载并打开附件",
                        "",
                        Some(Action::StudioOpenUrl(url.clone())),
                    );
                    push(
                        "复制附件链接",
                        "",
                        Some(Action::CopyText(url.clone(), "已复制附件链接".into())),
                    );
                }
                if let Some(idx) = remove_idx {
                    push("移除此附件", "", Some(Action::AmooRemoveAttachment(*idx)));
                }
            }
            Target::ChatMessage {
                assistant,
                text,
                session_key,
                retry_text,
            } => {
                if let Some(sel) = menu.text.as_ref().filter(|s| !s.is_empty()) {
                    push(
                        "复制选中文字",
                        &copy_key,
                        Some(Action::CopyText(sel.clone(), "已复制所选文字".into())),
                    );
                }
                push(
                    "复制整条消息",
                    "",
                    (!text.is_empty())
                        .then(|| Action::CopyText(text.clone(), "已复制消息内容".into())),
                );
                push(
                    "引用并继续提问",
                    "",
                    (!text.is_empty())
                        .then(|| Action::Workspace(WorkspaceAction::QuoteTurn(text.clone()))),
                );
                if *assistant {
                    if let (Some(sk), Some(rt)) = (session_key, retry_text) {
                        push(
                            "重新生成回复",
                            "",
                            Some(Action::AmooRetryMessage {
                                session_key: sk.clone(),
                                text: rt.clone(),
                            }),
                        );
                    } else {
                        push(
                            "重新生成回复",
                            "",
                            Some(Action::Workspace(WorkspaceAction::Retry)),
                        );
                    }
                    push(
                        "朗读此回复",
                        "",
                        (!text.is_empty()).then(|| Action::Speak("chat-msg".into(), text.clone())),
                    );
                } else {
                    push(
                        "编辑并重新发送",
                        "",
                        (!text.is_empty()).then(|| Action::EditUserMessage(text.clone())),
                    );
                }
                let first_line = text
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or(text.trim());
                let snippet: String = first_line.chars().take(20).collect();
                push(
                    "转为日历日程提醒…",
                    "",
                    (!snippet.is_empty()).then(|| {
                        Action::BackScreen(BackScreenAction::CalendarPrefillFromText {
                            title: snippet,
                            time_hint: "09:30-10:00".into(),
                        })
                    }),
                );
            }
            Target::ChatSession {
                workspace_task_id,
                phone_session_idx,
                title,
            } => {
                if let Some(tid) = workspace_task_id {
                    let is_pinned = self
                        .workspace
                        .journal
                        .tasks
                        .iter()
                        .find(|t| &t.id == tid)
                        .is_some_and(|t| t.pinned);
                    push(
                        "切换至此对话",
                        "Enter",
                        Some(Action::Workspace(WorkspaceAction::Select(tid.clone()))),
                    );
                    push(
                        if is_pinned {
                            "取消置顶此对话"
                        } else {
                            "置顶此对话"
                        },
                        "",
                        Some(Action::Workspace(WorkspaceAction::TogglePin(tid.clone()))),
                    );
                    push(
                        "复制全部对话内容",
                        "",
                        Some(Action::Workspace(WorkspaceAction::CopyTaskTranscript(
                            tid.clone(),
                        ))),
                    );
                    push(
                        "复制对话标题",
                        "",
                        Some(Action::CopyText(title.clone(), "已复制对话标题".into())),
                    );
                    push(
                        "删除此对话",
                        "Delete",
                        Some(Action::Workspace(WorkspaceAction::Delete(tid.clone()))),
                    );
                } else if let Some(idx) = phone_session_idx {
                    let sess = self.amoo_sessions.get(*idx);
                    let is_pinned = sess.is_some_and(|s| {
                        self.workspace
                            .journal
                            .pinned_sessions
                            .contains(&s.session_key)
                    });
                    push(
                        "切换至此对话",
                        "Enter",
                        Some(Action::Workspace(WorkspaceAction::PhoneSession(*idx))),
                    );
                    if let Some(s) = sess {
                        push(
                            if is_pinned {
                                "取消置顶此对话"
                            } else {
                                "置顶此对话"
                            },
                            "",
                            Some(Action::Workspace(WorkspaceAction::TogglePin(format!(
                                "phone:{}",
                                s.session_key
                            )))),
                        );
                        push(
                            "复制全部对话内容",
                            "",
                            Some(Action::Workspace(WorkspaceAction::CopyTaskTranscript(
                                format!("phone:{}", s.session_key),
                            ))),
                        );
                    } else {
                        push(
                            "复制全部对话内容",
                            "",
                            Some(Action::CopyText(
                                title.clone(),
                                "已复制全部对话内容".into(),
                            )),
                        );
                    }
                    push(
                        "复制对话标题",
                        "",
                        Some(Action::CopyText(title.clone(), "已复制对话标题".into())),
                    );
                    if let Some(s) = sess {
                        push(
                            "删除此对话",
                            "Delete",
                            Some(Action::Workspace(WorkspaceAction::Delete(format!(
                                "phone:{}",
                                s.session_key
                            )))),
                        );
                    }
                }
            }
            Target::StudioWork(target) => match target {
                StudioWorkTarget::WorkspaceOutput {
                    task_id,
                    output_idx,
                    path,
                    title,
                    ..
                } => {
                    push(
                        "用系统默认程序打开",
                        "Enter",
                        Some(Action::Studio(StudioAction::OpenPath(path.clone()))),
                    );
                    push(
                        "在文件夹中显示",
                        "",
                        Some(Action::Studio(StudioAction::RevealPath(path.clone()))),
                    );
                    push(
                        "在 Amoo 中继续修改",
                        "",
                        Some(Action::Studio(StudioAction::ContinueOutputInAmoo(
                            task_id.clone(),
                            *output_idx,
                        ))),
                    );
                    push(
                        "发送到手机",
                        "",
                        Some(Action::Workspace(WorkspaceAction::SendTaskOutputToPhone(
                            task_id.clone(),
                            *output_idx,
                        ))),
                    );
                    push(
                        "查看所在对话",
                        "",
                        Some(Action::Studio(StudioAction::BackToTask(task_id.clone()))),
                    );
                    push(
                        "复制文件路径",
                        "",
                        Some(Action::CopyText(
                            path.display().to_string(),
                            format!("已复制「{title}」路径"),
                        )),
                    );
                }
                StudioWorkTarget::LocalArtifact {
                    round_idx,
                    path,
                    title,
                } => {
                    push(
                        "打开交互预览",
                        "Enter",
                        Some(Action::StudioLab(StudioLabAction::PreviewAt(*round_idx))),
                    );
                    push(
                        "用默认浏览器打开",
                        "",
                        Some(Action::Studio(StudioAction::OpenPath(path.clone()))),
                    );
                    push(
                        "在文件夹中显示",
                        "",
                        Some(Action::Studio(StudioAction::RevealPath(path.clone()))),
                    );
                    push(
                        "在 Amoo 中继续修改",
                        "",
                        Some(Action::Studio(StudioAction::ContinueLabInAmoo(*round_idx))),
                    );
                    push(
                        "发送到手机",
                        "",
                        Some(Action::StudioLab(StudioLabAction::DeliverPhoneAt(
                            *round_idx,
                        ))),
                    );
                    push(
                        "复制文件路径",
                        "",
                        Some(Action::CopyText(
                            path.display().to_string(),
                            format!("已复制「{title}」路径"),
                        )),
                    );
                    push(
                        "删除此作品",
                        "",
                        Some(Action::StudioLab(StudioLabAction::DeleteArtifactAt(
                            *round_idx,
                        ))),
                    );
                }
                StudioWorkTarget::CloudProject {
                    source,
                    idx,
                    url,
                    title,
                } => {
                    if !url.is_empty() {
                        push(
                            "在浏览器中打开",
                            "Enter",
                            Some(Action::StudioOpenUrl(url.clone())),
                        );
                    }
                    push(
                        "在手机上打开",
                        "",
                        Some(Action::Studio(StudioAction::UsePhone(*idx))),
                    );
                    push(
                        "加入我的收藏",
                        "",
                        Some(Action::Studio(if *source == 0 {
                            StudioAction::KeepWeb(*idx)
                        } else {
                            StudioAction::KeepPhone(*idx)
                        })),
                    );
                    if !url.is_empty() {
                        push(
                            "复制作品链接",
                            "",
                            Some(Action::CopyText(
                                url.clone(),
                                format!("已复制「{title}」链接"),
                            )),
                        );
                    }
                }
                StudioWorkTarget::SavedWork { idx, url, title } => {
                    if !url.is_empty() {
                        push(
                            "在浏览器中打开",
                            "Enter",
                            Some(Action::StudioOpenUrl(url.clone())),
                        );
                    }
                    push(
                        "在手机上打开",
                        "",
                        Some(Action::Studio(StudioAction::UseSavedOnPhone(*idx))),
                    );
                    if !url.is_empty() {
                        push(
                            "复制作品链接",
                            "",
                            Some(Action::CopyText(
                                url.clone(),
                                format!("已复制「{title}」链接"),
                            )),
                        );
                    }
                    push(
                        "移除收藏",
                        "",
                        Some(Action::Studio(StudioAction::DeleteSaved(*idx))),
                    );
                }
            },
            Target::SmsThread {
                address,
                name,
                last_body,
                otp_code,
            } => {
                let norm = stepx_companion::sms::normalize_address(address);
                let addr_key = if norm.is_empty() {
                    address.trim().to_string()
                } else {
                    norm
                };
                let pin_key =
                    stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(&addr_key);
                let is_pinned = self.push.prefs.is_pinned(&pin_key);
                let is_unread = self.push.prefs.manual_unread_targets.contains(&pin_key)
                    || (!self.read_sms_addresses.contains(&addr_key)
                        && self.sms.iter().any(|m| {
                            stepx_companion::sms::normalize_address(&m.address) == addr_key
                                && m.kind == 1
                                && !m.read
                        }));
                push(
                    if is_pinned { "取消置顶" } else { "置顶此对话" },
                    "",
                    (!addr_key.is_empty())
                        .then(|| Action::Push(PushAction::TogglePinTarget(pin_key))),
                );
                push(
                    if is_unread { "标为已读" } else { "标为未读" },
                    "",
                    (!address.is_empty()).then(|| {
                        Action::Push(PushAction::ToggleThreadRead {
                            is_im: false,
                            target_id: address.clone(),
                        })
                    }),
                );
                if let Some(code) = otp_code {
                    push(
                        &format!("复制验证码 / 取件码：{code}"),
                        "",
                        Some(Action::CopyText(
                            code.clone(),
                            format!("已复制验证码 {code}"),
                        )),
                    );
                }
                if !last_body.is_empty() {
                    push(
                        "复制最新短信全文",
                        "",
                        Some(Action::CopyText(last_body.clone(), "已复制短信全文".into())),
                    );
                }
                push(
                    "复制号码",
                    "",
                    (!address.is_empty()).then(|| {
                        Action::CopyText(address.clone(), format!("已复制号码 {address}"))
                    }),
                );
                if !name.is_empty() && name != address {
                    push(
                        "复制联系人姓名",
                        "",
                        Some(Action::CopyText(
                            name.clone(),
                            format!("已复制联系人 {name}"),
                        )),
                    );
                }
                push(
                    "回拨此号码",
                    "",
                    (!address.is_empty()).then(|| Action::PhoneDialNumber(address.clone())),
                );
                if !last_body.is_empty() {
                    let sender_label = if name.is_empty() { address } else { name };
                    let snippet: String = last_body.chars().take(18).collect();
                    push(
                        "转为日历日程提醒…",
                        "",
                        Some(Action::BackScreen(
                            BackScreenAction::CalendarPrefillFromText {
                                title: format!("短信待办（{sender_label}）：{snippet}"),
                                time_hint: "09:30-10:00".into(),
                            },
                        )),
                    );
                    push(
                        "发给 Amoo 处理",
                        "",
                        Some(Action::SendToAmoo(format!(
                            "请帮我处理来自「{name}」的短信：\n{last_body}"
                        ))),
                    );
                }
                push(
                    "清空短信记录",
                    "",
                    (!address.is_empty()).then(|| {
                        Action::Push(PushAction::ClearThreadMessages {
                            is_im: false,
                            target_id: address.clone(),
                            package_name: "com.android.mms".into(),
                            title: if name.is_empty() {
                                address.clone()
                            } else {
                                name.clone()
                            },
                        })
                    }),
                );
                push(
                    "删除此对话",
                    "Del",
                    (!address.is_empty()).then(|| {
                        Action::Push(PushAction::DeleteSmsThread {
                            address: address.clone(),
                            name: name.clone(),
                        })
                    }),
                );
            }
            Target::SmsMessage {
                address,
                sender,
                body,
                date_ms,
                otp_code,
            } => {
                if let Some(code) = otp_code {
                    push(
                        &format!("复制验证码 / 取件码：{code}"),
                        "",
                        Some(Action::CopyText(
                            code.clone(),
                            format!("已复制验证码 {code}"),
                        )),
                    );
                }
                push(
                    "复制短信全文",
                    &copy_key,
                    (!body.is_empty())
                        .then(|| Action::CopyText(body.clone(), "已复制短信全文".into())),
                );
                push(
                    "引用回复",
                    "",
                    (!body.is_empty()).then(|| {
                        Action::Push(PushAction::QuoteReply {
                            is_im: false,
                            sender: address.clone(),
                            text: body.clone(),
                        })
                    }),
                );
                push(
                    "复制发件号码",
                    "",
                    (!address.is_empty()).then(|| {
                        Action::CopyText(address.clone(), format!("已复制号码 {address}"))
                    }),
                );
                push(
                    "回拨电话",
                    "",
                    (!address.is_empty()).then(|| Action::PhoneDialNumber(address.clone())),
                );
                let sender_label = if sender.is_empty() { address } else { sender };
                let sms_snippet: String = body.chars().take(18).collect();
                push(
                    "转为日历日程提醒…",
                    "",
                    (!body.is_empty()).then(|| {
                        Action::BackScreen(BackScreenAction::CalendarPrefillFromText {
                            title: format!("短信待办（{sender_label}）：{sms_snippet}"),
                            time_hint: "09:30-10:00".into(),
                        })
                    }),
                );
                push(
                    "发给 Amoo 处理",
                    "",
                    (!body.is_empty()).then(|| {
                        Action::SendToAmoo(format!("请帮我处理来自「{sender}」的短信：\n{body}"))
                    }),
                );
                push(
                    "删除此条短信",
                    "Del",
                    (!body.is_empty() || *date_ms > 0).then(|| {
                        Action::Push(PushAction::DeleteSmsMessage {
                            address: address.clone(),
                            date_ms: *date_ms,
                            body: body.clone(),
                        })
                    }),
                );
            }
            Target::ImThread {
                thread_id,
                package_name,
                title,
                last_message,
                notification_key,
                otp_code,
            } => {
                let pin_key =
                    stepx_companion::desktop_notify::NotifyPreferences::im_target_key(thread_id);
                let is_pinned = self.push.prefs.is_pinned(&pin_key);
                let is_unread = self
                    .push
                    .snapshot
                    .threads
                    .iter()
                    .find(|t| &t.thread_id == thread_id)
                    .is_some_and(|t| t.unread_count > 0);
                push(
                    if is_pinned { "取消置顶" } else { "置顶此对话" },
                    "",
                    (!thread_id.is_empty())
                        .then(|| Action::Push(PushAction::TogglePinTarget(pin_key))),
                );
                push(
                    if is_unread { "标为已读" } else { "标为未读" },
                    "",
                    (!thread_id.is_empty()).then(|| {
                        Action::Push(PushAction::ToggleThreadRead {
                            is_im: true,
                            target_id: thread_id.clone(),
                        })
                    }),
                );
                if let Some(code) = otp_code {
                    push(
                        &format!("复制验证码 / 取件码：{code}"),
                        "",
                        Some(Action::CopyText(
                            code.clone(),
                            format!("已复制验证码 {code}"),
                        )),
                    );
                }
                if !notification_key.is_empty() {
                    push(
                        "在手机上打开会话",
                        "",
                        Some(Action::Push(PushAction::OpenOnPhone(
                            notification_key.clone(),
                        ))),
                    );
                }
                if !last_message.is_empty() {
                    push(
                        "复制最新消息",
                        "",
                        Some(Action::CopyText(
                            last_message.clone(),
                            "已复制消息内容".into(),
                        )),
                    );
                    let snippet: String = last_message.chars().take(18).collect();
                    push(
                        "转为日历日程提醒…",
                        "",
                        Some(Action::BackScreen(
                            BackScreenAction::CalendarPrefillFromText {
                                title: format!("{title}：{snippet}"),
                                time_hint: "09:30-10:00".into(),
                            },
                        )),
                    );
                    push(
                        "发给 Amoo 处理",
                        "",
                        Some(Action::SendToAmoo(format!(
                            "请帮我处理或回复来自「{title}」的消息：\n{last_message}"
                        ))),
                    );
                }
                push(
                    "清空聊天记录",
                    "",
                    (!thread_id.is_empty()).then(|| {
                        Action::Push(PushAction::ClearThreadMessages {
                            is_im: true,
                            target_id: thread_id.clone(),
                            package_name: package_name.clone(),
                            title: title.clone(),
                        })
                    }),
                );
                push(
                    "删除此对话",
                    "Del",
                    (!thread_id.is_empty()).then(|| {
                        Action::Push(PushAction::DeleteImThread {
                            thread_id: thread_id.clone(),
                            package_name: package_name.clone(),
                            title: title.clone(),
                        })
                    }),
                );
            }
            Target::ImMessage {
                thread_id,
                package_name,
                title,
                msg_id,
                sender,
                text,
                notification_key,
                otp_code,
            } => {
                if let Some(code) = otp_code {
                    push(
                        &format!("复制验证码 / 取件码：{code}"),
                        "",
                        Some(Action::CopyText(
                            code.clone(),
                            format!("已复制验证码 {code}"),
                        )),
                    );
                }
                push(
                    "复制消息全文",
                    &copy_key,
                    (!text.is_empty())
                        .then(|| Action::CopyText(text.clone(), "已复制消息全文".into())),
                );
                push(
                    "引用回复",
                    "",
                    (!text.is_empty()).then(|| {
                        Action::Push(PushAction::QuoteReply {
                            is_im: true,
                            sender: sender.clone(),
                            text: text.clone(),
                        })
                    }),
                );
                if !notification_key.is_empty() {
                    push(
                        "在手机上打开应用",
                        "",
                        Some(Action::Push(PushAction::OpenOnPhone(
                            notification_key.clone(),
                        ))),
                    );
                }
                let im_snippet: String = text.chars().take(18).collect();
                push(
                    "转为日历日程提醒…",
                    "",
                    (!text.is_empty()).then(|| {
                        Action::BackScreen(BackScreenAction::CalendarPrefillFromText {
                            title: format!("{title} · {sender}：{im_snippet}"),
                            time_hint: "09:30-10:00".into(),
                        })
                    }),
                );
                push(
                    "发给 Amoo 处理",
                    "",
                    (!text.is_empty()).then(|| {
                        Action::SendToAmoo(format!(
                            "请帮我处理或回复「{title} · {sender}」的消息：\n{text}"
                        ))
                    }),
                );
                push(
                    "删除此条消息",
                    "Del",
                    (!msg_id.is_empty() || !text.is_empty()).then(|| {
                        Action::Push(PushAction::DeleteMessage {
                            thread_id: thread_id.clone(),
                            package_name: package_name.clone(),
                            msg_id: msg_id.clone(),
                            text: text.clone(),
                        })
                    }),
                );
            }
            Target::PhoneEntry { name, number } => {
                push(
                    "立即拨打此号码",
                    "Enter",
                    (!number.is_empty()).then(|| Action::PhoneCall(number.clone())),
                );
                push(
                    "填入拨号盘",
                    "",
                    (!number.is_empty()).then(|| Action::PhoneDialNumber(number.clone())),
                );
                push(
                    "发送短信",
                    "",
                    (!number.is_empty()).then(|| Action::PhoneSmsTo(number.clone())),
                );
                push(
                    "添加回电日程提醒…",
                    "",
                    (!number.is_empty()).then(|| {
                        Action::BackScreen(BackScreenAction::CalendarPrefillFromText {
                            title: format!(
                                "回电给 {} ({})",
                                if name.is_empty() { number } else { name },
                                number
                            ),
                            time_hint: "10:00-10:30".into(),
                        })
                    }),
                );
                push(
                    "复制电话号码",
                    "",
                    (!number.is_empty()).then(|| {
                        Action::CopyText(number.clone(), format!("已复制电话号码 {number}"))
                    }),
                );
                if !name.is_empty() && name != number && name != "隐藏号码" {
                    push(
                        "复制联系人姓名",
                        "",
                        Some(Action::CopyText(
                            name.clone(),
                            format!("已复制联系人姓名 {name}"),
                        )),
                    );
                }
            }
            Target::Notification(key) => {
                let notif = self
                    .push
                    .snapshot
                    .notifications
                    .iter()
                    .find(|n| &n.key == key)
                    .cloned();
                if let Some(n) = notif {
                    let combined = format!("{} {}", n.title, n.text);
                    if let Some(code) = super::pages::sms::extract_verification_code(&combined) {
                        push(
                            &format!("复制验证码 / 取件码：{code}"),
                            "",
                            Some(Action::Push(PushAction::CopyOtp(code))),
                        );
                    }
                    push(
                        "在手机上打开应用",
                        "Enter",
                        Some(Action::Push(PushAction::OpenOnPhone(n.key.clone()))),
                    );
                    push(
                        "复制通知内容",
                        "",
                        Some(Action::CopyText(
                            if n.title.is_empty() {
                                n.text.clone()
                            } else {
                                format!("{}：{}", n.title, n.text)
                            },
                            "已复制通知内容".into(),
                        )),
                    );
                    let raw_title = if n.title.is_empty() {
                        format!("{}：{}", n.app_name, n.text)
                    } else {
                        format!("{}：{}", n.title, n.text)
                    };
                    let notif_snippet: String = raw_title.chars().take(22).collect();
                    push(
                        "转为日历日程提醒…",
                        "",
                        Some(Action::BackScreen(
                            BackScreenAction::CalendarPrefillFromText {
                                title: notif_snippet,
                                time_hint: "09:30-10:00".into(),
                            },
                        )),
                    );
                    push(
                        "让 Amoo 帮我处理",
                        "",
                        Some(Action::SendToAmoo(format!(
                            "请帮我处理或回复来自「{}」的通知（{}）：\n{}",
                            n.app_name, n.title, n.text
                        ))),
                    );
                    let app_label = if n.app_name.trim().is_empty() {
                        stepx_companion::push::ImChannel::from_package(&n.package_name)
                            .label()
                            .to_string()
                    } else {
                        n.app_name.clone()
                    };
                    push(
                        "只看该应用通知",
                        "",
                        Some(Action::Push(PushAction::SelectAppFilter(Some(app_label)))),
                    );
                    push(
                        "清除此通知",
                        "Delete",
                        Some(Action::Push(PushAction::DismissNotification(n.key.clone()))),
                    );
                }
            }
            Target::App(pkg) => {
                if let Some(app) = self.apps.iter().find(|a| &a.package == pkg) {
                    push(
                        "查看应用详情与权限",
                        "Enter",
                        Some(Action::InspectApp(Some(app.package.clone()))),
                    );
                    push(
                        "投屏到电脑打开",
                        "",
                        (app.launchable && app.enabled)
                            .then(|| Action::OpenAppInMirror(app.package.clone())),
                    );
                    push(
                        "在手机上打开应用",
                        "",
                        (app.launchable && app.enabled)
                            .then(|| Action::OpenApp(app.package.clone())),
                    );
                    push(
                        "强行停止应用",
                        "",
                        app.enabled
                            .then(|| Action::ForceStopApp(app.package.clone())),
                    );
                    push(
                        "导出安装包 (APK)…",
                        "",
                        Some(Action::ExportApp(app.package.clone())),
                    );
                    push(
                        "在手机系统设置中打开",
                        "",
                        Some(Action::AppSettings(app.package.clone())),
                    );
                    push(
                        "复制应用包名",
                        "",
                        Some(Action::CopyText(
                            app.package.clone(),
                            format!("已复制应用包名 {}", app.package),
                        )),
                    );
                    if app.can_uninstall() {
                        push(
                            "卸载此应用…",
                            "",
                            Some(Action::RequestUninstallApp(app.package.clone())),
                        );
                    }
                }
            }
            Target::ScheduleEvent(event_id) => {
                let done = self
                    .back_screen
                    .completed_calendar_events
                    .contains(event_id);
                let now_ms = chrono::Local::now().timestamp_millis();
                let cal_key = format!("cal:{event_id}");
                let sched_key = format!("schedule:{event_id}");
                let is_suppressed = self.reminder_governance.is_any_key_suppressed(
                    &[cal_key.clone(), sched_key.clone()],
                    None,
                    None,
                    None,
                    now_ms,
                );
                let ev = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .find(|e| e.id == *event_id);
                let event_title_or_default = ev
                    .map(|e| e.title.clone())
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| format!("日程 #{event_id}"));
                push(
                    "编辑日程…",
                    "Enter",
                    Some(Action::BackScreen(BackScreenAction::CalendarEditEvent(
                        *event_id,
                    ))),
                );
                push(
                    if done {
                        "设为未完成"
                    } else {
                        "标记为已完成"
                    },
                    "",
                    Some(Action::BackScreen(BackScreenAction::CalendarToggleDone(
                        *event_id,
                    ))),
                );
                push(
                    if is_suppressed {
                        "恢复有效提醒"
                    } else {
                        "设为过期 · 不再提醒"
                    },
                    "",
                    Some(if is_suppressed {
                        let restore_key = if self.reminder_governance.is_key_suppressed(
                            &sched_key,
                            None,
                            None,
                            now_ms,
                        ) {
                            sched_key
                        } else {
                            cal_key
                        };
                        Action::HomeReminder(pages::home::HomeReminderAction::RestoreReminder(
                            restore_key,
                        ))
                    } else {
                        let spec =
                            stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                                cal_key,
                                stepx_companion::home_reminder_governance::ReminderCategory::ScheduleAndMeeting,
                                event_title_or_default,
                            )
                            .with_aliases([
                                sched_key,
                                format!("conflict:conflict-cal-{event_id}"),
                            ]);
                        Action::HomeReminder(pages::home::HomeReminderAction::ManualExpire(
                            pages::home::HomeReminderContextTarget {
                                spec,
                                ..Default::default()
                            },
                        ))
                    }),
                );
                push(
                    "顺延至明天同一时间 (+1天)",
                    "",
                    Some(Action::BackScreen(
                        BackScreenAction::CalendarPostponeOneDay(*event_id),
                    )),
                );
                if let Some(e) = ev {
                    let summary = format!("{}（{}）", e.title, e.hm_label());
                    push(
                        "复制日程时间与标题",
                        &copy_key,
                        Some(Action::CopyText(summary, "已复制日程信息".into())),
                    );
                }
                push(
                    "删除此日程",
                    "Delete",
                    Some(Action::BackScreen(BackScreenAction::CalendarDelete(
                        *event_id,
                    ))),
                );
            }
            Target::AlarmItem(alarm_id) => {
                let now_ms = chrono::Local::now().timestamp_millis();
                let alarm_key = format!("alarm:{alarm_id}");
                let cap_key = format!("capsule:alarm-{alarm_id}");
                let is_suppressed = self.reminder_governance.is_any_key_suppressed(
                    &[alarm_key.clone(), cap_key.clone()],
                    None,
                    None,
                    None,
                    now_ms,
                );
                let alarm = self
                    .back_screen
                    .stepos_suite
                    .alarms
                    .iter()
                    .find(|a| a.id == *alarm_id);
                if let Some(alarm) = alarm {
                    push(
                        if alarm.enabled {
                            "关闭此闹钟"
                        } else {
                            "开启此闹钟"
                        },
                        "",
                        Some(Action::BackScreen(BackScreenAction::AlarmToggle(
                            *alarm_id,
                            !alarm.enabled,
                        ))),
                    );
                }
                push(
                    "编辑闹钟时间与标签…",
                    "Enter",
                    Some(Action::BackScreen(BackScreenAction::AlarmEdit(*alarm_id))),
                );
                push(
                    "推迟 10 分钟响铃 (+10m)",
                    "",
                    Some(Action::BackScreen(BackScreenAction::AlarmShiftMinutes(
                        *alarm_id, 10,
                    ))),
                );
                push(
                    if is_suppressed {
                        "恢复今日闹钟提醒"
                    } else {
                        "今日不再提醒（设为过期）"
                    },
                    "",
                    Some(if is_suppressed {
                        let restore_key = if self.reminder_governance.is_key_suppressed(
                            &cap_key,
                            None,
                            None,
                            now_ms,
                        ) {
                            cap_key
                        } else {
                            alarm_key
                        };
                        Action::HomeReminder(pages::home::HomeReminderAction::RestoreReminder(
                            restore_key,
                        ))
                    } else {
                        let label = alarm
                            .map(|a| format!("{} {}", a.time_label(), a.display_title()))
                            .unwrap_or_else(|| format!("闹钟 #{alarm_id}"));
                        let spec =
                            stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                                alarm_key,
                                stepx_companion::home_reminder_governance::ReminderCategory::LiveTimer,
                                label,
                            )
                            .with_aliases([cap_key]);
                        Action::HomeReminder(pages::home::HomeReminderAction::ManualExpire(
                            pages::home::HomeReminderContextTarget {
                                spec,
                                capsule_id: format!("alarm-{alarm_id}"),
                                ..Default::default()
                            },
                        ))
                    }),
                );
                if let Some(alarm) = alarm {
                    push(
                        "复制闹钟时间与标签",
                        &copy_key,
                        Some(Action::CopyText(
                            format!("{} {}", alarm.time_label(), alarm.display_title()),
                            "已复制闹钟信息".into(),
                        )),
                    );
                }
                push(
                    "删除此闹钟",
                    "Delete",
                    Some(Action::BackScreen(BackScreenAction::AlarmDelete(*alarm_id))),
                );
            }
            Target::OverviewCard(kind) => match kind {
                OverviewContextKind::Device => {
                    if let Some(d) = &self.selected {
                        push(
                            "复制设备序列号",
                            "",
                            Some(Action::CopyText(
                                d.serial.clone(),
                                "已复制设备序列号".into(),
                            )),
                        );
                    }
                    push("刷新设备状态", "", Some(Action::Summary));
                    push(
                        "打开设备检测",
                        "",
                        Some(Action::OpenTool(Some(ToolPage::Device))),
                    );
                    push("切换 / 连接设备", "", Some(Action::Page(Page::Connect)));
                }
                OverviewContextKind::Battery => {
                    push("刷新电池与温度读数", "", Some(Action::Summary));
                    push(
                        "打开设备健康检测",
                        "",
                        Some(Action::OpenTool(Some(ToolPage::Device))),
                    );
                }
                OverviewContextKind::Storage => {
                    push("打开手机文件管理", "", Some(Action::Page(Page::Files)));
                    push(
                        "打开存储空间清理",
                        "",
                        Some(Action::OpenTool(Some(ToolPage::Storage))),
                    );
                    push("打开电脑扩容归档", "", Some(Action::Page(Page::Expansion)));
                }
                OverviewContextKind::ScreenPreview => {
                    push(
                        if self.live_screen_stream {
                            "暂停内嵌实时画面同步"
                        } else {
                            "开启内嵌实时画面同步"
                        },
                        "",
                        Some(Action::ToggleLiveScreenStream),
                    );
                    push(
                        if self.mirror.is_some() {
                            "结束独立窗口投屏"
                        } else {
                            "启动独立高帧率投屏 (scrcpy)"
                        },
                        "Enter",
                        Some(if self.mirror.is_some() {
                            Action::StopMirror
                        } else {
                            Action::Mirror
                        }),
                    );
                    for (route, text) in [
                        (
                            stepx_companion::mirror::MirrorAudioRoute::Pc,
                            "声音：电脑端播放 (PC)",
                        ),
                        (
                            stepx_companion::mirror::MirrorAudioRoute::Phone,
                            "声音：手机端播放",
                        ),
                        (
                            stepx_companion::mirror::MirrorAudioRoute::Both,
                            "声音：电脑+手机双端同播",
                        ),
                    ] {
                        let prefix = if self.mirror_options.audio_route == route {
                            "✓ "
                        } else {
                            ""
                        };
                        push(
                            &format!("{prefix}{text}"),
                            "",
                            Some(Action::SetMirrorAudioRoute(route)),
                        );
                    }
                    push(
                        "打开投屏与实时触控工坊",
                        "",
                        Some(Action::OpenTool(Some(ToolPage::Screen))),
                    );
                    push("一键点亮并上滑解锁", "", Some(Action::ScreenGesture(7)));
                    push("下拉手机通知栏", "", Some(Action::ScreenGesture(4)));
                    push("返回上一级 (Back)", "", Some(Action::Key(4)));
                    push("回到手机桌面 (Home)", "", Some(Action::Key(3)));
                    push("截取当前手机屏幕", "", Some(Action::Screenshot));
                    if self.screenshot.is_some() {
                        push("保存截图到电脑…", "", Some(Action::SaveScreenshot));
                        push(
                            "将当前画面转为背屏像素画",
                            "",
                            Some(Action::ScreenshotToBackScreen),
                        );
                    }
                    push("打开背屏点阵创作", "", Some(Action::Page(Page::BackScreen)));
                    push(
                        "切换正 / 背屏预览",
                        "",
                        Some(Action::OverviewBackPreview(!self.overview_back_preview)),
                    );
                }
            },
            Target::BackScreenPreset(preset) => {
                push(
                    "预览此预设效果",
                    "",
                    Some(Action::BackScreen(BackScreenAction::SelectPreset(*preset))),
                );
                push(
                    "立即应用到手机背屏",
                    "Enter",
                    Some(Action::BackScreen(BackScreenAction::ApplyPreset(*preset))),
                );
                push(
                    "复制预设名称与说明",
                    "",
                    Some(Action::CopyText(
                        format!("{}：{}", preset.label(), preset.description()),
                        "已复制背屏预设信息".into(),
                    )),
                );
            }
            Target::BackScreenRecentImage { index, path } => {
                push(
                    "转为背屏像素画",
                    "Enter",
                    Some(Action::BackScreen(BackScreenAction::RecentImage(*index))),
                );
                push(
                    "用系统看图打开",
                    "",
                    Some(Action::Studio(StudioAction::OpenPath(PathBuf::from(path)))),
                );
                push(
                    "在文件夹中显示",
                    "",
                    Some(Action::RevealLocalPath(PathBuf::from(path))),
                );
                push(
                    "复制图片路径",
                    "",
                    Some(Action::CopyText(path.clone(), "已复制图片路径".into())),
                );
            }
            Target::BackScreenApp { label, package } => {
                push(
                    "关联到背屏通知灯效",
                    "",
                    Some(Action::BackScreen(BackScreenAction::SelectApp(
                        label.clone(),
                        package.clone(),
                    ))),
                );
                push(
                    "投屏到电脑打开",
                    "",
                    Some(Action::OpenAppInMirror(package.clone())),
                );
                push(
                    "在手机上打开应用",
                    "",
                    Some(Action::OpenApp(package.clone())),
                );
                push(
                    "复制应用包名",
                    "",
                    Some(Action::CopyText(
                        package.clone(),
                        format!("已复制包名 {package}"),
                    )),
                );
            }
            Target::ToolCard(tool) => {
                push(
                    &format!("打开「{}」", tool.title()),
                    "Enter",
                    Some(Action::OpenTool(Some(*tool))),
                );
                push(
                    "复制工具说明",
                    "",
                    Some(Action::CopyText(
                        format!("{}：{}", tool.title(), tool.description()),
                        "已复制工具说明".into(),
                    )),
                );
            }
            Target::ToolFile { path, operation } => {
                push(
                    "在文件管理中定位",
                    "Enter",
                    Some(Action::LocateToolFile(path.clone())),
                );
                if let Some(op) = operation {
                    push(
                        op.label(),
                        "",
                        Some(Action::PrepareToolOperation(op.clone())),
                    );
                }
                push(
                    "复制手机文件路径",
                    "",
                    Some(Action::CopyText(path.clone(), "已复制手机文件路径".into())),
                );
            }
            Target::Screenshot(path_str) => {
                let path = PathBuf::from(path_str);
                push(
                    "用系统看图软件打开",
                    "Enter",
                    Some(Action::Studio(StudioAction::OpenPath(path.clone()))),
                );
                push("另存截图到电脑…", "", Some(Action::SaveScreenshot));
                push("在文件夹中显示", "", Some(Action::RevealLocalPath(path)));
                push(
                    "发给 Amoo 分析",
                    "",
                    Some(Action::AmooDropFiles(vec![PathBuf::from(path_str)])),
                );
                push(
                    "复制截图路径",
                    "",
                    Some(Action::CopyText(
                        path_str.clone(),
                        "已复制截图文件路径".into(),
                    )),
                );
            }
            Target::ServiceRow { name, pkg, enabled } => {
                let can_toggle = self
                    .system_services
                    .iter()
                    .find(|s| &s.package == pkg)
                    .map(|s| {
                        s.safe_to_remove
                            && !s.status_note.contains("当前默认")
                            && s.status_note != "未安装"
                    })
                    .unwrap_or(true);
                push(
                    if *enabled {
                        "停用此系统服务"
                    } else {
                        "启用此系统服务"
                    },
                    "",
                    can_toggle.then(|| {
                        Action::PrepareUtility(Box::new(Action::ToggleSystemService(
                            pkg.clone(),
                            !*enabled,
                        )))
                    }),
                );
                push(
                    "复制服务包名",
                    "",
                    Some(Action::CopyText(
                        pkg.clone(),
                        format!("已复制服务包名 {pkg}"),
                    )),
                );
                push(
                    "复制服务名称与包名",
                    "",
                    Some(Action::CopyText(
                        format!("{name} ({pkg})"),
                        "已复制服务信息".into(),
                    )),
                );
            }
            Target::ConnectDevice {
                id,
                model,
                online,
                selected,
            } => {
                if *online && !*selected {
                    push("连接此设备", "Enter", Some(Action::Select(id.clone())));
                }
                push("刷新设备列表", "", Some(Action::Refresh));
                push(
                    "复制设备地址 / 序列号",
                    "",
                    Some(Action::CopyText(id.clone(), format!("已复制设备标识 {id}"))),
                );
                push(
                    "复制设备型号",
                    "",
                    Some(Action::CopyText(
                        model.clone(),
                        format!("已复制设备型号 {model}"),
                    )),
                );
            }
            Target::RememberedDevice {
                serial,
                model,
                is_preferred,
            } => {
                if !*is_preferred {
                    push(
                        "设为默认连接设备",
                        "Enter",
                        Some(Action::SetPreferredDevice(serial.clone())),
                    );
                }
                push(
                    "复制设备序列号",
                    "",
                    Some(Action::CopyText(
                        serial.clone(),
                        format!("已复制序列号 {serial}"),
                    )),
                );
                push(
                    "复制设备型号",
                    "",
                    Some(Action::CopyText(
                        model.clone(),
                        format!("已复制设备型号 {model}"),
                    )),
                );
                push(
                    "移除信任设备",
                    "",
                    Some(Action::RemoveRememberedDevice(serial.clone())),
                );
            }
            Target::CopyableText { title, text } => {
                push(
                    &format!("复制{title}"),
                    &copy_key,
                    (!text.is_empty())
                        .then(|| Action::CopyText(text.clone(), format!("已复制{title}"))),
                );
                push(
                    "发给 Amoo 分析",
                    "",
                    (!text.is_empty()).then(|| Action::SendToAmoo(text.clone())),
                );
            }
            Target::CalendarDate { year, month, day } => {
                use chrono::Datelike;
                let now_local = chrono::Local::now();
                let today = now_local.date_naive();
                let now_ms = now_local.timestamp_millis();
                let day_events: Vec<_> = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .filter(|ev| {
                        ev.date_ymd() == (*year, *month, *day)
                            && !stepx_companion::home_hub::any_synthetic_e2e_probe(&[
                                &ev.title,
                                &ev.location,
                                &ev.description,
                            ])
                            && !ev.title.contains("StepX 日历网格验证")
                            && !ev.title.contains("StepX E2E")
                    })
                    .collect();
                let target_date = chrono::NaiveDate::from_ymd_opt(*year, *month, *day);
                let valid_day_events = day_events
                    .iter()
                    .filter(|ev| {
                        if self.back_screen.completed_calendar_events.contains(&ev.id) {
                            return false;
                        }
                        if self.reminder_governance.is_any_key_suppressed(
                            &[format!("cal:{}", ev.id), format!("schedule:{}", ev.id)],
                            None,
                            None,
                            None,
                            now_ms,
                        ) {
                            return false;
                        }
                        match target_date {
                            Some(d) if d > today => true,
                            Some(d) if d == today => {
                                ev.all_day
                                    || (if ev.dtend > ev.dtstart {
                                        ev.dtend
                                    } else {
                                        ev.dtstart + 3_600_000
                                    }) > now_ms
                            }
                            _ => false,
                        }
                    })
                    .count();
                push(
                    &format!("在 {month:02}月{day:02}日 新建日程…"),
                    "Enter",
                    Some(Action::BackScreen(
                        BackScreenAction::CalendarStartCreateOnDate(*year, *month, *day),
                    )),
                );
                if (*year, *month, *day) != (today.year(), today.month(), today.day()) {
                    push(
                        "回到「今天」",
                        "",
                        Some(Action::BackScreen(BackScreenAction::CalendarToday)),
                    );
                }
                if !day_events.is_empty() {
                    let summary = day_events
                        .iter()
                        .map(|ev| format!("• {} {}", ev.hm_label(), ev.title))
                        .collect::<Vec<_>>()
                        .join("\n");
                    push(
                        &format!(
                            "复制当日日程清单（{} 项有效 / 共 {} 项）",
                            valid_day_events,
                            day_events.len()
                        ),
                        &copy_key,
                        Some(Action::CopyText(
                            summary,
                            format!(
                                "已复制 {month:02}月{day:02}日 {} 项日程",
                                day_events.len()
                            ),
                        )),
                    );
                }
                push(
                    "同步日历",
                    "",
                    Some(Action::BackScreen(BackScreenAction::InspectStepOsSuite)),
                );
                if !day_events.is_empty() {
                    push(
                        &format!("清空当日全部 {} 项日程", day_events.len()),
                        "Delete",
                        Some(Action::BackScreen(BackScreenAction::CalendarClearDay(
                            *year, *month, *day,
                        ))),
                    );
                }
            }
            Target::BackScreenCanvas => {
                push(
                    "立即应用到手机背屏",
                    "Enter",
                    Some(Action::BackScreen(BackScreenAction::Play)),
                );
                push(
                    "反色当前点阵画布",
                    "",
                    Some(Action::BackScreen(BackScreenAction::InvertFrame)),
                );
                push(
                    "向上平移 1 格",
                    "",
                    Some(Action::BackScreen(BackScreenAction::ShiftFrame(0, -1))),
                );
                push(
                    "向下平移 1 格",
                    "",
                    Some(Action::BackScreen(BackScreenAction::ShiftFrame(0, 1))),
                );
                push(
                    "向左平移 1 格",
                    "",
                    Some(Action::BackScreen(BackScreenAction::ShiftFrame(-1, 0))),
                );
                push(
                    "向右平移 1 格",
                    "",
                    Some(Action::BackScreen(BackScreenAction::ShiftFrame(1, 0))),
                );
                push(
                    "清空点阵画布",
                    "Delete",
                    Some(Action::BackScreen(BackScreenAction::ClearDiyFrame)),
                );
                push(
                    "停止手机背屏播放",
                    "",
                    Some(Action::BackScreen(BackScreenAction::Stop)),
                );
            }
            Target::HomeReminder(target) => {
                use stepx_companion::home_reminder_governance::ReminderCategory;
                if let Some((ref label, ref act)) = target.primary_action {
                    push(label, "Enter", Some((**act).clone()));
                }
                if !target.otp_or_pickup_code.is_empty() {
                    push(
                        &format!("复制码值 {}", target.otp_or_pickup_code),
                        &copy_key,
                        Some(Action::CopyText(
                            target.otp_or_pickup_code.clone(),
                            format!("已复制码值 {}", target.otp_or_pickup_code),
                        )),
                    );
                }
                push(
                    "✓ 立即过期 · 不再提醒",
                    "Delete",
                    Some(Action::HomeReminder(
                        pages::home::HomeReminderAction::ManualExpire(target.clone()),
                    )),
                );
                push(
                    "⏱ 稍后提醒：30 分钟后",
                    "",
                    Some(Action::HomeReminder(
                        pages::home::HomeReminderAction::SnoozeMinutes(target.clone(), 30),
                    )),
                );
                push(
                    "⏱ 稍后提醒：2 小时后",
                    "",
                    Some(Action::HomeReminder(
                        pages::home::HomeReminderAction::SnoozeMinutes(target.clone(), 120),
                    )),
                );
                push(
                    "⏱ 稍后提醒：今晚 20:00",
                    "",
                    Some(Action::HomeReminder(
                        pages::home::HomeReminderAction::SnoozeTonight20(target.clone()),
                    )),
                );
                push(
                    "⏱ 稍后提醒：明早 09:00",
                    "",
                    Some(Action::HomeReminder(
                        pages::home::HomeReminderAction::SnoozeTomorrow09(target.clone()),
                    )),
                );
                push(
                    "⌛ 设为 1 小时后自动过期",
                    "",
                    Some(Action::HomeReminder(
                        pages::home::HomeReminderAction::CustomExpireHours(target.clone(), 1),
                    )),
                );
                push(
                    "⌛ 设为 4 小时后自动过期",
                    "",
                    Some(Action::HomeReminder(
                        pages::home::HomeReminderAction::CustomExpireHours(target.clone(), 4),
                    )),
                );
                push(
                    "⌛ 设为今日 24:00 自动过期",
                    "",
                    Some(Action::HomeReminder(
                        pages::home::HomeReminderAction::CustomExpireTonight24(target.clone()),
                    )),
                );
                let cur_ttl_mins = self
                    .reminder_governance
                    .effective_ttl_mins(target.spec.category);
                let presets: &[(u32, &str)] = match target.spec.category {
                    ReminderCategory::OtpCode => &[(15, "15分钟"), (30, "30分钟"), (60, "1小时")],
                    ReminderCategory::TakeoutAndMobility => {
                        &[(60, "1小时"), (120, "2小时"), (240, "4小时")]
                    }
                    ReminderCategory::ExpressPickup => {
                        &[(720, "12小时"), (1440, "24小时"), (2880, "48小时")]
                    }
                    _ => &[(60, "1小时"), (240, "4小时"), (1440, "24小时")],
                };
                for &(mins, label_str) in presets {
                    let prefix = if cur_ttl_mins == mins { "✓ ⚙" } else { "⚙" };
                    push(
                        &format!(
                            "{prefix} 设「{}」默认过期为 {label_str}",
                            target.spec.category.label()
                        ),
                        "",
                        Some(Action::HomeReminder(
                            pages::home::HomeReminderAction::SetCategoryTtlMins {
                                category: target.spec.category,
                                ttl_mins: Some(mins),
                            },
                        )),
                    );
                }
                if !target.mute_source_key.is_empty() {
                    push(
                        &format!("🔕 屏蔽「{}」此类提醒", target.mute_source_label),
                        "",
                        Some(Action::HomeReminder(
                            pages::home::HomeReminderAction::MuteSource {
                                source_key: target.mute_source_key.clone(),
                                label: target.mute_source_label.clone(),
                            },
                        )),
                    );
                }
                push(
                    "复制提醒标题与详情",
                    "",
                    Some(Action::CopyText(
                        target.copy_text.clone(),
                        format!("已复制「{}」", target.spec.title),
                    )),
                );
                push(
                    "发给 Amoo 跟进处理",
                    "",
                    Some(Action::SendToAmoo(target.amoo_prompt.clone())),
                );
                push(
                    "🗂 打开首页提醒与过期管理…",
                    "",
                    Some(Action::HomeReminder(
                        pages::home::HomeReminderAction::OpenGovernanceDrawer(true),
                    )),
                );
            }
            Target::HomeModule(mod_id) => {
                for width in pages::home::HomeModuleWidth::ALL {
                    push(&format!("{}宽度 · {}", if self.home_modules.module_width(*mod_id) == width { "✓ " } else { "" }, width.short_label()), "",
                        Some(Action::HomeModule(pages::home::HomeModuleAction::SetModuleWidth(*mod_id, width))));
                }
                for height in pages::home::HomeModuleHeight::ALL {
                    push(&format!("{}内容 · {}", if self.home_modules.module_height(*mod_id) == height { "✓ " } else { "" }, height.short_label()), "",
                        Some(Action::HomeModule(pages::home::HomeModuleAction::SetModuleHeight(*mod_id, height))));
                }
                push("向前移动", "", Some(Action::HomeModule(pages::home::HomeModuleAction::MoveUp(*mod_id))));
                push("向后移动", "", Some(Action::HomeModule(pages::home::HomeModuleAction::MoveDown(*mod_id))));
                push("移到最前", "", Some(Action::HomeModule(pages::home::HomeModuleAction::MoveToTop(*mod_id))));
                push("隐藏模块", "", Some(Action::HomeModule(pages::home::HomeModuleAction::RemoveModule(*mod_id))));
                if !self.home_module_edit_mode {
                    push("调整首页布局", "", Some(Action::HomeModule(pages::home::HomeModuleAction::SetEditMode(true))));
                }
            }
            Target::HomeCanvas => {
                push(
                    "🗂 打开首页提醒与过期管理…",
                    "",
                    Some(Action::HomeReminder(
                        pages::home::HomeReminderAction::OpenGovernanceDrawer(true),
                    )),
                );
                push(
                    "🧹 一键清理已结束提醒",
                    "",
                    Some(Action::HomeReminder(
                        pages::home::HomeReminderAction::PurgeExpiredNow,
                    )),
                );
                push("刷新首页全部数据", "", Some(Action::HomeRefreshAll));
                push(
                    if self.home_module_edit_mode {
                        "完成首页模块编辑"
                    } else {
                        "调整首页模块布局…"
                    },
                    "",
                    Some(Action::HomeModule(
                        pages::home::HomeModuleAction::ToggleEditMode,
                    )),
                );
                push(
                    &format!("显示全部 {} 个首页模块", pages::home::HomeModuleId::ALL.len()),
                    "",
                    Some(Action::HomeModule(
                        pages::home::HomeModuleAction::ShowAllModules,
                    )),
                );
                push(
                    "恢复首页默认布局",
                    "",
                    Some(Action::HomeModule(
                        pages::home::HomeModuleAction::ResetDefault,
                    )),
                );
            }
        }
        let is_intent = matches!(menu.target, Target::WorkspaceIntent(_));
        let is_home_ctx = matches!(
            menu.target,
            Target::HomeReminder(_) | Target::HomeModule(_) | Target::HomeCanvas
        );
        let total = raw_entries.len();
        let mut prev_group: Option<u8> = None;
        let mut prev_danger = false;
        raw_entries
            .into_iter()
            .enumerate()
            .map(|(idx, (label, key, action))| {
                let (icon, is_danger, group) = classify_context_entry(&label, action.as_ref());
                let separator_before = if is_intent || idx == 0 {
                    false
                } else if is_home_ctx {
                    prev_group != Some(group)
                } else if is_danger && !prev_danger {
                    true
                } else if !is_danger && prev_danger {
                    !label.starts_with("停止手机背屏")
                } else if label.starts_with("在此目录新建文件夹") {
                    true
                } else if total >= 4 && prev_group == Some(0) && group > 0 {
                    true
                } else {
                    false
                };
                prev_group = Some(group);
                prev_danger = is_danger;
                ContextMenuEntry {
                    label,
                    key,
                    action,
                    icon,
                    is_danger,
                    separator_before,
                }
            })
            .collect()
    }
    pub(super) fn context_overlay(&self) -> Option<El> {
        let menu = self.context_menu.as_ref()?;
        let p = self.p();
        let entries = self.context_entries(menu);
        let is_intent = matches!(menu.target, Target::WorkspaceIntent(_));
        let mut rows: Vec<El> = Vec::with_capacity(entries.len() + 4);
        for (i, entry) in entries.into_iter().enumerate() {
            if entry.separator_before && i > 0 {
                rows.push(
                    Container::new(self.divider())
                        .with_horizontal_padding(S2)
                        .with_vertical_padding(3.)
                        .finish(),
                );
            }
            let enabled = entry.action.is_some();
            let label = entry.label;
            let key = entry.key;
            let action = entry.action;
            let icon = entry.icon;
            let is_danger = entry.is_danger;
            let row = Hoverable::new(self.mouse_state(format!("context-{i}")), move |mouse| {
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
                let active = hovered || menu.selected == Some(i) || mouse.is_clicked();
                let content = if is_intent {
                    use stepx_companion::amoo_workspace::Intent;
                    let description = [
                        "按你说的内容自动判断，聊天或直接生成",
                        "直接与手机 Amoo 对话、查信息，不生成文件",
                        "整理资料，生成 PPT、Word 或 Excel 文件",
                        "制作可试玩、可发手机的小应用与小游戏",
                        "描述画面，生成图片或视觉海报",
                        "描述图案，生成并推送到手机背屏点阵",
                        "投屏接管手机屏幕，AI 自动点击滑动",
                    ]
                    .get(i)
                    .copied()
                    .unwrap_or("");
                    let selected = [
                        Some(Intent::Auto),
                        Some(Intent::Chat),
                        Some(Intent::Files),
                        Some(Intent::App),
                        Some(Intent::Image),
                        Some(Intent::Screen),
                        None,
                    ]
                    .get(i)
                    .copied()
                    .flatten()
                        == Some(self.workspace.intent);
                    let icon_color = if selected || active {
                        p.accent
                    } else {
                        p.text2
                    };
                    let text_col = if self.workspace.viewport.y() < 560. || description.is_empty() {
                        self.line_w(&label, BODY, p.text, Weight::Medium)
                    } else {
                        self.vstack(
                            1.,
                            vec![
                                self.line_w(&label, BODY, p.text, Weight::Medium),
                                self.line(description, MICRO, p.text3),
                            ],
                        )
                    };
                    let left = self.hstack(
                        S2,
                        vec![
                            self.ui_icon(icon, 16., icon_color),
                            Expanded::new(1., text_col).finish(),
                        ],
                    );
                    self.between(
                        left,
                        if selected {
                            self.ui_icon(icons::UiIcon::Check, 16., p.text)
                        } else {
                            self.line("", MICRO, p.text3)
                        },
                    )
                } else {
                    let (text_color, icon_color, key_color) = if !enabled {
                        (p.text3, p.text3, p.text3)
                    } else if is_danger {
                        (p.danger, p.danger, mix_color(p.danger, p.text3, 0.35))
                    } else {
                        (
                            p.text,
                            if active { p.accent } else { p.text2 },
                            p.text3,
                        )
                    };
                    let display_label = label
                        .strip_prefix("✓ ⚙ ")
                        .or_else(|| label.strip_prefix("✓ "))
                        .or_else(|| label.strip_prefix("⏱ "))
                        .or_else(|| label.strip_prefix("⌛ "))
                        .or_else(|| label.strip_prefix("⚙ "))
                        .or_else(|| label.strip_prefix("🔕 "))
                        .or_else(|| label.strip_prefix("🗂 "))
                        .or_else(|| label.strip_prefix("🧹 "))
                        .unwrap_or(&label);
                    let left = self.hstack(
                        S2,
                        vec![
                            self.ui_icon(icon, 15., icon_color),
                            Expanded::new(
                                1.,
                                self.line_w(display_label, BODY, text_color, Weight::Medium),
                            )
                            .finish(),
                        ],
                    );
                    if key.is_empty() {
                        left
                    } else {
                        self.between(left, self.mono(&key, MICRO, key_color))
                    }
                };
                let bg = if !enabled {
                    ColorU::transparent_black()
                } else if is_danger {
                    if mouse.is_clicked() {
                        mix_color(p.danger_soft, p.danger, 0.18)
                    } else if menu.selected == Some(i) {
                        p.danger_soft
                    } else {
                        mix_color(ColorU::transparent_black(), p.danger_soft, t)
                    }
                } else if mouse.is_clicked() {
                    p.accent_soft
                } else if menu.selected == Some(i) {
                    p.hover
                } else {
                    mix_color(ColorU::transparent_black(), p.hover, t)
                };
                Container::new(content)
                    .with_horizontal_padding(S3)
                    .with_vertical_padding(if is_intent { S1 } else { S2 })
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                    .finish()
            })
            .with_cursor(if enabled {
                Cursor::PointingHand
            } else {
                Cursor::Arrow
            })
            .on_click(move |ctx, _, _| {
                if let Some(a) = &action {
                    ctx.dispatch_typed_action(Action::ContextRun(Box::new(a.clone())));
                }
            })
            .finish();
            let row = SavePosition::new(row, &format!("context-choice-{i}"))
                .for_single_frame()
                .finish();
            rows.push(row);
        }
        let menu_w = match menu.target {
            Target::WorkspaceIntent(_) => 280.,
            Target::CalendarDate { .. } => 296.,
            Target::ScheduleEvent(_) | Target::AlarmItem(_) => 268.,
            Target::HomeReminder(_) | Target::HomeModule(_) | Target::HomeCanvas => 296.,
            _ => 248.,
        };
        let surface = self.fixed_w(
            menu_w,
            Container::new(self.vstack(2., rows))
                .with_uniform_padding(S1)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(p.border_strong))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
        );
        let surface = if is_intent {
            motion::enter(
                surface,
                self.workspace.panel_entered,
                !self.settings.value.reduced_motion,
            )
        } else {
            surface
        };
        let card_bounds: std::rc::Rc<std::cell::Cell<Option<pathfinder_geometry::rect::RectF>>> =
            std::rc::Rc::new(std::cell::Cell::new(None));
        let tracked_surface = ContextMenuCardBounds {
            child: surface,
            bounds: card_bounds.clone(),
        }
        .finish();
        let positioned_surface = SavePosition::new(tracked_surface, "context-surface")
            .for_single_frame()
            .finish();
        let overlay_stack = Stack::new()
            .with_child(Align::new(Empty::new().finish()).finish())
            .with_positioned_child(
                positioned_surface,
                if let Target::WorkspaceIntent(above) = menu.target {
                    OffsetPositioning::offset_from_save_position_element(
                        "workspace-intent-anchor",
                        Vector2F::new(0., if above { -6. } else { 6. }),
                        PositionedElementOffsetBounds::WindowByPosition,
                        if above {
                            PositionedElementAnchor::TopLeft
                        } else {
                            PositionedElementAnchor::BottomLeft
                        },
                        if above {
                            ChildAnchor::BottomLeft
                        } else {
                            ChildAnchor::TopLeft
                        },
                    )
                } else {
                    OffsetPositioning::offset_from_parent(
                        menu.at,
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    )
                },
            )
            .finish();
        Some(
            ContextMenuOverlayStack {
                overlay_stack,
                card_bounds,
            }
            .finish(),
        )
    }
}

struct ContextMenuCardBounds {
    child: El,
    bounds: std::rc::Rc<std::cell::Cell<Option<pathfinder_geometry::rect::RectF>>>,
}

impl facloud_ui::Element for ContextMenuCardBounds {
    fn layout(
        &mut self,
        c: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &facloud_ui::AppContext,
    ) -> Vector2F {
        self.child.layout(c, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &facloud_ui::AppContext) {
        self.child.after_layout(ctx, app);
        if let (Some(origin), Some(sz)) = (self.child.origin(), self.child.size()) {
            self.bounds.set(Some(pathfinder_geometry::rect::RectF::new(
                pathfinder_geometry::vector::vec2f(origin.x(), origin.y()),
                sz,
            )));
        }
    }

    fn paint(
        &mut self,
        origin: Vector2F,
        ctx: &mut PaintContext,
        app: &facloud_ui::AppContext,
    ) {
        if let Some(sz) = self.child.size() {
            self.bounds
                .set(Some(pathfinder_geometry::rect::RectF::new(origin, sz)));
        }
        self.child.paint(origin, ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &facloud_ui::AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }
}

struct ContextMenuOverlayStack {
    overlay_stack: El,
    card_bounds: std::rc::Rc<std::cell::Cell<Option<pathfinder_geometry::rect::RectF>>>,
}

impl ContextMenuOverlayStack {
    fn point_in_overlay(&self, pos: Vector2F) -> bool {
        let Some(rect) = self.card_bounds.get() else {
            return true;
        };
        let margin = 6.0;
        let min_x = rect.origin().x() - margin;
        let max_x = rect.origin().x() + rect.width() + margin;
        let min_y = rect.origin().y() - margin;
        let max_y = rect.origin().y() + rect.height() + margin;
        pos.x() >= min_x && pos.x() <= max_x && pos.y() >= min_y && pos.y() <= max_y
    }
}

impl facloud_ui::Element for ContextMenuOverlayStack {
    fn layout(
        &mut self,
        c: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &facloud_ui::AppContext,
    ) -> Vector2F {
        self.overlay_stack.layout(c, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &facloud_ui::AppContext) {
        self.overlay_stack.after_layout(ctx, app);
    }

    fn paint(
        &mut self,
        origin: Vector2F,
        ctx: &mut PaintContext,
        app: &facloud_ui::AppContext,
    ) {
        self.overlay_stack.paint(origin, ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.overlay_stack.size()
    }

    fn origin(&self) -> Option<Point> {
        self.overlay_stack.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &facloud_ui::AppContext,
    ) -> bool {
        let pointer_pos = match event.raw_event() {
            Event::ScrollWheel { position, .. }
            | Event::MouseMoved { position, .. }
            | Event::LeftMouseDown { position, .. }
            | Event::LeftMouseUp { position, .. }
            | Event::LeftMouseDragged { position, .. }
            | Event::RightMouseDown { position, .. } => Some(*position),
            _ => None,
        };

        if let Some(pos) = pointer_pos {
            if self.point_in_overlay(pos) {
                let _ = self.overlay_stack.dispatch_event(event, ctx, app);
                return true;
            }
            match event.raw_event() {
                Event::ScrollWheel { .. } => return true,
                Event::LeftMouseDown { .. } => {
                    ctx.dispatch_typed_action(Action::ContextClose);
                    return true;
                }
                Event::RightMouseDown { .. } => {
                    ctx.dispatch_typed_action(Action::ContextClose);
                    return false;
                }
                _ => {
                    let _ = self.overlay_stack.dispatch_event(event, ctx, app);
                    return true;
                }
            }
        }

        self.overlay_stack.dispatch_event(event, ctx, app)
    }
}

fn clamp_menu_position(at: Vector2F, item_count: usize, window_size: Option<Vector2F>) -> Vector2F {
    clamp_menu_position_with_width(at, item_count, window_size, 248.0)
}

fn clamp_menu_position_with_width(
    at: Vector2F,
    item_count: usize,
    window_size: Option<Vector2F>,
    menu_w: f32,
) -> Vector2F {
    const ROW_H: f32 = 34.0;
    const PADDING_H: f32 = 16.0;
    const MARGIN: f32 = 8.0;
    let est_h = PADDING_H + (item_count.max(1) as f32) * ROW_H;
    let Some(size) = window_size else {
        return Vector2F::new(at.x().max(MARGIN), at.y().max(MARGIN));
    };
    let max_x = (size.x() - menu_w - MARGIN).max(MARGIN);
    let raw_y = if at.y() + est_h + MARGIN > size.y() && at.y() > est_h + MARGIN {
        at.y() - est_h
    } else {
        at.y()
    };
    let max_y = (size.y() - est_h - MARGIN).max(MARGIN);
    Vector2F::new(at.x().clamp(MARGIN, max_x), raw_y.clamp(MARGIN, max_y))
}

// Window-local coordinates. Prefer the button's lower edge; only flip for the bottom composer.
fn intent_menu_position(x: f32, y: f32, anchor_height: f32, width: f32, height: f32) -> (f32, f32) {
    let menu_height = 240.;
    let below = y + anchor_height + 6.;
    (
        x.clamp(8., (width - 288.).max(8.)),
        if below + menu_height <= height - 8. {
            below
        } else {
            (y - menu_height - 6.).max(8.)
        },
    )
}
#[cfg(test)]
mod intent_tests {
    use super::*;
    #[test]
    fn opens_below_when_room() {
        assert_eq!(
            intent_menu_position(120., 200., 36., 1000., 800.),
            (120., 242.)
        );
    }
    #[test]
    fn flips_above_bottom_composer() {
        assert_eq!(
            intent_menu_position(120., 700., 36., 1000., 800.),
            (120., 454.)
        );
    }
    #[test]
    fn stays_inside_narrow_window() {
        assert_eq!(
            intent_menu_position(480., 500., 36., 600., 650.),
            (312., 254.)
        );
    }
    #[test]
    fn clamps_right_click_menu_near_window_edges() {
        let clamped = clamp_menu_position(
            Vector2F::new(980., 760.),
            6,
            Some(Vector2F::new(1024., 800.)),
        );
        assert!(clamped.x() <= 1024. - 248. - 8.);
        assert!(clamped.y() < 760.);
    }
    #[test]
    fn calendar_date_canvas_and_files_directory_menus_include_smart_actions() {
        let app = Companion::new_for_test();
        let cal_menu = Menu {
            target: Target::CalendarDate {
                year: 2026,
                month: 10,
                day: 26,
            },
            at: Vector2F::new(100., 100.),
            text: None,
            selected: None,
        };
        let cal_labels: Vec<String> = app
            .context_entries(&cal_menu)
            .into_iter()
            .map(|e| e.label)
            .collect();
        assert!(cal_labels.iter().any(|l| l.contains("新建日程")));
        assert!(cal_labels.iter().any(|l| l.contains("同步日历")));
        assert!(!cal_labels.iter().any(|l| l.contains("快速添加")));
        assert!(!cal_labels.iter().any(|l| l.contains("让 Amoo 规划")));

        let canvas_menu = Menu {
            target: Target::BackScreenCanvas,
            at: Vector2F::new(100., 100.),
            text: None,
            selected: None,
        };
        let canvas_labels: Vec<String> = app
            .context_entries(&canvas_menu)
            .into_iter()
            .map(|e| e.label)
            .collect();
        assert!(canvas_labels.iter().any(|l| l == "反色当前点阵画布"));
        assert!(canvas_labels.iter().any(|l| l == "清空点阵画布"));

        let dir_menu = Menu {
            target: Target::FilesDirectory,
            at: Vector2F::new(100., 100.),
            text: None,
            selected: None,
        };
        let dir_labels: Vec<String> = app
            .context_entries(&dir_menu)
            .into_iter()
            .map(|e| e.label)
            .collect();
        assert!(dir_labels.iter().any(|l| l == "新建文件夹…"));
        assert!(dir_labels.iter().any(|l| l == "新建文本文件…"));
        assert!(dir_labels.iter().any(|l| l == "上传电脑文件到此目录…"));

        let screen_menu = Menu {
            target: Target::OverviewCard(OverviewContextKind::ScreenPreview),
            at: Vector2F::new(100., 100.),
            text: None,
            selected: None,
        };
        let screen_labels: Vec<String> = app
            .context_entries(&screen_menu)
            .into_iter()
            .map(|e| e.label)
            .collect();
        assert!(screen_labels.iter().any(|l| l.contains("实时画面同步")));
        assert!(screen_labels.iter().any(|l| l.contains("独立高帧率投屏")));
        assert!(screen_labels
            .iter()
            .any(|l| l == "✓ 声音：电脑端播放 (PC)"));
        assert!(screen_labels.iter().any(|l| l == "声音：手机端播放"));
        assert!(screen_labels.iter().any(|l| l == "声音：电脑+手机双端同播"));
        assert!(screen_labels
            .iter()
            .any(|l| l.contains("打开投屏与实时触控工坊")));
        assert!(screen_labels
            .iter()
            .any(|l| l.contains("一键点亮并上滑解锁")));
    }

    #[test]
    fn context_entries_assign_icons_dividers_and_danger_styles() {
        let mut app = Companion::new_for_test();
        let mut t = stepx_companion::amoo_workspace::Task::new(None, None);
        t.title = "测试对话".into();
        let tid = t.id.clone();
        app.workspace.journal.tasks.push(t);

        let session_menu = Menu {
            target: Target::ChatSession {
                workspace_task_id: Some(tid),
                phone_session_idx: None,
                title: "测试对话".into(),
            },
            at: Vector2F::new(100., 100.),
            text: None,
            selected: None,
        };
        let entries = app.context_entries(&session_menu);
        assert!(entries
            .iter()
            .any(|e| e.label == "切换至此对话" && e.icon == icons::UiIcon::Chat));
        assert!(entries
            .iter()
            .any(|e| e.label == "置顶此对话" && e.icon == icons::UiIcon::Pin));
        assert!(entries
            .iter()
            .any(|e| e.label == "复制全部对话内容" && e.icon == icons::UiIcon::Copy));
        assert!(entries
            .iter()
            .any(|e| e.label == "删除此对话"
                && e.icon == icons::UiIcon::Trash
                && e.is_danger
                && e.separator_before));
    }
}
