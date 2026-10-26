use std::{
    process::Command,
    time::{Duration, Instant},
};
use stepx_companion::{
    adb::{run_process, shell_quote, Adb},
    bridge::{parse_amoo_snapshot, parse_bridge_output, parse_capabilities, parse_sms_status},
    device::{format_summary, friendly_app_name, parse_devices, visible_text, DeviceMetrics},
    pairing::{pair_code, parse_services, validate_endpoint, Preferences, QrSession},
    storage::{format_bytes, parse_entries, validate_entry_name, validate_path, Entry},
};

#[test]
fn parses_multiple_transports_without_guessing_a_target() {
    let devices = parse_devices("List of devices attached\nstep-a device usb:1 model:PZ261A product:pandora\nother unauthorized\n10.0.0.5:3456 offline\n");
    assert_eq!(devices.len(), 3);
    assert_eq!(devices[1].state, "unauthorized");
    assert_eq!(devices[2].id, "10.0.0.5:3456");
}

#[test]
fn pairing_and_connection_ports_are_distinct() {
    let services = parse_services("List of discovered mdns services\nstudio-stepx-x _adb-tls-pairing._tcp. 192.168.1.4:37000\nadb-device-x _adb-tls-connect._tcp. 192.168.1.4:39000\nbad _adb-tls-connect._tcp. ;reboot\n");
    assert_eq!(services.len(), 2);
    assert_eq!(services[0].kind, "_adb-tls-pairing._tcp");
    assert_eq!(services[1].endpoint, "192.168.1.4:39000");
    assert!(validate_endpoint("[fd00::1]:5555").is_ok());
    for endpoint in [
        "device",
        "127.0.0.1:0",
        "0.0.0.0:5555",
        "224.0.0.1:5555",
        "1.2.3.4:5555;reboot",
    ] {
        assert!(validate_endpoint(endpoint).is_err());
    }
}

#[test]
fn automatic_code_pairing_requires_one_unambiguous_device() {
    use stepx_companion::pairing::unique_code_endpoint;
    let mut services = parse_services("adb-phone _adb-tls-pairing._tcp. 192.168.1.4:37000\nadb-phone _adb-tls-connect._tcp. 192.168.1.4:39000\nstudio-qr _adb-tls-pairing._tcp. 192.168.1.5:38000\n");
    assert_eq!(
        unique_code_endpoint(&services).unwrap(),
        "192.168.1.4:37000"
    );
    services.extend(parse_services(
        "adb-other _adb-tls-pairing._tcp. 192.168.1.6:37000\n",
    ));
    assert!(unique_code_endpoint(&services).is_err());
    assert!(unique_code_endpoint(&[]).is_err());
}

#[test]
fn qr_sessions_are_unique_and_temporary() {
    let first = QrSession::new().unwrap();
    let second = QrSession::new().unwrap();
    assert_ne!(first.service, second.service);
    assert!(!first.expired());
    assert!(image::open(&first.path).is_ok());
    let path = first.path.clone();
    drop(first);
    assert!(!path.exists());
}

