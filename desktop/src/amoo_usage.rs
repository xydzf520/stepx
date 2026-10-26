//! Usage reported by the phone's model service. Missing counters stay unknown.
//! These are inference counters, not a fabricated account bill or quota.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UsageMetrics {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    /// Input consumed by the last inference, not cumulative billed tokens.
    pub context_used: Option<u64>,
    pub context_limit: Option<u64>,
    /// Service-reported response duration; may include tool execution.
    pub response_ms: Option<u64>,
    /// Output tokens / response duration; this is average response throughput.
    pub tokens_per_second: Option<f64>,
    /// Only populated when the service also identifies USD as the currency.
    pub cost_usd: Option<f64>,
}

fn count(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        value[*key]
            .as_u64()
            .or_else(|| value[*key].as_str().and_then(|s| s.parse::<u64>().ok()))
    })
}
fn positive_number(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value[*key]
            .as_f64()
            .or_else(|| value[*key].as_str().and_then(|s| s.parse::<f64>().ok()))
            .filter(|v| v.is_finite() && *v >= 0.0)
    })
}
fn object(value: &Value) -> Option<Value> {
    if value.is_object() {
        Some(value.clone())
    } else {
        value
            .as_str()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .filter(Value::is_object)
    }
}

/// Accept only explicit usage fields. No character estimates, catalog prices or
/// session-list zero placeholders enter these counters.
pub fn parse_message(message: &Value) -> UsageMetrics {
    let metadata = object(&message["metadata"])
        .or_else(|| object(&message["meta"]))
        .unwrap_or_default();
    let usage = object(&message["usage"])
        .or_else(|| object(&message["tokenUsage"]))
        .or_else(|| object(&metadata["usage"]))
        .unwrap_or_default();
    let input = count(
        &usage,
        &["input", "inputTokens", "input_tokens", "prompt_tokens"],
    );
    let output = count(
        &usage,
        &[
            "output",
            "outputTokens",
            "output_tokens",
            "completion_tokens",
        ],
    );
    let total = count(&usage, &["totalTokens", "total_tokens"])
        .or_else(|| input.zip(output).and_then(|(a, b)| a.checked_add(b)));
    let response_ms = count(message, &["durationMs", "duration_ms", "responseMs"])
        .or_else(|| count(&metadata, &["durationMs", "duration_ms", "responseMs"]))
        .or_else(|| count(&usage, &["durationMs", "duration_ms"]))
        .filter(|n| *n > 0);
    let cost_usd = positive_number(message, &["cost_usd"])
        .or_else(|| positive_number(&metadata, &["cost_usd"]))
        .or_else(|| positive_number(&usage, &["cost_usd"]))
        .or_else(|| {
            (usage["cost"]["currency"].as_str() == Some("USD"))
                .then(|| positive_number(&usage["cost"], &["total"]))
                .flatten()
        });
    UsageMetrics {
        input_tokens: input,
        output_tokens: output,
        total_tokens: total,
        cache_read_tokens: count(
            &usage,
            &["cacheRead", "cacheReadTokens", "cache_read_input_tokens"],
        )
        .or_else(|| usage["prompt_tokens_details"]["cached_tokens"].as_u64()),
        cache_write_tokens: count(
            &usage,
            &[
                "cacheWrite",
                "cacheWriteTokens",
                "cache_creation_input_tokens",
            ],
        ),
        context_used: count(&usage, &["contextTokens", "context_tokens", "contextUsed"]).or(input),
        context_limit: count(message, &["contextWindow", "contextLimit"])
            .or_else(|| count(&metadata, &["contextWindow", "contextLimit"]))
            .or_else(|| count(&usage, &["contextWindow", "contextLimit", "context_window"]))
            .filter(|n| *n > 0),
        response_ms,
        tokens_per_second: output
            .zip(response_ms)
            .map(|(n, ms)| n as f64 * 1000.0 / ms as f64),
        cost_usd,
    }
}

