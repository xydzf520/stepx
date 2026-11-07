//! A machine-authored request to organize one selected file is local intent.
//! It must not become a user's instructions for every matter in a period.
use super::{Draft, GenerationRequest, SourceEvidence, Target};
use anyhow::{ensure, Result};
use std::collections::BTreeSet;

fn material_request(source: &SourceEvidence) -> bool {
    let Some(locator) = source.locator.strip_prefix("资料收件箱 ") else {
        return false;
    };
    let Some(id) = locator.strip_suffix(" / 显式整理按钮") else {
        return false;
    };
    let Some((intent, revision)) = source.text.rsplit_once("\n资料正文版本：") else {
        return false;
    };
    source.source == "用户明确发起的资料整理请求"
        && source.user_authored
        && source.authoritative
        && source.author == "我"
        && id.len() == 64
        && id.bytes().all(|b| b.is_ascii_hexdigit())
        && revision.len() == 32
        && revision.bytes().all(|b| b.is_ascii_hexdigit())
        && intent.starts_with("用户明确请求根据资料《")
        && intent.contains("》生成可编辑方案和候选待办。")
        && intent.contains(
            "仅授权资料整理，不确认识别准确性，不采纳待办，不更改原安排，不自动执行动作。",
        )
}

fn restriction_bits(text: &str) -> u8 {
    let mut bits = 0;
    if text.contains("整理")
        && ["仅授权", "只授权", "仅允许", "只允许", "仅做", "只做"]
            .iter()
            .any(|word| text.contains(word))
    {
        bits |= 1;
    }
    if text.contains("不采纳") {
        bits |= 2;
    }
    if ["不更改", "不修改", "不改原", "不改现有"]
        .iter()
        .any(|word| text.contains(word))
    {
        bits |= 4;
    }
    if ["不自动执行", "不执行任何"]
        .iter()
        .any(|word| text.contains(word))
    {
        bits |= 8;
    }
    if text.contains("不确认") && text.contains("准确") {
        bits |= 16;
    }
    bits
}

fn local_subject(text: &str) -> bool {
    [
        "资料《",
        "图片《",
        "文件《",
        "该图片",
        "这张图片",
        "该图",
        "该截图",
        "这份资料",
        "该资料",
        "该文件",
        "该录音",
        "本事项",
        "该事项",
        "这件事",
        "这份材料",
        "该材料",
    ]
    .iter()
    .any(|word| text.contains(word))
}

fn strong_global(text: &str) -> bool {
    [
        "任何待办",
        "所有待办",
        "全部待办",
        "所有事项",
        "全部事项",
        "所有事情",
        "全部事情",
        "全部资料",
        "所有资料",
        "整个周期",
        "整周",
        "整月",
    ]
    .iter()
    .any(|word| text.contains(word))
}

fn global_frame(text: &str) -> bool {
    strong_global(text)
        || (!local_subject(text)
            && [
                "本期",
                "本周",
                "本月",
                "这周",
                "这个月",
                "现有资料",
                "用户本次",
                "本次仅",
                "本次只",
            ]
            .iter()
            .any(|word| text.contains(word)))
}

fn progress_topics(text: &str) -> u8 {
    ["成果", "承诺", "执行", "完成", "进展", "交付"]
        .iter()
        .enumerate()
        .fold(0, |bits, (index, word)| {
            bits | if text.contains(word) { 1 << index } else { 0 }
        })
}

fn global_absence(text: &str) -> bool {
    [
        "所有事项",
        "全部事项",
        "全部事情",
        "所有事情",
        "全部工作",
        "所有工作",
        "所有任务",
        "全部任务",
        "所有项目",
        "全部项目",
    ]
    .iter()
    .any(|word| text.contains(word))
        && [
            "均未", "都未", "没有", "尚未", "尚无", "均无", "都无", "并无", "全无",
        ]
        .iter()
        .any(|word| text.contains(word))
        && progress_topics(text) != 0
}

