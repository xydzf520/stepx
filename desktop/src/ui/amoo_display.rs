//! Presentation of real Amoo history. Tool invocations are records, not replies.

use super::AmooChatItem;
use stepx_companion::openclaw::RealChatMessage;

/// Presentation only: native session names sometimes contain an internal plan
/// (or the start of one). Never rename the source session or infer from prose.
pub(super) fn session_title(title: &str) -> String {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(hit) = guard.get(title) {
            return hit.clone();
        }
    }
    let computed = compute_session_title(title);
    if let Ok(mut guard) = cache.lock() {
        if guard.len() > 512 {
            guard.clear();
        }
        guard.insert(title.to_string(), computed.clone());
    }
    computed
}

pub(super) fn friendly_tool_label(raw: &str) -> &str {
    stepx_companion::amoo::friendly_tool_label(raw)
}

fn compute_session_title(title: &str) -> String {
    fn backscreen_label(kind: &str) -> Option<&'static str> {
        match kind {
            "image" => Some("背屏图片创作"),
            "preset" => Some("背屏动画创作"),
            "native" => Some("背屏功能设置"),
            "app" => Some("背屏应用联动"),
            _ => None,
        }
    }
    fn format_label_with_summary(label: &str, summary: &str) -> String {
        let summary = summary.split_whitespace().collect::<Vec<_>>().join(" ");
        if summary.is_empty() || summary.starts_with(['{', '<', '`']) {
            return label.into();
        }
        let short: String = summary.chars().take(28).collect();
        format!(
            "{label} · {short}{}",
            if summary.chars().count() > 28 {
                "…"
            } else {
                ""
            }
        )
    }
    fn with_summary(label: &str, value: &serde_json::Value) -> String {
        let summary = value["summary"]
            .as_str()
            .or_else(|| value["title"].as_str())
            .or_else(|| value["goal"].as_str())
            .unwrap_or("");
        format_label_with_summary(label, summary)
    }
    fn extract_truncated_json_field(body: &str, key: &str) -> Option<String> {
        let pattern = format!("\"{key}\"");
        let idx = body.find(&pattern)?;
        let rest = body[idx + pattern.len()..].trim_start();
        let rest = rest.strip_prefix(':')?.trim_start();
        let rest = rest.strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '"' => break,
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        match escaped {
                            'n' | 'r' | 't' => out.push(' '),
                            other => out.push(other),
                        }
                    }
                }
                other => out.push(other),
            }
        }
        let cleaned = out.split_whitespace().collect::<Vec<_>>().join(" ");
        let cleaned = cleaned.trim_end_matches('…').trim();
        if cleaned.is_empty() || cleaned.starts_with(['{', '<', '`']) {
            None
        } else {
            Some(cleaned.to_string())
        }
    }
    let trimmed = title.trim();
    if trimmed == "main" || trimmed == "default" || trimmed.starts_with("agent:") {
        return "主对话".into();
    }
    let body = trimmed
        .strip_prefix("```json")
        .and_then(|s| s.trim().strip_suffix("```"))
        .or_else(|| {
            trimmed
                .strip_prefix("<amoo_work_protocol>")
                .and_then(|s| s.strip_suffix("</amoo_work_protocol>"))
        })
        .unwrap_or(trimmed)
        .trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        if value.get("amoo_work").is_some() {
            return with_summary("文件任务", &value);
        }
        if let Some(label) = value["kind"].as_str().and_then(backscreen_label) {
            if value.get("brightness").is_some()
                && value.get("fps").is_some()
                && value.get("duration_secs").is_some()
                && ["native_feature", "image_prompt", "preset", "app_package"]
                    .iter()
                    .any(|key| value.get(key).is_some())
            {
                return with_summary(label, &value);
            }
        }
    } else {
        if body.starts_with('{') && body.contains("\"amoo_work\"") {
            if let Some(summary) = extract_truncated_json_field(body, "summary")
                .or_else(|| extract_truncated_json_field(body, "title"))
                .or_else(|| extract_truncated_json_field(body, "goal"))
            {
                return format_label_with_summary("文件任务", &summary);
            }
            return "文件任务".into();
        }
        // Native auto-titles can be cut in the middle of a JSON field. This
        // exact protocol-specific prefix is sufficient; generic JSON is not.
        let compact: String = body
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .take(160)
            .collect();
        for kind in ["image", "preset", "native", "app"] {
            let prefix = format!("{{\"kind\":\"{kind}\",\"native_feature\"");
            if compact
                .strip_prefix(&prefix)
                .is_some_and(|tail| tail.is_empty() || tail.starts_with(':') || tail == "…")
            {
                return backscreen_label(kind).unwrap().into();
            }
        }
    }
    title.into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ToolRecord {
    pub name: String,
    pub time_label: String,
    /// Only details actually supplied by the backend are shown.
    pub detail: Option<String>,
}

