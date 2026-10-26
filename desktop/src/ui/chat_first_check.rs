//! Opt-in acceptance of chat routing and anchored menus; never runs on ordinary launches.
use super::*;
use facloud_ui::TypedActionView;
use stepx_companion::amoo_workspace::{Executor, Intent};
#[derive(Default)]
struct Check {
    step: usize,
    last: Option<Instant>,
    start: Option<Instant>,
    task: String,
    attachment: PathBuf,
    captured: Option<usize>,
}
thread_local! { static CHAT_CHECK: RefCell<Check> = RefCell::new(Check::default()); }
fn capture(ctx: &mut ViewContext<Companion>, path: PathBuf) {
    if let Some(w) = ctx.windows().platform_window(ctx.window_id()) {
        w.as_ctx().request_frame_capture(Box::new(move |f| {
            image::save_buffer(path, &f.data, f.width, f.height, image::ColorType::Rgba8).unwrap()
        }));
    }
}
fn pointer(ctx: &mut ViewContext<Companion>, position: Vector2F) {
    let id = ctx.window_id();
    let sent = std::cell::Cell::new(false);
    // Deliver after this view update; dispatching while the view is borrowed
    // cannot reach its action handler. Exercise painted hit regions, not actions.
    ctx.on_frame_drawn(move |ctx, drawn| {
        if id != drawn || sent.replace(true) {
            return;
        }
        let window = ctx.windows().platform_window(id).unwrap();
        for event in [
            Event::MouseMoved {
                position,
                cmd: false,
                shift: false,
                is_synthetic: false,
            },
            Event::LeftMouseDown {
                position,
                modifiers: Default::default(),
                click_count: 1,
                is_first_mouse: false,
            },
            Event::LeftMouseUp {
                position,
                modifiers: Default::default(),
            },
        ] {
            (window.callbacks().event_callback)(event, ctx);
        }
    });
}
fn click(ctx: &mut ViewContext<Companion>, id: &str) {
    let rect = ctx
        .element_position_by_id(id.to_owned())
        .expect("visible click target");
    pointer(ctx, rect.origin() + rect.size() / 2.);
}
fn aligned(ctx: &ViewContext<Companion>) {
    let a = ctx
        .element_position_by_id("workspace-chat-heading".to_owned())
        .unwrap();
    for id in ["workspace-chat-messages", "workspace-chat-input"] {
        let b = ctx.element_position_by_id(id.to_owned()).unwrap();
        assert!(
            (a.origin().x() - b.origin().x()).abs() < 1. && (a.width() - b.width()).abs() < 1.,
            "chat columns misaligned: {a:?} vs {b:?}"
        );
    }
}
impl Companion {
    pub(super) fn verify_chat_first(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(dir) = self
            .capture
            .as_ref()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
        else {
            return;
        };
        assert!(self.demo_mode);
        CHAT_CHECK.with(|check| {
            let mut c=check.borrow_mut();
            if c.step==99 {return;}
            let start=*c.start.get_or_insert_with(Instant::now);
            assert!(start.elapsed()<Duration::from_secs(120), "chat-first acceptance timeout at {}",c.step);
            if self.busy || self.initializing || c.last.is_some_and(|t| t.elapsed()<Duration::from_millis(800)) { return; }
            c.last=Some(Instant::now());
            use WorkspaceAction as W;
            let run=|v:&mut Companion,a,ctx:&mut ViewContext<Self>| v.handle_action(&Action::Workspace(a),ctx);
            if std::env::var_os("STEPX_CHAT_FIRST_NATIVE").is_some() && c.step > 0 {
                let w = facloud_ui::windowing::winit::winit_window_for_id(ctx,ctx.window_id()).unwrap(); let origin=w.inner_position().unwrap();
                let positions=["workspace-intent-anchor","context-surface","context-choice-0","context-choice-1","context-choice-2","context-choice-3"].into_iter().map(|id|(id,ctx.element_position_by_id(id.to_string()).map(|r|[r.origin().x(),r.origin().y(),r.width(),r.height()]))).collect::<std::collections::BTreeMap<_,_>>();
                let value=serde_json::json!({"origin":[origin.x,origin.y],"scale":w.scale_factor(),"positions":positions,"menu":self.context_menu.is_some(),"intent":self.workspace.intent,"running":self.workspace_busy(),"panel":self.workspace.panel});
                std::fs::write(dir.join("native-state.tmp"),value.to_string()).unwrap();std::fs::rename(dir.join("native-state.tmp"),dir.join("native-state.json")).unwrap();
                if std::fs::remove_file(dir.join("native-capture")).is_ok(){capture(ctx,dir.join("native-menu.png"));ctx.notify();}
                if std::fs::remove_file(dir.join("native-exit")).is_ok(){ctx.close_window();c.step=99;}
                return;
            }
            let screenshot = match c.step { 1=>Some("01-chat.png"),2=>Some("02-dropdown-below.png"),4=>Some("03-inline-confirmation.png"),5=>Some("04-compact-chat-draft.png"),6=>Some("05-compact-dropdown.png"),8=>Some("06-compact-conversation.png"),9=>Some("07-dropdown-above.png"),10=>Some("08-full-conversation.png"),_=>None };
            if c.captured != Some(c.step) { if let Some(name) = screenshot { capture(ctx,dir.join(name)); c.captured=Some(c.step); ctx.notify(); return; } }
            match c.step {
                0=>{self.set_compact_mode(ShellMode::Full,ctx); self.amoo_sessions.clear(); run(self,W::New,ctx); self.page=Page::Amoo; assert_eq!(self.workspace.intent,Intent::Auto);},
                1=>{click(ctx,"workspace-intent-anchor");},
                2=>{
                    assert_eq!(self.workspace.panel,0);assert!(self.context_menu.is_some());
                    let anchor=ctx.element_position_by_id("workspace-intent-anchor".to_string()).unwrap();
                    let menu=ctx.element_position_by_id("context-surface".to_string()).unwrap();
                    assert!(menu.origin().y()>=anchor.origin().y()+anchor.height(),"roomy menu must open below");
                    assert!((menu.origin().x()-anchor.origin().x()).abs()<1.,"menu must align with trigger");

                    click(ctx,"context-choice-1");
                },
                3=>{
                    assert_eq!(self.workspace.intent,Intent::Files);assert!(self.context_menu.is_none());assert!(self.workspace.proposal.is_none());assert!(!self.workspace_busy());
                    c.attachment=dir.join("read-only.csv"); std::fs::write(&c.attachment,"item,amount\nA,12\nB,18\n").unwrap(); self.add_amoo_paths(&[c.attachment.clone()]).unwrap();
                    self.fields[F_AMOO].set("计算合计并整理结果表");run(self,W::Prepare,ctx);
                    assert!(self.workspace.proposal.is_none());assert!(!self.workspace_busy());
                    assert_eq!(self.workspace_task().unwrap().route,Some(Intent::Files));
                    c.task=self.workspace.active.clone().unwrap();
                },
                4=>{run(self,W::Reroute(Intent::Chat),ctx);assert_eq!(self.workspace.intent,Intent::Auto);assert_eq!(self.workspace_task().unwrap().route,None);assert!(self.workspace_task().unwrap().route_note.is_empty());assert_eq!(self.amoo_attachments.len(),1);assert_eq!(self.fields[F_AMOO].text,"计算合计并整理结果表");self.set_compact_mode(ShellMode::MiniAi,ctx);},
                5=>{assert_eq!(self.workspace.active.as_ref(),Some(&c.task));run(self,W::Menu(7),ctx);},
                6=>{assert_eq!(self.workspace.panel,0);self.handle_action(&Action::Escape,ctx);assert!(self.context_menu.is_none());run(self,W::New,ctx);self.fields[F_AMOO].set("另一个会话的草稿");run(self,W::Select(c.task.clone()),ctx);assert_eq!(self.amoo_attachments[0].local_path.as_ref(),Some(&c.attachment));assert_eq!(self.fields[F_AMOO].text,"计算合计并整理结果表");},
                7=>{
                    // Filled conversation pins the composer at the bottom, forcing the menu upward.
                    self.amoo_attachments.clear();self.fields[F_AMOO].clear();
                    let task=self.workspace.journal.tasks.iter_mut().find(|t|t.id==c.task).unwrap();
                    task.turns=vec![stepx_companion::amoo_workspace::Turn{user:true,text:"这份表格合计多少？".into()},stepx_companion::amoo_workspace::Turn{user:false,text:"合计 **30**。两项分别为 12 和 18。".into()}];
                    task.steps=vec![stepx_companion::amoo_workspace::Step{executor:Executor::Phone,phase:stepx_companion::amoo_workspace::Phase::Done,note:String::new()}];
                    task.session=None;self.reveal_latest_chat();
                },
                8=>{aligned(ctx);click(ctx,"workspace-intent-anchor");},
                9=>{
                    let anchor=ctx.element_position_by_id("workspace-intent-anchor".to_string()).unwrap();let menu=ctx.element_position_by_id("context-surface".to_string()).unwrap();
                    assert!(menu.origin().y()+menu.height()<=anchor.origin().y(),"bottom menu must flip above");
                    assert_eq!(self.push.shell_mode,ShellMode::MiniAi);pointer(ctx,vec2f(10.,150.));
                },
                10=>{assert!(self.context_menu.is_none());self.set_compact_mode(ShellMode::Full,ctx);},
                11=>{
                    run(self,W::New,ctx);self.fields[F_AMOO].set("聊天失败时仍然保留草稿");
                    run(self,W::Prepare,ctx);assert!(self.workspace.proposal.is_none());assert!(!self.workspace_busy());assert_eq!(self.fields[F_AMOO].text,"聊天失败时仍然保留草稿");assert!(self.workspace.error.is_some());
                    self.add_amoo_paths(&[c.attachment.clone()]).unwrap();std::fs::remove_file(&c.attachment).unwrap();run(self,W::Prepare,ctx);
                    assert!(self.workspace.error.as_ref().is_some_and(|e|e.contains("附件")));assert_eq!(self.amoo_attachments.len(),1);assert!(self.workspace.proposal.is_none());
                },
                12=>{self.amoo_attachments.clear();self.fields[F_AMOO].clear();run(self,W::New,ctx);self.error=false;self.status="就绪".into();self.set_compact_mode(ShellMode::Full,ctx);let w=facloud_ui::windowing::winit::winit_window_for_id(ctx,ctx.window_id()).unwrap();let _=w.request_inner_size(winit::dpi::LogicalSize::new(960.,680.));},
                13=>{let rect=ctx.element_position_by_id("workspace-chat-welcome".to_string()).unwrap();assert!((rect.origin().x()+rect.width()/2.-(216.+(960.-216.)/2.)).abs()<4.,"welcome not centered {rect:?}");assert!(rect.width()<=960.-216.-24.);capture(ctx,dir.join("09-full-960.png"));},
                14=>{self.set_compact_mode(ShellMode::MiniAi,ctx);let w=facloud_ui::windowing::winit::winit_window_for_id(ctx,ctx.window_id()).unwrap();let _=w.request_inner_size(winit::dpi::LogicalSize::new(440.,620.));},
                15=>{let rect=ctx.element_position_by_id("workspace-chat-welcome".to_string()).unwrap();assert!((rect.origin().x()+rect.width()/2.-220.).abs()<2.,"compact not centered {rect:?}");assert!(rect.width()<=416.);capture(ctx,dir.join("10-mini-440.png"));},
                _=>{eprintln!("CHAT_FIRST_UI passed: default chat, anchored below/above, keyboard select, Esc/outside close, explicit confirmation, cancel retention, full/compact and conversation draft preservation");c.step=99;ctx.close_window();return;}
            }
            c.step+=1;ctx.notify();
        });
    }
}

