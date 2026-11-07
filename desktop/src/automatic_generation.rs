//! Durable identities for unattended model requests, including repairs.
//! This is a replay guard, not a model quota: the phone/provider decides usage.
//! Kept outside document scopes so clearing documents cannot replay an unknown
//! request. Each identity has its own file; no daily or cumulative request cap.
use crate::{context_ledger, model_source::Source};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::{Path, PathBuf}};

// Only bounds reads of the old aggregate format. New requests do not append to
// it or use its size, count or dates as an admission/phone-allowance decision.
const MAX_LEGACY_FILE_BYTES: u64 = 1024 * 1024;
#[derive(Deserialize)]
struct LegacyLedger { schema: u32, scope_id: String, entries: Vec<LegacyReservation> }
#[derive(Deserialize)]
struct LegacyReservation { id: String, source: Source }
#[derive(Serialize)]
struct RequestIntent<'a> {
    schema: u32,
    scope_id: &'a str,
    source: Source,
    request_id: &'a str,
    started_ms: i64,
}
fn check_directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(meta.is_dir() && !meta.file_type().is_symlink(), "后台请求登记目录不能是链接");
    Ok(())
}
fn create_private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    check_directory(path)?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn legacy_contains(path: &Path, scope: &str, source: Source, id: &str) -> Result<bool> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("无法读取旧后台请求登记"),
    };
    ensure!(meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= MAX_LEGACY_FILE_BYTES,
        "旧后台请求登记无效，未发送模型请求");
    let ledger: LegacyLedger = serde_json::from_slice(&fs::read(path)?)
        .context("旧后台请求登记损坏，未发送模型请求")?;
    ensure!(ledger.schema == 1 && ledger.scope_id == scope, "旧后台请求登记不属于当前设备账号");
    Ok(ledger.entries.iter().any(|entry| entry.source == source && entry.id == id))
}
fn intent_path(dir: &Path, source: Source, id: &str) -> Result<PathBuf> {
    let identity = serde_json::to_string(&(source, id))?;
    let key = context_ledger::scope_id(&identity, Some("automatic-request-intent-v2"));
    Ok(dir.join(format!("{key}.json")))
}

