//! Device, file, SMS, phone, apps, toolbox, and connection action handlers for `Companion`.

use stepx_companion::file_dialog::DialogKind;
use super::*;

impl Companion {
    pub(super) fn refresh(&mut self) {
        self.last_scan = Instant::now();
        let prefs = self.preferences.clone();
        let pairing_hosts = self.pairing_hosts.clone();
        self.submit("正在查找设备…", move |adb| {
            let (devices, services) =
                stepx_companion::reconnect::scan(adb, &prefs, &pairing_hosts)?;
            let devices = stepx_companion::device::deduplicate_transports(devices);
            Ok(Outcome::Transports(devices, services))
        });
        self.job_tool = None;
    }

    pub(super) fn auto_load_for_page(&mut self) {
        if self.page == Page::Studio && !self.studio.loaded && !self.busy {
            self.refresh_studio();
            return;
        }
        if self.selected.is_none() {
            if self.page == Page::Gallery
                && !self.demo_mode
                && !self.back_screen.gallery_checked
                && !self.back_screen.gallery_loading
            {
                let (_, mut exp_items) = stepx_companion::toolbox::load_expansion_gallery_items();
                exp_items.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
                self.back_screen.gallery_checked = true;
                if !exp_items.is_empty() {
                    let count = exp_items.len();
                    self.apply(Outcome::PhoneGallery(
                        exp_items,
                        format!("已从电脑扩容库加载 {count} 张归档照片"),
                    ));
                }
            }
            return;
        }
        match self.page {
            Page::Tools
                if self.tool_page == Some(ToolPage::Storage)
                    && self.storage_report.is_none()
                    && !self.demo_mode =>
            {
                self.handle_device_action(&Action::ScanStorage);
                return;
            }
            Page::Connect if !self.demo_mode && !self.companion_status.checked => {
                self.handle_device_action(&Action::CompanionInspect);
                return;
            }
            Page::Tools if !self.demo_mode => match self.tool_page {
                Some(ToolPage::Google) if !self.google_suite.checked => {
                    self.handle_device_action(&Action::GoogleSuiteInspect);
                    return;
                }
                Some(ToolPage::Companion) if !self.companion_status.checked => {
                    self.handle_device_action(&Action::CompanionInspect);
                    return;
                }
                Some(ToolPage::Neo) if !self.neo_status.checked => {
                    self.handle_device_action(&Action::NeoInspect);
                    return;
                }
                Some(ToolPage::InputMethod) if !self.input_method.checked => {
                    self.handle_device_action(&Action::InspectInputMethod);
                    return;
                }
                Some(ToolPage::Services) if self.system_services.is_empty() => {
                    self.handle_device_action(&Action::InspectServices);
                    return;
                }
                Some(ToolPage::Screen) => {
                    if !self.lan_cast.auto_started && self.selected.is_some() {
                        self.lan_cast.auto_started = true;
                        if let Ok(stats) = self.ensure_lan_cast_server_started() {
                            self.lan_cast.status_note = Some((
                                format!("局域网多协议投屏网关已就绪：{}", stats.web_url),
                                false,
                            ));
                        }
                        self.trigger_lan_cast_discovery();
                    }
                    return;
                }
                Some(ToolPage::Device) if self.summary.is_empty() && !self.busy => {
                    self.handle_device_action(&Action::Summary);
                    return;
                }
                None
                | Some(
                    ToolPage::Clipboard
                    | ToolPage::McpHub
                    | ToolPage::AiSafety
                    | ToolPage::Wellbeing,
                ) if !self.back_screen.stepos_suite.checked
                    && !self.back_screen.stepos_suite_loading
                    && self.selected.is_some() =>
                {
                    self.back_screen.stepos_suite_loading = true;
                    self.with_device_bulk(
                        "正在读取 StepOS 原生服务、屏幕统计与双端剪贴板…",
                        |adb, d| {
                            Ok(Outcome::StepOsSuite(
                                stepx_companion::toolbox::inspect_stepos_ai_suite(adb, &d)?,
                            ))
                        },
                    );
                    return;
                }
                _ => return,
            },
            Page::Apps | Page::AppStore if !self.apps_loaded => {
                self.handle_device_action(&Action::Apps);
                return;
            }
            _ => {}
        }
        if self.page == Page::Amoo && !self.demo_mode {
            self.refresh_amoo_background();
            return;
        }
        if !self.demo_mode
            && matches!(self.page, Page::Home | Page::Overview)
            && !self.amoo_ui.usage_attempted
        {
            self.start_account_usage_sync();
        }
        if self.busy {
            return;
        }
        match self.page {
            Page::Home
                if !self.demo_mode
                    && (self.summary.is_empty()
                        || (!self.sms_loaded && self.sms_error.is_none())
                        || (!self.phone_loaded && self.call_logs_error.is_none())) =>
            {
                self.with_device(
                    "正在刷新首页消息、日程与手机状态…",
                    |adb, d| {
                        let (summary_res, caps, sms, call_logs, contacts) =
                            std::thread::scope(|s| {
                                let h_sum = s.spawn(|| d.summary(adb));
                                let h_cap = s.spawn(|| bridge::capabilities(adb, &d).ok());
                                let h_sms = s.spawn(|| read_result(d.messages(adb)));
                                let h_log = s.spawn(|| read_result(d.call_logs(adb)));
                                let h_con = s.spawn(|| read_result(d.contacts(adb)));
                                (
                                    h_sum.join().unwrap(),
                                    h_cap.join().unwrap(),
                                    h_sms.join().unwrap(),
                                    h_log.join().unwrap(),
                                    h_con.join().unwrap(),
                                )
                            });
                        Ok(Outcome::HomeRefreshed(
                            summary_res?,
                            caps,
                            sms,
                            call_logs,
                            contacts,
                        ))
                    },
                );
            }
            Page::Overview if !self.demo_mode => {
                self.with_device("正在刷新设备状态…", |adb, d| {
                    Ok(Outcome::Summary(
                        d.summary(adb)?,
                        bridge::capabilities(adb, &d).ok(),
                    ))
                });
            }
            Page::Work if !self.demo_mode => self.refresh_work(),
            Page::Account if !self.demo_mode => {
                self.handle_amoo_ui_action(&AmooUiAction::RefreshUsage)
            }
            Page::Amoo if !self.demo_mode => {
                self.refresh_amoo_background();
            }
            Page::Files if !self.recent_files_view && self.files.is_empty() => {
                self.browse(self.directory.clone())
            }
            Page::Gallery
                if !self.demo_mode
                    && !self.back_screen.gallery_checked
                    && !self.back_screen.gallery_loading =>
            {
                self.back_screen.gallery_loading = true;
                self.with_device("正在读取手机相册…", |adb, d| {
                    let items = stepx_companion::toolbox::list_phone_gallery(adb, &d)?;
                    let msg = format!("已加载手机相册 {} 张照片", items.len());
                    Ok(Outcome::PhoneGallery(items, msg))
                });
            }
            Page::Sms if !self.demo_mode => {
                self.with_device("正在读取短信…", |adb, d| {
                    let caps = bridge::capabilities(adb, &d).ok();
                    Ok(Outcome::Sms(read_result(d.messages(adb)), caps))
                });
            }
            Page::Notifications if !self.demo_mode => {
                if !self.push.loaded && !self.push.syncing && self.push.error.is_none() {
                    self.trigger_immediate_push_sync();
                }
                if !self.back_screen.stepos_suite.checked
                    && !self.back_screen.stepos_suite_loading
                    && self.selected.is_some()
                {
                    self.back_screen.stepos_suite_loading = true;
                    self.with_device_bulk(
                        "正在读取 StepOS 原生服务、日程与双端剪贴板…",
                        |adb, d| {
                            Ok(Outcome::StepOsSuite(
                                stepx_companion::toolbox::inspect_stepos_ai_suite(adb, &d)?,
                            ))
                        },
                    );
                }
                if (!self.sms_loaded && self.sms_error.is_none())
                    || (!self.phone_loaded && self.call_logs_error.is_none())
                {
                    self.spawn_message_sync();
                }
            }
            Page::Phone if !self.demo_mode => {
                self.with_device("正在读取通话记录与通讯录…", |adb, d| {
                    let logs = read_result(d.call_logs(adb));
                    let contacts = read_result(d.contacts(adb));
                    Ok(Outcome::PhoneData(logs, contacts))
                });
            }
            Page::Studio => {}
            _ => {}
        }
    }

    pub(super) fn clear_device_state(&mut self) {
        self.doctor_report = None;
        self.doctor_running = false;
        self.auto_sync.on_device_disconnected();
        self.home_ai = Default::default();
        self.bg_receivers.clear();
        self.context.device_cleared();
        self.clear_text_interaction();
        self.speech = Default::default();
        self.bulk_rx = None;
        self.bulk_busy = false;
        self.bulk_status.clear();
        self.bulk_tool = None;
        self.bulk_started_at = None;
        let mode = self.push.shell_mode;
        let pin = self.push.pin_window;
        let prev_seen_toasts = std::mem::take(&mut self.push.seen_toast_keys);
        self.push = PushUiState::default();
        self.push.shell_mode = mode;
        self.push.pin_window = pin;
        self.push.seen_toast_keys = prev_seen_toasts;
        for index in [F_IM_REPLY, F_PUSH_REPLY, F_TOAST_REPLY] {
            self.fields[index].clear();
        }
        self.studio_lab.cloud_models.clear();
        self.studio_lab.cloud_model = None;
        self.studio_lab.cloud_scope = None;
        self.studio.phone_note = None;
        self.studio.cloud = None;
        self.studio.web = None;
        self.studio.web_selected = None;
        self.studio.web_error = None;
        self.studio.cloud_selected = None;
        self.studio.cloud_checked = None;
        self.studio.cloud_error = None;
        self.studio.connecting_until = None;
        let native_gui_unconfirmed =
            (self.amoo_ui.gui_running && !self.amoo_ui.gui_desktop) || self.amoo_ui.gui_unconfirmed;
        self.google_suite = GoogleSuiteStatus::default();
        self.companion_status = CompanionStatus::default();
        self.neo_status = Default::default();
        self.phone_link_notice = None;
        self.deep_backscreen_granted = false;
        self.deep_amoo_granted = false;
        self.amoo_bg_keepalive = false;
        self.system_services.clear();
        self.tool_confirmation = None;
        self.tools_feedback = None;
        self.storage_report = None;
        self.tools_pending = None;
        self.tools_result_page = 0;
        self.system_compression = None;
        self.tools_last_result = None;
        self.stop_mirror();
        self.capabilities = None;
        self.amoo_snapshot = None;
        self.amoo_poll_remaining = 0;
        self.amoo_pending_reply.clear();
        self.input_method = Default::default();
        self.amoo_ui = AmooUiState::default();
        self.amoo_attachments.clear();
        self.amoo_cron_jobs.clear();
        self.amoo_gateway_online = false;
        self.work.disconnect();
        let prev_clip = self.back_screen.last_synced_phone_clip.clone();
        let prev_dismissed = std::mem::take(&mut self.back_screen.dismissed_capsule_ids);
        self.back_screen = BackScreenUiState::default();
        self.back_screen.last_synced_phone_clip = prev_clip;
        self.back_screen.dismissed_capsule_ids = prev_dismissed;
        self.updates = UpdateState::default();
        if !self.demo_mode {
            self.amoo_sessions.clear();
            self.amoo_skills.clear();
            self.amoo_cron_count = 0;
            self.amoo_total_skills = 0;
            self.amoo_bm25_entries = 0;
            self.express_parcels.clear();
        }
        self.pending_sms_plan = None;
        self.last_sms_status = None;
        self.selected_thread = None;
        self.content.clear();
        self.sms.clear();
        self.local_sent_sms.clear();
        self.sms_loaded = false;
        self.phone_loaded = false;
        self.pending_call = None;
        self.selected_call_sim = None;
        self.call_logs.clear();
        self.contacts.clear();
        self.sms_error = None;
        self.call_logs_error = None;
        self.contacts_error = None;
        self.selected_sim = None;
        self.active_call_number = None;
        self.selected_contact = 0;
        for field in [
            F_SMS_TO,
            F_SMS_BODY,
            F_PHONE_DIAL,
            F_PHONE_CONTACT_NAME,
            F_CONTACT_SEARCH,
            F_SMS_SEARCH,
        ] {
            self.fields[field].clear();
        }
        self.summary.clear();
        self.files.clear();
        self.apps.clear();
        self.apps_loaded = false;
        self.apps_error = None;
        self.pending_app_uninstall = None;
        self.live_screen_rx = None;
        self.live_screen_stream = false;
        self.screenshot = None;
        self.selected_file = None;
        self.file_modal = None;
        self.trash_confirm = false;
        let prev_seen_updatable_apps = self.notification_center.seen_updatable_apps.clone();
        let prev_app_update_read_ids: Vec<String> = self
            .notification_center
            .read_ids
            .iter()
            .filter(|id| id.starts_with("app-update:") || *id == "app-updates-summary")
            .cloned()
            .collect();
        self.sync_notification_center();
        self.notification_center.seen_updatable_apps = prev_seen_updatable_apps;
        self.notification_center
            .read_ids
            .retain(|id| !id.starts_with("app-update:") && id != "app-updates-summary");
        self.notification_center
            .read_ids
            .extend(prev_app_update_read_ids);
        if native_gui_unconfirmed {
            self.status = "已停止桌面等待，手机原生操作可能仍继续，请在手机上核对".into();
        }
    }

