//! Independent adversarial review of source-backed outcome persistence.
//! Synthetic inputs only: no model, ADB, network, or user data is used.
use serde_json::{json, Value};
use stepx_companion::{
    context_ledger::{self as ledger, outcomes::{self, GenerationRequest, Target}},
    phone_context::PhoneMessage,
};

const DEVICE: &str = "independent-review-phone";
const ACCOUNT: &str = "independent-review-account";
const INTRO: &str = "项目：图书交流会。活动方案仍需整理，场地仍需询价，报名入口还需制作。请整理执行方案与待办。";

struct Fixture {
    root: tempfile::TempDir,
    request: GenerationRequest,
    draft: Value,
}
impl Fixture {
    fn new(extra: &[&str]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let now = 1_793_318_400_000;
        let texts: Vec<_> = std::iter::once(INTRO).chain(extra.iter().copied()).collect();
        let capture = ledger::Capture {
            device: DEVICE.into(), account: Some(ACCOUNT.into()), now_ms: now,
            utc_offset_seconds: 8 * 3600, connected: true,
            phone_discussions: texts.iter().enumerate().map(|(index, text)| PhoneMessage {
                device: DEVICE.into(), account: ACCOUNT.into(), session_key: "book-club-review".into(),
                message_id: format!("message-{index}"), title: "图书交流会".into(), role: "user".into(),
                text: (*text).into(), timestamp_ms: Some(now - 60_000 + index as i64 * 1000),
                ..Default::default()
            }).collect(), ..Default::default()
        };
        let overview = ledger::process(root.path(), &capture).unwrap();
        assert_eq!(overview.items.len(), 1);
        let request = outcomes::prepare(root.path(), DEVICE, Some(ACCOUNT), &Target::Item(overview.items[0].id.clone())).unwrap();
        let intro = request.sources.iter().find(|s| s.text == INTRO).unwrap();
        let citation = json!([{"id": intro.id,"quote": INTRO,"version":"current"}]);
        let row = |text: &str| json!({"text":text,"evidence":citation});
        let draft = json!({
            "schema":1,
            "conclusion":row("目前需要整理活动方案并明确未决安排"),
            "key_facts":[row("活动方案与执行细节仍需要整理")],
            "plan":[row("先把活动约束整理成可核对的执行方案"),row("再核对活动的未决信息并完善执行顺序")],
            "todos":[{"action":"整理活动执行清单并核对未决信息","kind":"suggested","owner":null,"due":null,"evidence":citation}],
            "unknowns":[],"reuse":[],"next_period":[]
        });
        Self { root, request, draft }
    }
    fn reference(&self, quote: &str) -> Value {
        let source = self.request.sources.iter().find(|s| s.text.contains(quote)).unwrap();
        json!({"id":source.id,"quote":quote,"version":"current"})
    }
    fn row(&self, text: &str, quote: &str) -> Value {
        json!({"text":text,"evidence":[self.reference(quote)]})
    }
    fn assert_rejected(self, reason: &str) {
        let result = outcomes::save(self.root.path(), DEVICE, Some(ACCOUNT), &self.request, &self.draft.to_string());
        assert!(result.is_err(), "{reason}: an unsupported result was persisted");
    }

    fn pipeline_path(&self) -> std::path::PathBuf {
        let key = ledger::scope_id(
            &serde_json::to_string(&self.request.target).unwrap(),
            Some(&self.request.input_revision),
        );
        self.root.path().join(&self.request.scope_id).join("pipeline").join(format!("{key}.json"))
    }

    fn seed_failed_job(&self) -> Value {
        let path = self.pipeline_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let attempts: Vec<Value> = (0..3).map(|index| {
            let raw = path.parent().unwrap().join(format!("rejected-{index}.json.txt"));
            std::fs::write(&raw, format!("Rejected synthetic original draft {index}\n")).unwrap();
            json!({"started_ms":1000 + index,"raw_path":raw,"model":"synthetic-no-network",
                "error":format!("来源校验未通过：合成错误 {index}")})
        }).collect();
        let job = json!({"schema":1,"scope_id":self.request.scope_id,"target":self.request.target,
            "input_revision":self.request.input_revision,"attempts":attempts,"bundle":null,
            "deliveries":[],"last_delivery_error":null,"last_delivery_attempt_ms":0,
            "round_start":0,"retries":[]});
        self.write_job(&job);
        job
    }

