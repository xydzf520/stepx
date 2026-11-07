//! Phone-owned native tools over AgentBus (USB or wireless ADB).
//! Discovery is read-only. Requests go directly to the registered local provider,
//! never to a language model, shell command planner or credential proxy.
use crate::{
    adb::Adb,
    amoo::{AgentBus, LocalInvokeEnd},
    device::Device,
};
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub struct Client {
    bus: AgentBus,
    tools: BTreeMap<String, Value>,
    mutation_target: String,
}

fn catalog(reply: &Value) -> Result<BTreeMap<String, Value>> {
    ensure!(
        reply["ok"] == true && reply["payload"]["ok"] == true,
        "手机原生能力目录读取失败；尚未发送操作"
    );
    let rows = reply["payload"]["abilities"]
        .as_array()
        .context("手机原生能力目录格式已变化；尚未发送操作")?;
    let mut tools = BTreeMap::new();
    for row in rows {
        // Static documentation, cloud tools and strings mentioning a name do not
        // count as a locally callable MCP provider.
        if row["callingMethod"] == "LOCAL" && row["callType"] == "MCP" {
            if let Some(name) = row["name"].as_str() {
                tools.insert(name.to_owned(), row["inputSchema"].clone());
            }
        }
    }
    Ok(tools)
}

fn validate(schema: &Value, args: &Value) -> Result<()> {
    let args = args.as_object().context("原生工具参数必须是对象")?;
    for key in schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        ensure!(args.contains_key(key), "原生工具缺少参数：{key}");
    }
    for (key, value) in args {
        let field = &schema["properties"][key];
        ensure!(!field.is_null(), "当前手机原生工具不支持参数：{key}");
        let valid = match field["type"].as_str().unwrap_or("") {
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "integer" | "int" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            // PendingIntent and other Binder objects cannot be fabricated as JSON.
            _ => false,
        };
        ensure!(valid, "当前手机原生工具参数类型不匹配：{key}");
        if let Some(allowed) = field["enum"].as_array() {
            ensure!(allowed.contains(value), "原生工具参数值不支持：{key}");
        }
    }
    Ok(())
}

fn error_message(reply: &Value) -> String {
    [
        &reply["error"]["message"],
        &reply["payload"]["error"]["message"],
    ]
    .into_iter()
    .find_map(Value::as_str)
    .unwrap_or("手机拒绝调用")
    .chars()
    .filter(|c| !c.is_control() || *c == '\n')
    .take(300)
    .collect()
}

fn decode_content(value: &Value, depth: usize) -> Result<Value> {
    ensure!(depth < 8, "原生工具回执嵌套过深");
    if let Some(text) = value.as_str() {
        if text.trim_start().starts_with(['{', '[']) {
            if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                return decode_content(&parsed, depth + 1);
            }
        }
        return Ok(value.clone());
    }
    ensure!(
        value["isError"] != true
            && value["success"] != false
            && value["ok"] != false
            && !matches!(value["status"].as_str(), Some("failed" | "error")),
        "原生工具返回业务失败；未自动重试"
    );
    if let Some(items) = value["content"].as_array() {
        let mut texts = Vec::new();
        for item in items {
            if item["type"] == "text" {
                texts.push(decode_content(&item["text"], depth + 1)?);
            }
        }
        if texts.len() == 1 {
            return Ok(texts.remove(0));
        }
        if !texts.is_empty() {
            return Ok(Value::Array(texts));
        }
    }
    Ok(value.clone())
}

pub(crate) fn decode_reply(reply: &Value) -> Result<Value> {
    ensure!(
        reply["ok"] == true && reply["payload"]["ok"] == true,
        "原生工具未完成：{}。未自动换通道重试",
        error_message(reply)
    );
    let content = &reply["payload"]["content"];
    ensure!(
        !content.is_null(),
        "手机只返回了接收回执，尚未确认执行结果；未自动重试"
    );
    decode_content(content, 0)
}

impl Client {
    pub fn connect(adb: &Adb, device: &Device) -> Result<Self> {
        device.verify(adb)?;
        let mut bus = AgentBus::observe(adb, device)?;
        let tools = catalog(&bus.local_gui_catalog()?)?;
        Ok(Self { bus, tools, mutation_target: if device.serial.is_empty() { device.transport.clone() } else { device.serial.clone() } })
    }

    pub fn supports(&self, method: &str) -> bool {
        self.tools.contains_key(method)
    }
    pub fn tool_names(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(String::as_str)
    }
    pub fn tool_schema(&self, name: &str) -> Option<&Value> {
        self.tools.get(name)
    }

    pub fn call(&mut self, method: &str, args: Value) -> Result<Value> {
        self.call_cancellable(method, args, &AtomicBool::new(false))
    }