impl UsageMetrics {
    pub fn has_tokens(&self) -> bool {
        self.input_tokens.is_some() || self.output_tokens.is_some() || self.total_tokens.is_some()
    }
    /// A client-measured end-to-end duration is allowed when displayed as an
    /// average response speed. It must never be labelled pure generation speed.
    pub fn with_observed_response_ms(mut self, elapsed_ms: u64) -> Self {
        if self.response_ms.is_none() && elapsed_ms > 0 && self.output_tokens.is_some() {
            self.response_ms = Some(elapsed_ms);
            self.tokens_per_second = self
                .output_tokens
                .map(|n| n as f64 * 1000.0 / elapsed_ms as f64);
        }
        self
    }
}

/// Sum a caller's explicitly scoped, deduplicated messages. Context and speed
/// remain those of the latest reported inference; repeated snapshots are not
/// accumulated. Callers must disclose when history is only partly loaded.
pub fn aggregate<'a>(metrics: impl IntoIterator<Item = &'a UsageMetrics>) -> UsageMetrics {
    fn add(target: &mut Option<u64>, next: Option<u64>) {
        if let Some(value) = next {
            *target = Some(target.unwrap_or(0).saturating_add(value));
        }
    }
    let mut result = UsageMetrics::default();
    for item in metrics {
        add(&mut result.input_tokens, item.input_tokens);
        add(&mut result.output_tokens, item.output_tokens);
        add(&mut result.total_tokens, item.total_tokens);
        add(&mut result.cache_read_tokens, item.cache_read_tokens);
        add(&mut result.cache_write_tokens, item.cache_write_tokens);
        if let Some(cost) = item.cost_usd {
            result.cost_usd = Some(result.cost_usd.unwrap_or(0.0) + cost);
        }
        if item.context_used.is_some() {
            result.context_used = item.context_used;
            result.context_limit = item.context_limit;
        }
        if item.has_tokens() {
            result.tokens_per_second = item.tokens_per_second;
            result.response_ms = item.response_ms;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn parses_real_agentbus_usage_without_treating_unknown_cost_as_billing() {
        let u = parse_message(
            &json!({"usage":{"input":27014,"output":252,"totalTokens":27266,"cacheRead":26752,"cacheWrite":0,"cost":{"total":0}}}),
        );
        assert_eq!(u.input_tokens, Some(27014));
        assert_eq!(u.output_tokens, Some(252));
        assert_eq!(u.total_tokens, Some(27266));
        assert_eq!(u.context_used, Some(27014));
        assert_eq!(u.cache_read_tokens, Some(26752));
        assert_eq!(u.cost_usd, None);
        assert_eq!(u.tokens_per_second, None);
    }
    #[test]
    fn session_placeholder_counts_and_text_do_not_become_usage() {
        let u = parse_message(
            &json!({"inputTokens":0,"outputTokens":0,"totalTokens":0,"text":"hello world"}),
        );
        assert!(!u.has_tokens());
        assert_eq!(u.context_used, None);
    }
    #[test]
    fn parses_bff_string_metadata_and_duration() {
        let u = parse_message(
            &json!({"metadata":{"usage":"{\"input_tokens\":30,\"output_tokens\":20}","duration_ms":"2000","cost_usd":"0.01","contextWindow":"128000"}}),
        );
        assert_eq!(u.total_tokens, Some(50));
        assert_eq!(u.tokens_per_second, Some(10.0));
        assert_eq!(u.cost_usd, Some(0.01));
        assert_eq!(u.context_limit, Some(128000));
    }
    #[test]
    fn negative_counts_zero_duration_and_overflow_are_not_fabricated() {
        let u = parse_message(&json!({"usage":{"input":-1,"output":10},"duration_ms":0}));
        assert_eq!(u.input_tokens, None);
        assert_eq!(u.total_tokens, None);
        assert_eq!(u.tokens_per_second, None);
        assert_eq!(
            parse_message(&json!({"usage":{"input":u64::MAX,"output":1}})).total_tokens,
            None
        );
    }
    #[test]
    fn aggregates_costs_and_counts_but_preserves_last_context() {
        let a = parse_message(&json!({"usage":{"input":30,"output":20},"duration_ms":1000}));
        let b = parse_message(&json!({"usage":{"input":50,"output":10}}));
        let u = aggregate([&a, &b]);
        assert_eq!(u.total_tokens, Some(110));
        assert_eq!(u.context_used, Some(50));
        assert_eq!(u.tokens_per_second, None);
        assert_eq!(u.context_limit, None);
    }
    #[test]
    fn actual_zero_is_preserved_and_observed_speed_requires_tokens() {
        let u = parse_message(&json!({"usage":{"input":12,"output":0}}))
            .with_observed_response_ms(2000);
        assert_eq!(u.output_tokens, Some(0));
        assert_eq!(u.tokens_per_second, Some(0.0));
        assert_eq!(
            UsageMetrics::default()
                .with_observed_response_ms(100)
                .tokens_per_second,
            None
        );
    }
}

static SCOPE_CACHE: std::sync::Mutex<Option<(String, String, std::time::Instant, Option<String>)>> =
    std::sync::Mutex::new(None);

/// Non-secret, account-scoped cache key for the Android foreground user.
/// Reads the account registry only; no credential or auth-token API is used.
pub fn account_scope(
    adb: &crate::adb::Adb,
    device: &crate::device::Device,
) -> anyhow::Result<Option<String>> {
    use anyhow::{anyhow, Context};
    use std::time::{Duration, Instant};
    let can_cache = device.serial != "phone"
        && adb
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("adb"));
    if can_cache {
        if let Ok(guard) = SCOPE_CACHE.lock() {
            if let Some((transport, serial, at, cached)) = guard.as_ref() {
                if transport == &device.transport
                    && serial == &device.serial
                    && at.elapsed() < Duration::from_secs(15)
                {
                    return Ok(cached.clone());
                }
            }
        }
    }
    let user = adb
        .shell(&device.transport, &["am", "get-current-user"])
        .map_err(|_| anyhow!("无法确定手机当前用户"))?
        .trim()
        .parse::<u32>()
        .context("手机当前用户标识无效")?;
    let dump = adb
        .shell(&device.transport, &["dumpsys", "account"])
        .map_err(|_| anyhow!("无法读取手机账号状态"))?;
    let Some(account) = account_name(&dump, user) else {
        if can_cache {
            if let Ok(mut guard) = SCOPE_CACHE.lock() {
                *guard = Some((
                    device.transport.clone(),
                    device.serial.clone(),
                    Instant::now(),
                    None,
                ));
            }
        }
        return Ok(None);
    };
    let domain = format!(
        "stepx-amoo-account-v1\0{}\0{}\0{}",
        device.serial, user, account
    );
    let digest = crate::device::sha256(domain.as_bytes());
    let scope: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if can_cache {
        if let Ok(mut guard) = SCOPE_CACHE.lock() {
            *guard = Some((
                device.transport.clone(),
                device.serial.clone(),
                Instant::now(),
                Some(scope.clone()),
            ));
        }
    }
    Ok(Some(scope))
}

fn account_name(dump: &str, user: u32) -> Option<String> {
    let heading = format!("User UserInfo{{{user}:");
    let mut current = false;
    let mut names = std::collections::BTreeSet::new();
    for line in dump.lines() {
        if line.starts_with("User ") {
            current = line.starts_with(&heading);
        }
        if !current {
            continue;
        }
        let line = line.trim();
        if let Some(body) = line.strip_prefix("Account {name=") {
            if let Some(name) = body.strip_suffix(", type=com.stepos.account}") {
                if !name.is_empty() && !matches!(name, "unknown" | "null" | "***" | "********") {
                    names.insert(name.to_string());
                }
            }
        }
    }
    if names.len() == 1 {
        names.into_iter().next()
    } else {
        None
    }
}

#[cfg(test)]
mod account_tests {
    use super::*;
    #[test]
    fn scopes_only_foreground_user_and_native_account_type() {
        let dump="User UserInfo{0:Owner:4c13}:\n  Account {name=other, type=com.google}\n  Account {name=primary, type=com.stepos.account}\nUser UserInfo{10:Work:30}:\n  Account {name=work, type=com.stepos.account}\n";
        assert_eq!(account_name(dump, 0).as_deref(), Some("primary"));
        assert_eq!(account_name(dump, 10).as_deref(), Some("work"));
        assert_eq!(account_name(dump, 11), None);
    }
    #[test]
    fn ambiguous_or_missing_native_account_is_not_cached() {
        assert_eq!(account_name("User UserInfo{0:O:0}:\nAccount {name=a, type=com.stepos.account}\nAccount {name=b, type=com.stepos.account}",0),None);
        assert_eq!(
            account_name(
                "User UserInfo{0:O:0}:\nAccount {name=unknown, type=com.stepos.account}",
                0
            ),
            None
        );
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageEntry {
    pub session_key: String,
    pub message_id: String,
    pub usage: UsageMetrics,
    pub timestamp_ms: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountUsageSnapshot {
    pub scope: String,
    pub entries: Vec<UsageEntry>,
    pub sessions_read: usize,
    pub partial: bool,
}

/// Explicit read-only account-history sync. It never sends inference or exports
/// conversation text. The bounded coverage is reported instead of claiming a bill.
pub fn fetch_account_usage(
    adb: &crate::adb::Adb,
    device: &crate::device::Device,
) -> anyhow::Result<AccountUsageSnapshot> {
    use anyhow::{bail, Context};
    use serde_json::json;
    use std::time::{Duration, Instant};
    let scope = account_scope(adb, device)?.context("未读取到唯一的手机 Amoo 账号")?;
    let mut bus = crate::amoo::AgentBus::connect(adb, device)?;
    let list = bus.call("sessions.list", json!({"limit":100}))?;
    let sessions = list["sessions"].as_array().context("手机未返回会话目录")?;
    let mut snapshot = AccountUsageSnapshot {
        scope: scope.clone(),
        entries: vec![],
        sessions_read: 0,
        partial: sessions.len() >= 100,
    };
    let deadline = Instant::now() + Duration::from_secs(55);
    let mut seen = std::collections::HashSet::new();
    for session in sessions {
        if Instant::now() >= deadline {
            snapshot.partial = true;
            break;
        }
        let Some(key) = session["key"].as_str().filter(|s| !s.is_empty()) else {
            snapshot.partial = true;
            continue;
        };
        let history = match bus.call_with_timeout(
            "chat.history",
            json!({"sessionId":key,"limit":100}),
            Duration::from_secs(4),
        ) {
            Ok(v) => v,
            Err(_) => {
                snapshot.partial = true;
                continue;
            }
        };
        let Some(messages) = history["messages"].as_array() else {
            snapshot.partial = true;
            continue;
        };
        snapshot.sessions_read += 1;
        if history["page"]["hasMore"]
            .as_bool()
            .unwrap_or(messages.len() >= 100)
        {
            snapshot.partial = true;
        }
        for row in messages {
            if row["role"] != "assistant" {
                continue;
            }
            let usage = parse_message(row);
            if !usage.has_tokens() {
                if row["text"]
                    .as_str()
                    .is_some_and(|text| !text.trim().is_empty())
                {
                    snapshot.partial = true;
                }
                continue;
            }
            let Some(id) = row["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| row["responseId"].as_str().filter(|s| !s.is_empty()))
            else {
                snapshot.partial = true;
                continue;
            };
            if !seen.insert((key.to_string(), id.to_string())) {
                continue;
            }
            snapshot.entries.push(UsageEntry {
                session_key: key.into(),
                message_id: id.into(),
                usage,
                timestamp_ms: row["timestamp"].as_i64().unwrap_or(0),
            });
        }
    }
    if account_scope(adb, device)?.as_deref() != Some(&scope) {
        bail!("同步期间手机账号已改变，未混合保存用量")
    }
    snapshot.entries.sort_by_key(|entry| entry.timestamp_ms);
    Ok(snapshot)
}
