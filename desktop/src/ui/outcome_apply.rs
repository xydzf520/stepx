//! Background job outcomes and state application for `Companion`.

use super::*;

pub(super) type ReadResult<T> = std::result::Result<Vec<T>, String>;

pub(super) fn read_result<T>(result: Result<Vec<T>>) -> ReadResult<T> {
    result.map_err(|error| format!("{error:#}"))
}

pub(super) enum Outcome {
    BulkFailed(Page, Option<ToolPage>, String),
    InputMethod(stepx_companion::input_method::Status, String),
    AmooUi(AmooUiOutcome),
    Work(WorkOutcome),
    StorageScanned(String, StorageReport, Option<toolbox::SystemCompression>),
    SystemCompression(String, toolbox::SystemCompression),
    ToolOperated(
        String,
        String,
        toolbox::OperationResult,
        Option<StorageReport>,
    ),
    Sms(ReadResult<Message>, Option<Capabilities>),
    SmsPrepared(SmsPlan),
    SmsSent(SmsSendStatus, ReadResult<Message>),
    SmsStatus(SmsSendStatus),
    PhoneCallPrepared(phone::DialPlan),
    DialSubmitted(String, String),
    CallHangup(String),
    AmooState(AmooSnapshot, String, bool),
    AmooRealData(RealAmooData, String),
    AmooObservationUnavailable,
    AmooCreated(String),
    AmooSent(amoo::PendingReply),
    WorkspaceDelivered(String, PathBuf, String, std::result::Result<String, String>),
    Transports(Vec<Transport>, Vec<Service>),
    Reattached(Device),
    Selected(
        Device,
        String,
        Option<Capabilities>,
        Option<RealAmooData>,
        ReadResult<Message>,
        ReadResult<CallRecord>,
        ReadResult<ContactEntry>,
    ),
    /// Content to show on the page that started the job (e.g. screen text).
    Text(String),
    /// Result that only needs a line in the status bar.
    Notice(String),
    Summary(String, Option<Capabilities>),
    Files(String, Vec<Entry>, Option<String>),
    Apps(Vec<AppInfo>),
    ApkInstalled(String, Vec<AppInfo>),
    Mirror(String, String, MirrorSession),
    Screenshot(tempfile::TempDir, PathBuf),
    ScreenshotWithNotice(tempfile::TempDir, PathBuf, String),
    Trash(Trashed),
    Restored,
    Qr(Option<String>),
    Paired(String),
    BackScreenImportedImage(PathBuf),
    Studio(StudioOutcome),
    StudioLab(StudioLabOutcome),
    PhoneData(ReadResult<CallRecord>, ReadResult<ContactEntry>),
    GoogleSuite(GoogleSuiteStatus, CompanionStatus, String),
    CompanionStatus(CompanionStatus, String),
    NeoStatus(stepx_companion::neo_agent::NeoStatus),
    SystemServices(Vec<SystemServiceInfo>, String),
    DeepAmooGranted(String),
    DeepBackScreenGranted(String),
    PhoneGallery(Vec<stepx_companion::toolbox::PhoneGalleryItem>, String),
    StepOsCalendarUpdated(Vec<stepx_companion::toolbox::PhoneCalendarEvent>, String),
    StepOsAlarmsUpdated(Vec<stepx_companion::toolbox::PhoneAlarmItem>, String),
    StepOsSuite(stepx_companion::toolbox::StepOsAiSuiteReport),
}

