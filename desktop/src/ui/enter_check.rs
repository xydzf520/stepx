//! Opt-in regression through the painted keyboard handler, not a send action.
use super::*;
use facloud_ui::TypedActionView;
use stepx_companion::amoo_workspace::{Executor, Intent, Phase};

#[derive(Default)]
struct Check {
    stage: usize,
    since: Option<Instant>,
    last: Option<Instant>,
    task: String,
}
thread_local! { static CHECK: RefCell<Check> = RefCell::new(Check::default()); }

fn key(
    ctx: &mut ViewContext<Companion>,
    dir: &std::path::Path,
    stage: usize,
    name: &str,
    shift: bool,
) {
    if std::env::var_os("STEPX_ENTER_NATIVE").is_some() {
        // The external runner delivers native X11 keyboard events to this window.
        std::fs::write(
            dir.join("native-event.json"),
            serde_json::json!({
                "stage": stage, "key": name, "shift": shift, "pid": std::process::id()
            })
            .to_string(),
        )
        .unwrap();
        return;
    }
    let event = Event::KeyDown {
        keystroke: facloud_ui::keymap::Keystroke {
            key: name.into(),
            shift,
            ..Default::default()
        },
        chars: String::new(),
        details: Default::default(),
        is_composing: false,
    };
    raw(ctx, event);
}
fn raw(ctx: &mut ViewContext<Companion>, event: Event) {
    let id = ctx.window_id();
    let sent = std::cell::Cell::new(false);
    ctx.on_frame_drawn(move |ctx, drawn| {
        if drawn != id || sent.replace(true) {
            return;
        }
        let window = ctx.windows().platform_window(id).unwrap();
        (window.callbacks().event_callback)(event.clone(), ctx);
    });
}