    fn write_job(&self, job: &Value) {
        stepx_companion::data_store::write_json(&self.pipeline_path(), &serde_json::to_vec(job).unwrap()).unwrap();
    }

    fn read_job(&self) -> Value {
        serde_json::from_slice(&stepx_companion::data_store::read_bytes(&self.pipeline_path(), 2 * 1024 * 1024).unwrap()).unwrap()
    }
}

#[test]
fn independent_control_valid_suggestion_saves() {
    let f = Fixture::new(&[]);
    outcomes::save(f.root.path(), DEVICE, Some(ACCOUNT), &f.request, &f.draft.to_string()).unwrap();
}

#[test]
fn payment_and_signature_completion_need_actual_source_support() {
    let mut f = Fixture::new(&[]);
    f.draft["conclusion"] = f.row("场地合同已经签署，费用已经支付，可以直接执行活动", INTRO);
    f.assert_rejected("A request to prepare a plan is not a signed contract or payment receipt");
}

#[test]
fn plan_section_cannot_smuggle_an_asserted_completion() {
    let mut f = Fixture::new(&[]);
    f.draft["plan"][0] = f.row("场地已预订，可以直接发布报名信息", INTRO);
    f.assert_rejected("Putting a completion assertion in a plan does not turn it into a suggestion");
}

#[test]
fn unrelated_pending_clause_does_not_negate_a_completion_assertion() {
    let mut f = Fixture::new(&[]);
    f.draft["conclusion"] = f.row("报名入口已经完成但场地待确认", INTRO);
    f.assert_rejected("A pending venue does not negate the unsupported claim that the entry form is complete");
}

#[test]
fn three_cited_previews_cannot_prove_the_users_complete_cross_app_activity() {
    let previews = ["校园服务：晚餐取餐提醒已送达。", "配送通知：订单正在配送途中。", "生活提醒：附近门店营业安排更新。"];
    for claim in ["用户本期未向任何会话发送新消息。", "所有用户发出的消息均发生于采集时段开始前。", "本期无新互动，也无工作推进。"] {
        let mut f = Fixture::new(&previews);
        f.draft["key_facts"][0] = json!({"text":claim,"evidence":previews.iter().map(|preview| f.reference(preview)).collect::<Vec<_>>()});
        let error = outcomes::save(f.root.path(), DEVICE, Some(ACCOUNT), &f.request, &f.draft.to_string()).unwrap_err().to_string();
        assert!(error.contains("有限采集"), "The public persistence path must reject the coverage inference: {error}");
    }
    let mut f = Fixture::new(&previews);
    f.draft["key_facts"][0] = f.row("在本次已采集材料中，出现生活服务提醒，无法确认其他会话的发送历史。", previews[0]);
    outcomes::save(f.root.path(), DEVICE, Some(ACCOUNT), &f.request, &f.draft.to_string()).unwrap();
}

#[test]
fn current_fact_cannot_retain_a_superseded_owner() {
    let old = "小林负责报名入口，我负责询价。";
    let mut f = Fixture::new(&[old, "分工调整，我负责报名入口，小林负责询价。"]);
    f.draft["key_facts"][0] = f.row("当前由小林负责报名入口，分工已经明确", "小林负责报名入口");
    f.assert_rejected("The old owner must not survive in key_facts just because todo.owner is checked separately");
}

#[test]
fn explicit_but_superseded_deadline_cannot_survive_in_todo_metadata() {
    let old = "我10月31日18点前做好报名入口。";
    let mut f = Fixture::new(&[old, "报名入口截止时间改为11月1日20点前。"]);
    f.draft["todos"][0] = json!({
        "action":"制作报名入口并形成可检查的页面","kind":"suggested","owner":null,
        "due":{"value":"10月31日18点前","evidence":f.reference(old)},
        "evidence":[f.reference(old)]
    });
    f.assert_rejected("A real old deadline is still obsolete after an explicit deadline change");
}

