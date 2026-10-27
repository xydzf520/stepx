use super::super::{
    expansion_state::ExpansionAction as EA, icons::UiIcon, theme::*, widgets::*, *,
};
use facloud_ui::{assets::asset_cache::AssetSource, elements::*, fonts::Weight};
use serde_json::Value;
fn label(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").into()
}
fn state(v: &Value) -> &'static str {
    match v["state"].as_str().unwrap_or("") {
        "archived" => "已迁移",
        "restored" => "已恢复",
        "failed" => "未完成",
        "verified" => "等待手机确认",
        _ => "待迁移",
    }
}
fn timeline_date_bucket(j: &Value) -> String {
    if let Some(group) = j["dateGroup"].as_str().filter(|s| !s.trim().is_empty()) {
        return group.to_string();
    }
    let ts = j["mtimeMs"]
        .as_i64()
        .or_else(|| j["modified"].as_i64())
        .or_else(|| j["archivedAtMs"].as_i64())
        .or_else(|| j["archivedAt"].as_i64())
        .unwrap_or(0);
    if ts > 0 {
        if let Some(dt) = chrono::DateTime::from_timestamp_millis(ts) {
            let local = dt.with_timezone(&chrono::Local);
            let today = chrono::Local::now().date_naive();
            let d = local.date_naive();
            if d == today {
                return format!("{} · 今天", local.format("%Y-%m-%d"));
            } else if Some(d) == today.pred_opt() {
                return format!("{} · 昨天", local.format("%Y-%m-%d"));
            }
            return local.format("%Y-%m-%d").to_string();
        }
    }
    let name = j["name"].as_str().unwrap_or("");
    let bytes = name.as_bytes();
    if bytes.len() >= 8 {
        for w in bytes.windows(8) {
            if w.starts_with(b"20") && w.iter().all(|b| b.is_ascii_digit()) {
                let y = std::str::from_utf8(&w[0..4]).unwrap_or("");
                let m = std::str::from_utf8(&w[4..6]).unwrap_or("");
                let d = std::str::from_utf8(&w[6..8]).unwrap_or("");
                return format!("{y}-{m}-{d}");
            }
        }
    }
    "最近归档".into()
}
fn media_ext_badge(j: &Value) -> String {
    let name = j["name"].as_str().unwrap_or("");
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_uppercase();
    if ext == "MP4" && name.contains("4K") {
        return "MP4 4K".into();
    }
    if (ext == "PNG" || ext == "JPG" || ext == "JPEG") && name.contains("HDR") {
        return format!("{ext} HDR");
    }
    if !ext.is_empty() {
        return ext;
    }
    match j["kind"].as_str() {
        Some("photos") => "JPG".into(),
        Some("videos") => "MP4".into(),
        Some("documents") => "PDF".into(),
        Some("audiobooks") => "M4B".into(),
        Some("packages") => "APK".into(),
        _ => "FILE".into(),
    }
}
impl Companion {
    fn ex_button(&self, id: impl Into<String>, text: &str, a: EA, style: Btn) -> El {
        self.button(id, text, Action::Expansion(a), style)
    }
    fn ex_button_icon(
        &self,
        id: &str,
        label: impl Into<String>,
        icon: UiIcon,
        a: EA,
        kind: Btn,
    ) -> El {
        self.button_icon(format!("ex-{id}"), label, icon, Action::Expansion(a), kind)
    }
    pub(in super::super) fn expansion_card(&self) -> El {
        let p = self.p();
        let s = &self.expansion.snapshot;
        let effective_files: Vec<Value> = {
            let mut merged = s.files.clone();
            if let Some(arr) = s.status["recentFiles"].as_array() {
                for item in arr {
                    let key = item["id"]
                        .as_str()
                        .or_else(|| item["name"].as_str())
                        .unwrap_or("");
                    if !key.is_empty()
                        && !merged.iter().any(|existing| {
                            existing["id"].as_str() == Some(key)
                                || existing["name"].as_str() == Some(key)
                        })
                    {
                        merged.push(item.clone());
                    }
                }
            }
            if merged.is_empty() && self.demo_mode && std::env::var_os("STEPX_EMPTY").is_none() {
                expansion_state::demo_files()
            } else {
                merged
            }
        };
        let is_archived_item = |j: &Value| {
            let st = j["state"].as_str().unwrap_or("");
            st.is_empty() || st == "archived" || st == "已迁移"
        };
        let item_category = |j: &Value| -> &'static str { expansion_state::expansion_item_kind(j) };
        let bytes = s.status["releasedBytes"].as_u64().unwrap_or_else(|| {
            effective_files
                .iter()
                .filter(|j| is_archived_item(j))
                .map(|j| j["size"].as_u64().unwrap_or(0))
                .sum()
        });
        let count = s.status["count"].as_u64().unwrap_or_else(|| {
            effective_files
                .iter()
                .filter(|j| is_archived_item(j))
                .count() as u64
        });
        let headline = self.between(
            self.hstack(
                S2,
                vec![
                    self.ui_icon(UiIcon::Storage, 20., p.text),
                    self.heading("存储扩容"),
                ],
            ),
            if self.page == Page::Expansion {
                self.line("照片 · 视频 · 文档", CAPTION, p.text3)
            } else {
                self.button(
                    "expansion-manage",
                    "管理扩容",
                    Action::Page(Page::Expansion),
                    Btn::Secondary,
                )
            },
        );
        let metric = self.between(
            self.hstack(
                S4,
                vec![
                    self.line_w(
                        storage::format_bytes(bytes),
                        METRIC,
                        p.text,
                        Weight::Semibold,
                    ),
                    self.line(
                        format!(
                            "{} · {count} 个文件保存在电脑",
                            if s.status["releasedBytes"].is_u64() {
                                "已释放"
                            } else {
                                "原件已归档"
                            }
                        ),
                        CAPTION,
                        p.text2,
                    ),
                ],
            ),
            self.pill_badge(
                if s.status["policy"]["enabled"] == true {
                    "自动迁移已开启"
                } else {
                    if s.status["policy"].is_null() {
                        "尚未读取规则"
                    } else if s.status["policy"]["desktopId"]
                        .as_str()
                        .unwrap_or("")
                        .is_empty()
                    {
                        "尚未设置"
                    } else {
                        "自动迁移已暂停"
                    }
                },
                p.text2,
                p.hover,
            ),
        );
        let phase = if s.active.is_some() {
            s.phase.clone()
        } else if !s.online {
            "连接手机后自动迁移，手机保留文件入口".into()
        } else if s.error {
            s.phase.clone()
        } else {
            if s.phase.is_empty() {
                "正在检查迁移条件 · 只读取清单，不会误删文件".into()
            } else {
                s.phase.clone()
            }
        };
        let mut children = vec![
            headline,
            metric,
            self.text(phase, CAPTION, if s.error { p.danger } else { p.text2 }),
        ];
        if s.active.is_some() {
            let step = if s.phase.contains("传输") {
                0
            } else if s.phase.contains("校验电脑") {
                1
            } else if s.phase.contains("入口") {
                2
            } else if s.phase.contains("完成") {
                3
            } else {
                0
            };
            children.push(
                self.hstack(
                    S2,
                    ["1 传输", "2 校验", "3 保留入口", "4 释放空间"]
                        .into_iter()
                        .enumerate()
                        .map(|(i, t)| {
                            self.pill_badge(
                                t,
                                if i == step { p.success } else { p.text3 },
                                if i == step { p.success_soft } else { p.hover },
                            )
                        })
                        .collect(),
                ),
            );
        }
        if let Some(j) = s.active.as_ref() {
            children.push(self.text(label(j, "name"), CAPTION, p.text));
        } else {
            let mut recent = effective_files
                .iter()
                .filter(|j| j["state"] != "failed")
                .collect::<Vec<_>>();
            recent.sort_by_key(|j| {
                (
                    std::cmp::Reverse(is_archived_item(j)),
                    std::cmp::Reverse(j["archivedAt"].as_u64().unwrap_or(0)),
                )
            });
            if let Some(j) = recent.first() {
                children.push(self.between(
                    self.line(format!("最近迁移 · {}", label(j, "name")), CAPTION, p.text2),
                    self.line(state(j), CAPTION, p.text3),
                ));
            }
        }
        if self.page == Page::Expansion {
            let e = &self.expansion;
            let cat_stats = |cat_key: &str| -> (u64, u64, u64) {
                let mut cnt = 0u64;
                let mut sz = 0u64;
                for j in &effective_files {
                    if is_archived_item(j) && item_category(j) == cat_key {
                        cnt += 1;
                        sz += j["size"].as_u64().unwrap_or(0);
                    }
                }
                let phone_cnt = s.report["counts"][cat_key].as_u64().unwrap_or(0);
                (cnt, sz, phone_cnt)
            };
            let (archived_photos, photos_bytes, phone_photos) = cat_stats("photos");
            let (archived_videos, videos_bytes, phone_videos) = cat_stats("videos");
            let (archived_docs, docs_bytes, phone_docs) = cat_stats("documents");
            let (archived_audio, audio_bytes, phone_audio) = cat_stats("audio");
            children.push(
                self.hstack(
                    S3,
                    [
                        (
                            "照片",
                            archived_photos,
                            photos_bytes,
                            phone_photos,
                            UiIcon::Image,
                        ),
                        (
                            "视频",
                            archived_videos,
                            videos_bytes,
                            phone_videos,
                            UiIcon::Video,
                        ),
                        ("文档", archived_docs, docs_bytes, phone_docs, UiIcon::File),
                        (
                            "音频",
                            archived_audio,
                            audio_bytes,
                            phone_audio,
                            UiIcon::Music,
                        ),
                    ]
                    .into_iter()
                    .map(|(cat_label, cnt, cat_bytes, phone_cnt, icon)| {
                        let summary_text = if cnt > 0 && cat_bytes > 0 {
                            format!("{cnt} 项已归档 · {}", storage::format_bytes(cat_bytes))
                        } else if cnt > 0 {
                            format!("{cnt} 项已归档")
                        } else if phone_cnt > 0 {
                            format!("0 项已归档 · 手机 {phone_cnt} 项")
                        } else {
                            "0 项已归档".to_string()
                        };
                        Expanded::new(
                            1.,
                            Container::new(self.vstack(
                                S1,
                                vec![
                                    self.hstack(
                                        S1,
                                        vec![
                                            self.ui_icon(icon, 16., p.text2),
                                            self.line(cat_label, CAPTION, p.text2),
                                        ],
                                    ),
                                    self.line_w(summary_text, SUBHEAD, p.text, Weight::Semibold),
                                ],
                            ))
                            .with_uniform_padding(S2)
                            .with_background_color(p.surface)
                            .with_border(Border::all(1.).with_border_color(p.border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                            .finish(),
                        )
                        .finish()
                    })
                    .collect(),
                ),
            );
            children.push(self.divider());
            let rule_line = format!(
                "当前规则：{}  ·  电脑位置：{}",
                expansion_state::ExpansionUi::policy_summary(&s.status["policy"]),
                e.config
                    .folder
                    .as_ref()
                    .map(|f| f.join("StepX Storage").display().to_string())
                    .unwrap_or_else(|| "未设置".into())
            );
            let has_configured_rule = !s.status["policy"]["desktopId"]
                .as_str()
                .unwrap_or("")
                .is_empty();
            let mut rule_actions = Vec::new();
            if s.status["policy"]["enabled"] == true {
                rule_actions.push(self.ex_button(
                    "exp-pause",
                    "暂停自动迁移",
                    EA::Pause,
                    Btn::Secondary,
                ));
            } else if has_configured_rule {
                rule_actions.push(self.ex_button(
                    "exp-enable",
                    "恢复自动迁移",
                    EA::Enable,
                    Btn::Primary,
                ));
            }
            rule_actions.push(self.ex_button_icon(
                "exp-manual-import-card",
                "手动归档文件",
                UiIcon::Plus,
                EA::ManualImportLocalFiles,
                Btn::Secondary,
            ));
            rule_actions.push(self.ex_button_icon(
                "exp-open-current",
                "打开电脑扩容文件夹",
                UiIcon::Folder,
                EA::OpenFolder,
                Btn::Secondary,
            ));
            children.push(self.between(
                self.text(rule_line, CAPTION, p.text2),
                self.wrap(rule_actions),
            ));
        }
        self.panel(children)
    }
    pub(in super::super) fn expansion_actions(&self) -> Vec<El> {
        if self.expansion.settings {
            return vec![];
        }
        vec![
            self.ex_button_icon(
                "exp-hdr-open-folder",
                "打开电脑扩容文件夹",
                UiIcon::Folder,
                EA::OpenFolder,
                Btn::Secondary,
            ),
            self.ex_button_icon(
                "exp-hdr-manual-import",
                "手动归档文件",
                UiIcon::Plus,
                EA::ManualImportLocalFiles,
                Btn::Secondary,
            ),
            self.ex_button("exp-refresh", "检查迁移", EA::Refresh, Btn::Secondary),
            self.ex_button("exp-settings", "设置自动迁移", EA::Settings, Btn::Primary),
        ]
    }
    fn expansion_report_card(&self, report: &Value, preview: bool) -> El {
        let p = self.p();
        let e = &self.expansion;
        let title = if preview {
            "按这些规则，会迁移什么？"
        } else {
            "文件检查"
        };
        if !report.is_object() {
            let error = if preview {
                e.preview_error.as_str()
            } else {
                e.snapshot.report_error.as_str()
            };
            return self.panel(vec![
                self.heading(title),
                self.text(
                    if error.is_empty() {
                        "正在检查手机文件，只读取清单，不会移动或清理文件。"
                    } else {
                        error
                    },
                    BODY,
                    p.text2,
                ),
                self.wrap(vec![self.ex_button(
                    if preview {
                        "exp-preview-retry"
                    } else {
                        "exp-check-again"
                    },
                    "重新检查",
                    if preview { EA::Inspect } else { EA::Refresh },
                    Btn::Secondary,
                )]),
            ]);
        }
        let metrics = [
            ("可以迁移", report["candidateCount"].as_u64().unwrap_or(0)),
            (
                "保留在手机",
                report["retainedCount"].as_u64().unwrap_or(0)
                    + report["writingCount"].as_u64().unwrap_or(0),
            ),
            (
                "已处理 / 跳过",
                report["excludedCount"].as_u64().unwrap_or(0)
                    + report["invalidCount"].as_u64().unwrap_or(0),
            ),
        ];
        let checked_label = report["checkedAt"]
            .as_i64()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|at| {
                format!(
                    "检查于 {}",
                    at.with_timezone(&chrono::Local).format("%H:%M:%S")
                )
            });
        let header_right = if preview {
            self.ex_button("exp-preview-refresh", "重新检查", EA::Inspect, Btn::Ghost)
        } else if let Some(lbl) = &checked_label {
            self.pill_badge(lbl, p.text2, p.hover)
        } else {
            Empty::new().finish()
        };
        let mut rows = vec![
            self.between(self.heading(title), header_right),
            self.hstack(
                S4,
                metrics
                    .into_iter()
                    .map(|(name, n)| {
                        Expanded::new(
                            1.,
                            self.vstack(
                                S1,
                                vec![
                                    self.line_w(
                                        format!("{n} 个"),
                                        METRIC,
                                        p.text,
                                        Weight::Semibold,
                                    ),
                                    self.line(name, CAPTION, p.text2),
                                ],
                            ),
                        )
                        .finish()
                    })
                    .collect(),
            ),
            self.text(
                format!(
                    "识别到：{} 张照片 · {} 个视频 · {} 份文档（照片/视频：DCIM 与 Camera；文档：Documents / Download）",
                    report["counts"]["photos"],
                    report["counts"]["videos"],
                    report["counts"]["documents"]
                ),
                CAPTION,
                p.text2,
            ),
        ];
        if report["candidateCount"].as_u64().unwrap_or(0) > 0 {
            rows.push(self.text(
                format!(
                    "预计迁移 {}。{}",
                    storage::format_bytes(report["candidateBytes"].as_u64().unwrap_or(0)),
                    if preview {
                        "确认后开始；执行前逐个复核。"
                    } else {
                        "按当前已确认的规则处理。"
                    }
                ),
                BODY,
                p.text,
            ));
        } else if let Some(at) = report["nextEligibleAt"]
            .as_i64()
            .filter(|n| *n > 0)
            .and_then(chrono::DateTime::from_timestamp_millis)
        {
            rows.push(self.text(
                format!(
                    "最早一批将在 {} 达到迁移时间，届时保持手机和本客户端连接即可。",
                    at.with_timezone(&chrono::Local).format("%m 月 %d 日 %H:%M")
                ),
                BODY,
                p.text,
            ));
        } else {
            rows.push(self.text(
                "目前没有符合这些规则的文件。可以调整文件类型或保留时间后重新检查。",
                BODY,
                p.text2,
            ));
        }
        if preview && report["retainedCount"].as_u64().unwrap_or(0) > 0 {
            rows.push(self.text(
                "想迁移最近拍摄的文件？返回第一步，将时间改为“写入完成后迁移”，再检查并确认。",
                CAPTION,
                p.text2,
            ));
        }
        if report["partial"] == true {
            rows.push(self.text("文件较多，本次清单统计不完整。", CAPTION, p.warning));
        }
        self.panel(rows)
    }
    pub(in super::super) fn expansion_page(&self) -> El {
        let p = self.p();
        let e = &self.expansion;
        let q = e.draft.as_ref().unwrap_or(&e.snapshot.status["policy"]);
        let mut sections = vec![self.storage_mode_bar()];
        if e.settings {
            sections.push(self.between(
                self.heading("设置自动迁移"),
                self.text(format!("第 {} 步 / 共 3 步", e.step + 1), CAPTION, p.text2),
            ));
            sections.push(
                self.hstack(
                    S3,
                    ["1  选择内容", "2  电脑位置", "3  检查并开启"]
                        .into_iter()
                        .enumerate()
                        .map(|(i, t)| {
                            self.pill_badge(
                                t,
                                if i == e.step as usize {
                                    p.on_accent
                                } else {
                                    p.text2
                                },
                                if i == e.step as usize {
                                    p.accent
                                } else {
                                    p.hover
                                },
                            )
                        })
                        .collect(),
                ),
            );
            sections.push(self.text(
                "修改尚未生效，手机继续使用原规则。完成第三步确认后保存并应用。",
                CAPTION,
                p.text2,
            ));
            match e.step {
                0 => {
                    let mut cards = vec![];
                    for (key, name, note, icon) in [
                        ("photos", "拍摄照片", "DCIM 根目录 / Camera", UiIcon::Image),
                        ("videos", "拍摄视频", "DCIM 根目录 / Camera", UiIcon::Video),
                        ("documents", "文档", "Documents / Download", UiIcon::File),
                    ] {
                        let on = q[key] == true;
                        cards.push(
                            Expanded::new(
                                1.,
                                self.panel(vec![
                                    self.hstack(
                                        S2,
                                        vec![self.ui_icon(icon, 20., p.text), self.heading(name)],
                                    ),
                                    self.text(note, CAPTION, p.text2),
                                    self.wrap(vec![if on {
                                        self.ex_button_icon(
                                            &format!("exp-type-{key}"),
                                            "已选择",
                                            UiIcon::Check,
                                            EA::Type(key.into()),
                                            Btn::Primary,
                                        )
                                    } else {
                                        self.ex_button(
                                            format!("exp-type-{key}"),
                                            "选择",
                                            EA::Type(key.into()),
                                            Btn::Secondary,
                                        )
                                    }]),
                                ]),
                            )
                            .finish(),
                        );
                    }
                    sections.push(self.heading("选择要自动迁移的类型"));
                    sections.push(self.hstack(S3, cards));
                    sections.push(self.panel(vec![self.heading("什么时候迁移？"),self.wrap([(0,"写入完成后迁移"),(7,"保留最近 7 天"),(30,"保留最近 30 天")].into_iter().map(|(d,t)|self.ex_button(format!("exp-days-{d}"),t,EA::Days(d),if q["retentionDays"].as_u64().unwrap_or(7)==d{Btn::Primary}else{Btn::Secondary})).collect()),
                        self.text(if q["retentionDays"]==0{"文件停止写入至少 2 分钟后迁移；连接时新文件也会按此规则处理。"}else{"只迁移早于所选天数的文件。最近拍摄的照片和视频会继续留在手机，满期后再迁移。"},BODY,p.text2),
                        self.text("截图、应用缓存与隐藏文件不在自动迁移范围；恢复到手机的文件不会被重复迁移。",CAPTION,p.text3)]));
                }
                1 => {
                    sections.push(self.panel(vec![self.heading("选择电脑上的原件保存位置"),self.text(e.draft_folder.as_ref().map(|f|f.join("StepX Storage").display().to_string()).unwrap_or_else(||"尚未选择保存文件夹".into()),BODY,p.text),
                        self.wrap(vec![self.ex_button("exp-folder","选择文件夹…",EA::Folder,Btn::Primary),if e.draft_folder.is_some(){self.ex_button("exp-open-folder","打开位置",EA::OpenFolder,Btn::Ghost)}else{self.disabled_button("打开位置")}]),
                        self.text("迁移后的原件保存在这里。请使用长期连接、空间充足的磁盘，并保留这个目录。",BODY,p.text2)]));
                    sections.push(self.panel(vec![
                        self.heading("连接与权限"),
                        if e.snapshot.online {
                            self.hstack(
                                6.,
                                vec![
                                    self.ui_icon(UiIcon::Check, 14., p.success),
                                    self.line("手机已连接，USB 与无线 ADB 均可使用", BODY, p.text2),
                                ],
                            )
                        } else {
                            self.line("请先连接手机；连接成功后继续", BODY, p.text2)
                        },
                        if e.snapshot.status["permission"] == true {
                            self.hstack(
                                6.,
                                vec![
                                    self.ui_icon(UiIcon::Check, 14., p.success),
                                    self.line("手机已授予文件管理权限", BODY, p.text2),
                                ],
                            )
                        } else {
                            self.line(
                                "请在手机伴侣 → 文件 → 存储扩容中开启文件管理权限",
                                BODY,
                                p.text2,
                            )
                        },
                    ]));
                }
                _ => {
                    sections.push(
                        self.expansion_report_card(
                            e.preview.as_ref().unwrap_or(&Value::Null),
                            true,
                        ),
                    );
                    sections.push(self.panel(vec![self.heading("确认后的规则"),self.text(expansion_state::ExpansionUi::policy_summary(q),BODY,p.text),self.text(format!("电脑位置：{}",e.draft_folder.as_ref().map(|f|f.join("StepX Storage").display().to_string()).unwrap_or_default()),CAPTION,p.text2),
                        self.text("电脑收到原件并校验成功后，才清理手机原件。手机保留文件入口和相册预览。",BODY,p.text2),self.text("取回原图：在相册分享预览图至“取回电脑原图”，或从伴侣文件库恢复。电脑离线时不能取回未缓存的原件。",CAPTION,p.text3)]));
                }
            }
        } else {
            sections.push(self.expansion_card());
        }
        if !e.feedback.is_empty() {
            sections.push(self.text(
                &e.feedback,
                BODY,
                if e.feedback.contains("失败") || e.feedback.contains("未完成") {
                    p.danger
                } else {
                    p.text2
                },
            ));
        }
        if e.snapshot.error {
            sections.push(self.text(&e.snapshot.phase, BODY, p.danger));
        }

        let fallback_demo = if e.snapshot.files.is_empty()
            && self.demo_mode
            && std::env::var_os("STEPX_EMPTY").is_none()
        {
            expansion_state::demo_files()
        } else {
            Vec::new()
        };
        let all_files = if !e.snapshot.files.is_empty() {
            &e.snapshot.files
        } else {
            &fallback_demo
        };

        if !e.settings && !all_files.is_empty() {
            let mut switcher_btns = vec![
                self.ex_button(
                    "exp-view-grid",
                    "网格视图",
                    EA::SetGalleryGrid(true),
                    if e.gallery_grid_mode {
                        Btn::Primary
                    } else {
                        Btn::Secondary
                    },
                ),
                self.ex_button(
                    "exp-view-list",
                    "列表视图",
                    EA::SetGalleryGrid(false),
                    if !e.gallery_grid_mode {
                        Btn::Primary
                    } else {
                        Btn::Secondary
                    },
                ),
            ];
            if e.gallery_grid_mode {
                for (cols, label_txt) in [(2u8, "2列大图"), (3, "3列标准"), (4, "4列紧凑")]
                {
                    let active = e.grid_cols == cols;
                    switcher_btns.push(self.ex_button(
                        format!("exp-grid-cols-{cols}"),
                        label_txt,
                        EA::SetGridCols(cols),
                        if active { Btn::Secondary } else { Btn::Ghost },
                    ));
                }
            }
            let view_switcher = self.wrap(switcher_btns);
            sections.push(self.between(self.heading("已扩容文件 · 迁移记录"), view_switcher));
            let total_cnt = all_files.len();
            let photos_cnt = all_files
                .iter()
                .filter(|j| expansion_state::expansion_item_kind(j) == "photos")
                .count();
            let videos_cnt = all_files
                .iter()
                .filter(|j| expansion_state::expansion_item_kind(j) == "videos")
                .count();
            let docs_cnt = all_files
                .iter()
                .filter(|j| expansion_state::expansion_item_kind(j) == "documents")
                .count();
            let audio_cnt = all_files
                .iter()
                .filter(|j| expansion_state::expansion_item_kind(j) == "audio")
                .count();
            let mut controls = vec![self.ex_button(
                "exp-category-all",
                &format!("全部 ({total_cnt})"),
                EA::Filter("all".into()),
                if e.config.kind == "all" {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            )];
            for (k, icon, n) in [
                ("photos", UiIcon::Image, format!("照片 ({photos_cnt})")),
                ("videos", UiIcon::Video, format!("视频 ({videos_cnt})")),
                ("documents", UiIcon::File, format!("文档 ({docs_cnt})")),
                ("audio", UiIcon::Music, format!("音频 ({audio_cnt})")),
            ] {
                controls.push(self.ex_button_icon(
                    &format!("exp-category-{k}"),
                    n,
                    icon,
                    EA::Filter(k.into()),
                    if e.config.kind == k {
                        Btn::Primary
                    } else {
                        Btn::Secondary
                    },
                ));
            }
            let mut sorts = vec![];
            for (k, n) in [("date", "日期"), ("name", "名称"), ("size", "大小")] {
                sorts.push(self.ex_button(
                    format!("exp-sort-{k}"),
                    n,
                    EA::Sort(k.into()),
                    if e.config.sort == k {
                        Btn::Secondary
                    } else {
                        Btn::Ghost
                    },
                ));
            }
            sorts.push(self.ex_button_icon(
                "exp-direction",
                if e.config.descending {
                    "降序"
                } else {
                    "升序"
                },
                if e.config.descending {
                    UiIcon::ArrowDown
                } else {
                    UiIcon::ArrowUp
                },
                EA::Descending,
                Btn::Ghost,
            ));
            let status_filters: Vec<El> = [
                ("all", "全部状态"),
                ("archived", "已迁移"),
                ("restored", "已恢复"),
                ("prepared", "待迁移"),
                ("verified", "待确认"),
                ("failed", "失败"),
            ]
            .into_iter()
            .map(|(k, n)| {
                self.ex_button(
                    format!("exp-state-{k}"),
                    n,
                    EA::State(k.into()),
                    if e.config.filter == k {
                        Btn::Secondary
                    } else {
                        Btn::Ghost
                    },
                )
            })
            .collect();
            let mut search_row = vec![self.input(
                F_GALLERY_SEARCH,
                "搜索归档文件名 / 路径 / 日期…",
                Some(240.),
            )];
            if !self.fields[F_GALLERY_SEARCH].text.trim().is_empty() {
                search_row.push(self.button(
                    "exp-clear-search",
                    "清空搜索",
                    Action::ClearField(F_GALLERY_SEARCH),
                    Btn::Ghost,
                ));
            }
            sections.push(self.panel(vec![
                self.between(
                    self.text(
                        "筛选文件库 · 只改变下面显示的记录，不改变迁移规则",
                        CAPTION,
                        p.text2,
                    ),
                    self.hstack(S2, search_row),
                ),
                self.between(self.wrap(controls), self.wrap(sorts)),
                self.wrap(status_filters),
            ]));
            let files_owned = self.expansion_filtered_files();
            let files: Vec<&Value> = files_owned.iter().collect();
            if files.is_empty() {
                sections.push(self.panel(if all_files.is_empty() {
                    vec![
                        self.heading("尚未迁移文件"),
                        self.text(
                            "先选择文件类型和电脑保存位置，再确认开启。不会因为点击上面的分类按钮就迁移文件。",
                            BODY,
                            p.text2,
                        ),
                        self.wrap(vec![self.ex_button(
                            "exp-empty-settings",
                            "设置迁移规则",
                            EA::Settings,
                            Btn::Primary,
                        )]),
                    ]
                } else {
                    vec![
                        self.heading("没有符合筛选条件的记录"),
                        self.text("切换为“全部”和“全部状态”或清空搜索词可查看其他文件。迁移规则未改变。", BODY, p.text2),
                    ]
                }));
            } else if e.gallery_grid_mode {
                let limited: Vec<&Value> = files.iter().copied().take(e.limit).collect();
                sections.extend(self.expansion_timeline_gallery(&limited));
            } else {
                for j in files.iter().take(e.limit) {
                    let id = label(j, "id");
                    let phone_rel = label(j, "path");
                    let is_archived = j["state"] == "archived";
                    let local_media_path =
                        expansion_state::resolve_media_local_path(&self.expansion.config, j);
                    let selected = e.selected_media_id.as_deref() == Some(id.as_str());
                    let date = chrono::DateTime::from_timestamp_millis(
                        j["modified"].as_i64().unwrap_or(0),
                    )
                    .map(|d| {
                        d.with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                    let icon = match j["kind"].as_str() {
                        Some("photos") => UiIcon::Image,
                        Some("videos") => UiIcon::Video,
                        _ => UiIcon::File,
                    };
                    let mut actions = vec![
                        self.ex_button(
                            format!("exp-open-{id}"),
                            "打开文件",
                            EA::Open(id.clone()),
                            Btn::Secondary,
                        ),
                        self.ex_button(
                            format!("exp-reveal-{id}"),
                            "在文件夹中显示",
                            EA::RevealInFolder(id.clone()),
                            Btn::Secondary,
                        ),
                    ];
                    if is_archived {
                        actions.push(self.ex_button(
                            format!("exp-restore-{id}"),
                            "还原回手机",
                            EA::Restore(id.clone()),
                            Btn::Ghost,
                        ));
                    }
                    actions.push(self.ex_button(
                        format!("exp-ai-{id}"),
                        "发给 Amoo",
                        EA::AnalyzeWithAmoo(id.clone()),
                        Btn::Ghost,
                    ));
                    let row_content = self.vstack(
                        S3,
                        vec![
                            self.between(
                                self.hstack(
                                    S2,
                                    vec![
                                        self.ui_icon(icon, 20., p.text2),
                                        self.heading(label(j, "name")),
                                    ],
                                ),
                                self.pill_badge(state(j), p.text2, p.hover),
                            ),
                            self.text(
                                format!(
                                    "{date} · {} · {}",
                                    storage::format_bytes(j["size"].as_u64().unwrap_or(0)),
                                    phone_rel
                                ),
                                CAPTION,
                                p.text3,
                            ),
                            self.wrap(actions),
                        ],
                    );
                    let click_id = id.clone();
                    let dbl_id = id.clone();
                    let menu_id = id.clone();
                    let row_state = self.mouse_state(format!("exp-list-row:{id}"));
                    sections.push(
                        Hoverable::new(row_state, move |mouse| {
                            let hovered = mouse.is_mouse_over_element();
                            let border_col = if mouse.is_clicked() || selected || hovered {
                                p.accent
                            } else {
                                p.border
                            };
                            Container::new(row_content)
                                .with_uniform_padding(S4)
                                .with_background_color(p.bg)
                                .with_border(
                                    Border::all(if selected { 1.5 } else { 1. })
                                        .with_border_color(border_col),
                                )
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                                .finish()
                        })
                        .with_defer_events_to_children()
                        .with_cursor(facloud_ui::platform::Cursor::PointingHand)
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(Action::Expansion(EA::SelectMedia(Some(
                                click_id.clone(),
                            ))))
                        })
                        .on_double_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(Action::Expansion(EA::Open(dbl_id.clone())))
                        })
                        .on_right_click(move |ctx, _, at| {
                            ctx.dispatch_typed_action(Action::ContextOpen(
                                crate::ui::context_menu::Target::ExpansionItem {
                                    id: menu_id.clone(),
                                    local_path: local_media_path.clone(),
                                    phone_path: phone_rel.to_string(),
                                    is_archived,
                                },
                                at,
                            ))
                        })
                        .finish(),
                    );
                }
            }
            if files.len() > e.limit {
                sections.push(self.wrap(vec![self.ex_button(
                    "exp-more",
                    "加载更多",
                    EA::More,
                    Btn::Secondary,
                )]));
            }
        }
        let body = self.sub_enter(
            self.pane_scroll(
                self.scroll.clone(),
                Container::new(self.vstack(S4, sections))
                    .with_padding_right(S2)
                    .with_padding_bottom(S5)
                    .finish(),
            ),
        );
        if !e.settings {
            return body;
        }
        let mut buttons = vec![self.ex_button("exp-cancel", "取消", EA::Back, Btn::Ghost)];
        if e.step > 0 {
            buttons.push(self.ex_button("exp-previous", "上一步", EA::Previous, Btn::Secondary));
        }
        let ready = e.pending.is_none()
            && (e.step < 2 || (e.preview.is_some() && e.preview_pending.is_none()));
        buttons.push(if ready {
            self.ex_button(
                "exp-next",
                if e.step == 2 {
                    "确认并开启自动迁移"
                } else {
                    "下一步"
                },
                if e.step == 2 { EA::Confirm } else { EA::Next },
                Btn::Primary,
            )
        } else {
            self.disabled_button(if e.pending.is_some() {
                "正在保存…"
            } else {
                "正在检查…"
            })
        });
        let footer = Container::new(self.between(
            self.vstack(
                S1,
                vec![
                    self.line_w(
                        format!("第 {} / 3 步", e.step + 1),
                        BODY,
                        p.text,
                        Weight::Semibold,
                    ),
                    self.line(
                        if e.step == 2 {
                            "确认后生效，取消保留当前手机规则"
                        } else {
                            "完成选择后，点击右侧下一步"
                        },
                        CAPTION,
                        p.text2,
                    ),
                ],
            ),
            self.wrap(buttons),
        ))
        .with_uniform_padding(S3)
        .with_background_color(p.bg)
        .with_border(Border::top(1.).with_border_color(p.border))
        .finish();
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Expanded::new(1., body).finish())
            .with_child(footer)
            .finish()
    }

    fn expansion_timeline_gallery(&self, files: &[&Value]) -> Vec<El> {
        let p = self.p();
        let cols = (self.expansion.grid_cols as usize).clamp(2, 4);
        let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
        for j in files {
            let bucket = timeline_date_bucket(j);
            if let Some((_, list)) = groups.iter_mut().find(|(b, _)| *b == bucket) {
                list.push(*j);
            } else {
                groups.push((bucket, vec![*j]));
            }
        }
        let mut out = Vec::new();
        for (bucket_title, items) in groups {
            let dot = ConstrainedBox::new(
                Container::new(Empty::new().finish())
                    .with_background_color(p.success)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .finish(),
            )
            .with_width(8.)
            .with_height(8.)
            .finish();
            let header = self.between(
                self.hstack(
                    S2,
                    vec![
                        dot,
                        self.line_w(&bucket_title, SUBHEAD, p.text, Weight::Semibold),
                    ],
                ),
                self.pill_badge(format!("共 {} 项", items.len()), p.text2, p.hover),
            );
            out.push(header);
            for chunk in items.chunks(cols) {
                let mut row_cards = Vec::with_capacity(cols);
                for j in chunk {
                    row_cards.push(Expanded::new(1., self.expansion_media_card(j)).finish());
                }
                while row_cards.len() < cols {
                    row_cards.push(Expanded::new(1., Empty::new().finish()).finish());
                }
                out.push(self.hstack(S3, row_cards));
            }
        }
        out
    }

    fn expansion_media_card(&self, j: &Value) -> El {
        let p = self.p();
        let id = label(j, "id");
        let name = label(j, "name");
        let path_str = label(j, "path");
        let kind = j["kind"].as_str().unwrap_or("");
        let size_str = storage::format_bytes(j["size"].as_u64().unwrap_or(0));
        let ext_badge = media_ext_badge(j);
        let cols = (self.expansion.grid_cols as usize).clamp(2, 4);
        let thumb_h = match cols {
            2 => 180.0,
            4 => 96.0,
            _ => 124.0,
        };
        let deg = self
            .expansion
            .media_rotations
            .get(&id)
            .copied()
            .unwrap_or(0)
            % 360;
        let selected = self.expansion.selected_media_id.as_deref() == Some(id.as_str());

        let rotated_cache = expansion_state::rotated_media_cache_path(&id, deg);
        let resolved_local = rotated_cache
            .or_else(|| expansion_state::resolve_media_local_path(&self.expansion.config, j));
        let is_local_image = resolved_local.as_ref().is_some_and(|path| {
            cached_is_file(path)
                && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    matches!(
                        e.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "webp" | "gif"
                    )
                })
        });

        let dims_opt = resolved_local
            .as_ref()
            .and_then(|p| cached_image_dimensions(p));

        let badge_label = match (dims_opt, deg) {
            (Some((w, h)), d) if d != 0 => format!("{ext_badge} · {w}×{h} · {d}°"),
            (Some((w, h)), _) => format!("{ext_badge} · {w}×{h}"),
            (None, d) if d != 0 => format!("{ext_badge} · {d}°"),
            (None, _) => format!("{ext_badge} · 原图"),
        };

        let local_media_path = resolved_local.clone();
        let phone_rel = path_str.clone();
        let is_archived = j["state"] == "archived";

        let raw_preview = if is_local_image {
            let local_path = resolved_local.unwrap();
            let img = ConstrainedBox::new(
                Image::new(
                    AssetSource::LocalFile {
                        path: local_path.to_string_lossy().into_owned(),
                    },
                    CacheOption::BySize,
                )
                .contain()
                .finish(),
            )
            .with_max_width(thumb_h * 1.8)
            .with_height(thumb_h)
            .finish();
            let badge = self.pill_badge(badge_label, p.text, p.bg);
            Stack::new()
                .with_child(
                    Container::new(img)
                        .with_background_color(p.surface)
                        .with_border(Border::all(1.).with_border_color(if selected {
                            p.accent
                        } else {
                            p.border
                        }))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish(),
                )
                .with_positioned_child(
                    badge,
                    OffsetPositioning::offset_from_parent(
                        vec2f(-S2, S2),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopRight,
                        ChildAnchor::TopRight,
                    ),
                )
                .finish()
        } else if j["previewAsset"].as_str().is_some() || (self.demo_mode && kind == "photos") {
            let asset_name = match j["previewAsset"].as_str() {
                Some("neo_front_frame.png") => "neo_front_frame.png",
                Some("neo_rear_housing.png") => "neo_rear_housing.png",
                _ => "stepx_home_screen.png",
            };
            let img = ConstrainedBox::new(
                Image::new(
                    AssetSource::Bundled { path: asset_name },
                    CacheOption::BySize,
                )
                .contain()
                .finish(),
            )
            .with_max_width(thumb_h * 1.8)
            .with_height(thumb_h)
            .finish();
            let badge = self.pill_badge(format!("{ext_badge} · 缩略图"), p.text, p.bg);
            Stack::new()
                .with_child(
                    Container::new(img)
                        .with_background_color(p.surface)
                        .with_border(Border::all(1.).with_border_color(if selected {
                            p.accent
                        } else {
                            p.border
                        }))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish(),
                )
                .with_positioned_child(
                    badge,
                    OffsetPositioning::offset_from_parent(
                        vec2f(-S2, S2),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopRight,
                        ChildAnchor::TopRight,
                    ),
                )
                .finish()
        } else {
            let icon = match kind {
                "videos" => UiIcon::Monitor,
                "documents" => UiIcon::Folder,
                "audiobooks" => UiIcon::Music,
                "packages" => UiIcon::Package,
                "photos" => UiIcon::Image,
                _ => UiIcon::File,
            };
            ConstrainedBox::new(
                Container::new(
                    Flex::column()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_child(self.between(
                            self.pill_badge(&ext_badge, p.text, p.bg),
                            self.pill_badge(&size_str, p.text2, p.bg),
                        ))
                        .with_child(
                            Flex::row()
                                .with_main_axis_size(MainAxisSize::Max)
                                .with_main_axis_alignment(MainAxisAlignment::Center)
                                .with_child(self.ui_icon(icon, 28., p.text2))
                                .finish(),
                        )
                        .with_child(self.line(&path_str, MICRO, p.text3))
                        .finish(),
                )
                .with_uniform_padding(S2)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(if selected {
                    p.accent
                } else {
                    p.border
                }))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            )
            .with_height(thumb_h)
            .finish()
        };

        let (badge_text, badge_fg, badge_bg) = match j["state"].as_str().unwrap_or("") {
            "archived" => ("已释放手机空间", p.success_text, p.success_soft),
            "restored" => ("双端保留", p.accent, p.accent_soft),
            _ => (state(j), p.text2, p.hover),
        };
        let meta_row = self.between(
            self.pill_badge(badge_text, badge_fg, badge_bg),
            self.mono(&size_str, MICRO, p.text2),
        );
        let mut action_btns = vec![
            self.ex_button(
                format!("exp-open-{id}"),
                "打开文件",
                EA::Open(id.clone()),
                Btn::Secondary,
            ),
            self.ex_button(
                format!("exp-reveal-{id}"),
                "在文件夹中显示",
                EA::RevealInFolder(id.clone()),
                Btn::Secondary,
            ),
        ];
        if is_archived {
            action_btns.push(self.ex_button(
                format!("exp-restore-{id}"),
                "还原回手机",
                EA::Restore(id.clone()),
                Btn::Ghost,
            ));
        }
        action_btns.push(self.ex_button(
            format!("exp-ai-{id}"),
            "发给 Amoo",
            EA::AnalyzeWithAmoo(id.clone()),
            Btn::Ghost,
        ));
        let actions = self.wrap(action_btns);
        let card_inner = self.vstack(
            S3,
            vec![
                raw_preview,
                self.line_w(&name, BODY, p.text, Weight::Medium),
                meta_row,
                actions,
            ],
        );
        let click_id = id.clone();
        let dbl_id = id.clone();
        let menu_id = id.clone();
        let card_state = self.mouse_state(format!("exp-card:{id}"));
        Hoverable::new(card_state, move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            let border_col = if mouse.is_clicked() || selected || hovered {
                p.accent
            } else {
                p.border
            };
            Container::new(card_inner)
                .with_uniform_padding(S4)
                .with_background_color(p.bg)
                .with_border(
                    Border::all(if selected { 1.5 } else { 1. }).with_border_color(border_col),
                )
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish()
        })
        .with_defer_events_to_children()
        .with_cursor(facloud_ui::platform::Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::Expansion(EA::SelectMedia(Some(click_id.clone()))))
        })
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::Expansion(EA::Open(dbl_id.clone())))
        })
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::ExpansionItem {
                    id: menu_id.clone(),
                    local_path: local_media_path.clone(),
                    phone_path: phone_rel.to_string(),
                    is_archived,
                },
                at,
            ))
        })
        .finish()
    }
}
