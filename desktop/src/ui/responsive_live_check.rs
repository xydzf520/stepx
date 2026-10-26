//! Explicit real-device acceptance. No personal files or external contacts are used.
use super::*;
use facloud_ui::TypedActionView;
use stepx_companion::amoo_workspace::{Executor, Intent, Phase};

#[derive(Default)]
struct Check {
    step: usize,
    last: Option<Instant>,
    since: Option<Instant>,
    first: String,
    second: String,
    draft: String,
    captured: Option<usize>,
}
thread_local! { static CHECK: RefCell<Check> = RefCell::new(Check::default()); }
fn capture(ctx: &mut ViewContext<Companion>, path: PathBuf) {
    ctx.windows()
        .platform_window(ctx.window_id())
        .unwrap()
        .as_ctx()
        .request_frame_capture(Box::new(move |f| {
            image::save_buffer(path, &f.data, f.width, f.height, image::ColorType::Rgba8).unwrap()
        }));
}
fn resize(ctx: &mut ViewContext<Companion>, w: f32, h: f32) {
    let window = facloud_ui::windowing::winit::winit_window_for_id(ctx, ctx.window_id()).unwrap();
    window.set_title("StepX Companion · 真机场景验证");
    let _ = window.request_inner_size(winit::dpi::LogicalSize::new(w, h));
}
impl Companion {
    pub(super) fn verify_responsive_live(&mut self, ctx: &mut ViewContext<Self>) {
        assert!(!self.demo_mode);
        let Some(dir) = self
            .capture
            .as_ref()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
        else {
            return;
        };
        let fixtures = dir.parent().unwrap().join("fixtures");
        CHECK.with(|cell| {
            let mut c=cell.borrow_mut();if c.step==99{return;}
            let since=*c.since.get_or_insert_with(Instant::now);
            assert!(since.elapsed()<Duration::from_secs(1200),"real scenarios timeout stage={} status={}",c.step,self.status);
            if c.last.is_some_and(|t|t.elapsed()<Duration::from_millis(1000)) || self.initializing {return;}
            c.last=Some(Instant::now());
            let serial=std::env::var("STEPX_WORKSPACE_TEST_SERIAL").expect("explicit serial required");
            if self.selected.as_ref().is_none_or(|d|d.serial!=serial){if !self.busy {self.handle_action(&Action::Select(serial),ctx);}return;}
            // Stage 1 deliberately edits while the real upload/reply is pending.
            if c.step!=1 && (self.busy||self.workspace_busy()||self.workspace.next||!self.studio_lab.loaded) {return;}
            use WorkspaceAction as W;
            let run=|v:&mut Companion,a,ctx:&mut ViewContext<Self>|v.handle_action(&Action::Workspace(a),ctx);
            // Resume already-produced fixtures for preview inspection without
            // sending another model request or generating duplicate documents.
            if c.step==0 {
                if let Ok(id)=std::env::var("STEPX_RESPONSIVE_REVIEW_TASK") {
                    if !self.workspace.journal.tasks.iter().any(|t|t.id==id&&self.workspace_visible(t)){return;}
                    self.set_compact_mode(ShellMode::MiniAi,ctx);resize(ctx,440.,620.);run(self,W::Select(id),ctx);c.step=10;ctx.notify();return;
                }
            }
            let image_name=match c.step {2=>Some("01-mixed-compact.png"),7=>Some("02-session-restored.png"),9=>Some("03-file-confirmation.png"),11=>Some("04-excel-preview.png"),12=>Some("05-word-preview.png"),_=>None};
            if let Some(name)=image_name {
                if c.captured!=Some(c.step) && self.work.local_preview_loading.is_none() {
                    capture(ctx,dir.join(name));c.captured=Some(c.step);ctx.notify();return;
                }
            }
            let reply=|v:&Companion|v.workspace_task().unwrap().turns.iter().rev().find(|t|!t.user).expect("actual reply").text.clone();
            let send=|v:&mut Companion,text:&str,ctx:&mut ViewContext<Self>| {
                assert_eq!(v.workspace.intent,Intent::Auto);v.fields[F_AMOO].set(text);run(v,W::Prepare,ctx);
                assert!(v.workspace.proposal.is_none());assert_eq!(v.workspace.running.as_ref().expect(&v.status).executor,Executor::Phone);
            };
            match c.step {
                0=>{
                    self.set_compact_mode(ShellMode::MiniAi,ctx);resize(ctx,440.,620.);self.page=Page::Amoo;run(self,W::New,ctx);
                    self.add_amoo_paths(&[fixtures.join("图像识别与附件预览_红方块绿圆形蓝三角.png"),fixtures.join("需求说明_完整保留中文标点和草稿.txt"),fixtures.join("项目资料_中文名称与空格 单价数量.csv")]).unwrap();
                    let prompt=format!("这是 StepX Companion 多附件验收，只分析本次提供的三份测试附件，不操作手机，不生成文件。\n{}\n最终简短回答：图片上方六位数字是什么，三个图形的颜色和形状是什么；文字资料的项目代号和剩余预算是什么；CSV 按数量乘单价的金额合计是多少？", "请保持三个附件的信息彼此区分，确实读到再回答，无法读取时明确说明。\n".repeat(12));
                    send(self,&prompt,ctx);c.first=self.workspace.active.clone().unwrap();
                },
                1=>{c.draft="等待回复期间的新草稿：下一步请解释结果，先不要发送。\n中文、emoji 🌙 和标点均需保留。".repeat(6);self.fields[F_AMOO].clear();self.focus_field(F_AMOO,ctx);self.handle_action(&Action::Insert(c.draft.clone()),ctx);resize(ctx,420.,520.);},
                2=>{
                    let r=reply(self);std::fs::write(dir.join("01-mixed-reply.txt"),&r).unwrap();
                    assert!(r.contains("482751") && r.contains("68125") && r.contains("1585") && r.contains("249"),"mixed attachment answer: {r}");
                    assert!(r.contains('红')&&r.contains('绿')&&r.contains('蓝'),"colors: {r}");
                    assert_eq!(self.fields[F_AMOO].text,c.draft,"async completion erased new draft");assert!(self.amoo_attachments.is_empty());assert!(self.workspace_task().unwrap().outputs.is_empty());
                    self.add_amoo_paths(&[fixtures.join("预览报告.md")]).unwrap();run(self,W::New,ctx);
                    send(self,"这是第二条测试对话，请只回答 23 加 19 的结果，不执行其他任务。",ctx);c.second=self.workspace.active.clone().unwrap();
                },
                3=>{assert!(reply(self).contains("42"));self.fields[F_AMOO].set("第二条会话：未发送的不同草稿");self.add_amoo_paths(&[fixtures.join("需求说明_完整保留中文标点和草稿.txt")]).unwrap();run(self,W::Select(c.first.clone()),ctx);},
                4=>{assert_eq!(self.fields[F_AMOO].text,c.draft);assert_eq!(self.amoo_attachments[0].name,"预览报告.md");self.set_compact_mode(ShellMode::Full,ctx);resize(ctx,960.,600.);},
                5=>{assert_eq!(self.fields[F_AMOO].text,c.draft);run(self,W::Select(c.second.clone()),ctx);},
                6=>{assert_eq!(self.fields[F_AMOO].text,"第二条会话：未发送的不同草稿");assert_eq!(self.amoo_attachments.len(),1);run(self,W::Select(c.first.clone()),ctx);self.set_compact_mode(ShellMode::MiniAi,ctx);resize(ctx,600.,760.);},
                7=>{assert_eq!(self.fields[F_AMOO].text,c.draft);assert_eq!(self.amoo_attachments[0].name,"预览报告.md");self.amoo_attachments.clear();send(self,"只根据本会话刚才附件的结果，预算还剩多少？表格合计加 1 是多少？只回答两个数字。",ctx);},
                8=>{let r=reply(self);assert!(r.contains("1585")&&r.contains("250"),"context lost: {r}");std::fs::write(dir.join("02-followup.txt"),r).unwrap();run(self,W::New,ctx);self.add_amoo_paths(&[fixtures.join("项目资料_中文名称与空格 单价数量.csv")]).unwrap();self.fields[F_AMOO].set("验收办公场景：读取附件 CSV，计算每行金额和总计。用桌面文件工具生成一份 Excel 结果表 result.xlsx（含计算公式）和一份简短 Word 报告 report.docx；二者必须基于附件的真实数据。不要联网，不读取其他个人文件。任务很小，请一次完成并验证文件可读。");},
                9=>{run(self,W::Prepare,ctx);assert!(self.workspace.proposal.is_none());assert!(self.workspace.running.is_some(),"{}",self.status);},
                10=>{let t=self.workspace_task().unwrap();std::fs::write(dir.join("03-office-result.json"),serde_json::to_vec_pretty(t).unwrap()).unwrap();assert!(t.steps.iter().all(|s|s.phase==Phase::Done),"{:?}",t.steps);assert!(t.outputs.iter().any(|o|o.name.ends_with(".xlsx")),"missing Excel");assert!(t.outputs.iter().any(|o|o.name.ends_with(".docx")),"missing Word");self.fields[F_AMOO].set("预览期间的草稿，保留给下一次发送");self.add_amoo_paths(&[fixtures.join("预览报告.md")]).unwrap();let i=self.workspace_task().unwrap().outputs.iter().position(|o|o.name.ends_with(".xlsx")).unwrap();run(self,W::Preview(i),ctx);},
                11=>{if self.work.local_preview_loading.is_some(){return;}assert!(self.work.local_preview.is_some(),"Excel preview {:?}",self.work.local_preview_error);assert_eq!(self.push.shell_mode,ShellMode::MiniAi);let i=self.workspace_task().unwrap().outputs.iter().position(|o|o.name.ends_with(".docx")).unwrap();run(self,W::Preview(i),ctx);},
                12=>{if self.work.local_preview_loading.is_some(){return;}assert!(self.work.local_preview.is_some(),"Word preview {:?}",self.work.local_preview_error);run(self,W::ClosePanel,ctx);},
                13=>{assert_eq!(self.fields[F_AMOO].text,"预览期间的草稿，保留给下一次发送");assert_eq!(self.amoo_attachments[0].name,"预览报告.md");assert_eq!(self.push.shell_mode,ShellMode::MiniAi);if std::env::var_os("STEPX_RESPONSIVE_REVIEW_TASK").is_some(){eprintln!("RESPONSIVE_PREVIEW passed: saved Excel/Word, draft and attachments retained");c.step=99;ctx.close_window();return;}self.amoo_attachments.clear();send(self,"现在继续普通对话，不再生成文件。刚才结果表中 B 项的金额是多少？一句话回答。",ctx);},
                14=>{assert!(reply(self).contains("100"),"{}",reply(self));assert_eq!(self.workspace.intent,Intent::Auto);assert_eq!(self.workspace_task().unwrap().route,None);assert!(!self.work.running);capture(ctx,dir.join("06-return-to-chat.png"));std::fs::write(dir.join("final-task.json"),serde_json::to_vec_pretty(self.workspace_task().unwrap()).unwrap()).unwrap();},
                _=>{eprintln!("RESPONSIVE_LIVE passed: mixed image/text/CSV upload, pending draft retention, two real phone sessions, full/compact sizes, context followup, real Excel/Word generation and compact previews, return to normal chat");c.step=99;ctx.close_window();return;}
            }
            eprintln!("RESPONSIVE_LIVE stage {} passed after {}s",c.step,since.elapsed().as_secs());c.step+=1;ctx.notify();
        });
    }
}