    pub(super) fn browse(&mut self, path: String) {
        self.recent_files_view = false;
        if self.busy {
            return;
        }
        self.with_device("正在读取目录…", move |adb, device| {
            Ok(Outcome::Files(
                path.clone(),
                storage::list(adb, &device, &path)?,
                None,
            ))
        });
    }

    pub(super) fn stop_mirror(&mut self) {
        self.mirror_interrupted_at = None;
        if let Some(child) = self.mirror.take() {
            child.stop_in_background();
        }
    }

    pub(super) fn start_mirror(&mut self) {
        self.demote_busy_if_background();
        if !self.busy && self.mirror.is_none() {
            let options = self.mirror_options;
            self.with_device("正在开启投屏…", move |adb, d| {
                let active_transport = d
                    .resolve_active_transport(adb)
                    .unwrap_or_else(|_| d.transport.clone());
                let session =
                    stepx_companion::mirror::start_with_options(adb, &active_transport, options)?;
                Ok(Outcome::Mirror(d.serial, active_transport, session))
            });
        }
    }

    pub(super) fn restart_active_mirror(&mut self, status_prefix: String) {
        self.error = false;
        self.demote_busy_if_background();
        if self.mirror.is_some() && !self.busy {
            self.stop_mirror();
            let options = self.mirror_options;
            self.with_device(
                &format!("{status_prefix}，正在无缝切换投屏通道…"),
                move |adb, d| {
                    let active_transport = d
                        .resolve_active_transport(adb)
                        .unwrap_or_else(|_| d.transport.clone());
                    let session = stepx_companion::mirror::start_with_options(
                        adb,
                        &active_transport,
                        options,
                    )?;
                    Ok(Outcome::Mirror(d.serial, active_transport, session))
                },
            );
        } else {
            self.status = status_prefix;
            if self.page == Page::Tools && self.tool_page == Some(ToolPage::Screen) {
                self.tools_feedback = Some((Some(ToolPage::Screen), self.status.clone(), false));
            }
        }
    }

