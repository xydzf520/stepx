//! Background job outcomes and state application for `Companion`.

use super::*;

pub(super) type ReadResult<T> = std::result::Result<Vec<T>, String>;

pub(super) fn read_result<T>(result: Result<Vec<T>>) -> ReadResult<T> {
    result.map_err(|error| format!("{error:#}"))
}

pub(super) enum Outcome {
    AutoSyncMessages {
        device_serial: String,
        request_id: u64,
        result: std::result::Result<Box<Outcome>, String>,
    },
    BulkFailed(Page, Option<ToolPage>, String),
    JobFailed(Page, Option<ToolPage>, anyhow::Error),
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
    AmooSkillRequested(amoo::PendingReply),
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
    HomeRefreshed(
        String,
        Option<Capabilities>,
        ReadResult<Message>,
        ReadResult<CallRecord>,
        ReadResult<ContactEntry>,
    ),
    Files(String, Vec<Entry>, Option<String>),
    FilePreviewLoaded {
        path: String,
        text: Option<String>,
        image_path: Option<PathBuf>,
    },
    Apps(Vec<AppInfo>),
    AppInspected(
        String,
        stepx_companion::apps::AppRuntimeDetails,
        Option<String>,
    ),
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
    DoctorHarnessReport(stepx_companion::doctor_harness::DoctorReport),
}