#[test]
fn preferences_do_not_contain_pairing_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("devices.json");
    let mut prefs = Preferences::default();
    prefs.remember("serial-a".into(), "PZ261A".into());
    prefs.remember("serial-a".into(), "PZ261A".into());
    prefs.save_to(&path).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["devices"].as_array().unwrap().len(), 1);
    assert_eq!(value.as_object().unwrap().len(), 4);
    assert_eq!(value["first_connect_done"], true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn storage_blocks_traversal_and_preserves_unicode_delimiters() {
    for path in [
        "/data/private",
        "/sdcard/../data",
        "/sdcard/./Download",
        "/sdcard/Download/../../etc",
        "/sdcard2",
        "/sdcard/a\nfile",
    ] {
        assert!(validate_path(path).is_err(), "{path}");
    }
    assert!(validate_path("/sdcard/Download/中文 ' $(whoami).txt").is_ok());
    let list = parse_entries(
        "81a4\n12\n/storage/emulated/0/空 格 ' :file.txt\0\n".trim_end(),
        "/storage/emulated/0",
        "/sdcard",
    )
    .unwrap();
    assert_eq!(list[0].bytes, 12);
    assert_eq!(list[0].path, "/sdcard/空 格 ' :file.txt");
    assert!(parse_entries("81a4\n1\n/other/file\0", "/storage/emulated/0", "/sdcard").is_err());
}

#[test]
fn hierarchy_unescapes_content_and_omits_password_fields() {
    let text = visible_text(r#"<hierarchy><node text="你好 &amp; welcome"/><node text="secret" password="true"/><node text="" content-desc="发送"/></hierarchy>"#).unwrap();
    assert_eq!(text, "你好 & welcome\n\n发送");
    assert!(visible_text("<hierarchy/>").is_err());
}

#[test]
fn sms_framing_keeps_body_that_looks_like_metadata() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let body = "你好, body=world\nRow: 1 address=wrong\n末行\n";
    let encoded = STANDARD.encode(format!("Row: 0 body={body}\n"));
    assert_eq!(
        stepx_companion::sms::decode_field(&encoded, "body").unwrap(),
        body
    );
    assert!(
        stepx_companion::sms::decode_field(&STANDARD.encode("No result found."), "body").is_err()
    );
}

#[test]
fn status_uses_live_battery_and_storage_values() {
    let formatted = format_summary(" level: 73\n status: 2\n temperature: 281\n/dev/fuse 213G 25G 189G 12% /storage/emulated\n");
    assert!(formatted.contains("73%"));
    assert!(formatted.contains("28.1"));
    assert!(formatted.contains("189G"));
    let metrics = DeviceMetrics::from_summary(&formatted);
    assert_eq!(metrics.battery_level, Some(73));
    assert_eq!(metrics.charging_state, "正在充电");
    assert_eq!(metrics.temperature, "28.1 °C");
    assert_eq!(metrics.storage_total, "213G");
    assert_eq!(metrics.storage_used, "25G");
    assert_eq!(metrics.storage_avail, "189G");
    assert_eq!(metrics.storage_percent, Some(12));
}

#[test]
fn file_sizes_and_kind_badges_are_human_friendly() {
    assert_eq!(format_bytes(512), "512 B");
    assert_eq!(format_bytes(2048), "2.0 KB");
    assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
    let entry = Entry {
        name: "photo.png".into(),
        path: "/sdcard/photo.png".into(),
        bytes: 2048,
        directory: false,
        symlink: false,
    };
    assert_eq!(entry.kind_badge(), "图片");
    assert_eq!(
        friendly_app_name("com.stepos.ailauncher"),
        Some("Amoo 智能助手")
    );
}

#[cfg(unix)]
#[test]
fn shell_arguments_cannot_execute_metacharacters() {
    for value in ["hello world", "中文' $(exit 42);\nnext", "`whoami`", "-n"] {
        let output = Command::new("sh")
            .args(["-c", &format!("printf %s {}", shell_quote(value))])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), value);
    }
}

