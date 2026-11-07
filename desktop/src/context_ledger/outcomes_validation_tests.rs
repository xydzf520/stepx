use super::*;

#[test]
fn list_ordinals_do_not_authorize_amounts_decimals_or_dates() {
    let mut r=request();r.sources=vec![source("plain","请整理图片中的功能线索并核对原文。",1000)];
    let citations=vec![reference("plain","请整理图片中的功能线索并核对原文。")];
    validate_text(&r,"1. 核对图片中的功能线索。\n2. 按原文保留需要核实的问题。",&citations,false).unwrap();
    assert!(validate_text(&r,"1. 计划预算500元并核对图片中的功能线索。",&citations,false).unwrap_err().to_string().contains("数字500"));
    for text in ["按1.5元预算核对资料内容。","在2026.11.02日核对资料内容。","在2026. 11.02日核对资料内容。"] {
        assert!(validate_text(&r,text,&citations,false).is_err(),"real numeric content must remain checked: {text}");
    }
}

#[test]
fn only_exact_filenames_of_cited_sources_are_numeric_metadata() {
    let mut r=request();let mut file=source("file","请整理选定图片中的功能线索。",1000);file.title="S61101-20314951.png".into();
    r.sources=vec![file,source("other","请根据已有文字核对资料。",1000)];
    let citations=vec![reference("file","请整理选定图片中的功能线索。")];
    validate_text(&r,"核对S61101-20314951.png中的功能线索。",&citations,false).unwrap();
    for text in ["核对S61101-20314952.png中的功能线索。","核对AS61101-20314951.png中的功能线索。", "核对S61101-20314951.png.pdf中的功能线索。"] {
        assert!(validate_text(&r,text,&citations,false).is_err(),"invented filename must not be masked: {text}");
    }
    assert!(validate_text(&r,"核对S61101-20314951.png并按预算500元处理。",&citations,false).unwrap_err().to_string().contains("数字500"));
    assert!(validate_text(&r,"核对S61101-20314951.png中的功能线索。",&[reference("other","请根据已有文字核对资料。")],false).is_err(),"an uncited source title cannot authorize a filename");
}

fn source(id: &str, text: &str, time: i64) -> SourceEvidence {
    SourceEvidence { id:id.into(),matter_id:"outing".into(),title:"周末活动".into(),text:text.into(),author:"我".into(),source:"手机 Amoo 讨论".into(),locator:format!("消息 {id}"),
        occurred_ms:Some(time),observed_ms:time,changed_ms:time,kind:"讨论材料".into(),authoritative:true,user_authored:true,in_period:true,baseline:None }
}
fn request() -> GenerationRequest {
    GenerationRequest {scope_id:"scope".into(),target:Target::Item("outing".into()),input_revision:"revision".into(),title:"周末活动".into(),prompt:String::new(),
        sources:vec![source("old","活动时间是周六14点；预算600元；我负责询价，小林负责报名入口。",1000),
            source("notice","活动时间改为周日15点；预算由600元降到480元；小林负责询价，我负责报名入口。",2000),
            source("confirmed","我10月31日18点前做好手机报名入口，小林10月31日12点前询价；场地还未订，报名尚未开放，费用未定。",3000)],
        eligible_for_automation:true,coverage:"仅覆盖授权来源".into(),begin_ms:None,end_ms:None,history_baseline_available:false,utc_offset_seconds:8*3600 }
}
fn reference(id: &str,quote: &str) -> Reference { Reference {id:id.into(),quote:quote.into(),version:"current".into()} }

#[test]
fn complete_short_sources_can_be_quoted_without_accepting_fragments_or_completion_claims() {
    let mut r = request();
    for text in ["好", "你好", "收到啦"] {
        r.sources = vec![source("short", text, 1000)];
        refs(&r, &[reference("short", text)], true).unwrap();
    }
    r.sources = vec![source("short", "你好，方案仍待确认。", 1000)];
    assert!(refs(&r, &[reference("short", "你好")], true).is_err());
    r.sources = vec![source("short", "好", 1000)];
    assert!(validate_text(&r, "验收已完成。", &[reference("short", "好")], true).is_err());
    r.sources = vec![source("short", " ", 1000)];
    assert!(refs(&r, &[reference("short", " ")], true).is_err());
}