fn limited_observation(text: &str) -> bool {
    // A negated inference is safe only when it precedes the universal claim.
    // A trailing "cannot judge" must not excuse "all tasks were not executed".
    let universal_at = [
        "所有事项",
        "全部事项",
        "全部事情",
        "所有事情",
        "全部工作",
        "所有工作",
        "所有任务",
        "全部任务",
        "所有项目",
        "全部项目",
    ]
    .iter()
    .filter_map(|word| text.find(word))
    .min()
    .unwrap_or(text.len());
    if [
        "不能判断",
        "不能推断",
        "不代表",
        "不足以证明",
        "不能证明",
        "无法确认",
    ]
    .iter()
    .any(|word| text[..universal_at].contains(word))
    {
        return true;
    }
    let limited = [
        "本次已采集材料",
        "本次采集材料",
        "本次材料",
        "已采集的材料",
        "已采集资料",
        "所提供来源",
        "提供的来源",
        "现有记录中",
        "本次已保存",
        "本次已提供",
    ]
    .iter()
    .any(|word| text.contains(word));
    let evidence_absent = [
        "未提供",
        "未见",
        "未找到",
        "没有提供",
        "未发现",
        "尚无独立",
        "没有独立",
    ]
    .iter()
    .any(|word| text.contains(word));
    let reality_absent = [
        "现实中",
        "实际均",
        "实际都",
        "均未形成",
        "都未形成",
        "均未执行",
        "都未执行",
        "均未完成",
        "都未完成",
        "均无成果",
        "都无成果",
    ]
    .iter()
    .any(|word| text.contains(word));
    // Mentioning a cache cannot justify an explicit assertion about all of
    // reality. A bounded statement about the evidence itself remains useful.
    limited && evidence_absent && !reality_absent
}

pub(super) fn validate_period_coverage(request: &GenerationRequest, draft: &Draft) -> Result<()> {
    if !matches!(request.target, Target::Period(_)) {
        return Ok(());
    }
    for clause in draft.conclusion.text.split(['。', '；', ';', '\n']) {
        if !global_absence(clause) || limited_observation(clause) {
            continue;
        }
        let supported = draft.conclusion.evidence.iter().any(|reference| {
            request.sources.iter().any(|source| {
                source.id == reference.id
                    && reference.version == "current"
                    && source.authoritative
                    && !material_request(source)
                    && source.text.contains(&reference.quote)
                    && global_absence(&reference.quote)
                    && progress_topics(clause) & !progress_topics(&reference.quote) == 0
            })
        });
        ensure!(supported,
            "有限采集材料不能证明所有事项现实均无成果、承诺或执行。请写‘本次已采集材料未提供独立执行/验收证据，实际进展待核对’，不要写所有事项均未形成成果；若来源确有全局状态报告，须引用对应原话。");
    }
    Ok(())
}

