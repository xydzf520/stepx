//! Bounded, account-bound imports of Amoo discussions not opened on desktop.
//! Other apps remain limited to the separately authorized notification/screen
//! sources; AgentBus history is not a general-purpose all-app history reader.
use crate::{adb::Adb, amoo::AgentBus, device::Device};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

const CATALOG_LIMIT: usize = 200; // verified native catalog maximum
const HISTORY_LIMIT: usize = 100;
const SESSION_BUDGET: usize = 12;
const TEXT_LIMIT: usize = 12_000;
const REFRESH_MS: i64 = 10 * 60 * 1000;
const AUX_CONTEXT_CACHE_TTL: Duration = Duration::from_secs(180);

#[derive(Clone, Debug, Default)]
struct CachedAuxPhoneContext {
    calls: Option<Vec<crate::bridge::BridgeCall>>,
    contacts: Option<Vec<crate::bridge::BridgeContact>>,
    media: Option<Vec<crate::bridge::BridgeMediaItem>>,
    status: Option<crate::bridge::BridgeStatus>,
}

static AUX_CONTEXT_CACHE: OnceLock<Mutex<HashMap<String, (Instant, CachedAuxPhoneContext)>>> =
    OnceLock::new();

fn aux_context_cache() -> &'static Mutex<HashMap<String, (Instant, CachedAuxPhoneContext)>> {
    AUX_CONTEXT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn invalidate_phone_context_cache() {
    if let Ok(mut guard) = aux_context_cache().lock() {
        guard.clear();
    }
}

fn collect_aux_phone_context(
    adb: &Adb,
    device: &Device,
    deadline: Instant,
    cancel: &AtomicBool,
) -> CachedAuxPhoneContext {
    let cache_key = if !device.serial.trim().is_empty() {
        device.serial.trim().to_string()
    } else {
        device.transport.trim().to_string()
    };
    if !cache_key.is_empty() {
        if let Ok(guard) = aux_context_cache().lock() {
            if let Some((cached_at, snapshot)) = guard.get(&cache_key) {
                if cached_at.elapsed() < AUX_CONTEXT_CACHE_TTL {
                    return snapshot.clone();
                }
            }
        }
    }
    if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
        return CachedAuxPhoneContext::default();
    }
    let transport = device.transport.trim();
    if transport.is_empty() {
        return CachedAuxPhoneContext::default();
    }

    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_secs(15))
        .max(Duration::from_secs(2));
    let batch_req = json!({
        "op": "batch",
        "ops": [
            { "id": "calls", "op": "read_calls", "limit": 20 },
            { "id": "contacts", "op": "read_contacts", "limit": 200 },
            { "id": "recordings", "op": "read_recordings" },
            { "id": "capabilities", "op": "capabilities" },
            { "id": "usage", "op": "stepos_usage" },
        ]
    });
    let find_script = "find /sdcard/Recordings /sdcard/Music/Recordings /sdcard/Sounds /sdcard/MIUI/sound_recorder /sdcard/Pictures/Screenshots /sdcard/DCIM/Screenshots /sdcard/DCIM/Camera -maxdepth 3 -type f -printf '%T@|%s|%p\\n' 2>/dev/null | sort -rn | head -n 20";

    let (batch_res, find_res) = std::thread::scope(|s| {
        let find_handle = s.spawn(move || adb.script(transport, find_script));
        let batch_res =
            crate::bridge::call_bridge_on_transport(adb, transport, &batch_req, timeout);
        let find_res = find_handle
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("media find thread panicked")));
        (batch_res, find_res)
    });

    let mut out = CachedAuxPhoneContext::default();
    if let Ok(batch_val) = batch_res {
        let results = &batch_val["results"];
        if results["contacts"]["ok"].as_bool() == Some(true) {
            if let Ok(mut list) = crate::bridge::parse_contacts(&results["contacts"]) {
                list.truncate(200);
                out.contacts = Some(list);
            }
        }
        if results["calls"]["ok"].as_bool() == Some(true) {
            if let Ok(mut list) = crate::bridge::parse_calls(&results["calls"]) {
                list.truncate(20);
                if list.iter().any(|c| c.name.trim().is_empty()) {
                    if let Some(ref contacts) = out.contacts {
                        enrich_calls_with_contacts(&mut list, contacts);
                    }
                }
                out.calls = Some(list);
            }
        }
        let mut combined_media = Vec::new();
        if results["recordings"]["ok"].as_bool() == Some(true) {
            if let Ok(rec_items) = crate::bridge::parse_media(&results["recordings"], 20) {
                combined_media.extend(rec_items);
            }
        }
        if let Ok(raw) = find_res {
            for item in crate::bridge::parse_media_find_output(&raw, 20) {
                if !combined_media
                    .iter()
                    .any(|existing: &crate::bridge::BridgeMediaItem| existing.path == item.path)
                {
                    combined_media.push(item);
                }
            }
        }
        if !combined_media.is_empty() || results["recordings"]["ok"].as_bool() == Some(true) {
            combined_media.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
            combined_media.truncate(20);
            out.media = Some(combined_media);
        }
        let usage_opt = results
            .get("usage")
            .filter(|u| u["ok"].as_bool() == Some(true));
        if results["capabilities"]["ok"].as_bool() == Some(true) {
            if let Ok(st) = crate::bridge::parse_status(&results["capabilities"], usage_opt) {
                out.status = Some(st);
            }
        } else if let Some(u) = usage_opt {
            let mut st = crate::bridge::BridgeStatus::default();
            st.clipboard = u["clipboard"].as_str().unwrap_or("").trim().to_string();
            out.status = Some(st);
        }
    } else {
        if let Ok(mut calls) = crate::bridge::calls(adb, transport, 20) {
            if calls.iter().any(|c| c.name.trim().is_empty()) && !cancel.load(Ordering::Relaxed) {
                if let Ok(contacts) = crate::bridge::contacts(adb, transport, 200) {
                    enrich_calls_with_contacts(&mut calls, &contacts);
                    out.contacts = Some(contacts);
                }
            }
            out.calls = Some(calls);
        }
        if !cancel.load(Ordering::Relaxed) && Instant::now() < deadline {
            if let Ok(media_items) = crate::bridge::media(adb, transport, 20) {
                out.media = Some(media_items);
            }
        }
        if !cancel.load(Ordering::Relaxed) && Instant::now() < deadline {
            if let Ok(status) = crate::bridge::status(adb, transport) {
                out.status = Some(status);
            }
        }
    }

    if !cache_key.is_empty()
        && (out.calls.is_some()
            || out.contacts.is_some()
            || out.media.is_some()
            || out.status.is_some())
    {
        if let Ok(mut guard) = aux_context_cache().lock() {
            guard.insert(cache_key, (Instant::now(), out.clone()));
        }
    }
    out
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneMessage {
    pub device: String,
    pub account: String,
    pub session_key: String,
    pub message_id: String,
    /// Only the session directory's title; never a model-inferred project name.
    pub title: String,
    /// `assistant` is an unverified AI suggestion, not a user decision/receipt.
    pub role: String,
    pub text: String,
    /// Identity-only invalidation when a previously imported message becomes
    /// protected/sensitive/unavailable. Never store its replacement contents.
    #[serde(default)]
    pub withdrawn: bool,
    /// Source occurrence time, never the collection time. Missing stays unknown.
    pub timestamp_ms: Option<i64>,
}

