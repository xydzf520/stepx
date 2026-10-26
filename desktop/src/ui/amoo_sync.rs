//! Passive conversation reads never occupy the foreground command worker.
//! Any foreground action invalidates an older snapshot before it can overwrite a send.
use super::*;

pub(super) struct SnapshotJob {
    generation: u64,
    serial: String,
    key: Option<String>,
    receiver: mpsc::Receiver<std::result::Result<Option<RealAmooData>, String>>,
}
struct SyncRequest {
    key: Option<String>,
    pending: Vec<String>,
    task: bool,
    reply: mpsc::Sender<std::result::Result<Option<RealAmooData>, String>>,
    generation: u64,
}
pub(super) struct SyncWorker {
    transport: String,
    sender: mpsc::Sender<SyncRequest>,
    pub(super) reset_baseline: Arc<std::sync::atomic::AtomicBool>,
    has_pending: Arc<std::sync::atomic::AtomicBool>,
    delta_rx: mpsc::Receiver<Vec<amoo::StreamDelta>>,
}
impl SyncWorker {
    fn start(adb: Adb, device: Device) -> Self {
        let (sender, receiver) = mpsc::channel::<SyncRequest>();
        let (delta_tx, delta_rx) = mpsc::channel::<Vec<amoo::StreamDelta>>();
        let transport = device.transport.clone();
        let reset_baseline = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_reset = reset_baseline.clone();
        let has_pending = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_pending = has_pending.clone();
        std::thread::spawn(move || {
            let mut bus: Option<(bool, amoo::AgentBus)> = None;
            let mut previous: Option<(u64, Option<String>, u64)> = None;
            let mut active_pending = false;
            loop {
                let mut next_req: Option<SyncRequest> = None;
                if bus.is_some()
                    && (active_pending || worker_pending.load(std::sync::atomic::Ordering::Relaxed))
                {
                    for _ in 0..180 {
                        if let Ok(req) = receiver.try_recv() {
                            next_req = Some(req);
                            break;
                        }
                        if !active_pending
                            && !worker_pending.load(std::sync::atomic::Ordering::Relaxed)
                        {
                            break;
                        }
                        let Some((_, ref mut active_bus)) = bus else {
                            break;
                        };
                        match active_bus.poll_stream_events(Duration::from_millis(55)) {
                            Ok(deltas) => {
                                if !deltas.is_empty() {
                                    let _ = delta_tx.send(deltas);
                                }
                            }
                            Err(_) => {
                                bus = None;
                                break;
                            }
                        }
                    }
                }
                let mut request = match next_req {
                    Some(request) => request,
                    None => match receiver.recv_timeout(Duration::from_secs(25)) {
                        Ok(request) => request,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            bus = None;
                            continue;
                        }
                        Err(_) => break,
                    },
                };
                while let Ok(newer) = receiver.try_recv() {
                    request = newer;
                }
                active_pending = !request.pending.is_empty();
                worker_pending.store(active_pending, std::sync::atomic::Ordering::Relaxed);
                if worker_reset.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    previous = None;
                }
                // Escalation releases the previous passive observer on this worker,
                // never on the UI thread. A warm task bus already supports observe reads
                // and is released automatically when the worker idles out.
                if bus.as_ref().is_some_and(|(task, _)| !*task && request.task) {
                    bus = None;
                }
                let started = Instant::now();
                let reused = bus.is_some();
                let mut try_snapshot = |use_task_connect: bool,
                                        reset_bus: bool|
                 -> Result<RealAmooData> {
                    if reset_bus {
                        bus = None;
                    }
                    if bus.is_none() {
                        bus = Some(if use_task_connect {
                            (true, amoo::AgentBus::connect(&adb, &device)?)
                        } else {
                            match amoo::AgentBus::observe(&adb, &device) {
                                Ok(observed) => (false, observed),
                                Err(_) => (true, amoo::AgentBus::connect(&adb, &device)?),
                            }
                        });
                    }
                    let active_bus = &mut bus.as_mut().unwrap().1;
                    let data = active_bus
                        .snapshot_with_pending(request.key.as_deref(), &request.pending)?;
                    if let Ok(deltas) = active_bus.poll_stream_events(Duration::from_millis(5)) {
                        if !deltas.is_empty() {
                            let _ = delta_tx.send(deltas);
                        }
                    }
                    Ok(data)
                };
                let result = match try_snapshot(request.task, false) {
                    Ok(data) => Ok(data),
                    Err(_) if !request.task => try_snapshot(true, true),
                    Err(err) => Err(err),
                }
                .map_err(|e| format!("{e:#}"));
                if result.is_err() {
                    bus = None;
                }
                if std::env::var_os("STEPX_PERF_TRACE").is_some() {
                    eprintln!(
                        "AMOO_SYNC ok={} reused={} elapsed_ms={}",
                        result.is_ok(),
                        reused,
                        started.elapsed().as_millis()
                    );
                }
                let key_sig = request.key.clone();
                let result = result.map(|data| {
                    use std::hash::{Hash, Hasher};
                    let mut hash = std::collections::hash_map::DefaultHasher::new();
                    serde_json::to_vec(&data)
                        .unwrap_or_default()
                        .hash(&mut hash);
                    let signature = (request.generation, key_sig, hash.finish());
                    if previous == Some(signature.clone()) {
                        None
                    } else {
                        previous = Some(signature);
                        Some(data)
                    }
                });
                if result.is_err() {
                    previous = None;
                }
                let _ = request.reply.send(result);
            }
        });
        Self {
            transport,
            sender,
            reset_baseline,
            has_pending,
            delta_rx,
        }
    }
}
fn matches_request(
    job: &SnapshotJob,
    generation: u64,
    serial: Option<&str>,
    key: Option<&str>,
    busy: bool,
) -> bool {
    !busy
        && job.generation == generation
        && serial == Some(job.serial.as_str())
        && key == job.key.as_deref()
}
impl Companion {
    pub(super) fn current_amoo_sync_key(&self) -> Option<String> {
        let work_key = (self.page == Page::Work && self.workspace.running.is_none())
            .then(|| self.work_session().map(|s| s.session_key.clone()))
            .flatten();
        work_key
            .or_else(|| self.workspace_task().and_then(|t| t.session.clone()))
            .or_else(|| {
                self.amoo_sessions
                    .get(self.amoo_session_idx)
                    .map(|s| s.session_key.clone())
            })
    }