/// Save one request identity before the pipeline's durable attempt and before
/// any model I/O. An already recorded or partially recorded identity is never
/// replayed, including after restart, date changes or clearing documents.
/// No phone credentials, token allowance or desktop request quota are involved.
pub fn record_intent(root: &Path, device: &str, account: Option<&str>, source: Source,
    target: &str, revision: &str, attempt: usize, started_ms: i64) -> Result<()> {
    ensure!(!device.trim().is_empty() && !target.is_empty() && !revision.is_empty() && attempt > 0,
        "后台生成请求身份不完整");
    ensure!(source != Source::PhoneAccount || account.is_some_and(|value| !value.trim().is_empty()),
        "手机账号尚未确认，未登记或发送后台请求");
    let id = serde_json::to_string(&(target, revision, attempt))?;
    ensure!(id.len() <= 2048, "后台生成请求身份过长");
    create_private_directory(root)?;
    let registry = root.join(".automatic-generation");
    create_private_directory(&registry)?;
    let scope = context_ledger::scope_id(device, account);
    ensure!(!legacy_contains(&registry.join(format!("{scope}.json")), &scope, source, &id)?,
        "该后台请求已有发送意图，未重复发送；请先核对已有结果");
    let scope_dir = registry.join(&scope);
    create_private_directory(&scope_dir)?;
    let dir = scope_dir.join("requests");
    create_private_directory(&dir)?;
    let path = intent_path(&dir, source, &id)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // create_new is the cross-process claim: no count/size scan or rollover.
    let mut file = match options.open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            anyhow::bail!("该后台请求已有发送意图，未重复发送；请先核对已有结果")
        }
        Err(error) => return Err(error).context("无法登记后台请求，未发送模型请求"),
    };
    let intent = RequestIntent {schema: 2, scope_id: &scope, source, request_id: &id, started_ms};
    file.write_all(&serde_json::to_vec(&intent)?)
        .context("后台请求登记未完成，未发送模型请求")?;
    file.sync_all().context("无法持久保存后台请求意图，未发送模型请求")?;
    #[cfg(unix)] {
        fs::File::open(&dir)?.sync_all()?;
        fs::File::open(&scope_dir)?.sync_all()?;
        fs::File::open(&registry)?.sync_all()?;
        fs::File::open(root)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const NOW: i64 = 1_793_620_800_000;
    fn admit(root: &Path, attempt: usize) -> Result<()> {
        record_intent(root, "phone", Some("account"), Source::PhoneAccount, "day", "revision", attempt, NOW)
    }
    #[test]
    fn distinct_concurrent_requests_are_not_limited_by_a_desktop_daily_quota() {
        let temp = tempfile::tempdir().unwrap();
        let threads: Vec<_> = (1..=32).map(|attempt| {
            let root = temp.path().to_owned();
            std::thread::spawn(move || admit(&root, attempt))
        }).collect();
        for thread in threads { thread.join().unwrap().unwrap(); }
        admit(temp.path(), 33).unwrap();
        assert!(admit(temp.path(), 1).is_err());
    }
    #[test]
    fn an_identical_concurrent_request_is_claimed_once_and_survives_date_changes() {
        let temp = tempfile::tempdir().unwrap();
        let threads: Vec<_> = (0..16).map(|_| {
            let root = temp.path().to_owned();
            std::thread::spawn(move || admit(&root, 1).is_ok())
        }).collect();
        assert_eq!(threads.into_iter().map(|thread| thread.join().unwrap()).filter(|ok| *ok).count(), 1);
        assert!(record_intent(temp.path(), "phone", Some("account"), Source::PhoneAccount,
            "day", "revision", 1, NOW + 91 * 86_400_000).is_err());
        record_intent(temp.path(), "phone", Some("other"), Source::PhoneAccount, "day", "revision", 1, NOW).unwrap();
        record_intent(temp.path(), "phone", Some("account"), Source::IndependentApi, "day", "revision", 1, NOW).unwrap();
    }
    #[test]
    fn old_reservations_prevent_replay_without_retaining_the_old_quota() {
        let temp = tempfile::tempdir().unwrap();
        let scope = context_ledger::scope_id("phone", Some("account"));
        let registry = temp.path().join(".automatic-generation");
        fs::create_dir(&registry).unwrap();
        let legacy = registry.join(format!("{scope}.json"));
        let entries: Vec<_> = (1..=12).map(|attempt| serde_json::json!({
            "id": serde_json::to_string(&("day", "revision", attempt)).unwrap(),
            "source": Source::PhoneAccount, "local_day": "2026-11-02", "at_ms": NOW
        })).collect();
        let bytes = serde_json::to_vec(&serde_json::json!({"schema":1,"scope_id":scope,"entries":entries})).unwrap();
        fs::write(&legacy, &bytes).unwrap();
        assert!(admit(temp.path(), 1).is_err());
        for attempt in 13..=32 { admit(temp.path(), attempt).unwrap(); }
        assert_eq!(fs::read(legacy).unwrap(), bytes);
    }
    #[test]
    fn interrupted_intents_and_document_clear_do_not_authorize_replay() {
        let temp = tempfile::tempdir().unwrap();
        admit(temp.path(), 1).unwrap();
        let scope = context_ledger::scope_id("phone", Some("account"));
        let documents = temp.path().join(&scope);
        fs::create_dir(&documents).unwrap();
        fs::remove_dir_all(&documents).unwrap();
        let dir = temp.path().join(".automatic-generation").join(scope).join("requests");
        let path = intent_path(&dir, Source::PhoneAccount, &serde_json::to_string(&("day", "revision", 1)).unwrap()).unwrap();
        fs::write(&path, b"interrupted").unwrap();
        assert!(admit(temp.path(), 1).is_err());
        assert_eq!(fs::read(path).unwrap(), b"interrupted");
        admit(temp.path(), 2).unwrap();
    }
    #[test]
    fn corrupt_legacy_records_and_symlinks_are_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let scope = context_ledger::scope_id("phone", Some("account"));
        let registry = temp.path().join(".automatic-generation");
        fs::create_dir(&registry).unwrap();
        let legacy = registry.join(format!("{scope}.json"));
        fs::write(&legacy, b"broken").unwrap();
        assert!(admit(temp.path(), 1).is_err());
        assert_eq!(fs::read(&legacy).unwrap(), b"broken");
        #[cfg(unix)] {
            fs::remove_file(&legacy).unwrap();
            let outside = temp.path().join("unrelated.txt");
            fs::write(&outside, b"original").unwrap();
            std::os::unix::fs::symlink(&outside, &legacy).unwrap();
            assert!(admit(temp.path(), 1).is_err());
            assert_eq!(fs::read(outside).unwrap(), b"original");
        }
        let other = tempfile::tempdir().unwrap();
        assert!(record_intent(other.path(), "phone", None, Source::PhoneAccount, "day", "revision", 1, NOW).is_err());
        assert!(!other.path().join(".automatic-generation").exists());
    }
}