/// Scheduling metadata only: no conversation body or credentials are persisted.
/// Commit the returned cursor only after the ledger has accepted the batch.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Cursor {
    device: String,
    account: String,
    since_ms: i64,
    source_policy: u32,
    sessions: BTreeMap<String, SessionCursor>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct SessionCursor {
    source_updated_ms: i64,
    last_read_ms: i64,
    last_attempt_ms: i64,
    partial: bool,
    backscreen_protocol: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Batch {
    pub messages: Vec<PhoneMessage>,
    pub cursor: Cursor,
    pub coverage_notes: Vec<String>,
    pub partial: bool,
    pub sessions_read: usize,
    pub pending_sessions: usize,
    pub coverage_issue: Option<String>,
}

#[derive(Clone, Debug)]
struct Session {
    key: String,
    title: String,
    updated_ms: i64,
    backscreen_protocol: bool,
}

fn catalog_has_more(list: &Value, count: usize) -> bool {
    list["page"]["hasMore"]
        .as_bool()
        .unwrap_or(count >= CATALOG_LIMIT)
}

// Intentional privacy/protocol exclusions are tracked in coverage notes, never
// used to infer missing historical pages. Explicit server pagination wins.
fn history_window_incomplete(history: &Value, count: usize, truncated: usize) -> bool {
    history["page"]["hasMore"]
        .as_bool()
        .unwrap_or(count >= HISTORY_LIMIT)
        || count > HISTORY_LIMIT
        || truncated > 0
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "已停止读取手机上下文");
    Ok(())
}

fn checked_scope(adb: &Adb, device: &Device, expected: &str) -> Result<()> {
    ensure!(!expected.is_empty(), "尚未确认手机 Amoo 账号，未读取讨论");
    // Verify this exact transport, not a failover candidate that the following
    // account/history calls would not use. Wireless endpoints can be reassigned.
    let serial = adb
        .run_timeout(
            &["-s", &device.transport, "shell", "getprop", "ro.serialno"],
            Duration::from_secs(3),
        )
        .context("无法核验手机上下文的设备身份")?;
    ensure!(
        serial.trim() == device.serial && !device.serial.is_empty(),
        "手机连接指向的设备已改变，未读取讨论"
    );
    let actual = crate::amoo_usage::account_scope_fresh(adb, device)?;
    ensure!(
        actual.as_deref() == Some(expected),
        "手机 Amoo 账号已改变或无法确认，未保存本次讨论"
    );
    Ok(())
}

/// Uses the existing authorized AgentBus channel, without opening an app or
/// invoking tools. Cancellation/account changes return no data and no cursor.
pub fn collect_recent(
    adb: &Adb,
    device: &Device,
    expected_account: &str,
    since_ms: i64,
    previous: &Cursor,
    cancel: &AtomicBool,
) -> Result<Batch> {
    cancelled(cancel)?;
    checked_scope(adb, device, expected_account)?;
    cancelled(cancel)?;
    // The authenticated task connection owns the existing, explicitly enabled
    // CPU/cloud lease until this read ends; it never launches or wakes the UI.
    // A passive observer shortens screen-off RPCs to 800 ms, which is invalid
    // for this leased read even when another lease is already active.
    let mut bus = AgentBus::connect(adb, device)
        .or_else(|_| {
            cancelled(cancel)?;
            AgentBus::connect(adb, device)
        })
        .context("Amoo 后台连接暂不可用，已保留上次资料，将自动重试")?;
    cancelled(cancel)?;
    // Startup sync can contend with this read. Retry only this idempotent
    // catalogue request, on a fresh socket after a timeout taints the old one.
    let list = match bus.call_cancellable(
        "sessions.list",
        json!({"limit":CATALOG_LIMIT}),
        Duration::from_secs(8),
        cancel,
    ) {
        Ok(list) => list,
        Err(_) => {
            cancelled(cancel)?;
            let replacement =
                AgentBus::connect(adb, device).context("Amoo 会话目录连接中断，已保留上次资料")?;
            bus = replacement;
            bus.call_cancellable(
                "sessions.list",
                json!({"limit":CATALOG_LIMIT}),
                Duration::from_secs(8),
                cancel,
            )
            .context("Amoo 会话目录读取未完成，已保留上次资料，将自动重试")?
        }
    };
    let deadline = Instant::now() + Duration::from_secs(25);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let rows = list["sessions"]
        .as_array()
        .context("手机尚未返回 Amoo 会话目录")?;
    let (mut sessions, catalog_omitted) = parse_sessions(rows, since_ms);
    let owned = crate::inference_sessions::known(
        &crate::inference_sessions::default_context_root(),
        &device.serial,
        Some(expected_account),
    )?;
    sessions.retain(|session| !owned.contains(&session.key));
    let mut cursor = if previous.device == device.serial
        && previous.account == expected_account
        && previous.since_ms == since_ms
        && previous.source_policy == 2
    {
        previous.clone()
    } else {
        Cursor {
            device: device.serial.clone(),
            account: expected_account.into(),
            since_ms,
            source_policy: 2,
            ..Default::default()
        }
    };
    let keys: BTreeSet<_> = sessions.iter().map(|s| s.key.clone()).collect();
    cursor.sessions.retain(|key, _| keys.contains(key));
    let pending = pending_sessions(&sessions, &cursor, now_ms);
    let mut batch = Batch::default();
    // The inbox receives only bounded metadata after the final scope check.
    // Discovering a recording or screenshot does not authorize content parsing.
    let mut inbox_media = Vec::new();
    let catalog_partial = catalog_omitted || catalog_has_more(&list, rows.len());
    if catalog_partial {
        batch
            .coverage_notes
            .push("Amoo 会话目录仅覆盖最近 200 个可读取会话，不能代表全部历史。".into());
    }
    let mut incomplete = false;
    let mut skipped = 0;
    let mut unknown_time = 0;
    let mut text_truncated = 0;
    let pending_count = pending.len();
    for mut session in pending.into_iter().take(SESSION_BUDGET) {
        cancelled(cancel)?;
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            incomplete = true;
            break;
        };
        if remaining < Duration::from_millis(500) {
            incomplete = true;
            break;
        }
        // An attempt marker rotates failing/large sessions fairly. It is not a
        // history watermark and never records a failed read as complete.
        cursor
            .sessions
            .entry(session.key.clone())
            .or_default()
            .last_attempt_ms = now_ms;
        let history = match bus.call_cancellable(
            "chat.history",
            json!({"sessionId":session.key,"limit":HISTORY_LIMIT}),
            remaining.min(Duration::from_secs(5)),
            cancel,
        ) {
            Ok(value) => value,
            Err(_) => {
                cancelled(cancel)?;
                incomplete = true;
                break;
            }
        };
        let Some(rows) = history["messages"].as_array() else {
            incomplete = true;
            break;
        };
        session.backscreen_protocol = cursor
            .sessions
            .get(&session.key)
            .is_some_and(|s| s.backscreen_protocol);
        let page = parse_history(
            rows,
            &session,
            &device.serial,
            expected_account,
            since_ms,
            now_ms,
        );
        let history_partial = history_window_incomplete(&history, rows.len(), page.truncated);
        skipped += page.omitted;
        unknown_time += page.unknown_time;
        text_truncated += page.truncated;
        batch.messages.extend(page.messages);
        batch.sessions_read += 1;
        cursor.sessions.insert(
            session.key.clone(),
            SessionCursor {
                source_updated_ms: session.updated_ms,
                last_read_ms: now_ms,
                last_attempt_ms: now_ms,
                partial: history_partial,
                backscreen_protocol: page.backscreen_protocol,
            },
        );
    }
    cancelled(cancel)?;
    if Instant::now() < deadline {
        let aux = collect_aux_phone_context(adb, device, deadline, cancel);
        if let Some(calls) = aux.calls {
            let call_msgs =
                calls_to_phone_messages(&calls, &device.serial, expected_account, since_ms);
            if !call_msgs.is_empty() {
                batch
                    .coverage_notes
                    .push(format!("已纳入通话记录 {} 条", call_msgs.len()));
                batch.messages.extend(call_msgs);
            }
        }
        if let Some(media_items) = aux.media {
            inbox_media = media_items.clone();
            let (media_msgs, rec_count, shot_count) = media_clues_to_phone_messages(
                &media_items,
                &device.serial,
                expected_account,
                since_ms,
            );
            if rec_count > 0 {
                batch
                    .coverage_notes
                    .push(format!("已发现手机录音 {rec_count} 条"));
            }
            if shot_count > 0 {
                batch
                    .coverage_notes
                    .push(format!("已发现手机截屏 {shot_count} 条"));
            }
            batch.messages.extend(media_msgs);
        }
        if let Some(status) = aux.status {
            if !status.clipboard.trim().is_empty() {
                let express_msgs = express_clues_to_phone_messages(
                    &[status.clipboard.as_str()],
                    &device.serial,
                    expected_account,
                    Some(now_ms),
                );
                if !express_msgs.is_empty() {
                    batch
                        .coverage_notes
                        .push(format!("已发现快递线索 {} 条", express_msgs.len()));
                    batch.messages.extend(express_msgs);
                }
            }
        }
    }
    cancelled(cancel)?;
    // This intentionally bypasses account_scope's UI cache even for fast reads.
    checked_scope(adb, device, expected_account)?;
    cancelled(cancel)?;
    if !inbox_media.is_empty() {
        let scope = crate::material_inbox::Scope {
            device: device.serial.clone(),
            account: expected_account.to_owned(),
        };
        if crate::material_inbox::remember_media(&scope, &inbox_media).is_err() {
            batch
                .coverage_notes
                .push("发现了手机资料线索，但收件箱记录未能保存；未读取或解析文件内容。".into());
        }
    }
    let deferred = pending_count.saturating_sub(batch.sessions_read);
    // Background maintenance reads are not an endlessly growing history backlog.
    batch.pending_sessions = sessions
        .iter()
        .filter(|session| {
            cursor.sessions.get(&session.key).is_none_or(|state| {
                state.last_read_ms == 0 || state.source_updated_ms != session.updated_ms
            })
        })
        .count();
    if deferred > 0 || incomplete {
        batch.coverage_notes.push(format!("本次读取 {} 个 Amoo 会话；{} 个待更新会话尚未完成，将在后续整理重试。断线或未读不表示没有讨论。", batch.sessions_read, deferred));
    }
    if cursor.sessions.values().any(|s| s.partial) {
        batch.coverage_notes.push(
            "部分 Amoo 会话仅读取最近 100 条消息，或含不可整理的消息；更早内容可能未覆盖。".into(),
        );
    }
    if skipped > 0 {
        batch.coverage_notes.push(format!("本次未导入 {skipped} 条缺少稳定标识、受保护、敏感或内部运行协议消息；不据此推断其内容。"));
    }
    if unknown_time > 0 {
        batch.coverage_notes.push(format!(
            "{unknown_time} 条 Amoo 消息缺少可靠发生时间，不会把本次读取时间当作讨论发生时间。"
        ));
    }
    if text_truncated > 0 {
        batch.coverage_notes.push(format!(
            "{text_truncated} 条超长 Amoo 消息已明确截断，结论需回原会话核对。"
        ));
    }
    batch.coverage_notes.push("Amoo 讨论来自已登录账号的可读取会话；其他 App 仍仅覆盖已授权的通知和已捕获上下文，AI 回复不作为用户事实。".into());
    batch.partial = catalog_partial
        || deferred > 0
        || incomplete
        || cursor.sessions.values().any(|s| s.partial);
    // Window limits and intentionally excluded internal/sensitive messages are
    // recorded in source coverage, not represented as failed live synchronization.
    batch.coverage_issue = incomplete
        .then(|| format!("Amoo 本轮读取未完成，{deferred} 个会话待重试；已收到的资料仍可用"));
    batch.cursor = cursor;
    Ok(batch)
}