#[test]
fn deadline_marker_in_another_clause_does_not_turn_event_time_into_due_date() {
    let source = "之前讨论过活动，活动周日15点举行。";
    let mut f = Fixture::new(&[source]);
    f.draft["todos"][0] = json!({
        "action":"核对活动安排并形成执行清单","kind":"suggested","owner":null,
        "due":{"value":"周日15点","evidence":f.reference(source)},
        "evidence":[f.reference(source)]
    });
    f.assert_rejected("The unrelated word 之前 is not a deadline marker for the event time");
}

#[test]
fn an_unrecognized_action_cannot_borrow_a_different_actions_owner_and_deadline() {
    let source = "小王负责报名，并在周五前提交报名表。";
    let mut f = Fixture::new(&[source]);
    f.draft["todos"][0] = json!({
        "action":"打包物资并形成装箱核对清单","kind":"suggested",
        "owner":{"value":"小王","evidence":f.reference("小王负责报名")},
        "due":{"value":"周五前","evidence":f.reference("并在周五前提交报名表")},
        "evidence":[f.reference(source)]
    });
    f.assert_rejected("Fallback topic matching must not bind an unrelated person's assignment to a new suggestion");
}

#[test]
fn actual_payment_and_signature_reports_remain_usable_as_reported_facts() {
    let source = "图书交流会的场地合同已经签署，费用已经支付。";
    let mut f = Fixture::new(&[source]);
    f.draft["conclusion"] = f.row("场地合同已经签署，费用已经支付，接下来核对执行安排", source);
    outcomes::save(f.root.path(), DEVICE, Some(ACCOUNT), &f.request, &f.draft.to_string()).unwrap();
}

#[test]
fn a_completed_activity_cannot_prove_completion_of_an_unlisted_activity() {
    let source = "报名已经完成。";
    let mut f = Fixture::new(&[source]);
    f.draft["conclusion"] = f.row("数据库迁移已经完成，接下来核对业务连通性", source);
    f.assert_rejected("An activity absent from the common-topic list still needs evidence about that activity");
}

#[test]
fn a_completed_unlisted_activity_remains_usable_with_its_own_evidence() {
    let source = "我确认数据库迁移已完成。";
    let mut f = Fixture::new(&[source]);
    f.draft["conclusion"] = f.row("数据库迁移已经完成，接下来核对业务连通性", source);
    outcomes::save(f.root.path(), DEVICE, Some(ACCOUNT), &f.request, &f.draft.to_string()).unwrap();
}

#[test]
fn latest_explicit_deadline_remains_usable() {
    let old = "我10月31日18点前做好报名入口。";
    let revised = "报名入口截止时间改为11月1日20点前。";
    let mut f = Fixture::new(&[old, revised]);
    f.draft["todos"][0] = json!({
        "action":"制作报名入口并形成可检查的页面","kind":"suggested","owner":null,
        "due":{"value":"11月1日20点前","evidence":f.reference(revised)},
        "evidence":[f.reference(revised)]
    });
    outcomes::save(f.root.path(), DEVICE, Some(ACCOUNT), &f.request, &f.draft.to_string()).unwrap();
}

#[test]
fn new_suggested_actions_without_a_sourced_assignment_remain_usable() {
    let mut f = Fixture::new(&[]);
    f.draft["todos"][0]["action"] = json!("打包物资并形成装箱核对清单");
    outcomes::save(f.root.path(), DEVICE, Some(ACCOUNT), &f.request, &f.draft.to_string()).unwrap();
}

#[test]
fn delivery_only_refuses_a_stale_revision_before_creating_a_job_or_touching_adb() {
    use stepx_companion::{adb::Adb, context_pipeline, device::Device};
    use std::sync::atomic::AtomicBool;
    let f = Fixture::new(&[]);
    let result = context_pipeline::deliver_existing(f.root.path(),
        &Adb { path: "/nonexistent/review-adb".into() },
        &Device { serial: DEVICE.into(), ..Default::default() }, Some(ACCOUNT), &f.request.target,
        "stale-revision", &AtomicBool::new(false));
    assert!(result.unwrap_err().to_string().contains("资料已更新"));
    assert!(!f.root.path().join(&f.request.scope_id).join("pipeline").exists());
}