    pub(super) fn filtered_files(&self) -> Vec<(usize, &Entry)> {
        let query = self.fields[F_FILE_SEARCH].text.trim().to_lowercase();
        let in_trash = self.directory.contains(".StepXTrash");
        let mut items: Vec<(usize, &Entry)> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                !super::pages::files::is_hidden_phone_file_entry(&e.name, in_trash)
                    && (query.is_empty()
                        || e.name.to_lowercase().contains(&query)
                        || e.kind_badge().contains(&query))
            })
            .collect();
        match self.file_sort {
            FileSort::Name => {}
            FileSort::Size => items.sort_by(|(_, a), (_, b)| {
                b.directory
                    .cmp(&a.directory)
                    .then(b.bytes.cmp(&a.bytes))
                    .then(a.name.cmp(&b.name))
            }),
            FileSort::Kind => items.sort_by(|(_, a), (_, b)| {
                b.directory
                    .cmp(&a.directory)
                    .then(a.kind_badge().cmp(b.kind_badge()))
                    .then(a.name.cmp(&b.name))
            }),
            FileSort::Time => items.sort_by(|(_, a), (_, b)| {
                b.directory
                    .cmp(&a.directory)
                    .then(b.sort_time_key().cmp(&a.sort_time_key()))
                    .then(b.name.cmp(&a.name))
            }),
        }
        items
    }

    pub(super) fn thread_messages(&self, address: &str) -> Vec<&Message> {
        let norm_target = stepx_companion::sms::normalize_address(address);
        let mut items: Vec<&Message> = self
            .sms
            .iter()
            .filter(|m| stepx_companion::sms::normalize_address(&m.address) == norm_target)
            .collect();
        items.sort_by_key(|m| m.date_ms);
        items
    }

    pub(super) fn sms_threads(&self) -> Vec<SmsThread> {
        let query = self.fields[F_SMS_SEARCH].text.trim().to_lowercase();
        let mut contact_names: HashMap<String, String> = HashMap::new();
        if !query.is_empty() {
            for c in &self.contacts {
                let norm = stepx_companion::sms::normalize_address(&c.number);
                let name_lower = c.name.to_lowercase();
                contact_names
                    .entry(norm)
                    .and_modify(|existing| {
                        existing.push('\n');
                        existing.push_str(&name_lower);
                    })
                    .or_insert(name_lower);
            }
        }
        let selected_norm = self
            .selected_thread
            .as_deref()
            .filter(|s| !s.is_empty() && *s != "__new__")
            .map(stepx_companion::sms::normalize_address)
            .unwrap_or_default();
        let mut threads: Vec<SmsThread> = Vec::new();
        let mut thread_flags: Vec<(bool, bool)> = Vec::new();
        let mut thread_indices: HashMap<String, usize> = HashMap::new();
        for m in &self.sms {
            let addr = m.address.trim();
            let norm_addr = stepx_companion::sms::normalize_address(addr);
            if !query.is_empty()
                && !m.address.to_lowercase().contains(&query)
                && !m.body.to_lowercase().contains(&query)
                && !contact_names
                    .get(&norm_addr)
                    .is_some_and(|names| names.contains(&query))
            {
                continue;
            }
            let is_in = m.kind == 1 && !m.outgoing;
            let is_out = m.kind == 2 || m.outgoing;
            if let Some(&idx) = thread_indices.get(&norm_addr) {
                let t = &mut threads[idx];
                t.count += 1;
                t.unread |= is_in && !m.read;
                thread_flags[idx].0 |= is_in;
                thread_flags[idx].1 |= is_out;
                if m.date_ms >= t.last_ms {
                    t.last_ms = m.date_ms;
                    t.last_body = m.body.clone();
                }
            } else {
                thread_indices.insert(norm_addr, threads.len());
                thread_flags.push((is_in, is_out));
                threads.push(SmsThread {
                    address: addr.to_string(),
                    count: 1,
                    last_ms: m.date_ms,
                    last_body: m.body.clone(),
                    unread: is_in && !m.read,
                });
            }
        }
        let mut filtered: Vec<SmsThread> = threads
            .into_iter()
            .zip(thread_flags)
            .filter_map(|(t, (has_in, has_out))| {
                let norm = stepx_companion::sms::normalize_address(&t.address);
                let is_selected = !selected_norm.is_empty() && norm == selected_norm;
                let matches_filter = match self.sms_filter {
                    SmsFilter::All => true,
                    SmsFilter::Incoming => has_in || is_selected,
                    SmsFilter::Outgoing => has_out || is_selected,
                };
                matches_filter.then_some(t)
            })
            .collect();
        filtered.sort_by(|a, b| b.last_ms.cmp(&a.last_ms));
        filtered
    }

    pub(super) fn is_stepos_or_system_app(app: &AppInfo) -> bool {
        app.system
            || app.updated_system
            || app.package == "android"
            || app.package.starts_with("android.")
            || app.package.starts_with("com.android.")
            || app.package.starts_with("vendor.")
            || app.package.starts_with("com.qti.")
            || app.package.starts_with("com.qualcomm.")
            || app.package.starts_with("com.stepos.")
            || app.package.starts_with("com.stepx.")
            || (!app.launchable && app.package.starts_with("com.google."))
    }

    pub(super) fn filtered_apps(&self) -> Vec<&AppInfo> {
        let query = self.fields[F_APP_SEARCH].text.trim();
        self.apps
            .iter()
            .filter(|app| {
                (self.include_system_apps
                    || (app.in_default_scope() && !Self::is_stepos_or_system_app(app)))
                    && app.matches_query(query)
            })
            .collect()
    }

    pub(super) fn save_preferences(&mut self, ok: &str) {
        match self.preferences.save() {
            Ok(()) => {
                self.status = ok.into();
                self.error = false;
            }
            Err(e) => {
                self.status = e.to_string();
                self.error = true;
            }
        }
    }

    pub(super) fn save_screenshot(&mut self) {
        let Some((_, src)) = &self.screenshot else {
            return;
        };
        let src = src.clone();
        let name = format!("StepX-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"));
        self.choose_path(
            stepx_companion::file_dialog::FileDialog::new()
                .set_title("保存截图")
                .set_filename(&name)
                .set_location(&stepx_companion::settings::downloads()),
            DialogKind::SaveFile,
            move |app, dest, _| {
                std::fs::copy(&src, &dest)?;
                app.status = format!("截图已保存到 {}", dest.display());
                Ok(())
            },
        );
    }

    pub(super) fn activate_file(&mut self, i: usize) -> Option<Action> {
        if self.busy {
            return None;
        }
        let entry = self.files.get(i)?;
        self.selected_file = Some(i);
        Some(if entry.directory {
            Action::Browse(entry.path.clone())
        } else {
            Action::OpenFile(i)
        })
    }

    pub(super) fn handle_device_action(&mut self, action: &Action) {
        let origin = interaction::OperationFailure::for_action(
            action,
            self.page,
            self.selected.as_ref().map(|d| d.serial.clone()),
        );
        let previous = self.interaction.active_operation.replace(origin);
        self.handle_device_action_inner(action);
        self.interaction.active_operation = previous;
    }

    fn handle_device_action_inner(&mut self, action: &Action) {
        let is_bulk_action = matches!(
            action,
            Action::ScanStorage
                | Action::ProbeSystemCompression
                | Action::InspectServices
                | Action::InspectInputMethod
                | Action::GoogleSuiteInspect
                | Action::CompanionInspect
                | Action::NeoInspect
                | Action::Apps
                | Action::ExportApp(_)
                | Action::Mirror
                | Action::OpenAppInMirror(_)
                | Action::RunDoctorHarness { .. }
                | Action::ExecuteDoctorQuickAction(_)
        );
        if !is_bulk_action && self.busy && self.page == Page::Tools {
            return;
        }
        match action {
            Action::ScanStorage => {
                if self.bulk_busy {
                    return;
                }
                self.storage_report = None;
                self.tools_pending = None;
                self.with_device_bulk(
                    "正在扫描公开目录并核对重复文件…",
                    |adb, d| {
                        Ok(Outcome::StorageScanned(
                            d.serial.clone(),
                            toolbox::scan(adb, &d)?,
                            toolbox::probe_system(adb, &d).ok(),
                        ))
                    },
                );
            }
            Action::ProbeSystemCompression => {
                if self.bulk_busy {
                    return;
                }
                self.system_compression = None;
                self.with_device_bulk("正在检测系统压缩能力…", |adb, d| {
                    Ok(Outcome::SystemCompression(
                        d.serial.clone(),
                        toolbox::probe_system(adb, &d)?,
                    ))
                });
            }
            Action::OpenStorageSettings => {
                if self.busy {
                    return;
                }
                self.with_device("正在打开手机存储设置…", |adb, d| {
                    toolbox::open_storage_settings(adb, &d)?;
                    Ok(Outcome::Notice("已在手机上打开存储设置".into()))
                });
            }
            Action::ConfirmToolOperation => {
                if self.busy {
                    return;
                }
                if let Some(operation) = self.tools_pending.take() {
                    self.storage_report = None;
                    self.with_device(
                        "正在处理手机文件并校验内容…",
                        move |adb, d| {
                            let result = toolbox::operate(adb, &d, &operation)?;
                            let report = toolbox::scan(adb, &d).ok();
                            Ok(Outcome::ToolOperated(
                                d.serial.clone(),
                                operation.label().into(),
                                result,
                                report,
                            ))
                        },
                    );
                }
            }
            Action::Refresh => self.refresh(),
            Action::Select(id) => self.select(id.clone()),
            Action::Forget => {
                // Disconnect pauses this client, it does not revoke trust or erase pairing.
                self.allow_first_connect = false;
                self.connection_watch.user_paused = true;
                self.qr = None;
                self.connection_watch.detached_serial = None;
                if let Some(device) = self.selected.take() {
                    self.last_connected_serial = Some(device.serial);
                }
                self.clear_device_state();
                self.status = "已断开并暂停自动连接；已信任手机保留，点击连接可恢复".into();
            }
            Action::PairCode => {
                let endpoint = self.fields[F_PAIR_ADDR].text.trim().to_string();
                let code = self.fields[F_PAIR_CODE].text.trim().to_string();
                if code.is_empty() {
                    self.status = "请输入配对码".into();
                    self.error = true;
                    return;
                }
                self.connection_watch.user_paused = false;
                self.submit("正在配对…", move |adb| {
                    let endpoint = if endpoint.is_empty() {
                        pairing::unique_code_endpoint(
                            &stepx_companion::discovery::discover_services(adb)?,
                        )?
                    } else {
                        endpoint
                    };
                    pairing::pair_code(adb, &endpoint, &code)?;
                    Ok(Outcome::Paired(endpoint))
                });
            }
            Action::Connect | Action::ConnectEndpoint(_) => {
                let endpoint = match action {
                    Action::ConnectEndpoint(e) => e.trim().to_string(),
                    _ => self.fields[F_CONNECT].text.trim().to_string(),
                };
                if let Err(error) = pairing::validate_endpoint(&endpoint) {
                    self.status = error.to_string();
                    self.error = true;
                    return;
                }
                self.connection_watch.user_paused = false;
                self.submit("正在连接…", move |adb| {
                    pairing::validate_endpoint(&endpoint)?;
                    adb.run(&["connect", &endpoint])?;
                    Ok(Outcome::Paired(endpoint))
                });
            }
            Action::Qr => match QrSession::new() {
                Ok(qr) => {
                    self.connection_watch.user_paused = false;
                    self.qr = Some(Arc::new(qr));
                    self.pair_tab = PairTab::Qr;
                    self.status = "已生成新的二维码".into();
                    self.last_scan = Instant::now() - Duration::from_secs(6);
                }
                Err(e) => {
                    self.status = e.to_string();
                    self.error = true;
                }
            },
            Action::Summary => self.with_device("正在读取设备状态…", |adb, d| {
                let summary = d.summary(adb)?;
                let caps = bridge::capabilities(adb, &d).ok();
                Ok(Outcome::Summary(summary, caps))
            }),
            Action::Mirror => {
                self.demote_busy_if_background();
                if !self.busy && self.mirror.is_none() {
                    let options = self.mirror_options;
                    self.with_device("正在开启投屏…", move |adb, d| {
                        let active_transport = d
                            .resolve_active_transport(adb)
                            .unwrap_or_else(|_| d.transport.clone());
                        let session = stepx_companion::mirror::start_with_options(
                            adb,
                            &active_transport,
                            options,
                        )?;
                        Ok(Outcome::Mirror(d.serial, active_transport, session))
                    });
                }
            }
            Action::Screenshot => self.with_device("正在截图…", |adb, d| {
                let active_transport = d.resolve_active_transport(adb)?;
                let bytes = adb.screenshot(&active_transport)?;
                let dir = tempfile::tempdir()?;
                let path = dir.path().join("screen.png");
                std::fs::write(&path, bytes)?;
                Ok(Outcome::Screenshot(dir, path))
            }),
            Action::Key(code) => {
                let code = *code;
                let sync_screen = self.mirror.is_none();
                self.with_device("正在发送按键…", move |adb, d| {
                    d.key(adb, code)?;
                    let should_refresh_preview =
                        sync_screen && (code != 26 && code != 223 || code == 223 || code == 26);
                    if should_refresh_preview {
                        let delay_ms = if code == 224 {
                            320
                        } else if code == 223 || code == 26 {
                            220
                        } else {
                            180
                        };
                        std::thread::sleep(Duration::from_millis(delay_ms));
                        if let Ok(bytes) = adb.screenshot(&d.transport) {
                            if let Ok(dir) = tempfile::tempdir() {
                                let path = dir.path().join("screen.png");
                                if std::fs::write(&path, bytes).is_ok() {
                                    return Ok(Outcome::ScreenshotWithNotice(
                                        dir,
                                        path,
                                        "已发送按键并同步手机画面".into(),
                                    ));
                                }
                            }
                        }
                    }
                    Ok(Outcome::Notice("已发送按键".into()))
                });
            }
            Action::ScreenTapNormalized(nx, ny) => {
                let (nx, ny) = (*nx, *ny);
                self.with_device(
                    "正在点击手机屏幕并同步画面…",
                    move |adb, d| {
                        let (px, py) = d.tap_normalized(adb, nx, ny)?;
                        std::thread::sleep(Duration::from_millis(220));
                        if let Ok(bytes) = adb.screenshot(&d.transport) {
                            if let Ok(dir) = tempfile::tempdir() {
                                let path = dir.path().join("screen.png");
                                if std::fs::write(&path, bytes).is_ok() {
                                    return Ok(Outcome::ScreenshotWithNotice(
                                        dir,
                                        path,
                                        format!("已点击手机屏幕坐标 ({px}, {py}) 并同步最新画面"),
                                    ));
                                }
                            }
                        }
                        Ok(Outcome::Notice(format!("已点击手机屏幕坐标 ({px}, {py})")))
                    },
                );
            }
            Action::ScreenGesture(gesture) => {
                let gesture = *gesture;
                self.with_device(
                    "正在执行屏幕手势并同步画面…",
                    move |adb, d| {
                        let msg = d.screen_gesture(adb, gesture)?;
                        std::thread::sleep(Duration::from_millis(260));
                        if let Ok(bytes) = adb.screenshot(&d.transport) {
                            if let Ok(dir) = tempfile::tempdir() {
                                let path = dir.path().join("screen.png");
                                if std::fs::write(&path, bytes).is_ok() {
                                    return Ok(Outcome::ScreenshotWithNotice(
                                        dir,
                                        path,
                                        format!("{msg} · 画面已同步"),
                                    ));
                                }
                            }
                        }
                        Ok(Outcome::Notice(msg))
                    },
                );
            }
            Action::Amoo | Action::AmooSnapshot | Action::AmooOpenTasks => {
                if matches!(action, Action::AmooOpenTasks) {
                    self.amoo_show_schedules = true;
                }
                if self.selected.is_none() {
                    self.page = Page::Connect;
                    self.status = "请先连接设备".into();
                    self.error = true;
                    return;
                }
                self.error = false;
                self.status = "正在后台同步对话…".into();
                self.refresh_amoo_background();
            }
            Action::AmooResetSession => {
                self.demote_busy_if_background();
                self.with_device("正在后台创建新会话…", |adb, d| {
                    Ok(Outcome::AmooCreated(
                        amoo::AgentBus::connect(adb, &d)?.create_session()?,
                    ))
                });
            }
            Action::AmooRetryMessage { session_key, text } => {
                self.demote_busy_if_background();
                if self.busy || self.amoo_pending_reply.contains_key(session_key) {
                    self.status = "上一条请求仍在处理中，请等待结果".into();
                    return;
                }
                if self.demo_mode {
                    self.fields[F_AMOO].set(text);
                    return;
                }
                let key = session_key.clone();
                let text = text.clone();
                self.with_device("正在重新发送这条请求…", move |adb, d| {
                    let receipt = amoo::AgentBus::connect(adb, &d)?.send(&key, &text)?;
                    Ok(Outcome::AmooSent(receipt))
                });
            }
            Action::AmooRetryReply => {
                if self
                    .amoo_sessions
                    .get(self.amoo_session_idx)
                    .is_some_and(|s| self.amoo_pending_reply.contains_key(&s.session_key))
                {
                    self.status = "上一条消息仍在等待结果，正在继续同步，请勿重复发送".into();
                    return;
                }
                let Some(session) = self.amoo_sessions.get(self.amoo_session_idx) else {
                    return;
                };
                let key = session.session_key.clone();
                let text = session.items.iter().rev().find_map(|item| match item {
                    AmooChatItem::UserText { text, .. } => Some(text.clone()),
                    _ => None,
                });
                if let Some(text) = text {
                    if self.demo_mode {
                        self.fields[F_AMOO].set(&text);
                    } else {
                        self.with_device("正在后台重新发送请求…", move |adb, d| {
                            let receipt = amoo::AgentBus::connect(adb, &d)?.send(&key, &text)?;
                            Ok(Outcome::AmooSent(receipt))
                        });
                    }
                }
            }
            Action::AmooPushArtifactBolt => {
                if self.back_screen.playing
                    || self.back_screen.generating
                    || self.back_screen.applying_native
                    || self.back_screen.companion_busy
                {
                    self.status = "请先停止背屏播放或等待创作完成".into();
                    self.error = true;
                    return;
                }
                self.page = Page::BackScreen;
                self.back_screen.native_feature = None;
                self.back_screen.source = stepx_companion::backscreen::Source::Image(
                    BackScreenPattern::Bolt.frame_391(8, false).to_vec(),
                );
                self.back_screen.frame = stepx_companion::backscreen::frame(
                    &self.back_screen.source,
                    &self.back_screen.config,
                    Duration::ZERO,
                    &Default::default(),
                );
                self.back_screen.image_path = None;
                self.back_screen.title = "赛博闪电".into();
                self.back_screen.origin = "内置示例图案".into();
                self.back_screen.description = "请预览灯珠布局，再选择在手机播放。".into();
                self.back_screen.status = "图案已载入预览，尚未推送手机".into();
                self.status = self.back_screen.status.clone();
            }
            Action::AmooPrepare | Action::AmooSend => {
                if matches!(action, Action::AmooPrepare) {
                    self.status = "消息已保留在桌面输入框，点击发送即可从后台提交".into();
                } else {
                    self.send_amoo_draft();
                }
            }
            Action::ReadVisible => {
                self.with_device("正在读取当前手机画面文字…", |adb, d| {
                    let _ = adb.shell(&d.transport, &["input", "keyevent", "224"]);
                    Ok(Outcome::Text(d.visible_text(adb)?))
                })
            }
            Action::ReadSms => self.with_device("正在读取短信…", |adb, d| {
                let msgs = read_result(d.messages(adb));
                let caps = bridge::capabilities(adb, &d).ok();
                Ok(Outcome::Sms(msgs, caps))
            }),
            Action::OpenSms => self.with_device("正在打开短信…", |adb, d| {
                d.open_messages(adb)?;
                Ok(Outcome::Notice("已在手机上打开短信".into()))
            }),
            Action::DraftSms => {
                let number = self.fields[F_SMS_TO].text.clone();
                let body = self.fields[F_SMS_BODY].text.clone();
                self.with_device("正在创建草稿…", move |adb, d| {
                    Ok(Outcome::Notice(d.draft_sms(adb, &number, &body)?))
                });
            }
            Action::PrepareDirectSms => {
                let to = if self.fields[F_SMS_TO].text.trim().is_empty() {
                    self.selected_thread
                        .as_deref()
                        .filter(|s| *s != "__new__" && !s.trim().is_empty())
                        .map(str::to_string)
                        .or_else(|| self.sms_threads().first().map(|t| t.address.clone()))
                        .or_else(|| self.demo_mode.then(|| "10086".to_string()))
                        .unwrap_or_default()
                } else {
                    self.fields[F_SMS_TO].text.trim().to_string()
                };
                let raw_body = self.fields[F_SMS_BODY].text.trim().to_string();
                if to.is_empty() || raw_body.is_empty() {
                    self.status = "请填写收件人号码和短信内容".into();
                    self.error = true;
                    return;
                }
                if !self.demo_mode && self.selected.is_none() {
                    self.status = "手机已断开，短信未发送；收件人和正文已保留".into();
                    self.error = true;
                    return;
                }
                let body = if let Some((q_sender, q_text, true)) = self.push.reply_quote.take() {
                    if q_sender.trim().is_empty() {
                        format!("「{q_text}」\n{raw_body}")
                    } else {
                        format!("「{}: {q_text}」\n{raw_body}", q_sender.trim())
                    }
                } else {
                    raw_body
                };
                let now_ms = chrono::Utc::now().timestamp_millis();
                let optimistic_msg = stepx_companion::sms::Message {
                    provider_id: Some(now_ms),
                    address: to.clone(),
                    body: body.clone(),
                    date_ms: now_ms,
                    outgoing: true,
                    kind: 2,
                    read: true,
                    subscription: self.selected_sim,
                };
                self.fields[F_SMS_TO].set(&to);

                self.pending_sms_plan = None;
                self.selected_thread = Some(to.clone());
                self.sms.insert(0, optimistic_msg.clone());
                self.local_sent_sms.insert(0, optimistic_msg.clone());
                self.fields[F_SMS_BODY].clear();
                Self::scroll_to_end(&self.sms_messages_scroll);
                if self.demo_mode {
                    self.status = format!("已发送短信至 {to}（演示模式）");
                    self.error = false;
                    return;
                }
                self.status = format!("正在向 {to} 发送短信，等待手机确认…");
                self.error = false;
                let sim = self.selected_sim;
                if let Some(mut device) = self.selected.clone() {
                    let (tx, rx) = mpsc::channel();
                    self.bg_receivers.push((Page::Sms, None, rx));
                    std::thread::spawn(move || {
                        let outcome = Adb::discover().and_then(|adb| {
                            device.transport = device.resolve_active_transport(&adb)?;
                            let plan = bridge::sms_prepare(&adb, &device, &to, &body, sim)?;
                            let status = bridge::sms_send(&adb, &device, &plan)?;
                            let mut messages = read_result(device.messages(&adb));
                            merge_confirmed_sms(&mut messages, &status, optimistic_msg);
                            Ok(Outcome::SmsSent(status, messages))
                        });
                        let _ = tx.send(outcome);
                    });
                }
            }
            Action::ConfirmDirectSms => {
                let Some(plan) = self.pending_sms_plan.take() else {
                    self.status = "请先准备短信并确认收件人和 SIM 卡".into();
                    self.error = true;
                    return;
                };
                let now_ms = chrono::Utc::now().timestamp_millis();
                let optimistic_msg = stepx_companion::sms::Message {
                    provider_id: Some(now_ms),
                    address: plan.number.clone(),
                    body: plan.body.clone(),
                    date_ms: now_ms,
                    outgoing: true,
                    kind: 2,
                    read: true,
                    subscription: self.selected_sim,
                };
                self.selected_thread = Some(plan.number.clone());
                self.sms.insert(0, optimistic_msg.clone());
                self.local_sent_sms.insert(0, optimistic_msg.clone());
                self.fields[F_SMS_BODY].clear();
                Self::scroll_to_end(&self.sms_messages_scroll);
                // Consume the confirmation once; never fabricate a send plan or retry a timed-out send.
                self.with_device("正在通过手机发送短信…", move |adb, d| {
                    let status = bridge::sms_send(adb, &d, &plan)?;
                    let mut messages = read_result(d.messages(adb));
                    merge_confirmed_sms(&mut messages, &status, optimistic_msg);
                    Ok(Outcome::SmsSent(status, messages))
                });
            }
            Action::CheckSmsStatus(request_id) => {
                let request_id = request_id.clone();
                self.with_device("正在查询发送状态…", move |adb, d| {
                    Ok(Outcome::SmsStatus(bridge::sms_status(
                        adb,
                        &d,
                        &request_id,
                    )?))
                });
            }
            Action::Browse(path) => self.browse(path.clone()),
            Action::SubmitFileModal => self.submit_file_modal(),
            Action::Upload => {
                if self.busy {
                    return;
                }
                let directory = self.directory.clone();
                let device = self.selected.as_ref().map(|d| d.serial.clone());
                self.choose_path(
                    stepx_companion::file_dialog::FileDialog::new().set_title("选择要上传到手机的文件"),
                    DialogKind::OpenFile,
                    move |app, path, _| {
                        app.ensure_file_dialog_device(&device)?;
                        anyhow::ensure!(!app.busy, "当前操作尚未完成，请重新选择要上传的文件");
                        app.prepare_file_transfer(vec![path], Some(directory));
                        Ok(())
                    },
                );
            }
            Action::TransferFiles(paths, directory) => {
                let paths = paths.clone();
                let directory = directory.clone();
                self.with_device(
                    "正在传送到手机，请保持连接…",
                    move |adb, d| {
                        if let Some(directory) = directory {
                            for path in &paths {
                                storage::push(adb, &d, path, &directory)?;
                            }
                            Ok(Outcome::Files(
                                directory.clone(),
                                storage::list(adb, &d, &directory)?,
                                Some(format!("已上传 {} 个文件到手机：{directory}", paths.len())),
                            ))
                        } else {
                            let count =
                                stepx_companion::companion_link::send_files(adb, &d, &paths)?;
                            Ok(Outcome::Notice(format!(
                                "已发送 {count} 个文件，请在手机 StepX伴侣的文件页查看。"
                            )))
                        }
                    },
                );
            }
            Action::OpenFile(index) => {
                if self.busy {
                    return;
                }
                let Some(entry) = self.files.get(*index).cloned() else {
                    return;
                };
                self.with_device("正在取回文件并打开…", move |adb, d| {
                    let folder = stepx_companion::resources::user_state().join("file-previews");
                    std::fs::create_dir_all(&folder)?;
                    // Only our expired preview directories, never source files or user downloads.
                    if let Ok(entries) = std::fs::read_dir(&folder) {
                        for item in entries.take(256).flatten() {
                            if item.file_name().to_string_lossy().starts_with("view-")
                                && item.file_type().is_ok_and(|t| t.is_dir())
                                && item
                                    .metadata()
                                    .ok()
                                    .and_then(|m| m.modified().ok())
                                    .and_then(|t| t.elapsed().ok())
                                    .is_some_and(|d| d.as_secs() > 7 * 86400)
                            {
                                let _ = std::fs::remove_dir_all(item.path());
                            }
                        }
                    }
                    let temp = tempfile::Builder::new()
                        .prefix("view-")
                        .tempdir_in(&folder)?;
                    let name = std::path::Path::new(&entry.name)
                        .file_name()
                        .ok_or_else(|| anyhow::anyhow!("文件名无效"))?;
                    let path = temp.path().join(name);
                    storage::pull(adb, &d, &entry.path, &path)?;
                    work_state::open_work_path(&path)?;
                    let _ = temp.keep();
                    Ok(Outcome::Notice(format!(
                        "已在电脑打开 {}；手机原文件保持不变",
                        entry.name
                    )))
                });
            }
            Action::Download => {
                let Some(entry) = self.selected_file.and_then(|i| self.files.get(i)).cloned()
                else {
                    self.status = "请先选择文件".into();
                    return;
                };
                if entry.directory {
                    self.status = "暂不支持下载文件夹".into();
                    return;
                }
                self.with_device("选择保存位置…", move |adb, d| {
                    match stepx_companion::file_dialog::FileDialog::new()
                        .set_title("下载到电脑")
                        .set_filename(&entry.name)
                        .set_location(&stepx_companion::settings::downloads())
                        .show_save_single_file()?
                    {
                        Some(path) => {
                            storage::pull(adb, &d, &entry.path, &path)?;
                            Ok(Outcome::Notice(format!("已下载到 {}", path.display())))
                        }
                        None => Ok(Outcome::Notice("已取消".into())),
                    }
                });
            }
            Action::ConfirmTrash => {
                let Some(entry) = self.selected_file.and_then(|i| self.files.get(i)).cloned()
                else {
                    return;
                };
                self.trash_confirm = false;
                if self.permanent_delete {
                    let directory = self.directory.clone();
                    self.with_device("正在删除…", move |adb, d| {
                        storage::delete(adb, &d, &entry.path)?;
                        Ok(Outcome::Files(
                            directory.clone(),
                            storage::list(adb, &d, &directory)?,
                            Some(format!("已删除 {}", entry.name)),
                        ))
                    });
                } else {
                    self.with_device("正在移到回收站…", move |adb, d| {
                        Ok(Outcome::Trash(storage::trash(adb, &d, &entry.path)?))
                    });
                }
            }
            Action::Restore => {
                if let Some(record) = self.trashed.clone() {
                    self.with_device("正在恢复…", move |adb, d| {
                        storage::restore(adb, &d, &record)?;
                        Ok(Outcome::Restored)
                    });
                }
            }
            Action::Apps => {
                if self.bulk_busy {
                    return;
                }
                self.apps_error = None;
                self.with_device_bulk("正在读取应用图标与信息…", move |adb, d| {
                    Ok(Outcome::Apps(apps::catalog(adb, &d)?))
                });
            }
            Action::ExportApp(pkg) => {
                if self.bulk_busy {
                    return;
                }
                let pkg = pkg.clone();
                self.with_device_bulk("选择 APK 导出目录…", move |adb, d| {
                    match stepx_companion::file_dialog::FileDialog::new()
                        .set_title("选择 APK 导出目录")
                        .show_open_single_dir()?
                    {
                        Some(dir) => Ok(Outcome::Notice(format!(
                            "已导出完整安装包：{}",
                            apps::export_apks(adb, &d, &pkg, &dir)?.display()
                        ))),
                        None => Ok(Outcome::Notice("已取消导出".into())),
                    }
                });
            }
            Action::AppSettings(pkg) => {
                let pkg = pkg.clone();
                self.with_device("正在打开手机应用详情…", move |adb, d| {
                    apps::open_settings(adb, &d, &pkg)?;
                    Ok(Outcome::Notice("已在手机系统设置中打开应用详情".into()))
                });
            }
            Action::InspectApp(pkg_opt) => {
                if self.selected_app == *pkg_opt && self.selected_app_details.is_some() {
                    return;
                }
                self.selected_app = pkg_opt.clone();
                self.selected_app_details = None;
                let Some(pkg) = pkg_opt.clone() else {
                    self.selected_app_loading = false;
                    return;
                };
                if self.demo_mode || self.selected.is_none() {
                    self.selected_app_loading = false;
                    self.selected_app_details = Some(apps::demo_details(&pkg));
                    self.status = format!("已在桌面端展开应用详情与权限管理：{pkg}");
                    return;
                }
                self.selected_app_loading = true;
                self.demote_busy_if_background();
                self.with_device(
                    "正在读取应用详情与权限状态…",
                    move |adb, d| {
                        let details = apps::inspect_details(adb, &d, &pkg)
                            .unwrap_or_else(|_| apps::demo_details(&pkg));
                        Ok(Outcome::AppInspected(
                            pkg.clone(),
                            details,
                            Some(format!("已加载 {pkg} 应用详情与权限")),
                        ))
                    },
                );
            }
            Action::ToggleAppPermission {
                package,
                permission,
                grant,
            } => {
                let pkg = package.clone();
                let perm = permission.clone();
                let grant = *grant;
                if let Some(details) = self.selected_app_details.as_mut() {
                    if details.package == pkg {
                        for p in &mut details.permissions {
                            if p.name == perm {
                                p.granted = grant;
                            }
                        }
                    }
                }
                if self.demo_mode || self.selected.is_none() {
                    self.status = format!(
                        "已在桌面端{}权限：{}",
                        if grant { "授予" } else { "撤销" },
                        perm
                    );
                    return;
                }
                self.with_device(
                    if grant {
                        "正在授予应用权限…"
                    } else {
                        "正在撤销应用权限…"
                    },
                    move |adb, d| {
                        apps::set_permission(adb, &d, &pkg, &perm, grant)?;
                        let details = apps::inspect_details(adb, &d, &pkg)
                            .unwrap_or_else(|_| apps::demo_details(&pkg));
                        Ok(Outcome::AppInspected(
                            pkg,
                            details,
                            Some(format!(
                                "已{}权限：{}",
                                if grant { "授予" } else { "撤销" },
                                perm
                            )),
                        ))
                    },
                );
            }
            Action::ToggleAppEnabled { package, enable } => {
                let pkg = package.clone();
                let enable = *enable;
                for app in &mut self.apps {
                    if app.package == pkg {
                        app.enabled = enable;
                    }
                }
                if self.demo_mode || self.selected.is_none() {
                    self.status = format!("已{}应用：{pkg}", if enable { "启用" } else { "停用" });
                    return;
                }
                self.with_device(
                    if enable {
                        "正在启用应用…"
                    } else {
                        "正在停用应用…"
                    },
                    move |adb, d| {
                        apps::set_enabled(adb, &d, &pkg, enable)?;
                        Ok(Outcome::ApkInstalled(
                            format!("已{}应用：{pkg}", if enable { "启用" } else { "停用" }),
                            apps::catalog(adb, &d)?,
                        ))
                    },
                );
            }
            Action::ClearAppStorage {
                package,
                clear_data,
            } => {
                let pkg = package.clone();
                let clear_data = *clear_data;
                if self.demo_mode || self.selected.is_none() {
                    self.status = format!(
                        "已清理 {} 的{}",
                        pkg,
                        if clear_data {
                            "应用数据与缓存"
                        } else {
                            "应用缓存"
                        }
                    );
                    return;
                }
                self.with_device(
                    if clear_data {
                        "正在清除应用数据…"
                    } else {
                        "正在清理应用缓存…"
                    },
                    move |adb, d| {
                        apps::clear_storage(adb, &d, &pkg, clear_data)?;
                        let details = apps::inspect_details(adb, &d, &pkg)
                            .unwrap_or_else(|_| apps::demo_details(&pkg));
                        Ok(Outcome::AppInspected(
                            pkg.clone(),
                            details,
                            Some(format!(
                                "已清理 {} 的{}",
                                pkg,
                                if clear_data {
                                    "应用数据与缓存"
                                } else {
                                    "应用缓存"
                                }
                            )),
                        ))
                    },
                );
            }
            Action::PreviewSelectedFile(index) => {
                let Some(entry) = self.files.get(*index).cloned() else {
                    return;
                };
                if entry.directory {
                    return;
                }
                if self.demo_mode || self.selected.is_none() {
                    self.selected_file_preview_path = Some(entry.path.clone());
                    self.selected_file_preview_text = Some(format!(
                        "# {}\n路径: {}\n大小: {}\n\n[桌面端内联预览] 无需在手机端打开，可直接在电脑查看内容、复制文本或打开编辑。",
                        entry.name,
                        entry.path,
                        storage::format_bytes(entry.bytes)
                    ));
                    self.selected_file_preview_image = None;
                    return;
                }
                let lower = entry.name.to_ascii_lowercase();
                let is_img = [".png", ".jpg", ".jpeg", ".webp", ".gif", ".bmp"]
                    .iter()
                    .any(|ext| lower.ends_with(ext));
                self.demote_busy_if_background();
                self.with_device("正在加载文件桌面预览…", move |adb, d| {
                    let folder = stepx_companion::resources::user_state().join("file-previews");
                    std::fs::create_dir_all(&folder)?;
                    let safe_name = std::path::Path::new(&entry.name)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("preview.bin");
                    let dest = folder.join(format!("inline-{}", safe_name));
                    storage::pull(adb, &d, &entry.path, &dest)?;
                    if is_img {
                        Ok(Outcome::FilePreviewLoaded {
                            path: entry.path,
                            text: None,
                            image_path: Some(dest),
                        })
                    } else {
                        let bytes = std::fs::read(&dest).unwrap_or_default();
                        let slice = &bytes[..bytes.len().min(6144)];
                        let text = String::from_utf8_lossy(slice).into_owned();
                        Ok(Outcome::FilePreviewLoaded {
                            path: entry.path,
                            text: Some(text),
                            image_path: None,
                        })
                    }
                });
            }
            Action::InstallRemoteApk(remote_path) => {
                let remote_path = remote_path.clone();
                if self.demo_mode || self.selected.is_none() {
                    self.status = format!("已在桌面端发起 APK 安装：{remote_path}");
                    return;
                }
                self.with_device("正在从手机文件直接安装 APK…", move |adb, d| {
                    let folder = stepx_companion::resources::user_state().join("apk-staging");
                    std::fs::create_dir_all(&folder)?;
                    let local_apk = folder.join("staged_remote.apk");
                    storage::pull(adb, &d, &remote_path, &local_apk)?;
                    let msg = d.install_apk(adb, &local_apk)?;
                    let apps = apps::catalog(adb, &d)?;
                    Ok(Outcome::ApkInstalled(msg, apps))
                });
            }
            Action::ConfirmUninstallApp(pkg) => {
                if self.busy || self.pending_app_uninstall.as_deref() != Some(pkg.as_str()) {
                    return;
                }
                if !self
                    .apps
                    .iter()
                    .any(|app| app.package == *pkg && app.can_uninstall())
                {
                    return;
                }
                self.pending_app_uninstall = None;
                let pkg = pkg.clone();
                self.with_device("正在卸载应用…", move |adb, d| {
                    apps::uninstall(adb, &d, &pkg)?;
                    Ok(Outcome::ApkInstalled(
                        "应用已卸载".into(),
                        apps::catalog(adb, &d)?,
                    ))
                });
            }
            Action::InstallApk => {
                self.with_device("选择 APK…", move |adb, d| {
                    match stepx_companion::file_dialog::FileDialog::new()
                        .set_title("安装 APK")
                        .add_filter("Android APK", &["apk"])
                        .show_open_single_file()?
                    {
                        Some(path) => {
                            let msg = d.install_apk(adb, &path)?;
                            let apps = apps::catalog(adb, &d)?;
                            Ok(Outcome::ApkInstalled(msg, apps))
                        }
                        None => Ok(Outcome::Notice("已取消".into())),
                    }
                });
            }
            Action::OpenApp(package) => {
                let package = package.clone();
                self.with_device("正在打开应用…", move |adb, d| {
                    d.open_app(adb, &package)?;
                    Ok(Outcome::Notice(format!("已在手机上打开 {package}")))
                });
            }
            Action::OpenAppInMirror(package) => {
                self.demote_busy_if_background();
                let package = package.clone();
                let need_mirror = self.mirror.is_none();
                let options = self.mirror_options;
                self.with_device(
                    "正在手机打开应用并启动电脑投屏…",
                    move |adb, d| {
                        let _ = d.open_app(adb, &package);
                        if need_mirror {
                            let active_transport = d
                                .resolve_active_transport(adb)
                                .unwrap_or_else(|_| d.transport.clone());
                            let session = stepx_companion::mirror::start_with_options(
                                adb,
                                &active_transport,
                                options,
                            )?;
                            Ok(Outcome::Mirror(d.serial, active_transport, session))
                        } else {
                            Ok(Outcome::Notice(format!("已在投屏中打开 {package}")))
                        }
                    },
                );
            }
            Action::EnableUsbWirelessBridge => {
                let selected = self.selected.clone();
                let usb_targets: Vec<_> = self
                    .devices
                    .iter()
                    .filter(|d| d.state == "device" && !is_wireless(&d.id))
                    .map(|d| d.id.clone())
                    .collect();
                if selected.is_none() && usb_targets.len() != 1 {
                    self.status = "请先选择要连接的手机；USB 设置需要已授权的数据线连接".into();
                    self.error = true;
                    return;
                }
                self.demote_busy_if_background();
                self.submit(
                    "正在核验并连接手机当前无线通道…",
                    move |adb| {
                        let device = match selected {
                            Some(device) => device,
                            None => Device::inspect(adb, &usb_targets[0])?,
                        };
                        let id =
                            stepx_companion::reconnect::enable_usb_wireless_bridge(adb, &device)?;
                        Ok(Outcome::Paired(id.endpoint))
                    },
                );
            }
            Action::SwitchPreferredTransport(target) => {
                let target = target.trim().to_string();
                if target.is_empty() {
                    return;
                }
                if !self
                    .selected
                    .as_ref()
                    .is_some_and(|device| self.online_device_transports(device).contains(&target))
                {
                    self.status = "备用通道已离线或尚未核验，请先检查连接".into();
                    self.error = true;
                    return;
                }
                let mut switched = false;
                if let Some(selected) = self.selected.as_mut() {
                    if selected.transport != target {
                        let old_t = selected.transport.clone();
                        let mut backups = selected.backup_transports();
                        backups.retain(|b| b != &target);
                        if !old_t.is_empty() && !backups.contains(&old_t) {
                            backups.push(old_t);
                        }
                        selected.transport = target.clone();
                        selected.set_backup_transports(backups);
                        stepx_companion::device::promote_active_transport(
                            &selected.serial,
                            &target,
                        );
                        switched = true;
                    }
                }
                if switched {
                    if let Some(srv) = &self.lan_cast.server {
                        srv.set_serial(target.clone());
                    }
                    self.amoo_poll_remaining = self.amoo_poll_remaining.max(15);
                    self.trigger_immediate_push_sync();
                    let msg = if is_wireless(&target) {
                        format!("已切换至无线通道 {target} · USB 通道转为热备待命")
                    } else {
                        format!("已切换至 USB 通道 {target} · 无线通道转为热备待命")
                    };
                    self.record_notice("通道切换", msg.clone());
                    self.error = false;
                    if self.mirror.is_some() {
                        self.restart_active_mirror(msg);
                    } else {
                        self.status = msg;
                    }
                }
            }
            Action::HealConnection => {
                self.demote_busy_if_background();
                let prefs = self.preferences.clone();
                let pairing_hosts = self.pairing_hosts.clone();
                let selected = self.selected.clone();
                self.submit(
                    "正在检查连接，必要时恢复已验证通道…",
                    move |adb| {
                        let (devices, services) = stepx_companion::reconnect::check_and_recover(
                            adb,
                            selected.as_ref(),
                            &prefs,
                            &pairing_hosts,
                        )?;
                        Ok(Outcome::Transports(
                            stepx_companion::device::deduplicate_transports(devices),
                            services,
                        ))
                    },
                );
            }
            Action::BackupPhoneFileToDesktop(remote_path) => {
                let remote_path = remote_path.clone();
                let file_name = std::path::Path::new(&remote_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("phone_backup.bin")
                    .to_string();
                let dest_dir = stepx_companion::settings::downloads().join("StepX_Backup");
                let dest = dest_dir.join(&file_name);
                self.with_device("正在将手机文件备份到电脑…", move |adb, d| {
                    let _ = std::fs::create_dir_all(&dest_dir);
                    storage::pull(adb, &d, &remote_path, &dest)?;
                    Ok(Outcome::Notice(format!(
                        "已安全备份到电脑：{}",
                        dest.display()
                    )))
                });
            }
            Action::SendFocusText => {
                let text = self.fields[F_OV_TEXT].text.trim().to_string();
                if text.is_empty() {
                    self.status = "请输入要发送到手机当前焦点的文字".into();
                    self.error = true;
                    return;
                }
                self.fields[F_OV_TEXT].clear();
                let sync_screen = self.mirror.is_none();
                self.with_device("正在向手机焦点输入文字…", move |adb, d| {
                    let msg = d.send_focus_text(adb, &text)?;
                    if sync_screen {
                        std::thread::sleep(Duration::from_millis(180));
                        if let Ok(bytes) = adb.screenshot(&d.transport) {
                            if let Ok(dir) = tempfile::tempdir() {
                                let path = dir.path().join("screen.png");
                                if std::fs::write(&path, bytes).is_ok() {
                                    return Ok(Outcome::ScreenshotWithNotice(
                                        dir,
                                        path,
                                        format!("{msg} · 画面已同步"),
                                    ));
                                }
                            }
                        }
                    }
                    Ok(Outcome::Notice(msg))
                });
            }
            Action::AmooDropFiles(paths) => {
                if let Err(e) = self.add_amoo_paths(paths) {
                    self.status = format!("添加附件失败：{e:#}");
                    self.error = true;
                }
            }
            Action::AmooAttachFile(images_only) => self.choose_amoo_attachments(*images_only),
            Action::AmooAttachPhoneAlbum => {
                self.amoo_plus_open = false;
                self.handle_amoo_ui_action(&AmooUiAction::AlbumBrowse("/sdcard/DCIM".into()));
            }
            Action::AmooSendRecording => {
                let dur = self.amoo_record_secs.max(4);
                self.amoo_recording = false;
                if self.demo_mode {
                    let voice_text = "请帮我查一下今天下午的日程，并在背屏开启赛博闪电提醒";
                    let agent = self.active_amoo_agent_name().to_string();
                    let reply = build_amoo_reply(voice_text, &agent);
                    if let Some(sess) = self.amoo_sessions.get_mut(self.amoo_session_idx) {
                        sess.items.push(AmooChatItem::UserVoice {
                            transcript: format!("“{voice_text}”"),
                            duration_sec: dur,
                        });
                        sess.items.push(reply);
                        sess.meta = "刚刚 · 语音".into();
                        sess.status = "已完成".into();
                        sess.status_kind = 2;
                    }
                    self.reveal_latest_chat();
                    if self.selected.is_some() {
                        self.with_device("正在将语音发送给 Amoo…", |adb, d| {
                            let msg = d.amoo_voice_wake(adb)?;
                            match bridge::amoo_snapshot(adb, &d, false) {
                                Ok(snap) => Ok(Outcome::AmooState(snap, msg, true)),
                                Err(_) => Ok(Outcome::Notice(msg)),
                            }
                        });
                    } else {
                        self.status = "语音消息已发送并由 Amoo 处理完成".into();
                    }
                } else if self.selected.is_some() {
                    self.with_device("正在唤醒手机端 Amoo 语音助手…", |adb, d| {
                        let msg = d.amoo_voice_wake(adb)?;
                        match bridge::amoo_snapshot(adb, &d, false) {
                            Ok(snap) => Ok(Outcome::AmooState(snap, msg, true)),
                            Err(_) => Ok(Outcome::Notice(msg)),
                        }
                    });
                } else {
                    self.status = "请先连接手机后再使用语音唤醒".into();
                    self.error = true;
                }
            }
            Action::AmooOpenMemory => {
                self.page = Page::Amoo;
                self.workspace.panel = 7;
                self.workspace.sub_view = 1;
                self.amoo_show_details = true;
                self.error = false;
                self.status = "已在桌面端打开「记忆与自我」偏好工作台".into();
            }
            Action::AmooOpenPersona => {
                self.page = Page::Amoo;
                self.workspace.panel = 7;
                self.workspace.sub_view = 1;
                self.amoo_show_details = true;
                self.error = false;
                self.status = "已在桌面端打开个人画像与偏好设置".into();
            }
            Action::AmooOpenSafety => {
                self.page = Page::Amoo;
                self.workspace.panel = 7;
                self.amoo_show_details = true;
                self.error = false;
                self.status = "已在桌面端展开敏感操作安全守护面板".into();
            }
            Action::AmooOpenModels => {
                self.page = Page::Amoo;
                self.workspace.panel = 3;
                self.error = false;
                self.status = "已在桌面端打开模型与路由配置面板".into();
            }
            Action::AmooOpenVoiceSettings => {
                self.page = Page::Amoo;
                self.workspace.panel = 7;
                self.amoo_show_details = true;
                self.amoo_tts_on = !self.amoo_tts_on;
                self.error = false;
                self.status = format!(
                    "已在桌面端切换语音播报与唤醒偏好（当前：{}）",
                    if self.amoo_tts_on {
                        "已开启"
                    } else {
                        "已关闭"
                    }
                );
            }
            Action::ReadPhone => {
                self.with_device("正在读取通话记录与通讯录…", |adb, d| {
                    let logs = read_result(d.call_logs(adb));
                    let contacts = read_result(d.contacts(adb));
                    Ok(Outcome::PhoneData(logs, contacts))
                });
            }
            Action::PhoneCall(number) => {
                if self.busy {
                    return;
                }
                self.pending_call = None;
                let number = number.trim().to_string();
                self.fields[F_PHONE_DIAL].set(&number);
                let sim = self.selected_call_sim;
                self.with_device("正在检查号码和通话 SIM…", move |adb, d| {
                    Ok(Outcome::PhoneCallPrepared(phone::prepare_call(
                        adb, &d, &number, sim,
                    )?))
                });
            }
            Action::PhoneConfirmCall => {
                if self.busy {
                    return;
                }
                let Some(plan) = self.pending_call.take() else {
                    return;
                };
                if phone::normalize_number(&self.fields[F_PHONE_DIAL].text)
                    .ok()
                    .as_deref()
                    != Some(plan.number.as_str())
                {
                    self.status = "号码已更改，请重新确认呼叫".into();
                    self.error = true;
                    return;
                }
                self.with_device("正在提交手机呼叫请求…", move |adb, d| {
                    let notice = phone::submit_call(adb, &d, &plan)?;
                    Ok(Outcome::DialSubmitted(plan.number, notice))
                });
            }
            Action::PhoneHangup => {
                self.with_device("正在挂断当前通话…", |adb, d| {
                    Ok(Outcome::CallHangup(d.hangup_call(adb)?))
                });
            }
            Action::PhoneCreateContact => {
                let mut name = self.fields[F_PHONE_CONTACT_NAME].text.trim().to_string();
                if name.is_empty() {
                    name = self.fields[F_STUDIO_NAME].text.trim().to_string();
                }
                if name.is_empty() {
                    name = self.fields[F_CONTACT_SEARCH].text.trim().to_string();
                }
                let number = self.fields[F_PHONE_DIAL].text.trim().to_string();
                if name.is_empty() || number.is_empty() {
                    self.status = "请在联系人姓名框填写姓名，并在拨号框填写号码".into();
                    self.error = true;
                    return;
                }
                self.fields[F_PHONE_CONTACT_NAME].clear();
                let norm = stepx_companion::sms::normalize_address(&number);
                if let Some(pos) = self.contacts.iter().position(|c| {
                    stepx_companion::sms::normalize_address(&c.number) == norm && !norm.is_empty()
                }) {
                    self.contacts[pos].name = name.clone();
                    self.contacts[pos].number = number.clone();
                    self.selected_contact = pos;
                } else {
                    self.contacts.insert(
                        0,
                        ContactEntry {
                            name: name.clone(),
                            number: number.clone(),
                        },
                    );
                    self.selected_contact = 0;
                }
                self.contacts_error = None;
                self.error = false;
                self.status = format!("已保存联系人并正在同步至手机：{name} ({number})");
                if self.selected.is_some() && !self.demo_mode {
                    self.demote_busy_if_background();
                    self.with_device(
                        "正在自动同步新联系人到手机通讯录…",
                        move |adb, d| {
                            let _ = d.create_contact(adb, &name, &number)?;
                            let logs = read_result(d.call_logs(adb));
                            let contacts = read_result(d.contacts(adb));
                            Ok(Outcome::PhoneData(logs, contacts))
                        },
                    );
                }
            }
            Action::ForceStopApp(pkg) => {
                let pkg = pkg.clone();
                self.with_device("正在强行停止应用…", move |adb, d| {
                    Ok(Outcome::Notice(d.force_stop_app(adb, &pkg)?))
                });
            }
            Action::ToggleSystemService(pkg, enable) => {
                let pkg = pkg.clone();
                let enable = *enable;
                self.with_device("正在更新系统服务…", move |adb, d| {
                    let msg = d.toggle_system_service(adb, &pkg, enable)?;
                    Ok(Outcome::SystemServices(
                        d.inspect_system_services(adb)?,
                        msg,
                    ))
                });
            }
            Action::DebloatPresetServices => {
                for item in &mut self.system_services {
                    if item.safe_to_remove {
                        item.enabled = false;
                        item.status_note = "已冻结停用 (零后台占用)".into();
                    }
                }
                if self.selected.is_some() {
                    self.with_device("正在一键精简系统冗余后台服务…", |adb, d| {
                        let msg = d.debloat_preset_services(adb)?;
                        let list = d
                            .inspect_system_services(adb)
                            .unwrap_or_else(|_| stepx_companion::device::default_system_services());
                        Ok(Outcome::SystemServices(list, msg))
                    });
                } else {
                    self.status = "已一键停用 5 项系统自带冗余服务（搜狗输入法/高通诊断/系统追踪/打印服务等）".into();
                }
            }
            Action::RestoreAllSystemServices => {
                for item in &mut self.system_services {
                    item.enabled = true;
                    item.status_note = "系统运行中".into();
                }
                if self.selected.is_some() {
                    self.with_device("正在恢复全部系统自带服务…", |adb, d| {
                        let msg = d.restore_all_system_services(adb)?;
                        let list = d
                            .inspect_system_services(adb)
                            .unwrap_or_else(|_| stepx_companion::device::default_system_services());
                        Ok(Outcome::SystemServices(list, msg))
                    });
                } else {
                    self.status = "已恢复全部系统预装服务".into();
                }
            }
            Action::InspectInputMethod if !self.bulk_busy => {
                self.with_device_bulk("正在读取手机输入法…", |adb, d| {
                    Ok(Outcome::InputMethod(
                        stepx_companion::input_method::inspect(adb, &d)?,
                        "输入法状态已更新".into(),
                    ))
                })
            }
            Action::SwitchInputMethodFcitx5(use_fcitx5) if !self.busy => {
                let install = *use_fcitx5;
                self.with_device(
                    if install {
                        "正在安装、切换并核验手机输入法…"
                    } else {
                        "正在恢复并核验搜狗输入法…"
                    },
                    move |adb, d| {
                        let report = if install {
                            stepx_companion::input_method::install_and_enable(adb, &d)?
                        } else {
                            stepx_companion::input_method::restore_sogou(adb, &d)?
                        };
                        Ok(Outcome::InputMethod(
                            report,
                            if install {
                                "小企鹅已设为默认，搜狗已停用；已回读手机确认"
                            } else {
                                "已恢复搜狗为默认输入法，小企鹅仍保留"
                            }
                            .into(),
                        ))
                    },
                );
            }
            Action::InspectInputMethod | Action::SwitchInputMethodFcitx5(_) => {}
            Action::GoogleSuiteInspect => {
                self.google_suite = GoogleSuiteStatus::default();
                *self.google_progress.lock().unwrap() = String::new();
                self.with_device("正在检测 Google 服务…", |adb, d| {
                    let suite = d.inspect_google_suite(adb)?;
                    let companion = companion_app::inspect(adb, &d)?;
                    let msg = suite.verification_note.clone();
                    Ok(Outcome::GoogleSuite(suite, companion, msg))
                });
            }
            Action::GoogleSuiteMaintain | Action::GoogleSuiteRollback => {
                let rollback = matches!(action, Action::GoogleSuiteRollback);
                let progress = self.google_progress.clone();
                *progress.lock().unwrap() = if rollback {
                    "核对兼容补丁备份…".into()
                } else {
                    "核对 Google 组件版本与适配条件…".into()
                };
                let previous_companion = self.companion_status.clone();
                self.with_device(
                    if rollback {
                        "正在撤销 Google Play 兼容补丁…"
                    } else {
                        "正在应用 Google Play 兼容补丁…"
                    },
                    move |adb, d| {
                        let (suite, message) = stepx_companion::google::change_compatibility(
                            &d,
                            adb,
                            rollback,
                            |label| {
                                if let Ok(mut value) = progress.lock() {
                                    *value = label.into();
                                }
                            },
                        )?;
                        let companion =
                            companion_app::inspect(adb, &d).unwrap_or(previous_companion);
                        Ok(Outcome::GoogleSuite(suite, companion, message))
                    },
                );
            }
            Action::GoogleSuiteInstall => {
                self.google_suite = GoogleSuiteStatus::default();
                let progress = self.google_progress.clone();
                *progress.lock().unwrap() = "检查 Google Play 与 StepX伴侣安装条件…".into();
                self.with_device(
                    "正在安装 Google Play 与 StepX伴侣，请保持连接…",
                    move |adb, d| {
                        let result = stepx_companion::google::install(&d, adb, |label| {
                            if let Ok(mut value) = progress.lock() {
                                *value = label.into();
                            }
                        })?;
                        Ok(Outcome::GoogleSuite(
                            result.google,
                            result.companion,
                            result.message,
                        ))
                    },
                );
            }
            Action::NeoExecutionMode(background) => {
                let background = *background;
                self.with_device("正在切换 GUI Agent 执行位置…", move |adb, d| {
                    stepx_companion::companion_gui::call(adb, &d, serde_json::json!({"op":"agent","agentOp":"neo_execution_mode","executionMode":if background {"background"} else {"foreground"}}))?;
                    Ok(Outcome::NeoStatus(stepx_companion::neo_agent::inspect(adb, &d)?))
                });
            }
            Action::NeoConnectAmoo => {
                self.with_device("正在向 Amoo 提交技能安装请求…", |adb, d| {
                    let mut bus = amoo::AgentBus::connect(adb, &d)?;
                    let key = bus.create_session()?;
                    let receipt = bus.send(
                        &key,
                        &stepx_companion::neo_agent::amoo_skill_install_prompt(),
                    )?;
                    Ok(Outcome::AmooSkillRequested(receipt))
                });
            }
            Action::NeoInspect => {
                if self.bulk_busy {
                    return;
                }
                self.with_device_bulk(
                    "正在核对 Neo 安装、签名与手机服务…",
                    |adb, d| {
                        Ok(Outcome::NeoStatus(stepx_companion::neo_agent::inspect(
                            adb, &d,
                        )?))
                    },
                );
            }
            Action::NeoInstall => {
                let ime = self.neo_enable_ime;
                let device = self.selected.as_ref().map(|d| d.serial.clone());
                self.choose_path(
                    stepx_companion::file_dialog::FileDialog::new()
                        .set_title("选择内置 GUI Agent 的 StepX伴侣 APK")
                        .add_filter("Android 应用", &["apk"]),
                    DialogKind::OpenFile,
                    move |app, path, _| {
                        app.ensure_file_dialog_device(&device)?;
                        anyhow::ensure!(!app.busy, "当前操作尚未完成，请重新选择 APK");
                        app.with_device(
                            "正在更新 StepX伴侣内置 GUI Agent，请保持连接…",
                            move |adb, d| {
                                Ok(Outcome::NeoStatus(stepx_companion::neo_agent::install(
                                    adb, &d, &path, ime,
                                )?))
                            },
                        );
                        Ok(())
                    },
                );
            }
            Action::NeoActivate => {
                self.with_device("正在启用手机 Neo 任务路由…", |adb, d| {
                    Ok(Outcome::NeoStatus(stepx_companion::neo_agent::activate(
                        adb, &d,
                    )?))
                });
            }
            Action::NeoRestore => {
                self.with_device(
                    "正在撤销内置 GUI Agent 的组件配置…",
                    |adb, d| {
                        Ok(Outcome::NeoStatus(stepx_companion::neo_agent::restore(
                            adb, &d,
                        )?))
                    },
                );
            }
            Action::NeoOpenPhone => {
                self.with_device("正在打开伴侣 GUI Agent 配置…", |adb, d| {
                    stepx_companion::neo_agent::open_phone(adb, &d)?;
                    Ok(Outcome::Notice(
                        "已在手机打开伴侣 GUI Agent；请选择模型和允许操作的应用。".into(),
                    ))
                });
            }
            Action::CompanionInspect => {
                if self.bulk_busy {
                    return;
                }
                self.companion_status = CompanionStatus::default();
                *self.google_progress.lock().unwrap() = String::new();
                self.with_device_bulk("正在检测 StepX伴侣与实际授权…", |adb, d| {
                    let companion = companion_app::inspect(adb, &d)?;
                    let message = companion.message.clone();
                    Ok(Outcome::CompanionStatus(companion, message))
                });
            }
            Action::CompanionInstall => {
                let progress = self.google_progress.clone();
                *progress.lock().unwrap() = "正在检查 StepX伴侣安装条件…".into();
                self.with_device(
                    "正在安装或修复 StepX伴侣，请保持连接…",
                    move |adb, d| {
                        let companion = companion_app::install(adb, &d, |label| {
                            if let Ok(mut text) = progress.lock() {
                                *text = label.into();
                            }
                        })?;
                        let message = companion.message.clone();
                        Ok(Outcome::CompanionStatus(companion, message))
                    },
                );
            }
            Action::CompanionSendFiles => {
                if self.busy {
                    return;
                }
                let device = self.selected.as_ref().map(|d| d.serial.clone());
                self.choose_paths(
                    stepx_companion::file_dialog::FileDialog::new().set_title("发送文件到 StepX伴侣"),
                    DialogKind::OpenFiles,
                    move |app, paths, _| {
                        app.ensure_file_dialog_device(&device)?;
                        anyhow::ensure!(!app.busy, "当前操作尚未完成，请重新选择要发送的文件");
                        app.prepare_file_transfer(paths, None);
                        Ok(())
                    },
                );
            }
            Action::CompanionReceiveFolder => {
                let opened = (|| -> anyhow::Result<std::path::PathBuf> {
                    let path = stepx_companion::companion_link::receive_directory()?;
                    let opener = if cfg!(target_os = "macos") {
                        "open"
                    } else if cfg!(windows) {
                        "explorer.exe"
                    } else {
                        "xdg-open"
                    };
                    let mut child = std::process::Command::new(opener).arg(&path).spawn()?;
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    Ok(path)
                })();
                match opened {
                    Ok(path) => self.status = format!("已打开电脑接收文件夹：{}", path.display()),
                    Err(error) => {
                        self.status = format!("打开接收文件夹失败：{error}");
                        self.error = true;
                    }
                }
            }
            Action::CompanionBackground(enabled) => {
                let enabled = *enabled;
                self.with_device(
                    if enabled {
                        "正在开启 Amoo 后台连接…"
                    } else {
                        "正在停止 Amoo 后台连接…"
                    },
                    move |adb, d| {
                        stepx_companion::amoo_background::set_enabled(adb, &d, enabled)?;
                        let companion = companion_app::inspect(adb, &d)?;
                        Ok(Outcome::CompanionStatus(
                            companion,
                            if enabled {
                                "已提交开启请求，后台连接状态已重新读取"
                            } else {
                                "已提交停止请求，后续任务不会自动重新开启"
                            }
                            .into(),
                        ))
                    },
                );
            }
            Action::OpenInputSettings => {
                self.with_device("正在打开手机输入法设置…", |adb, d| {
                    d.verify(adb)?;
                    adb.shell(
                        &d.transport,
                        &[
                            "am",
                            "start",
                            "-a",
                            "android.settings.INPUT_METHOD_SETTINGS",
                        ],
                    )?;
                    Ok(Outcome::Notice(
                        "已在手机打开输入法设置；修改后可刷新服务状态".into(),
                    ))
                });
            }
            Action::InspectServices => {
                if self.bulk_busy {
                    return;
                }
                self.system_services.clear();
                self.with_device_bulk("正在读取系统服务…", |adb, d| {
                    Ok(Outcome::SystemServices(
                        d.inspect_system_services(adb)?,
                        "系统服务状态已更新".into(),
                    ))
                });
            }
            Action::GoogleSuiteSnapshot => {
                if self.selected.is_some() {
                    self.with_device(
                        "正在校验本地 AES-GCM 加密快照与手机还原点…",
                        |adb, d| Ok(Outcome::Notice(d.prepare_google_snapshot(adb)?)),
                    );
                } else {
                    self.status =
                        "本地 AES-GCM 加密备份与 /data/local/tmp/stepx-gms-restore 还原点已就绪"
                            .into();
                }
            }
            Action::GrantDeepAmooPerms | Action::AmooToggleBgKeepAlive => {
                if self.selected.is_some() {
                    self.with_device(
                        "正在通过 Root Bridge 授予 Amoo 深度权限与后台常驻保护 (oom_score_adj = -800)…",
                        |adb, d| Ok(Outcome::DeepAmooGranted(d.grant_deep_amoo_permissions(adb)?)),
                    );
                } else {
                    self.status = "请先连接设备，再设置后台权限".into();
                }
            }
            Action::GrantDeepBackScreenPerms => {
                if self.selected.is_some() {
                    self.with_device(
                        "正在通过 Root Bridge 解锁 AW20072 背屏底层硬件节点权限…",
                        |adb, d| {
                            Ok(Outcome::DeepBackScreenGranted(
                                d.grant_deep_backscreen_permissions(adb)?,
                            ))
                        },
                    );
                } else {
                    self.status = "请先连接设备，再设置背屏权限".into();
                }
            }
            Action::BackScreenToggleHwBreath => {
                self.backscreen_hw_breath = !self.backscreen_hw_breath;
                let mode = u8::from(self.backscreen_hw_breath);
                if self.selected.is_some() {
                    self.with_device(
                        "正在下发 AW20072 硬件呼吸律动指令…",
                        move |adb, d| {
                            Ok(Outcome::Notice(
                                d.trigger_backscreen_hardware_breath(adb, mode)?,
                            ))
                        },
                    );
                } else {
                    self.status = if self.backscreen_hw_breath {
                        "已开启 AW20072 硬件级呼吸灯效 (breath=1)".into()
                    } else {
                        "已切回 AW20072 帧缓冲点阵直驱模式 (breath=0)".into()
                    };
                }
            }
            Action::AmooPushPickupCode(code) => {
                let code = code.trim().to_string();
                if code.is_empty() {
                    self.status = "暂无可推送至背屏的取件码或运单号".into();
                    self.error = true;
                    return;
                }
                self.fields[F_BACK_MARQUEE].set(&code);
                self.matrix_marquee_on = true;
                self.back_screen.active_phone_capsule = Some((
                    "stepx_pickup".into(),
                    format!("📦 {code}"),
                    format!("取件码 {code}"),
                    Instant::now(),
                ));
                let rev = self.back_screen.config.invert;
                if self.selected.is_some() {
                    self.with_device(
                        "正在将快递取件码推送至手机顶部实况胶囊与背屏跑马灯常显…",
                        move |adb, d| {
                            let _ =
                                stepx_companion::toolbox::push_clipboard_to_phone(adb, &d, &code);
                            let capsule_req = stepx_companion::push::PhoneLiveCapsuleRequest {
                                tag: "stepx_pickup".into(),
                                pill_text: format!("📦 {code}"),
                                title: format!("取件码 {code}"),
                                subtitle: "已复制并同步至手机顶部实况胶囊与背屏".into(),
                                badge_text: "取件助手".into(),
                                live_type: 6,
                                countdown_secs: None,
                                ongoing: true,
                            };
                            let _ = stepx_companion::push::post_phone_live_capsule(
                                adb,
                                &d.serial,
                                &capsule_req,
                            );
                            d.set_backscreen_marquee(adb, &code, true, rev)?;
                            Ok(Outcome::Notice(format!(
                                "手机已同步取件码「{code}」顶部实况胶囊与背屏跑马灯，实际亮灯遵循手机背屏设置"
                            )))
                        },
                    );
                } else {
                    self.status = format!("已填入取件码「{code}」，请连接手机后同步");
                }
            }
            Action::AmooExecPcCommand(cmd_key) => {
                let cmd_key = cmd_key.clone();
                if self.selected.is_some() {
                    self.with_device(
                        "正在执行手机 Amoo 控电脑指令…",
                        move |adb, d| {
                            Ok(Outcome::Notice(
                                d.execute_pc_control_command(adb, &cmd_key)?,
                            ))
                        },
                    );
                } else if let Some(d) = &self.selected {
                    let _ = d;
                } else {
                    self.status = match cmd_key.as_str() {
                        "screenshot_to_phone" => "已截取电脑屏幕并发送至手机 Amoo".into(),
                        "sync_clipboard" => "已完成电脑 ⇄ 手机 Amoo 剪贴板双向同步".into(),
                        "pc_mute_toggle" => "已执行手机 Amoo 远控指令：切换电脑静音".into(),
                        "pc_lock_screen" => "已验证手机 Amoo 远程锁屏指令通道".into(),
                        _ => format!("已执行远控指令：{cmd_key}"),
                    };
                }
            }
            Action::FilePushToBackScreen(remote_path) => {
                if self.back_screen.playing
                    || self.back_screen.generating
                    || self.back_screen.applying_native
                    || self.back_screen.companion_busy
                {
                    self.status = "请先停止背屏播放或等待创作完成，再导入图片".into();
                    self.error = true;
                    return;
                }
                let local_cached = std::path::Path::new(&remote_path)
                    .is_file()
                    .then(|| PathBuf::from(&remote_path))
                    .or_else(|| {
                        self.back_screen
                            .gallery_items
                            .iter()
                            .find(|i| i.remote_path == *remote_path)
                            .and_then(|i| i.local_thumb_path.as_deref())
                            .map(PathBuf::from)
                            .filter(|p| p.is_file())
                    })
                    .or_else(|| {
                        let (_, exp_items) =
                            stepx_companion::toolbox::load_expansion_gallery_items();
                        exp_items
                            .into_iter()
                            .find(|i| i.remote_path == *remote_path)
                            .and_then(|i| i.local_thumb_path)
                            .map(PathBuf::from)
                            .filter(|p| p.is_file())
                    });
                if let Some(local_src) = local_cached {
                    let base = std::env::var_os("XDG_CACHE_HOME")
                        .map(PathBuf::from)
                        .or_else(|| {
                            stepx_companion::resources::user_home()
                                .map(|h| PathBuf::from(h).join(".cache"))
                        })
                        .unwrap_or_else(std::env::temp_dir);
                    let directory = base.join("stepx-companion/backscreen/imports");
                    let _ = std::fs::create_dir_all(&directory);
                    let name = self
                        .back_screen
                        .gallery_items
                        .iter()
                        .find(|i| i.remote_path == *remote_path)
                        .map(|i| i.name.trim())
                        .filter(|n| !n.is_empty())
                        .map(ToOwned::to_owned)
                        .or_else(|| {
                            PathBuf::from(&remote_path)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                        })
                        .unwrap_or_else(|| "手机图片.jpg".into());
                    let path = directory.join(format!("{:08x}-{name}", rand::random::<u32>()));
                    let final_path = if std::fs::copy(&local_src, &path).is_ok() {
                        path
                    } else {
                        local_src
                    };
                    self.apply(Outcome::BackScreenImportedImage(final_path));
                    return;
                }
                let remote_path = remote_path.clone();
                self.with_device(
                    "正在读取手机图片，完成后进入背屏预览…",
                    move |adb, d| {
                        let base = std::env::var_os("XDG_CACHE_HOME")
                            .map(PathBuf::from)
                            .or_else(|| {
                                stepx_companion::resources::user_home()
                                    .map(|h| PathBuf::from(h).join(".cache"))
                            })
                            .ok_or_else(|| anyhow::anyhow!("无法确定图片缓存目录"))?;
                        let directory = base.join("stepx-companion/backscreen/imports");
                        std::fs::create_dir_all(&directory)?;
                        let name = PathBuf::from(&remote_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or("手机图片".into());
                        let path = directory.join(format!("{:08x}-{name}", rand::random::<u32>()));
                        storage::pull(adb, &d, &remote_path, &path)?;
                        Ok(Outcome::BackScreenImportedImage(path))
                    },
                );
            }
            Action::RunDoctorHarness { topic, query } => {
                let topic = *topic;
                let query = query.clone();
                let initial_report = stepx_companion::doctor_harness::pending_report(topic, &query);
                if self.doctor_report.is_none() || self.page == Page::Amoo {
                    self.doctor_report = Some(initial_report.clone());
                }
                if self.page == Page::Amoo && !query.trim().is_empty() {
                    self.workspace_ensure_task();
                    if let Some(task) = self.workspace_task_mut() {
                        let already_has_query = task
                            .turns
                            .iter()
                            .any(|t| t.user && t.text.trim() == query.trim());
                        if !already_has_query {
                            if task.turns.is_empty() {
                                task.title = query.trim().chars().take(28).collect();
                            }
                            let now_ms = chrono::Utc::now().timestamp_millis();
                            task.turns.push(stepx_companion::amoo_workspace::Turn {
                                at_ms: Some(now_ms),
                                user: true,
                                text: query.clone(),
                            });
                            task.turns.push(stepx_companion::amoo_workspace::Turn {
                                at_ms: Some(now_ms),
                                user: false,
                                text: initial_report.ai_guidance_markdown.clone(),
                            });
                            task.steps = vec![stepx_companion::amoo_workspace::Step {
                                executor: stepx_companion::amoo_workspace::Executor::Phone,
                                phase: stepx_companion::amoo_workspace::Phase::Running,
                                note: format!(
                                    "{}：{}",
                                    initial_report.model_label, initial_report.headline
                                ),
                            }];
                            task.route_note = topic.label().to_string();
                            task.updated = chrono::Utc::now().timestamp();
                        }
                    }
                    self.reveal_latest_chat();
                }
                self.doctor_running = true;
                if self.demo_mode {
                    let report =
                        stepx_companion::doctor_harness::default_demo_report(topic, &query);
                    self.apply(Outcome::DoctorHarnessReport(report));
                } else if self.selected.is_none() {
                    let page = self.page;
                    let tool = self.tool_page;
                    self.spawn_bulk(
                        "正在检查环境并请求手机账号 AI 诊断…",
                        move || {
                            let result = Adb::discover().and_then(|adb| {
                                stepx_companion::doctor_harness::run_diagnosis(
                                    &adb,
                                    None,
                                    topic,
                                    &query,
                                    |_| {},
                                )
                            });
                            match result {
                                Ok(report) => Outcome::DoctorHarnessReport(report),
                                Err(err) => Outcome::BulkFailed(page, tool, format!("{err:#}")),
                            }
                        },
                    );
                } else {
                    self.with_device_bulk(
                        "正在检查环境并请求手机账号 AI 诊断…",
                        move |adb, d| {
                            let report = stepx_companion::doctor_harness::run_diagnosis(
                                adb,
                                Some(&d),
                                topic,
                                &query,
                                |_| {},
                            )?;
                            Ok(Outcome::DoctorHarnessReport(report))
                        },
                    );
                }
            }
            Action::ExecuteDoctorQuickAction(kind) => {
                use stepx_companion::doctor_harness::DoctorActionKind;
                match kind {
                    DoctorActionKind::InstallGoogleSuite => {
                        self.trigger_subtab_switch();
                        self.page = Page::Tools;
                        self.tool_page = Some(ToolPage::Google);
                        self.tools_pending = None;
                        self.tool_confirmation = None;
                        self.focus = None;
                        self.scroll = Default::default();
                        self.auto_load_for_page();
                        if !self.busy && self.selected.is_some() {
                            self.tool_confirmation = Some(Box::new(Action::GoogleSuiteInstall));
                            self.scroll.scroll_to(facloud_ui::units::Pixels::new(0.));
                        } else if self.selected.is_none() {
                            self.status =
                                "已打开 Google Play 向导专页，连接手机后即可一键安装".into();
                        }
                    }
                    DoctorActionKind::RepairGoogleCompatibility => {
                        self.trigger_subtab_switch();
                        self.page = Page::Tools;
                        self.tool_page = Some(ToolPage::Google);
                        self.tools_pending = None;
                        self.tool_confirmation = None;
                        self.focus = None;
                        self.scroll = Default::default();
                        self.auto_load_for_page();
                        if !self.busy && self.selected.is_some() {
                            self.tool_confirmation = Some(Box::new(Action::GoogleSuiteMaintain));
                            self.scroll.scroll_to(facloud_ui::units::Pixels::new(0.));
                        } else if self.selected.is_none() {
                            self.status =
                                "已打开 Google Play 向导专页，连接手机后即可一键修复权限与服务"
                                    .into();
                        }
                    }
                    DoctorActionKind::InspectGoogleSuite => {
                        self.handle_device_action(&Action::GoogleSuiteInspect);
                    }
                    DoctorActionKind::OpenPlayStoreInMirror => {
                        self.handle_device_action(&Action::OpenAppInMirror(
                            "com.android.vending".into(),
                        ));
                    }
                    DoctorActionKind::OpenPlayStoreOnPhone => {
                        self.handle_device_action(&Action::OpenApp("com.android.vending".into()));
                    }
                    DoctorActionKind::OpenGoogleToolPage => {
                        self.trigger_subtab_switch();
                        self.page = Page::Tools;
                        self.tool_page = Some(ToolPage::Google);
                        self.tools_pending = None;
                        self.tool_confirmation = None;
                        self.focus = None;
                        self.scroll = Default::default();
                        self.auto_load_for_page();
                        self.status = "已打开 Google Play 端到端安装与使用向导".into();
                    }
                    DoctorActionKind::RepairCompanionPermissions
                    | DoctorActionKind::ReconnectWirelessAdb
                    | DoctorActionKind::CleanPcTempCache => {
                        let kind = *kind;
                        let device = self.selected.clone();
                        let page = self.page;
                        let tool = self.tool_page;
                        self.spawn_bulk("正在执行修复并核验结果…", move || {
                            match Adb::discover().and_then(|adb| {
                                stepx_companion::doctor_harness::execute_quick_repair(
                                    &adb,
                                    device.as_ref(),
                                    kind,
                                )
                            }) {
                                Ok(message) => Outcome::Notice(message),
                                Err(error) => Outcome::BulkFailed(page, tool, format!("{error:#}")),
                            }
                        });
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn submit_file_modal(&mut self) {
        let Some(modal) = self.file_modal else {
            return;
        };
        let name = self.fields[F_FILE_NAME].text.trim().to_string();
        if name.is_empty() {
            self.status = "请输入名称".into();
            self.error = true;
            return;
        }
        let directory = self.directory.clone();
        match modal {
            FileModal::NewFolder => self.with_device("正在新建文件夹…", move |adb, d| {
                storage::mkdir(adb, &d, &directory, &name)?;
                let list = storage::list(adb, &d, &directory)?;
                Ok(Outcome::Files(
                    directory,
                    list,
                    Some(format!("已新建文件夹 {name}")),
                ))
            }),
            FileModal::NewTextFile => {
                self.with_device("正在新建文本文件…", move |adb, d| {
                    let filename = if name.contains('.') {
                        name.clone()
                    } else {
                        format!("{name}.txt")
                    };
                    let tmp = std::env::temp_dir().join(&filename);
                    std::fs::write(&tmp, "")?;
                    storage::push(adb, &d, &tmp, &directory)?;
                    let _ = std::fs::remove_file(&tmp);
                    let list = storage::list(adb, &d, &directory)?;
                    Ok(Outcome::Files(
                        directory,
                        list,
                        Some(format!("已新建文本文件 {filename}")),
                    ))
                })
            }
            FileModal::Rename(idx) => {
                if let Some(entry) = self.files.get(idx).cloned() {
                    self.with_device("正在重命名…", move |adb, d| {
                        storage::rename(adb, &d, &entry.path, &name)?;
                        let list = storage::list(adb, &d, &directory)?;
                        Ok(Outcome::Files(
                            directory,
                            list,
                            Some(format!("已重命名为 {name}")),
                        ))
                    });
                }
            }
        }
    }

    pub(super) fn active_amoo_agent_name(&self) -> &str {
        self.amoo_agents
            .get(self.amoo_agent_idx)
            .map(String::as_str)
            .unwrap_or("Amoo 全能助手")
    }

    pub(super) fn active_amoo_model_name(&self) -> &str {
        self.active_amoo_model_short()
    }

    pub(super) fn active_amoo_model_short(&self) -> &str {
        self.work
            .selected_model
            .as_deref()
            .or_else(|| {
                self.amoo_sessions
                    .get(self.amoo_session_idx)
                    .and_then(|session| self.work.session_models.get(&session.session_key))
                    .map(String::as_str)
            })
            .unwrap_or("手机默认路由")
    }
}

pub(super) fn build_amoo_reply(prompt: &str, agent: &str) -> AmooChatItem {
    let wants_bolt = prompt.contains("背屏")
        || prompt.contains("闪电")
        || prompt.contains("点阵")
        || prompt.contains("像素")
        || prompt.contains("图案");
    let wants_invoice =
        prompt.contains("发票") || prompt.contains("报销") || prompt.contains("账单");
    let wants_sms = prompt.contains("短信") || prompt.contains("李明") || prompt.contains("发消息");

    let (tools_summary, steps, reply_text) = if wants_bolt {
        (
            "背屏点阵生成 · 亮度校准".to_string(),
            vec![
                format!("1. [{agent}] 解析背屏点阵视觉描述：「{prompt}」"),
                "2. 合成 23×17 (391 颗 LED) 像素灰度矩阵并适配圆角掩码".into(),
                "3. 生成背屏实时预览卡片，支持一键投递到手机副屏".into(),
            ],
            Some(
                "已为你生成 23×17 背屏像素图案方案。你可以点击下方「投到背屏」按钮直接同步至 StepX 手机背屏常显。"
                    .to_string(),
            ),
        )
    } else if wants_invoice {
        (
            "端侧 OCR · 报销单归档".to_string(),
            vec![
                format!("1. [{agent}] 扫描票据抬头、金额与开票日期"),
                "2. 校验高德打车与差旅餐饮电子发票明细（合计 ¥386.00）".into(),
                "3. 写入「10月报销单」并生成可导出结构化台账".into(),
            ],
            Some(
                "已完成票据识别与归档，本月累计入账 ¥386.00。点击下方「打开」可查看完整报销明细。"
                    .into(),
            ),
        )
    } else if prompt.contains("研究") || prompt.contains("热点") || prompt.contains("总结") {
        (
            "深度研究 · 跨端知识整理".to_string(),
            vec![
                format!("1. [{agent}] 拆解研究主题：「{prompt}」"),
                "2. 检索本地向量库与实时网页信源，交叉核验核心数据".into(),
                "3. 提炼 3 条结构化核心结论与行动建议".into(),
            ],
            Some(format!(
                "针对「{prompt}」已完成深度梳理：\n1. 核心结论：端侧算力与桌面跨端协同可将日常流转效率提升约 40%。\n2. 关键要点：结合个人记忆偏好自动执行高频归档与提醒。\n3. 下一步建议：可将结论同步至飞书文档或设为每周定时任务。"
            )),
        )
    } else {
        (
            format!("{agent} · 端侧推理"),
            vec![
                format!("1. [{agent}] 理解指令上下文与跨会话记忆（28 条偏好）"),
                "2. 调度已启用技能并生成直接、精炼的执行结果".into(),
            ],
            Some(format!(
                "收到你的指令：「{prompt}」。\n已结合你的「直接精炼」交流偏好处理完毕，如需投递到手机背屏或同步飞书日历可随时告诉我。"
            )),
        )
    };

    AmooChatItem::AssistantRich {
        thinking_time: "1.4s".into(),
        thinking_steps: steps,
        tools_summary,
        text: reply_text,
        has_bolt_artifact: wants_bolt,
        has_invoice_artifact: wants_invoice,
        has_safety_confirm: wants_sms,
        time_label: "刚刚".into(),
    }
}

pub(super) fn parent_dir(dir: &str) -> String {
    dir.trim_end_matches('/')
        .rsplit_once('/')
        .map(|(p, _)| p)
        .filter(|p| p.starts_with("/sdcard"))
        .unwrap_or("/sdcard")
        .to_string()
}

/// Only a confirmed SENT receipt can fill a temporarily missing provider row.
/// Pending/failed/unknown sends keep the draft and cannot create a success bubble.
pub(super) fn merge_confirmed_sms(
    messages: &mut std::result::Result<Vec<stepx_companion::sms::Message>, String>,
    status: &stepx_companion::bridge::SmsSendStatus,
    candidate: stepx_companion::sms::Message,
) {
    if status.state != "sent" {
        return;
    }
    if let Ok(list) = messages {
        let address = stepx_companion::sms::normalize_address(&candidate.address);
        let present = list.iter().any(|message| {
            message.outgoing
                && stepx_companion::sms::normalize_address(&message.address) == address
                && message.body.trim() == candidate.body.trim()
                && message.date_ms.abs_diff(candidate.date_ms) < 60_000
        });
        if !present {
            list.insert(0, candidate);
        }
    }
}

#[cfg(test)]
mod messaging_receipt_tests {
    use super::*;
    #[test]
    fn sms_success_rows_require_confirmed_sent_and_do_not_duplicate_provider_rows() {
        let candidate = stepx_companion::sms::Message {
            provider_id: Some(1),
            address: "10086".into(),
            body: "reply".into(),
            date_ms: 1_000,
            outgoing: true,
            kind: 2,
            read: true,
            subscription: None,
        };
        for state in ["failed", "sending", "unknown", "sent"] {
            let status = stepx_companion::bridge::SmsSendStatus {
                id: "request".into(),
                state: state.into(),
                parts: 1,
                completed: usize::from(state == "sent"),
                error_code: None,
                error: None,
                updated_ms: None,
            };
            let mut messages = Ok(vec![]);
            merge_confirmed_sms(&mut messages, &status, candidate.clone());
            assert_eq!(
                messages.as_ref().unwrap().len(),
                usize::from(state == "sent")
            );
            merge_confirmed_sms(&mut messages, &status, candidate.clone());
            assert_eq!(
                messages.as_ref().unwrap().len(),
                usize::from(state == "sent")
            );
            let mut failed_read = Err("provider unavailable".into());
            merge_confirmed_sms(&mut failed_read, &status, candidate.clone());
            assert!(failed_read.is_err());
        }
    }
    #[test]
    fn disconnected_direct_sms_keeps_the_draft_and_does_not_create_a_success_row() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = crate::ui::TestUiConfigDirGuard::set(dir.path());
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = None;
        app.fields[F_SMS_TO].set("10086");
        app.fields[F_SMS_BODY].set("keep this reply");
        let before = app.sms.len();
        app.handle_device_action(&Action::PrepareDirectSms);
        assert!(app.error && app.status.contains("未发送"));
        assert_eq!(app.fields[F_SMS_BODY].text, "keep this reply");
        assert_eq!(app.sms.len(), before);
    }
}