pub(super) fn chat_items(
    session_key: &str,
    messages: Vec<RealChatMessage>,
    status_kind: u8,
) -> Vec<AmooChatItem> {
    let mut items = Vec::new();
    // A run may contain intermediate text between its tool invocations. Keep its
    // records together, but never merge across a user message or a different run.
    let mut group: Option<(String, usize)> = None;
    for (ordinal, message) in messages.into_iter().enumerate() {
        if message.role == "user" {
            group = None;
            if !message.text.trim().is_empty() {
                items.push(AmooChatItem::UserText {
                    attachment: None,
                    text: message.text,
                });
            }
            append_attachments(&mut items, &message.attachments);
            continue;
        }
        if message.role != "assistant" {
            continue;
        }

        if group
            .as_ref()
            .is_some_and(|(run_id, _)| *run_id != message.run_id)
        {
            group = None;
        }
        let attachments = message.attachments.clone();
        let mut records = tool_records(&message);
        let has_tools = !records.is_empty();
        let has_text = !message.text.trim().is_empty();
        if !has_text && !message.is_error && !message.thinking.is_empty() {
            if let Some(record) = records.first_mut() {
                if record.detail.is_none() {
                    record.detail = Some(message.thinking.join("\n"));
                }
            } else {
                records.push(ToolRecord {
                    name: "思考记录".into(),
                    time_label: message.time_label.clone(),
                    detail: Some(message.thinking.join("\n")),
                });
            }
        }

        if !records.is_empty() {
            if let Some((_, index)) = group.as_ref() {
                if let AmooChatItem::ToolRecords {
                    records: existing, ..
                } = &mut items[*index]
                {
                    existing.extend(records);
                }
            } else {
                let index = items.len();
                items.push(AmooChatItem::ToolRecords {
                    key: record_key(session_key, &message, ordinal),
                    records,
                });
                group = Some((message.run_id.clone(), index));
            }
        }

        if has_text || message.is_error {
            let text = reply_text(&message);
            items.push(AmooChatItem::AssistantRich {
                thinking_time: if message.stop_reason == "tracking_limit" {
                    "同步提示"
                } else if message.is_error {
                    "请求失败"
                } else if status_kind == 1 && message.stop_reason.is_empty() {
                    "生成中"
                } else {
                    "回复"
                }
                .into(),
                thinking_steps: message.thinking,
                // Tool names already have their own record area.
                tools_summary: if has_tools {
                    String::new()
                } else {
                    message.tools_summary
                },
                text: Some(text),
                has_bolt_artifact: false,
                has_invoice_artifact: false,
                has_safety_confirm: false,
                time_label: message.time_label,
            });
            // Without a run ID, only adjacent invocation messages are known to
            // belong together. A response provides an unambiguous boundary.
            if message.run_id.is_empty() {
                group = None;
            }
        }
        append_attachments(&mut items, &attachments);
    }
    items
}

fn append_attachments(
    items: &mut Vec<AmooChatItem>,
    attachments: &[stepx_companion::openclaw::RealAttachment],
) {
    for attachment in attachments {
        items.push(AmooChatItem::Attachment {
            name: if attachment.name.is_empty() {
                if attachment.kind == "image" {
                    "手机生成图片".into()
                } else {
                    "手机附件".into()
                }
            } else {
                attachment.name.clone()
            },
            kind: attachment.kind.clone(),
            mime: attachment.mime.clone(),
            url: attachment.url.clone(),
            path: attachment.path.clone(),
            size_bytes: attachment.size_bytes,
        });
    }
}