#[test]
fn delivery_only_missing_saved_job_never_falls_back_to_model_generation() {
    use stepx_companion::{adb::Adb, context_pipeline, device::Device};
    use std::sync::atomic::AtomicBool;
    let f = Fixture::new(&[]);
    // A document alone is insufficient to assume the user's requested delivery
    // has a durable, exact-revision pipeline record.
    outcomes::save(f.root.path(), DEVICE, Some(ACCOUNT), &f.request, &f.draft.to_string()).unwrap();
    let result = context_pipeline::deliver_existing(f.root.path(),
        &Adb { path: "/nonexistent/review-adb".into() },
        &Device { serial: DEVICE.into(), ..Default::default() }, Some(ACCOUNT), &f.request.target,
        &f.request.input_revision, &AtomicBool::new(false));
    assert!(result.unwrap_err().to_string().contains("回传操作不会重新调用模型"));
    let pipeline = f.root.path().join(&f.request.scope_id).join("pipeline");
    assert!(std::fs::read_dir(pipeline).unwrap().all(|entry|
        entry.unwrap().path().extension().is_some_and(|extension| extension == "lock")));
}

#[test]
fn explicit_retry_keeps_every_rejected_original_error_and_reason() {
    use stepx_companion::context_pipeline;
    let f = Fixture::new(&[]);
    let original = f.seed_failed_job();
    let raw_before: Vec<_> = original["attempts"].as_array().unwrap().iter()
        .map(|attempt| std::fs::read(attempt["raw_path"].as_str().unwrap()).unwrap()).collect();
    let reason = "用户修正了文本 API 配置，明确要求重新生成";
    assert!(context_pipeline::retry_failed(f.root.path(), DEVICE, Some(ACCOUNT), &f.request.target, reason).unwrap());
    let next = f.read_job();
    assert_eq!(next["attempts"], original["attempts"], "a new round must not overwrite old intents, errors or raw paths");
    assert_eq!(next["round_start"], 3);
    assert_eq!(next["retries"].as_array().unwrap().len(), 1);
    assert_eq!(next["retries"][0]["attempt_start"], 3);
    assert_eq!(next["retries"][0]["reason"], reason);
    assert!(next["retries"][0]["started_ms"].as_i64().unwrap() > 0);
    for (attempt, before) in next["attempts"].as_array().unwrap().iter().zip(raw_before) {
        assert_eq!(std::fs::read(attempt["raw_path"].as_str().unwrap()).unwrap(), before);
    }
    let persisted = stepx_companion::data_store::read_bytes(&f.pipeline_path(), 2 * 1024 * 1024).unwrap();
    assert!(!context_pipeline::retry_failed(f.root.path(), DEVICE, Some(ACCOUNT), &f.request.target, "再次点击").unwrap());
    assert_eq!(stepx_companion::data_store::read_bytes(&f.pipeline_path(), 2 * 1024 * 1024).unwrap(), persisted,
        "double clicking retry before another attempt must not create another round");
}

#[test]
fn another_explicit_retry_appends_history_without_reopening_earlier_attempt_slots() {
    use stepx_companion::context_pipeline;
    let f = Fixture::new(&[]);
    let original = f.seed_failed_job();
    context_pipeline::retry_failed(f.root.path(), DEVICE, Some(ACCOUNT), &f.request.target, "第一次明确重试").unwrap();
    let mut next = f.read_job();
    let first_retry = next["retries"][0].clone();
    // Simulate a crashed request in the new round. Its persisted intent still
    // belongs to that round even though no model response reached disk.
    next["attempts"].as_array_mut().unwrap().push(json!({
        "started_ms":2000,"raw_path":null,"model":null,"error":null
    }));
    f.write_job(&next);
    assert!(context_pipeline::retry_failed(f.root.path(), DEVICE, Some(ACCOUNT), &f.request.target, "确认上次中断后重试").unwrap());
    let retried = f.read_job();
    assert_eq!(retried["round_start"], 4);
    assert_eq!(retried["attempts"], next["attempts"]);
    assert_eq!(&retried["attempts"].as_array().unwrap()[..3], original["attempts"].as_array().unwrap());
    assert_eq!(retried["retries"].as_array().unwrap().len(), 2);
    assert_eq!(retried["retries"][0], first_retry);
    assert_eq!(retried["retries"][1]["attempt_start"], 4);
    assert_eq!(retried["retries"][1]["reason"], "确认上次中断后重试");
}

