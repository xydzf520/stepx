use crate::{adb::Adb, bridge::call_bridge, device::Device};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
    time::Duration,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveCall {
    pub multiple_calls: bool,
    pub available: bool,
    pub id: String,
    pub state: String,
    pub number: String,
    pub name: String,
    pub started_ms: i64,
    pub observed_ms: i64,
}

impl LiveCall {
    pub fn ongoing(&self, now_ms: i64) -> bool {
        self.available && !self.id.is_empty()
            && matches!(self.state.as_str(), "ringing" | "active")
            && (now_ms - 15_000..=now_ms + 2_000).contains(&self.observed_ms)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallControl { Answer, Hangup }

static INFLIGHT_CATALOG_PROBES: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

struct CatalogProbeSlot {
    key: String,
}

impl CatalogProbeSlot {
    fn try_acquire(device_key: &str) -> Option<Self> {
        let key = device_key.trim();
        let normalized = if key.is_empty() { "<default>" } else { key };
        let set = INFLIGHT_CATALOG_PROBES.get_or_init(|| Mutex::new(HashSet::new()));
        let mut guard = set.lock().ok()?;
        if !guard.insert(normalized.to_string()) {
            return None;
        }
        Some(Self {
            key: normalized.to_string(),
        })
    }
}

impl Drop for CatalogProbeSlot {
    fn drop(&mut self) {
        if let Some(set) = INFLIGHT_CATALOG_PROBES.get() {
            if let Ok(mut guard) = set.lock() {
                guard.remove(&self.key);
            }
        }
    }
}

fn probe_native_catalog_bounded(adb: &Adb, device: &Device) -> Option<crate::native_mcp::Client> {
    let probe_key = if !device.serial.trim().is_empty() {
        device.serial.as_str()
    } else {
        device.transport.as_str()
    };
    let slot = CatalogProbeSlot::try_acquire(probe_key)?;
    let (catalog_tx, catalog_rx) = std::sync::mpsc::sync_channel(1);
    let discovery_adb = adb.clone();
    let discovery_device = device.clone();
    let handle = std::thread::Builder::new()
        .name("stepx-call-mcp-probe".into())
        .spawn(move || {
            let _slot = slot;
            let _ = catalog_tx.send(crate::native_mcp::Client::connect(
                &discovery_adb,
                &discovery_device,
            ));
        })
        .ok()?;

    match catalog_rx.recv_timeout(Duration::from_millis(500)) {
        Ok(res) => {
            drop(catalog_rx);
            let _ = handle.join();
            res.ok()
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let _ = handle.join();
            None
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            // Drop receiver immediately so if the bounded probe finishes later, any
            // connected AgentBus client is dropped at once instead of lingering.
            drop(catalog_rx);
            None
        }
    }
}

fn check_call_control_confirmation(
    action: CallControl,
    expected_id: &str,
    state: &LiveCall,
) -> Result<bool> {
    match action {
        CallControl::Hangup => Ok(
            state.state == "idle" || (!expected_id.is_empty() && state.id != expected_id),
        ),
        CallControl::Answer => {
            if state.id == expected_id && state.state == "active" {
                return Ok(true);
            }
            if state.state == "idle" || (!state.id.is_empty() && state.id != expected_id) {
                bail!("通话已变化，请核对手机当前状态");
            }
            Ok(false)
        }
    }
}

pub fn control_call(adb: &Adb, device: &Device, expected_id: &str, action: CallControl) -> Result<LiveCall> {
    let transport = device.resolve_active_transport(adb)?;
    let current = crate::push::fetch_live_call(adb, &transport)?;
    ensure_current_call(&current, expected_id, action, chrono::Utc::now().timestamp_millis())?;

    // Discovery is read-only and bounded to at most one probe per device.
    // A failed or timed-out catalog uses the fixed system adapter; once dispatched, never switch route.
    let mut native = probe_native_catalog_bounded(adb, device);
    let method = match action { CallControl::Answer => "dialer_accept_call", CallControl::Hangup => "dialer_hangup_call" };
    let _mutation = crate::device::protect_transport_mutation(&device.serial)?;
    if let Some(client) = native.as_mut().filter(|client| client.supports(method)) {
        let res = client.call(method, json!({}));
        drop(native);
        res?;
    } else {
        drop(native);
        call_bridge(adb, device, &json!({"op":"phone_control", "action":if action == CallControl::Answer {"answer"} else {"hangup"}, "state":current.state}), Duration::from_secs(12))?;
    }
    // The command receipt alone cannot establish that the call changed state.
    for _ in 0..12 {
        let next_transport = device.resolve_active_transport(adb)?;
        let state = crate::push::fetch_live_call(adb, &next_transport)?;
        if check_call_control_confirmation(action, expected_id, &state)? {
            return Ok(state);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    bail!("手机尚未确认通话状态，请查看手机后重试")
}

fn ensure_current_call(call: &LiveCall, expected_id: &str, action: CallControl, now_ms: i64) -> Result<()> {
    anyhow::ensure!(!call.multiple_calls, "手机存在多路通话，请在手机上选择要操作的通话");
    anyhow::ensure!(call.ongoing(now_ms) && call.id == expected_id, "这次通话已结束或变化，请刷新后重试");
    anyhow::ensure!(action != CallControl::Answer || call.state == "ringing", "来电已不在响铃，请查看当前通话状态");
    Ok(())
}

/// A call is prepared without placing it. The UI must display this exact plan
/// and require a second click before submitting it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialPlan {
    pub request_id: String,
    pub device_serial: String,
    pub number: String,
    pub subscription: i32,
    pub subscription_label: String,
    pub account_key: String,
}

pub fn normalize_number(number: &str) -> Result<String> {
    let number = number.trim();
    if number.is_empty() || number.len() > 64 {
        bail!("请输入有效的电话号码");
    }
    let mut clean = String::new();
    for ch in number.chars() {
        if ch.is_ascii_digit() || (ch == '+' && clean.is_empty()) {
            clean.push(ch);
        } else if !" -()".contains(ch) {
            bail!("号码只支持数字、开头的 + 和空格、括号、短横线；服务代码请在手机拨号");
        }
    }
    let digits = clean.trim_start_matches('+');
    if digits.len() < 2 || digits.len() > 40 || !digits.chars().all(|c| c.is_ascii_digit()) {
        bail!("请输入有效的电话号码");
    }
    Ok(clean)
}

pub fn prepare_call(
    adb: &Adb,
    device: &Device,
    number: &str,
    subscription: Option<i32>,
) -> Result<DialPlan> {
    let number = normalize_number(number)?;
    let result = call_bridge(
        adb,
        device,
        &json!({"op": "phone_prepare", "number": number, "subscription": subscription.unwrap_or(-1)}),
        Duration::from_secs(15),
    )?;
    Ok(DialPlan {
        request_id: format!(
            "{:016x}{:016x}",
            rand::random::<u64>(),
            rand::random::<u64>()
        ),
        device_serial: device.serial.clone(),
        number,
        subscription: result
            .get("subscription")
            .and_then(|v| v.as_i64())
            .context("手机未返回有效的通话 SIM")? as i32,
        subscription_label: result
            .get("subscription_label")
            .and_then(|v| v.as_str())
            .context("手机未返回通话 SIM 名称")?
            .to_owned(),
        account_key: result
            .get("account_key")
            .and_then(|v| v.as_str())
            .context("手机未返回可用的通话账户")?
            .to_owned(),
    })
}

pub fn submit_call(adb: &Adb, device: &Device, plan: &DialPlan) -> Result<String> {
    if plan.device_serial != device.serial {
        bail!("连接设备已变更，请重新确认呼叫号码与 SIM 卡");
    }
    let _mutation = crate::device::protect_transport_mutation(&device.serial)?;
    let result = call_bridge(
        adb,
        device,
        &json!({
            "op": "phone_call", "id": plan.request_id, "number": plan.number,
            "subscription": plan.subscription, "account_key": plan.account_key,
        }),
        Duration::from_secs(15),
    )
    .context("未能确认呼叫请求结果，未自动重拨；请先检查手机通话状态")?;
    match result.get("state").and_then(|v| v.as_str()) {
        Some("submitted") => Ok(format!(
            "已通过 {} 提交呼叫请求，请查看手机通话状态",
            plan.subscription_label
        )),
        Some("unknown") => bail!("呼叫请求状态未确认，未自动重拨；请先检查手机通话状态"),
        _ => bail!("手机未返回有效的呼叫提交状态，未自动重拨"),
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_number;

    #[test]
    fn live_controls_reject_stale_calls_and_changed_identity() {
        use super::*;
        let call = LiveCall { available:true, id:"first".into(), state:"ringing".into(), observed_ms:20_000, ..Default::default() };
        assert!(ensure_current_call(&call,"first",CallControl::Answer,20_000).is_ok());
        assert!(ensure_current_call(&call,"second",CallControl::Hangup,20_000).is_err());
        assert!(ensure_current_call(&call,"first",CallControl::Answer,40_000).is_err());
        let active = LiveCall { state:"active".into(), ..call };
        assert!(ensure_current_call(&active,"first",CallControl::Answer,20_000).is_err());
        assert!(ensure_current_call(&active,"first",CallControl::Hangup,20_000).is_ok());
        let multiple = LiveCall { multiple_calls: true, ..active };
        assert!(ensure_current_call(&multiple,"first",CallControl::Answer,20_000).is_err());
        assert!(ensure_current_call(&multiple,"first",CallControl::Hangup,20_000).is_err());
        assert!(!LiveCall::default().ongoing(20_000));
    }

    #[test]
    fn formats_numbers_without_changing_destination() {
        assert_eq!(
            normalize_number(" +1 (202) 555-0148 ").unwrap(),
            "+12025550148"
        );
        assert_eq!(normalize_number("010-5555 0123").unwrap(), "01055550123");
    }

    #[test]
    fn rejects_mmi_extensions_and_invalid_characters_instead_of_stripping_them() {
        for input in [
            "",
            " ",
            "+",
            "1",
            "+  +123",
            "123+456",
            "123abc456",
            "123,456",
            "123;456",
            "*#06#",
            "１２３４５",
        ] {
            assert!(normalize_number(input).is_err(), "accepted {input:?}");
        }
    }

    #[test]
    fn hangup_confirms_on_idle_or_replaced_call_id_and_bounds_catalog_probes() {
        use super::*;

        // 1. Hangup immediately confirms on `state == "idle"` regardless of `available`
        let idle_unavailable = LiveCall {
            available: false,
            id: String::new(),
            state: "idle".into(),
            ..Default::default()
        };
        assert!(check_call_control_confirmation(CallControl::Hangup, "call-1", &idle_unavailable).unwrap());

        // 2. Hangup immediately confirms when `state.id != expected_id` (e.g., new incoming call arrived or id cleared)
        let next_ringing = LiveCall {
            available: true,
            id: "call-2".into(),
            state: "ringing".into(),
            ..Default::default()
        };
        assert!(check_call_control_confirmation(CallControl::Hangup, "call-1", &next_ringing).unwrap());

        let cleared_id = LiveCall {
            available: true,
            id: String::new(),
            state: "disconnecting".into(),
            ..Default::default()
        };
        assert!(check_call_control_confirmation(CallControl::Hangup, "call-1", &cleared_id).unwrap());

        // 3. Hangup keeps polling while same `expected_id` is still active
        let still_active = LiveCall {
            available: true,
            id: "call-1".into(),
            state: "active".into(),
            ..Default::default()
        };
        assert!(!check_call_control_confirmation(CallControl::Hangup, "call-1", &still_active).unwrap());

        // 4. Answer confirms only when `expected_id` reaches `"active"` and errors if call changed or ended
        assert!(check_call_control_confirmation(CallControl::Answer, "call-1", &still_active).unwrap());
        let still_ringing = LiveCall {
            state: "ringing".into(),
            ..still_active
        };
        assert!(!check_call_control_confirmation(CallControl::Answer, "call-1", &still_ringing).unwrap());
        assert!(check_call_control_confirmation(CallControl::Answer, "call-1", &next_ringing).is_err());
        assert!(check_call_control_confirmation(CallControl::Answer, "call-1", &idle_unavailable).is_err());

        // 5. CatalogProbeSlot bounds concurrent discovery probes to at most 1 per device
        let slot1 = CatalogProbeSlot::try_acquire("PHONE-PROBE-DEV-1").expect("first slot should acquire");
        assert!(
            CatalogProbeSlot::try_acquire("PHONE-PROBE-DEV-1").is_none(),
            "duplicate concurrent probe on same device must be rejected"
        );
        assert!(
            CatalogProbeSlot::try_acquire("PHONE-PROBE-DEV-2").is_some(),
            "different device must remain independent"
        );
        drop(slot1);
        assert!(
            CatalogProbeSlot::try_acquire("PHONE-PROBE-DEV-1").is_some(),
            "slot must release cleanly on drop"
        );
    }

    #[cfg(unix)]
    #[test]
    fn control_call_hangup_succeeds_when_new_call_arrives_without_false_failure() {
        use super::*;
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let call_state_polls = Arc::new(AtomicUsize::new(0));
        let polls_clone = call_state_polls.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        listener.set_nonblocking(true).unwrap();

        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            while !stop_clone.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                let stream = match listener.accept() {
                    Ok((s, _)) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let poll_idx = polls_clone.fetch_add(1, Ordering::SeqCst);
                let now_ms = chrono::Utc::now().timestamp_millis();
                // Poll 0 (pre-hangup check): call-old is active.
                // Poll 1 (post-hangup check): call-old ended and call-new is already ringing!
                let resp = if poll_idx == 0 {
                    json!({
                        "ok": true,
                        "available": true,
                        "id": "call-old",
                        "state": "active",
                        "multiple_calls": false,
                        "observed_ms": now_ms
                    })
                } else {
                    json!({
                        "ok": true,
                        "available": true,
                        "id": "call-new",
                        "state": "ringing",
                        "multiple_calls": false,
                        "observed_ms": now_ms
                    })
                };
                let mut out = serde_json::to_vec(&resp).unwrap();
                out.push(b'\n');
                let _ = (&stream).write_all(&out);
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let fake_adb = dir.path().join("adb");
        let serial = "PHONEHANGUPTEST01";
        std::fs::write(
            &fake_adb,
            format!(
                r#"#!/bin/sh
case "$*" in
  *"shell getprop ro.serialno"*)
    printf '{serial}\n'
    exit 0
    ;;
  *"forward tcp:0 localabstract:stepx_push_v1"*)
    printf '{port}\n'
    exit 0
    ;;
  *"stat -c %s"*)
    printf 'READY'
    exit 0
    ;;
  *"com.stepx.companion.Bridge"*)
    printf 'STEPX_RESULT:{{"ok":true}}\n'
    exit 0
    ;;
esac
exit 1
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake_adb, std::fs::Permissions::from_mode(0o755)).unwrap();

        crate::bridge::mark_bridge_jar_cached(serial);
        let device = Device {
            transport: serial.into(),
            serial: serial.into(),
            ..Default::default()
        };
        let confirmed_state =
            control_call(&Adb { path: fake_adb }, &device, "call-old", CallControl::Hangup)
                .expect("hangup must confirm success when old call disappears and new call arrives");
        assert_eq!(confirmed_state.id, "call-new");

        stop.store(true, Ordering::Release);
        let _ = server.join();
    }
}
