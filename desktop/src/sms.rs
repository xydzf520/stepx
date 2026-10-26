use crate::{adb::Adb, device::Device};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};

#[derive(Clone, Debug)]
pub struct Message {
    pub address: String,
    pub body: String,
    pub date_ms: i64,
    pub outgoing: bool,
    pub kind: u8,
    pub read: bool,
    pub subscription: Option<i32>,
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