#[test]
fn explicit_retry_requires_a_reason_and_never_creates_a_missing_job() {
    use stepx_companion::context_pipeline;
    let f = Fixture::new(&[]);
    assert!(context_pipeline::retry_failed(f.root.path(), DEVICE, Some(ACCOUNT), &f.request.target, " \n\t").is_err());
    assert!(!f.pipeline_path().exists());
    assert!(!context_pipeline::retry_failed(f.root.path(), DEVICE, Some(ACCOUNT), &f.request.target, "用户明确重试").unwrap());
    assert!(!f.pipeline_path().exists(), "retry is not a model invocation or a new durable request intent");
}

#[test]
fn a_successful_bundle_cannot_be_reset_by_retry_or_regenerated_by_run() {
    use stepx_companion::{adb::Adb, context_pipeline, device::Device};
    use std::sync::atomic::AtomicBool;
    let f = Fixture::new(&[]);
    let mut job = f.seed_failed_job();
    let bundle = outcomes::save(f.root.path(), DEVICE, Some(ACCOUNT), &f.request, &f.draft.to_string()).unwrap();
    let files_before: Vec<_> = bundle.files.iter().map(|file| std::fs::read(&file.path).unwrap()).collect();
    job["bundle"] = serde_json::to_value(&bundle).unwrap();
    f.write_job(&job);
    let before = stepx_companion::data_store::read_bytes(&f.pipeline_path(), 2 * 1024 * 1024).unwrap();
    assert!(!context_pipeline::retry_failed(f.root.path(), DEVICE, Some(ACCOUNT), &f.request.target, "误点重新生成").unwrap());
    assert_eq!(stepx_companion::data_store::read_bytes(&f.pipeline_path(), 2 * 1024 * 1024).unwrap(), before);
    // A nonexistent executable makes any unexpected phone-account preflight
    // fail before a model request. Reuse must complete without that preflight.
    let result = context_pipeline::run(f.root.path(),
        &Adb { path: "/nonexistent/review-adb".into() },
        &Device { serial: DEVICE.into(), ..Default::default() }, Some(ACCOUNT), &f.request.target,
        &AtomicBool::new(false), false).unwrap();
    assert!(result.reused);
    assert_eq!(serde_json::to_value(result.bundle).unwrap(), job["bundle"]);
    assert_eq!(stepx_companion::data_store::read_bytes(&f.pipeline_path(), 2 * 1024 * 1024).unwrap(), before);
    for (file, contents) in bundle.files.iter().zip(files_before) {
        assert_eq!(std::fs::read(&file.path).unwrap(), contents);
    }
}

#[test]
fn delivery_of_failed_or_reopened_round_never_retries_or_records_a_model_intent() {
    use stepx_companion::{adb::Adb, context_pipeline, device::Device};
    use std::sync::atomic::AtomicBool;
    let f = Fixture::new(&[]);
    f.seed_failed_job();
    for reopened in [false, true] {
        if reopened {
            context_pipeline::retry_failed(f.root.path(), DEVICE, Some(ACCOUNT), &f.request.target, "用户明确开启新轮").unwrap();
        }
        let before = stepx_companion::data_store::read_bytes(&f.pipeline_path(), 2 * 1024 * 1024).unwrap();
        let error = context_pipeline::deliver_existing(f.root.path(),
            &Adb { path: "/nonexistent/review-adb".into() },
            &Device { serial: DEVICE.into(), ..Default::default() }, Some(ACCOUNT), &f.request.target,
            &f.request.input_revision, &AtomicBool::new(false)).unwrap_err();
        assert!(error.to_string().contains("回传操作不会重新调用模型"), "{error:#}");
        assert_eq!(stepx_companion::data_store::read_bytes(&f.pipeline_path(), 2 * 1024 * 1024).unwrap(), before,
            "delivery must not advance round_start, add retries, or append a request intent");
    }
}