impl Companion {
    pub(super) fn verify_chat_first_live(&mut self, ctx: &mut ViewContext<Self>) {
        assert!(!self.demo_mode);
        let Some(dir) = self
            .capture
            .as_ref()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
        else {
            return;
        };
        CHAT_CHECK.with(|check| {
            let mut c=check.borrow_mut(); if c.step==99{return;}
            let since=*c.start.get_or_insert_with(Instant::now);
            assert!(since.elapsed()<Duration::from_secs(900),"live chat-first timeout step={} status={}",c.step,self.status);
            if c.last.is_some_and(|t|t.elapsed()<Duration::from_millis(1200)) || self.busy || self.initializing {return;}
            c.last=Some(Instant::now());
            let serial=std::env::var("STEPX_WORKSPACE_TEST_SERIAL").expect("explicit serial required");
            if self.selected.as_ref().is_none_or(|d|d.serial!=serial){self.handle_action(&Action::Select(serial),ctx);return;}
            if !self.studio_lab.loaded || self.workspace_busy() || self.workspace.next {return;}
            use WorkspaceAction as W;
            let run=|v:&mut Companion,a,ctx:&mut ViewContext<Self>| v.handle_action(&Action::Workspace(a),ctx);
            let reply=|v:&Companion| v.workspace_task().unwrap().turns.iter().rev().find(|t|!t.user).unwrap().text.clone();
            let send=|v:&mut Companion,prompt:&str,ctx:&mut ViewContext<Self>| {
                assert_eq!(v.workspace.intent,Intent::Auto);v.fields[F_AMOO].set(prompt);run(v,W::Prepare,ctx);
                assert!(v.workspace.proposal.is_none(),"chat must not create plan");assert!(v.workspace.running.is_some(),"{}",v.status);
                assert_eq!(v.workspace.running.as_ref().unwrap().executor,Executor::Phone);
            };
            match c.step {
                0=>{self.set_compact_mode(ShellMode::Full,ctx);run(self,W::New,ctx);send(self,"这是 StepX Companion 聊天验收。请只回答：7 乘 8 等于多少？不要执行其他任务。",ctx);c.task=self.workspace.active.clone().unwrap();},
                1=>{assert!(reply(self).contains("56"),"{}",reply(self));assert!(self.workspace_task().unwrap().workspace.is_none());capture(ctx,dir.join("01-real-chat.png"));},
                2=>{self.set_compact_mode(ShellMode::MiniAi,ctx);send(self,"把刚才的计算结果除以 7，只回复一个数字。",ctx);},
                3=>{assert!(reply(self).contains('8'),"{}",reply(self));assert_eq!(self.workspace.active.as_ref(),Some(&c.task));capture(ctx,dir.join("02-real-followup.png"));},
                4=>{self.add_amoo_paths(&[dir.join("vision.png")]).unwrap();send(self,"请直接看附件里的图片，读出上方六位数字，并按从左到右说出三个图形和颜色。只回答看见的内容，不操作手机。",ctx);},
                5=>{let response=reply(self);assert!(response.contains("724936"),"vision number missing: {response}");assert!(response.contains("红")&&response.contains("绿")&&response.contains("蓝"),"vision colors missing: {response}");assert_eq!(self.push.shell_mode,ShellMode::MiniAi);capture(ctx,dir.join("03-real-vision.png"));},
                6=>{let path=dir.join("memo.txt");std::fs::write(&path,"内部测试资料\n验收代号：NEBULA-49317\n周一收入 125 元，周二收入 275 元。\n").unwrap();self.add_amoo_paths(&[path]).unwrap();send(self,"阅读附件，告诉我验收代号和两天收入合计。只用文字回答，不生成文件，不启动工作流程。",ctx);},
                7=>{let response=reply(self);assert!(response.contains("49317")&&response.contains("400"),"file reading failed: {response}");assert!(self.workspace_task().unwrap().outputs.is_empty());capture(ctx,dir.join("04-real-file-question.png"));},
                8=>{self.set_compact_mode(ShellMode::Full,ctx);let path=dir.join("figures.csv");std::fs::write(&path,"item,quantity,price\nA,2,15\nB,3,20\nC,1,10\n").unwrap();self.add_amoo_paths(&[path]).unwrap();self.fields[F_AMOO].set("验收：读取附件，计算每行金额和总计，生成 result.csv 与一段简短的 analysis.md。禁止外网，不读取其他个人文件。快速完成这两个小文件。");},
                9=>{capture(ctx,dir.join("05-real-confirmation.png"));},
                10=>{run(self,W::Prepare,ctx);assert!(self.workspace.proposal.is_none());assert!(self.workspace.running.is_some(),"{}",self.status);assert_eq!(self.workspace.intent,Intent::Auto);assert_eq!(self.workspace_task().unwrap().route,Some(Intent::Files));},
                11=>{let task=self.workspace_task().unwrap();assert!(task.steps.iter().all(|s|s.phase==stepx_companion::amoo_workspace::Phase::Done),"{:?}",task.steps);assert!(task.outputs.iter().any(|o|o.name.ends_with(".csv")));std::fs::write(dir.join("work-result.json"),serde_json::to_vec_pretty(task).unwrap()).unwrap();capture(ctx,dir.join("06-real-work-complete.png"));},
                12=>{send(self,"工作已完成，现在只继续普通聊天：刚才表格里的总金额是多少？请直接用自然语言回答，不生成新文件，不调用桌面工具。",ctx);},
                13=>{let response=reply(self);assert!(response.contains("100"),"post-work answer missing: {response}");assert_eq!(self.workspace_task().unwrap().steps[0].executor,Executor::Phone);assert!(!self.work.running);assert!(!response.contains("amoo_work_protocol"));capture(ctx,dir.join("07-real-back-to-chat.png"));},
                14=>{self.set_compact_mode(ShellMode::MiniAi,ctx);self.fields[F_AMOO].set("未发送的草稿：请继续");self.add_amoo_paths(&[dir.join("memo.txt")]).unwrap();run(self,W::Preview(0),ctx);},
                15=>{assert_eq!(self.push.shell_mode,ShellMode::MiniAi);capture(ctx,dir.join("08-real-compact-preview.png"));},
                16=>{run(self,W::ClosePanel,ctx);self.set_compact_mode(ShellMode::Full,ctx);assert_eq!(self.fields[F_AMOO].text,"未发送的草稿：请继续");assert_eq!(self.amoo_attachments.len(),1);self.set_compact_mode(ShellMode::MiniAi,ctx);assert_eq!(self.amoo_attachments.len(),1);std::fs::write(dir.join("live-result.json"),serde_json::to_vec_pretty(self.workspace_task().unwrap()).unwrap()).unwrap();},
                _=>{eprintln!("CHAT_FIRST_LIVE passed: text, contextual followup, actual vision, actual file reading, explicit file generation, post-work ordinary chat, preview/compact/full draft attachment retention");c.step=99;ctx.close_window();return;}
            }
            eprintln!("CHAT_FIRST_LIVE stage {} passed",c.step);c.step+=1;ctx.notify();
        });
    }
}