    pub fn call_cancellable(
        &mut self,
        method: &str,
        args: Value,
        cancel: &AtomicBool,
    ) -> Result<Value> {
        let schema = self
            .tools
            .get(method)
            .context("当前手机未提供该原生 MCP 工具；尚未发送操作")?;
        validate(schema, &args)?;
        let _mutation = crate::device::protect_transport_mutation(&self.mutation_target)?;
        ensure!(
            !cancel.load(Ordering::Relaxed),
            "操作已取消，尚未发送到手机"
        );
        match self
            .bus
            .invoke_local_ability(method, args, Duration::from_secs(30), cancel)?
            .end
        {
            LocalInvokeEnd::Reply(reply) if method == "common_ability_clipboard_text" => {
                // JSON, numbers and strings saying "error" can all be clipboard
                // data. Never interpret them as an executor's business receipt.
                ensure!(
                    reply["ok"] == true && reply["payload"]["ok"] == true,
                    "原生剪贴板读取失败：{}",
                    error_message(&reply)
                );
                ensure!(
                    !reply["payload"]["content"].is_null(),
                    "原生剪贴板未返回内容"
                );
                Ok(reply["payload"]["content"].clone())
            }
            LocalInvokeEnd::Reply(reply) => decode_reply(&reply),
            LocalInvokeEnd::WaitCancelled => {
                bail!("已停止等待，手机可能仍在执行；请核对结果后再操作")
            }
            LocalInvokeEnd::TimedOut => bail!("原生工具等待超时，手机可能仍在执行；未自动重试"),
            LocalInvokeEnd::Disconnected => bail!("连接中断，手机执行结果未确认；未自动重试"),
        }
    }
}

/// None means a successfully read directory does not offer this tool. Errors
/// (including uncertain delivery) must propagate, never trigger a second route.
pub fn try_call(adb: &Adb, device: &Device, method: &str, args: Value) -> Result<Option<Value>> {
    let mut client = Client::connect(adb, device)?;
    if !client.supports(method) {
        return Ok(None);
    }
    client.call(method, args).map(Some)
}

pub fn text(value: &Value) -> Result<String> {
    let text = value
        .as_str()
        .context("原生工具未返回预期文本；未使用推测内容")?;
    Ok(text.to_owned())
}

pub fn response_note(value: &Value) -> String {
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    format!(
        "手机原生接口返回：{}",
        text.chars().take(600).collect::<String>()
    )
}

fn number(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}

fn planner_calendar_row(row: &Value) -> Value {
    let mut normalized = json!({"id":number(&row["event_id"]),"title":row["title"],"description":row["description"],
        "location":row["eventLocation"],"start_ms":number(&row["begin"]),"end_ms":number(&row["end"]),"all_day":number(&row["allDay"]) == Some(1)});
    for key in ["rrule", "rdate", "recurrenceRule", "recurrence_rule", "recurring", "is_recurring", "isRecurring"] {
        if let Some(value) = row.get(key) {
            normalized[key] = value.clone();
        }
    }
    normalized
}

/// Read the exact provider rows only to validate a native write and obtain stable
/// IDs for later user edits. The write itself belongs to the phone MCP service.
fn planner_snapshot(adb: &Adb, device: &Device, kind: &str, start: i64, end: i64) -> Result<Value> {
    let request = if kind == "alarms" {
        json!({"op":"read_alarms"})
    } else {
        json!({"op":"read_calendar","start_ms":start.saturating_sub(1000).max(0),"end_ms":end.saturating_add(1000),"limit":1000})
    };
    let raw = crate::bridge::call_bridge(adb, device, &request, Duration::from_secs(12))?;
    ensure!(
        raw["has_more"] != true,
        "手机记录过多，未取得完整核验范围；不能猜测新记录 ID"
    );
    let rows = raw["rows"]
        .as_array()
        .context("未取得手机记录，不能核验原生操作")?;
    let rows = rows.iter().map(|r| if kind == "alarms" {
        json!({"id":number(&r["_id"]),"hour":number(&r["hour"]),"minutes":number(&r["minutes"]),
            "days_of_week":number(&r["daysofweek"]),"enabled":number(&r["enabled"]) == Some(1),
            "vibrate":number(&r["vibrate"]) == Some(1),"label":r["label"]})
    } else {
        planner_calendar_row(r)
    }).collect::<Vec<_>>();
    Ok(json!({kind:rows}))
}

fn new_record_id(
    before: &Value,
    after: &Value,
    kind: &str,
    matches: impl Fn(&Value) -> bool,
) -> Result<i64> {
    let old = before[kind].as_array().context("缺少操作前快照")?;
    let rows = after[kind].as_array().context("缺少操作后快照")?;
    let mut found = rows
        .iter()
        .filter(|r| {
            number(&r["id"]).is_some_and(|id| id > 0)
                && !old.iter().any(|v| v["id"] == r["id"])
                && matches(r)
        })
        .filter_map(|r| number(&r["id"]))
        .collect::<Vec<_>>();
    found.sort_unstable();
    found.dedup();
    ensure!(
        found.len() == 1,
        "原生操作已提交，但尚未唯一核对到新记录；请刷新手机确认，未自动重试"
    );
    Ok(found[0])
}

