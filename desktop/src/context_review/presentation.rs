//! Small, source-backed Home cards. No I/O, model calls or inferred completion.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatterAction {
    #[default]
    ReviewSources,
    ContinueDiscussion,
    ReviewOutput,
    PrepareSchedule,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MatterPresentation {
    pub headline: String,
    pub what_happened: String,
    pub next_step: String,
    /// A filename recorded by a task, not proof the file still exists or is final.
    pub available_output: Option<String>,
    pub primary_action: MatterAction,
    /// Zero means background only; the Home recommendation list should omit it.
    pub priority: u8,
    /// Recheck at render time so a window left open does not promote an expired
    /// appointment or an old request. None is used for durable file records.
    pub valid_until_ms: Option<i64>,
    pub evidence_ids: Vec<String>,
    /// Attribution and limitations belong in the expanded details.
    pub basis_note: String,
}

fn plain_fact(fact: &ReviewFact, max: usize) -> String {
    let text = if matches!(fact.kind, FactKind::Artifact | FactKind::Receipt | FactKind::Calendar) {
        fact.text.lines().next().unwrap_or(&fact.text)
    } else { &fact.text };
    clip(text, max)
}

fn time_label(time: i64, offset: i32) -> String {
    FixedOffset::east_opt(offset).and_then(|offset| offset.timestamp_millis_opt(time).single())
        .map(|time| time.format("%m-%d %H:%M").to_string()).unwrap_or_else(|| "时间待核对".into())
}

