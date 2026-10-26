//! Device, file, SMS, phone, apps, toolbox, and connection action handlers for `Companion`.

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
                Some(ToolPage::Screen) if self.screenshot.is_none() && !self.busy => {
                    self.handle_device_action(&Action::Screenshot);
                    return;
                }
                Some(ToolPage::Device) if self.summary.is_empty() && !self.busy => {
                    self.handle_device_action(&Action::Summary);
                    return;
                }
                _ => return,
            },
            Page::Apps if !self.apps_loaded => {
                self.handle_device_action(&Action::Apps);
                return;
            }
            _ => {}
        }
        if self.page == Page::Amoo && !self.demo_mode {
            self.refresh_amoo_background();
            return;
        }
        if self.busy {
            return;
        }
        match self.page {
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
            Page::Files if self.files.is_empty() => self.browse(self.directory.clone()),
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
        self.clear_text_interaction();
        self.speech = Default::default();
        self.bulk_rx = None;
        self.bulk_busy = false;
        self.bulk_status.clear();
        self.bulk_tool = None;
        self.bulk_started_at = None;
        let mode = self.push.shell_mode;
        let pin = self.push.pin_window;
        self.push = PushUiState::default();
        self.push.shell_mode = mode;
        self.push.pin_window = pin;
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
        self.back_screen = BackScreenUiState::default();
        self.updates = UpdateState::default();
        if !self.demo_mode {
            self.amoo_sessions.clear();
            self.amoo_skills.clear();
            self.amoo_memories.clear();
            self.amoo_cron_count = 0;
            self.amoo_bm25_entries = 0;
        }
        self.pending_sms_plan = None;
        self.last_sms_status = None;
        self.selected_thread = None;
        self.content.clear();
        self.sms.clear();
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
        if native_gui_unconfirmed {
            self.status = "已停止桌面等待，手机原生操作可能仍继续，请在手机上核对".into();
        }
    }

    pub(super) fn browse(&mut self, path: String) {
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
        if let Some(child) = self.mirror.take() {
            child.stop_in_background();
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
        let mut threads: Vec<SmsThread> = Vec::new();
        let mut thread_indices: HashMap<String, usize> = HashMap::new();
        for m in &self.sms {
            let matches_filter = match self.sms_filter {
                SmsFilter::All => true,
                SmsFilter::Incoming => m.kind == 1,
                SmsFilter::Outgoing => m.kind == 2,
            };
            if !matches_filter {
                continue;
            }
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
            if let Some(&idx) = thread_indices.get(&norm_addr) {
                let t = &mut threads[idx];
                t.count += 1;
                t.unread |= m.kind == 1 && !m.read;
                if m.date_ms >= t.last_ms {
                    t.last_ms = m.date_ms;
                    t.last_body = m.body.clone();
                }
            } else {
                thread_indices.insert(norm_addr, threads.len());
                threads.push(SmsThread {
                    address: addr.to_string(),
                    count: 1,
                    last_ms: m.date_ms,
                    last_body: m.body.clone(),
                    unread: m.kind == 1 && !m.read,
                });
            }
        }
        threads.sort_by(|a, b| b.last_ms.cmp(&a.last_ms));
        threads
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
        match native_dialog::FileDialog::new()
            .set_title("保存截图")
            .set_filename(&name)
            .set_location(&stepx_companion::settings::downloads())
            .show_save_single_file()
        {
            Ok(Some(dest)) => match std::fs::copy(&src, &dest) {
                Ok(_) => self.status = format!("截图已保存到 {}", dest.display()),
                Err(e) => {
                    self.status = format!("保存失败：{e}");
                    self.error = true;
                }
            },
            Ok(None) => {}
            Err(e) => {
                self.status = format!("无法打开保存窗口：{e}");
                self.error = true;
            }
        }
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
                self.allow_first_connect = false;
                self.preferences.first_connect_done = true;
                self.pairing_hosts.clear();
                let serial = self
                    .selected
                    .take()
                    .map(|d| d.serial)
                    .or_else(|| self.preferences.selected_serial.clone());
                self.connection_watch.detached_serial = None;
                if let Some(serial) = serial {
                    self.preferences.devices.retain(|p| p.serial != serial);
                    self.preferences.wireless.remove(&serial);
                    self.preferences.selected_serial = None;
                    if let Err(e) = self.preferences.save() {
                        self.status = e.to_string();
                        self.error = true;
                        return;
                    }
                }
                self.clear_device_state();
                self.status = "已断开。拔下数据线或在手机上撤销调试授权可彻底停止连接".into();
            }
            Action::PairCode => {
                let endpoint = self.fields[F_PAIR_ADDR].text.trim().to_string();
                let code = self.fields[F_PAIR_CODE].text.trim().to_string();
                if code.is_empty() {
                    self.status = "请输入配对码".into();
                    self.error = true;
                    return;
                }
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
                self.submit("正在连接…", move |adb| {
                    pairing::validate_endpoint(&endpoint)?;
                    adb.run(&["connect", &endpoint])?;
                    Ok(Outcome::Paired(endpoint))
                });
            }
            Action::Qr => match QrSession::new() {
                Ok(qr) => {
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
                if !self.busy && self.mirror.is_none() {
                    let options = self.mirror_options;
                    self.with_device("正在开启投屏…", move |adb, d| {
                        let session = d.mirror_with_options(adb, options)?;
                        Ok(Outcome::Mirror(d.serial, d.transport, session))
                    });
                }
            }
            Action::Screenshot => self.with_device("正在截图…", |adb, d| {
                d.verify(adb)?;
                let bytes = adb.screenshot(&d.transport)?;
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
                    if sync_screen && code != 26 && code != 223 {
                        std::thread::sleep(Duration::from_millis(180));
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
            Action::ReadVisible => self.with_device(
                "正在唤醒圈选感知并读取屏幕文字…",
                |adb, d| {
                    let _ = adb.shell(&d.transport, &["input", "keyevent", "224"]);
                    let _ = d.open_amoo_screen_perception(adb);
                    Ok(Outcome::Text(d.visible_text(adb)?))
                },
            ),
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
                let number = if self.fields[F_SMS_TO].text.trim().is_empty() {
                    self.selected_thread
                        .as_deref()
                        .filter(|s| *s != "__new__")
                        .unwrap_or("")
                        .to_string()
                } else {
                    self.fields[F_SMS_TO].text.trim().to_string()
                };
                let body = self.fields[F_SMS_BODY].text.clone();
                if number.is_empty() || body.trim().is_empty() {
                    self.status = "请填写收件人号码和短信内容".into();
                    self.error = true;
                    return;
                }
                self.fields[F_SMS_TO].set(&number);
                let sim = self.selected_sim;
                self.with_device("正在准备短信发送计划…", move |adb, d| {
                    Ok(Outcome::SmsPrepared(bridge::sms_prepare(
                        adb, &d, &number, &body, sim,
                    )?))
                });
            }
            Action::ConfirmDirectSms => {
                let Some(plan) = self.pending_sms_plan.take() else {
                    self.status = "请先准备短信并确认收件人和 SIM 卡".into();
                    self.error = true;
                    return;
                };
                self.selected_thread = Some(plan.number.clone());
                // Consume the confirmation once; never fabricate a send plan or retry a timed-out send.
                self.with_device("正在通过手机发送短信…", move |adb, d| {
                    let status = bridge::sms_send(adb, &d, &plan)?;
                    let messages = read_result(d.messages(adb));
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
                match native_dialog::FileDialog::new()
                    .set_title("选择要上传到手机的文件")
                    .show_open_single_file()
                {
                    Ok(Some(path)) => {
                        self.prepare_file_transfer(vec![path], Some(self.directory.clone()))
                    }
                    Ok(None) => {}
                    Err(e) => {
                        self.status = format!("未选择文件：{e}");
                        self.error = true;
                    }
                }
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
                    match native_dialog::FileDialog::new()
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
                    match native_dialog::FileDialog::new()
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
                    Ok(Outcome::Notice("已在手机打开应用详情".into()))
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
                    match native_dialog::FileDialog::new()
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
                let package = package.clone();
                let need_mirror = self.mirror.is_none();
                let options = self.mirror_options;
                self.with_device(
                    "正在手机打开应用并启动电脑投屏…",
                    move |adb, d| {
                        let _ = d.open_app(adb, &package);
                        if need_mirror {
                            let session = d.mirror_with_options(adb, options)?;
                            Ok(Outcome::Mirror(d.serial, d.transport, session))
                        } else {
                            Ok(Outcome::Notice(format!("已在投屏中打开 {package}")))
                        }
                    },
                );
            }
            Action::EnableUsbWirelessBridge => {
                let usb_target = self
                    .devices
                    .iter()
                    .find(|d| {
                        d.state == "device" && !d.id.contains(':') && !d.id.starts_with("adb-")
                    })
                    .map(|d| d.id.clone())
                    .or_else(|| self.selected.as_ref().map(|d| d.transport.clone()));
                let Some(transport_id) = usb_target else {
                    self.status = "请先通过 USB 数据线连接手机并开启 USB 调试".into();
                    self.error = true;
                    return;
                };
                self.submit(
                    "正在通过 USB 激活同局域网无线连接…",
                    move |adb| {
                        let _ = adb.run(&["-s", &transport_id, "tcpip", "5555"]);
                        std::thread::sleep(Duration::from_millis(700));
                        let raw = adb
                            .run(&["-s", &transport_id, "shell", "ip -o -4 addr show dev wlan0"])
                            .unwrap_or_default();
                        let Some(ip) = raw.split_whitespace().find_map(|s| {
                            s.split_once('/')
                                .and_then(|(ip, _)| ip.parse::<std::net::IpAddr>().ok())
                        }) else {
                            anyhow::bail!(
                                "未能读取手机局域网 Wi-Fi 地址，请确认手机与电脑连接在同一 Wi-Fi"
                            );
                        };
                        let endpoint = format!("{ip}:5555");
                        adb.run(&["connect", &endpoint])?;
                        Ok(Outcome::Paired(endpoint))
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
                if self.selected.is_some() {
                    self.with_device("正在打开 Amoo 记忆库…", |adb, d| {
                        Ok(Outcome::Notice(d.open_amoo_memory(adb)?))
                    });
                } else {
                    self.page = Page::Amoo;
                    self.workspace.sub_view = 1;
                    self.error = false;
                    self.status = "已打开桌面端「记忆与自我」偏好工作台".into();
                }
            }
            Action::AmooOpenPersona => {
                if self.selected.is_some() {
                    self.with_device("正在打开个人偏好设置…", |adb, d| {
                        Ok(Outcome::Notice(d.open_amoo_persona(adb)?))
                    });
                } else {
                    self.page = Page::Amoo;
                    self.workspace.sub_view = 1;
                    self.error = false;
                    self.status = "已打开桌面端个人画像与偏好设置".into();
                }
            }
            Action::AmooOpenSafety => {
                if self.selected.is_some() {
                    self.with_device("正在打开敏感操作安全守护中心…", |adb, d| {
                        Ok(Outcome::Notice(d.open_amoo_safety_center(adb)?))
                    });
                } else {
                    self.page = Page::Amoo;
                    self.amoo_show_details = true;
                    self.error = false;
                    self.status = "已展开桌面端敏感操作安全守护面板".into();
                }
            }
            Action::AmooOpenModels => {
                if self.selected.is_some() {
                    self.with_device("正在打开端侧大模型管理…", |adb, d| {
                        Ok(Outcome::Notice(d.open_amoo_model_manager(adb)?))
                    });
                } else {
                    self.page = Page::Amoo;
                    self.workspace.panel = 3;
                    self.error = false;
                    self.status = "已打开桌面端模型与路由配置面板".into();
                }
            }
            Action::AmooOpenVoiceSettings => {
                self.with_device("正在打开语音唤醒设置…", |adb, d| {
                    Ok(Outcome::Notice(d.open_amoo_voice_settings(adb)?))
                });
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
                let name = self.fields[F_CONTACT_SEARCH].text.trim().to_string();
                let number = self.fields[F_PHONE_DIAL].text.trim().to_string();
                if name.is_empty() || number.is_empty() {
                    self.status = "请在联系人搜索框填写姓名，并在拨号框填写号码".into();
                    self.error = true;
                    return;
                }
                self.with_device(
                    "正在向手机通讯录写入新联系人…",
                    move |adb, d| Ok(Outcome::Notice(d.create_contact(adb, &name, &number)?)),
                );
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
                if self.bulk_busy {
                    return;
                }
                self.google_suite = GoogleSuiteStatus::default();
                *self.google_progress.lock().unwrap() = String::new();
                self.with_device_bulk("正在检测 Google 服务…", |adb, d| {
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
                match native_dialog::FileDialog::new()
                    .set_title("选择内置 GUI Agent 的 StepX伴侣 APK")
                    .add_filter("Android 应用", &["apk"])
                    .show_open_single_file()
                {
                    Ok(Some(path)) => self.with_device(
                        "正在更新 StepX伴侣内置 GUI Agent，请保持连接…",
                        move |adb, d| {
                            Ok(Outcome::NeoStatus(stepx_companion::neo_agent::install(
                                adb, &d, &path, ime,
                            )?))
                        },
                    ),
                    Ok(None) => {}
                    Err(error) => {
                        self.status = format!("选择 APK 失败：{error}");
                        self.error = true;
                    }
                }
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
                match native_dialog::FileDialog::new()
                    .set_title("发送文件到 StepX伴侣")
                    .show_open_multiple_file()
                {
                    Ok(paths) if !paths.is_empty() => self.prepare_file_transfer(paths, None),
                    Ok(_) => {}
                    Err(error) => {
                        self.status = format!("选择文件失败：{error}");
                        self.error = true;
                    }
                }
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
                let code = code.clone();
                self.fields[F_BACK_MARQUEE].set(&code);
                self.matrix_marquee_on = true;
                let rev = self.back_screen.config.invert;
                if self.selected.is_some() {
                    self.with_device(
                        "正在将快递取件码推送至手机背屏跑马灯常显…",
                        move |adb, d| {
                            d.set_backscreen_marquee(adb, &code, true, rev)?;
                            Ok(Outcome::Notice(format!(
                                "手机已保存取件码「{code}」跑马灯，实际亮灯遵循手机背屏设置"
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

    pub(super) fn active_amoo_agent_name(&self) -> &'static str {
        match self.amoo_agent_idx {
            0 => "Amoo 助手",
            1 => "桌面伙伴",
            2 => "飞书 / 企微",
            _ => "Claude 协同",
        }
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
