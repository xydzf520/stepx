//! Persistence boundaries for actual source-backed artifacts, not UI snapshots.
use stepx_companion::{context_ledger::{self as ledger, outcomes::{self, Target}}, phone_context::PhoneMessage};
use serde_json::{json, Value};
use std::{fs, path::Path};

fn fixture(root: &Path) -> (ledger::Capture, outcomes::GenerationRequest, String) {
    let now = 1_793_318_400_000;
    let capture = ledger::Capture { device: "test-phone".into(), account: Some("account-a".into()), now_ms: now,
        utc_offset_seconds: 8 * 3600, connected: true,
        phone_discussions: vec![PhoneMessage { device: "test-phone".into(), account: "account-a".into(),
            session_key: "session-one".into(), message_id: "message-one".into(), title: "项目：读书会".into(), role: "user".into(),
            text: "项目：读书会。我们计划组织读书会，场地报价仍待确认，方案还需讨论。请整理具体方案与待办。".into(),
            timestamp_ms: Some(now - 60_000), ..Default::default() }], ..Default::default() };
    let view = ledger::process(root, &capture).unwrap();
    let request = outcomes::prepare(root, "test-phone", Some("account-a"), &Target::Item(view.items[0].id.clone())).unwrap();
    let citation = json!([{"id":request.sources[0].id,"quote":request.sources[0].text,"version":"current"}]);
    let row = |text: &str| json!({"text":text,"evidence":citation});
    let draft = json!({"schema":1,"conclusion":row("当前需要先核对场地报价再确定方案"),
        "key_facts":[row("场地报价仍待确认，方案还需讨论")],
        "plan":[row("先询问场地报价并整理报价单"),row("根据确认后的报价制作执行方案")],
        "todos":[{"action":"询问场地报价并保存报价单","kind":"suggested","owner":null,"due":null,"evidence":citation}],
        "unknowns":[row("场地报价仍待确认，需进一步询问")],"reuse":[],"next_period":[]});
    (capture, request, draft.to_string())
}
#[test]
fn forged_request_scope_or_quotes_cannot_save_a_claim() {
    let root = tempfile::tempdir().unwrap();
    let (_, mut request, raw) = fixture(root.path());
    assert!(outcomes::save(root.path(), "test-phone", Some("account-b"), &request, &raw).is_err());
    let mut draft: Value = serde_json::from_str(&raw).unwrap();
    request.sources[0].text = "场地已经签约，费用已经支付".into();
    draft["key_facts"][0]["text"] = json!("场地已经签约，费用已经支付");
    draft["key_facts"][0]["evidence"][0]["quote"] = json!(request.sources[0].text);
    assert!(outcomes::save(root.path(), "test-phone", Some("account-a"), &request, &draft.to_string()).is_err(),
        "A serialized request is not source truth");
    assert!(!root.path().join(ledger::scope_id("test-phone", Some("account-b"))).exists());
}
#[test]
fn changed_or_cleared_sources_reject_a_late_model_reply() {
    let root = tempfile::tempdir().unwrap();
    let (mut capture, request, raw) = fixture(root.path());
    capture.now_ms += 60_000;
    capture.phone_discussions[0].text = "项目：读书会。计划取消，暂不询问场地报价。".into();
    ledger::process(root.path(), &capture).unwrap();
    assert!(outcomes::save(root.path(), "test-phone", Some("account-a"), &request, &raw).is_err());
    ledger::clear(root.path(), "test-phone", Some("account-a")).unwrap();
    assert!(outcomes::save(root.path(), "test-phone", Some("account-a"), &request, &raw).is_err());
    assert!(!root.path().join(ledger::scope_id("test-phone", Some("account-a"))).exists(), "Late saves cannot resurrect cleared material");
}
#[test]
fn edited_document_is_preserved_and_new_manifest_points_at_new_copy() {
    let root = tempfile::tempdir().unwrap();
    let (_, request, raw) = fixture(root.path());
    let first = outcomes::save(root.path(), "test-phone", Some("account-a"), &request, &raw).unwrap();
    let repeated = outcomes::save(root.path(), "test-phone", Some("account-a"), &request, &raw).unwrap();
    assert_eq!(serde_json::to_value(&first).unwrap(), serde_json::to_value(&repeated).unwrap());
    fs::write(&first.files[0].path, "用户手工修改后的方案，必须保留。\n").unwrap();
    let revised = outcomes::save(root.path(), "test-phone", Some("account-a"), &request, &raw).unwrap();
    assert_ne!(first.files[0].path, revised.files[0].path);
    assert_eq!(fs::read_to_string(&first.files[0].path).unwrap(), "用户手工修改后的方案，必须保留。\n");
    assert_ne!(first.manifest_path, revised.manifest_path);
    let actual: outcomes::OutcomeBundle = serde_json::from_slice(&fs::read(&revised.manifest_path).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(revised).unwrap());
}
#[test]
#[cfg(unix)]
fn scope_and_ledger_links_cannot_import_other_paths() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap(); let outside = tempfile::tempdir().unwrap();
    let (_, request, _) = fixture(outside.path());
    let scope = ledger::scope_id("test-phone", Some("account-a"));
    symlink(outside.path().join(&scope), root.path().join(&scope)).unwrap();
    assert!(outcomes::prepare(root.path(), "test-phone", Some("account-a"), &request.target).is_err());
    fs::remove_file(root.path().join(&scope)).unwrap(); fs::create_dir(root.path().join(&scope)).unwrap();
    symlink(outside.path().join(&scope).join("ledger.json"), root.path().join(&scope).join("ledger.json")).unwrap();
    assert!(outcomes::prepare(root.path(), "test-phone", Some("account-a"), &request.target).is_err());
}
