//! Opt-in scenario matrix through production state, layout and preview workers.
use super::*;
use facloud_ui::TypedActionView;
use stepx_companion::amoo_workspace::{Intent, Output, Task, Turn};
const CASES: &[(bool, f32, f32)] = &[
    (true, 360., 480.),
    (true, 420., 520.),
    (true, 440., 620.),
    (true, 600., 760.),
    (true, 820., 560.),
    (false, 960., 600.),
    (false, 1080., 720.),
    (false, 1320., 860.),
    (false, 1600., 1000.),
];
#[derive(Default)]
struct Check {
    case: usize,
    step: usize,
    last: Option<Instant>,
    since: Option<Instant>,
    first: String,
    second: String,
    draft: String,
    files: Vec<AmooAttachment>,
    geometry: Vec<serde_json::Value>,
    welcome_y: f32,
}
thread_local! {static CHECK:RefCell<Check>=RefCell::new(Check::default());}
fn capture(ctx: &mut ViewContext<Companion>, path: PathBuf) {
    ctx.windows()
        .platform_window(ctx.window_id())
        .unwrap()
        .as_ctx()
        .request_frame_capture(Box::new(move |f| {
            image::save_buffer(path, &f.data, f.width, f.height, image::ColorType::Rgba8).unwrap()
        }));
}
fn resize(ctx: &mut ViewContext<Companion>, width: f32, height: f32) {
    let w = facloud_ui::windowing::winit::winit_window_for_id(ctx, ctx.window_id()).unwrap();
    w.set_title("StepX Companion · 窗口场景验证");
    let _ = w.request_inner_size(winit::dpi::LogicalSize::new(width, height));
}
fn rect(ctx: &ViewContext<Companion>, id: &str) -> pathfinder_geometry::rect::RectF {
    ctx.element_position_by_id(id.to_owned())
        .unwrap_or_else(|| panic!("missing region {id}"))
}
fn geometry(v: &Companion, ctx: &ViewContext<Companion>, label: &str) -> serde_json::Value {
    let names = [
        "composer-surface",
        "composer-editor",
        "composer-attachments",
        "composer-toolbar",
        "composer-send",
        "workspace-chat-heading",
        "workspace-chat-messages",
        "workspace-chat-input",
    ];
    let areas = names
        .into_iter()
        .filter_map(|id| {
            ctx.element_position_by_id(id.to_owned())
                .map(|r| (id, [r.origin().x(), r.origin().y(), r.width(), r.height()]))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    serde_json::json!({"scene":label,"viewport":[v.workspace.viewport.x(),v.workspace.viewport.y()],"regions":areas,"draft_bytes":v.fields[F_AMOO].text.len(),"attachments":v.amoo_attachments.len(),"editor_scroll":v.amoo_ui.composer_scroll.scroll_start().as_f32(),"chat_metrics":v.chat_scroll_metrics.get(),"mode":format!("{:?}",v.push.shell_mode)})
}
fn check_composer(v: &Companion, ctx: &ViewContext<Companion>) {
    let s = rect(ctx, "composer-surface");
    let editor = rect(ctx, "composer-editor");
    let send = rect(ctx, "composer-send");
    let toolbar = rect(ctx, "composer-toolbar");
    let w = v.workspace.viewport.x();
    let h = v.workspace.viewport.y();
    assert!(
        s.origin().x() >= 0. && s.max_x() <= w + 1.,
        "composer outside width: {s:?} in {w}"
    );
    assert!(
        s.max_y() <= h - 27.,
        "composer covers status bar: {s:?} in {h}"
    );
    assert!(
        send.origin().x() >= s.origin().x() && send.max_x() <= s.max_x(),
        "send clipped {send:?} {s:?}"
    );
    assert!(
        send.origin().y() >= toolbar.origin().y() && send.max_y() <= toolbar.max_y() + 1.,
        "send outside toolbar"
    );
    assert!(
        editor.max_y() <= toolbar.origin().y(),
        "text overlaps toolbar"
    );
    if v.focus == Some(F_AMOO) && v.fields[F_AMOO].text.len() > 500 {
        let caret = rect(ctx, &format!("input-caret:{F_AMOO}"));
        assert!(
            caret.origin().y() >= editor.origin().y() && caret.max_y() <= editor.max_y() + 1.,
            "caret outside editor: {caret:?} {editor:?}"
        );
    }
    if v.workspace_task().is_some_and(|t| !t.turns.is_empty()) {
        // Reserve at least four readable text lines, beyond the pinned heading.
        assert!(
            v.chat_scroll_metrics.get().1 >= 96.,
            "composer squeezed conversation: {:?}",
            v.chat_scroll_metrics.get()
        );
    }
}
impl Companion {
    pub(super) fn verify_responsive(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(dir) = self
            .capture
            .as_ref()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
        else {
            return;
        };
        assert!(self.demo_mode);
        let fixtures = dir.parent().unwrap().join("fixtures");
        CHECK.with(|cell| {
            let mut c=cell.borrow_mut();if c.case>=CASES.len(){return;}
            let since=*c.since.get_or_insert_with(Instant::now);assert!(since.elapsed()<Duration::from_secs(360),"matrix timeout {}:{}",c.case,c.step);
            if self.initializing||self.busy||c.last.is_some_and(|t|t.elapsed()<Duration::from_millis(650)){return;}c.last=Some(Instant::now());
            let (compact,width,height)=CASES[c.case];let mode=if compact{ShellMode::MiniAi}else{ShellMode::Full};
            // A background X11 window can paint later than its resize event.
            // Assert only a frame laid out at the requested size, never old caches.
            if !matches!(c.step,0|11) && ctx.element_position_by_id("test-window-layout".to_owned()).is_none_or(|r|(r.width()-width).abs()>1. || (r.height()-height).abs()>1.) {ctx.notify();return;}
            let name=format!("{}-{}x{}",if compact{"mini"}else{"full"},width as u32,height as u32);
            let run=|v:&mut Companion,a,ctx:&mut ViewContext<Self>|v.handle_action(&Action::Workspace(a),ctx);
            use WorkspaceAction as W;
            match c.step {
                0=>{
                    self.set_compact_mode(mode,ctx);resize(ctx,width,height);self.page=Page::Amoo;self.amoo_sessions.clear();run(self,W::New,ctx);self.error=false;self.status="正在验证窗口布局".into();
                    let mut task=Task::new(None,None);task.title="多场景测试：保留当前对话中的草稿与全部附件".into();task.workspace=Some(fixtures.clone());
                    task.turns=vec![Turn{at_ms:None,user:true,text:"请核对资料，说明下一步。".into()},Turn{at_ms:None,user:false,text:"## 核对结果\n\n资料已准备好。**仍在编辑的文字和附件应保持原样**。\n\n1. 查看图片。\n2. 阅读表格与文档。\n3. 在另一条会话中讨论，再返回继续。\n\n| 项目 | 数量 |\n|---|---:|\n| 甲 | 3 |\n| 乙 | 4 |\n\n这是用于检查文字选择、长段落换行和窗口缩放的内容。".repeat(3)}];
                    for (file,kind) in [("图像识别与附件预览_红方块绿圆形蓝三角.png","图片"),("项目资料_中文名称与空格 单价数量.csv","表格")] {task.add_output(Output{path:fixtures.join(file),name:file.into(),kind:kind.into(),lab_id:None});}
                    c.first=task.id.clone();self.workspace.journal.tasks.push(task);run(self,W::Select(c.first.clone()),ctx);self.focus_field(F_AMOO,ctx);
                },
                1=>{c.geometry.push(geometry(self,ctx,&format!("{name}-empty")));check_composer(self,ctx);},
                2=>{
                    run(self,W::Intent(Intent::Files),ctx);
                    c.draft=(0..48).map(|i|format!("第 {} 项：请保留中文标点、换行和表情 🌙，逐项核对预算与附件内容。https://example.test/long-path/abcdefghijklmnopqrstuvwxyz\n",i+1)).collect();
                    self.handle_action(&Action::Insert(c.draft.clone()),ctx);
                    let paths=["图像识别与附件预览_红方块绿圆形蓝三角.png","项目资料_中文名称与空格 单价数量.csv","需求说明_完整保留中文标点和草稿.txt","预览报告.md"].map(|p|fixtures.join(p));self.add_amoo_paths(&paths).unwrap();c.files=self.amoo_attachments.clone();
                },
                3=>{check_composer(self,ctx);assert_eq!(self.fields[F_AMOO].text,c.draft);assert!(self.amoo_ui.composer_scroll.scroll_start().as_f32()>0.,"long draft caret not revealed");c.geometry.push(geometry(self,ctx,&format!("{name}-long")));capture(ctx,dir.join(format!("{name}-long.png")));},
                4=>{run(self,W::Preview(0),ctx);},
                5=>{if self.work.local_preview_loading.is_some(){return;}assert!(self.work.local_preview.as_ref().is_some_and(|p|p.image),"image preview failed {:?}",self.work.local_preview_error);assert_eq!(self.push.shell_mode,mode);assert_eq!(self.fields[F_AMOO].text,c.draft);capture(ctx,dir.join(format!("{name}-image.png")));},
                6=>{run(self,W::Preview(1),ctx);},
                7=>{if self.work.local_preview_loading.is_some(){return;}assert!(self.work.local_preview.as_ref().and_then(|p|p.text.as_ref()).is_some_and(|t|t.contains("quantity")),"csv preview failed");assert_eq!(self.amoo_attachments,c.files);run(self,W::ClosePanel,ctx);run(self,W::History,ctx);},
                8=>{capture(ctx,dir.join(format!("{name}-history.png")));run(self,W::New,ctx);self.fields[F_AMOO].set("另一条会话的草稿，不要覆盖原会话。");self.add_amoo_paths(&[fixtures.join("需求说明_完整保留中文标点和草稿.txt")]).unwrap();run(self,W::History,ctx);c.second=self.workspace.active.clone().unwrap();run(self,W::Select(c.first.clone()),ctx);},
                9=>{assert_eq!(self.fields[F_AMOO].text,c.draft);assert_eq!(self.amoo_attachments,c.files);self.handle_action(&Action::AmooRemoveAttachment(2),ctx);c.files.remove(2);assert_eq!(self.fields[F_AMOO].text,c.draft);run(self,W::Select(c.second.clone()),ctx);assert_eq!(self.fields[F_AMOO].text,"另一条会话的草稿，不要覆盖原会话。");assert_eq!(self.amoo_attachments.len(),1);run(self,W::Select(c.first.clone()),ctx);},
                10=>{self.set_compact_mode(if compact{ShellMode::Full}else{ShellMode::MiniAi},ctx);},
                11=>{assert_eq!(self.fields[F_AMOO].text,c.draft);assert_eq!(self.amoo_attachments,c.files);self.set_compact_mode(mode,ctx);resize(ctx,width,height);},
                12=>{assert_eq!(self.fields[F_AMOO].text,c.draft);assert_eq!(self.amoo_attachments,c.files);check_composer(self,ctx);c.geometry.push(geometry(self,ctx,&format!("{name}-restored")));capture(ctx,dir.join(format!("{name}-restored.png")));run(self,W::Menu(7),ctx);},
                13=>{let menu=rect(ctx,"context-surface");assert!(menu.origin().x()>=0. && menu.max_x()<=width+1. && menu.origin().y()>=0. && menu.max_y()<=height+1.,"dropdown outside window: {menu:?}");self.handle_action(&Action::Escape,ctx);run(self,W::New,ctx);self.focus_field(F_AMOO,ctx);},
                14=>{c.welcome_y=rect(ctx,"composer-surface").origin().y();self.handle_action(&Action::Preedit("zhong wen".into()),ctx);assert!(self.fields[F_AMOO].text.is_empty());},
                15=>{self.handle_action(&Action::Insert("中文输入测试，光标和输入框不应无故跳动。\n".repeat(22)),ctx);self.add_amoo_paths(&[fixtures.join("图像识别与附件预览_红方块绿圆形蓝三角.png")]).unwrap();},
                16=>{check_composer(self,ctx);let shift=rect(ctx,"composer-surface").origin().y()-c.welcome_y;assert!(shift.abs()<1.,"welcome editor jumped {shift}px");c.geometry.push(serde_json::json!({"scene":format!("{name}-welcome-growth"),"composer_top_shift":shift}));capture(ctx,dir.join(format!("{name}-welcome-long.png")));eprintln!("RESPONSIVE case {name} passed; welcome input shift {shift}");c.case+=1;c.step=0;if c.case==CASES.len(){std::fs::write(dir.join("geometry.json"),serde_json::to_vec_pretty(&c.geometry).unwrap()).unwrap();eprintln!("RESPONSIVE passed: 9 sizes, long Chinese input, mixed attachments, image/CSV previews, conversation drafts, removal and shell switches");ctx.close_window();return;}ctx.notify();return;},
                _=>unreachable!(),
            }
            c.step+=1;ctx.notify();
        });
    }
}