impl Companion {
    pub(super) fn apply(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::BulkFailed(page, _tool, err) => {
                self.record_notice("操作未完成", err.clone());
                if matches!(page, Page::Apps | Page::AppStore) {
                    self.apps_loaded = true;
                    self.apps_error = Some(err.clone());
                }
                if page == Page::Tools {
                    self.back_screen.stepos_suite_loading = false;
                }
                self.error = true;
                self.status = err;
            }
            Outcome::Work(result) => self.apply_work(result),
            Outcome::AmooUi(result) => self.apply_amoo_ui(result),
            Outcome::StorageScanned(serial, report, system) => {
                if self.selected.as_ref().is_some_and(|d| d.serial == serial) {
                    self.status = format!(
                        "已扫描 {} 个公开文件；{} 个文件未完成重复校验",
                        report.scanned_files, report.hash_skipped
                    );
                    self.storage_report = Some(report);
                    self.system_compression = system;
                    self.tools_pending = None;
                    self.tools_result_page = 0;
                }
            }
            Outcome::SystemCompression(serial, result) => {
                if self.selected.as_ref().is_some_and(|d| d.serial == serial) {
                    self.status = if result.volume_support {
                        "数据分区支持压缩，仍需验证写入与还原兼容性"
                    } else {
                        "数据分区未启用 compression，系统权限无法单独开启透明压缩"
                    }
                    .into();
                    self.system_compression = Some(result);
                }
            }
            Outcome::ToolOperated(serial, label, result, report) => {
                if self.selected.as_ref().is_some_and(|d| d.serial == serial) {
                    let delta = result.available_after as i128 - result.available_before as i128;
                    let change = format!(
                        "{}{}",
                        if delta < 0 { "−" } else { "+" },
                        storage::format_bytes(delta.unsigned_abs() as u64)
                    );
                    let text = format!(
                        "{label}{} · 手机可用空间变化 {change} · {} → {}",
                        if result.completed {
                            "完成"
                        } else {
                            "未完全完成"
                        },
                        storage::format_bytes(result.available_before),
                        storage::format_bytes(result.available_after)
                    );
                    self.status = text.clone();
                    self.tools_last_result = Some(format!("{text}\n{}", result.result_path));
                    self.storage_report = report;
                    self.tools_pending = None;
                    self.tools_result_page = 0;
                }
            }
            Outcome::Transports(devices, services) => {
                let devices = stepx_companion::device::deduplicate_transports(devices);
                let mut failed_over_to: Option<String> = None;
                let mut disconnected = false;
                if let Some(selected) = self.selected.as_mut() {
                    let primary_online = devices
                        .iter()
                        .any(|t| t.id == selected.transport && t.state == "device");
                    if !primary_online {
                        let known_backups = selected.backup_transports();
                        let fallback = known_backups
                            .iter()
                            .find(|b| devices.iter().any(|t| &t.id == *b && t.state == "device"))
                            .cloned()
                            .or_else(|| {
                                devices
                                    .iter()
                                    .find(|t| {
                                        t.state == "device"
                                            && (stepx_companion::device::canonical_serial_from_transport(&t.id)
                                                .as_deref()
                                                == Some(selected.serial.as_str())
                                                || (!t.model.is_empty()
                                                    && t.model == selected.model))
                                    })
                                    .map(|t| t.id.clone())
                            });
                        if let Some(next_transport) = fallback {
                            let remaining_backups: Vec<String> = known_backups
                                .into_iter()
                                .filter(|b| b != &next_transport)
                                .collect();
                            selected.transport = next_transport.clone();
                            selected.set_backup_transports(remaining_backups);
                            failed_over_to = Some(next_transport);
                        } else {
                            disconnected = true;
                        }
                    } else if let Some(active) = devices
                        .iter()
                        .find(|t| t.id == selected.transport && t.state == "device")
                    {
                        let active_backups = active.backup_transports();
                        if !active_backups.is_empty() {
                            selected.set_backup_transports(active_backups);
                        }
                    }
                }
                if let Some(next_transport) = failed_over_to {
                    self.connection_watch.disconnected_at = None;
                    if let Some(srv) = &self.lan_cast.server {
                        srv.set_serial(next_transport.clone());
                    }
                    let msg = if is_wireless(&next_transport) {
                        format!(
                            "USB 已断开，已无感切换至无线通道 {next_transport} · 对话与任务不中断"
                        )
                    } else {
                        format!("已无感切换至 USB 通道 {next_transport} · 对话与任务不中断")
                    };
                    self.record_notice("多通道热备切换", msg.clone());
                    self.error = false;
                    if self.mirror.is_some() {
                        self.restart_active_mirror(msg);
                    } else {
                        self.status = msg;
                    }
                } else if disconnected {
                    self.connection_watch.detached_serial =
                        self.selected.as_ref().map(|d| d.serial.clone());
                    self.connection_watch.disconnected_at = Some(Instant::now());
                    self.selected = None;
                    self.amoo_gateway_online = false;
                    self.stop_mirror();
                    self.status = "连接暂时断开，正在自动重连；草稿与附件已保留".into();
                }
                self.devices = devices;
                self.mdns_services = services;
                if self.selected.is_none() {
                    self.auto_select();
                }
            }
            Outcome::Reattached(device) => {
                if self.connection_watch.detached_serial.as_deref() == Some(device.serial.as_str())
                {
                    self.connection_watch.detached_serial = None;
                    self.connection_watch.disconnected_at = None;
                    self.connection_watch.identity_at = None;
                    if let Some(srv) = &self.lan_cast.server {
                        srv.set_serial(device.transport.clone());
                    }
                    self.selected = Some(device);
                    self.connect_guide_step = None;
                    self.initializing = false;
                    self.error = false;
                    self.workspace.reconciled_sessions.clear();
                    self.amoo_poll_remaining = self.amoo_poll_remaining.max(25);
                    self.status = "已自动恢复连接 · 正在同步核对会话与任务".into();
                }
            }
            Outcome::Selected(device, summary, caps, real_amoo, sms, call_logs, contacts) => {
                self.allow_first_connect = false;
                let resumed = self.connection_watch.detached_serial.take().as_deref()
                    == Some(device.serial.as_str());
                self.connection_watch.disconnected_at = None;
                self.workspace.reconciled_sessions.clear();
                self.amoo_poll_remaining = self.amoo_poll_remaining.max(20);
                if !resumed {
                    self.clear_device_state();
                }
                self.connection_watch.identity_at = None;
                self.apply_capabilities(caps);
                self.apply_sms_data(sms);
                self.apply_phone_data(call_logs, contacts);
                if !self.demo_mode && self.selected_thread.is_none() {
                    if let Some(first) = self.sms_threads().first() {
                        let addr = first.address.clone();
                        self.fields[F_SMS_TO].set(&addr);
                        self.selected_thread = Some(addr);
                    }
                }
                if let Some(real) = real_amoo {
                    self.apply_real_amoo_data(real);
                }
                self.preferences
                    .remember(device.serial.clone(), device.model.clone());
                if let Err(e) = self.preferences.save() {
                    self.status = format!("已连接，但保存设备失败：{e}");
                    self.error = true;
                } else {
                    self.status = format!("已连接 {}", device.model);
                }
                self.initializing = false;
                if let Some(srv) = &self.lan_cast.server {
                    srv.set_serial(device.transport.clone());
                }
                self.selected = Some(device);
                self.summary = summary;
                // Collapse the newbie wizard once a device is connected; the user can
                // still open it manually via "连接引导" in the Connect page header.
                self.connect_guide_step = None;
                if self.page == Page::Overview {
                    self.with_device_bulk("正在读取应用数量与图标…", |adb, d| {
                        Ok(Outcome::Apps(apps::catalog(adb, &d)?))
                    });
                } else {
                    self.auto_load_for_page();
                }
            }
            Outcome::Text(text) => {
                self.content = text;
                self.output_page = self.job_page;
            }
            Outcome::Notice(text) => {
                if text != "就绪" && text != "已记住的设备不在线" {
                    self.record_notice("执行反馈", text.clone());
                }
                self.initializing = false;
                self.status = text;
            }
            Outcome::Summary(text, caps) => {
                self.summary = text;
                self.apply_capabilities(caps);
            }
            Outcome::Sms(messages, caps) => {
                self.apply_capabilities(caps);
                self.apply_sms_data(messages);
                if !self.demo_mode && self.selected_thread.is_none() {
                    if let Some(first) = self.sms_threads().first() {
                        let addr = first.address.clone();
                        self.fields[F_SMS_TO].set(&addr);
                        self.selected_thread = Some(addr);
                    }
                }
            }
            Outcome::SmsPrepared(plan) => {
                self.status = format!(
                    "已就绪：将通过 {} 发送至 {}，请点击「确认发送」",
                    plan.subscription_label, plan.number
                );
                let target = self
                    .selected
                    .as_ref()
                    .map(|d| format!("手机：{} · {}", d.model, d.serial))
                    .unwrap_or_default();
                self.review_action(
                    &Action::ConfirmDirectSms,
                    "确认发送这条短信？",
                    target,
                    vec![
                        format!("收件人：{} · 使用 {}", plan.number, plan.subscription_label),
                        plan.body.clone(),
                        format!(
                            "共 {} 段短信，运营商可能按套餐计费。确认后实际从手机发出。",
                            plan.parts
                        ),
                    ],
                    "确认发送",
                    true,
                    false,
                );
                self.pending_sms_plan = Some(plan);
            }
            Outcome::SmsSent(send_status, messages) => {
                self.pending_sms_plan = None;
                if send_status.state == "sent" {
                    self.fields[F_SMS_BODY].clear();
                }
                self.status = send_status.summary_label();
                self.error = send_status.state == "failed";
                self.last_sms_status = Some(send_status);
                let send_notice = self.status.clone();
                self.apply_sms_data(messages);
                self.status = if self.sms_error.is_some() {
                    format!("{send_notice}；短信列表读取失败，请刷新")
                } else {
                    send_notice
                };
                Self::scroll_to_end(&self.scroll_aux);
            }
            Outcome::SmsStatus(send_status) => {
                self.status = send_status.summary_label();
                self.error = send_status.state == "failed";
                self.last_sms_status = Some(send_status);
            }
            Outcome::AmooState(snapshot, msg, start_poll) => {
                let count = snapshot.turns.len();
                if !self.demo_mode && !snapshot.turns.is_empty() {
                    if let Some(sess) = self.amoo_sessions.get_mut(self.amoo_session_idx) {
                        let mut new_items = Vec::new();
                        for t in &snapshot.turns {
                            if t.is_user {
                                new_items.push(AmooChatItem::UserText {
                                    attachment: None,
                                    text: t.text.clone(),
                                });
                            } else {
                                let has_bolt = t.text.contains("背屏") || t.text.contains("点阵");
                                new_items.push(AmooChatItem::AssistantRich {
                                    thinking_time: "已完成".into(),
                                    thinking_steps: vec![
                                        format!("会话：{}", sess.title),
                                        "来源：手机 Amoo (com.stepos.ailauncher) 实时同步".into(),
                                    ],
                                    tools_summary: "手机 Amoo · 实机消息".into(),
                                    text: Some(t.text.clone()),
                                    has_bolt_artifact: has_bolt,
                                    has_invoice_artifact: false,
                                    has_safety_confirm: false,
                                    time_label: sess.status.clone(),
                                });
                            }
                        }
                        if !new_items.is_empty() {
                            sess.items = new_items;
                        }
                    }
                } else if let Some(last_ai) = snapshot
                    .turns
                    .iter()
                    .rev()
                    .find(|t| !t.is_user && !t.text.trim().is_empty())
                {
                    if let Some(sess) = self.amoo_sessions.get_mut(self.amoo_session_idx) {
                        if let Some(AmooChatItem::AssistantRich {
                            text,
                            tools_summary,
                            ..
                        }) = sess.items.last_mut()
                        {
                            if text.is_some() {
                                *text = Some(last_ai.text.clone());
                                *tools_summary = "手机端 Amoo 实时回复".into();
                            }
                        }
                    }
                }
                self.amoo_snapshot = Some(snapshot);
                if self.page == Page::Amoo {
                    self.reveal_latest_chat();
                }
                if start_poll {
                    self.fields[F_AMOO].clear();
                    self.amoo_poll_remaining = self.amoo_poll_remaining.max(4);
                    self.last_amoo_poll = Instant::now();
                }
                self.status = if msg.is_empty() {
                    format!("已同步 {count} 条消息")
                } else {
                    msg
                };
            }
            Outcome::AmooCreated(key) => {
                self.amoo_sessions.insert(
                    0,
                    AmooSessionItem {
                        session_key: key,
                        title: "新对话".into(),
                        agent: "Amoo 助手".into(),
                        status: "待提问".into(),
                        status_kind: 0,
                        meta: "后台已创建 · 尚无消息".into(),
                        items: Vec::new(),
                    },
                );
                self.amoo_session_idx = 0;
                self.amoo_poll_remaining = 0;
                self.status = "已在后台创建 Amoo 会话，可直接输入消息".into();
            }
            Outcome::WorkspaceDelivered(task, path, serial, result) => {
                let note = result
                    .clone()
                    .unwrap_or_else(|e| format!("发送未完成：{e}"));
                self.status = note.clone();
                self.error = result.is_err();
                if let Some(task) = self
                    .workspace
                    .journal
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == task)
                {
                    task.receipts
                        .push(stepx_companion::amoo_workspace::Receipt {
                            path,
                            note,
                            success: result.is_ok(),
                            device: serial,
                            account: None,
                            cloud_url: None,
                        });
                }
                self.workspace_persist();
            }
            Outcome::AmooSent(receipt) => {
                self.workspace.stopped_sessions.remove(&receipt.key);
                self.workspace_bind_session(&receipt.key);
                let key = receipt.key.clone();
                let text = receipt.text.clone();
                self.amoo_ui
                    .session_started
                    .insert(key.clone(), Instant::now());
                self.amoo_pending_reply.insert(key.clone(), receipt);
                if let Some(session) = self.amoo_sessions.iter_mut().find(|s| s.session_key == key)
                {
                    session.items.push(AmooChatItem::UserText {
                        attachment: None,
                        text,
                    });
                    session.status = "等待回复".into();
                    session.status_kind = 1;
                    session.meta = "后台已接收 · 正在等待回复".into();
                }
                self.amoo_poll_remaining = 100;
                self.last_amoo_poll = Instant::now();
                self.status = "Amoo 后台已接收消息，正在等待回复".into();
                self.reveal_latest_chat();
            }
            Outcome::AmooObservationUnavailable => {
                // Idle reads are not task failures. Retain the loaded history,
                // draft and attachments while marking the connection unconfirmed.
                self.amoo_gateway_online = false;
                self.amoo_poll_remaining = 0;
                self.status = "手机 Amoo 暂未连接，已保留上次内容；发送任务时会重新连接".into();
            }
            Outcome::AmooRealData(real, mut msg) => {
                // Measure completion before removing the matching pending run.
                self.record_amoo_usage(&real);
                self.reply_motion
                    .live
                    .extend(self.amoo_pending_reply.keys().cloned());
                let was_pending = self.amoo_pending_reply.len();
                let finished_error = self.amoo_pending_reply.values().find_map(|pending| {
                    let session = real.sessions.iter().find(|s| pending.finished(s))?;
                    session
                        .messages
                        .iter()
                        .rev()
                        .find(|m| m.run_id == pending.run_id && m.is_error)
                        .map(|m| {
                            m.error_message
                                .clone()
                                .unwrap_or_else(|| "手机返回了错误，请查看会话后继续".into())
                        })
                });
                if let Some(run) = self.workspace.running.clone().filter(|r| {
                    matches!(
                        r.executor,
                        stepx_companion::amoo_workspace::Executor::Phone
                            | stepx_companion::amoo_workspace::Executor::Image
                    )
                }) {
                    let key = self
                        .workspace
                        .journal
                        .tasks
                        .iter()
                        .find(|t| t.id == run.task)
                        .and_then(|t| t.session.as_ref());
                    let completed = key
                        .and_then(|key| self.amoo_pending_reply.get(key))
                        .and_then(|pending| {
                            real.sessions
                                .iter()
                                .find(|s| pending.finished(s))
                                .map(|s| (s, pending))
                        });
                    if let Some((session, pending)) = completed {
                        let failure = session
                            .messages
                            .iter()
                            .rev()
                            .find(|m| m.run_id == pending.run_id && m.is_error)
                            .map(|m| {
                                m.error_message
                                    .clone()
                                    .unwrap_or("手机返回错误，请查看回复".into())
                            });
                        let mut note = session
                            .messages
                            .iter()
                            .filter(|m| m.run_id == pending.run_id && m.role == "assistant")
                            .filter_map(|m| {
                                stepx_companion::work_protocol::display_text(&m.text, false)
                            })
                            .filter(|t| !t.trim().is_empty())
                            .collect::<Vec<_>>()
                            .join("\n\n");
                        if note.is_empty() {
                            if let Some(idx) = session
                                .messages
                                .iter()
                                .rposition(|m| m.role == "user" && m.run_id == pending.run_id)
                            {
                                note = session.messages[idx + 1..]
                                    .iter()
                                    .filter(|m| m.role == "assistant")
                                    .filter_map(|m| {
                                        stepx_companion::work_protocol::display_text(&m.text, false)
                                    })
                                    .filter(|t| !t.trim().is_empty())
                                    .collect::<Vec<_>>()
                                    .join("\n\n");
                            }
                        }
                        if let Some(error) = failure {
                            self.workspace_finish(
                                stepx_companion::amoo_workspace::Phase::Failed,
                                error,
                            );
                        } else if run.executor == stepx_companion::amoo_workspace::Executor::Image {
                            self.workspace_deliver_images(
                                session
                                    .messages
                                    .iter()
                                    .filter(|m| m.run_id == pending.run_id && m.role != "user")
                                    .cloned()
                                    .collect(),
                                note,
                            );
                        } else {
                            self.workspace_finish(
                                stepx_companion::amoo_workspace::Phase::Done,
                                note,
                            );
                        }
                    }
                }
                let reconciled = self.reconcile_workspace_with_real_data(&real);
                self.amoo_pending_reply
                    .retain(|_, pending| !real.sessions.iter().any(|s| pending.finished(s)));
                if !self.amoo_pending_reply.is_empty() && self.amoo_poll_remaining < 20 {
                    self.amoo_poll_remaining = 30;
                }
                if self.amoo_pending_reply.len() < was_pending {
                    if self.amoo_pending_reply.is_empty() {
                        self.amoo_poll_remaining = 0;
                    }
                    if let Some(error) = finished_error {
                        msg = error.clone();
                        self.work.error = Some(error);
                        self.error = true;
                    } else {
                        msg = "已收到 Amoo 本次请求的结果".into();
                    }
                } else if reconciled > 0 && msg.is_empty() {
                    msg = format!("已与手机自动同步并核对完成 {reconciled} 条待核对记录");
                } else if !self.amoo_pending_reply.is_empty() && self.amoo_poll_remaining == 0 {
                    msg = "Amoo 尚未返回完整结果，后台会继续同步；请勿重复发送".into();
                }
                let follow_tail = self.chat_near_end() && self.selected_text.is_none();
                let previous = self
                    .amoo_sessions
                    .get(self.amoo_session_idx)
                    .map(|s| (s.session_key.clone(), s.items.clone()));
                self.apply_real_amoo_data_recorded(real);
                let current = self
                    .amoo_sessions
                    .get(self.amoo_session_idx)
                    .map(|s| (s.session_key.as_str(), s.items.as_slice()));
                if matches!(self.page, Page::Amoo | Page::Work)
                    && previous
                        .as_ref()
                        .map(|(k, items)| (k.as_str(), items.as_slice()))
                        != current
                    && follow_tail
                {
                    self.reveal_latest_chat();
                }
                if !msg.is_empty() {
                    self.status = msg;
                }
            }
            Outcome::Files(path, files, notice) => {
                if path != self.directory {
                    self.scroll = Default::default();
                    self.fields[F_FILE_SEARCH].clear();
                    if self.focus == Some(F_FILE_SEARCH) {
                        self.focus = None;
                    }
                }
                self.directory = path;
                self.files = files;
                self.file_page = 0;
                self.selected_file = None;
                self.file_modal = None;
                self.trash_confirm = false;
                if let Some(msg) = notice {
                    self.record_notice("文件操作", msg.clone());
                    self.status = msg;
                }
            }
            Outcome::Apps(apps) => {
                self.apps_loaded = true;
                self.apps_error = None;
                self.status = format!("已读取 {} 个应用", apps.len());
                self.apps = apps;
                self.sync_app_store_with_installed();
            }
            Outcome::ApkInstalled(msg, apps) => {
                self.apps_loaded = true;
                self.apps_error = None;
                self.status = msg;
                self.apps = apps;
                self.sync_app_store_with_installed();
            }
            Outcome::Mirror(serial, transport, child) => {
                if let Some(selected) = self.selected.as_mut().filter(|d| d.serial == serial) {
                    if !transport.is_empty() && selected.transport != transport {
                        selected.transport = transport.clone();
                        if let Some(srv) = &self.lan_cast.server {
                            srv.set_serial(transport);
                        }
                    }
                    self.status = child.started_message().into();
                    self.mirror = Some(child);
                } else {
                    // Dropping the session closes a window started for a stale selection.
                    self.status = "设备已切换，已取消原设备的投屏".into();
                }
            }
            Outcome::Screenshot(directory, path) => {
                if self.amoo_ui.screen_mode || self.amoo_ui.gui_running || self.amoo_ui.gui_paused {
                    self.amoo_ui.gui_screenshot = Some(path.clone());
                }
                self.screenshot = Some((directory, path));
                self.status = "已截图".into();
            }
            Outcome::ScreenshotWithNotice(directory, path, notice) => {
                if self.amoo_ui.screen_mode || self.amoo_ui.gui_running || self.amoo_ui.gui_paused {
                    self.amoo_ui.gui_screenshot = Some(path.clone());
                }
                self.screenshot = Some((directory, path));
                self.status = notice;
            }
            Outcome::Trash(record) => {
                self.trashed = Some(record);
                self.status = "已移到回收站".into();
                self.browse(self.directory.clone());
            }
            Outcome::Restored => {
                self.trashed = None;
                self.status = "已恢复".into();
                self.browse(self.directory.clone());
            }
            Outcome::Qr(endpoint) => {
                if let Some(endpoint) = endpoint {
                    self.remember_pairing_host(&endpoint);
                    self.qr = None;
                    self.status = "配对成功，正在连接…".into();
                    self.refresh();
                } else {
                    self.refresh();
                    self.status = "等待扫码 · 正在检查设备通道…".into();
                }
            }
            Outcome::Paired(endpoint) => {
                self.remember_pairing_host(&endpoint);
                self.fields[F_PAIR_CODE].clear();
                self.status = format!("已连接 {endpoint}，正在验证…");
                self.refresh();
            }
            Outcome::BackScreenImportedImage(path) => {
                self.page = Page::BackScreen;
                self.back_screen.tab = 2;
                match self.back_screen.apply_image(path, "手机文件") {
                    Ok(()) => {
                        self.status = "正在转换手机图片，请预览后播放".into();
                    }
                    Err(error) => {
                        self.status = error.to_string();
                        self.error = true;
                    }
                }
            }
            Outcome::Studio(result) => self.apply_studio(result),
            Outcome::StudioLab(result) => self.apply_studio_lab(result),
            Outcome::PhoneData(logs, contacts) => {
                self.apply_phone_data(logs, contacts);
            }
            Outcome::PhoneCallPrepared(plan) => {
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|d| d.serial == plan.device_serial)
                    && phone::normalize_number(&self.fields[F_PHONE_DIAL].text)
                        .ok()
                        .as_deref()
                        == Some(plan.number.as_str())
                    && self
                        .selected_call_sim
                        .map_or(true, |id| id == plan.subscription)
                {
                    self.status = format!(
                        "请确认通过 {} 呼叫 {}",
                        plan.subscription_label, plan.number
                    );
                    let target = format!("手机：{}", plan.device_serial);
                    self.review_action(
                        &Action::PhoneConfirmCall,
                        "确认用手机拨打？",
                        target,
                        vec![
                            format!("号码：{}", plan.number),
                            format!("SIM：{}", plan.subscription_label),
                            "确认后手机将发起实际通话，可能产生运营商费用。".into(),
                        ],
                        "确认拨打",
                        true,
                        false,
                    );
                    self.pending_call = Some(plan);
                }
            }
            Outcome::DialSubmitted(number, notice) => {
                self.active_call_number = Some(number);
                self.status = notice;
            }
            Outcome::CallHangup(notice) => {
                self.active_call_number = None;
                self.status = notice;
            }
            Outcome::GoogleSuite(suite, companion, msg) => {
                self.google_suite = suite;
                self.companion_status = companion;
                self.status = msg;
            }
            Outcome::NeoStatus(neo) => {
                self.status = neo.message.clone();
                self.neo_status = neo;
            }
            Outcome::CompanionStatus(companion, msg) => {
                self.companion_status = companion;
                if self.page == Page::Connect
                    || (self.page == Page::Tools && self.tool_page == Some(ToolPage::Companion))
                {
                    let short = msg
                        .split(['；', '。'])
                        .next()
                        .unwrap_or(&msg)
                        .trim()
                        .to_string();
                    self.status = if short.is_empty() { msg } else { short };
                }
            }
            Outcome::InputMethod(report, message) => {
                self.fcitx5_active = report.active;
                self.input_method = report;
                self.status = message;
            }
            Outcome::SystemServices(services, msg) => {
                self.fcitx5_active = services
                    .iter()
                    .find(|s| s.package == "org.fcitx.fcitx5.android")
                    .map(|s| s.status_note.contains("当前默认"))
                    .unwrap_or(self.fcitx5_active);
                self.system_services = services;
                self.status = msg;
            }
            Outcome::DeepAmooGranted(msg) => {
                self.deep_amoo_granted = true;
                self.amoo_bg_keepalive = true;
                self.back_screen.stepos_suite.gui_accessibility_enabled = true;
                self.back_screen.stepos_suite.notification_listener_enabled = true;
                self.back_screen.stepos_suite.mcp_permission_granted = true;
                self.back_screen.stepos_suite.screen_perception_ready = true;
                self.back_screen.stepos_suite.last_action_note = msg.clone();
                self.tools_feedback = Some((self.tool_page, msg.clone(), false));
                self.status = msg;
            }
            Outcome::DeepBackScreenGranted(msg) => {
                self.deep_backscreen_granted = true;
                self.status = msg;
            }
            Outcome::PhoneGallery(items, msg) => {
                self.back_screen.gallery_items = items;
                self.sync_gallery_image_previews();
                self.back_screen.gallery_loading = false;
                self.back_screen.gallery_checked = true;
                if self.back_screen.gallery_selected >= self.back_screen.gallery_items.len() {
                    self.back_screen.gallery_selected = 0;
                }
                self.error = false;
                self.status = msg;
            }
            Outcome::StepOsCalendarUpdated(events, msg) => {
                self.fields[F_PHONE_SCHEDULE_TITLE].clear();
                self.back_screen.stepos_suite.checked = true;
                self.back_screen.stepos_suite.calendar_events = events;
                self.back_screen.stepos_suite.last_action_note = msg.clone();
                self.error = false;
                self.status = msg;
            }
            Outcome::StepOsAlarmsUpdated(alarms, msg) => {
                self.fields[F_PHONE_SCHEDULE_TITLE].clear();
                self.back_screen.stepos_suite.checked = true;
                self.back_screen.stepos_suite.alarms = alarms;
                self.back_screen.stepos_suite.last_action_note = msg.clone();
                self.error = false;
                self.status = msg;
            }
            Outcome::StepOsSuite(report) => {
                self.back_screen.stepos_suite_loading = false;
                let phone_clip = report.phone_clipboard.trim().to_string();
                if !phone_clip.is_empty() {
                    self.back_screen.record_clipboard_history(&phone_clip);
                }
                let note = report.last_action_note.clone();
                self.back_screen.stepos_suite = report;
                if !self.busy {
                    self.status = note.clone();
                }
                self.tools_feedback = Some((self.tool_page, note, false));
            }
        }
    }

    pub(super) fn apply_capabilities(&mut self, caps: Option<Capabilities>) {
        let selected = caps.as_ref().and_then(|c| {
            self.selected_sim
                .filter(|id| c.subscriptions.iter().any(|s| s.id == *id))
                .or_else(|| {
                    c.subscriptions
                        .iter()
                        .find(|s| s.id == c.default_subscription)
                        .map(|s| s.id)
                })
                .or_else(|| (c.subscriptions.len() == 1).then(|| c.subscriptions[0].id))
        });
        if self.selected_sim != selected {
            self.pending_sms_plan = None;
        }
        self.selected_sim = selected;
        self.capabilities = caps;
    }

    pub(super) fn apply_sms_data(&mut self, result: ReadResult<Message>) {
        self.sms_loaded = result.is_ok();
        match result {
            Ok(messages) => {
                self.status = format!("已从手机读取 {} 条短信", messages.len());
                self.sms = messages;
                self.sms_error = None;
                // Never leave a deleted thread selected or a previous recipient in the composer.
                if self.selected_thread.as_deref().is_some_and(|addr| {
                    addr != "__new__"
                        && !self.sms.iter().any(|m| {
                            stepx_companion::sms::normalize_address(&m.address)
                                == stepx_companion::sms::normalize_address(addr)
                        })
                }) {
                    self.selected_thread = None;
                    self.fields[F_SMS_TO].clear();
                    self.pending_sms_plan = None;
                }
            }
            Err(error) => {
                self.sms.clear();
                self.status = format!("短信读取失败：{error}");
                self.sms_error = Some(error);
                self.error = true;
            }
        }
    }

    pub(super) fn apply_phone_data(
        &mut self,
        logs: ReadResult<CallRecord>,
        contacts: ReadResult<ContactEntry>,
    ) {
        self.phone_loaded = true;
        match logs {
            Ok(logs) => {
                self.call_logs = logs;
                self.call_logs_error = None;
            }
            Err(error) => {
                self.call_logs.clear();
                self.call_logs_error = Some(error);
            }
        }
        match contacts {
            Ok(contacts) => {
                self.contacts = contacts;
                self.contacts_error = None;
            }
            Err(error) => {
                self.contacts.clear();
                self.contacts_error = Some(error);
            }
        }
        self.error = self.call_logs_error.is_some() || self.contacts_error.is_some();
        self.status = if self.error {
            "部分手机数据读取失败，详情见页面".into()
        } else {
            format!(
                "已同步 {} 条通话记录与 {} 位联系人",
                self.call_logs.len(),
                self.contacts.len()
            )
        };
        self.selected_contact = 0;
    }

    /// Reconnect the remembered phone, or auto-select the only StepX phone on first launch.
    pub(super) fn auto_select(&mut self) {
        let candidates: Vec<_> = self
            .devices
            .iter()
            .filter(|d| d.state == "device" && is_stepx_model(&d.model))
            .map(|d| d.id.clone())
            .collect();
        if let Some(preferred) = self.preferences.selected_serial.clone() {
            let resuming = self.connection_watch.detached_serial.clone();
            if !candidates.is_empty() {
                let allow_fallback = self.allow_first_connect && candidates.len() == 1;
                self.submit("正在连接 StepX 设备…", move |adb| {
                    for id in &candidates {
                        if let Ok(device) = Device::inspect(adb, id) {
                            if device.serial == preferred || allow_fallback {
                                if resuming.as_deref() == Some(device.serial.as_str()) {
                                    return Ok(Outcome::Reattached(device));
                                }
                                let (summary_res, caps, sms, call_logs, contacts) =
                                    std::thread::scope(|s| {
                                        let h_summary = s.spawn(|| device.summary(adb));
                                        let h_caps =
                                            s.spawn(|| bridge::capabilities(adb, &device).ok());
                                        let h_sms = s.spawn(|| read_result(device.messages(adb)));
                                        let h_calls =
                                            s.spawn(|| read_result(device.call_logs(adb)));
                                        let h_contacts =
                                            s.spawn(|| read_result(device.contacts(adb)));
                                        (
                                            h_summary.join().unwrap(),
                                            h_caps.join().unwrap(),
                                            h_sms.join().unwrap(),
                                            h_calls.join().unwrap(),
                                            h_contacts.join().unwrap(),
                                        )
                                    });
                                let summary = summary_res?;
                                let real_amoo = None; // Load conversations lazily on the Amoo page.
                                return Ok(Outcome::Selected(
                                    device, summary, caps, real_amoo, sms, call_logs, contacts,
                                ));
                            }
                        }
                    }
                    Ok(Outcome::Notice("已记住的设备不在线".into()))
                });
            } else {
                self.initializing = false;
                self.status = "已记住的设备不在线".into();
            }
        } else if candidates.len() == 1 && self.allow_first_connect {
            let id = candidates[0].clone();
            self.select(id);
        } else if self.devices.is_empty() {
            self.initializing = false;
            self.status = "未连接设备".into();
        } else {
            self.initializing = false;
            self.status = format!("发现 {} 台设备", self.devices.len());
        }
    }

    pub(super) fn select(&mut self, id: String) {
        self.submit("正在验证设备…", move |adb| {
            let device = Device::inspect(adb, &id)?;
            let (summary_res, caps, sms, call_logs, contacts) = std::thread::scope(|s| {
                let h_summary = s.spawn(|| device.summary(adb));
                let h_caps = s.spawn(|| bridge::capabilities(adb, &device).ok());
                let h_sms = s.spawn(|| read_result(device.messages(adb)));
                let h_calls = s.spawn(|| read_result(device.call_logs(adb)));
                let h_contacts = s.spawn(|| read_result(device.contacts(adb)));
                (
                    h_summary.join().unwrap(),
                    h_caps.join().unwrap(),
                    h_sms.join().unwrap(),
                    h_calls.join().unwrap(),
                    h_contacts.join().unwrap(),
                )
            });
            let summary = summary_res?;
            let real_amoo = None; // Load conversations lazily on the Amoo page.
            Ok(Outcome::Selected(
                device, summary, caps, real_amoo, sms, call_logs, contacts,
            ))
        });
    }

    pub(super) fn apply_real_amoo_data(&mut self, real: RealAmooData) {
        self.record_amoo_usage(&real);
        self.apply_real_amoo_data_recorded(real);
    }

    pub(super) fn apply_real_amoo_data_recorded(&mut self, real: RealAmooData) {
        self.record_work_attachment_identities(&real);
        for session in &real.sessions {
            if !session.model.is_empty() {
                self.work
                    .session_models
                    .insert(session.key.clone(), session.model.clone());
            }
        }
        self.amoo_gateway_version = real.gateway_version;
        self.amoo_gateway_online = real.gateway_online;
        self.amoo_cron_count = real.cron_jobs.len();
        self.amoo_cron_jobs = real.cron_jobs.clone();
        if real.memory_bm25_entries > 0 {
            self.amoo_bm25_entries = real.memory_bm25_entries;
        } else {
            self.amoo_bm25_entries = self.amoo_bm25_entries.max(self.amoo_memories.len());
        }
        if real.total_skills_count > 0 {
            self.amoo_total_skills = real.total_skills_count;
        } else {
            self.amoo_total_skills = self.amoo_total_skills.max(self.amoo_skills.len());
        }
        if real.memory_initialized {
            self.amoo_memory_enabled = true;
        }

        let current_key = self
            .workspace_task()
            .and_then(|t| t.session.clone())
            .or_else(|| {
                self.amoo_sessions
                    .get(self.amoo_session_idx)
                    .map(|s| s.session_key.clone())
            });

        let current_key = if self.amoo_session_idx == usize::MAX {
            current_key
        } else {
            current_key.or(real.selected_session)
        };
        let mut new_sessions = Vec::new();
        for s in real.sessions {
            if self.workspace.deleted_sessions.contains(&s.key) {
                continue;
            }
            let mut items = if s.history_loaded || !s.messages.is_empty() {
                Vec::new()
            } else {
                self.amoo_sessions
                    .iter()
                    .find(|old| old.session_key == s.key)
                    .map(|old| old.items.clone())
                    .unwrap_or_default()
            };
            items.extend(amoo_display::chat_items(&s.key, s.messages, s.status_kind));
            let pending = self.amoo_pending_reply.contains_key(&s.key);
            let stopped = !pending && self.workspace.stopped_sessions.contains(&s.key);
            new_sessions.push(AmooSessionItem {
                session_key: s.key,
                title: s.title,
                agent: s.agent,
                status: if pending {
                    "等待完整结果".into()
                } else if stopped {
                    "已停止".into()
                } else {
                    s.status
                },
                status_kind: if pending {
                    1
                } else if stopped {
                    0
                } else {
                    s.status_kind
                },
                meta: s.meta,
                items,
            });
        }

        for old in &self.amoo_sessions {
            if self.workspace.deleted_sessions.contains(&old.session_key) {
                continue;
            }
            if (old.status == "待提问" || self.amoo_pending_reply.contains_key(&old.session_key))
                && !new_sessions
                    .iter()
                    .any(|s| s.session_key == old.session_key)
            {
                let mut cloned = old.clone();
                if self
                    .workspace
                    .stopped_sessions
                    .contains(&cloned.session_key)
                    && !self.amoo_pending_reply.contains_key(&cloned.session_key)
                {
                    cloned.status_kind = 0;
                    cloned.status = "已停止".into();
                }
                new_sessions.push(cloned);
            }
        }
        if !self.settings.value.reduced_motion && !self.settings.hidden {
            for next in &new_sessions {
                if current_key.as_deref() != Some(&next.session_key)
                    || !(self.amoo_pending_reply.contains_key(&next.session_key)
                        || self.reply_motion.live.contains(&next.session_key))
                {
                    continue;
                }
                let Some(old) = self
                    .amoo_sessions
                    .iter()
                    .find(|s| s.session_key == next.session_key)
                else {
                    continue;
                };
                for (i, item) in next.items.iter().enumerate() {
                    if let AmooChatItem::AssistantRich {
                        text: Some(text), ..
                    } = item
                    {
                        let previous = match old.items.get(i) {
                            Some(AmooChatItem::AssistantRich { text: Some(t), .. }) => t.as_str(),
                            _ => "",
                        };
                        let before = stepx_companion::work_protocol::display_text(previous, false)
                            .unwrap_or_default();
                        let after = stepx_companion::work_protocol::display_text(text, false)
                            .unwrap_or_default();
                        self.reply_motion
                            .track(next.session_key.clone(), i, &before, &after);
                    }
                }
            }
        }
        self.reply_motion.live.clear();
        self.amoo_sessions = new_sessions;
        // Passive updates preserve the user's session-list scroll position.
        if let Some(key) = current_key {
            if let Some(pos) = self.amoo_sessions.iter().position(|s| s.session_key == key) {
                self.amoo_session_idx = pos;
            } else if self.amoo_session_idx != usize::MAX {
                self.amoo_session_idx = 0;
            }
        } else if self.amoo_session_idx != usize::MAX {
            self.amoo_session_idx = 0;
        }

        if !real.skills.is_empty() {
            self.amoo_skills = real
                .skills
                .into_iter()
                .map(|sk| AmooSkillEntry {
                    skill_key: sk.skill_key,
                    name: sk.display_name,
                    enabled: sk.enabled,
                    prompt: sk.prompt,
                })
                .collect();
        }

        let mut user_mems: Vec<AmooMemoryEntry> = self
            .amoo_memories
            .iter()
            .filter(|m| m.sub.starts_with("桌面") || m.sub.starts_with("对话"))
            .cloned()
            .collect();
        for (title, sub) in real.memory_entries {
            user_mems.push(AmooMemoryEntry { title, sub });
        }
        self.amoo_memories = user_mems;
        self.load_work_previews();
    }

    pub(super) fn remember_pairing_host(&mut self, endpoint: &str) {
        if let Ok(address) = pairing::validate_endpoint(endpoint) {
            if !self.pairing_hosts.contains(&address.ip()) {
                self.pairing_hosts.push(address.ip());
            }
        }
    }
}