fn normalize_phone_digits(raw: &str) -> String {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() > 11 && digits.starts_with("86") {
        digits[digits.len() - 11..].to_string()
    } else {
        digits
    }
}

pub fn enrich_calls_with_contacts(
    calls: &mut [crate::bridge::BridgeCall],
    contacts: &[crate::bridge::BridgeContact],
) {
    for call in calls.iter_mut() {
        if !call.name.trim().is_empty() {
            continue;
        }
        let call_norm = normalize_phone_digits(&call.number);
        if call_norm.len() < 5 {
            continue;
        }
        if let Some(found) = contacts.iter().find(|c| {
            !c.display_name.trim().is_empty() && normalize_phone_digits(&c.number) == call_norm
        }) {
            call.name = found.display_name.trim().to_string();
        }
    }
}

pub fn extract_express_tracking_token(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || contains_secret_or_otp(trimmed) {
        return None;
    }
    for token in trimmed.split(|c: char| !c.is_ascii_alphanumeric() && c != '-') {
        let up = token.trim().to_ascii_uppercase();
        if (up.starts_with("SF")
            || up.starts_with("JD")
            || up.starts_with("YT")
            || up.starts_with("JT")
            || up.starts_with("ZTO")
            || up.starts_with("STO")
            || up.starts_with("YD")
            || up.starts_with("EMS"))
            && up.len() >= 10
            && up.len() <= 24
            && up.chars().filter(|c| c.is_ascii_digit()).count() >= 8
        {
            return Some(up);
        }
    }
    let has_express_keyword = [
        "取件码",
        "驿站",
        "丰巢",
        "快递",
        "快件",
        "包裹",
        "顺丰",
        "京东快递",
        "中通",
        "圆通",
        "韵达",
        "申通",
        "极兔",
        "菜鸟",
    ]
    .iter()
    .any(|kw| trimmed.contains(kw));
    if has_express_keyword {
        for token in trimmed.split(|c: char| !c.is_ascii_alphanumeric() && c != '-') {
            let t = token.trim();
            if t.len() >= 4
                && t.len() <= 20
                && t.chars().any(|c| c.is_ascii_digit())
                && (t.contains('-') || t.len() >= 10)
            {
                return Some(t.to_ascii_uppercase());
            }
        }
    }
    None
}

pub fn express_clues_to_phone_messages(
    texts: &[&str],
    device: &str,
    account: &str,
    timestamp_ms: Option<i64>,
) -> Vec<PhoneMessage> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in texts {
        let Some(code) = extract_express_tracking_token(raw) else {
            continue;
        };
        if !seen.insert(code.clone()) {
            continue;
        }
        let snippet: String = raw
            .trim()
            .chars()
            .filter(|c| !c.is_control() || *c == ' ')
            .take(160)
            .collect();
        out.push(PhoneMessage {
            device: device.to_string(),
            account: account.to_string(),
            session_key: format!("phone-express:{code}"),
            message_id: format!("express-{code}"),
            title: format!("快递与取件线索 · {code}"),
            role: "user".to_string(),
            text: format!(
                "发现手机快递/取件线索「{code}」（内容：{snippet}），可核对物流状态或安排取件。"
            ),
            withdrawn: false,
            timestamp_ms: timestamp_ms.filter(|&v| v > 0),
        });
    }
    out
}