pub(crate) fn create_alarm(
    adb: &Adb,
    device: &Device,
    hour: u8,
    minute: u8,
    label: &str,
) -> Result<Option<Value>> {
    ensure!(hour < 24 && minute < 60, "闹钟时间无效");
    let mut client = Client::connect(adb, device)?;
    if !client.supports("alarm_clock_add") {
        return Ok(None);
    }
    let before = planner_snapshot(adb, device, "alarms", 0, 0)?;
    client.call(
        "alarm_clock_add",
        json!({"hour":hour,"minute":minute,"label":label,"repeatDays":0}),
    )?;
    let mut after = planner_snapshot(adb, device, "alarms", 0, 0)?;
    let id = new_record_id(&before, &after, "alarms", |r| {
        r["hour"] == hour
            && r["minutes"] == minute
            && r["label"] == label
            && r["enabled"] == true
            && r["days_of_week"] == 0
    })?;
    after["created_id"] = json!(id);
    Ok(Some(after))
}

pub(crate) fn create_calendar(
    adb: &Adb,
    device: &Device,
    title: &str,
    description: &str,
    location: &str,
    start: i64,
    end: i64,
) -> Result<Option<Value>> {
    ensure!(start > 0 && end > start, "日程时间范围无效");
    let mut client = Client::connect(adb, device)?;
    if !client.supports("calendar_add_event") {
        return Ok(None);
    }
    let transport = device.resolve_active_transport(adb)?;
    let phone_time = |ms: i64| -> Result<String> {
        let text = adb.shell(
            &transport,
            &[
                "date",
                "-d",
                &format!("@{}", ms / 1000),
                "+%Y-%m-%d %H:%M:%S",
            ],
        )?;
        chrono::NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M:%S")
            .context("手机时区时间转换失败，尚未创建日程")?;
        Ok(text.trim().into())
    };
    let args = json!({"title":title,"description":description,"location":location,
        "start_time":phone_time(start)?,"end_time":phone_time(end)?,"is_all_day":false,"recurrence_type":0,"reminder_offset":0});
    let before = planner_snapshot(adb, device, "calendar_events", start, end)?;
    client.call("calendar_add_event", args)?;
    let after = planner_snapshot(adb, device, "calendar_events", start, end)?;
    let id = new_record_id(&before, &after, "calendar_events", |r| {
        r["title"] == title
            && r["description"].as_str().unwrap_or("") == description
            && r["location"].as_str().unwrap_or("") == location
            && number(&r["start_ms"]) == Some(start / 1000 * 1000)
            && number(&r["end_ms"]) == Some(end / 1000 * 1000)
    })?;
    // The exact verification window must not replace the home calendar with a
    // single-event list. Return the normal overview plus the verified new row.
    let now = chrono::Utc::now().timestamp_millis();
    let mut overview = planner_snapshot(
        adb,
        device,
        "calendar_events",
        now - 7 * 86_400_000,
        now + 32 * 86_400_000,
    )
    .context("日程已经创建并核验，但列表刷新失败；请刷新列表，勿重复创建")?;
    let rows = overview["calendar_events"]
        .as_array_mut()
        .context("日历列表格式错误")?;
    if !rows.iter().any(|r| number(&r["id"]) == Some(id)) {
        if let Some(row) = after["calendar_events"]
            .as_array()
            .and_then(|rows| rows.iter().find(|r| number(&r["id"]) == Some(id)))
        {
            rows.push(row.clone());
        }
    }
    overview["created_id"] = json!(id);
    Ok(Some(overview))
}

/// Exact mappings only: opening a settings page is not the same as changing it.
pub(crate) fn quick_action(action: &str, payload: &str) -> Option<(&'static str, Value)> {
    let component = match action {
        "open_aiengine" => Some((
            "com.stepos.aiengine",
            "com.stepos.aiengine.modelmanager.ModelManagerActivity",
        )),
        "open_safety_ledger" => Some((
            "com.stepos.agentsafetycenter",
            "com.stepos.agentsafetycenter.auth.LaunchAuthActivity",
        )),
        "open_alphame" => Some((
            "com.stepos.alphame",
            "com.stepos.alphame.persona.ui.PersonalMainActivity",
        )),
        "open_amoo_voice" => Some((
            "com.stepos.ailauncher",
            "com.stepos.ailauncher.chat.activity.VoiceSettingActivity",
        )),
        "open_backscreen" | "open_backscreen_settings" => Some((
            "com.stepos.backscreen",
            "com.stepos.backscreen.settings.BackScreenSettingsActivity",
        )),
        _ => None,
    };
    if let Some((package, class)) = component {
        return Some((
            "open_app_by_package",
            json!({"packageName":package,"className":class}),
        ));
    }
    let package = match action {
        "open_filemanager" => Some("com.stepos.filemanager"),
        "open_mms" => Some("com.android.mms"),
        "open_gallery" => Some("com.stepos.media.gallery"),
        "open_calculator" => Some("com.stepos.calculator"),
        "open_logreport" => Some("com.stepos.logreport"),
        "open_data_migration" => Some("com.stepos.datamigration"),
        _ => None,
    };
    if let Some(package) = package {
        return Some(("open_app_by_package", json!({"packageName":package})));
    }
    match action {
        "open_soundrecorder" => Some(("lite_recorder_open_home", json!({}))),
        "open_feedback" => Some(("settings_open_feedback", json!({}))),
        "open_gallery_trash" => Some(("gallery_open_trash_bin", json!({}))),
        "open_file_trash" => Some((
            "security_permissioncontroller_enter_file_recycle_bin_page",
            json!({}),
        )),
        "open_safebox" => Some(("security_safebox_open", json!({}))),
        "open_security_scan" => Some(("security_manager_into_virus_page", json!({}))),
        "open_secureservice" => Some(("security_center_open_main", json!({}))),
        "open_app_manager" => Some(("settings_open_manage_applications", json!({}))),
        "open_dialer" => Some(("dialer_show_dialpad", json!({}))),
        "open_language_settings" => Some(("settings_open_language", json!({}))),
        "open_search" if payload.trim().is_empty() => Some(("search_open", json!({}))),
        "search_files" | "open_search" if !payload.trim().is_empty() => Some((
            "filemanager_search_file",
            json!({"keywords":payload.trim()}),
        )),
        "voice_tts" if !payload.trim().is_empty() => {
            Some(("speak_text", json!({"text":payload.trim()})))
        }
        "volume_up" | "volume_down" => Some((
            "settings_adjust_sound",
            json!({"soundType":"media","openState":action == "volume_up"}),
        )),
        "volume_mute" | "volume_50" => Some((
            "settings_set_media_sound",
            json!({"newValue":if action == "volume_mute" {"0"} else {"50"}}),
        )),
        "open_clip_url_phone" => {
            let url = payload
                .split_whitespace()
                .find(|s| s.starts_with("https://") || s.starts_with("http://"))?;
            Some(("open_url_with_browser", json!({"url":url})))
        }
        _ => None,
    }
}