#[test]
fn delivery_of_a_locally_edited_bundle_preserves_edits_without_regeneration() {
    use stepx_companion::{adb::Adb, context_pipeline, device::Device};
    use std::sync::atomic::AtomicBool;
    let f = Fixture::new(&[]);
    let mut job = f.seed_failed_job();
    let bundle = outcomes::save(f.root.path(), DEVICE, Some(ACCOUNT), &f.request, &f.draft.to_string()).unwrap();
    job["bundle"] = serde_json::to_value(&bundle).unwrap();
    f.write_job(&job);
    let file = &bundle.files[0].path;
    std::fs::write(file, "用户自己修订的成果，必须保留。\n").unwrap();
    let before = stepx_companion::data_store::read_bytes(&f.pipeline_path(), 2 * 1024 * 1024).unwrap();
    let error = context_pipeline::deliver_existing(f.root.path(),
        &Adb { path: "/nonexistent/review-adb".into() },
        &Device { serial: DEVICE.into(), ..Default::default() }, Some(ACCOUNT), &f.request.target,
        &f.request.input_revision, &AtomicBool::new(false)).unwrap_err();
    assert!(error.to_string().contains("成果文件已被编辑"), "{error:#}");
    assert_eq!(std::fs::read_to_string(file).unwrap(), "用户自己修订的成果，必须保留。\n");
    assert_eq!(stepx_companion::data_store::read_bytes(&f.pipeline_path(), 2 * 1024 * 1024).unwrap(), before);
}

#[test]
fn cached_bundle_cannot_override_the_jobs_account_target_or_revision() {
    use stepx_companion::{adb::Adb, context_pipeline, device::Device};
    use std::sync::atomic::AtomicBool;
    let f = Fixture::new(&[]);
    let mut job = f.seed_failed_job();
    let bundle = outcomes::save(f.root.path(), DEVICE, Some(ACCOUNT), &f.request, &f.draft.to_string()).unwrap();
    let original = serde_json::to_value(&bundle).unwrap();
    for (field, wrong) in [
        ("scope_id", json!(ledger::scope_id(DEVICE, Some("another-account")))),
        ("target", json!({"item":"another-matter"})),
        ("input_revision", json!("another-source-revision")),
    ] {
        let mut cached = original.clone();
        cached[field] = wrong;
        if field == "scope_id" {
            let foreign = f.root.path().join(cached["scope_id"].as_str().unwrap());
            std::fs::create_dir_all(&foreign).unwrap();
            let document = foreign.join("another-accounts-plan.md");
            std::fs::write(&document, "Private material belonging to a different phone account.").unwrap();
            cached["files"] = json!([{"kind":"plan","path":document,
                "sha256":stepx_companion::device::sha256_file(&document).unwrap()}]);
        }
        job["bundle"] = cached;
        f.write_job(&job);
        let before = stepx_companion::data_store::read_bytes(&f.pipeline_path(), 2 * 1024 * 1024).unwrap();
        let result = context_pipeline::run(f.root.path(),
            &Adb { path: "/nonexistent/review-adb".into() },
            &Device { serial: DEVICE.into(), ..Default::default() }, Some(ACCOUNT), &f.request.target,
            &AtomicBool::new(false), false);
        assert!(result.is_err(), "The outer job identity must not authorize a cached bundle with a different {field}");
        assert_eq!(stepx_companion::data_store::read_bytes(&f.pipeline_path(), 2 * 1024 * 1024).unwrap(), before, "Identity rejection must not call a model or rewrite history");
    }
}