#[cfg(unix)]
#[test]
fn timeout_kills_process_group_without_waiting_for_inherited_pipes() {
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 10"]);
    let started = Instant::now();
    assert!(run_process(command, None, Duration::from_millis(100)).is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn pair_code_is_passed_on_stdin_not_process_arguments() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("adb");
    std::fs::write(&path, "#!/bin/sh\n[ \"$#\" = 2 ] || exit 1\n[ \"$1\" = pair ] || exit 1\nread -r code\n[ \"$code\" = 123456 ] || exit 1\nprintf 'Successfully paired\\n'\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let adb = Adb { path };
    assert!(pair_code(&adb, "192.168.1.4:37891", "123456").is_ok());
    assert!(pair_code(&adb, "192.168.1.4:37891", "12345").is_err());
}

#[cfg(unix)]
#[test]
fn device_identity_change_prevents_input_on_reassigned_transport() {
    use std::os::unix::fs::PermissionsExt;
    use stepx_companion::device::Device;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("adb");
    let log = directory.path().join("calls");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf 'call\\n' >> {}\nprintf 'different-phone\\n'\n",
            shell_quote(log.to_str().unwrap())
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let device = Device {
        transport: "192.168.1.4:5555".into(),
        serial: "expected-phone".into(),
        model: "PZ261A".into(),
        android: "16".into(),
        product: "pandora".into(),
    };
    assert!(device.key(&Adb { path }, 3).is_err());
    assert_eq!(std::fs::read_to_string(log).unwrap(), "call\n");
}

#[cfg(unix)]
#[test]
fn root_alias_cannot_be_trashed_or_deleted() {
    use std::os::unix::fs::PermissionsExt;
    use stepx_companion::{device::Device, storage};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("adb");
    std::fs::write(&path, "#!/bin/sh\ncase \"$*\" in\n*getprop*) printf 'phone\\n';;\n*readlink*) printf '/storage/emulated/0\\n';;\n*) exit 0;;\nesac\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let device = Device {
        transport: "phone".into(),
        serial: "phone".into(),
        model: "PZ261A".into(),
        android: "16".into(),
        product: "pandora".into(),
    };
    let adb = Adb { path };
    assert!(storage::delete(&adb, &device, "/sdcard/root-alias/")
        .unwrap_err()
        .to_string()
        .contains("根目录"));
    assert!(storage::trash(&adb, &device, "/sdcard/root-alias/")
        .unwrap_err()
        .to_string()
        .contains("根目录"));
}

#[test]
fn entry_name_validation_blocks_separators_and_special_names() {
    assert_eq!(
        validate_entry_name("  工作文档 2026  ").unwrap(),
        "工作文档 2026"
    );
    for bad in ["", "   ", ".", "..", "a/b", "a\\b", "a\nb"] {
        assert!(validate_entry_name(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn bridge_parser_handles_capabilities_amoo_and_sms_status() {
    let raw = "WARNING: something\nSTEPX_RESULT:{\"ok\":true,\"send_sms\":true,\"locked\":false,\"default_subscription\":2,\"protocol\":1,\"subscriptions\":[{\"id\":1,\"slot\":1,\"label\":\"中国移动\"},{\"id\":2,\"slot\":2,\"label\":\"中国联通 (+8613800000000)\"}]}\n";
    let val = parse_bridge_output(raw, "").unwrap();
    let caps = parse_capabilities(&val).unwrap();
    assert!(caps.send_sms);
    assert!(!caps.locked);
    assert_eq!(caps.default_subscription, 2);
    assert_eq!(caps.subscriptions.len(), 2);
    assert_eq!(caps.subscriptions[0].label, "中国移动");
    assert_eq!(caps.subscriptions[1].slot, 2);

    let amoo_raw = "STEPX_RESULT:{\"ok\":true,\"prepared\":true,\"rows\":[[{\"id\":\"com.stepos.ailauncher:id/user_text\",\"text\":\"今天天气\",\"description\":\"\"}],[{\"id\":\"com.stepos.ailauncher:id/ai_markdown\",\"text\":\"今天天气晴朗，24°C\",\"description\":\"\"}]]}\n";
    let amoo_val = parse_bridge_output(amoo_raw, "").unwrap();
    let snap = parse_amoo_snapshot(&amoo_val);
    assert!(snap.prepared);
    assert_eq!(snap.turns.len(), 2);
    assert!(snap.turns[0].is_user);
    assert!(!snap.turns[1].is_user);

    let sms_raw = "STEPX_RESULT:{\"ok\":true,\"id\":\"sms-1\",\"state\":\"sent\",\"parts\":1,\"completed\":1,\"updated_ms\":1700000000000}\n";
    let sms_val = parse_bridge_output(sms_raw, "").unwrap();
    let status = parse_sms_status(&sms_val);
    assert_eq!(status.state, "sent");
    assert!(!status.is_pending_or_unknown());
    assert!(status.summary_label().contains("已发送成功"));

    let err_raw = "STEPX_RESULT:{\"ok\":false,\"error\":\"当前未检测到可用 SIM 卡\"}\n";
    assert!(parse_bridge_output(err_raw, "")
        .unwrap_err()
        .to_string()
        .contains("未检测到可用 SIM 卡"));
}

#[test]
fn openclaw_gateway_token_matches_stepos_hmac_sha256_derivation() {
    use stepx_companion::device::derive_openclaw_token;
    assert_eq!(
        derive_openclaw_token("23392eef6ce29cb0"),
        "c8037d4ac8507d6bbde7e57b20318cfc9e6edd2dc490747b23cb3b97e3731f10"
    );
}

#[test]
fn backscreen_matrix_preserves_379_active_leds_and_12_corner_cutouts() {
    use stepx_companion::device::{is_backscreen_corner, BackScreenPattern};
    let mut corners = 0;
    let mut active = 0;
    for y in 0..23 {
        for x in 0..17 {
            if is_backscreen_corner(x, y) {
                corners += 1;
            } else {
                active += 1;
            }
        }
    }
    assert_eq!(corners, 12);
    assert_eq!(active, 379);
    assert_eq!(corners + active, 391);

    for pat in BackScreenPattern::ALL {
        let frame = pat.frame_391(8, false);
        assert_eq!(frame.len(), 391);
        assert!(frame.iter().any(|&v| v > 0));
        assert_eq!(frame[0], 0);
        assert_eq!(frame[16], 0);
        assert_eq!(frame[22 * 17], 0);
        assert_eq!(frame[22 * 17 + 16], 0);
    }
}

#[test]
fn call_logs_and_contacts_parse_cleanly() {
    use stepx_companion::device::{parse_call_logs, parse_contacts};
    let raw_calls = "Row: 0 number=13800138000, name=王磊, type=1, date=1700000050000, duration=125\nRow: 1 number=13910245678, name=NULL, type=3, date=1700000090000, duration=0\n";
    let calls = parse_call_logs(raw_calls);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].type_label(), "未接");
    assert_eq!(calls[1].type_label(), "呼入");
    assert_eq!(calls[1].format_duration(), "2分5秒");

    let raw_contacts = "Row: 0 display_name=王磊, data1=138 0013 8000\nRow: 1 display_name=王磊, data1=138 0013 8000\nRow: 2 display_name=林晓雨, data1=139 1024 5678\n";
    let contacts = parse_contacts(raw_contacts);
    assert_eq!(contacts.len(), 2);
}

#[test]
fn structured_phone_data_preserves_delimiters_empty_results_and_all_message_states() {
    use serde_json::json;
    use stepx_companion::{
        device::{parse_call_records, parse_contact_records},
        sms::parse_messages,
    };
    let body =
        "你好, body=NULL\nRow: 1 _id=2, date=123, type=2, address=wrong, body=假的\n\n末行\n";
    let rows: Vec<_> = (0..75)
        .map(|i| {
            json!({
                "address": "10086", "body": body, "date": (1700000000000_i64 + i).to_string(),
                "type": (i % 6 + 1).to_string(), "read": "0", "sub_id": "7"
            })
        })
        .collect();
    let messages = parse_messages(&json!({"rows": rows})).unwrap();
    assert_eq!(messages.len(), 75);
    assert!(messages
        .iter()
        .all(|m| m.body == body && m.subscription == Some(7)));
    assert_eq!(messages[2].status_label(), "草稿");
    assert_eq!(messages[4].status_label(), "发送失败");
    assert!(!messages[0].outgoing);
    assert!(parse_messages(&json!({"rows": []})).unwrap().is_empty());
    assert!(parse_messages(&json!({"error": "permission denied"})).is_err());
    assert!(parse_messages(&json!({"rows": [{"type": "not a number"}]})).is_err());
    let contacts = parse_contact_records(&json!({"rows": [
        {"display_name": "张三, data1=保留\n完整姓名", "data1": "+86 138 (0013) 8000"}
    ]}))
    .unwrap();
    assert_eq!(contacts[0].name, "张三, data1=保留\n完整姓名");
    let calls = parse_call_records(&json!({"rows": [
        {"number": null, "name": null, "type": "3", "date": "1700000000000", "duration": "0"}
    ]}))
    .unwrap();
    assert_eq!(calls.len(), 1); // Hidden numbers are still real call records.
    assert!(calls[0].number.is_empty());
    assert_eq!(calls[0].type_label(), "未接");
}

#[test]
fn storage_prefers_userdata_and_never_invents_missing_capacity() {
    let metrics = DeviceMetrics::from_summary(&format_summary(
        "level: 51\n/dev/fuse 213G 27G 186G 13% /storage/emulated\n/dev/block/dm-48 213G 28G 185G 14% /data\n"
    ));
    assert_eq!(metrics.storage_used, "28G");
    assert_eq!(metrics.storage_total, "213G");
    assert_eq!(metrics.storage_percent, Some(14));
    let missing = DeviceMetrics::from_summary(&format_summary("level: 51\n"));
    assert!(missing.storage_total.is_empty());
    assert_eq!(missing.storage_percent, None);
}

#[cfg(unix)]
#[test]
fn sms_bridge_errors_never_fabricate_success_or_write_phone_records() {
    use std::os::unix::fs::PermissionsExt;
    use stepx_companion::{bridge, device::Device};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("adb");
    let log = dir.path().join("calls");
    std::fs::write(
        &path,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {}
case "$*" in
*getprop*) printf 'phone\n';;
*stat*) printf 'READY\n';;
*app_process*) read -r request; printf 'STEPX_RESULT:{{"ok":false,"error":"permission denied"}}\n';;
*) exit 9;;
esac
"#,
            shell_quote(log.to_str().unwrap())
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let adb = Adb { path };
    let device = Device {
        transport: "phone".into(),
        serial: "phone".into(),
        model: "PZ261A".into(),
        android: "16".into(),
        product: "pandora".into(),
    };
    let plan = bridge::SmsPlan {
        request_id: "0123456789abcdef0123456789abcdef".into(),
        number: "10086".into(),
        body: "测试".into(),
        subscription: 7,
        subscription_label: "SIM 2".into(),
        parts: 1,
    };
    let status = bridge::sms_send(&adb, &device, &plan).unwrap();
    assert_eq!(status.state, "unknown");
    assert_eq!(status.completed, 0);
    assert!(bridge::sms_status(&adb, &device, &plan.request_id).is_err());
    assert!(bridge::sms_prepare(&adb, &device, "10086", "测试", Some(7)).is_err());
    assert!(device.messages(&adb).is_err());
    assert!(device.call_logs(&adb).is_err());
    assert!(device.contacts(&adb).is_err());
    let calls = std::fs::read_to_string(log).unwrap();
    assert!(!calls.contains("service call"));
    assert!(!calls.contains("content insert"));
    assert!(!calls.contains("appops set"));
}

#[test]
fn adb_onboarding_is_only_automatic_for_new_users_and_remembers_skip() {
    let mut prefs = Preferences::default();
    assert!(prefs.should_show_adb_guide());
    prefs.adb_guide_seen = true;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("devices.json");
    prefs.save_to(&path).unwrap();
    let restored: Preferences = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(!restored.should_show_adb_guide());
    let legacy: Preferences =
        serde_json::from_str(r#"{"devices":[],"selected_serial":null,"first_connect_done":true}"#)
            .unwrap();
    assert!(!legacy.should_show_adb_guide());
    let legacy_device: Preferences = serde_json::from_str(
        r#"{"devices":[{"serial":"example","model":"StepX"}],"selected_serial":"example"}"#,
    )
    .unwrap();
    assert!(!legacy_device.should_show_adb_guide());
    let mut connected = Preferences::default();
    connected.remember("example".into(), "StepX".into());
    assert!(!connected.should_show_adb_guide());
}

#[cfg(unix)]
#[test]
fn low_level_protocols_batch_updates_ota_companion_and_sms_prepare() {
    use std::os::unix::fs::PermissionsExt;
    use stepx_companion::{bridge, companion_app, device::Device, ota, updates};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("adb");
    let log = dir.path().join("calls");
    std::fs::write(
        &path,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> {}
case "$*" in
*ro.serialno*)
    printf 'phone-e2e\n'
    ;;
*resolve-activity*stepos.settings.SYSTEM_UPDATE_SETTINGS*)
    printf 'Step AOS 1.2.0\n2026-10-01\npriority=0\ncom.stepos.update/.SystemUpgradeListActivity\n'
    ;;
*stepx-ota.jar*stat*)
    printf 'READY\n'
    ;;
*OtaProbe*)
    printf 'STEPX_OTA_RESULT:{{"protocol":1,"current_version":"Step AOS 1.2.0_20261001_120000","model":"PZ261A","environment":"prod","identity_complete":true,"identity_fields":{{"dsid":true}},"install_status":{{"internal_permission":true,"shop_demo":false,"system_update":{{"status":0,"channel":"stable","mask_id":"Step AOS 1.2.0"}}}},"channels":[{{"channel":"current","server_code":200,"http_status":200,"releases":[]}},{{"channel":"stable","server_code":200,"http_status":200,"releases":[]}},{{"channel":"beta","server_code":200,"http_status":200,"releases":[]}},{{"channel":"daily","server_code":200,"http_status":200,"releases":[]}}]}}\n'
    ;;