impl Companion {
    pub(super) fn verify_enter(&mut self, ctx: &mut ViewContext<Self>, live: bool) {
        let Some(dir) = self
            .capture
            .as_ref()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
        else {
            return;
        };
        CHECK.with(|cell| {
            let mut c = cell.borrow_mut();
            if c.stage == 99 { return; }
            let since = *c.since.get_or_insert_with(Instant::now);
            assert!(since.elapsed() < Duration::from_secs(300), "ENTER_CHECK timed out at {}: {}", c.stage, self.status);
            let interval = if std::env::var_os("STEPX_ENTER_NATIVE").is_some() { 3500 } else { 1100 };
            if self.initializing || c.last.is_some_and(|t| t.elapsed() < Duration::from_millis(interval)) { return; }
            c.last = Some(Instant::now());
            if live {
                let serial = std::env::var("STEPX_WORKSPACE_TEST_SERIAL").expect("explicit phone required");
                if self.selected.as_ref().is_none_or(|d| d.serial != serial) {
                    if !self.busy { self.handle_action(&Action::Select(serial), ctx); }
                    return;
                }
            }
            if c.stage == 0 && (self.busy || self.workspace_busy()) { return; }
            let run = |v: &mut Companion, action, ctx: &mut ViewContext<Self>| v.handle_action(&Action::Workspace(action), ctx);
            use WorkspaceAction as W;
            match c.stage {
                0 => {
                    self.set_compact_mode(ShellMode::MiniAi, ctx);
                    run(self, W::New, ctx);
                    self.fields[F_AMOO].set("回车测试");
                    self.focus_field(F_AMOO, ctx);
                }
                1 => key(ctx, &dir, c.stage, "enter", true),
                2 => {
                    assert_eq!(self.fields[F_AMOO].text, "回车测试\n", "Shift+Enter must insert one newline");
                    raw(ctx, Event::SetMarkedText { marked_text: "zhong wen".into(), selected_range: 0..9 });
                }
                3 => {
                    assert_eq!(self.fields[F_AMOO].preedit, "zhong wen");
                    // An IME candidate-confirm key must never submit the draft.
                    raw(ctx, Event::KeyDown { keystroke: facloud_ui::keymap::Keystroke { key: "enter".into(), ..Default::default() }, chars: String::new(), details: Default::default(), is_composing: true });
                }
                4 => {
                    assert!(!self.workspace_busy() && self.workspace.proposal.is_none());
                    assert_eq!(self.fields[F_AMOO].text, "回车测试\n");
                    raw(ctx, Event::TypedCharacters { chars: "中文🙂".into() });
                }
                5 => {
                    assert_eq!(self.fields[F_AMOO].text, "回车测试\n中文🙂");
                    assert!(self.fields[F_AMOO].preedit.is_empty());
                    self.demo_mode = true;
                    run(self, W::Intent(Intent::Files), ctx);
                    key(ctx, &dir, c.stage, "enter", false);
                }
                6 => {
                    assert!(self.workspace.proposal.is_none() && !self.workspace_busy());
                    assert_eq!(self.workspace_task().expect("Enter dropped before workspace handler").route, Some(Intent::Files));
                    key(ctx, &dir, c.stage, "enter", false);
                }
                7 => {
                    assert!(self.workspace.proposal.is_none() && !self.workspace_busy());
                    assert_eq!(self.fields[F_AMOO].text, "回车测试\n中文🙂");
                    run(self, W::Reroute(Intent::Chat), ctx);
                    assert_eq!(self.workspace_task().unwrap().route, None);
                    run(self, W::Intent(Intent::Image), ctx);
                    key(ctx, &dir, c.stage, "enter", false);
                }
                8 => {
                    assert_eq!(self.workspace_task().unwrap().route, Some(Intent::Image));
                    self.set_compact_mode(ShellMode::Full, ctx);
                    self.focus_field(F_AMOO, ctx);
                    run(self, W::Intent(Intent::App), ctx);
                    key(ctx, &dir, c.stage, "return", false);
                }
                9 => {
                    assert_eq!(self.workspace_task().unwrap().route, Some(Intent::App));
                    self.demo_mode = !live;
                    run(self, W::New, ctx);
                    self.set_compact_mode(ShellMode::MiniAi, ctx);
                    self.focus_field(F_AMOO, ctx);
                    self.fields[F_AMOO].set("这是键盘发送验收。记住测试数 47，只回答：已收到，47。不要操作手机或执行任务。");
                    key(ctx, &dir, c.stage, "enter", false);
                }
                10 => {
                    assert!(self.workspace.proposal.is_none());
                    if live {
                        let task = self.workspace_task().expect("actual chat task");
                        assert_eq!(task.turns.iter().filter(|t| t.user).count(), 1);
                        c.task = task.id.clone();
                        assert!(task.steps.iter().all(|s| s.executor == Executor::Phone));
                        self.fields[F_AMOO].set("回复期间保留的未发送草稿");
                    } else {
                        assert!(self.workspace.error.as_deref().is_some_and(|e| e.contains("演示模式")), "Chat Enter must reach the send guard");
                    }
                }
                11 => {
                    if live {
                        if self.busy || self.workspace_busy() { return; }
                        let task = self.workspace_task().unwrap();
                        assert!(task.steps.iter().all(|s| s.phase == Phase::Done));
                        assert!(task.turns.iter().any(|t| !t.user && t.text.contains("47")), "phone did not reply");
                        assert_eq!(self.fields[F_AMOO].text, "回复期间保留的未发送草稿");
                        std::fs::write(dir.join("compact-result.json"), serde_json::to_vec_pretty(task).unwrap()).unwrap();
                    }
                    self.set_compact_mode(ShellMode::Full, ctx);
                    self.focus_field(F_AMOO, ctx);
                    self.fields[F_AMOO].set("刚才的测试数加 5 是多少？只回答数字。");
                    key(ctx, &dir, c.stage, "return", false);
                }
                12 => {
                    if live {
                        if self.busy || self.workspace_busy() { return; }
                        let task = self.workspace_task().unwrap();
                        assert_eq!(task.id, c.task);
                        assert_eq!(task.turns.iter().filter(|t| t.user).count(), 2, "duplicate or missing send");
                        assert!(task.turns.iter().rev().find(|t| !t.user).unwrap().text.contains("52"));
                        std::fs::write(dir.join("full-result.json"), serde_json::to_vec_pretty(task).unwrap()).unwrap();
                    } else {
                        assert!(self.workspace.error.as_deref().is_some_and(|e| e.contains("演示模式")));
                    }
                    self.fields[F_AMOO].clear();
                    self.workspace.error = None;
                    key(ctx, &dir, c.stage, "enter", false);
                }
                13 => {
                    assert!(!self.workspace_busy() && self.workspace.proposal.is_none());
                    assert!(self.fields[F_AMOO].text.is_empty());
                    if live { assert_eq!(self.workspace_task().unwrap().turns.iter().filter(|t| t.user).count(), 2); }
                    std::fs::write(dir.join("passed.json"), serde_json::json!({"live":live,"native_keys":std::env::var_os("STEPX_ENTER_NATIVE").is_some(),"checks":["Shift+Enter newline","IME confirmation no send","Chinese commit","Files/Image/App Enter opens confirmation","repeat Enter cannot approve","compact chat Enter","full chat keypad Enter","empty Enter no request","no duplicate turns"]}).to_string()).unwrap();
                    eprintln!("ENTER_CHECK passed (live={live})");
                    c.stage = 99;
                    ctx.close_window();
                    return;
                }
                _ => unreachable!(),
            }
            eprintln!("ENTER_CHECK stage {} passed ({}s)", c.stage, since.elapsed().as_secs());
            c.stage += 1;
            ctx.notify();
        });
    }
}