fn row(text: &str,id: &str,quote: &str) -> SupportedText {SupportedText {text:text.into(),evidence:vec![reference(id,quote)]} }
fn good_draft() -> Draft {
    Draft {schema:1,
        conclusion:row("当前应先落实报名入口和询价，活动尚待场地与费用决定。","confirmed","我10月31日18点前做好手机报名入口，小林10月31日12点前询价；场地还未订，报名尚未开放，费用未定。"),
        key_facts:vec![row("当前预算480元，按此上限准备活动。","notice","预算由600元降到480元")],
        plan:vec![row("建议先写好手机报名入口所需字段和说明。","confirmed","我10月31日18点前做好手机报名入口"),
            row("建议将询价结果整理为可供场地选择的对比表。","confirmed","小林10月31日12点前询价")],
        todos:vec![Todo {action:"做好手机报名入口供用户检查。".into(),kind:"commitment".into(),
            owner:Some(SourcedValue {value:"我".into(),evidence:reference("confirmed","我10月31日18点前做好手机报名入口")}),
            due:Some(SourcedValue {value:"10月31日18点前".into(),evidence:reference("confirmed","我10月31日18点前做好手机报名入口")}),
            evidence:vec![reference("confirmed","我10月31日18点前做好手机报名入口")]}],
        unknowns:vec![row("场地和费用仍需决定，报名开放时间需确认。","confirmed","场地还未订，报名尚未开放，费用未定")],reuse:vec![],next_period:vec![] }
}
#[test]
fn useful_plan_preserves_latest_constraints_and_explicit_commitment() {
    let r=request();let d=good_draft();validate(&r,&d).unwrap();
    let docs=render(&r,&d);assert_eq!(docs.len(),2);assert_eq!(docs[1].0,"todos");
    assert!(docs[0].1.contains("预算由600元降到480元"),"facts display the actual current source wording");assert!(docs[1].1.contains("来源负责人：我；来源期限：10月31日18点前（原话）"));
    assert!(docs[1].1.contains("- [ ]"));assert!(!docs[1].1.contains("- [x]"));
}
#[test]
fn old_budget_and_weekday_cannot_survive_using_valid_old_citations() {
    let r=request();
    assert!(validate_text(&r,"当前预算600元，可按原数准备。",&[reference("old","预算600元")],true).is_err());
    assert!(validate_text(&r,"活动时间确定周六14点。",&[reference("old","活动时间是周六14点")],true).is_err());
    // A current message itself contains both old/new values; old half is not current truth.
    assert!(validate_text(&r,"当前预算600元，可按此执行。",&[reference("notice","预算由600元降到480元")],true).is_err());
    validate_text(&r,"预算由600元调整为480元。",&[reference("notice","预算由600元降到480元")],true).unwrap();
}
#[test]
fn role_swap_and_deadline_cannot_borrow_other_clause_or_activity_time() {
    let r=request(); let mut d=good_draft();
    d.todos[0].owner=Some(SourcedValue {value:"小林".into(),evidence:reference("old","小林负责报名入口")});
    assert!(validate(&r,&d).is_err());
    d=good_draft();d.todos[0].owner=Some(SourcedValue {value:"小林".into(),evidence:reference("confirmed","我10月31日18点前做好手机报名入口，小林10月31日12点前询价")});
    assert!(validate(&r,&d).is_err());
    d=good_draft();d.todos[0].due=Some(SourcedValue {value:"周日15点".into(),evidence:reference("notice","活动时间改为周日15点")});
    assert!(validate(&r,&d).is_err());
}
#[test]
fn notification_first_person_is_not_the_users_commitment() {
    let mut r=request();r.sources[2].user_authored=false;r.sources[2].author.clear();r.sources[2].source="通知".into();
    assert!(validate(&r,&good_draft()).unwrap_err().to_string().contains("第三方"));
}
#[test]
fn invented_completion_ai_facts_unknown_numbers_and_empty_results_are_rejected() {
    let r=request();let mut d=good_draft();d.conclusion=row("手机报名入口已经完成，可立即开放。","confirmed","我10月31日18点前做好手机报名入口");
    assert!(validate(&r,&d).is_err());
    d=good_draft();d.conclusion=row("手机报名入口成功发布，可以收报名了。","confirmed","我10月31日18点前做好手机报名入口");assert!(validate(&r,&d).is_err());
    d=good_draft();d.plan[0].text="建议招募20名参与者并开放报名。".into();assert!(validate(&r,&d).is_err());
    d=good_draft();d.key_facts.clear();assert!(validate(&r,&d).is_err());
    let mut r=request();r.sources.iter_mut().for_each(|s|s.authoritative=false);assert!(validate(&r,&good_draft()).is_err());
}
#[test]
fn actual_completion_report_is_allowed_but_negation_and_condition_are_not() {
    let mut r=request();r.sources.push(source("done","手机报名入口已经完成，尚未发布。",4000));
    validate_text(&r,"手机报名入口已经完成，发布仍需确认。",&[reference("done","手机报名入口已经完成，尚未发布")],true).unwrap();
    r.sources.push(source("conditional","如果手机报名入口已经完成，就可以交给我核对。",5000));
    assert!(validate_text(&r,"手机报名入口已经完成，可以核对。",&[reference("conditional","如果手机报名入口已经完成")],true).is_err());
}