fn tool_records(message: &RealChatMessage) -> Vec<ToolRecord> {
    let summary = message.tools_summary.trim();
    // Legacy OpenClaw histories also put model/error labels in this field.
    // Those are metadata, not evidence that a tool was invoked.
    if summary.is_empty()
        || summary == format!("{}/{}", message.provider, message.model)
        || summary.starts_with("请求失败")
    {
        return Vec::new();
    }
    summary
        .split(" · ")
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            let friendly = friendly_tool_label(name);
            ToolRecord {
                name: friendly.into(),
                time_label: message.time_label.clone(),
                detail: message
                    .tool_authorization_failures
                    .iter()
                    .any(|failed| failed == name || friendly_tool_label(failed) == friendly)
                    .then(|| "授权超时：手机端 180 秒内未确认，此次工具调用已取消。请在手机 Amoo 中完成授权后重新发起任务；这不是执行成功。".into()),
            }
        })
        .collect()
}

fn record_key(session_key: &str, message: &RealChatMessage, ordinal: usize) -> String {
    let anchor = if !message.run_id.is_empty() {
        format!("run:{}", message.run_id)
    } else if !message.message_id.is_empty() {
        format!("message:{}", message.message_id)
    } else if message.timestamp_ms != 0 {
        format!("time:{}", message.timestamp_ms)
    } else {
        // Old histories without IDs/timestamps still retain state when more
        // messages are appended; never derive the key from the group length.
        format!("entry:{ordinal}")
    };
    format!("{}:{session_key}:{anchor}", session_key.len())
}

fn reply_text(message: &RealChatMessage) -> String {
    let mut body = message.text.clone();
    if message.is_error {
        if let Some(error) = message
            .error_message
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            if !body.contains(error) {
                if !body.trim().is_empty() {
                    body.push('\n');
                }
                body.push_str("错误详情：");
                body.push_str(error);
            }
        }
        if body.trim().is_empty() {
            body = "请求失败，后台未提供错误详情。".into();
        }
    }
    body
}

