//! Opt-in native acceptance; live mode uses production actions and actual workers.
use super::*;
use facloud_ui::TypedActionView;
use stepx_companion::amoo_workspace::{Executor, Intent, Output, Phase, Step, Task, Turn};
#[derive(Default)]
struct Check {
    step: usize,
    last: Option<Instant>,
    since: Option<Instant>,
    task: String,
    attachments: Vec<AmooAttachment>,
    app: bool,
}
thread_local! {static CHECK:RefCell<Check>=RefCell::new(Check::default());}
fn capture(ctx: &mut ViewContext<Companion>, path: PathBuf) {
    if let Some(w) = ctx.windows().platform_window(ctx.window_id()) {
        w.as_ctx().request_frame_capture(Box::new(move |f| {
            image::save_buffer(path, &f.data, f.width, f.height, image::ColorType::Rgba8).unwrap();
        }));
    }
}
impl Companion {
    pub(super) fn verify_workspace(&mut self, ctx: &mut ViewContext<Self>) {
        if std::env::var_os("STEPX_SCENARIO_DIR").is_some() {
            self.scenario_tick(ctx);
            return;
        }
        let live = std::env::args().any(|a| a == "--verify-workspace-live");
        if !live && !std::env::args().any(|a| a == "--verify-workspace") {
            return;
        }
        assert_eq!(self.demo_mode, !live);
        if std::env::var_os("STEPX_ENTER_TEST").is_some() {
            self.verify_enter(ctx, live);
            return;
        }
        if std::env::var_os("STEPX_RESPONSIVE_TEST").is_some() {
            if live {
                self.verify_responsive_live(ctx);
            } else {
                self.verify_responsive(ctx);
            }
            return;
        }
        if std::env::var_os("STEPX_CHAT_FIRST_TEST").is_some() {
            if live {
                self.verify_chat_first_live(ctx);
            } else {
                self.verify_chat_first(ctx);
            }
            return;
        }
        let Some(dir) = self
            .capture
            .as_ref()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
        else {
            return;
        };
        CHECK.with(|c|{
            let mut c=c.borrow_mut();if c.step==99{return;}
            let since=*c.since.get_or_insert_with(Instant::now);
            if since.elapsed()>Duration::from_secs(if live{900}else{120}){eprintln!("WORKSPACE_CHECK timeout step={} status={}",c.step,self.status);std::process::exit(2);}
            if c.last.is_some_and(|t|t.elapsed()<Duration::from_millis(1200)){return;}c.last=Some(Instant::now());
            if self.busy||self.initializing{return;}
            let action=|a|Action::Workspace(a);
            use WorkspaceAction as W;
            if live {
                let serial=std::env::var("STEPX_WORKSPACE_TEST_SERIAL").expect("explicit test phone");
                if self.selected.as_ref().is_none_or(|d|d.serial!=serial){self.handle_action(&Action::Select(serial),ctx);return;}
                if !self.studio_lab.loaded{return;}
                if std::env::var_os("STEPX_WORKSPACE_CHAT_TEST").is_some() {
                    match c.step {
                        0=>{
                            self.set_compact_mode(ShellMode::Full,ctx);
                            self.handle_action(&action(W::New),ctx);
                            self.fields[F_AMOO].set("这是 StepX Companion 对话验收。请只回复：对话连接正常，7 乘 8 等于 56。不要执行其他任务。");
                            assert_eq!(self.workspace.intent,Intent::Auto);
                            self.handle_action(&action(W::Prepare),ctx);
                            assert!(self.workspace.proposal.is_none());
                            assert!(self.workspace.running.is_some(),"{}",self.status);
                            c.task=self.workspace.active.clone().unwrap();
                        },
                        1=>{
                            if self.workspace_busy(){return;}
                            let task=self.workspace_task().unwrap();
                            assert!(task.steps.iter().all(|s|s.phase==Phase::Done),"{:?}",task.steps);
                            let session=self.amoo_sessions.iter().find(|s|Some(&s.session_key)==task.session.as_ref()).unwrap();
                            let response=session.items.iter().filter_map(|m|match m {AmooChatItem::AssistantRich{text:Some(t),..}=>Some(t.as_str()),_=>None}).collect::<Vec<_>>().join("\n");
                            assert!(response.contains("56"),"no actual answer received");
                            std::fs::write(dir.join("chat-result.json"),serde_json::to_vec_pretty(task).unwrap()).unwrap();
                            capture(ctx,dir.join("chat-full.png"));
                        },
                        2=>{self.set_compact_mode(ShellMode::MiniAi,ctx);self.fields[F_AMOO].set("未发送草稿：谢谢");},
                        3=>{assert_eq!(self.workspace.active.as_ref(),Some(&c.task));assert_eq!(self.fields[F_AMOO].text,"未发送草稿：谢谢");capture(ctx,dir.join("chat-compact.png"));},
                        _=>{eprintln!("WORKSPACE_CHAT passed: direct production send, actual answer 56, verified completion, compact preserves task and draft");c.step=99;ctx.close_window();return;}
                    }
                    c.step+=1;ctx.notify();return;
                }
                match c.step {
                    0=>{
                        self.set_compact_mode(ShellMode::Full,ctx);
                        if let Ok(id)=std::env::var("STEPX_WORKSPACE_REVIEW_TASK") {
                            if !self.workspace.journal.tasks.iter().any(|t|t.id==id && self.workspace_visible(t)){return;}
                            self.handle_action(&action(W::Select(id.clone())),ctx);c.task=id;
                            assert!(self.workspace_task().unwrap().title.contains("统一任务验收"));c.step=2;ctx.notify();return;
                        }
                        c.app=false;
                        self.handle_action(&action(W::New),ctx);
                        let file=dir.join("sales-acceptance.csv");std::fs::write(&file,"product,quantity,price\nAlpha,2,15\nBeta,3,20\nGamma,1,10\n").unwrap();
                        self.add_amoo_paths(&[file]).unwrap();
                        self.fields[F_AMOO].set("这是 StepX Companion 统一任务验收。分析附件销售表格：计算各项金额及合计，生成 result.csv 和 app-brief.md；再做一个手机销售看板，显示三项金额与总计，带一个显示/隐藏明细按钮。禁止使用外网，不读取其他个人文件。应用由下一阶段完成。请快速完成这个小任务。");
                        if !c.app {self.fields[F_AMOO].set("这是 StepX Companion 统一任务验收。分析附件销售表格，计算各项金额及合计，生成 result.csv 和简短的 analysis.md。禁止使用外网，不读取其他个人文件。请快速完成这个小任务。");}
                        assert_eq!(self.workspace.intent,Intent::Auto);
                        self.handle_action(&action(W::Prepare),ctx);
                        assert!(self.workspace.proposal.is_none());
                        assert_eq!(self.workspace_task().unwrap().route,Some(Intent::Files));
                        assert!(self.workspace.running.is_some(),"{}",self.status);
                        c.task=self.workspace.active.clone().unwrap();
                    },
                    1=>{capture(ctx,dir.join("live-plan.png"));},
                    2=>{
                        if self.workspace_busy()||self.workspace.next{return;}
                        let t=self.workspace_task().unwrap();
                        std::fs::write(dir.join("live-result.json"),serde_json::to_vec_pretty(t).unwrap()).unwrap();
                        assert!(t.steps.iter().all(|s|s.phase==Phase::Done),"{:?}",t.steps);
                        assert!(t.outputs.iter().any(|o|o.name.ends_with(".csv")),"missing real file");
                        let index=if c.app{t.outputs.iter().position(|o|o.lab_id.is_some()).expect("missing real validated app")}else{t.outputs.iter().position(|o|o.name.ends_with(".csv")).unwrap()};
                        self.handle_action(&action(W::Preview(index)),ctx);
                    },
                    3=>capture(ctx,dir.join("live-results.png")),
                    4=>{self.handle_action(&action(W::ClosePanel),ctx);self.fields[F_AMOO].set("未发送草稿：继续改进配色");self.set_compact_mode(ShellMode::MiniAi,ctx);},
                    5=>{assert_eq!(self.workspace.active.as_ref(),Some(&c.task));assert_eq!(self.fields[F_AMOO].text,"未发送草稿：继续改进配色");capture(ctx,dir.join("live-compact.png"));},
                    6=>{self.fields[F_AMOO].clear();self.set_compact_mode(ShellMode::Full,ctx);let i=self.workspace_task().unwrap().outputs.iter().position(|o|o.name=="result.csv").unwrap();self.handle_action(&action(W::Preview(i)),ctx);},
                    7=>{self.handle_action(&action(W::Deliver(false)),ctx);},
                    8=>capture(ctx,dir.join("live-delivery-confirm.png")),
                    9=>{self.handle_action(&action(W::CancelDelivery),ctx);assert!(self.workspace.delivery.is_none());self.handle_action(&action(W::Deliver(false)),ctx);self.handle_action(&action(W::ConfirmDelivery),ctx);},
                    10=>{let task=self.workspace_task().unwrap();assert!(task.receipts.iter().any(|r|r.path.file_name().is_some_and(|n|n=="result.csv")&&r.success),"{}",self.status);std::fs::write(dir.join("live-delivery.json"),serde_json::to_vec_pretty(&task.receipts).unwrap()).unwrap();capture(ctx,dir.join("live-delivered.png"));},
                    11=>{self.handle_action(&action(W::Menu(4)),ctx);},
                    12=>capture(ctx,dir.join("live-studio-projects.png")),
                    _=>{eprintln!("WORKSPACE_LIVE passed: phone CSV -> real files, same task, compact draft retention, verified file delivery; app_live_verified={}; no publishing",c.app);c.step=99;ctx.close_window();return;}
                }
            } else {
                match c.step {
                    0=>{self.amoo_attachments.clear();self.fields[F_AMOO].clear();self.workspace.active=None;self.amoo_sessions.clear();self.page=Page::Amoo;self.set_compact_mode(ShellMode::Full,ctx);},
                    1=>capture(ctx,dir.join("01-start.png")),
                    2=>{
                        let file=dir.join("sales.csv");std::fs::write(&file,"item,total\nA,100\nB,200\n").unwrap();self.add_amoo_paths(&[file]).unwrap();
                        self.fields[F_AMOO].set("分析销售表格，生成汇报 PPT，再做一个手机看板");
                        assert_eq!(self.workspace.intent,Intent::Auto);
                        self.handle_action(&action(W::Prepare),ctx);
                        assert!(self.workspace.proposal.is_none());
                        assert_eq!(self.workspace_task().unwrap().route_note,"先整理文件再做应用");
                        assert_eq!(self.workspace_task().unwrap().route,Some(Intent::App));
                        assert!(!self.workspace_busy());
                        c.task=self.workspace.active.clone().unwrap();c.attachments=self.amoo_attachments.clone();
                    },
                    3=>capture(ctx,dir.join("02-plan.png")),
                    4=>{self.handle_action(&action(W::Reroute(Intent::Chat)),ctx);assert_eq!(self.workspace_task().unwrap().route,None);self.handle_action(&action(W::New),ctx);self.fields[F_AMOO].set("第二个任务的草稿");self.handle_action(&action(W::History),ctx);self.handle_action(&action(W::Select(c.task.clone())),ctx);assert_eq!(self.amoo_attachments,c.attachments);assert!(self.fields[F_AMOO].text.starts_with("分析销售表格"));self.set_compact_mode(ShellMode::MiniAi,ctx);},
                    5=>{assert_eq!(self.amoo_attachments,c.attachments);capture(ctx,dir.join("03-compact-draft.png"));},
                    6=>{self.handle_action(&action(W::Menu(2)),ctx);},
                    7=>{assert_eq!(self.push.shell_mode,ShellMode::MiniAi);capture(ctx,dir.join("04-compact-resources.png"));self.handle_action(&action(W::ClosePanel),ctx);self.set_compact_mode(ShellMode::Full,ctx);},
                    8=>{
                        let mut task=Task::new(None,None);task.title="销售复盘与汇报 · 布局测试".into();task.workspace=Some(dir.clone());task.turns=vec![Turn{user:true,text:"核对销售数据，整理结果表。".into()},Turn{user:false,text:"已核对两项数据，合计 **300**。结果已保存在电脑，可继续提问或发送到手机。".into()}];task.steps=vec![Step{executor:Executor::Files,phase:Phase::Done,note:"本地 UI 测试样例".into()}];task.add_output(Output{path:dir.join("sales.csv"),name:"核对结果.csv".into(),kind:"表格".into(),lab_id:None});let id=task.id.clone();self.workspace.journal.tasks.push(task);self.handle_action(&action(W::Select(id)),ctx);self.handle_action(&action(W::Preview(0)),ctx);
                    },
                    9=>capture(ctx,dir.join("05-results.png")),
                    10=>{self.set_compact_mode(ShellMode::MiniAi,ctx);},
                    11=>{assert_eq!(self.push.shell_mode,ShellMode::MiniAi);capture(ctx,dir.join("06-compact-preview.png"));self.handle_action(&action(W::ClosePanel),ctx);self.handle_action(&action(W::Select(c.task.clone())),ctx);},
                    12=>{assert_eq!(self.amoo_attachments,c.attachments);self.set_compact_mode(ShellMode::Full,ctx);self.handle_action(&action(W::Settings),ctx);},
                    13=>capture(ctx,dir.join("07-models.png")),
                    _=>{eprintln!("WORKSPACE_CHECK passed: ordered proposal/no execution, per-task draft+attachments, compact/full preservation, native preview stays compact");c.step=99;ctx.close_window();return;}
                }
            }
            c.step+=1;ctx.notify();
        });
    }
}