#[test]
fn absence_of_completion_evidence_is_not_an_affirmative_completion_claim() {
    let mut r=request();r.sources=vec![source("check","请核对活动执行与验收的相关证据。",1000)];
    let citations=vec![reference("check","请核对活动执行与验收的相关证据。")];
    for text in ["本次已采集材料中未发现任何已完成验收的独立证据。","本次已采集材料中未见已验收记录。","本次已采集材料中未找到已交付的独立证据。"] {
        validate_text(&r,text,&citations,true).unwrap();
    }
    for text in ["未发现缺陷时验收已完成。","未发现遗漏，但活动已经完成。","缺少材料但验收已完成。"] {
        assert!(validate_text(&r,text,&citations,true).is_err(),"a separate affirmative claim still needs evidence: {text}");
    }
}

#[test]
fn historical_questions_can_produce_an_evidenced_summary_without_new_obligations() {
    let mut r = request();
    let question = "速冻饺子怎样煮熟比较合适？";
    let mut past = source("past-question", question, 1000);
    past.in_period = false;
    r.sources = vec![past];
    let d = Draft { schema: 1,
        conclusion: row("现有资料只有历史烹饪问答，不能据此判断用户还有待完成的任务。", "past-question", question),
        key_facts: vec![row("已有资料中出现过关于速冻饺子煮法的提问。", "past-question", question)],
        plan: vec![], todos: vec![], unknowns: vec![], reuse: vec![], next_period: vec![] };
    for target in [Target::Item("outing".into()), Target::Period(Period::Day), Target::Period(Period::Week), Target::Period(Period::Month)] {
        r.target = target;
        validate(&r, &d).unwrap();
        let documents = render(&r, &d);
        assert!(documents.iter().all(|(_, text)| !text.contains("## 候选待办")));
        if let Some((_,todos))=documents.iter().find(|(kind,_)|kind=="todos") {
            assert!(todos.contains("未形成新的行动候选") && todos.chars().count()<300);
            assert!(!todos.contains("### 来源"),"an empty checklist should not repeat the whole source appendix");
        }
        assert!(documents.iter().all(|(_, text)| !text.contains("- [ ]")));
    }
}
#[test]
fn partial_capture_cannot_become_global_absence_or_complete_message_statistics() {
    let r = request();
    let citations = vec![reference("notice", "活动时间改为周日15点")];
    for text in ["用户本期未向任何会话发送新消息。", "所有用户发出的消息均发生于采集时段开始前。",
        "本期无新互动，也无工作推进。", "本期采集时段内新增的消息均为生活服务提醒。",
        "在本次已采集材料中，出现了一条通知。用户本期未向任何会话发送新消息。"] {
        assert!(validate_text(&r, text, &citations, true).unwrap_err().to_string().contains("有限采集"), "{text}");
    }
    for text in ["在本次已采集材料中，未发现新的工作承诺；本期新增消息均为生活服务通知，本期无新互动。",
        "在本次已采集材料中，未发现新的工作承诺，无法确认未采集的会话内容。",
        "有限通知不能据此判断用户本期无新互动。", "建议将所有App的上下文关联成可审阅的成果。"] {
        validate_text(&r, text, &citations, true).unwrap();
    }
}
#[test]
fn derived_missed_call_requires_checking_before_a_direct_callback() {
    let mut r = request();
    let text = "收到来自未知号码的未接来电，是否已回拨或跟进尚待核对。";
    let mut call = source("missed", text, 4000);
    call.user_authored = false; call.author = "手机记录".into(); call.source = "手机通话记录".into();
    r.sources = vec![call];
    let citations = vec![reference("missed", text)];
    for action in ["回拨号码并跟进对方需求。", "建议尽快回拨未接来电。", "当前未接来电仍待回拨处理。", "当前尚未回拨，应立即处理。", "不要忽略这个号码，建议回拨核对需求。",
        "未见用户本人明确的回拨意图，但建议立即回拨。", "未见用户本人明确的意图但建议回拨。",
        "不能证明当前需要回拨但应立即回拨。", "这些号码不足以说明现在需要回拨，建议尽快致电核实。"] {
        let todo = Todo { action: action.into(), kind: "suggested".into(), owner: None, due: None, evidence: citations.clone() };
        assert!(validate_todo(&r, &todo).unwrap_err().to_string().contains("系统未接记录"));
    }
    for action in ["先核对是否已回拨，再确认是否需要跟进。", "核对未接来电的回拨状态，确认此前是否已处理。", "先核对记录是否已经处理，未确认前不安排回拨。", "该记录只作为待核对信息，不要回拨。",
        "未见用户本人明确的回拨意图或人工往来佐证。", "尚未发现明确的回电需求。",
        "因此这些号码不足以说明现在需要回拨或跟进。", "系统未接记录不能证明当前需要回拨。"] {
        validate_text(&r, action, &citations, false).unwrap();
    }
    r.sources.push(source("intent", "请帮我回拨该未接来电，确认对方联系原因。", 5000));
    let mut supported = citations;
    supported.push(reference("intent", "请帮我回拨该未接来电，确认对方联系原因。"));
    validate_text(&r, "建议回拨未接来电并核对联系原因。", &supported, false).unwrap();
}
#[test]
fn current_explicit_user_commitment_requires_a_supported_action_but_history_does_not() {
    let mut r = request();
    r.sources[2].kind = Kind::Commitment.label().into();
    let mut d = good_draft();
    validate(&r, &d).unwrap();
    d.todos.clear();
    assert!(validate(&r, &d).unwrap_err().to_string().contains("明确承诺"));
    d.todos = vec![Todo { action: "建议准备与手机报名入口无关的询价清单。".into(), kind: "suggested".into(), owner: None, due: None,
        evidence: vec![reference("notice", "小林负责询价")] }];
    assert!(validate(&r, &d).unwrap_err().to_string().contains("明确承诺"));
    d.todos.clear();
    r.sources[2].in_period = false;
    validate(&r, &d).unwrap();
    r.sources[2].in_period = true;
    r.sources[2].user_authored = false;
    validate(&r, &d).unwrap();
}
#[test]
fn another_matter_baseline_does_not_authorize_trend_claims() {
    let mut r=request();r.target=Target::Period(Period::Month);r.history_baseline_available=true;
    let mut other=source("other","另一项目效率明显提升。",1);other.matter_id="another-project".into();other.baseline=Some("另一项目刚启动。".into());r.sources.push(other);
    assert!(validate_text(&r,"报名工作效率明显提升。",&[reference("confirmed","我10月31日18点前做好手机报名入口"),Reference {id:"other".into(),quote:"另一项目刚启动。".into(),version:"baseline".into()}],true).is_err());
}
#[test]
fn changed_scope_source_and_fabricated_quote_do_not_validate() {
    let r=request();let mut d=good_draft();d.conclusion.evidence[0].quote="手机报名已完成且费用确定。".into();assert!(validate(&r,&d).is_err());
    d=good_draft();d.conclusion.evidence[0].id="another-account-evidence".into();assert!(validate(&r,&d).is_err());
}