/// Present only what a matter's actual records support. A source question is not
/// automatically an obligation; old/unknown discussion and expired schedules are
/// background. File records are useful to revisit even if their creation time is
/// unknown, but opening them remains the user's explicit action.
pub fn present_matter(matter: &ReviewMatter, now_ms: i64, utc_offset_seconds: i32) -> MatterPresentation {
    let mut facts: Vec<_> = matter.facts.iter().filter(|fact| fact.current && human(fact)).collect();
    // A late import is not a new arrangement. Only an actual stored text
    // revision may outrank the source's occurrence time.
    facts.sort_by_key(|fact| std::cmp::Reverse(fact.occurred_ms.unwrap_or(fact.observed_ms)
        .max(if fact.previous.is_empty() { 0 } else { fact.changed_ms })));
    let recent: Vec<_> = facts.iter().copied().filter(|fact| {
        fact.kind != FactKind::Calendar && fact.occurred_ms.is_some_and(|at| at <= now_ms && at >= now_ms.saturating_sub(7 * 86_400_000))
    }).collect();
    let output = facts.iter().copied().find(|fact| fact.kind == FactKind::Artifact && fact.observed_ms <= now_ms);
    let mut result = MatterPresentation {
        headline: clip(&matter.title, 44),
        available_output: output.map(|fact| plain_fact(fact, 60)),
        ..Default::default()
    };
    let selected = if let Some(fact) = facts.iter().copied().find(|fact| fact.kind == FactKind::Receipt
        && fact.text.contains("未成功") && fact.observed_ms <= now_ms && fact.observed_ms >= now_ms.saturating_sub(7 * 86_400_000)) {
        result.priority = 100;
        result.what_happened = plain_fact(fact, 100);
        result.next_step = "查看上次发送结果，确认文件和接收手机后再重试。".into();
        result.primary_action = MatterAction::ReviewOutput;
        Some(fact)
    } else if let Some(fact) = facts.iter().copied().find(|fact| fact.kind != FactKind::Calendar
        && fact.occurred_ms.is_some_and(|time| time <= now_ms)
        && !fact.previous.is_empty() && fact.changed_ms <= now_ms && fact.changed_ms >= now_ms.saturating_sub(7 * 86_400_000)) {
        result.priority = 95;
        result.what_happened = format!("记录更新为：{}", plain_fact(fact, 95));
        result.next_step = "打开修改前后的原文，确认现有稿件是否需要调整。".into();
        result.primary_action = MatterAction::ReviewSources;
        Some(fact)
    } else if let Some(fact) = recent.iter().copied().find(|fact| blocker(&fact.text).is_some() && open_question(&recent, fact)) {
        let gap = clip(blocker(&fact.text).unwrap_or(&fact.text), 72);
        result.priority = 90;
        result.what_happened = gap.clone();
        result.next_step = format!("先核对“{gap}”是否已有答复，再继续原讨论。");
        result.primary_action = MatterAction::ContinueDiscussion;
        Some(fact)
    } else if let Some(fact) = output {
        result.priority = 85;
        result.what_happened = format!("任务记录了《{}》。", plain_fact(fact, 60));
        result.next_step = "打开文件检查内容，再决定是否发送到手机。".into();
        result.primary_action = MatterAction::ReviewOutput;
        result.basis_note = "目前仅核对了任务中的文件记录；未检查文件是否仍存在、内容是否定稿，也不代表已发送或验收。".into();
        Some(fact)
    } else if let Some(fact) = recent.iter().copied().find(|fact| directed_request(fact) && open_question(&recent, fact)) {
        result.priority = 80;
        result.what_happened = format!("原讨论中的请求：{}", plain_fact(fact, 100));
        result.next_step = "打开原讨论，补充能确认的信息；不确定的部分保留为问题。".into();
        result.primary_action = MatterAction::ContinueDiscussion;
        Some(fact)
    } else if let Some(fact) = recent.iter().copied().find(|fact| matches!(fact.kind, FactKind::Change | FactKind::Decision | FactKind::Commitment)) {
        result.priority = 65;
        result.what_happened = format!("讨论中提到：{}", plain_fact(fact, 100));
        result.next_step = "回到这段讨论确认安排，再补充相关草稿。".into();
        result.primary_action = MatterAction::ContinueDiscussion;
        Some(fact)
    } else if let Some(fact) = recent.iter().copied().find(|fact| fact.author == "我"
        && ["帮我", "请", "我想", "我要", "我们计划"].iter().any(|word| fact.text.contains(word))) {
        result.priority = 50;
        result.what_happened = format!("你讨论过：{}", plain_fact(fact, 100));
        result.next_step = "继续这段讨论，把要交付的内容写清楚。".into();
        result.primary_action = MatterAction::ContinueDiscussion;
        Some(fact)
    } else if let Some(fact) = facts.iter().copied().filter(|fact| fact.kind == FactKind::Calendar
        && fact.occurred_ms.is_some_and(|time| time >= now_ms && time < now_ms.saturating_add(7 * 86_400_000)))
        .min_by_key(|fact| fact.occurred_ms) {
        result.priority = 30;
        result.what_happened = format!("{} · {}", time_label(fact.occurred_ms.unwrap(), utc_offset_seconds), plain_fact(fact, 75));
        result.next_step = "打开日程核对时间和地点，准备需要带的材料。".into();
        result.primary_action = MatterAction::PrepareSchedule;
        result.basis_note = "来自当前读到的日历计划，不代表已参加；再次打开时需核对安排是否仍有效。".into();
        Some(fact)
    } else {
        result.what_happened = "已保留讨论资料，尚未找到可以直接推荐的当前行动。".into();
        result.next_step = "打开原文，确认是否还需要继续。".into();
        result.basis_note = "历史、未知时间和普通问句仅作为背景，不自动成为当前待办。".into();
        None
    };
    if let Some(fact) = selected {
        result.evidence_ids.push(fact.id.clone());
        result.valid_until_ms = if fact.kind == FactKind::Calendar { fact.occurred_ms }
            else if fact.kind == FactKind::Artifact { None }
            else {
                Some(if !fact.previous.is_empty() { fact.changed_ms }
                    else { fact.occurred_ms.unwrap_or(fact.observed_ms) }.saturating_add(7 * 86_400_000))
            };
        if result.basis_note.is_empty() {
            result.basis_note = format!("来源：{}；{}。这是来源中的表述，建议尚未执行。", fact.source,
                fact.occurred_ms.map(|time| time_label(time, utc_offset_seconds)).unwrap_or_else(|| "发生时间未知".into()));
        }
    }
    if let Some(output) = output {
        if !result.evidence_ids.contains(&output.id) { result.evidence_ids.push(output.id.clone()); }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::{fact, input, matter};

    #[test]
    fn importing_an_older_arrangement_later_does_not_replace_the_current_card() {
        let input = input(Period::Day);
        let mut earlier = fact(1, FactKind::Decision, "预算最初定为600元。", "2026-10-29T10:00:00+08:00");
        earlier.observed_ms = input.now_ms;
        earlier.changed_ms = input.now_ms;
        let latest = fact(2, FactKind::Change, "确认预算调整为480元。", "2026-10-29T11:00:00+08:00");
        let card = present_matter(&matter(vec![earlier, latest]), input.now_ms, input.utc_offset_seconds);
        assert!(card.what_happened.contains("480"));
        assert!(!card.what_happened.contains("600"));
    }

    #[test]
    fn explicit_work_request_beats_calendar_and_never_claims_a_draft_exists() {
        let input = input(Period::Day);
        let request = matter(vec![fact(1, FactKind::Question, "请提供活动方案的预算明细。", "2026-10-29T10:00:00+08:00")]);
        let scheduled = matter(vec![fact(2, FactKind::Calendar, "明日评审", "2026-10-30T10:00:00+08:00")]);
        let work = present_matter(&request, input.now_ms, input.utc_offset_seconds);
        let calendar = present_matter(&scheduled, input.now_ms, input.utc_offset_seconds);
        assert!(work.priority > calendar.priority);
        assert_eq!(work.primary_action, MatterAction::ContinueDiscussion);
        assert!(work.what_happened.contains("预算明细"));
        assert!(work.available_output.is_none());
        assert!(!work.what_happened.contains("成稿"));
        assert!(calendar.what_happened.contains("10-30 10:00"));
        assert_eq!(calendar.valid_until_ms, scheduled.facts[0].occurred_ms);
    }

    #[test]
    fn stale_calendar_unknown_time_and_bare_questions_are_background() {
        let input = input(Period::Day);
        for mut entry in [
            fact(1, FactKind::Calendar, "过期安排", "2026-10-20T10:00:00+08:00"),
            fact(2, FactKind::Question, "为什么会这样？", "2026-10-29T10:00:00+08:00"),
            fact(3, FactKind::Question, "请确认预算", "2026-10-29T10:00:00+08:00"),
        ] {
            if entry.id == format!("{:032x}", 3) { entry.occurred_ms = None; }
            if entry.occurred_ms.is_none() {
                entry.previous.push(ReviewRevision { text: "旧屏幕片段".into(), observed_ms: entry.observed_ms - 1, occurred_ms: None });
            }
            let card = present_matter(&matter(vec![entry]), input.now_ms, input.utc_offset_seconds);
            assert_eq!(card.priority, 0);
            assert_ne!(card.primary_action, MatterAction::PrepareSchedule);
        }
    }

    #[test]
    fn recorded_output_is_useful_without_claiming_current_existence_or_delivery() {
        let input = input(Period::Day);
        let mut output = fact(1, FactKind::Artifact, "读书会方案.md\n文件：/not/verified.md", "2026-10-29T10:00:00+08:00");
        output.occurred_ms = None;
        let card = present_matter(&matter(vec![output]), input.now_ms, input.utc_offset_seconds);
        assert_eq!(card.available_output.as_deref(), Some("读书会方案.md"));
        assert_eq!(card.primary_action, MatterAction::ReviewOutput);
        assert!(card.basis_note.contains("未检查文件是否仍存在"));
        assert!(!card.what_happened.contains("已完成"));
        assert!(!card.what_happened.contains("已送达"));
    }
}
