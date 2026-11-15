//! Phone OTA jobs run outside the UI/device worker; results are scoped to a verified serial.
use super::*;
use stepx_companion::{
    ota::{self, OtaReport, OtaStatus},
    ota_transfer as transfer,
    updates::{self, PhoneVersion},
};

#[derive(Clone, Debug)]
pub(super) enum UpdateAction {
    Check,
    CheckOfficial,
    PhoneOpen,
    ToggleDetails,
    Download(usize, usize),
    CancelDownload,
    PrepareInstall,
    CancelInstall,
    ConfirmInstall,
    PrepareCleanup,
    ConfirmCleanup,
    CancelCleanup,
}
enum UpdateEvent {
    Progress(String, String),
    Phone(String, std::result::Result<PhoneVersion, String>),
    Report(String, std::result::Result<OtaReport, String>),
    Opened(String, std::result::Result<(), String>),
    Installation(String, std::result::Result<String, String>),
    CleanupInfo(String, std::result::Result<transfer::StagingUsage, String>),
    Cleaned(String, std::result::Result<transfer::StagingUsage, String>),
    Done,
}
#[derive(Default)]
pub(super) struct UpdateState {
    pub report: Option<OtaReport>,
    pub phone: Option<PhoneVersion>,
    pub phone_error: Option<String>,
    pub error: Option<String>,
    pub busy: bool,
    pub details: bool,
    pub progress: Option<String>,
    pub notice: Option<String>,
    receiver: Option<mpsc::Receiver<UpdateEvent>>,
    pub transfer: Option<transfer::Job>,
    pub transfer_serial: Option<String>,
    pub transfer_progress: Option<(String, u64, u64)>,
    pub staged: Option<transfer::StagedPackage>,
    pub confirm_install: bool,
    pub transfer_cancelled: bool,
    pub cleanup: Option<transfer::StagingUsage>,
}
impl UpdateState {
    pub fn title(&self) -> &str {
        if self.transfer.is_some() {
            return "正在准备手机更新包";
        }
        if self.busy {
            return "正在处理手机 OTA";
        }
        match &self.report {
            Some(r) if r.channels.iter().any(|c| c.status == OtaStatus::Available) => {
                "发现升级候选"
            }
            Some(r) if r.channels.iter().any(|c| c.status == OtaStatus::Metadata) => {
                "已获取版本信息"
            }
            Some(r)
                if r.channels
                    .iter()
                    .find(|c| c.channel == "current")
                    .is_some_and(|c| c.status == OtaStatus::NoNewerReturned) =>
            {
                "当前渠道未返回升级候选"
            }
            Some(_) => "查看检测详情",
            None => "官方 OTA 与测试渠道",
        }
    }
}
impl Companion {
    pub(super) fn handle_update_action(
        &mut self,
        action: &UpdateAction,
        ctx: &mut ViewContext<Self>,
    ) {
        let result = self.perform_update_action(action);
        if let Err(error) = result {
            self.updates.error = Some(format!("{error:#}"));
        }
        ctx.notify();
    }
    fn perform_update_action(&mut self, action: &UpdateAction) -> Result<()> {
        match action {
            UpdateAction::CancelDownload => {
                self.updates.transfer_cancelled = true;
                if let Some(job) = &self.updates.transfer {
                    job.cancel();
                }
                self.updates.notice = Some("正在取消，请等待清理完成。".into());
                return Ok(());
            }
            UpdateAction::CancelCleanup => {
                self.updates.cleanup = None;
                return Ok(());
            }
            UpdateAction::CancelInstall => {
                self.updates.confirm_install = false;
                return Ok(());
            }
            _ => {}
        }
        if matches!(action, UpdateAction::ToggleDetails) {
            self.updates.details = !self.updates.details;
            return Ok(());
        }
        if self.updates.busy || self.updates.transfer.is_some() {
            return Ok(());
        }
        if matches!(
            action,
            UpdateAction::PrepareCleanup | UpdateAction::ConfirmCleanup
        ) {
            let mut device = self
                .selected
                .clone()
                .ok_or_else(|| anyhow::anyhow!("请先连接手机"))?;
            let clean = matches!(action, UpdateAction::ConfirmCleanup);
            anyhow::ensure!(!clean || self.updates.cleanup.is_some(), "请先检查暂存包");
            self.updates.busy = true;
            self.updates.error = None;
            self.updates.progress = Some(
                if clean {
                    "正在清理手机 OTA 暂存包…"
                } else {
                    "正在检查手机 OTA 暂存包…"
                }
                .into(),
            );
            let (tx, rx) = mpsc::channel();
            self.updates.receiver = Some(rx);
            std::thread::spawn(move || {
                let result = Adb::discover()
                    .and_then(|adb| {
                        device.transport = device.resolve_active_transport(&adb)?;
                        if clean {
                            transfer::cleanup_staged(&adb, &device)
                        } else {
                            transfer::inspect_staged(&adb, &device)
                        }
                    })
                    .map_err(|e| format!("{e:#}"));
                let event = if clean {
                    UpdateEvent::Cleaned(device.serial, result)
                } else {
                    UpdateEvent::CleanupInfo(device.serial, result)
                };
                let _ = tx.send(event);
                let _ = tx.send(UpdateEvent::Done);
            });
            return Ok(());
        }
        if let UpdateAction::Download(ci, ri) = action {
            anyhow::ensure!(
                self.updates.staged.is_none(),
                "手机上已有准备好的更新包，请先移除再下载其他版本"
            );
            self.updates.transfer_cancelled = false;
            let device = self
                .selected
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("请先连接手机"))?;
            let release = self
                .updates
                .report
                .as_ref()
                .filter(|r| r.serial == device.serial)
                .and_then(|r| r.channels.get(*ci))
                .and_then(|c| c.releases.get(*ri))
                .ok_or_else(|| anyhow::anyhow!("版本列表已变化，请重新检查 OTA"))?
                .clone();
            self.updates.transfer = Some(transfer::start_download(release)?);
            self.updates.transfer_serial = Some(device.serial.clone());
            self.updates.transfer_progress = Some(("正在准备下载".into(), 0, 0));
            self.updates.error = None;
            self.updates.notice = None;
            self.updates.staged = None;
            self.updates.confirm_install = false;
            return Ok(());
        }
        if matches!(action, UpdateAction::PrepareInstall) {
            anyhow::ensure!(self.updates.staged.is_some(), "请先下载并校验更新包");
            self.updates.confirm_install = true;
            return Ok(());
        }
        if matches!(action, UpdateAction::ConfirmInstall) {
            anyhow::ensure!(self.updates.confirm_install, "请先核对升级信息");
            let mut device = self
                .selected
                .clone()
                .ok_or_else(|| anyhow::anyhow!("请先连接手机"))?;
            let staged = self
                .updates
                .staged
                .clone()
                .ok_or_else(|| anyhow::anyhow!("更新包尚未就绪"))?;
            let report = self
                .updates
                .report
                .clone()
                .ok_or_else(|| anyhow::anyhow!("请先检查 OTA"))?;
            let (tx, rx) = mpsc::channel();
            self.updates.receiver = Some(rx);
            self.updates.busy = true;
            self.updates.confirm_install = false;
            self.updates.error = None;
            self.updates.progress = Some("正在提交手机官方校验…".into());
            std::thread::spawn(move || {
                let result = Adb::discover()
                    .and_then(|adb| {
                        device.transport = device.resolve_active_transport(&adb)?;
                        transfer::submit_install(
                            &adb,
                            &device,
                            &staged,
                            &report,
                            staged.package.firmware(),
                        )
                    })
                    .map(|receipt| receipt.message);
                let _ = tx.send(UpdateEvent::Installation(
                    device.serial,
                    result.map_err(|e| format!("{e:#}")),
                ));
                let _ = tx.send(UpdateEvent::Done);
            });
            return Ok(());
        }
        let mut device = self
            .selected
            .clone()
            .ok_or_else(|| anyhow::anyhow!("请先连接手机"))?;
        self.updates.error = None;
        self.updates.phone_error = None;
        self.updates.notice = None;
        self.updates.busy = true;
        self.updates.progress = Some("正在连接手机官方更新服务…".into());
        let open = matches!(action, UpdateAction::PhoneOpen);
        let official = matches!(action, UpdateAction::CheckOfficial);
        if !open {
            self.updates.report = None;
        }
        let (tx, rx) = mpsc::channel();
        self.updates.receiver = Some(rx);
        std::thread::spawn(move || {
            let serial = device.serial.clone();
            if open {
                let result = Adb::discover().and_then(|adb| {
                    device.transport = device.resolve_active_transport(&adb)?;
                    updates::open_phone_update(&adb, &device)
                });
                let _ = tx.send(UpdateEvent::Opened(
                    serial,
                    result.map_err(|e| format!("{e:#}")),
                ));
            } else {
                let result = Adb::discover().and_then(|adb| {
                    device.transport = device.resolve_active_transport(&adb)?;
                    let phone = updates::phone_version(&adb, &device);
                    let _ = tx.send(UpdateEvent::Phone(
                        serial.clone(),
                        phone.map_err(|e| format!("{e:#}")),
                    ));
                    let progress = |message: &str| {
                        let _ = tx.send(UpdateEvent::Progress(serial.clone(), message.into()));
                    };
                    if official {
                        ota::check_official(&adb, &device, progress)
                    } else {
                        ota::check_with_progress(&adb, &device, progress)
                    }
                });
                let _ = tx.send(UpdateEvent::Report(
                    serial,
                    result.map_err(|e| format!("{e:#}")),
                ));
            }
            let _ = tx.send(UpdateEvent::Done);
        });
        Ok(())
    }
    pub(super) fn poll_updates(&mut self) -> bool {
        let mut changed = self.poll_ota_transfer();
        loop {
            let event = match self.updates.receiver.as_ref().map(|rx| rx.try_recv()) {
                Some(Ok(event)) => event,
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    self.updates.receiver = None;
                    self.updates.busy = false;
                    self.updates.progress = None;
                    return true;
                }
                _ => break,
            };
            changed = true;
            match event {
                UpdateEvent::Progress(serial, message)
                    if self.selected.as_ref().is_some_and(|d| d.serial == serial) =>
                {
                    self.updates.progress = Some(message)
                }
                UpdateEvent::Phone(serial, result)
                    if self.selected.as_ref().is_some_and(|d| d.serial == serial) =>
                {
                    match result {
                        Ok(phone) => self.updates.phone = Some(phone),
                        Err(error) => {
                            self.updates.phone = None;
                            self.updates.phone_error = Some(error);
                        }
                    }
                }
                UpdateEvent::Report(serial, result)
                    if self.selected.as_ref().is_some_and(|d| d.serial == serial) =>
                {
                    match result {
                        Ok(report) => self.updates.report = Some(report),
                        Err(error) => self.updates.error = Some(error),
                    }
                }
                UpdateEvent::Opened(serial, result)
                    if self.selected.as_ref().is_some_and(|d| d.serial == serial) =>
                {
                    match result {
                        Ok(()) => self.updates.notice = Some("已打开手机官方系统更新页面。".into()),
                        Err(error) => self.updates.phone_error = Some(error),
                    }
                }
                UpdateEvent::CleanupInfo(serial, result)
                    if self.selected.as_ref().is_some_and(|d| d.serial == serial) =>
                {
                    match result {
                        Ok(usage) => self.updates.cleanup = Some(usage),
                        Err(error) => self.updates.error = Some(error),
                    }
                }
                UpdateEvent::Cleaned(serial, result)
                    if self.selected.as_ref().is_some_and(|d| d.serial == serial) =>
                {
                    match result {
                        Ok(usage) => {
                            self.updates.cleanup = None;
                            self.updates.staged = None;
                            self.updates.confirm_install = false;
                            self.updates.notice = Some(format!(
                                "已清理 {} 个手机 OTA 暂存包，释放 {}。",
                                usage.count,
                                stepx_companion::storage::format_bytes(usage.bytes)
                            ));
                        }
                        Err(error) => self.updates.error = Some(error),
                    }
                }
                UpdateEvent::Installation(serial, result)
                    if self.selected.as_ref().is_some_and(|d| d.serial == serial) =>
                {
                    match result {
                        Ok(message) => self.updates.notice = Some(message),
                        Err(error) => self.updates.error = Some(error),
                    }
                }
                UpdateEvent::Done => {
                    self.updates.busy = false;
                    self.updates.receiver = None;
                    self.updates.progress = None;
                    break;
                }
                _ => {}
            }
        }
        changed
    }
    fn poll_ota_transfer(&mut self) -> bool {
        let mut changed = false;
        loop {
            let event = match self
                .updates
                .transfer
                .as_ref()
                .map(|job| job.events.try_recv())
            {
                Some(Ok(event)) => event,
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    self.updates.transfer = None;
                    self.updates.transfer_progress = None;
                    self.updates.error = Some("更新包准备中断，请重试。".into());
                    return true;
                }
                _ => break,
            };
            changed = true;
            if !self
                .selected
                .as_ref()
                .is_some_and(|d| Some(&d.serial) == self.updates.transfer_serial.as_ref())
            {
                if let Some(job) = self.updates.transfer.take() {
                    job.cancel();
                }
                self.updates.transfer_progress = None;
                break;
            }
            match event {
                transfer::Event::Progress { phase, done, total } => {
                    let label = match phase.as_str() {
                        "download" => "正在下载官方固件",
                        "verify" => "正在校验电脑上的固件",
                        "transfer" => "正在传输到手机",
                        "phone_verify" => "正在校验手机上的固件",
                        _ => &phase,
                    };
                    self.updates.transfer_progress = Some((label.to_owned(), done, total));
                }
                transfer::Event::Downloaded(package) => {
                    self.updates.transfer = None;
                    if self.updates.transfer_cancelled {
                        self.updates.transfer_progress = None;
                        self.updates.notice =
                            Some("已取消；电脑上已完成校验的缓存可用于下次下载。".into());
                        break;
                    }
                    let Some(mut device) = self.selected.clone() else {
                        self.updates.transfer_progress = None;
                        self.updates.error =
                            Some("手机已断开连接；已下载的固件缓存在电脑上，重连后可继续。".into());
                        break;
                    };
                    let active_transport = self.active_transport();
                    if !active_transport.is_empty() {
                        device.transport = active_transport;
                    } else {
                        device.transport =
                            super::active_transport_for(&device.serial, &device.transport);
                    }
                    match Adb::discover().and_then(|adb| {
                        if let Ok(active) = device.resolve_active_transport(&adb) {
                            device.transport = active;
                        }
                        transfer::start_stage(adb, device, package)
                    }) {
                        Ok(job) => self.updates.transfer = Some(job),
                        Err(error) => {
                            self.updates.error = Some(format!("{error:#}"));
                            self.updates.transfer_progress = None;
                        }
                    }
                }
                transfer::Event::Staged(package) => {
                    self.updates.staged = Some(package);
                    self.updates.transfer = None;
                    self.updates.transfer_progress = None;
                    self.updates.notice = Some("更新包已传到手机，电脑与手机两端校验一致。".into());
                }
                transfer::Event::Failed(error) => {
                    self.updates.error = Some(error);
                    self.updates.transfer = None;
                    self.updates.transfer_progress = None;
                }
                transfer::Event::Cancelled => {
                    self.updates.notice = Some("已取消更新包准备。".into());
                    self.updates.transfer = None;
                    self.updates.transfer_progress = None;
                }
            }
        }
        changed
    }
}