#[test]
fn narrative_responsibility_cannot_hide_a_role_swap_outside_todo_fields() {
    let r=request();
    assert!(validate_text(&r,"小林负责报名入口，需继续处理。",&[reference("old","小林负责报名入口")],true).is_err());
    assert!(validate_text(&r,"小林负责报名入口，需继续处理。",&[reference("confirmed","我10月31日18点前做好手机报名入口，小林10月31日12点前询价")],true).is_err());
    validate_text(&r,"我负责报名入口，按已约定任务准备。",&[reference("confirmed","我10月31日18点前做好手机报名入口")],true).unwrap();
}

#[test]
fn indexed_references_expand_exact_source_without_rewriting_model_text() {
    let r=request();let mut raw=serde_json::to_value(good_draft()).unwrap();
    let indexed=serde_json::json!({"id":"confirmed","excerpt":0,"version":"current"});
    raw["todos"][0]["evidence"][0]=indexed.clone();raw["todos"][0]["owner"]["evidence"]=indexed.clone();raw["todos"][0]["due"]["evidence"]=indexed;
    let original=raw.to_string();let draft=parse_and_validate(&r,&original).unwrap();
    assert_eq!(draft.todos[0].evidence[0].quote,source_excerpts(&r.sources[2].text)[0]);
    assert!(original.contains("\"excerpt\":0"));assert_eq!(raw.to_string(),original);
    let projection=source_projection(&r.sources[2]).unwrap();assert_eq!(projection["excerpts"][0]["index"],0);assert!(projection.get("text").is_none());
}
#[test]
fn excerpt_index_never_authorizes_unknown_source_or_mismatched_quote() {
    let r=request();
    for reference in [serde_json::json!({"id":"other-account","excerpt":0}),serde_json::json!({"id":"confirmed","excerpt":999}),
        serde_json::json!({"id":"confirmed","excerpt":-1}),serde_json::json!({"id":"confirmed","excerpt":0,"quote":"被模型删改的引用"}),
        serde_json::json!({"id":"confirmed","excerpt":0,"version":"baseline"})] {
        let mut draft=serde_json::to_value(good_draft()).unwrap();draft["todos"][0]["evidence"][0]=reference;
        assert!(parse_and_validate(&r,&draft.to_string()).is_err());
    }
}
#[test]
fn one_repair_reports_structure_completion_quote_number_and_due_errors_together() {
    let r=request();let mut draft=serde_json::to_value(good_draft()).unwrap();
    draft["todos"][0]["owner"]=serde_json::json!("我");
    draft["todos"][0]["due"]["value"]=serde_json::json!("明天");
    draft["conclusion"]["text"]=serde_json::json!("手机报名入口已经完成，可立即使用。");
    draft["plan"][0]["text"]=serde_json::json!("建议准备至少99份报名材料并确认页面。");
    draft["key_facts"][0]["evidence"][0]["quote"]=serde_json::json!("预算经过删节后改到480元");
    let error=parse_and_validate(&r,&draft.to_string()).unwrap_err().to_string();
    for field in ["conclusion","key_facts[0]","plan[0]","todos[0].owner","due"] {assert!(error.contains(field),"missing {field}: {error}");}
    assert!(error.contains("已完成"));assert!(error.contains("99"));assert!(error.contains("连续原文"));
}
#[test]
fn todo_action_error_does_not_hide_invalid_owner_and_due() {
    let r=request();let mut todo=good_draft().todos.remove(0);todo.action="制作99个报名入口页面并提交检查。".into();
    todo.owner=Some(SourcedValue {value:"小林".into(),evidence:reference("old","小林负责报名入口")});
    todo.due=Some(SourcedValue {value:"周日15点".into(),evidence:reference("notice","活动时间改为周日15点")});
    let error=validate_todo(&r,&todo).unwrap_err().to_string();
    for field in ["action/evidence","owner","due"] {assert!(error.contains(field),"{error}");}
}