impl Companion {
    pub(super) fn apply(&mut self, outcome: Outcome) {
        let sync_results = self.auto_sync_outcome_results(&outcome);
        match outcome {
            Outcome::AutoSyncMessages {
                device_serial,
                request_id,
                result,
            } => {
                self.apply_message_sync_result(&device_serial, request_id, result);
                return;
            }
            Outcome::JobFailed(page, tool, error) => {
                let context = error
                    .downcast_ref::<interaction::OperationFailure>()
                    .cloned();
                self.apply(Outcome::BulkFailed(page, tool, format!("{error:#}")));
                self.attach_operation_failure(context);
                return;
            }
            Outcome::BulkFailed(page, _tool, err) => {
                self.record_notice("操作未完成", err.clone());
                if matches!(page, Page::Apps | Page::AppStore) {
                    self.apps_loaded = true;
                    self.apps_error = Some(err.clone());
                    self.selected_app_loading = false;
                }
                if page == Page::Tools {
                    self.back_screen.stepos_suite_loading = false;
                }
                self.doctor_running = false;
                self.error = true;
                self.status = self.connection_failure_status(&err);
            }
            Outcome::FilePreviewLoaded {
                path,
                text,
                image_path,
            } => {
                self.selected_file_preview_path = Some(path);
                self.selected_file_preview_text = text;
                self.selected_file_preview_image = image_path;
            }
            Outcome::AppInspected(pkg, details, notice) => {
                self.selected_app_loading = false;
                if self.selected_app.as_deref() == Some(pkg.as_str()) {
                    self.selected_app_details = Some(details);
                }
                if let Some(msg) = notice {
                    self.status = msg;
                }
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
                let mut transient_miss = false;
                if let Some(selected) = self.selected.as_mut() {
                    let primary_online = devices.iter().any(|t| {
                        t.state == "device"
                            && (t.id == selected.transport
                                || t.backup_transports()
                                    .iter()
                                    .any(|b| b == &selected.transport))
                    });
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
                                                == Some(selected.serial.as_str()))
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
                            stepx_companion::device::promote_active_transport(
                                &selected.serial,
                                &next_transport,
                            );
                            failed_over_to = Some(next_transport);
                        } else {
                            self.connection_watch.misses =
                                self.connection_watch.misses.saturating_add(1);
                            if self.connection_watch.misses >= 2 {
                                disconnected = true;
                            } else {
                                transient_miss = true;
                            }
                        }
                    } else {
                        self.connection_watch.misses = 0;
                        self.connection_watch.disconnected_at = None;
                        if let Some(active) = devices.iter().find(|t| {
                            t.state == "device"
                                && (t.id == selected.transport
                                    || t.backup_transports()
                                        .iter()
                                        .any(|b| b == &selected.transport))
                        }) {
                            let mut active_backups: Vec<String> =
                                std::iter::once(active.id.clone())
                                    .chain(active.backup_transports())
                                    .filter(|b| b != &selected.transport)
                                    .collect();
                            active_backups.dedup();
                            if !active_backups.is_empty() {
                                selected.set_backup_transports(active_backups);
                            }
                        }
                    }
                }
                if transient_miss {
                    self.begin_connection_watch();
                }
                if let Some(next_transport) = failed_over_to {
                    self.connection_watch.misses = 0;
                    self.connection_watch.disconnected_at = None;
                    if let Some(srv) = &self.lan_cast.server {
                        srv.set_serial(next_transport.clone());
                    }
                    let msg = if is_wireless(&next_transport) {
                        format!("已切换至可用无线通道 {next_transport}，正在继续同步")
                    } else {
                        format!("已切换至 USB 通道 {next_transport}，正在继续同步")
                    };
                    self.record_notice("多通道热备切换", msg.clone());
                    self.error = false;
                    if self.mirror.is_some() {
                        self.restart_active_mirror(msg);
                    } else if self
                        .mirror_interrupted_at
                        .is_some_and(|t| t.elapsed() < Duration::from_secs(10))
                    {
                        self.mirror_interrupted_at = None;
                        self.start_mirror();
                        self.status = format!("{msg} · 正在无感恢复无线投屏");
                    } else {
                        self.status = msg;
                    }
                } else if disconnected {
                    self.connection_watch.detached_serial =
                        self.selected.as_ref().map(|d| d.serial.clone());
                    self.connection_watch.disconnected_at = Some(Instant::now());
                    self.selected = None;
                    self.amoo_gateway_online = false;
                    let had_mirror = self.mirror.is_some();
                    self.stop_mirror();
                    if had_mirror {
                        self.mirror_interrupted_at = Some(Instant::now());
                    }
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
                    self.connection_watch.misses = 0;
                    self.connection_watch.detached_serial = None;
                    self.connection_watch.disconnected_at = None;
                    self.connection_watch.identity_at = None;
                    stepx_companion::device::promote_active_transport(
                        &device.serial,
                        &device.transport,
                    );
                    if let Some(srv) = &self.lan_cast.server {
                        srv.set_serial(device.transport.clone());
                    }
                    self.last_connected_serial = Some(device.serial.clone());
                    self.selected = Some(device);
                    self.connect_guide_step = None;
                    self.initializing = false;
                    self.error = false;
                    self.workspace.reconciled_sessions.clear();
                    self.amoo_poll_remaining = self.amoo_poll_remaining.max(25);
                    self.auto_load_for_page();
                    if self
                        .mirror_interrupted_at
                        .is_some_and(|t| t.elapsed() < Duration::from_secs(10))
                    {
                        self.mirror_interrupted_at = None;
                        self.start_mirror();
                    }
                    self.status = "已自动恢复连接 · 正在同步核对会话与任务".into();
                }
            }
            Outcome::Selected(device, summary, caps, real_amoo, sms, call_logs, contacts) => {
                if self.connection_watch.user_paused {
                    return;
                }
                self.allow_first_connect = false;
                let same_selected = self
                    .selected
                    .as_ref()
                    .is_some_and(|d| d.serial == device.serial);
                let resumed = self.connection_watch.detached_serial.take().as_deref()
                    == Some(device.serial.as_str())
                    || same_selected;
                self.last_connected_serial = Some(device.serial.clone());
                self.connection_watch.misses = 0;
                self.connection_watch.disconnected_at = None;
                self.workspace.reconciled_sessions.clear();
                self.amoo_poll_remaining = self.amoo_poll_remaining.max(20);
                if !resumed {
                    self.clear_device_state();
                }
                self.connection_watch.identity_at = None;
                stepx_companion::device::promote_active_transport(
                    &device.serial,
                    &device.transport,
                );
                self.apply_capabilities(caps);
                self.apply_sms_data(sms);
                self.apply_phone_data(call_logs, contacts);
                if !self.demo_mode
                    && self.selected_thread.is_none()
                    && self.push.selected_im_thread.is_none()
                    && self.push.snapshot.threads.is_empty()
                {
                    if let Some(first) = self.sms_threads().first() {
                        let addr = first.address.clone();
                        self.fields[F_SMS_TO].set(&addr);
                        self.selected_thread = Some(addr);
                    }
                }
                self.sync_live_express_parcels();
                self.sync_notification_center();
                if let Some(real) = real_amoo {
                    self.apply_real_amoo_data(real);
                }
                self.preferences.adb_guide_seen = true;
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
                if !summary.is_empty() || self.summary.is_empty() {
                    self.summary = summary;
                }
                // Collapse the newbie wizard once a device is connected; the user can
                // still open it manually via "连接引导" in the Connect page header.
                self.connect_guide_step = None;
                self.adb_guide_step = None;
                if self.page == Page::Overview {
                    self.with_device_bulk("正在读取应用数量与图标…", |adb, d| {
                        Ok(Outcome::Apps(apps::catalog(adb, &d)?))
                    });
                    self.auto_load_for_page();
                } else if self.page == Page::Home && self.auto_sync.enabled {
                    if !self.amoo_ui.usage_attempted {
                        self.start_account_usage_sync();
                    }
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
            Outcome::HomeRefreshed(summary, caps, sms, call_logs, contacts) => {
                if !summary.is_empty() {
                    self.summary = summary;
                }
                self.apply_capabilities(caps);
                self.apply_sms_data(sms);
                self.apply_phone_data(call_logs, contacts);
                if !self.demo_mode
                    && self.selected_thread.is_none()
                    && self.push.selected_im_thread.is_none()
                    && self.push.snapshot.threads.is_empty()
                {
                    if let Some(first) = self.sms_threads().first() {
                        let addr = first.address.clone();
                        self.fields[F_SMS_TO].set(&addr);
                        self.selected_thread = Some(addr);
                    }
                }
                self.sync_live_express_parcels();
                self.sync_notification_center();
            }
            Outcome::Sms(messages, caps) => {
                self.apply_capabilities(caps);
                self.apply_sms_data(messages);
                if !self.demo_mode
                    && self.selected_thread.is_none()
                    && self.push.selected_im_thread.is_none()
                    && self.push.snapshot.threads.is_empty()
                {
                    if let Some(first) = self.sms_threads().first() {
                        let addr = first.address.clone();
                        self.fields[F_SMS_TO].set(&addr);
                        self.selected_thread = Some(addr);
                    }
                }
                self.sync_live_express_parcels();
                self.trigger_auto_context_refresh();
                self.sync_notification_center();
            }
            Outcome::SmsPrepared(plan) => {
                self.status = format!(
                    "已就绪：将通过 {} 发送至 {}，请点击「确认发送」",
                    plan.subscription_label, plan.number
                );
                self.pending_sms_plan = Some(plan);
                self.focus = Some(F_SMS_BODY);
            }
            Outcome::SmsSent(send_status, messages) => {
                self.pending_sms_plan = None;
                if send_status.state == "sent" {
                    self.fields[F_SMS_BODY].clear();
                } else if send_status.state == "failed" {
                    if let Some(failed_msg) = self.local_sent_sms.first().cloned() {
                        self.local_sent_sms.remove(0);
                        self.sms.retain(|m| {
                            !(m.outgoing
                                && m.date_ms == failed_msg.date_ms
                                && m.body == failed_msg.body)
                        });
                        if self.fields[F_SMS_BODY].text.trim().is_empty() {
                            self.fields[F_SMS_BODY].set(&failed_msg.body);
                        }
                    }
                }
                self.status = send_status.summary_label();
                self.error = send_status.state == "failed";
                self.last_sms_status = Some(send_status);
                let send_notice = self.status.clone();
                let sms_refresh_failed = messages.is_err();
                self.apply_sms_data(messages);
                self.status = if sms_refresh_failed {
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
            Outcome::AmooSkillRequested(receipt) => {
                // Preserve the user's current conversation, attachments and draft.
                if let Err(error) = self.workspace_switch_new() {
                    self.status =
                        format!("请求已发给 Amoo，但切换会话失败：{error}；请在会话记录查看");
                    self.error = true;
                    return;
                }
                self.apply(Outcome::AmooCreated(receipt.key.clone()));
                if let Some(session) = self.amoo_sessions.first_mut() {
                    session.title = "连接伴侣后台操作".into();
                }
                self.apply(Outcome::AmooSent(receipt));
                self.page = Page::Amoo;
                self.status="安装请求已发出，请查看 Amoo 的读回结果。成功后新建对话即可使用；如手机要求授权，请本人确认。".into();
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
                self.mark_auto_sync_domain_synced(AutoSyncDomain::AmooAndTasks);
                if self.page == Page::Amoo {
                    self.status = "手机 Amoo 暂未连接，已保留上次内容；发送任务时会重新连接".into();
                }
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
                self.record_notice("应用安装", msg.clone());
                self.status = msg;
                self.apps = apps;
                self.sync_app_store_with_installed();
                self.sync_notification_center();
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
                self.record_notice("手机截图", format!("已获取手机截图 {}", path.display()));
                self.screenshot = Some((directory, path));
                self.status = "已截图".into();
                self.sync_notification_center();
            }
            Outcome::ScreenshotWithNotice(directory, path, notice) => {
                if self.amoo_ui.screen_mode || self.amoo_ui.gui_running || self.amoo_ui.gui_paused {
                    self.amoo_ui.gui_screenshot = Some(path.clone());
                }
                self.record_notice("手机截图", notice.clone());
                self.screenshot = Some((directory, path));
                self.status = notice;
                self.sync_notification_center();
            }
            Outcome::Trash(record) => {
                self.record_notice("文件管理", format!("已移到回收站：{}", record.original));
                self.trashed = Some(record);
                self.status = "已移到回收站".into();
                self.browse(self.directory.clone());
                self.sync_notification_center();
            }
            Outcome::Restored => {
                self.record_notice("文件管理", "已从回收站恢复文件".to_string());
                self.trashed = None;
                self.status = "已恢复".into();
                self.browse(self.directory.clone());
                self.sync_notification_center();
            }
            Outcome::Qr(endpoint) => {
                if self.connection_watch.user_paused {
                    return;
                }
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
                if self.connection_watch.user_paused {
                    return;
                }
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
                // A successful grant script is not a fresh observation of four
                // independent capabilities. The following inspection supplies
                // each permission's actual/unknown state.
                self.back_screen.stepos_suite.capability_evidence = Default::default();
                self.back_screen.stepos_suite.checked = false;
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
                if self.back_screen.stepos_suite_synced_at.is_none() {
                    self.back_screen.stepos_suite_synced_at = Some(Instant::now());
                }
                self.back_screen.stepos_suite.calendar_events = events;
                self.back_screen.stepos_suite.last_action_note = msg.clone();
                self.error = false;
                self.status = msg;
            }
            Outcome::StepOsAlarmsUpdated(alarms, msg) => {
                self.fields[F_PHONE_SCHEDULE_TITLE].clear();
                self.back_screen.stepos_suite.checked = true;
                if self.back_screen.stepos_suite_synced_at.is_none() {
                    self.back_screen.stepos_suite_synced_at = Some(Instant::now());
                }
                self.back_screen.stepos_suite.alarms = alarms;
                self.back_screen.stepos_suite.last_action_note = msg.clone();
                self.error = false;
                self.status = msg;
            }
            Outcome::StepOsSuite(report) => {
                self.back_screen.stepos_suite_loading = false;
                if self.reject_incomplete_stepos_snapshot(&report) {
                    self.apply_auto_sync_results(sync_results);
                    self.reconcile_auto_sync_domains_from_state();
                    return;
                }
                self.back_screen.stepos_suite_synced_at = Some(Instant::now());
                let sync_error = report
                    .read_error
                    .as_deref()
                    .map(str::trim)
                    .filter(|error| !error.is_empty())
                    .map(str::to_string);
                self.back_screen.stepos_suite_error = sync_error.clone();
                let phone_clip = report.phone_clipboard.trim().to_string();
                if !phone_clip.is_empty() {
                    self.back_screen.record_clipboard_history(&phone_clip);
                }
                let sig_text = report.backscreen_signature.trim().to_string();
                if !sig_text.is_empty() {
                    self.back_screen.frame =
                        stepx_companion::backscreen::synthesize_chinese_signature_frame(
                            &sig_text,
                            self.back_screen.config.invert,
                        );
                }
                let note = report.last_action_note.clone();
                self.back_screen.stepos_suite = report;
                self.sync_live_express_parcels();
                if !self.busy {
                    self.status = note.clone();
                    self.error = sync_error.is_some();
                }
                if sync_error.is_some() && self.tool_page.is_none() {
                    self.tools_feedback = Some((None, note, true));
                }
            }
            Outcome::DoctorHarnessReport(report) => {
                self.doctor_running = false;
                if let Some(ref gs) = report.google_status {
                    self.google_suite = gs.clone();
                }
                if let Some(ref cs) = report.companion_status {
                    self.companion_status = cs.clone();
                }
                self.error = false;
                self.status = format!("{}：{}", report.model_label, report.headline);
                if self.page == Page::Amoo {
                    if let Some(task) = self.workspace_task_mut() {
                        let matches_task = task.title.contains(&report.query)
                            || report.query.contains(&task.title)
                            || task.turns.iter().any(|t| {
                                t.user
                                    && (t.text.contains(&report.query)
                                        || stepx_companion::amoo_route::detect_doctor_or_google_guide_query(&t.text).is_some())
                            })
                            || stepx_companion::amoo_route::detect_doctor_or_google_guide_query(&task.title).is_some();
                        if matches_task && !report.ai_guidance_markdown.trim().is_empty() {
                            if let Some(last_turn) = task.turns.last_mut() {
                                if last_turn.user {
                                    task.turns.push(stepx_companion::amoo_workspace::Turn {
                                        at_ms: Some(
                                            std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .unwrap_or_default()
                                                .as_millis()
                                                as i64,
                                        ),
                                        user: false,
                                        text: report.ai_guidance_markdown.clone(),
                                    });
                                } else {
                                    last_turn.text = report.ai_guidance_markdown.clone();
                                }
                            } else {
                                task.turns.push(stepx_companion::amoo_workspace::Turn {
                                    at_ms: Some(
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_millis()
                                            as i64,
                                    ),
                                    user: false,
                                    text: report.ai_guidance_markdown.clone(),
                                });
                            }
                            for step in &mut task.steps {
                                if matches!(
                                    step.phase,
                                    stepx_companion::amoo_workspace::Phase::Ready
                                        | stepx_companion::amoo_workspace::Phase::Running
                                ) {
                                    step.phase = stepx_companion::amoo_workspace::Phase::Done;
                                    step.note =
                                        format!("{}：{}", report.model_label, report.headline);
                                }
                            }
                        }
                    }
                    self.reveal_latest_chat();
                }
                self.doctor_report = Some(report);
            }
        }
        self.apply_auto_sync_results(sync_results);
        self.reconcile_auto_sync_domains_from_state();
        self.sync_notification_center();
    }

    pub(super) fn sync_live_express_parcels(&mut self) {
        if self.demo_mode {
            return;
        }
        let previous = self
            .selected
            .as_ref()
            .and_then(|device| self.push.prefs.parcel_receipts.get(&device.serial))
            .cloned()
            .unwrap_or_default();
        let mut receipts = stepx_companion::parcel_state::CompletionIndex::reconcile(
            &previous,
            &self.sms,
            &self.push.snapshot.notifications,
            &self.push.snapshot.threads,
        );
        let now = chrono::Local::now().timestamp_millis();
        receipts.merge(&Default::default(), now);
        receipts.record_capsule_signals(&self.fulfillment_signals(now), now);
        if let Some(serial) = self
            .selected
            .as_ref()
            .map(|d| d.serial.clone())
            .filter(|s| !s.is_empty() && !s.contains(':') && !s.contains('.'))
        {
            if self.push.prefs.parcel_receipts.get(&serial) != Some(&receipts) {
                self.push
                    .prefs
                    .parcel_receipts
                    .insert(serial, receipts.clone());
                if let Err(error) = self.push.prefs.save() {
                    eprintln!("Could not persist parcel receipts: {error}");
                }
            }
        }
        self.push.parcel_receipts = receipts;
        let mut parcels = stepx_companion::home_hub::extract_live_express_parcels_with_receipts(
            &self.sms,
            &self.push.snapshot.notifications,
            &self.push.snapshot.threads,
            &self.back_screen.stepos_suite.phone_clipboard,
            &self.push.parcel_receipts,
        );
        if !self.push.prefs.dismissed_pickup_codes.is_empty() {
            parcels.retain(|p| {
                let pk = p.pickup_code.trim().to_uppercase();
                let trk = p.tracking_no.trim().to_uppercase();
                (pk.is_empty() || !self.push.prefs.dismissed_pickup_codes.contains(&pk))
                    && (trk.is_empty() || !self.push.prefs.dismissed_pickup_codes.contains(&trk))
            });
        }
        self.express_parcels = parcels;
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
        match result {
            Ok(mut messages) => {
                for local_msg in &self.local_sent_sms {
                    let norm_local = stepx_companion::sms::normalize_address(&local_msg.address);
                    let present = messages.iter().any(|m| {
                        m.outgoing
                            && stepx_companion::sms::normalize_address(&m.address) == norm_local
                            && m.body.trim() == local_msg.body.trim()
                            && m.date_ms.abs_diff(local_msg.date_ms) < 120_000
                    });
                    if !present {
                        messages.insert(0, local_msg.clone());
                    }
                }

                let mut prefs_dirty = false;
                for m in &messages {
                    let norm_addr = stepx_companion::sms::normalize_address(&m.address);
                    let key1 = stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                        &m.address,
                    );
                    let key2 = stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                        &norm_addr,
                    );
                    let del_ts = self
                        .push
                        .prefs
                        .deleted_threads
                        .get(&key1)
                        .copied()
                        .or_else(|| self.push.prefs.deleted_threads.get(&key2).copied());
                    if let Some(ts) = del_ts {
                        if m.date_ms > ts {
                            self.push.prefs.deleted_threads.remove(&key1);
                            self.push.prefs.deleted_threads.remove(&key2);
                            prefs_dirty = true;
                        }
                    }
                }

                messages.retain(|m| {
                    let norm_addr = stepx_companion::sms::normalize_address(&m.address);
                    let key1 = stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                        &m.address,
                    );
                    let key2 = stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                        &norm_addr,
                    );
                    if self.push.prefs.deleted_threads.contains_key(&key1)
                        || self.push.prefs.deleted_threads.contains_key(&key2)
                    {
                        return false;
                    }
                    if let Some(clr_ts) = self
                        .push
                        .prefs
                        .cleared_threads
                        .get(&key1)
                        .copied()
                        .or_else(|| self.push.prefs.cleared_threads.get(&key2).copied())
                    {
                        if m.date_ms <= clr_ts {
                            return false;
                        }
                    }
                    let msg_key = format!("sms_msg:{norm_addr}::{}::{}", m.date_ms, m.body.trim());
                    !self.push.prefs.deleted_messages.contains(&msg_key)
                });

                messages.sort_by(|a, b| b.date_ms.cmp(&a.date_ms));

                let mut seen_manual_unread_addrs = std::collections::HashSet::new();
                for m in &mut messages {
                    let norm_addr = stepx_companion::sms::normalize_address(&m.address);
                    let key1 = stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                        &m.address,
                    );
                    let key2 = stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                        &norm_addr,
                    );
                    if (self.push.prefs.manual_unread_targets.contains(&key1)
                        || self.push.prefs.manual_unread_targets.contains(&key2))
                        && seen_manual_unread_addrs.insert(norm_addr)
                    {
                        m.read = false;
                    }
                }
                if prefs_dirty {
                    let _ = stepx_companion::desktop_notify::save_preferences(&self.push.prefs);
                }

                let is_initial_sms_load = !self.sms_loaded;
                self.sms_loaded = true;
                if is_initial_sms_load
                    && self.notification_center.connect_baseline_enabled
                    && self.selected.is_some()
                {
                    for m in &messages {
                        let raw_addr = m.address.trim();
                        let norm_addr = stepx_companion::sms::normalize_address(&m.address);
                        let key1 =
                            stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                                &m.address,
                            );
                        let key2 =
                            stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(
                                &norm_addr,
                            );
                        if !self.push.prefs.manual_unread_targets.contains(&key1)
                            && !self.push.prefs.manual_unread_targets.contains(&key2)
                        {
                            if !norm_addr.is_empty() {
                                self.read_sms_addresses.insert(norm_addr.clone());
                            }
                            if !raw_addr.is_empty() {
                                self.read_sms_addresses.insert(raw_addr.to_string());
                            }
                        }
                        if let Some(code) = stepx_companion::push::extract_otp_code(&m.body) {
                            for otp_id in [
                                format!("otp:{code}"),
                                format!("sms-otp:{}:{code}", m.date_ms),
                            ] {
                                self.notification_center.read_ids.insert(otp_id.clone());
                                self.notification_center.popup_shown_ids.insert(otp_id);
                            }
                        }
                        let sender = if raw_addr.is_empty() {
                            "短信"
                        } else {
                            raw_addr
                        };
                        for sms_id in [
                            format!("sms:{sender}:{}", m.date_ms),
                            format!("sms:{norm_addr}:{}", m.date_ms),
                            format!("sms-delivery:{}:{norm_addr}", m.date_ms),
                            format!("sms-delivery:{}:{raw_addr}", m.date_ms),
                            format!("sms-bill:{}:{norm_addr}", m.date_ms),
                            format!("sms-bill:{}:{raw_addr}", m.date_ms),
                            format!("sms-unread:{norm_addr}"),
                            format!("sms-unread:{raw_addr}"),
                        ] {
                            self.notification_center.read_ids.insert(sms_id.clone());
                            self.notification_center.popup_shown_ids.insert(sms_id);
                        }
                    }
                    self.notification_center.save();
                } else if !is_initial_sms_load {
                    let prev_keys: std::collections::HashSet<(String, i64, String)> = self
                        .sms
                        .iter()
                        .map(|m| {
                            (
                                stepx_companion::sms::normalize_address(&m.address),
                                m.date_ms,
                                m.body.trim().to_string(),
                            )
                        })
                        .collect();
                    let active_sms_norm = (self.push.shell_mode
                        == crate::ui::push_state::ShellMode::Full
                        && self.page == Page::Sms
                        && (self.push.im_channel == stepx_companion::push::ImChannel::Sms
                            || self.push.selected_im_thread.as_deref() == Some("")))
                    .then(|| {
                        self.selected_thread
                            .as_deref()
                            .map(stepx_companion::sms::normalize_address)
                    })
                    .flatten();
                    for m in &messages {
                        if !m.outgoing && !m.read {
                            let raw_addr = m.address.trim();
                            let norm_addr = stepx_companion::sms::normalize_address(&m.address);
                            let is_new = !prev_keys.contains(&(
                                norm_addr.clone(),
                                m.date_ms,
                                m.body.trim().to_string(),
                            ));
                            if is_new && active_sms_norm.as_deref() != Some(norm_addr.as_str()) {
                                self.read_sms_addresses.remove(&norm_addr);
                                self.read_sms_addresses.remove(raw_addr);
                            }
                        }
                    }
                }
                self.status = format!("已从手机读取 {} 条短信", messages.len());
                self.sms = messages;
                self.sync_read_sms_state();
                self.sms_error = None;
                self.sync_live_express_parcels();
                if self.push.prefs.auto_copy_otp && !is_initial_sms_load {
                    let now_ms = chrono::Local::now().timestamp_millis();
                    if let Some(code) = self
                        .sms
                        .iter()
                        .filter(|m| {
                            !m.outgoing
                                && !m.read
                                && (m.date_ms <= 0
                                    || now_ms <= 1_700_000_000_000
                                    || now_ms.saturating_sub(m.date_ms) <= 15 * 60_000)
                        })
                        .find_map(|m| stepx_companion::push::extract_otp_code(&m.body))
                    {
                        if self.notification_center.last_copied_text.as_deref()
                            != Some(code.as_str())
                        {
                            self.notification_center.last_copied_text = Some(code.clone());
                            self.notification_center.pending_clipboard_otp = Some(code);
                        }
                    }
                }
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
                if self.sms.is_empty() {
                    self.sms_loaded = false;
                    self.status = format!("短信读取失败：{error}");
                    self.sms_error = Some(error);
                    self.error = true;
                } else {
                    self.sms_loaded = true;
                    self.sms_error = None;
                    self.status =
                        format!("短信增量刷新受阻（已保留 {} 条本地缓存）", self.sms.len());
                }
            }
        }
    }

    pub(super) fn apply_phone_data(
        &mut self,
        logs: ReadResult<CallRecord>,
        contacts: ReadResult<ContactEntry>,
    ) {
        let is_initial_phone_load = !self.phone_loaded;
        self.phone_loaded = true;
        let mut partial_err = false;
        match logs {
            Ok(logs) => {
                self.call_logs = logs;
                self.call_logs_error = None;
            }
            Err(error) => {
                if self.call_logs.is_empty() {
                    self.call_logs_error = Some(error);
                } else {
                    self.call_logs_error = None;
                    partial_err = true;
                }
            }
        }
        match contacts {
            Ok(contacts) => {
                self.contacts = contacts;
                self.contacts_error = None;
            }
            Err(error) => {
                if self.contacts.is_empty() {
                    self.contacts_error = Some(error);
                } else {
                    self.contacts_error = None;
                    partial_err = true;
                }
            }
        }
        if (is_initial_phone_load
            && self.notification_center.connect_baseline_enabled
            && self.selected.is_some())
            || self.page == Page::Phone
        {
            let mut changed = false;
            for r in &self.call_logs {
                if r.call_type == 3 {
                    let raw_num = r.number.trim();
                    let num = if raw_num.is_empty() {
                        "unknown"
                    } else {
                        raw_num
                    };
                    for id in [
                        format!("missed-call:{num}:{}", r.date_ms),
                        format!("missed-call:{raw_num}:{}", r.date_ms),
                    ] {
                        if self.notification_center.read_ids.insert(id.clone()) {
                            changed = true;
                        }
                        if self.notification_center.popup_shown_ids.insert(id) {
                            changed = true;
                        }
                    }
                }
            }
            if changed {
                self.notification_center.save();
            }
        }
        self.error = self.call_logs_error.is_some() || self.contacts_error.is_some();
        self.status = if self.error {
            "部分手机数据读取失败，详情见页面".into()
        } else if partial_err {
            format!(
                "已保留 {} 条通话记录与 {} 位联系人缓存",
                self.call_logs.len(),
                self.contacts.len()
            )
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
        if self.connection_watch.user_paused {
            return;
        }
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
                                    if is_wireless(&device.transport) {
                                        let (summary_res, caps) = std::thread::scope(|s| {
                                            let h_summary = s.spawn(|| device.summary(adb));
                                            let h_caps =
                                                s.spawn(|| bridge::capabilities(adb, &device).ok());
                                            (h_summary.join().unwrap(), h_caps.join().unwrap())
                                        });
                                        let (sms, call_logs) = std::thread::scope(|s| {
                                            let h_sms =
                                                s.spawn(|| read_result(device.messages(adb)));
                                            let h_calls =
                                                s.spawn(|| read_result(device.call_logs(adb)));
                                            (h_sms.join().unwrap(), h_calls.join().unwrap())
                                        });
                                        let contacts = read_result(device.contacts(adb));
                                        (summary_res, caps, sms, call_logs, contacts)
                                    } else {
                                        std::thread::scope(|s| {
                                            let h_summary = s.spawn(|| device.summary(adb));
                                            let h_caps =
                                                s.spawn(|| bridge::capabilities(adb, &device).ok());
                                            let h_sms =
                                                s.spawn(|| read_result(device.messages(adb)));
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
                                        })
                                    };
                                let summary = summary_res.unwrap_or_default();
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
        self.connection_watch.user_paused = false;
        self.submit("正在验证设备…", move |adb| {
            let device = Device::inspect(adb, &id)?;
            let (summary_res, caps, sms, call_logs, contacts) = if is_wireless(&device.transport) {
                let (summary_res, caps) = std::thread::scope(|s| {
                    let h_summary = s.spawn(|| device.summary(adb));
                    let h_caps = s.spawn(|| bridge::capabilities(adb, &device).ok());
                    (h_summary.join().unwrap(), h_caps.join().unwrap())
                });
                let (sms, call_logs) = std::thread::scope(|s| {
                    let h_sms = s.spawn(|| read_result(device.messages(adb)));
                    let h_calls = s.spawn(|| read_result(device.call_logs(adb)));
                    (h_sms.join().unwrap(), h_calls.join().unwrap())
                });
                let contacts = read_result(device.contacts(adb));
                (summary_res, caps, sms, call_logs, contacts)
            } else {
                std::thread::scope(|s| {
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
                })
            };
            let summary = summary_res.unwrap_or_default();
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
        if real.memory_bm25_entries > 0 || !self.demo_mode {
            self.amoo_bm25_entries = real.memory_bm25_entries;
        } else {
            self.amoo_bm25_entries = self.amoo_bm25_entries.max(self.amoo_memories.len());
        }
        if real.total_skills_count > 0 || !self.demo_mode {
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

        if !real.skills.is_empty() || (!self.demo_mode && !cfg!(test)) {
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
            if self.amoo_total_skills == 0 {
                self.amoo_total_skills = self.amoo_skills.len();
            }
        }

        if !self.demo_mode && !cfg!(test) {
            self.amoo_memories = real
                .memory_entries
                .into_iter()
                .map(|(title, sub)| AmooMemoryEntry { title, sub })
                .collect();
        } else {
            let mut user_mems: Vec<AmooMemoryEntry> = self.amoo_memories.clone();
            for (title, sub) in real.memory_entries {
                if !user_mems.iter().any(|m| m.title == title) {
                    user_mems.push(AmooMemoryEntry { title, sub });
                }
            }
            self.amoo_memories = user_mems;
            self.amoo_bm25_entries = self.amoo_bm25_entries.max(self.amoo_memories.len());
        }
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