    pub(super) fn request_amoo_session_sync(&mut self, key: Option<String>) {
        if self.demo_mode || self.selected.is_none() {
            return;
        }
        let key = key.or_else(|| self.current_amoo_sync_key());
        self.amoo_ui.snapshot_generation = self.amoo_ui.snapshot_generation.wrapping_add(1);
        if let Some(worker) = self.amoo_ui.snapshot_worker.as_ref() {
            worker
                .reset_baseline
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.amoo_ui.snapshot_job = None;
        self.amoo_poll_remaining = self.amoo_poll_remaining.max(4);
        self.last_amoo_poll = Instant::now()
            .checked_sub(Duration::from_secs(60))
            .unwrap_or_else(Instant::now);
        let pending = self.workspace_sync_pending_keys();
        self.begin_amoo_sync(key, pending, true);
    }

    pub(super) fn refresh_amoo_background(&mut self) {
        let key = self.current_amoo_sync_key();
        self.request_amoo_session_sync(key);
    }

    pub(super) fn begin_amoo_sync(
        &mut self,
        key: Option<String>,
        pending: Vec<String>,
        initial: bool,
    ) {
        let key = key.or_else(|| self.current_amoo_sync_key());
        if let Some(worker) = self.amoo_ui.snapshot_worker.as_ref() {
            worker
                .has_pending
                .store(!pending.is_empty(), std::sync::atomic::Ordering::Relaxed);
        }
        if self.amoo_ui.snapshot_job.is_some() {
            return;
        }
        let Some(device) = self.selected.clone() else {
            return;
        };
        let Ok(adb) = Adb::discover() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.amoo_ui.snapshot_job = Some(SnapshotJob {
            generation: self.amoo_ui.snapshot_generation,
            serial: device.serial.clone(),
            key: key.clone(),
            receiver,
        });
        if self
            .amoo_ui
            .snapshot_worker
            .as_ref()
            .is_none_or(|w| w.transport != device.transport)
        {
            self.amoo_ui.snapshot_worker = Some(SyncWorker::start(adb, device));
        }
        if let Some(worker) = self.amoo_ui.snapshot_worker.as_ref() {
            worker
                .has_pending
                .store(!pending.is_empty(), std::sync::atomic::Ordering::Relaxed);
        }
        if self
            .amoo_ui
            .snapshot_worker
            .as_ref()
            .unwrap()
            .sender
            .send(SyncRequest {
                key,
                pending: pending.clone(),
                task: initial || !pending.is_empty(),
                reply: sender,
                generation: self.amoo_ui.snapshot_generation,
            })
            .is_err()
        {
            self.amoo_ui.snapshot_worker = None;
            self.amoo_ui.snapshot_job = None;
        }
    }

    fn apply_stream_delta(&mut self, delta: amoo::StreamDelta) -> bool {
        if !delta.session_key.is_empty()
            && (self.workspace.stopped_sessions.contains(&delta.session_key)
                || self.workspace.deleted_sessions.contains(&delta.session_key))
            && !self.amoo_pending_reply.contains_key(&delta.session_key)
        {
            return false;
        }

        let session_idx = if !delta.session_key.is_empty() {
            self.amoo_sessions
                .iter()
                .position(|s| s.session_key == delta.session_key)
        } else if self.amoo_session_idx < self.amoo_sessions.len() {
            Some(self.amoo_session_idx)
        } else {
            self.amoo_sessions
                .iter()
                .position(|s| self.amoo_pending_reply.contains_key(&s.session_key))
        };

        let mut resolved_session_key = delta.session_key.clone();
        if resolved_session_key.is_empty() {
            if let Some(idx) = session_idx {
                resolved_session_key = self.amoo_sessions[idx].session_key.clone();
            }
        }
        if !resolved_session_key.is_empty()
            && (self
                .workspace
                .stopped_sessions
                .contains(&resolved_session_key)
                || self
                    .workspace
                    .deleted_sessions
                    .contains(&resolved_session_key))
            && !self.amoo_pending_reply.contains_key(&resolved_session_key)
        {
            return false;
        }

        let active_task_session = self.workspace_task().and_then(|t| t.session.as_deref());
        let is_active = !resolved_session_key.is_empty()
            && (active_task_session == Some(resolved_session_key.as_str())
                || (active_task_session.is_none()
                    && self
                        .amoo_sessions
                        .get(self.amoo_session_idx)
                        .is_some_and(|s| s.session_key == resolved_session_key)));
        if is_active {
            self.workspace.stream_delta_count = self.workspace.stream_delta_count.saturating_add(1);
            if let Some(tool) = &delta.tool_status {
                self.workspace.stream_tool_status = Some(tool.clone());
            } else if delta.finished {
                self.workspace.stream_tool_status = None;
            }
        }

        let mut updated_text: Option<String> = None;
        if let Some(idx) = session_idx {
            let session = &mut self.amoo_sessions[idx];
            let needs_new_item = !matches!(
                session.items.last(),
                Some(AmooChatItem::AssistantRich { .. })
            );
            if needs_new_item {
                session.items.push(AmooChatItem::AssistantRich {
                    thinking_time: "实时增量流".into(),
                    thinking_steps: Vec::new(),
                    tools_summary: String::new(),
                    text: Some(String::new()),
                    has_bolt_artifact: false,
                    has_invoice_artifact: false,
                    has_safety_confirm: false,
                    time_label: "实时".into(),
                });
            }
            if let Some(AmooChatItem::AssistantRich {
                thinking_steps,
                tools_summary,
                text,
                ..
            }) = session.items.last_mut()
            {
                if !delta.thinking_text.is_empty()
                    && thinking_steps.last() != Some(&delta.thinking_text)
                {
                    thinking_steps.push(delta.thinking_text.clone());
                }
                if let Some(tool) = &delta.tool_status {
                    *tools_summary = tool.clone();
                }
                let text_buf = text.get_or_insert_with(String::new);
                if !delta.full_text.is_empty() {
                    *text_buf = stepx_companion::amoo_gui_chat::display_text(&delta.full_text);
                } else if !delta.delta_text.is_empty() {
                    if !text_buf.is_empty() && delta.delta_text.starts_with(text_buf.as_str()) {
                        *text_buf = delta.delta_text.clone();
                    } else if !text_buf.ends_with(&delta.delta_text) {
                        text_buf.push_str(&delta.delta_text);
                    }
                }
                if !text_buf.trim().is_empty() {
                    updated_text = Some(text_buf.clone());
                }
            }
        } else if !delta.full_text.is_empty() {
            updated_text = Some(stepx_companion::amoo_gui_chat::display_text(
                &delta.full_text,
            ));
        } else if !delta.delta_text.is_empty() {
            updated_text = Some(delta.delta_text.clone());
        }

        if let Some(live_text) = updated_text {
            let running_task_id = self
                .workspace
                .running
                .as_ref()
                .filter(|r| r.executor == stepx_companion::amoo_workspace::Executor::Phone)
                .map(|r| r.task.clone());
            if let Some(task) = self.workspace.journal.tasks.iter_mut().find(|t| {
                if !resolved_session_key.is_empty() {
                    t.session.as_deref() == Some(resolved_session_key.as_str())
                } else {
                    running_task_id.as_deref() == Some(t.id.as_str())
                }
            }) {
                if task.turns.last().is_some_and(|t| !t.user) {
                    if let Some(last) = task.turns.last_mut() {
                        if !delta.full_text.is_empty() || live_text.len() >= last.text.len() {
                            last.text = live_text;
                        } else if !last.text.ends_with(&live_text) {
                            last.text.push_str(&live_text);
                        }
                    }
                } else {
                    task.turns.push(stepx_companion::amoo_workspace::Turn {
                        user: false,
                        text: live_text,
                    });
                }
            }
        }

        if is_active {
            self.reveal_latest_chat();
        }
        true
    }

    pub(super) fn poll_amoo_sync(&mut self) -> bool {
        let mut streamed_batches = Vec::new();
        if let Some(worker) = self.amoo_ui.snapshot_worker.as_ref() {
            while let Ok(batch) = worker.delta_rx.try_recv() {
                streamed_batches.push(batch);
            }
        }
        let mut changed = false;
        for batch in streamed_batches {
            for delta in batch {
                if self.apply_stream_delta(delta) {
                    changed = true;
                }
            }
        }

        let Some(job) = self.amoo_ui.snapshot_job.as_ref() else {
            return changed;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return changed,
            Err(mpsc::TryRecvError::Disconnected) => Err("后台同步已结束，下次自动重试".into()),
        };
        let job = self.amoo_ui.snapshot_job.take().unwrap();
        let current_key = self.current_amoo_sync_key();
        let expected_key = if job.key.is_none() {
            None
        } else {
            current_key.as_deref()
        };
        if !matches_request(
            &job,
            self.amoo_ui.snapshot_generation,
            self.selected.as_ref().map(|d| d.serial.as_str()),
            expected_key,
            false,
        ) {
            // A newer foreground operation or session switch owns the state.
            // A discarded result must not become the deduplication baseline:
            // reset the worker baseline while keeping its warm AgentBus tunnel.
            if let Some(worker) = &self.amoo_ui.snapshot_worker {
                worker
                    .reset_baseline
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if self.selected.as_ref().map(|d| d.serial.as_str()) != Some(job.serial.as_str()) {
                self.amoo_ui.snapshot_worker = None;
            }
            return changed;
        }
        match result {
            Ok(None) => return changed,
            Ok(Some(data)) => {
                let live_items: Vec<(String, AmooChatItem)> = self
                    .amoo_sessions
                    .iter()
                    .filter(|s| self.amoo_pending_reply.contains_key(&s.session_key))
                    .filter_map(|s| {
                        s.items.last().and_then(|item| match item {
                            AmooChatItem::AssistantRich { text, .. }
                                if text.as_deref().is_some_and(|t| !t.is_empty()) =>
                            {
                                Some((s.session_key.clone(), item.clone()))
                            }
                            _ => None,
                        })
                    })
                    .collect();
                let msg = if self.busy {
                    String::new()
                } else {
                    "已刷新后台对话".into()
                };
                self.apply(Outcome::AmooRealData(data, msg));
                for (session_key, live_item) in live_items {
                    if self.amoo_pending_reply.contains_key(&session_key) {
                        if let Some(session) = self
                            .amoo_sessions
                            .iter_mut()
                            .find(|s| s.session_key == session_key)
                        {
                            if !matches!(
                                session.items.last(),
                                Some(AmooChatItem::AssistantRich { .. })
                            ) {
                                session.items.push(live_item);
                            }
                        }
                    }
                }
            }
            Err(_) => self.apply(Outcome::AmooObservationUnavailable),
        }
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passive_reads_cannot_overwrite_newer_actions_devices_or_sessions() {
        let (_sender, receiver) = mpsc::channel();
        let job = SnapshotJob {
            generation: 3,
            serial: "phone-a".into(),
            key: Some("chat-a".into()),
            receiver,
        };
        assert!(matches_request(
            &job,
            3,
            Some("phone-a"),
            Some("chat-a"),
            false
        ));
        assert!(!matches_request(
            &job,
            4,
            Some("phone-a"),
            Some("chat-a"),
            false
        ));
        assert!(!matches_request(
            &job,
            3,
            Some("phone-b"),
            Some("chat-a"),
            false
        ));
        assert!(!matches_request(
            &job,
            3,
            Some("phone-a"),
            Some("chat-b"),
            false
        ));
        assert!(!matches_request(&job, 3, None, Some("chat-a"), false));
        assert!(!matches_request(
            &job,
            3,
            Some("phone-a"),
            Some("chat-a"),
            true
        ));
    }

    #[test]
    fn passive_real_amoo_sync_preserves_welcome_screen_sentinel_index() {
        let mut app = Companion::new_for_test();
        app.page = Page::Amoo;
        app.workspace.active = None;
        app.amoo_session_idx = usize::MAX;
        let real = RealAmooData {
            gateway_version: "1.0".into(),
            gateway_online: true,
            selected_session: Some("agent:main:existing".into()),
            sessions: vec![openclaw::RealSessionInfo {
                key: "agent:main:existing".into(),
                title: "已有会话".into(),
                agent: "Amoo".into(),
                status: "就绪".into(),
                history_loaded: true,
                ..Default::default()
            }],
            memory_initialized: true,
            ..Default::default()
        };
        app.apply_real_amoo_data_recorded(real);
        assert_eq!(app.amoo_session_idx, usize::MAX);
        assert_eq!(app.amoo_sessions.len(), 1);
    }

    #[test]
    fn background_non_amoo_busy_demotes_on_amoo_and_does_not_drop_sync_snapshot() {
        let mut app = Companion::new_for_test();
        app.initializing = false;
        app.selected = Some(Device {
            serial: "phone-a".into(),
            transport: "phone-a".into(),
            model: "STEPX".into(),
            android: "15".into(),
            product: "stepx".into(),
        });
        app.busy = true;
        app.job_page = Page::Overview;
        app.page = Page::Amoo;
        app.demote_busy_if_background();
        assert!(!app.busy);
        assert_eq!(app.bg_receivers.len(), 1);
    }
}