/// Retry the question that led to this reply, not the newest question in history.
pub(super) fn retry_prompt(items: &[AmooChatItem], reply_index: usize) -> Option<String> {
    items
        .get(..reply_index)?
        .iter()
        .rev()
        .find_map(|item| match item {
            AmooChatItem::UserText { text, .. } if !text.trim().is_empty() => {
                stepx_companion::work_protocol::display_text(text, true)
                    .filter(|text| !text.trim().is_empty())
            }
            AmooChatItem::UserVoice { transcript, .. } if !transcript.trim().is_empty() => {
                Some(transcript.clone())
            }
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_titles_only_normalize_recognizable_internal_protocols() {
        let source = r#"{"kind":"image","native_feature":null,"image_prompt":"astronaut","brightness":4,"fps":6,"duration_secs":30,"summary":"液晶动态太空人"}"#;
        assert_eq!(session_title(source), "背屏图片创作 · 液晶动态太空人");
        assert_eq!(
            session_title(r#"{"kind":"image","native_feature""#),
            "背屏图片创作"
        );
        assert_eq!(
            session_title(r#"{ "kind" : "native", "native_feature": "clo"#),
            "背屏功能设置"
        );
        assert_eq!(
            session_title(r#"{"amoo_work":1,"summary":"整理会议纪要","done":true,"tools":[]}"#),
            "文件任务 · 整理会议纪要"
        );
        for ordinary in [
            "  我的照片  ",
            r#"{"kind":"image"}"#,
            r#"{"kind":"image","summary":"示例 JSON"}"#,
            "分析 {\"kind\":\"image\",\"native_feature\":null}",
            r#"{"kind":"custom","native_feature":"x""#,
        ] {
            assert_eq!(session_title(ordinary), ordinary);
        }
    }

    #[test]
    fn generated_attachment_is_visible_even_without_text_and_with_tool_records() {
        let message = RealChatMessage {
            role: "assistant".into(),
            tools_summary: "image_generate".into(),
            attachments: vec![stepx_companion::openclaw::RealAttachment {
                kind: "image".into(),
                name: "结果.png".into(),
                mime: "image/png".into(),
                url: "https://example.test/result.png".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let items = chat_items("work", vec![message], 0);
        assert!(items
            .iter()
            .any(|item| matches!(item, AmooChatItem::ToolRecords { .. })));
        assert!(items.iter().any(|item| matches!(item, AmooChatItem::Attachment { name, url, .. } if name == "结果.png" && url == "https://example.test/result.png")));
    }

    #[test]
    fn uploaded_document_keeps_verified_phone_download_reference() {
        let source = "/sdcard/Download/StepX-Amoo/财务 数据.csv";
        let message = RealChatMessage {
            role: "user".into(),
            text: "请分析表格".into(),
            attachments: vec![stepx_companion::openclaw::RealAttachment {
                kind: "file".into(),
                name: "财务 数据.csv".into(),
                mime: "text/csv".into(),
                path: source.into(),
                size_bytes: Some(42),
                ..Default::default()
            }],
            ..Default::default()
        };
        let items = chat_items("phone-session", vec![message], 0);
        assert!(matches!(&items[0], AmooChatItem::UserText { text, .. } if text == "请分析表格"));
        assert!(
            matches!(&items[1], AmooChatItem::Attachment { path, url, size_bytes: Some(42), .. } if path == source && url.is_empty())
        );
    }

    #[test]
    fn retry_uses_visible_question_instead_of_internal_work_tool_feedback() {
        let items = vec![
            AmooChatItem::UserText {
                attachment: None,
                text: "请分析这份表格".into(),
            },
            AmooChatItem::UserText {
                attachment: None,
                text: "<amoo_work_protocol>\nAmoo Work 桌面执行结果（第 1 轮）\n{}\n</amoo_work_protocol>".into(),
            },
        ];
        assert_eq!(retry_prompt(&items, 2).as_deref(), Some("请分析这份表格"));
    }

    fn message(id: &str, run: &str, role: &str, text: &str, tools: &str) -> RealChatMessage {
        RealChatMessage {
            message_id: id.into(),
            run_id: run.into(),
            role: role.into(),
            text: text.into(),
            tools_summary: tools.into(),
            time_label: "10:31".into(),
            ..Default::default()
        }
    }

    fn key(item: &AmooChatItem) -> &str {
        match item {
            AmooChatItem::ToolRecords { key, .. } => key,
            _ => panic!("expected records"),
        }
    }

    #[test]
    fn groups_one_run_and_keeps_intermediate_and_final_text() {
        let items = chat_items(
            "session",
            vec![
                message("u", "", "user", "检查登录", ""),
                message("1", "run", "assistant", "", "ability"),
                message("2", "run", "assistant", "正在检查", "exec"),
                message("3", "run", "assistant", "", "ability · exec"),
                message("4", "run", "assistant", "已完成检查", ""),
                message("5", "run", "assistant", "", ""),
            ],
            0,
        );
        assert_eq!(items.len(), 4);
        assert!(
            matches!(&items[1], AmooChatItem::ToolRecords { records, .. } if records.iter().map(|r|r.name.as_str()).collect::<Vec<_>>() == vec!["ability", "执行终端命令", "ability", "执行终端命令"])
        );
        assert!(
            matches!(&items[2], AmooChatItem::AssistantRich { text: Some(text), .. } if text == "正在检查")
        );
        assert!(
            matches!(&items[3], AmooChatItem::AssistantRich { text: Some(text), .. } if text == "已完成检查")
        );
    }

    #[test]
    fn never_merges_different_runs_or_questions() {
        let items = chat_items(
            "session",
            vec![
                message("1", "a", "assistant", "", "exec"),
                message("2", "b", "assistant", "", "exec"),
                message("3", "", "user", "下一问", ""),
                message("4", "c", "assistant", "", "ability"),
            ],
            0,
        );
        assert_eq!(items.len(), 4);
        assert_ne!(key(&items[0]), key(&items[1]));
        assert_ne!(key(&items[1]), key(&items[3]));
    }

    #[test]
    fn errors_and_thoughts_are_preserved_without_empty_replies() {
        let mut error = message("error", "run", "assistant", "部分结果", "");
        error.is_error = true;
        error.error_message = Some("权限不足".into());
        let mut thought = message("thought", "run", "assistant", "", "");
        thought.thinking = vec!["检查能力是否可用".into()];
        let items = chat_items("session", vec![thought, error], 0);
        assert!(
            matches!(&items[0], AmooChatItem::ToolRecords { records, .. } if records[0].detail.as_deref() == Some("检查能力是否可用"))
        );
        assert!(
            matches!(&items[1], AmooChatItem::AssistantRich { text: Some(text), .. } if text.contains("部分结果") && text.contains("权限不足"))
        );
        let mut empty_error = message("error", "run", "assistant", "", "");
        empty_error.is_error = true;
        assert!(
            matches!(&chat_items("session", vec![empty_error], 0)[0], AmooChatItem::AssistantRich { text: Some(text), .. } if !text.is_empty())
        );
    }

    #[test]
    fn expansion_key_is_stable_as_more_records_arrive_and_scoped_to_session() {
        let first = message("1", "run", "assistant", "", "exec");
        let before = chat_items("one", vec![first.clone()], 1);
        let after = chat_items(
            "one",
            vec![
                first.clone(),
                message("2", "run", "assistant", "", "ability"),
            ],
            1,
        );
        assert_eq!(key(&before[0]), key(&after[0]));
        assert_ne!(key(&before[0]), key(&chat_items("two", vec![first], 1)[0]));
    }

    #[test]
    fn histories_without_run_ids_only_group_adjacent_calls() {
        let items = chat_items(
            "session",
            vec![
                message("1", "", "assistant", "", "exec"),
                message("2", "", "assistant", "", "ability"),
                message("3", "", "assistant", "已完成", ""),
                message("4", "", "assistant", "", "exec"),
            ],
            0,
        );
        assert_eq!(items.len(), 3);
        assert_ne!(key(&items[0]), key(&items[2]));
    }

    #[test]
    fn retry_uses_the_question_preceding_the_selected_reply() {
        let items = chat_items(
            "session",
            vec![
                message("1", "", "user", "第一问", ""),
                message("2", "a", "assistant", "第一答", ""),
                message("3", "", "user", "第二问", ""),
                message("4", "b", "assistant", "第二答", ""),
            ],
            0,
        );
        assert_eq!(retry_prompt(&items, 1).as_deref(), Some("第一问"));
        assert_eq!(retry_prompt(&items, 3).as_deref(), Some("第二问"));
        assert!(retry_prompt(&items, 0).is_none());
    }

    #[test]
    fn legacy_model_and_error_summaries_are_not_tool_invocations() {
        let mut reply = message("1", "", "assistant", "回复", "provider/model");
        reply.provider = "provider".into();
        reply.model = "model".into();
        let mut error = message("2", "", "assistant", "请求失败", "请求失败 · 网络错误");
        error.is_error = true;
        assert_eq!(chat_items("session", vec![reply, error], 0).len(), 2);
    }

    #[test]
    fn friendly_tool_labels_and_clean_session_titles() {
        assert_eq!(
            friendly_tool_label("batch_file_upload_return_all"),
            "批量上传文件"
        );
        assert_eq!(friendly_tool_label("task_result_submit"), "提交任务成果");
        assert_eq!(friendly_tool_label("sessions_spawn"), "启动协同子智能体");
        assert_eq!(friendly_tool_label("task_create"), "创建拆解任务");
        assert_eq!(friendly_tool_label("text_to_image"), "AI 图像生成");

        assert_eq!(session_title("main"), "主对话");
        assert_eq!(session_title("agent:main:main"), "主对话");
        assert_eq!(
            session_title("{\"amoo_work\":1,\"summary\":\"整理季度财务报表\",\"tools\":["),
            "文件任务 · 整理季度财务报表"
        );
        assert_eq!(
            session_title("{\"amoo_work\":1,\"tools\":[{\"id\":\"1\""),
            "文件任务"
        );
    }
}