#[test]
fn temporal_and_commitment_words_do_not_become_part_of_a_persons_name() {
    let mut r=request();
    r.sources[2].text="我会在2026年10月31日18:00前做好手机报名入口，小林会在2026年10月31日12:00前完成场地询价。".into();
    let citations=vec![reference("confirmed","我会在2026年10月31日18:00前做好手机报名入口"),reference("confirmed","小林会在2026年10月31日12:00前完成场地询价")];
    for text in ["我承诺2026年10月31日18:00前完成手机报名入口搭建，小林承诺2026年10月31日12:00前完成场地询价", "用户承诺2026年10月31日18:00前完成手机报名入口搭建，小林在2026年10月31日12:00前完成场地询价"] {
        validate_text(&r,text,&citations,true).unwrap();
    }
    assert!(validate_text(&r,"小王承诺2026年10月31日12:00前完成场地询价",&citations,true).is_err());
    r.sources[2].user_authored=false;
    assert!(validate_text(&r,"用户承诺2026年10月31日18:00前完成手机报名入口",&citations,true).is_err());
}
#[test]
fn deadline_quote_does_not_need_to_repeat_a_separately_grounded_action_budget() {
    let r=request();let quote=reference("confirmed","小林10月31日12点前询价");
    let mut todo=Todo {action:"完成场地询价并反馈符合480元预算的报价方案。".into(),kind:"commitment".into(),
        owner:Some(SourcedValue {value:"小林".into(),evidence:quote.clone()}),due:Some(SourcedValue {value:"10月31日12点前".into(),evidence:quote.clone()}),
        evidence:vec![quote,reference("notice","预算由600元降到480元")]};
    validate_todo(&r,&todo).unwrap();
    todo.evidence.pop();let error=validate_todo(&r,&todo).unwrap_err().to_string();assert!(error.contains("action/evidence"));assert!(error.contains("480"));
}