pub fn calls_to_phone_messages(
    calls: &[crate::bridge::BridgeCall],
    device: &str,
    account: &str,
    since_ms: i64,
) -> Vec<PhoneMessage> {
    let mut out = Vec::new();
    for call in calls.iter().take(20) {
        if since_ms > 0 && call.date_ms > 0 && call.date_ms < since_ms {
            continue;
        }
        if !call.is_missed() && !call.is_long_business_call() {
            continue;
        }
        let peer_key = if !call.number.trim().is_empty() {
            call.number.trim().to_string()
        } else if !call.name.trim().is_empty() {
            call.name.trim().to_string()
        } else {
            format!("unknown-{}", call.date_ms)
        };
        let peer_display = call.display_peer();
        let type_label = call.type_label();
        let text = if call.is_missed() {
            format!("收到来自 {peer_display} 的{type_label}，是否已回拨或跟进尚待核对。")
        } else {
            format!(
                "与 {peer_display} 进行了 {} 秒的{type_label}，可整理通话要点与待办跟进。",
                call.duration_secs
            )
        };
        out.push(PhoneMessage {
            device: device.to_string(),
            account: account.to_string(),
            session_key: format!("phone-call:{peer_key}"),
            message_id: format!("call-{}-{}", call.date_ms, call.call_type),
            title: format!("{type_label} · {peer_display}"),
            role: "user".to_string(),
            text,
            withdrawn: false,
            timestamp_ms: (call.date_ms > 0).then_some(call.date_ms),
        });
    }
    out
}

pub fn media_clues_to_phone_messages(
    media: &[crate::bridge::BridgeMediaItem],
    device: &str,
    account: &str,
    since_ms: i64,
) -> (Vec<PhoneMessage>, usize, usize) {
    let mut out = Vec::new();
    let mut recordings = 0usize;
    let mut screenshots = 0usize;
    for item in media.iter().take(20) {
        if since_ms > 0 && item.modified_ms > 0 && item.modified_ms < since_ms {
            continue;
        }
        match item.kind.as_str() {
            "recording" => {
                recordings += 1;
                out.push(PhoneMessage {
                    device: device.to_string(),
                    account: account.to_string(),
                    session_key: format!("phone-recording:{}", item.name),
                    message_id: format!("rec-{}-{}", item.modified_ms, item.bytes),
                    title: format!("手机录音 · {}", item.name),
                    role: "user".to_string(),
                    text: format!(
                        "发现手机录音文件「{}」（路径：{}），目前仅有文件元数据，录音内容与是否需要整理尚待核对。",
                        item.name, item.path
                    ),
                    withdrawn: false,
                    timestamp_ms: (item.modified_ms > 0).then_some(item.modified_ms),
                });
            }
            "screenshot" => {
                screenshots += 1;
                out.push(PhoneMessage {
                    device: device.to_string(),
                    account: account.to_string(),
                    session_key: format!("phone-screenshot:{}", item.name),
                    message_id: format!("shot-{}-{}", item.modified_ms, item.bytes),
                    title: format!("手机截屏线索 · {}", item.name),
                    role: "user".to_string(),
                    text: format!(
                        "发现手机近期截屏「{}」（路径：{}），可核对截屏中的待办、单号或会议线索。",
                        item.name, item.path
                    ),
                    withdrawn: false,
                    timestamp_ms: (item.modified_ms > 0).then_some(item.modified_ms),
                });
            }
            _ => {}
        }
    }
    (out, recordings, screenshots)
}

fn stable_id(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
}

fn protected(value: &Value) -> bool {
    [
        "sensitive",
        "isSensitive",
        "protected",
        "isProtected",
        "redacted",
        "isRedacted",
    ]
    .iter()
    .any(|key| value[*key].as_bool() == Some(true))
}

#[inline]
pub(crate) fn contains_ascii_ignore_case(haystack: &str, needle: &str) -> bool {
    crate::home_hub::contains_ascii_ignore_case(haystack, needle.as_bytes())
}

pub(crate) fn contains_secret_or_otp(text: &str) -> bool {
    ["验证码", "动态密码", "动态口令", "一次性密码"]
        .iter()
        .any(|word| text.contains(word))
        || [
            "verification code",
            "one-time code",
            "one time password",
            "passcode",
            " otp ",
        ]
        .iter()
        .any(|word| contains_ascii_ignore_case(text, word))
        || crate::push::extract_otp_code(text).is_some()
}

#[inline]
fn secret(text: &str) -> bool {
    contains_secret_or_otp(text)
}

fn parse_sessions(rows: &[Value], since_ms: i64) -> (Vec<Session>, bool) {
    let mut result = BTreeMap::new();
    let mut omitted = false;
    for row in rows.iter().take(CATALOG_LIMIT) {
        let Some(key) = stable_id(&row["key"]) else {
            omitted = true;
            continue;
        };
        if protected(row) {
            omitted = true;
            continue;
        }
        let updated_ms = row["updatedAtMs"].as_i64().unwrap_or(0);
        if updated_ms > 0 && updated_ms < since_ms {
            continue;
        }
        let title = ["title", "displayName", "name"]
            .iter()
            .find_map(|field| row[*field].as_str().filter(|s| !s.trim().is_empty()))
            .unwrap_or("未命名 Amoo 讨论");
        if secret(title) {
            omitted = true;
            continue;
        }
        let title = title
            .chars()
            .filter(|c| !c.is_control())
            .take(160)
            .collect();
        result.insert(
            key.to_owned(),
            Session {
                key: key.into(),
                title,
                updated_ms,
                backscreen_protocol: false,
            },
        );
    }
    (result.into_values().collect(), omitted)
}

fn pending_sessions(sessions: &[Session], cursor: &Cursor, now_ms: i64) -> Vec<Session> {
    let mut pending: Vec<_> = sessions
        .iter()
        .filter(|session| {
            cursor.sessions.get(&session.key).is_none_or(|state| {
                state.last_read_ms == 0
                    || state.source_updated_ms != session.updated_ms
                    || session.updated_ms <= 0
                    || now_ms.saturating_sub(state.last_read_ms) >= REFRESH_MS
            })
        })
        .cloned()
        .collect();
    // Oldest attempted first prevents a busy newest session starving the rest.
    pending.sort_by(|a, b| {
        let at = |s: &Session| {
            cursor
                .sessions
                .get(&s.key)
                .map(|s| s.last_attempt_ms)
                .unwrap_or(0)
        };
        at(a)
            .cmp(&at(b))
            .then_with(|| b.updated_ms.cmp(&a.updated_ms))
            .then_with(|| a.key.cmp(&b.key))
    });
    pending
}

#[derive(Default)]
struct HistoryPage {
    messages: Vec<PhoneMessage>,
    omitted: usize,
    unknown_time: usize,
    truncated: usize,
    backscreen_protocol: bool,
}