*"<<<STEPX_DUMP>>>"*)
    printf 'package:/data/app/com.stepx.companion.app/base.apk\n'
    printf '\n<<<STEPX_DUMP>>>\nversionName=0.1.32\nUser 0:\n  com.stepos.permission.MANAGE_BACK_SCREEN: granted=true\n  android.permission.CAMERA: granted=true\n  android.permission.POST_NOTIFICATIONS: granted=true\n  android.permission.READ_PHONE_STATE: granted=true\n  android.permission.MANAGE_DEBUGGING: granted=true\n  android.permission.WRITE_SECURE_SETTINGS: granted=true\n'
    printf '\n<<<STEPX_PID>>>\n4321\n'
    printf '\n<<<STEPX_STATUS>>>\n{{"running":true,"mode":"standby","lastGameScore":12,"bestScore":30,"uptimeMs":10000}}\n'
    printf '\n<<<STEPX_PAIRING>>>\n{{"updatedAtMs":1700000000000,"pairingBridgeReady":true,"pairingSupported":true}}\n'
    printf '\n<<<STEPX_TIME>>>\n1700000001\n'
    printf '\n<<<STEPX_UPTIME>>>\n10.50 20.00\n'
    printf '\n<<<STEPX_BG>>>\n{{"enabled":true,"active":false,"lastError":"","leaseExpiresElapsedMs":5000}}\n'
    printf '\n<<<STEPX_AMOO_META>>>\nstep/pandora/pandora:16/BP2A.250605.031.A3/20260208:user/release-keys\n36\n03a799f776c8cb094b4da040eb51173b3588f8be79fd8e325d01b2753b7ffcf5  /system/priv-app/AgentBus/AgentBus.apk\n'
    printf '\n<<<STEPX_WIFI>>>\n1\n'
    printf '\n<<<STEPX_DOZE>>>\nuser,com.stepx.companion.app,10234\n'
    printf '\n<<<STEPX_OEM>>>\ncloud freeze white apps:\nuser freeze white apps:\nfreeze white app: com.stepx.companion.app\nlocal freeze white apps:\n'
    ;;