#[test]
fn real_commitment_sentence_uses_inquiry_as_one_compound_activity() {
    let mut r=request();r.sources[2].text="我承诺2026年10月31日18:00前做好手机报名入口，小林承诺2026年10月31日12:00前询价。".into();
    let citations=vec![reference("confirmed","我承诺2026年10月31日18:00前做好手机报名入口"),reference("confirmed","小林承诺2026年10月31日12:00前询价")];
    validate_text(&r,"我承诺2026年10月31日18:00前完成手机报名入口搭建，小林承诺2026年10月31日12:00前完成场地询价",&citations,true).unwrap();
    assert!(validate_text(&r,"小林负责场地和报名的全部准备工作。",&citations,true).is_err());
}

#[test]
fn sourced_actor_matching_accepts_combined_grammar_but_not_new_names() {
    let mut r=request();r.sources[2].text="我承诺2026年10月31日18:00前做好手机报名入口，小林承诺2026年10月31日12:00前询价。".into();
    let refs=vec![reference("confirmed","我承诺2026年10月31日18:00前做好手机报名入口"),reference("confirmed","小林承诺2026年10月31日12:00前询价"),reference("notice","小林负责询价")];
    for text in ["小林需按承诺在2026年10月31日12:00前完成场地询价。", "我需按承诺在2026年10月31日18:00前做好手机报名入口。",
        "场地询价由小林负责，需在2026年10月31日12:00前完成询价。", "优先跟进小林负责询价，整理报价依据。", "小林需要在2026年10月31日12:00前完成询价。", "小林应在2026年10月31日12:00前完成询价。"] {
        validate_text(&r,text,&refs,false).unwrap_or_else(|error|panic!("{text}: {error:#}"));
    }
    for text in ["小王需按承诺在2026年10月31日12:00前完成询价。", "张小林在2026年10月31日12:00前完成询价。", "小林俊在2026年10月31日12:00前完成询价。", "小林需按承诺在2026年10月31日18:00前做好手机报名入口。"] {
        assert!(validate_text(&r,text,&refs,false).is_err(),"accepted ungrounded actor: {text}");
    }
}

#[test]
fn short_citations_are_stable_deduplicated_and_traceable_in_both_documents() {
    let mut r=request();let mut d=good_draft();r.sources[2].baseline=Some("报名入口尚待开始。".into());
    d.conclusion.evidence.push(reference("confirmed","小林10月31日12点前询价"));
    d.conclusion.evidence.push(d.conclusion.evidence[0].clone());
    let labels=citation_labels(&r,&d);
    assert_eq!(labels.get("notice"),Some(&1));assert_eq!(labels.get("confirmed"),Some(&2));
    assert_eq!(citations(&labels,&d.conclusion.evidence),"〔来源2〕");
    let baseline=Reference {id:"confirmed".into(),quote:"报名入口尚待开始。".into(),version:"baseline".into()};
    assert_eq!(citations(&labels,&[d.conclusion.evidence[0].clone(),baseline.clone()]),"〔来源2〕 〔来源2·期初〕");
    let docs=render(&r,&d);
    for (_,md) in &docs {
        assert_eq!(md.matches("### 来源2\n").count(),1);
        assert!(!md.contains("原始编号：〔E:confirmed〕"));
        assert_eq!(md.matches("消息 confirmed").count(),1);
        assert!(md.contains("我10月31日18点前做好手机报名入口"),"exact original evidence stays available for review");
        assert!(md.contains("〔来源2〕"));assert!(md.contains("1970-01-01"));
    }
    d.reuse.push(SupportedText {text:"保留期初状态用于后续核对。".into(),evidence:vec![baseline]});
    let appendix=appendix(&r,&d,&labels);
    assert!(appendix.contains("### 来源2 · 期初保存记录"));assert!(appendix.contains("报名入口尚待开始。"));
    assert_eq!(appendix.matches("小林10月31日12点前询价").count(),2); // one full quote plus one short excerpt, no repeated metadata
}

#[test]
fn old_bundle_defaults_to_original_render_version() {
    let bundle:OutcomeBundle=serde_json::from_value(serde_json::json!({"scope_id":"scope","target":{"item":"outing"},"input_revision":"r","title":"活动","conclusion":"待核对","files":[],"manifest_path":"manifest.json","todo_count":0,"created_ms":1})).unwrap();
    assert_eq!(bundle.render_version,0);assert!(RENDER_VERSION > bundle.render_version);
}