pub(crate) fn screen_command(task: &str) -> Option<(&'static str, Value)> {
    let tool = match task.trim() {
        "打开设置" | "系统设置" | "打开系统设置" | "打开手机设置" => {
            "settings_open_settings_homepage_activity"
        }
        "Wi-Fi 设置"
        | "WiFi设置"
        | "Wi-Fi设置"
        | "打开 Wi-Fi 设置"
        | "打开WiFi设置"
        | "打开手机Wi-Fi设置"
        | "打开手机WiFi设置"
        | "无线网设置"
        | "打开无线网设置"
        | "打开 Wi-Fi"
        | "打开WiFi" => "connectivitysettings_open_settings_wifi_settings_activity",
        "蓝牙设置" | "打开蓝牙设置" | "打开手机蓝牙设置" | "打开蓝牙" | "连接蓝牙" => {
            "connectivitysettings_open_settings_bluetooth_settings_activity"
        }
        "声音设置" | "音量设置" | "打开声音设置" => {
            "settings_open_settings_sound_settings_activity"
        }
        "关于手机" | "关于本机" | "打开关于手机" => "settings_open_settings_device_info_settings_activity",
        "存储设置" | "打开存储设置" => "settings_open_settings_storage_dashboard_activity",
        "电池设置" | "打开电池设置" => "settings_open_step_os_settings_activity_step_os_battery_settings_activity",
        "字体大小" | "字体设置" | "打开字体设置" => "settings_open_step_os_settings_activity_step_os_screen_zoom_and_font_size_activity",
        "通知设置" | "打开通知设置" => "settings_open_step_os_settings_activity_step_os_configure_notification_settings_activity",
        "壁纸设置" | "打开壁纸设置" => "settings_open_step_os_settings_activity_step_os_wallpaper_type_settings_activity",
        "热点设置" | "打开热点设置" => "connectivitysettings_open_settings_tether_settings_activity",
        "VPN设置" | "VPN 设置" | "打开VPN设置" => "connectivitysettings_open_settings_vpn_settings_activity",
        "NFC设置" | "NFC 设置" | "打开NFC设置" => "connectivitysettings_open_settings_nfc_settings_activity",
        "移动网络设置" | "打开移动网络设置" => "connectivitysettings_open_settings_mobile_network_settings_activity",
        "语言设置" | "打开语言设置" => "settings_open_language",
        "打开手电筒" | "开启手电筒" => return Some(("open_flash_light",json!({"openState":true}))),
        "关闭手电筒" => return Some(("open_flash_light",json!({"openState":false}))),
        "音量调大" | "调大音量" | "音量+" | "增加音量" => {
            return quick_action("volume_up", "")
        }
        "音量调小" | "调小音量" | "音量-" | "减小音量" => {
            return quick_action("volume_down", "")
        }
        "静音" | "手机静音" | "把手机静音" => return quick_action("volume_mute", ""),
        _ => return None,
    };
    Some((tool, json!({})))
}

/// Observe through the registered MCP service without a model or uiautomator.
/// Never borrow another client's task or override a human pause.
pub(crate) fn visible_text(adb: &Adb, device: &Device) -> Result<Option<String>> {
    let mut native=Client::connect(adb,device)?;
    if !native.supports("stepx_gui_control") { return Ok(None); }
    let state=native.call("stepx_gui_control",json!({"op":"status"}))
        .context("无法读取 MCP 看屏服务；请在手机操控页启用并检查操控服务")?;
    ensure!(!matches!(state["phase"].as_str(),Some("running"|"paused")), "已有手机操控任务，请查看该任务的画面或结束后提取文字");
    let started=native.call("stepx_gui_control",json!({"op":"start","goal":"仅提取当前屏幕控件文字，不执行界面操作"}))?;
    let task=started["task"].as_str().filter(|s|s.len()==32&&s.bytes().all(|b|b.is_ascii_hexdigit())).context("MCP 未返回有效观察任务")?.to_owned();
    let observed=native.call("stepx_gui_control",json!({"op":"observe","task":task}));
    let stopped=native.call("stepx_gui_control",json!({"op":"stop","task":task}));
    ensure!(stopped.is_ok_and(|s|s["phase"]=="stopped"&&s["task"]==task),"MCP 观察任务尚未确认停止，未自动重试");
    let observed=observed?;
    ensure!(observed["task"]==task,"MCP 观察任务不匹配");
    Ok(Some(text_from_nodes(&observed["nodes"])?))
}

