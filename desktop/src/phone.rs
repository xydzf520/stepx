use crate::{adb::Adb, bridge::call_bridge, device::Device};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

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

pub fn control_call(adb: &Adb, device: &Device, expected_id: &str, action: CallControl) -> Result<LiveCall> {
    // Discovery is read-only. A failed catalog can use the fixed system adapter.
    // Once a tool is dispatched, an uncertain result never switches route.
    let (catalog_tx, catalog_rx) = std::sync::mpsc::channel();
    let discovery_adb = adb.clone();
    let discovery_device = device.clone();
    std::thread::spawn(move || {
        // Only a read-only catalog probe may outlive this short UI deadline.
        let _ = catalog_tx.send(crate::native_mcp::Client::connect(&discovery_adb, &discovery_device));
    });
    let mut native = catalog_rx.recv_timeout(Duration::from_millis(500)).ok().and_then(Result::ok);
    let current = crate::push::fetch_live_call(adb, &device.resolve_active_transport(adb)?)?;
    ensure_current_call(&current, expected_id, action, chrono::Utc::now().timestamp_millis())?;
    let method = match action { CallControl::Answer => "dialer_accept_call", CallControl::Hangup => "dialer_hangup_call" };
    let _mutation = crate::device::protect_transport_mutation(&device.serial)?;
    if let Some(client) = native.as_mut().filter(|client| client.supports(method)) {
        client.call(method, json!({}))?;
    } else {
        call_bridge(adb, device, &json!({"op":"phone_control", "action":if action == CallControl::Answer {"answer"} else {"hangup"}, "state":current.state}), Duration::from_secs(12))?;
    }
    // The command receipt alone cannot establish that the call changed state.
    for _ in 0..12 {
        let state = crate::push::fetch_live_call(adb, &device.resolve_active_transport(adb)?)?;
        let confirmed = match action {
            CallControl::Answer => state.id == expected_id && state.state == "active",
            CallControl::Hangup => state.available && state.state == "idle",
        };
        if confirmed { return Ok(state); }
        if !state.id.is_empty() && state.id != expected_id { bail!("通话已变化，请核对手机当前状态") }
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
}