*stepx-bridge.jar*stat*)
    printf 'READY\n'
    ;;
*com.stepx.companion.Bridge*)
    read -r req
    case "$req" in
    *sms_prepare*)
        printf 'STEPX_RESULT:{{"ok":true,"request_id":"req-123","number":"10086","body":"hello","subscription":3,"subscription_label":"中国移动 SIM 1","parts":1}}\n'
        ;;
    *)
        printf 'STEPX_RESULT:{{"ok":false,"error":"unexpected op"}}\n'
        ;;
    esac
    ;;
*)
    exit 9
    ;;
esac
"#,
            shell_quote(log.to_str().unwrap())
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let adb = Adb { path };
    let device = Device {
        transport: "phone-e2e".into(),
        serial: "phone-e2e".into(),
        model: "PZ261A".into(),
        android: "16".into(),
        product: "pandora".into(),
    };

    // 1. updates::phone_version runs in 2 ADB calls (verify + 1 batched script)
    let pv = updates::phone_version(&adb, &device).unwrap();
    assert_eq!(pv.firmware, "Step AOS 1.2.0");
    assert_eq!(pv.security_patch, "2026-10-01");
    assert!(pv.has_update_page);
    let calls_after_pv = std::fs::read_to_string(&log).unwrap().lines().count();
    assert_eq!(calls_after_pv, 2);

    // 2. ota::install_status caches stepx-ota.jar on first call (3 calls: verify + stat + OtaProbe)
    // and reuses cache on second call (2 calls: verify + OtaProbe, zero push/stat)
    let st1 = ota::install_status(&adb, &device).unwrap();
    assert_eq!(st1.internal_permission, Some(true));
    assert_eq!(st1.shop_demo, Some(false));
    let calls_after_ota1 = std::fs::read_to_string(&log).unwrap().lines().count();
    assert_eq!(calls_after_ota1 - calls_after_pv, 3);

    let report = ota::check(&adb, &device).unwrap();
    assert_eq!(report.current_version, "Step AOS 1.2.0_20261001_120000");
    let calls_after_ota2 = std::fs::read_to_string(&log).unwrap().lines().count();
    assert_eq!(calls_after_ota2 - calls_after_ota1, 2);

    // 3. companion_app::inspect runs in 2 ADB calls (verify + 1 unified script) instead of 18
    let comp = companion_app::inspect(&adb, &device).unwrap();
    assert!(comp.installed);
    assert_eq!(comp.version, "0.1.32");
    assert!(comp.permission_granted);
    assert!(comp.running);
    assert!(comp.pairing_ready);
    assert_eq!(comp.doze_exempt, Some(true));
    assert_eq!(comp.oem_freezer_exempt, Some(true));
    let calls_after_comp = std::fs::read_to_string(&log).unwrap().lines().count();
    assert_eq!(calls_after_comp - calls_after_ota2, 2);

    // 4. bridge::sms_prepare resolves subscription_label in a single JVM call (no separate capabilities call)
    bridge::mark_bridge_jar_cached(&device.transport);
    let plan = bridge::sms_prepare(&adb, &device, "10086", "hello", Some(3)).unwrap();
    assert_eq!(plan.subscription, 3);
    assert_eq!(plan.subscription_label, "中国移动 SIM 1");
    let calls_after_sms = std::fs::read_to_string(&log).unwrap().lines().count();
    assert_eq!(calls_after_sms - calls_after_comp, 2); // verify + 1 single app_process call
}