fn text_from_nodes(value:&Value)->Result<String> {
    let nodes=value.as_array().context("MCP 未返回当前画面节点")?;
    let mut lines=Vec::new();
    for node in nodes {
        if node["password"]==true {continue;}
        for field in ["text","description"] {
            if let Some(text)=node[field].as_str().map(str::trim).filter(|s|!s.is_empty()) {
                if !lines.iter().any(|s|s==text) {lines.push(text.to_owned());}
            }
        }
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_requires_actual_local_mcp_not_description_or_cloud() {
        let result = catalog(&json!({"ok":true,"payload":{"ok":true,"abilities":[
            {"name":"native","callingMethod":"LOCAL","callType":"MCP","inputSchema":{}},
            {"name":"cloud","callingMethod":"CLOUD","callType":"MCP"},
            {"name":"function","callingMethod":"LOCAL","callType":"FUNCTION_CALL"},
            {"description":"native"}]}}))
        .unwrap();
        assert_eq!(result.keys().collect::<Vec<_>>(), vec!["native"]);
        assert!(catalog(&json!({"ok":true,"payload":{"ok":false}})).is_err());
    }
    #[test]
    fn validate_rejects_unknown_arguments_types_and_native_objects() {
        let schema = json!({"required":["value"],"properties":{"value":{"type":"string"},"intent":{"type":"intent"}}});
        assert!(validate(&schema, &json!({"value":"50"})).is_ok());
        for args in [
            json!({}),
            json!({"value":50}),
            json!({"value":"50","extra":true}),
            json!({"value":"x","intent":{}}),
        ] {
            assert!(validate(&schema, &args).is_err());
        }
    }
    #[test]
    fn decode_rejects_false_success_and_ack_only() {
        for v in [
            json!({"ok":true,"payload":{"ok":true}}),
            json!({"ok":true,"payload":{"ok":false,"content":"done"}}),
            json!({"ok":true,"payload":{"ok":true,"content":"{\"isError\":true}"}}),
            json!({"ok":true,"payload":{"ok":true,"content":{"content":[{"type":"text","text":"{\"success\":false}"}]}}}),
        ] {
            assert!(decode_reply(&v).is_err());
        }
        assert_eq!(
            decode_reply(
                &json!({"ok":true,"payload":{"ok":true,"content":"屏幕尺寸(1200 x 2670)"}})
            )
            .unwrap(),
            json!("屏幕尺寸(1200 x 2670)")
        );
        assert_eq!(decode_reply(&json!({"ok":true,"payload":{"ok":true,"content":{"content":[{"type":"text","text":"{\"task\":\"a\",\"phase\":\"running\"}"}]}}})).unwrap()["phase"],"running");
    }
    #[test]
    fn quick_actions_do_not_confuse_page_opening_and_mutation() {
        assert_eq!(
            quick_action("open_mms", "").unwrap().0,
            "open_app_by_package"
        );
        assert!(quick_action("open_alipay_coupons", "").is_none());
        assert!(quick_action("open_clip_url_phone", "javascript:alert(1)").is_none());
    }

    #[test]
    fn native_write_requires_one_new_matching_row_not_any_latest_id() {
        let before = json!({"alarms":[{"id":1,"label":"old"}]});
        let after = json!({"alarms":[{"id":1,"label":"target"},{"id":2,"label":"other"}]});
        assert!(new_record_id(&before, &after, "alarms", |r| r["label"] == "target").is_err());
        let after = json!({"alarms":[{"id":3,"label":"target"}]});
        assert_eq!(
            new_record_id(&before, &after, "alarms", |r| r["label"] == "target").unwrap(),
            3
        );
        let ambiguous = json!({"alarms":[{"id":3,"label":"target"},{"id":4,"label":"target"}]});
        assert!(new_record_id(&before, &ambiguous, "alarms", |r| r["label"] == "target").is_err());
    }

    #[test]
    fn native_calendar_verification_preserves_recurrence_without_inventing_single_events() {
        for (metadata, expected) in [
            (json!({}), None),
            (json!({"recurring":false}), Some(false)),
            (json!({"isRecurring":true}), Some(true)),
            (json!({"rrule":"FREQ=WEEKLY;BYDAY=MO"}), Some(true)),
            (json!({"rdate":"20261109T090000Z"}), Some(true)),
        ] {
            let mut row = json!({"event_id":123,"title":"Planning meeting","begin":1_800_000_000_000i64,"end":1_800_003_600_000i64});
            row.as_object_mut().unwrap().extend(metadata.as_object().unwrap().clone());
            let normalized = planner_calendar_row(&row);
            let events = crate::toolbox::parse_bridge_calendar_events(&json!({"calendar_events":[normalized]}));
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].recurring, expected, "{metadata}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn production_transport_dispatches_once_and_never_replays_lost_reply() {
        for reply in [Some(json!("done")), None] {
            let fixture = test_support::Fixture::new(
                vec![
                    json!({"name":"native_test","callingMethod":"LOCAL","callType":"MCP","inputSchema":{"type":"object"}}),
                ],
                reply.clone(),
            );
            let mut client = Client::connect(&fixture.adb("exit 19"), &fixture.device()).unwrap();
            assert!(client
                .call("native_test", json!({"invented":true}))
                .is_err());
            let result = client.call("native_test", json!({}));
            assert_eq!(result.is_ok(), reply.is_some());
            let calls = fixture.calls.lock().unwrap();
            assert_eq!(calls.len(), 1, "lost reply must not replay any action");
            assert_eq!(calls[0]["payload"]["command"], "ability.call");
        }
    }

    #[cfg(unix)]
    #[test]
    fn missing_capability_and_cancel_send_zero_actions() {
        let fixture = test_support::Fixture::new(vec![], Some(json!("done")));
        let adb = fixture.adb("exit 19");
        assert_eq!(
            try_call(&adb, &fixture.device(), "not_registered", json!({})).unwrap(),
            None
        );
        assert!(fixture.calls.lock().unwrap().is_empty());
        let fixture = test_support::Fixture::new(
            vec![
                json!({"name":"native_test","callingMethod":"LOCAL","callType":"MCP","inputSchema":{"type":"object"}}),
            ],
            Some(json!("done")),
        );
        let mut client = Client::connect(&fixture.adb("exit 19"), &fixture.device()).unwrap();
        assert!(client
            .call_cancellable("native_test", json!({}), &AtomicBool::new(true))
            .is_err());
        assert!(fixture.calls.lock().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn clipboard_json_and_numeric_text_are_data_not_receipts() {
        for value in [
            json!("123456"),
            json!("{\"isError\":true}"),
            json!("null"),
            json!("  原样文字\n"),
            json!(123456),
            json!({"ok":false,"error":"copied data"}),
        ] {
            let fixture = test_support::Fixture::new(
                vec![
                    json!({"name":"common_ability_clipboard_text","callingMethod":"LOCAL","callType":"MCP","inputSchema":{"type":"object"}}),
                ],
                Some(value.clone()),
            );
            let mut client = Client::connect(&fixture.adb("exit 19"), &fixture.device()).unwrap();
            assert_eq!(
                client
                    .call("common_ability_clipboard_text", json!({}))
                    .unwrap(),
                value
            );
        }
    }

    #[test]
    fn ui_mappings_match_observed_firmware_schemas() {
        let observed: Value =
            serde_json::from_str(include_str!("../tests/fixtures/native-mcp-contracts.json"))
                .unwrap();
        let schemas = &observed["schemas"];
        for (action, argument) in [
            ("open_filemanager", ""),
            ("open_mms", ""),
            ("open_gallery", ""),
            ("open_calculator", ""),
            ("open_soundrecorder", ""),
            ("open_feedback", ""),
            ("open_gallery_trash", ""),
            ("open_file_trash", ""),
            ("open_safebox", ""),
            ("open_security_scan", ""),
            ("open_secureservice", ""),
            ("open_app_manager", ""),
            ("open_aiengine", ""),
            ("open_safety_ledger", ""),
            ("open_alphame", ""),
            ("open_amoo_voice", ""),
            ("open_backscreen", ""),
            ("open_backscreen_settings", ""),
            ("open_logreport", ""),
            ("open_data_migration", ""),
            ("open_dialer", ""),
            ("open_language_settings", ""),
            ("open_search", ""),
            ("open_search", "report"),
            ("search_files", "report"),
            ("voice_tts", "验证"),
            ("volume_up", ""),
            ("volume_down", ""),
            ("volume_mute", ""),
            ("volume_50", ""),
            ("open_clip_url_phone", "https://example.com/"),
        ] {
            let (tool, args) = quick_action(action, argument).unwrap();
            assert!(!schemas[tool].is_null(), "missing live contract: {tool}");
            validate(&schemas[tool], &args).unwrap();
        }
        for goal in [
            "打开设置",
            "Wi-Fi 设置",
            "蓝牙设置",
            "声音设置",
            "静音",
            "音量+",
            "音量-",
            "关于手机", "存储设置", "电池设置", "字体设置", "通知设置", "壁纸设置",
            "热点设置", "VPN 设置", "NFC 设置", "移动网络设置", "语言设置",
            "打开手电筒", "关闭手电筒",
        ] {
            let (tool, args) = screen_command(goal).unwrap();
            assert!(!schemas[tool].is_null(), "missing live contract: {tool}");
            validate(&schemas[tool], &args).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn independent_settings_entries_use_mcp_and_do_not_retry_rejections() {
        let observed: Value = serde_json::from_str(include_str!("../tests/fixtures/native-mcp-contracts.json")).unwrap();
        for rejected in [false, true] {
            let rows = ["open_app_by_package", "settings_open_settings_storage_dashboard_activity"]
                .into_iter().map(|name| json!({"name":name,"callingMethod":"LOCAL","callType":"MCP",
                    "inputSchema":observed["schemas"][name]})).collect();
            let fixture = test_support::Fixture::new(rows, Some(if rejected {json!({"success":false})} else {json!("accepted")}));
            let adb = fixture.adb("exit 19");
            let device = fixture.device();
            for result in [device.open_amoo(&adb), device.open_amoo_voice_settings(&adb),
                device.open_amoo_safety_center(&adb), device.open_amoo_persona(&adb),
                device.open_amoo_model_manager(&adb), crate::toolbox::open_storage_settings(&adb,&device).map(|_|String::new())] {
                assert_eq!(result.is_err(), rejected);
            }
            assert_eq!(fixture.calls.lock().unwrap().len(),6, "one dispatch per entry, never replay rejected calls");
        }
    }

    #[cfg(unix)]
    #[test]
    fn nap_uses_phone_timer_without_dnd_or_back_screen_mutations() {
        let observed: Value = serde_json::from_str(include_str!("../tests/fixtures/native-mcp-contracts.json")).unwrap();
        let fixture = test_support::Fixture::new(vec![json!({"name":"alarm_clock_timer_add",
            "callingMethod":"LOCAL","callType":"MCP","inputSchema":observed["schemas"]["alarm_clock_timer_add"]})], Some(json!("accepted")));
        let adb=fixture.adb("exit 19");
        let device=fixture.device();
        for bad in ["0","181","bad"] {
            assert!(crate::toolbox::trigger_stepos_mcp_action(&adb,&device,"start_back_screen_nap",bad).is_err());
        }
        assert!(fixture.calls.lock().unwrap().is_empty());
        let note=crate::toolbox::trigger_stepos_mcp_action(&adb,&device,"start_back_screen_nap","").unwrap();
        assert!(note.contains("20 分钟"));
        let calls=fixture.calls.lock().unwrap();
        assert_eq!(calls.len(),1);
        let args:Value=serde_json::from_str(calls[0]["payload"]["params"]["invokeParamsJson"].as_str().unwrap()).unwrap();
        assert_eq!(args["method"],"alarm_clock_timer_add");
        assert_eq!(args["params"]["seconds"],1200);
    }

    #[test]
    fn visible_nodes_never_expose_password_fields_and_deduplicate_labels() {
        let value=json!([
            {"text":"设置","description":"设置"},
            {"text":"password-secret","description":"hidden-description","password":true},
            {"text":"","description":"返回"},
            {"text":"设置"}
        ]);
        assert_eq!(text_from_nodes(&value).unwrap(),"设置\n返回");
        assert!(text_from_nodes(&Value::Null).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn reading_screen_does_not_take_over_an_active_or_paused_task() {
        for phase in ["running","paused"] {
            let fixture=test_support::Fixture::new(vec![json!({"name":"stepx_gui_control","callingMethod":"LOCAL","callType":"MCP","inputSchema":{"type":"object","properties":{"op":{"type":"string"}}}})],Some(json!({"phase":phase})));
            let error=visible_text(&fixture.adb("exit 19"),&fixture.device()).unwrap_err();
            assert!(error.to_string().contains("已有手机操控任务"));
            assert_eq!(fixture.calls.lock().unwrap().len(),1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn draft_and_dialpad_never_send_or_call_and_hangup_uses_native_tool() {
        let observed: Value =
            serde_json::from_str(include_str!("../tests/fixtures/native-mcp-contracts.json"))
                .unwrap();
        let names = ["sms_send", "dialer_show_dialpad", "dialer_hangup_call"];
        let rows = names.iter().map(|name| json!({"name":name,"callingMethod":"LOCAL","callType":"MCP","inputSchema":observed["schemas"][name]})).collect();
        let fixture = test_support::Fixture::new(rows, Some(json!("accepted")));
        let adb = fixture.adb("exit 19");
        let device = fixture.device();
        device.draft_sms(&adb, "10086", "draft fixture").unwrap();
        device.dial_number(&adb, "10086").unwrap();
        device.hangup_call(&adb).unwrap();
        let calls = fixture.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        let args: Vec<Value> = calls
            .iter()
            .map(|c| {
                serde_json::from_str(c["payload"]["params"]["invokeParamsJson"].as_str().unwrap())
                    .unwrap()
            })
            .collect();
        assert_eq!(args[0]["method"], "sms_send");
        assert_eq!(args[0]["params"]["send_directly"], false);
        assert_eq!(args[1]["method"], "dialer_show_dialpad");
        assert_eq!(args[2]["method"], "dialer_hangup_call");
    }

    #[cfg(unix)]
    #[test]
    fn native_fixture_preserves_delayed_partial_frames_and_idle_connections() {
        use std::io::{Read, Write};
        let fixture = test_support::Fixture::new(vec![], Some(json!("accepted")));
        let mut socket = std::net::TcpStream::connect(("127.0.0.1", fixture.port)).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let hello = serde_json::to_vec(&json!({"method":"connect","id":"slow-hello"})).unwrap();
        let header = (hello.len() as u32).to_be_bytes();
        // Each pause exceeds the old peer's 200ms read timeout. Partial headers
        // must survive timeout polling instead of being discarded or closed.
        socket.write_all(&header[..2]).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        socket.write_all(&header[2..]).unwrap();
        socket.write_all(&hello[..3]).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        socket.write_all(&hello[3..]).unwrap();
        let read_reply = |socket: &mut std::net::TcpStream| {
            let mut header = [0; 4];
            socket.read_exact(&mut header).unwrap();
            let mut body = vec![0; u32::from_be_bytes(header) as usize];
            socket.read_exact(&mut body).unwrap();
            serde_json::from_slice::<Value>(&body).unwrap()
        };
        assert_eq!(read_reply(&mut socket)["id"], "slow-hello");
        std::thread::sleep(Duration::from_millis(300));
        let body = serde_json::to_vec(&json!({"payload":{"command":"ability.list","id":"after-idle"}})).unwrap();
        let mut frame = (body.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(&body);
        socket.write_all(&frame).unwrap();
        assert_eq!(read_reply(&mut socket)["params"]["id"], "after-idle");
        assert!(fixture.calls.lock().unwrap().is_empty());
        // Drop must also stop a peer with a still-open idle client promptly.
        drop(fixture);
    }
}

#[cfg(all(test, unix))]
pub(crate) mod test_support {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    fn read_exact_until_stopped(
        stream: &mut std::net::TcpStream,
        mut bytes: &mut [u8],
        stop: &AtomicBool,
    ) -> std::io::Result<()> {
        while !bytes.is_empty() {
            if stop.load(Ordering::Relaxed) {
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            match stream.read(bytes) {
                Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
                Ok(count) => bytes = &mut bytes[count..],
                Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    /// Real framed AgentBus peer behind a fake ADB forward. Also used by legacy
    /// compatibility tests to explicitly advertise that no native tool exists.
    pub struct Fixture {
        dir: tempfile::TempDir,
        pub port: u16,
        serial: String,
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
        pub calls: Arc<Mutex<Vec<Value>>>,
    }
    impl std::ops::Deref for Fixture {
        type Target = tempfile::TempDir;
        fn deref(&self) -> &Self::Target {
            &self.dir
        }
    }
    impl Fixture {
        pub fn new(rows: Vec<Value>, reply: Option<Value>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let stopping = stop.clone();
            let calls = Arc::new(Mutex::new(Vec::new()));
            let records = calls.clone();
            let worker = thread::spawn(move || {
                while !stopping.load(Ordering::Relaxed) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    // BSD/macOS can inherit O_NONBLOCK from the listening
                    // socket, unlike Linux. The framed peer below expects a
                    // blocking stream with bounded shutdown polling.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_millis(100)))
                        .unwrap();
                    stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
                    stream.set_nodelay(true).unwrap();
                    loop {
                        let mut len = [0; 4];
                        // A poll timeout is only an opportunity to notice Drop.
                        // Slow scheduling or macOS delayed ACKs are not EOF, and
                        // partially received frame bytes must remain intact.
                        if read_exact_until_stopped(&mut stream, &mut len, &stopping).is_err() {
                            break;
                        }
                        let n = u32::from_be_bytes(len) as usize;
                        if n > 1024 * 1024 {
                            break;
                        }
                        let mut body = vec![0; n];
                        if read_exact_until_stopped(&mut stream, &mut body, &stopping).is_err() {
                            break;
                        }
                        let request: Value = serde_json::from_slice(&body).unwrap();
                        let response = if request["method"] == "connect" {
                            json!({"type":"res","id":request["id"],"ok":true,"payload":{"protocol":3,"server":{"version":"test"}}})
                        } else {
                            let result = if request["payload"]["command"] == "ability.list" {
                                json!({"ok":true,"abilities":rows})
                            } else {
                                records.lock().unwrap().push(request.clone());
                                let Some(value) = reply.as_ref() else { break };
                                json!({"ok":true,"content":value})
                            };
                            json!({"type":"req","method":"node.invoke.result","params":{"id":request["payload"]["id"],"ok":true,"payload":result}})
                        };
                        let body = serde_json::to_vec(&response).unwrap();
                        let mut frame = (body.len() as u32).to_be_bytes().to_vec();
                        frame.extend_from_slice(&body);
                        if stream.write_all(&frame).is_err() {
                            break;
                        }
                    }
                }
            });
            Self {
                dir,
                port,
                serial: format!("native-test-{:016x}", rand::random::<u64>()),
                stop,
                worker: Some(worker),
                calls,
            }
        }
        pub fn device(&self) -> Device {
            Device {
                transport: self.serial.clone(),
                serial: self.serial.clone(),
                model: "PZ261A".into(),
                android: "16".into(),
                product: "pandora".into(),
            }
        }
        pub fn adb(&self, body: &str) -> Adb {
            use std::os::unix::fs::PermissionsExt;
            let path = self.dir.path().join("adb");
            std::fs::write(&path,format!("#!/bin/sh\ncd {}\nprintf '%s\\n' \"$*\" >> calls\ncase \"$*\" in\n*ro.serialno*) echo {}; exit 0;;\n*forward*tcp:0*) echo {}; exit 0;;\n*get-state*) echo device; exit 0;;\nesac\n{}\n",crate::adb::shell_quote(self.dir.path().to_str().unwrap()),self.serial,self.port,body)).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Adb { path }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }
}