fn raw_text(row: &Value) -> String {
    if let Some(text) = row["text"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| row["content"].as_str())
    {
        return text.into();
    }
    row["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["type"] == "text")
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Recognizes StepX's fixed generators, not arbitrary JSON or image requests.
/// Kept public within the crate so old ledger records use the same source policy.
pub(crate) fn is_backscreen_protocol(text: &str) -> bool {
    let text = text.trim();
    (text.starts_with("你是 StepX 背屏设计助手。请仅输出一个 JSON 对象，不要 Markdown，不要调用工具，不要操作手机。")
        && text.contains("硬件：单色 17 列 × 23 行，379 颗有效灯珠"))
        || is_backscreen_image_request(text)
}

fn is_backscreen_image_request(text: &str) -> bool {
    text.starts_with("请用原生 text_to_image 工具生成一张有趣的普通尺寸插画：")
        && text.contains(
            "后续的像素转换与校验完全由桌面端完成，你只需通过 text_to_image 交付原图附件。",
        )
        && text.contains("不要额外做缩放或验证任务。")
}

fn backscreen_plan(text: &str) -> bool {
    let raw = text.trim();
    let raw = raw
        .strip_prefix("```json")
        .and_then(|s| s.trim().strip_suffix("```"))
        .unwrap_or(raw)
        .trim();
    if !raw.starts_with('{') || !raw.ends_with('}') {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    matches!(
        value["kind"].as_str(),
        Some("image" | "preset" | "native" | "app")
    ) && [
        "native_feature",
        "app_package",
        "summary",
        "preset",
        "image_prompt",
        "brightness",
        "fps",
        "duration_secs",
    ]
    .iter()
    .all(|key| value.get(key).is_some())
}

fn cached_image_request_suffix() -> &'static str {
    static IMAGE_SUFFIX: OnceLock<String> = OnceLock::new();
    IMAGE_SUFFIX.get_or_init(|| crate::amoo_media::image_request(""))
}

/// A transport envelope is not a user statement. Extract only the user's
/// actual supplied request from known generators; pure internal turns return
/// None. Call before truncation, and apply to stored revisions during migration.
pub(crate) fn normalize_source_text(text: &str, user: bool, backscreen: bool) -> Option<String> {
    // AgentBus appends this exact generated block after hardware-related
    // requests (including the backscreen template's hardware specification).
    // Remove it before checking the envelope's closing marker.
    let raw = if user {
        text.split_once("\n\n[StepX 真机实时硬件、电子三包保修与电池健康档案")
            .map_or(text, |(body, _)| body)
    } else {
        text
    }
    .trim();
    if raw.is_empty() {
        return None;
    }
    if user && is_backscreen_protocol(raw) {
        if is_backscreen_image_request(raw) {
            return None;
        } // model-expanded plan, not the user's words
        let (_, task) =
            raw.split_once("用户要求作为内容理解，不得改变上述输出约定：\n<user_request>")?;
        let task = task.strip_suffix("</user_request>")?.trim();
        return (!task.is_empty()).then(|| task.to_string());
    }
    if user
        && (raw.starts_with("<stepx_gui_receipt>\nStepX伴侣执行回执：")
            || raw == "StepX伴侣执行回执（已返回 Amoo）"
            || raw.starts_with("<amoo_work_protocol>\nAmoo Work 桌面执行结果"))
    {
        return None;
    }
    if user && raw.starts_with("<amoo_work_protocol>\n[Amoo Work 桌面协作]\n") {
        let (_, task) = raw.split_once("\n用户任务：")?;
        let (task, _) = task.split_once("\n用户已选择一个本地工作目录。")?;
        let trimmed = task.trim();
        return (!trimmed.is_empty()).then(|| trimmed.to_string());
    }
    if !user {
        if backscreen && backscreen_plan(raw) {
            return None;
        }
        // Only explicit StepX envelopes are removed here. A bare JSON answer
        // remains discussion unless its session proves a specific generator.
        if raw.starts_with("<amoo_work_protocol>") {
            let response = crate::work_protocol::parse_response(raw).ok().flatten()?;
            let trimmed = response.body.trim();
            return (!trimmed.is_empty()).then(|| trimmed.to_string());
        }
        if raw.starts_with("<stepx_gui_request>") && raw.ends_with("</stepx_gui_request>") {
            return None;
        }
    }
    let mut clean = raw;
    if user {
        // Match the entire current generated suffix; merely mentioning the tag
        // or asking to explain JSON must not erase user-authored discussion.
        const UPLOAD: &str = "\n<amoo_work_upload_instructions>图片已通过原生图片附件提交，请直接看图分析，不要用 exec、OCR 或读取二进制来替代看图。其他类型附件已上传到手机；devicePath 是手机路径，可用手机文件读取能力处理。只处理所列附件，不要搜索其他目录。</amoo_work_upload_instructions>";
        if let Some((body, tail)) = clean.split_once(UPLOAD) {
            if tail.is_empty() || tail.trim_start().starts_with("<extra>") {
                clean = body.trim();
            }
        }
        let image_suffix = cached_image_request_suffix();
        if let Some(body) = clean.strip_suffix(image_suffix) {
            clean = body.trim();
        }
        if let Some((body, context)) = clean.split_once("\n<stepx_gui_context>\n本次对话已连接 StepX伴侣 GUI 工具，具备真实看屏、点击、滑动、中文输入、重试和停止能力。") {
            if context.trim_end().ends_with("</stepx_gui_context>") {
                clean = body.trim();
            }
        }
    }
    let trimmed = clean.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(crate) fn normalize_source_title(title: &str, backscreen: bool) -> String {
    if backscreen && title.trim_start().starts_with('{') {
        "背屏创作讨论".into()
    } else {
        title.into()
    }
}

/// A known hidden/test session is not reintroduced through raw phone discovery.
/// Identity-only withdrawals also remove an earlier imported copy. The source
/// Amoo conversation is never modified.
pub fn exclude_sessions(batch: &mut Batch, sessions: &BTreeSet<String>) {
    for message in &mut batch.messages {
        if sessions.contains(&message.session_key) {
            message.withdrawn = true;
            message.text.clear();
            message.timestamp_ms = None;
            message.title = "已排除的 Amoo 会话".into();
        }
    }
}

fn parse_history(
    rows: &[Value],
    session: &Session,
    device: &str,
    account: &str,
    since_ms: i64,
    now_ms: i64,
) -> HistoryPage {
    let mut page = HistoryPage::default();
    page.backscreen_protocol = session.backscreen_protocol
        || rows
            .iter()
            .take(HISTORY_LIMIT)
            .any(|row| row["role"] == "user" && is_backscreen_protocol(&raw_text(row)));
    let session_title = normalize_source_title(&session.title, page.backscreen_protocol);
    let mut messages = BTreeMap::new();
    let mut conflicting = BTreeSet::new();
    for row in rows.iter().take(HISTORY_LIMIT) {
        if !matches!(row["role"].as_str(), Some("user" | "assistant")) {
            continue;
        }
        let Some(id) = stable_id(&row["id"]).or_else(|| stable_id(&row["responseId"])) else {
            page.omitted += 1;
            continue;
        };
        let withdrawal = || PhoneMessage {
            device: device.into(),
            account: account.into(),
            session_key: session.key.clone(),
            message_id: id.into(),
            title: "内容不可用的 Amoo 消息".into(),
            role: row["role"].as_str().unwrap_or("user").into(),
            withdrawn: true,
            ..Default::default()
        };
        if protected(row)
            || row["content"]
                .as_array()
                .is_some_and(|parts| parts.iter().any(protected))
        {
            messages.insert(id.to_owned(), withdrawal());
            page.omitted += 1;
            continue;
        }
        // Reuse known text/attachment/context cleanup, but only retain the
        // original top-level message. Tool results appended by this parser are
        // never imported as if the user/assistant had said them.
        let raw = raw_text(row);
        let Some(normalized) =
            normalize_source_text(&raw, row["role"] == "user", page.backscreen_protocol)
        else {
            messages.insert(id.to_owned(), withdrawal());
            page.omitted += 1;
            continue;
        };
        let parsed = if row["text"].as_str() == Some(normalized.as_str()) {
            crate::amoo::parse_messages(std::slice::from_ref(row))
        } else {
            let mut cleaned_row = row.clone();
            cleaned_row["text"] = json!(normalized);
            crate::amoo::parse_messages(std::slice::from_ref(&cleaned_row))
        };
        let Some(message) = parsed.ok().and_then(|v| v.into_iter().next()) else {
            messages.insert(id.to_owned(), withdrawal());
            continue;
        };
        if message.text.trim().is_empty() || message.is_error {
            messages.insert(id.to_owned(), withdrawal());
            continue;
        }
        if secret(&message.text)
            || ["text", "content"].iter().any(|key| {
                row[*key]
                    .as_str()
                    .is_some_and(|s| s.starts_with("<stepx_gui_receipt>"))
            })
        {
            messages.insert(id.to_owned(), withdrawal());
            page.omitted += 1;
            continue;
        }
        let at = (message.timestamp_ms >= 946_684_800_000
            && message.timestamp_ms <= now_ms.saturating_add(300_000))
        .then_some(message.timestamp_ms);
        if at.is_some_and(|time| since_ms > 0 && time < since_ms) {
            continue;
        }
        if at.is_none() {
            page.unknown_time += 1;
        }
        let mut text = String::with_capacity(message.text.len().min(TEXT_LIMIT * 3));
        let mut kept = 0usize;
        let mut total_chars = 0usize;
        for c in message.text.chars() {
            total_chars += 1;
            if kept < TEXT_LIMIT {
                if !c.is_control() || matches!(c, '\n' | '\t') {
                    text.push(c);
                    kept += 1;
                }
            } else if total_chars > TEXT_LIMIT {
                break;
            }
        }
        if total_chars > TEXT_LIMIT {
            text.push_str("\n〔原消息过长，后文未读取；需要回原会话核对〕");
            page.truncated += 1;
        }
        let message = PhoneMessage {
            device: device.into(),
            account: account.into(),
            session_key: session.key.clone(),
            message_id: id.into(),
            title: session_title.clone(),
            role: message.role,
            text,
            timestamp_ms: at,
            withdrawn: false,
        };
        if messages.get(id).is_some_and(|prior| prior != &message) {
            conflicting.insert(id.to_owned());
        }
        messages.insert(id.to_owned(), message);
    }
    for id in conflicting {
        if let Some(message) = messages.get_mut(&id) {
            message.withdrawn = true;
            message.text.clear();
            message.timestamp_ms = None;
            message.title = "身份冲突的 Amoo 消息".into();
        }
        page.omitted += 1;
    }
    page.messages = messages.into_values().collect();
    page.messages.sort_by(|a, b| {
        a.timestamp_ms
            .cmp(&b.timestamp_ms)
            .then_with(|| a.message_id.cmp(&b.message_id))
    });
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    const NOW: i64 = 1_793_400_000_000;
    fn session() -> Session {
        Session {
            key: "session-1".into(),
            title: "社团活动".into(),
            updated_ms: NOW,
            backscreen_protocol: false,
        }
    }
    fn parse(rows: Vec<Value>) -> HistoryPage {
        parse_history(&rows, &session(), "phone", "account", 0, NOW)
    }

    #[test]
    fn complete_page_and_intentional_exclusions_do_not_become_missing_history() {
        let complete = json!({"page":{"hasMore":false}});
        assert!(!history_window_incomplete(&complete, HISTORY_LIMIT, 0));
        assert!(history_window_incomplete(&complete, 4, 1));
        assert!(history_window_incomplete(
            &json!({"page":{"hasMore":true}}),
            1,
            0
        ));
        assert!(history_window_incomplete(&json!({}), HISTORY_LIMIT, 0));
    }

    #[test]
    fn explicit_complete_catalog_at_limit_does_not_warn() {
        assert!(!catalog_has_more(
            &json!({"page":{"hasMore":false}}),
            CATALOG_LIMIT
        ));
        assert!(catalog_has_more(&json!({"page":{"hasMore":true}}), 1));
        assert!(catalog_has_more(&json!({}), CATALOG_LIMIT));
    }

    #[test]
    fn source_ids_survive_reordering_and_content_revision() {
        let a = json!({"id":"a","role":"user","text":"周五活动预算600元","timestamp":NOW-20});
        let b = json!({"id":"b","role":"user","text":"预算改成480元","timestamp":NOW-10});
        assert_eq!(
            parse(vec![a.clone(), b.clone()]).messages,
            parse(vec![b, a.clone(), a]).messages
        );
        let changed = parse(vec![
            json!({"id":"a","role":"user","text":"已更正预算","timestamp":NOW-20}),
        ]);
        assert_eq!(changed.messages[0].message_id, "a");
        assert_eq!(changed.messages[0].session_key, "session-1");
    }

    #[test]
    fn unknown_time_and_ai_authorship_are_preserved() {
        let page = parse(vec![
            json!({"id":"a","role":"assistant","text":"建议由小李负责"}),
            json!({"id":"b","role":"user","text":"已确定活动","timestamp":NOW+600_000}),
        ]);
        assert!(page.messages.iter().all(|m| m.timestamp_ms.is_none()));
        assert_eq!(page.messages[0].role, "assistant");
        assert_eq!(page.unknown_time, 2);
    }

    #[test]
    fn sensitive_protected_missing_identity_and_tool_rows_are_not_imported() {
        let page = parse(vec![
            json!({"id":"otp","role":"user","text":"验证码123456"}),
            json!({"id":"private","role":"user","text":"受保护内容","sensitive":true}),
            json!({"id":"part","role":"user","content":[{"type":"text","text":"secret","protected":true}]}),
            json!({"role":"user","text":"无身份"}),
            json!({"id":"tool","role":"tool","text":"已完成"}),
        ]);
        assert!(page
            .messages
            .iter()
            .all(|m| m.withdrawn && m.text.is_empty()));
        assert_eq!(page.messages.len(), 3);
        assert_eq!(page.omitted, 4);
    }

    #[test]
    fn conflicting_duplicate_identity_is_not_arbitrarily_accepted() {
        let page = parse(vec![
            json!({"id":"same","role":"user","text":"600元"}),
            json!({"id":"same","role":"user","text":"480元"}),
        ]);
        assert_eq!(page.messages.len(), 1);
        assert!(page.messages[0].withdrawn && page.messages[0].text.is_empty());
        assert_eq!(page.omitted, 1);
    }

    #[test]
    fn busy_recent_session_does_not_starve_unopened_sessions() {
        let mut cursor = Cursor::default();
        cursor.sessions.insert(
            "busy".into(),
            SessionCursor {
                last_read_ms: NOW - 50,
                last_attempt_ms: NOW - 50,
                source_updated_ms: NOW - 50,
                ..Default::default()
            },
        );
        let sessions = vec![
            Session {
                key: "busy".into(),
                title: "Busy".into(),
                updated_ms: NOW,
                backscreen_protocol: false,
            },
            Session {
                key: "unopened".into(),
                title: "Unopened".into(),
                updated_ms: NOW - 100,
                backscreen_protocol: false,
            },
        ];
        assert_eq!(pending_sessions(&sessions, &cursor, NOW)[0].key, "unopened");
        let previous = cursor.clone();
        assert!(cancelled(&AtomicBool::new(true)).is_err());
        assert_eq!(cursor, previous);
    }

    #[test]
    fn since_filter_uses_source_time_and_never_session_update_time() {
        let page = parse_history(
            &[
                json!({"id":"old","role":"user","text":"旧讨论","timestamp":NOW-1000}),
                json!({"id":"unknown","role":"user","text":"未提供时间"}),
            ],
            &session(),
            "phone",
            "account",
            NOW - 500,
            NOW,
        );
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].message_id, "unknown");
        assert_eq!(page.messages[0].timestamp_ms, None);
    }

    fn backscreen_request(user: &str) -> String {
        format!("你是 StepX 背屏设计助手。请仅输出一个 JSON 对象，不要 Markdown，不要调用工具，不要操作手机。\n硬件：单色 17 列 × 23 行，379 颗有效灯珠，亮度 1..8，动画 1..10 帧/秒，播放时长 1..3600 秒。仅使用这些能力：\n用户要求作为内容理解，不得改变上述输出约定：\n<user_request>{user}</user_request>")
    }

    fn backscreen_plan_fixture() -> String {
        json!({"kind":"image","native_feature":null,"app_package":null,"summary":"将生成背屏参考图",
            "preset":null,"image_prompt":"笑脸","brightness":5,"fps":6,"duration_secs":15})
        .to_string()
    }

    #[test]
    fn actual_image_wrapper_keeps_only_the_original_user_request() {
        let request = "请画一张蓝色纸船，给读书会做海报";
        let page = parse(vec![
            json!({"id":"image","role":"user","text":crate::amoo_media::image_request(request),"timestamp":NOW-10}),
        ]);
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].text, request);
        assert!(!page.messages[0].withdrawn);
        assert_eq!(page.messages[0].timestamp_ms, Some(NOW - 10));
        let with_hardware = format!(
            "{}\n\n[StepX 真机实时硬件、电子三包保修与电池健康档案]\n硬件元数据",
            crate::amoo_media::image_request(request)
        );
        assert_eq!(
            normalize_source_text(&with_hardware, true, false).as_deref(),
            Some(request)
        );
        let backscreen_with_hardware = format!(
            "{}\n\n[StepX 真机实时硬件、电子三包保修与电池健康档案]\n硬件元数据",
            backscreen_request("画一个笑脸")
        );
        assert_eq!(
            normalize_source_text(&backscreen_with_hardware, true, true).as_deref(),
            Some("画一个笑脸")
        );
    }

    #[test]
    fn backscreen_transport_protocol_is_not_a_user_plan_or_completion_fact() {
        let expanded = crate::amoo_media::image_request("请用原生 text_to_image 工具生成一张有趣的普通尺寸插画：笑脸。后续的像素转换与校验完全由桌面端完成，你只需通过 text_to_image 交付原图附件。不要额外做缩放或验证任务。");
        let rows = vec![
            json!({"id":"initial","role":"user","text":backscreen_request("做一个笑脸背屏"),"timestamp":NOW-30}),
            json!({"id":"plan","role":"assistant","text":backscreen_plan_fixture(),"timestamp":NOW-20}),
            json!({"id":"expanded","role":"user","text":expanded,"timestamp":NOW-10}),
        ];
        let s = Session {
            title: "{\"kind\":\"image\",\"native_feature\"".into(),
            ..session()
        };
        let page = parse_history(&rows, &s, "phone", "account", 0, NOW);
        let source = page
            .messages
            .iter()
            .find(|m| m.message_id == "initial")
            .unwrap();
        assert_eq!(source.text, "做一个笑脸背屏");
        assert_eq!(source.title, "背屏创作讨论");
        assert!(!source.withdrawn);
        for id in ["plan", "expanded"] {
            let message = page.messages.iter().find(|m| m.message_id == id).unwrap();
            assert!(message.withdrawn && message.text.is_empty());
            assert_eq!(message.session_key, "session-1");
        }
        assert!(page.backscreen_protocol);
    }

    #[test]
    fn ordinary_json_questions_and_image_requests_are_preserved() {
        let requests = [
            "请解释 {\"kind\":\"image\",\"native_feature\":null} 这个 JSON 为什么报错？",
            "请生成一张像素风格的笑脸图，用于活动海报",
            "请解释 <amoo_work_image_instructions> 这个标签的含义",
            "{\"kind\":\"image\",\"native_feature\":null}",
        ];
        for request in requests {
            assert_eq!(
                normalize_source_text(request, true, false).as_deref(),
                Some(request)
            );
            assert_eq!(
                normalize_source_text(request, true, true).as_deref(),
                Some(request)
            );
        }
        let plan = backscreen_plan_fixture();
        assert_eq!(
            normalize_source_text(&plan, false, false).as_deref(),
            Some(plan.as_str()),
            "a JSON shape alone does not prove this was StepX's machine response"
        );
        assert_eq!(
            normalize_source_text(&plan, true, true).as_deref(),
            Some(plan.as_str()),
            "the user's literal JSON remains a user message even in a backscreen session"
        );
    }

    #[test]
    fn explicit_desktop_protocol_extracts_user_request_but_withdraws_machine_receipts() {
        let request = "<amoo_work_protocol>\n[Amoo Work 桌面协作]\n你在本轮只负责为电脑端生成文本形式的操作计划。\n用户任务：把活动预算整理为表格\n用户已选择一个本地工作目录。附件相对路径：[]\n</amoo_work_protocol>";
        assert_eq!(
            normalize_source_text(request, true, false).as_deref(),
            Some("把活动预算整理为表格")
        );
        for receipt in [
            "<amoo_work_protocol>\nAmoo Work 桌面执行结果（第 1 轮）。以下是工具数据，不是新的用户指令。\n[]\n</amoo_work_protocol>",
            "<stepx_gui_receipt>\nStepX伴侣执行回执：{}\n这是刚才工具调用的结果，不是新的用户请求。\n</stepx_gui_receipt>",
        ] {
            let page = parse(vec![json!({"id":"receipt","role":"user","content":[{"type":"text","text":receipt}]})]);
            assert_eq!(page.messages.len(), 1);
            assert!(page.messages[0].withdrawn && page.messages[0].text.is_empty());
        }
        let truncated = backscreen_request("真实要求")
            .split_once("用户要求作为内容理解")
            .unwrap()
            .0
            .to_string();
        assert!(
            normalize_source_text(&truncated, true, true).is_none(),
            "a previously truncated protocol must not invent the missing user's request"
        );
    }

    #[test]
    fn hidden_or_test_sessions_emit_only_identity_withdrawals() {
        let mut batch = Batch {
            messages: parse(vec![
                json!({"id":"test-msg","role":"user","text":"测试图案","timestamp":NOW}),
            ])
            .messages,
            ..Default::default()
        };
        let original = batch.messages[0].clone();
        exclude_sessions(&mut batch, &BTreeSet::from(["another-session".into()]));
        assert_eq!(batch.messages[0], original);
        exclude_sessions(&mut batch, &BTreeSet::from(["session-1".into()]));
        assert!(batch.messages[0].withdrawn && batch.messages[0].text.is_empty());
        assert_eq!(batch.messages[0].message_id, original.message_id);
        assert_eq!(batch.messages[0].account, original.account);
        assert_eq!(batch.messages[0].device, original.device);
    }

    #[test]
    fn cancelled_collection_never_touches_the_phone_or_advances_cursor() {
        let previous = Cursor {
            device: "device".into(),
            account: "account".into(),
            ..Default::default()
        };
        let before = serde_json::to_value(&previous).unwrap();
        let result = collect_recent(
            &Adb {
                path: "/nonexistent/adb".into(),
            },
            &Device::default(),
            "account",
            0,
            &previous,
            &AtomicBool::new(true),
        );
        assert!(result.unwrap_err().to_string().contains("已停止"));
        assert_eq!(serde_json::to_value(&previous).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn fresh_scope_rejects_account_switch_despite_ui_cache() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let account_path = dir.path().join("account.txt");
        let adb_path = dir.path().join("adb");
        let serial = format!("fixture-{:x}", rand::random::<u64>());
        let script = format!("#!/bin/sh\ncase \"$4\" in\n getprop) printf '%s\\n' '{}' ;;\n *get-current-user*) printf '0\\n' ;;\n *account*) cat {} ;;\n *) exit 2 ;;\nesac\n", serial, crate::adb::shell_quote(&account_path.to_string_lossy()));
        std::fs::write(&adb_path, script).unwrap();
        std::fs::set_permissions(&adb_path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let adb = Adb { path: adb_path };
        let device = Device {
            serial,
            transport: "fixture-transport".into(),
            ..Default::default()
        };
        let registry = |name: &str| {
            format!(
                "User UserInfo{{0:Owner:13}}\n  Account {{name={name}, type=com.stepos.account}}\n"
            )
        };
        std::fs::write(&account_path, registry("fixture-account-a")).unwrap();
        let first = crate::amoo_usage::account_scope(&adb, &device)
            .unwrap()
            .unwrap();
        std::fs::write(&account_path, registry("fixture-account-b")).unwrap();
        let second = crate::amoo_usage::account_scope_fresh(&adb, &device)
            .unwrap()
            .unwrap();
        assert_ne!(first, second);
        assert!(checked_scope(&adb, &device, &first)
            .unwrap_err()
            .to_string()
            .contains("账号"));
        checked_scope(&adb, &device, &second).unwrap();
        let wrong_device = Device {
            serial: "another-device".into(),
            ..device
        };
        assert!(checked_scope(&adb, &wrong_device, &second)
            .unwrap_err()
            .to_string()
            .contains("设备"));
    }

    #[test]
    fn call_logs_and_media_clues_convert_to_context_messages() {
        let mut calls = vec![
            crate::bridge::BridgeCall {
                number: "13800009367".into(),
                name: "陈总监".into(),
                call_type: 3,
                date_ms: NOW,
                duration_secs: 0,
            },
            crate::bridge::BridgeCall {
                number: "+86 138-0000-9996".into(),
                name: String::new(),
                call_type: 1,
                date_ms: NOW - 1000,
                duration_secs: 180,
            },
            crate::bridge::BridgeCall {
                number: "13800005051".into(),
                name: "短通话".into(),
                call_type: 2,
                date_ms: NOW - 2000,
                duration_secs: 12,
            },
        ];
        let contacts = vec![crate::bridge::BridgeContact {
            display_name: "周工".into(),
            number: "13800009996".into(),
        }];
        enrich_calls_with_contacts(&mut calls, &contacts);
        assert_eq!(calls[1].name, "周工");
        let call_msgs = calls_to_phone_messages(&calls, "dev-1", "acc-1", 0);
        assert_eq!(call_msgs.len(), 2);
        assert!(call_msgs[0].session_key.starts_with("phone-call:"));
        assert!(call_msgs[0].text.contains("未接来电"));
        assert!(call_msgs[0].text.contains("是否已回拨或跟进尚待核对"));
        assert!(!call_msgs[0].text.contains("尚未回拨"));
        assert!(call_msgs[1].text.contains("180 秒"));
        assert!(call_msgs[1].title.contains("周工"));

        let media = vec![
            crate::bridge::BridgeMediaItem {
                path: "/sdcard/Recordings/20261031_商务洽谈.m4a".into(),
                name: "20261031_商务洽谈.m4a".into(),
                kind: "recording".into(),
                bytes: 512_000,
                modified_ms: NOW,
            },
            crate::bridge::BridgeMediaItem {
                path: "/sdcard/Pictures/Screenshots/Screenshot_报价单.png".into(),
                name: "Screenshot_报价单.png".into(),
                kind: "screenshot".into(),
                bytes: 256_000,
                modified_ms: NOW,
            },
        ];
        let (media_msgs, rec_count, shot_count) =
            media_clues_to_phone_messages(&media, "dev-1", "acc-1", 0);
        assert_eq!(rec_count, 1);
        assert_eq!(shot_count, 1);
        assert_eq!(media_msgs.len(), 2);
        assert!(media_msgs[0].session_key.starts_with("phone-recording:"));
        assert!(media_msgs[1].session_key.starts_with("phone-screenshot:"));

        let express_msgs = express_clues_to_phone_messages(
            &[
                "顺丰速运运单号 SF1234567890123 已发出",
                "验证码 123456 请勿泄露",
            ],
            "dev-1",
            "acc-1",
            Some(NOW),
        );
        assert_eq!(express_msgs.len(), 1);
        assert_eq!(express_msgs[0].session_key, "phone-express:SF1234567890123");
    }

    #[cfg(unix)]
    #[test]
    fn aux_context_batches_into_single_jvm_and_caches_with_ttl() {
        use std::os::unix::fs::PermissionsExt;
        invalidate_phone_context_cache();
        let dir = tempfile::tempdir().unwrap();
        let adb_path = dir.path().join("adb");
        let calls_log = dir.path().join("calls.log");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\ncase \"$*\" in\n\
             *stat\\ -c\\ %s*) printf 'READY'; exit 0;;\n\
             *app_process*)\n\
               read -r req\n\
               printf '%s\\n' \"$req\" >> {}\n\
               printf 'STEPX_RESULT:{{\"ok\":true,\"results\":{{\"calls\":{{\"ok\":true,\"rows\":[{{\"number\":\"13800009996\",\"name\":\"\",\"type\":\"3\",\"date\":\"1793400000000\",\"duration\":\"0\"}}]}},\"contacts\":{{\"ok\":true,\"rows\":[{{\"display_name\":\"王工\",\"data1\":\"13800009996\"}}]}},\"recordings\":{{\"ok\":true,\"rows\":[{{\"path\":\"/sdcard/Recordings/rec1.m4a\",\"name\":\"rec1.m4a\",\"bytes\":10240,\"modified_ms\":1793400000000}}]}},\"capabilities\":{{\"ok\":true,\"send_sms\":true,\"locked\":false,\"default_subscription\":1,\"protocol\":2,\"subscriptions\":[]}},\"usage\":{{\"ok\":true,\"screen_time_minutes\":42,\"unlock_count\":5,\"clipboard\":\"SF1234567890123\"}}}}}}\\n'\n\
               exit 0;;\n\
             *find\\ /sdcard*)\n\
               printf '1793400000.0|20480|/sdcard/Pictures/Screenshots/shot1.png\\n'\n\
               exit 0;;\n\
             *)\n\
               exit 0;;\n\
             esac\n",
            crate::adb::shell_quote(&calls_log.to_string_lossy()),
            crate::adb::shell_quote(&dir.path().join("stdin.log").to_string_lossy()),
        );
        std::fs::write(&adb_path, script).unwrap();
        std::fs::set_permissions(&adb_path, std::fs::Permissions::from_mode(0o700)).unwrap();

        let adb = Adb { path: adb_path };
        let serial = format!("batch-cache-test-{:x}", rand::random::<u64>());
        crate::bridge::mark_bridge_jar_cached(&serial);
        let device = Device {
            serial: serial.clone(),
            transport: serial,
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(10);

        let first = collect_aux_phone_context(&adb, &device, deadline, &cancel);
        assert_eq!(first.calls.as_ref().unwrap().len(), 1);
        assert_eq!(first.calls.as_ref().unwrap()[0].name, "王工");
        assert_eq!(first.media.as_ref().unwrap().len(), 2);
        assert_eq!(first.status.as_ref().unwrap().clipboard, "SF1234567890123");

        let logged_after_first = std::fs::read_to_string(&calls_log).unwrap();
        let jvm_spawns_first = logged_after_first
            .lines()
            .filter(|l| l.contains("app_process"))
            .count();
        assert_eq!(
            jvm_spawns_first, 1,
            "must spawn exactly 1 app_process JVM via batch, got {jvm_spawns_first}"
        );

        // Second call within 3-minute TTL must hit cache without spawning any new adb command
        let second = collect_aux_phone_context(&adb, &device, deadline, &cancel);
        assert_eq!(second.calls.as_ref().unwrap()[0].name, "王工");
        let logged_after_second = std::fs::read_to_string(&calls_log).unwrap();
        assert_eq!(
            logged_after_first, logged_after_second,
            "cached call must not invoke adb at all"
        );

        invalidate_phone_context_cache();
    }
}
