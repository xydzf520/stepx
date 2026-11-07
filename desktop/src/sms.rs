use crate::{adb::Adb, device::Device};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};

#[derive(Clone, Debug)]
pub struct Message {
    /// Provider identity is absent on bridge versions predating incremental reads.
    pub provider_id: Option<i64>,
    pub address: String,
    pub body: String,
    pub date_ms: i64,
    pub outgoing: bool,
    pub kind: u8,
    pub read: bool,
    pub subscription: Option<i32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cursor {
    pub date_ms: i64,
    pub id: i64,
}

#[derive(Clone, Debug)]
pub struct Page {
    pub messages: Vec<Message>,
    pub next_cursor: Option<Cursor>,
    pub has_more: bool,
    pub supports_incremental: bool,
}

/// Append-only paging: deleted messages and read-status changes are not a change feed.
/// Older bridges are reported explicitly; callers must not poll them as incremental.
pub fn read_page(adb: &Adb, device: &Device, after: Option<Cursor>, limit: usize) -> Result<Page> {
    let cursor = after.unwrap_or_default();
    let value = crate::bridge::call_bridge(
        adb,
        device,
        &serde_json::json!({
            "op": "read_sms", "after_date_ms": cursor.date_ms.max(0),
            "after_id": cursor.id.max(0), "limit": limit.clamp(1, 1000),
        }),
        std::time::Duration::from_secs(30),
    )?;
    parse_page(&value)
}

pub fn parse_page(value: &serde_json::Value) -> Result<Page> {
    let messages = parse_messages(value)?;
    let supported = value["incremental"].as_bool() == Some(true);
    if supported {
        anyhow::ensure!(messages.iter().all(|m| m.provider_id.is_some_and(|id| id > 0)), "短信增量读取缺少稳定记录 ID");
        anyhow::ensure!(messages.windows(2).all(|pair| {
            (pair[0].date_ms, pair[0].provider_id) < (pair[1].date_ms, pair[1].provider_id)
        }), "短信增量读取顺序无效");
        anyhow::ensure!(!messages.is_empty() || value["has_more"].as_bool() != Some(true), "短信分页未返回可继续的记录");
    }
    let next_cursor = if supported {
        messages.last().and_then(|m| m.provider_id.map(|id| Cursor { date_ms: m.date_ms, id }))
    } else { None };
    Ok(Page {
        messages,
        next_cursor,
        has_more: supported && value["has_more"].as_bool().unwrap_or(false),
        supports_incremental: supported,
    })
}

/// Read structured records without parsing unescaped `content query` output.
pub fn read(adb: &Adb, device: &Device) -> Result<Vec<Message>> {
    let result = crate::bridge::call_bridge(
        adb,
        device,
        &serde_json::json!({ "op": "read_sms" }),
        std::time::Duration::from_secs(30),
    )
    .context("无法读取手机短信，请确认调试通道有读取权限")?;
    parse_messages(&result)
}

pub fn parse_messages(value: &serde_json::Value) -> Result<Vec<Message>> {
    let rows = value
        .get("rows")
        .and_then(serde_json::Value::as_array)
        .context("手机未返回短信列表")?;
    rows.iter()
        .map(|row| {
            let kind = crate::device::provider_number(row, "type")?;
            Ok(Message {
                provider_id: row["_id"].as_i64().or_else(|| row["_id"].as_str().and_then(|s| s.parse().ok())),
                address: row["address"].as_str().unwrap_or("").to_owned(),
                body: row["body"].as_str().unwrap_or("").to_owned(),
                date_ms: crate::device::provider_number(row, "date")?,
                outgoing: kind != 1,
                kind: kind as u8,
                read: crate::device::provider_number(row, "read")? != 0,
                subscription: row["sub_id"].as_str().and_then(|s| s.parse().ok()),
            })
        })
        .collect()
}

impl Message {
    pub fn status_label(&self) -> &'static str {
        match self.kind {
            1 => "已接收",
            2 => "已发送",
            3 => "草稿",
            4 => "发件箱",
            5 => "发送失败",
            6 => "排队中",
            _ => "状态未知",
        }
    }
}

pub fn decode_field(encoded: &str, name: &str) -> Result<String> {
    let decoded = String::from_utf8(STANDARD.decode(encoded)?)?;
    let value = decoded
        .strip_prefix(&format!("Row: 0 {name}="))
        .context("短信已删除或读取被系统拒绝，请重试")?;
    Ok(value.strip_suffix('\n').unwrap_or(value).to_owned())
}

/// Normalize formatting and mainland China (+86 / 0086 / 86) mobile prefixes so threads,
/// contacts, and call logs merge cleanly.
pub fn normalize_address(address: &str) -> String {
    let cleaned: String = address
        .chars()
        .filter(|c| !c.is_whitespace() && !"-().\u{00a0}\u{200b}\u{202a}\u{202c}".contains(*c))
        .flat_map(char::to_lowercase)
        .collect();
    let stripped = cleaned
        .strip_prefix("+86")
        .or_else(|| cleaned.strip_prefix("0086"))
        .or_else(|| {
            if cleaned.len() == 13 && cleaned.starts_with("861") {
                cleaned.strip_prefix("86")
            } else {
                None
            }
        });
    if let Some(rest) = stripped {
        let bytes = rest.as_bytes();
        if bytes.len() == 11
            && bytes[0] == b'1'
            && (b'3'..=b'9').contains(&bytes[1])
            && bytes.iter().all(|b| b.is_ascii_digit())
        {
            return rest.to_string();
        }
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: Option<i64>, date: i64) -> serde_json::Value {
        let mut row = serde_json::json!({"address":"10086","body":"提醒","date":date.to_string(),"type":"1","read":"0","sub_id":"1"});
        if let Some(id) = id { row["_id"] = id.to_string().into(); }
        row
    }

    #[test]
    fn same_millisecond_messages_keep_distinct_ids_and_page_cursor() {
        let page = parse_page(&serde_json::json!({"incremental":true,"has_more":true,"rows":[row(Some(7),100),row(Some(8),100)]})).unwrap();
        assert_eq!(page.messages.len(), 2);
        assert_eq!(page.next_cursor, Some(Cursor { date_ms:100,id:8 }));
        assert!(page.has_more && page.supports_incremental);
    }

    #[test]
    fn legacy_bridge_is_explicitly_not_incremental() {
        let page = parse_page(&serde_json::json!({"rows":[row(None,100)]})).unwrap();
        assert!(!page.supports_incremental);
        assert_eq!(page.next_cursor, None);
        assert_eq!(page.messages[0].provider_id, None);
    }

    #[test]
    fn malformed_incremental_page_cannot_silently_advance_or_loop() {
        for value in [
            serde_json::json!({"incremental":true,"rows":[row(None,100)]}),
            serde_json::json!({"incremental":true,"rows":[row(Some(9),100),row(Some(8),100)]}),
            serde_json::json!({"incremental":true,"has_more":true,"rows":[]}),
        ] { assert!(parse_page(&value).is_err()); }
    }

    #[test]
    fn normalizes_mainland_mobile_prefixes_to_same_key() {
        assert_eq!(normalize_address("+8613322988887"), "13322988887");
        assert_eq!(normalize_address("+86 133-2298-8887"), "13322988887");
        assert_eq!(normalize_address("008613322988887"), "13322988887");
        assert_eq!(normalize_address("8613322988887"), "13322988887");
        assert_eq!(normalize_address("13322988887"), "13322988887");
        assert_eq!(normalize_address("1069800095555"), "1069800095555");
    }
}