pub(super) fn validate_period_scope(request: &GenerationRequest, draft: &Draft) -> Result<()> {
    if !matches!(request.target, Target::Period(_)) {
        return Ok(());
    }
    validate_period_coverage(request, draft)?;
    let matters: BTreeSet<_> = request
        .sources
        .iter()
        .map(|source| source.matter_id.as_str())
        .collect();
    for (index, row) in std::iter::once(&draft.conclusion)
        .chain(&draft.key_facts)
        .enumerate()
    {
        let cited_requests: Vec<_> = row
            .evidence
            .iter()
            .filter_map(|reference| {
                request.sources.iter().find(|source| {
                    source.id == reference.id
                        && reference.version == "current"
                        && material_request(source)
                })
            })
            .collect();
        if cited_requests.is_empty() {
            continue;
        }
        // Only an independently quoted, actual user instruction can establish a
        // global restriction. Never infer one from a file, assistant or organizer.
        let supported_global = row
            .evidence
            .iter()
            .filter_map(|reference| {
                let source = request
                    .sources
                    .iter()
                    .find(|source| source.id == reference.id)?;
                (source.user_authored
                    && source.authoritative
                    && !material_request(source)
                    && reference.version == "current"
                    && source.text.contains(&reference.quote)
                    && global_frame(&reference.quote))
                .then(|| restriction_bits(&reference.quote))
            })
            .fold(0u8, |bits, next| bits | next);
        for clause in row.text.split(['。', '；', ';', '\n']) {
            let restricted = restriction_bits(clause) & !supported_global;
            let implicit_global = matters.len() > 1
                && !local_subject(clause)
                && clause.trim_start().starts_with("用户")
                && restriction_bits(clause) != 0;
            ensure!(restricted == 0 || !(global_frame(clause) || implicit_global),
            "{}：单份资料整理授权不能扩展成整期所有事项或待办的限制。该机器整理请求仅适用于 matter_id={}；请把限制明确写为‘对该资料/该事项’，保留其他事项各自的用户要求，不把它作为本周/本月的全局指示。真实全局指示必须引用独立用户原话。",
            if index==0 {"conclusion".into()} else {format!("key_facts[{}]",index-1)},cited_requests[0].matter_id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_ledger::{
        outcomes::{Reference, SupportedText},
        Period,
    };

    fn source(id: &str, matter: &str, text: &str) -> SourceEvidence {
        SourceEvidence {
            id: id.into(),
            matter_id: matter.into(),
            title: "图片资料".into(),
            text: text.into(),
            author: "我".into(),
            source: "手机 Amoo 讨论".into(),
            locator: "会话 original / 消息 1".into(),
            occurred_ms: None,
            observed_ms: 1,
            changed_ms: 1,
            kind: "讨论材料".into(),
            authoritative: true,
            user_authored: true,
            in_period: true,
            baseline: None,
        }
    }
    fn fixture(text: &str) -> (GenerationRequest, Draft) {
        let mut material = source("material-intent", "picture-matter", &format!(
            "用户明确请求根据资料《图片.png》生成可编辑方案和候选待办。仅授权资料整理，不确认识别准确性，不采纳待办，不更改原安排，不自动执行动作。\n资料正文版本：{}", "a".repeat(32)));
        material.source = "用户明确发起的资料整理请求".into();
        material.locator = format!("资料收件箱 {} / 显式整理按钮", "b".repeat(64));
        let reference = Reference {
            id: material.id.clone(),
            quote: material.text.clone(),
            version: "current".into(),
        };
        let request = GenerationRequest {
            scope_id: "scope".into(),
            target: Target::Period(Period::Week),
            input_revision: "revision".into(),
            title: "周复盘".into(),
            prompt: String::new(),
            sources: vec![
                material,
                source("work", "work-matter", "请继续代码评估，并进行优化和改造。"),
            ],
            eligible_for_automation: true,
            coverage: String::new(),
            begin_ms: Some(1),
            end_ms: Some(2),
            history_baseline_available: false,
            utc_offset_seconds: 0,
        };
        let draft = Draft {
            schema: 1,
            conclusion: SupportedText {
                text: text.into(),
                evidence: vec![reference],
            },
            key_facts: vec![],
            plan: vec![],
            todos: vec![],
            unknowns: vec![],
            reuse: vec![],
            next_period: vec![],
        };
        (request, draft)
    }
    #[test]
    fn rejects_exact_real_period_generalization_and_explicit_global_variants() {
        for text in [
            "用户本次仅授权对现有资料进行整理，不确认资料准确性，不采纳任何待办建议，不更改原有安排，不自动执行任何动作。",
            "本周只授权资料整理，所有事项不自动执行。",
            "用户仅授权资料整理，不采纳待办。",
            "对该图片只授权整理，本月所有事项不改原安排。",
        ] { let (r,d)=fixture(text);assert!(validate_period_scope(&r,&d).is_err(),"{text}"); }
    }
    #[test]
    fn preserves_local_focus_and_one_matter_as_the_main_period_judgment() {
        for text in [
            "本周对该图片仅授权整理，不确认准确性，不采纳待办；代码评估仍按原要求推进。",
            "对资料《图片.png》不更改原安排；另一个任务需要代码评估。",
            "本周最值得先处理的是该图片资料，先核对识别内容，再决定是否使用。",
        ] {
            let (r, d) = fixture(text);
            assert!(validate_period_scope(&r, &d).is_ok(), "{text}");
        }
        let (mut r, d) = fixture("用户仅授权资料整理，不采纳待办。");
        r.sources.truncate(1);
        assert!(validate_period_scope(&r, &d).is_ok());
        r.target = Target::Item("picture-matter".into());
        assert!(validate_period_scope(&r, &d).is_ok());
    }
    #[test]
    fn respects_an_independently_quoted_global_user_instruction_but_not_an_ai_claim() {
        let (mut r, mut d) = fixture("本周仅做资料整理，不采纳任何待办，不更改安排。");
        let instruction = "本周仅做资料整理，不采纳任何待办，不更改安排。";
        r.sources
            .push(source("global", "global-instruction", instruction));
        d.conclusion.evidence.push(Reference {
            id: "global".into(),
            quote: instruction.into(),
            version: "current".into(),
        });
        assert!(validate_period_scope(&r, &d).is_ok());
        r.sources.last_mut().unwrap().user_authored = false;
        r.sources.last_mut().unwrap().authoritative = false;
        assert!(validate_period_scope(&r, &d).is_err());
    }
    #[test]
    fn similar_user_words_without_machine_receipt_are_not_reclassified_as_material_authorization() {
        let (mut r, d) = fixture("本周仅授权资料整理，不采纳任何待办。");
        r.sources[0].source = "手机 Amoo 讨论".into();
        assert!(validate_period_scope(&r, &d).is_ok());
    }
    #[test]
    fn rejects_single_material_authorization_moved_into_period_key_facts() {
        let (r, mut d) = fixture("本周有两类事项，需要分别跟进。");
        let fact = SupportedText {
            text: "用户本次仅授权资料整理，不采纳任何待办，不自动执行动作。".into(),
            evidence: d.conclusion.evidence.clone(),
        };
        d.key_facts.push(fact);
        let error = validate_period_scope(&r, &d).unwrap_err().to_string();
        assert!(error.contains("key_facts[0]"));
        d.key_facts[0].text = "对该图片，用户仅授权资料整理，不采纳待办。".into();
        assert!(validate_period_scope(&r, &d).is_ok());
        // Plans may propose a local operating boundary; this guard only checks
        // the period judgment and asserted facts, not every candidate step.
        d.plan.push(d.key_facts.pop().unwrap());
        assert!(validate_period_scope(&r, &d).is_ok());
    }
    #[test]
    fn rejects_universal_real_world_absence_from_limited_evidence() {
        for text in [
            "所有事项均未形成已执行的成果或承诺。",
            "本次已采集材料未提供独立证据，所有事项均未完成。",
            "所有工作均未执行，不能判断是否完成。",
            "全部项目尚无成果。",
        ] {
            let (r, d) = fixture(text);
            assert!(validate_period_coverage(&r, &d).is_err(), "{text}");
        }
    }
    #[test]
    fn allows_bounded_evidence_gaps_without_asserting_real_world_absence() {
        for text in [
            "本次已采集材料未提供独立执行或验收证据，实际进展待核对。",
            "所提供来源未提供所有事项的独立执行证据。",
            "在现有记录中，所有事项尚未提供独立验收证据。",
            "不能推断所有事项均未完成，需要与用户核对实际进展。",
            "本次材料不足以证明所有事项均无成果。",
        ] {
            let (r, d) = fixture(text);
            assert!(validate_period_coverage(&r, &d).is_ok(), "{text}");
        }
    }
    #[test]
    fn respects_quoted_authoritative_global_status_but_not_topic_expansion_or_ai() {
        let (mut r, mut d) = fixture("所有事项均未完成。");
        let status = "所有事项均未完成。";
        r.sources
            .push(source("global-status", "status-matter", status));
        d.conclusion.evidence.push(Reference {
            id: "global-status".into(),
            quote: status.into(),
            version: "current".into(),
        });
        assert!(validate_period_coverage(&r, &d).is_ok());
        d.conclusion.text = "所有事项均未形成成果或承诺。".into();
        assert!(validate_period_coverage(&r, &d).is_err());
        d.conclusion.text = status.into();
        r.sources.last_mut().unwrap().authoritative = false;
        assert!(validate_period_coverage(&r, &d).is_err());
        r.sources.last_mut().unwrap().authoritative = true;
        d.conclusion.evidence.last_mut().unwrap().version = "baseline".into();
        assert!(validate_period_coverage(&r, &d).is_err());
    }
}
