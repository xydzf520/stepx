//! Sample data for design review screenshots (`--features ui-capture -- --demo`).
//! Never compiled into normal builds.

use super::*;
use stepx_companion::bridge::{AmooTurn, SimSubscription};

impl Companion {
    pub(super) fn load_demo(&mut self) {
        self.overview_back_preview = std::env::args().any(|arg| arg == "--rear-preview");
        let now = chrono::Local::now().timestamp_millis();
        let min = 60_000;
        self.selected = if std::env::var_os("STEPX_EMPTY").is_some() {
            None
        } else {
            Some(Device {
                transport: "STEPX_DEMO_DEVICE".into(),
                serial: "STEPX_DEMO_DEVICE".into(),
                model: "StepX PZ261A".into(),
                android: "16".into(),
                product: "pandora".into(),
            })
        };
        if self.selected.is_some() {
            self.seed_demo_usage();
        }
        self.devices = vec![Transport {
            id: "STEPX_DEMO_DEVICE".into(),
            state: "device".into(),
            model: "StepX_PZ261A".into(),
        }];
        self.summary = "电量  85%    ·    正在充电\n电池温度  30.8 °C\n\n手机存储  256G\n已用  96.4G    ·    可用  159.6G    ·    使用率  38%".into();
        self.capabilities = Some(Capabilities {
            send_sms: true,
            locked: false,
            default_subscription: 1,
            subscriptions: vec![
                SimSubscription {
                    id: 1,
                    slot: 0,
                    label: "中国移动".into(),
                },
                SimSubscription {
                    id: 2,
                    slot: 1,
                    label: "中国联通".into(),
                },
            ],
            protocol: 1,
        });
        self.selected_sim = Some(1);
        self.selected_call_sim = Some(1);
        self.active_call_number = None;
        self.pending_call = None;
        self.sms_loaded = true;
        self.sms_error = None;
        self.phone_loaded = true;
        self.call_logs_error = None;
        self.contacts_error = None;
        self.contacts = vec![
            stepx_companion::device::ContactEntry {
                name: "张晓峰".into(),
                number: "138 0013 8000".into(),
            },
            stepx_companion::device::ContactEntry {
                name: "林婉清".into(),
                number: "139 1024 6688".into(),
            },
            stepx_companion::device::ContactEntry {
                name: "王磊".into(),
                number: "186 2100 9527".into(),
            },
            stepx_companion::device::ContactEntry {
                name: "陈思远".into(),
                number: "137 6408 3310".into(),
            },
            stepx_companion::device::ContactEntry {
                name: "周明轩".into(),
                number: "150 0192 7741".into(),
            },
            stepx_companion::device::ContactEntry {
                name: "顺丰速运客服".into(),
                number: "95338".into(),
            },
        ];
        self.selected_contact = 0;
        self.call_logs = vec![
            stepx_companion::device::CallRecord {
                number: "138 0013 8000".into(),
                name: "张晓峰".into(),
                call_type: 1,
                date_ms: now - 18 * min,
                duration_sec: 265,
            },
            stepx_companion::device::CallRecord {
                number: "139 1024 6688".into(),
                name: "林婉清".into(),
                call_type: 3,
                date_ms: now - 46 * min,
                duration_sec: 0,
            },
            stepx_companion::device::CallRecord {
                number: "186 2100 9527".into(),
                name: "王磊".into(),
                call_type: 2,
                date_ms: now - 135 * min,
                duration_sec: 142,
            },
            stepx_companion::device::CallRecord {
                number: "137 6408 3310".into(),
                name: "陈思远".into(),
                call_type: 1,
                date_ms: now - 310 * min,
                duration_sec: 88,
            },
            stepx_companion::device::CallRecord {
                number: "021-68881024".into(),
                name: String::new(),
                call_type: 3,
                date_ms: now - 720 * min,
                duration_sec: 0,
            },
            stepx_companion::device::CallRecord {
                number: "150 0192 7741".into(),
                name: "周明轩".into(),
                call_type: 2,
                date_ms: now - 1480 * min,
                duration_sec: 410,
            },
        ];
        self.amoo_snapshot = Some(AmooSnapshot {
            turns: vec![
                AmooTurn {
                    speaker: "user".into(),
                    is_user: true,
                    text: "明天上午十点的会议改到几点了？".into(),
                },
                AmooTurn {
                    speaker: "amoo".into(),
                    is_user: false,
                    text: "根据今天 16:20 收到的短信，会议改到明天下午 2 点，地点不变，仍在 3 楼大会议室。".into(),
                },
                AmooTurn {
                    speaker: "user".into(),
                    is_user: true,
                    text: "帮我回复一下，说我会准时到。".into(),
                },
            ],
            sessions: Vec::new(),
            selected_title: String::new(),
            prepared: false,
        });
        let msg = |address: &str, body: &str, ago: i64, outgoing: bool| Message {
            address: address.into(),
            body: body.into(),
            date_ms: now - ago * min,
            outgoing,
            kind: if outgoing { 2 } else { 1 },
            read: true,
            subscription: None,
        };
        self.sms = vec![
            msg(
                "10086",
                "您本月套餐内流量已使用 80%，剩余 4.0GB。",
                30,
                false,
            ),
            msg(
                "138 0013 8000",
                "会议改到明天下午 2 点，地点不变。",
                125,
                false,
            ),
            msg("138 0013 8000", "收到，我会准时到。", 110, true),
            msg("138 0013 8000", "好的，记得带上周的数据。", 95, false),
            msg(
                "95588",
                "您尾号 1234 的账户于 10-20 14:02 支出 128.00 元。",
                1500,
                false,
            ),
            msg(
                "106980005678",
                "【顺丰速运】您的快件已到达小区驿站，取件码 5-2-1103。",
                2900,
                false,
            ),
        ];
        self.selected_thread = Some("138 0013 8000".into());
        self.fields[F_SMS_TO].set("138 0013 8000");
        let entry = |name: &str, bytes: u64, directory: bool| Entry {
            name: name.into(),
            path: format!("/sdcard/{name}"),
            bytes,
            directory,
            symlink: false,
        };
        self.files = vec![
            entry("Alarms", 0, true),
            entry("DCIM", 0, true),
            entry("Documents", 0, true),
            entry("Download", 0, true),
            entry("Music", 0, true),
            entry("Pictures", 0, true),
            entry("会议纪要-1020.pdf", 1_482_000, false),
            entry("IMG_20261019_183205.jpg", 4_215_000, false),
            entry("stepx-release.apk", 58_330_000, false),
            entry("录音 2026-10-18.m4a", 7_820_000, false),
        ];
        self.selected_file = Some(6);
        self.apps = [
            ("com.tencent.mm", "8.0.55", 2760u64),
            ("com.eg.android.AlipayGphone", "10.6.86.8000", 106868000u64),
            ("com.stepos.ailauncher", "1.4.2", 10402u64),
            ("com.sankuai.meituan", "12.24.400", 1224400u64),
            ("com.ss.android.ugc.aweme", "31.8.0", 310800u64),
            ("com.autonavi.minimap", "15.06.0", 150600u64),
            ("tv.danmaku.bili", "8.32.0", 8320200u64),
        ]
        .iter()
        .map(|(s, ver, code)| stepx_companion::apps::AppInfo {
            package: (*s).into(),
            label: stepx_companion::device::friendly_app_name(s)
                .unwrap_or(s)
                .into(),
            version_name: (*ver).into(),
            version_code: *code,
            enabled: true,
            launchable: true,
            ..stepx_companion::apps::AppInfo::default()
        })
        .collect();
        if let Ok(cat) = std::env::var("STEPX_STORE_CATEGORY") {
            match cat.trim() {
                "all" => {
                    self.app_store.category = stepx_companion::app_store::StoreCategory::All;
                }
                "social" | "global" => {
                    self.app_store.category = stepx_companion::app_store::StoreCategory::Social;
                }
                "media" => {
                    self.app_store.category = stepx_companion::app_store::StoreCategory::Media;
                }
                "productivity" | "office" => {
                    self.app_store.category =
                        stepx_companion::app_store::StoreCategory::Productivity;
                }
                "tools" | "nas" | "geek" => {
                    self.app_store.category = stepx_companion::app_store::StoreCategory::Tools;
                }
                "lifestyle" | "daily" => {
                    self.app_store.category = stepx_companion::app_store::StoreCategory::Lifestyle;
                }
                "updatable" => {
                    self.app_store.category = stepx_companion::app_store::StoreCategory::Updatable;
                }
                "installed" => {
                    self.app_store.category = stepx_companion::app_store::StoreCategory::Installed;
                }
                _ => {}
            }
        }
        if let Ok(sub) = std::env::var("STEPX_STORE_SUBTAG") {
            let trimmed = sub.trim().to_string();
            if !trimmed.is_empty() {
                self.app_store.sub_tag = Some(trimmed);
            }
        }
        if let Ok(p) = std::env::var("STEPX_STORE_PAGE") {
            if let Ok(idx) = p.parse::<usize>() {
                self.app_store.page = idx;
            }
        }
        if let Ok(search_q) = std::env::var("STEPX_STORE_SEARCH") {
            let q = search_q.trim().to_string();
            if !q.is_empty() {
                self.fields[F_STORE_SEARCH].set(&q);
                self.app_store.last_search_query = q.clone();
                let hits = if std::env::var_os("STEPX_STORE_LIVE_SEARCH").is_some() {
                    stepx_companion::app_store::search_multi_source(&q)
                        .unwrap_or_else(|_| stepx_companion::app_store::search_instant_local(&q).0)
                } else {
                    stepx_companion::app_store::search_instant_local(&q).0
                };
                self.app_store.search_results = Some(hits);
            }
        }
        self.prefetch_current_store_page_icons();
        if let Ok(uninst_pkg) = std::env::var("STEPX_STORE_UNINSTALL_CONFIRM") {
            if !uninst_pkg.trim().is_empty() {
                self.app_store.pending_uninstall = Some(uninst_pkg.trim().to_string());
            }
        }
        if let Ok(sel_pkg) = std::env::var("STEPX_STORE_SELECT") {
            let trimmed = sel_pkg.trim().to_string();
            if !trimmed.is_empty() {
                self.app_store.selected_package = Some(trimmed.clone());
                if let Ok(lb_idx) = std::env::var("STEPX_STORE_LIGHTBOX") {
                    let idx = lb_idx.parse::<usize>().unwrap_or(0);
                    let app_name = self
                        .app_store
                        .catalog
                        .iter()
                        .find(|a| a.package == trimmed)
                        .map(|a| a.name.clone())
                        .unwrap_or_else(|| trimmed.clone());
                    self.app_store.lightbox = Some(app_store_state::StoreLightboxState {
                        package: trimmed,
                        app_name,
                        screenshot_idx: idx,
                        zoom_pct: 100,
                    });
                }
            }
        }
        self.apps_loaded = true;
        self.apps_error = None;
        self.preferences.devices = vec![stepx_companion::pairing::RememberedDevice {
            serial: "PZ261A0001".into(),
            model: "StepX PZ261A".into(),
        }];
        self.preferences.selected_serial = Some("PZ261A0001".into());
        self.prefs = self.preferences.clone();
        self.status = "已连接 StepX PZ261A".into();
        self.error = false;
        self.companion_status.checked = true;
        self.companion_status.installed = true;
        self.companion_status.permission_granted = true;
        self.companion_status.pairing_permission_granted = true;
        self.companion_status.pairing_ready = true;
        self.companion_status.wireless_enabled = Some(true);
        self.companion_status.version = "0.1.32".into();
        self.companion_status.message = "已待机，等待电脑应用配置".into();
        self.companion_status.running = true;
        self.back_screen.companion_installed = Some(true);
        self.back_screen.companion_permission = true;
        self.back_screen.companion_status = "v0.1.32 · 已待机，等待电脑应用配置".into();
        if let Ok(tab_str) = std::env::var("STEPX_BACK_TAB") {
            if let Ok(tab) = tab_str.trim().parse::<u8>() {
                self.back_screen.tab = tab;
            }
        }
        if std::env::var_os("STEPX_CONNECT_WIRELESS").is_some() {
            self.selected = Some(Device {
                transport: "192.168.1.5:46315".into(),
                serial: "ZYPZ261A01M69200105".into(),
                model: "StepX PZ261A".into(),
                android: "16".into(),
                product: "pandora".into(),
            });
            self.devices = vec![Transport {
                id: "192.168.1.5:46315".into(),
                state: "device".into(),
                model: "PZ261A".into(),
            }];
            self.connect_guide_step = None;
        }
        if let Ok(step_str) = std::env::var("STEPX_CONNECT_GUIDE_STEP") {
            if step_str.trim() == "none" {
                self.connect_guide_step = None;
            } else if let Ok(step) = step_str.trim().parse::<u8>() {
                self.connect_guide_step = Some(step);
            }
        }
        self.back_screen.stepos_suite.checked = true;
        self.back_screen.stepos_suite.wellbeing.screen_time_minutes = 164;
        self.back_screen.stepos_suite.wellbeing.unlock_count = 38;
        self.back_screen.stepos_suite.wellbeing.alphame_running = true;
        self.back_screen.stepos_suite.wellbeing.top_apps = vec![
            ("Amoo 助手".into(), "com.stepos.ailauncher".into(), 68),
            ("微信".into(), "com.tencent.mm".into(), 46),
            ("Chrome 浏览器".into(), "com.android.chrome".into(), 32),
            ("相册".into(), "com.stepos.media.gallery".into(), 18),
        ];
        self.back_screen.stepos_suite.calendar_events = vec![
            stepx_companion::toolbox::PhoneCalendarEvent {
                id: 101,
                title: "第四季度产品与桌面协同评审会".into(),
                dtstart: now + 60 * min,
                dtend: now + 120 * min,
                all_day: false,
                location: "3 楼大会议室".into(),
                description: "讨论手机日历、闹钟与背屏双向联动体验".into(),
                source: "双端已同步".into(),
            },
            stepx_companion::toolbox::PhoneCalendarEvent {
                id: 102,
                title: "晚间重点待办复盘与周报归档".into(),
                dtstart: now + 240 * min,
                dtend: now + 300 * min,
                all_day: false,
                location: String::new(),
                description: "由 StepX 桌面端直接同步创建".into(),
                source: "双端已同步".into(),
            },
            stepx_companion::toolbox::PhoneCalendarEvent {
                id: 900_001,
                title: "StepX 跨端日历与相册架构同步".into(),
                dtstart: now + 1440 * min,
                dtend: now + 1500 * min,
                all_day: false,
                location: "线上协同会议室".into(),
                description: "电脑系统日历双向自动同步".into(),
                source: "电脑日历".into(),
            },
        ];
        self.back_screen.stepos_suite.alarms = vec![
            stepx_companion::toolbox::PhoneAlarmItem {
                id: 1,
                hour: 8,
                minutes: 0,
                enabled: true,
                days_of_week: 31,
                vibrate: true,
                message: "晨间重点事项与通勤提醒".into(),
            },
            stepx_companion::toolbox::PhoneAlarmItem {
                id: 2,
                hour: 13,
                minutes: 45,
                enabled: true,
                days_of_week: 0,
                vibrate: true,
                message: "下午评审会提前 15 分钟提醒".into(),
            },
            stepx_companion::toolbox::PhoneAlarmItem {
                id: 3,
                hour: 22,
                minutes: 30,
                enabled: false,
                days_of_week: 127,
                vibrate: true,
                message: "晚间休息与次日日程预览".into(),
            },
        ];
        let demo_thumb =
            stepx_companion::resources::root().join("desktop/assets/amoo_tti_portrait.png");
        let thumb_opt = demo_thumb
            .is_file()
            .then(|| demo_thumb.to_string_lossy().to_string());
        self.back_screen.gallery_checked = true;
        self.back_screen.gallery_items = vec![
            stepx_companion::toolbox::PhoneGalleryItem {
                remote_path: "/sdcard/DCIM/Camera/IMG_20261025_142018.jpg".into(),
                name: "IMG_20261025_142018.jpg".into(),
                category: "camera".into(),
                size_bytes: 3_842_190,
                modified_ms: now - 35 * min,
                local_thumb_path: thumb_opt.clone(),
            },
            stepx_companion::toolbox::PhoneGalleryItem {
                remote_path: "/sdcard/Pictures/Screenshots/StepX_20261025_135502.png".into(),
                name: "StepX_20261025_135502.png".into(),
                category: "screenshot".into(),
                size_bytes: 1_248_640,
                modified_ms: now - 90 * min,
                local_thumb_path: thumb_opt.clone(),
            },
            stepx_companion::toolbox::PhoneGalleryItem {
                remote_path: "/sdcard/DCIM/Camera/IMG_20261024_183205.jpg".into(),
                name: "IMG_20261024_183205.jpg".into(),
                category: "camera".into(),
                size_bytes: 4_215_000,
                modified_ms: now - 1180 * min,
                local_thumb_path: thumb_opt.clone(),
            },
            stepx_companion::toolbox::PhoneGalleryItem {
                remote_path: "/sdcard/Pictures/StepX_RearMatrix_Preview.png".into(),
                name: "StepX_RearMatrix_Preview.png".into(),
                category: "pictures".into(),
                size_bytes: 612_400,
                modified_ms: now - 2400 * min,
                local_thumb_path: thumb_opt,
            },
        ];
        self.back_screen.stepos_suite.controls = stepx_companion::toolbox::PhoneSystemControls {
            dnd_on: false,
            wifi_on: true,
            bluetooth_on: true,
            brightness_percent: 65,
        };
        self.back_screen.stepos_suite.phone_clipboard =
            "【顺丰速运】取件码 5-2-1103 · 3 楼大会议室 14:00 评审".into();
        self.back_screen.stepos_suite.thermal_summary =
            "多温区实时温控：机身 31.4°C · 电池 29.5°C · NPU 43.2°C".into();
        self.back_screen.stepos_suite.last_action_note = "已同步手机日程与闹钟状态".into();
        self.init_demo_push();
        // Alternate state set: confirmations, transient panels.
        if std::env::var_os("STEPX_DEMO_ALT").is_some() {
            self.trash_confirm = true;
            self.fields[F_SMS_BODY].set("收到，我会准时到。");
            self.pending_sms_plan = Some(SmsPlan {
                request_id: "demo".into(),
                number: "138 0013 8000".into(),
                body: "收到，我会准时到。".into(),
                subscription: 1,
                subscription_label: "卡 1 · 中国移动".into(),
                parts: 1,
            });
            self.content = "会议通知\n时间：10 月 22 日 14:00\n地点：3 楼大会议室\n参会人：产品组、设计组\n议题：第四季度版本规划\n请提前准备上周数据\n联系人：王磊".into();
            self.output_page = Page::Amoo;
            self.amoo_poll_remaining = 2;
            self.fields[F_AMOO].set("总结一下这条通知");
        }
    }
}
