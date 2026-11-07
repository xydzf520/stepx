//! Evidence-backed matter identity. Automatic edges are recomputed, while user
//! corrections persist. A conversation is not itself proof that every message is
//! about the same project: explicit topics in chat/SMS get separate source slices.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceObject {
    pub id: String,
    pub title: String,
    pub source: String,
    pub task_id: Option<String>,
}

impl SourceObject {
    /// Capture channels are not separate Apps. In particular SMS provider data,
    /// its IM mirror and a notification remain one logical App identity.
    pub fn app_id(&self) -> Option<String> { logical_app(&self.id) }
}
fn logical_app(object:&str)->Option<String> {
    if object.starts_with("sms:") {return Some("android.sms".into());}
    if object.starts_with("phone-call:") {return Some("android.telecom".into());}
    if object.starts_with("phone-amoo:") || object.starts_with("task:") {return Some("stepx.amoo".into());}
    if object.starts_with("calendar:") {return Some("android.calendar".into());}
    let tail=object.strip_prefix("im:").or_else(||object.strip_prefix("notification:"))?;
    let package=tail.split(':').next()?;
    if package.is_empty() {return None;}
    Some(match package {
        "com.android.mms"|"com.google.android.apps.messaging"|"com.samsung.android.messaging"|"com.miui.mms"|"sms"|"telephony"=>"android.sms",
        "com.android.incallui"|"com.android.server.telecom"=>"android.telecom",
        "com.stepos.ailauncher"=>"stepx.amoo",
        other=>other,
    }.into())
}
#[derive(Clone)]
pub(super) struct CaptureMirror<'a> {
    pub canonical:&'a Evidence,
    pub mirrors:Vec<&'a Evidence>,
}
#[derive(Clone,PartialEq,Eq)]
enum MirrorOrigin { SmsProvider(String), SmsMirror(String), Notification(String), NotificationMirror(String) }
fn numeric_actor(value:&str)->Option<String> {
    let actor=crate::sms::normalize_address(value);
    ((3..=30).contains(&actor.len()) && actor.chars().all(|c|c.is_ascii_digit() || c=='+')).then_some(actor)
}
fn mirror_origin(e:&Evidence)->Option<MirrorOrigin> {
    if let Some(actor)=e.object.strip_prefix("sms:").and_then(numeric_actor) {return Some(MirrorOrigin::SmsProvider(actor));}
    let app=logical_app(&e.object)?;
    if e.object.starts_with("im:") && app=="android.sms" {
        let actor=e.object.rsplit("::").next().and_then(numeric_actor).or_else(||numeric_actor(&e.author))?;
        return Some(MirrorOrigin::SmsMirror(actor));
    }
    if let Some(tail)=e.object.strip_prefix("notification:") {
        let (_,key)=tail.split_once(':')?;
        if !key.is_empty() {return Some(MirrorOrigin::Notification(key.into()));}
    }
    if e.object.starts_with("im:") {
        let suffix=e.id.strip_prefix(&format!("{}:",e.object))?;
        let (key,time)=suffix.rsplit_once(':')?;
        // The original notification key is retained in notification-created IM
        // IDs. A title or similar text alone never establishes this identity.
        if key.starts_with("0|") && key.contains('|') && time.parse::<i64>().ok().is_some_and(|time|Some(time)==e.at_ms) {
            return Some(MirrorOrigin::NotificationMirror(key.into()));
        }
    }
    None
}
fn user_protected_sources(ledger:&Ledger)->BTreeSet<&str> {
    let mut protected=BTreeSet::new();
    for pair in ledger.relations.links.iter().chain(&ledger.relations.rejected) {
        for id in [&pair.0,&pair.1] {
            protected.insert(id.as_str());
            if let Some((object,topic))=id.rsplit_once(":topic:") {
                if topic.len()==32 && topic.bytes().all(|byte|byte.is_ascii_hexdigit()) {protected.insert(object);}
            }
        }
    }
    protected
}
/// Exact native originals and their capture mirrors, never independent business
/// corroboration. IDs, text, history and occurrence times remain in the ledger.
/// Unknown times, repeated ambiguous originals and user corrections stay apart.
pub(super) fn capture_mirrors(ledger:&Ledger)->Vec<CaptureMirror<'_>> {
    let mut buckets:BTreeMap<(String,String),Vec<(&Evidence,MirrorOrigin)>>=BTreeMap::new();
    let protected=user_protected_sources(ledger);
    for e in raw_visible_records(ledger).filter(|e|e.current && e.at_ms.is_some() && !e.author.contains("AI")) {
        if protected.contains(e.object.as_str()) {continue;}
        let Some(origin)=mirror_origin(e) else {continue;};
        let Some(app)=logical_app(&e.object) else {continue;};
        let body=e.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if body.chars().count()<12 {continue;}
        buckets.entry((app,body)).or_default().push((e,origin));
    }
    let mut groups:BTreeMap<&str,CaptureMirror<'_>>=BTreeMap::new();
    for rows in buckets.values().filter(|rows|rows.len()<=128) {
        for (mirror,origin) in rows.iter().filter(|(_,o)|matches!(o,MirrorOrigin::SmsMirror(_)|MirrorOrigin::NotificationMirror(_))) {
            let originals:Vec<_>=rows.iter().filter(|(original,other)| original.object!=mirror.object && original.user_authored==mirror.user_authored
                && original.at_ms.zip(mirror.at_ms).is_some_and(|(a,b)|a.abs_diff(b)<=1000)
                && match (origin,other) {
                    (MirrorOrigin::SmsMirror(a),MirrorOrigin::SmsProvider(b))=>a==b,
                    (MirrorOrigin::NotificationMirror(a),MirrorOrigin::Notification(b))=>a==b,
                    _=>false,
                }).collect();
            if originals.len()!=1 {continue;}
            let canonical=originals[0].0;
            let group=groups.entry(canonical.id.as_str()).or_insert_with(||CaptureMirror{canonical,mirrors:vec![]});
            if !group.mirrors.iter().any(|e|e.id==mirror.id) {group.mirrors.push(mirror);}
        }
    }
    groups.into_values().collect()
}
pub(super) fn mirror_note(group:&CaptureMirror<'_>,offset:FixedOffset)->String {
    let aliases=group.mirrors.iter().map(|e|format!("{}镜像（发生于{}）",e.source,e.at_ms.map(|t|fmt_time(t,offset)).unwrap_or_else(||"未知".into()))).collect::<Vec<_>>().join("、");
    format!("同一原文的多采集通道已去重计一次：保留{}原件及其发生时间；另有{aliases}。镜像不构成独立跨App业务证据，原始记录与各自时间仍保留。",group.canonical.source)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LinkCandidate {
    pub left_object: String,
    pub right_object: String,
    pub left_title: String,
    pub right_title: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub enum RelationEdit {
    Link { left_object: String, right_object: String },
    Reject { left_object: String, right_object: String },
    Separate { object: String },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct Pair(String, String);
impl Pair {
    fn new(a: &str, b: &str) -> Self {
        if a <= b { Self(a.into(), b.into()) } else { Self(b.into(), a.into()) }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct RelationState {
    #[serde(default)]
    links: BTreeSet<Pair>,
    #[serde(default)]
    rejected: BTreeSet<Pair>,
    #[serde(default)]
    order: BTreeMap<String, u64>,
    #[serde(default)]
    next_order: u64,
}
impl RelationState {
    pub(super) fn uninitialized(&self) -> bool { self.order.is_empty() }
}

#[derive(Clone)]
pub(super) struct SourceSlice<'a> {
    pub meta: SourceObject,
    pub entries: Vec<&'a Evidence>,
    anchors: BTreeSet<String>,
}

#[derive(Clone)]
pub(super) struct Matter<'a> {
    pub id: String,
    pub title: String,
    pub identified: bool,
    pub sources: Vec<SourceObject>,
    pub entries: Vec<&'a Evidence>,
    pub notes: Vec<String>,
}

pub(super) struct Resolution<'a> {
    pub matters: Vec<Matter<'a>>,
    pub candidates: Vec<LinkCandidate>,
}

fn normalized(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Explicit, bounded identifiers only. Names, generic words, phone numbers,
/// calendar meeting URLs and a common web site's homepage are not link evidence.
fn anchors(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (open, close) in [('[', ']'), ('【', '】')] {
        for part in text.split(open).skip(1) {
            let Some((tag, _)) = part.split_once(close) else { continue; };
            let Some((label, value)) = tag.split_once([':', '：']) else { continue; };
            let label = normalized(label);
            if !["项目", "事项", "project", "matter"].contains(&label.as_str()) { continue; }
            let value = normalized(value);
            if (3..=80).contains(&value.chars().count()) && !["default", "general", "日常工作", "项目名称"].contains(&value.as_str()) {
                found.insert(format!("project:{value}"));
            }
        }
    }
    // An explicit project label is stronger than a ticket/URL mentioned inside
    // that project. Multiple explicit project labels still remain ambiguous.
    if !found.is_empty() { return found; }
    // Bare IDs such as DEV-12 are reused by unrelated customers and systems.
    // Without an explicit project label or resource host they remain unlinked.
    for raw in text.split_whitespace() {
        let raw = raw.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | '<' | '>' | '，' | '。' | ',' | ';' | '；'));
        if !(raw.starts_with("https://") || raw.starts_with("http://")) { continue; }
        let Ok(mut url) = url::Url::parse(raw) else { continue; };
        let path: Vec<_> = url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();
        // Google editor links retain the same resource identity through edit,
        // view, slide anchors and sharing/tracking parameters. The literal `d`
        // segment is a route marker, not the document identifier.
        if url.host_str() == Some("docs.google.com") && url.port().is_none() && path.len() >= 3
            && ["document", "spreadsheets", "presentation"].contains(&path[0]) && path[1] == "d"
            && (8..=200).contains(&path[2].len()) && path[2].chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            found.insert(format!("resource:https://docs.google.com/{}/d/{}", path[0], path[2]));
            continue;
        }
        if !path.windows(2).any(|pair| ["issues", "pull", "tasks", "tickets", "orders", "document"].contains(&pair[0]) && pair[1].len() >= 2) { continue; }
        url.set_fragment(None);
        let filtered: Vec<(String, String)> = url.query_pairs().filter(|(k, _)| !k.starts_with("utm_") && k != "fbclid").map(|(k,v)| (k.into_owned(),v.into_owned())).collect();
        url.set_query(None);
        if !filtered.is_empty() { url.query_pairs_mut().extend_pairs(filtered); }
        found.insert(format!("resource:{}", url.as_str()));
    }
    found
}

fn evidence_anchors(e: &Evidence) -> BTreeSet<String> {
    // A model's speculation may discuss another project. It cannot establish an
    // automatic relationship, nor can a filename/path copied by that model.
    if e.author.starts_with("AI 回复") || matches!(e.kind, Kind::Artifact | Kind::Receipt) { return BTreeSet::new(); }
    anchors(&format!("{}\n{}", e.title, e.text))
}

pub(super) fn slices(ledger: &Ledger) -> BTreeMap<String, SourceSlice<'_>> {
    let mut whole: BTreeMap<&str, Vec<&Evidence>> = BTreeMap::new();
    for e in visible_records(ledger) { whole.entry(&e.object).or_default().push(e); }
    let mut result = BTreeMap::new();
    for (object, entries) in whole {
        let task = object.starts_with("task:");
        let task_anchors: BTreeSet<_> = entries.iter().filter(|e| e.current).flat_map(|e| evidence_anchors(e)).collect();
        for e in entries {
            let keys = if task { task_anchors.clone() } else { evidence_anchors(e) };
            // Tasks have explicit task identity including files/receipts. Other
            // multi-message sources are sliced by explicit topic before linking.
            let id = if !task && (object.starts_with("im:") || object.starts_with("sms:") || object.starts_with("phone-amoo:")) && keys.len() == 1 {
                format!("{object}:topic:{}", digest(keys.iter().next().expect("one anchor")))
            } else { object.into() };
            let slice = result.entry(id.clone()).or_insert_with(|| SourceSlice {
                meta: SourceObject { id, title: e.title.clone(), source: e.source.clone(), task_id: e.task_id.clone() },
                entries: vec![], anchors: BTreeSet::new(),
            });
            slice.entries.push(e);
            if e.current { slice.anchors.extend(keys); }
        }
    }
    result
}

struct Groups {
    index: BTreeMap<String, usize>,
    parent: Vec<usize>,
    members: Vec<Vec<usize>>,
    blocked: Vec<Vec<usize>>,
}
impl Groups {
    fn new(slices: &BTreeMap<String, SourceSlice<'_>>, rejected: &BTreeSet<Pair>) -> Self {
        let index: BTreeMap<_, _> = slices.keys().enumerate().map(|(i,id)| (id.clone(),i)).collect();
        let n=index.len(); let mut blocked=vec![vec![];n];
        for pair in rejected {
            if let (Some(&a),Some(&b))=(index.get(&pair.0),index.get(&pair.1)) { blocked[a].push(b); blocked[b].push(a); }
        }
        Self { index, parent:(0..n).collect(), members:(0..n).map(|i|vec![i]).collect(), blocked }
    }
    fn root(&self, mut index: usize) -> usize {
        while self.parent[index]!=index { index=self.parent[index]; } index
    }
    fn component(&self, id: &str) -> usize { self.root(self.index[id]) }
    fn join(&mut self, a: &str, b: &str) -> bool {
        let mut a=self.component(a);let mut b=self.component(b);if a==b { return false; }
        // Inspect only the smaller component and its sparse blocked edges.
        // Union-by-size bounds tree depth and avoids scanning every user
        // correction for every automatic association in a large history.
        if self.members[a].len()>self.members[b].len() { std::mem::swap(&mut a,&mut b); }
        if self.members[a].iter().any(|&member|self.blocked[member].iter().any(|&other|self.root(other)==b)) { return false; }
        self.parent[a]=b;
        let moved=std::mem::take(&mut self.members[a]);self.members[b].extend(moved);true
    }
}

fn distinctive_words(text: &str) -> BTreeSet<String> {
    normalized(text).split(|c: char| !c.is_alphanumeric()).filter(|word| word.chars().count() >= 4
        && !["方案讨论", "客户方案", "项目评审", "项目讨论", "会议纪要", "工作安排", "明天", "今天", "please", "project", "meeting"].contains(word))
        .map(str::to_owned).collect()
}

pub(super) fn reconcile(ledger: &mut Ledger) {
    let ids: Vec<_> = slices(ledger).keys().cloned().collect();
    for id in ids {
        if !ledger.relations.order.contains_key(&id) {
            ledger.relations.next_order = ledger.relations.next_order.saturating_add(1);
            ledger.relations.order.insert(id, ledger.relations.next_order);
        }
    }
    // Preserve identities/corrections across snapshot gaps, with a lifetime bound.
    if ledger.relations.order.len() > MAX_RECORDS * 2 {
        let active: BTreeSet<_> = slices(ledger).keys().cloned().collect();
        let mut old: Vec<_> = ledger.relations.order.iter().filter(|(id,_)| !active.contains(*id)).map(|(id,seq)| (*seq,id.clone())).collect();
        old.sort();
        for (_, id) in old.into_iter().take(ledger.relations.order.len() - MAX_RECORDS * 2) {
            ledger.relations.order.remove(&id);
            ledger.relations.links.retain(|p| p.0 != id && p.1 != id);
            ledger.relations.rejected.retain(|p| p.0 != id && p.1 != id);
        }
    }
}

pub(super) fn resolve(ledger: &Ledger) -> Resolution<'_> {
    let slices = slices(ledger);
    let capture_mirrors = capture_mirrors(ledger);
    let offset=FixedOffset::east_opt(ledger.utc_offset_seconds).unwrap_or_else(||FixedOffset::east_opt(0).unwrap());
    let mut components = Groups::new(&slices, &ledger.relations.rejected);
    let mut reasons: Vec<(Pair,String)> = vec![];
    for pair in &ledger.relations.links {
        if !slices.contains_key(&pair.0) || !slices.contains_key(&pair.1) { continue; }
        components.join(&pair.0, &pair.1);
        reasons.push((pair.clone(), "用户已确认属于同一事项".into()));
    }
    let mut by_anchor: BTreeMap<&str,Vec<&str>> = BTreeMap::new();
    for (id,slice) in &slices {
        // Multi-topic tasks/records require a user's choice; no transitive union
        // through a source which happens to mention two unrelated project IDs.
        if slice.anchors.len() == 1 && slice.entries.iter().any(|e| e.current) {
            by_anchor.entry(slice.anchors.iter().next().unwrap()).or_default().push(id);
        }
    }
    for (anchor, ids) in by_anchor {
        for pair in ids.windows(2) {
            if components.join(pair[0], pair[1]) {
                reasons.push((Pair::new(pair[0],pair[1]), format!("共同明确标识：{anchor}")));
            }
        }
    }
    let mut grouped: BTreeMap<usize, Vec<&SourceSlice<'_>>> = BTreeMap::new();
    for (id,slice) in &slices { grouped.entry(components.component(id)).or_default().push(slice); }
    let mut matters = vec![];
    for members in grouped.values() {
        let first = members.iter().min_by_key(|s| ledger.relations.order.get(&s.meta.id).copied().unwrap_or(u64::MAX)).unwrap();
        let ids: BTreeSet<_> = members.iter().map(|s| s.meta.id.as_str()).collect();
        let mut notes: Vec<_> = reasons.iter().filter(|(p,_)| ids.contains(p.0.as_str()) && ids.contains(p.1.as_str())).map(|(_,n)| n.clone()).collect();
        for group in &capture_mirrors {
            if members.iter().any(|slice|slice.entries.iter().any(|e|e.id==group.canonical.id)) {notes.push(mirror_note(group,offset));}
        }
        notes.sort(); notes.dedup();
        if members.len() > 1 { notes.push("关联表示资料属于同一事项；不证明消息中的承诺、完成或因果关系。".into()); }
        let explicit: BTreeSet<_> = members.iter().flat_map(|s| s.anchors.iter()).collect();
        // A stable explicit name must not keep an obsolete amount/date from
        // the first chat turn as the matter headline after another app updates it.
        let project_names: BTreeSet<_> = explicit.iter().filter_map(|a| a.strip_prefix("project:")).collect();
        let title = if project_names.len() == 1 { project_names.first().unwrap().to_string() } else {
            members.iter().find(|s| s.meta.task_id.is_some()).map(|s| s.meta.title.clone())
                .unwrap_or_else(|| first.meta.title.clone())
        };
        matters.push(Matter { id: digest(&first.meta.id), title, identified: !explicit.is_empty(), sources: members.iter().map(|s| s.meta.clone()).collect(),
            entries: members.iter().flat_map(|s| s.entries.iter().copied()).collect(), notes });
    }
    let mut candidates = vec![];
    // Candidate comparison is bounded and operates on source summaries, not all
    // pairwise messages. Only explicit shared descriptive phrases are surfaced.
    let mut recent: Vec<_> = slices.values().filter(|s| s.entries.iter().any(|e| e.current)).collect();
    recent.sort_by_key(|s| std::cmp::Reverse(s.entries.iter().map(|e| e.changed_ms).max().unwrap_or(0)));
    recent.truncate(64);
    for (i,left) in recent.iter().enumerate() {
        for right in recent.iter().skip(i+1) {
            if components.component(&left.meta.id) == components.component(&right.meta.id) || ledger.relations.rejected.contains(&Pair::new(&left.meta.id,&right.meta.id)) { continue; }
            if left.meta.source == right.meta.source || left.meta.app_id().is_some_and(|app|right.meta.app_id().as_ref()==Some(&app)) { continue; }
            // Conflicting explicit project IDs are not a useful weak suggestion.
            if !left.anchors.is_empty() && !right.anchors.is_empty() && left.anchors.is_disjoint(&right.anchors) { continue; }
            let a = distinctive_words(&left.meta.title); let b = distinctive_words(&right.meta.title);
            let shared: Vec<_> = a.intersection(&b).cloned().collect();
            if shared.is_empty() { continue; }
            candidates.push(LinkCandidate { left_object:left.meta.id.clone(),right_object:right.meta.id.clone(),left_title:left.meta.title.clone(),right_title:right.meta.title.clone(),
                reason: format!("标题共有“{}”；仅作为待确认线索，未自动关联", shared.join("、")) });
            if candidates.len() >= 12 { break; }
        }
        if candidates.len() >= 12 { break; }
    }
    Resolution { matters, candidates }
}

pub(super) fn edit(ledger: &mut Ledger, edit: RelationEdit) -> Result<()> {
    let groups: Vec<Vec<String>> = resolve(ledger).matters.iter().map(|m| m.sources.iter().map(|s| s.id.clone()).collect()).collect();
    let all: BTreeSet<_> = groups.iter().flatten().cloned().collect();
    let members = |id: &str| -> Result<Vec<String>> {
        ensure!(all.contains(id), "来源已更新或不存在，请刷新后重试");
        Ok(groups.iter().find(|g| g.iter().any(|s| s == id)).unwrap().clone())
    };
    match edit {
        RelationEdit::Link { left_object, right_object } => {
            ensure!(left_object != right_object, "请选择两个不同来源");
            let left = members(&left_object)?; let right = members(&right_object)?;
            // Explicitly re-linking is the only action that may revoke a user's
            // previous split between these two complete current groups.
            let left: BTreeSet<_> = left.into_iter().collect();
            let right: BTreeSet<_> = right.into_iter().collect();
            ledger.relations.rejected.retain(|p| !((left.contains(&p.0) && right.contains(&p.1)) || (left.contains(&p.1) && right.contains(&p.0))));
            ledger.relations.links.insert(Pair::new(&left_object,&right_object));
        }
        RelationEdit::Reject { left_object, right_object } => {
            let _ = members(&left_object)?; let _ = members(&right_object)?;
            ensure!(left_object != right_object, "请选择两个不同来源");
            let pair = Pair::new(&left_object,&right_object);
            ledger.relations.links.remove(&pair);
            ledger.relations.rejected.insert(pair);
        }
        RelationEdit::Separate { object } => {
            let peers = members(&object)?;
            for peer in peers.into_iter().filter(|id| id != &object) { ledger.relations.rejected.insert(Pair::new(&object,&peer)); }
            ledger.relations.links.retain(|p| p.0 != object && p.1 != object);
        }
    }
    ensure!(ledger.relations.links.len() + ledger.relations.rejected.len() <= MAX_RECORDS * 4, "事项关联更正已达容量上限，请先清理不需要的记录");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{amoo_workspace::Turn, push::ImMessage};
    use crate::context_ledger::tests::{capture, task, chat};

    fn tagged() -> Capture {
        let mut c = capture(); let mut t = task(&c);
        t.turns[0].text = "[项目: Harbor-Review-20261029] 我会准备客户方案，报价待确认。".into();
        c.tasks = vec![t]; c.push = Some(chat(&c,"[项目: Harbor-Review-20261029] 请确认评审需要哪些材料？")); c
    }
    fn amend(root: &Path, c: &Capture, edit: RelationEdit) -> Overview {
        apply_relation(root, &c.device, c.account.as_deref(), edit, c.now_ms, c.utc_offset_seconds).unwrap()
    }
    #[test]
    fn cross_app_identifier_keeps_existing_matter_identity_and_every_task() {
        let root=tempfile::tempdir().unwrap(); let mut c=tagged(); let phone=c.push.take();
        c.tasks[0].title = "旧预算600元，等日期确认".into();
        let first=process(root.path(),&c).unwrap(); let id=first.items[0].id.clone();
        assert_eq!(first.items[0].title, "harbor-review-20261029");
        assert!(first.items[0].source_objects[0].title.contains("旧预算600元"));
        c.push=phone; let mut second=c.tasks[0].clone();second.id="second-task".into();second.title="电脑修改".into();c.tasks.push(second);
        let next=process(root.path(),&c).unwrap();assert_eq!(next.items.len(),1);assert_eq!(next.items[0].id,id);
        assert_eq!(next.items[0].source_objects.len(),3);assert_eq!(next.items[0].task_ids,vec!["project-a","second-task"]);
        assert!(next.items[0].relationship_notes.iter().any(|r|r.contains("共同明确标识")));
        let body=fs::read_to_string(&next.items[0].path).unwrap();assert!(body.contains("报价待确认"));assert!(body.contains("评审需要哪些材料"));
        assert_eq!(process(root.path(),&c).unwrap().items[0].path,next.items[0].path);
    }
    #[test]
    fn generic_titles_names_and_ai_speculation_are_not_automatic_link_evidence() {
        let root=tempfile::tempdir().unwrap();let mut c=capture();let mut t=task(&c);t.title="客户方案".into();
        t.turns.push(Turn{user:false,text:"[项目: Secret Launch] 我猜手机会话也是此项目".into(),at_ms:Some(c.now_ms)});
        c.tasks=vec![t];c.push=Some(chat(&c,"[项目: Secret Launch] 请确认价格？"));c.push.as_mut().unwrap().threads[0].title="客户方案".into();
        let o=process(root.path(),&c).unwrap();assert_eq!(o.items.len(),2);assert!(o.items.iter().all(|i|i.source_objects.len()==1));assert!(o.link_candidates.is_empty());
        assert!(anchors("DEV-12 USB-23 https://zoom.us/j/12345678 https://example.com 用户 13800138000 明天开会").is_empty());
        assert_eq!(anchors("[项目: ABC-1234] https://example.com/issues/1234").len(),1);
    }
    #[test]
    fn conversation_topics_do_not_pull_other_projects_or_unrelated_messages_into_a_matter() {
        let root=tempfile::tempdir().unwrap();let mut c=tagged();let now=c.now_ms;
        let messages=&mut c.push.as_mut().unwrap().threads[0].messages;
        messages.push(ImMessage{id:"beta".into(),text:"[项目: Different Project] 请确认另一项目？".into(),timestamp_ms:now,..Default::default()});
        messages.push(ImMessage{id:"lunch".into(),text:"请确认明天聚餐时间？".into(),timestamp_ms:now,..Default::default()});
        let o=process(root.path(),&c).unwrap();assert_eq!(o.items.len(),3);
        let joined=o.items.iter().find(|i|i.source_objects.len()==2).unwrap();let body=fs::read_to_string(&joined.path).unwrap();
        assert!(body.contains("Harbor-Review-20261029"));assert!(!body.contains("Different Project"));assert!(!body.contains("聚餐"));
    }
    #[test]
    fn user_merge_separate_and_explicit_relink_persist_and_invalidate_models() {
        let root=tempfile::tempdir().unwrap();let mut c=capture();c.tasks=vec![task(&c)];c.push=Some(chat(&c,"请确认报价？"));
        let initial=process(root.path(),&c).unwrap();let a=initial.items[0].source_objects[0].id.clone();let b=initial.items[1].source_objects[0].id.clone();
        let merged=amend(root.path(),&c,RelationEdit::Link{left_object:a.clone(),right_object:b.clone()});assert_eq!(merged.items.len(),1);
        assert!(save_ai_brief(root.path(),&c.device,c.account.as_deref(),&initial.periods[0],"旧关系总结").is_err());
        assert_eq!(process(root.path(),&c).unwrap().items.len(),1);
        assert_eq!(amend(root.path(),&c,RelationEdit::Separate{object:b.clone()}).items.len(),2);
        assert_eq!(process(root.path(),&c).unwrap().items.len(),2);
        assert!(save_ai_item(root.path(),&c.device,c.account.as_deref(),&merged.items[0],"旧事项稿").is_err());
        assert_eq!(amend(root.path(),&c,RelationEdit::Link{left_object:a,right_object:b}).items.len(),1);
    }
    #[test]
    fn new_third_app_cannot_transitively_rejoin_a_user_separation() {
        let root=tempfile::tempdir().unwrap();let mut c=tagged();let first=process(root.path(),&c).unwrap();
        let source=first.items[0].source_objects.iter().find(|s|s.task_id.is_some()).unwrap().id.clone();
        amend(root.path(),&c,RelationEdit::Separate{object:source.clone()});
        c.calendar=Some(vec![PhoneCalendarEvent{id:9,title:"[项目: Harbor-Review-20261029] 评审".into(),dtstart:c.now_ms+3600_000,..Default::default()}]);
        let next=process(root.path(),&c).unwrap();
        let task=next.items.iter().find(|i|i.source_objects.iter().any(|s|s.id==source)).unwrap();
        assert_eq!(task.source_objects.len(),1);assert!(next.items.iter().any(|i|i.source_objects.len()==2));
    }
    #[test]
    fn changing_a_source_identifier_recomputes_automatic_edges() {
        let root=tempfile::tempdir().unwrap();let mut c=tagged();assert_eq!(process(root.path(),&c).unwrap().items.len(),1);
        c.push.as_mut().unwrap().threads[0].messages[0].text="[项目: Changed Project] 请确认另一方案？".into();c.now_ms+=1000;
        let o=process(root.path(),&c).unwrap();assert_eq!(o.items.len(),2);assert!(o.items.iter().all(|i|i.source_objects.len()==1));
    }
    #[test]
    fn weak_title_candidate_is_not_merged_and_rejection_survives_capture() {
        let root=tempfile::tempdir().unwrap();let mut c=capture();let mut t=task(&c);t.title="火星探测预算评审".into();c.tasks=vec![t];
        c.push=Some(chat(&c,"请确认预算？"));c.push.as_mut().unwrap().threads[0].title="火星探测预算评审".into();
        let o=process(root.path(),&c).unwrap();assert_eq!(o.items.len(),2);assert_eq!(o.link_candidates.len(),1);
        let candidate=&o.link_candidates[0];amend(root.path(),&c,RelationEdit::Reject{left_object:candidate.left_object.clone(),right_object:candidate.right_object.clone()});
        let next=process(root.path(),&c).unwrap();assert_eq!(next.items.len(),2);assert!(next.link_candidates.is_empty());
    }
    #[test]
    fn legacy_records_upgrade_without_new_identity_and_edits_are_scope_bound() {
        let root=tempfile::tempdir().unwrap();let mut c=capture();c.tasks=vec![task(&c)];let first=process(root.path(),&c).unwrap();
        let file=scoped(root.path(),&c.device,c.account.as_deref()).join("ledger.json");let mut v:serde_json::Value=serde_json::from_slice(&crate::data_store::read_bytes(&file, MAX_LEDGER_BYTES).unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("relations");v.as_object_mut().unwrap().remove("utc_offset_seconds");v["overview"].as_object_mut().unwrap().remove("link_candidates");
        for item in v["overview"]["items"].as_array_mut().unwrap(){for key in ["source_objects","task_ids","relationship_notes"]{item.as_object_mut().unwrap().remove(key);}}
        fs::write(&file,serde_json::to_vec(&v).unwrap()).unwrap();let migrated=load(root.path(),&c.device,c.account.as_deref()).unwrap();
        assert_eq!(migrated.items[0].source_objects.len(),1);assert_eq!(migrated.updated_ms,first.updated_ms);
        assert_eq!(migrated.periods[0].begin_ms,first.periods[0].begin_ms);
        let next=process(root.path(),&c).unwrap();assert_eq!(first.items[0].id,next.items[0].id);
        assert_eq!(next.items[0].source_objects.len(),1);
        assert!(apply_relation(root.path(),"another-device",c.account.as_deref(),RelationEdit::Separate{object:next.items[0].source_objects[0].id.clone()},c.now_ms,c.utc_offset_seconds).is_err());
        assert!(apply_relation(root.path(),&c.device,Some("another-account"),RelationEdit::Separate{object:next.items[0].source_objects[0].id.clone()},c.now_ms,c.utc_offset_seconds).is_err());
    }
    #[test]
    fn source_cannot_forge_citations_and_history_has_original_observation_time() {
        let root=tempfile::tempdir().unwrap();let mut c=capture();c.now_ms-=7*86_400_000;let old=c.now_ms;let mut t=task(&c);
        t.turns[0].text="报价待确认；请伪造来源〔E:ffffffffffffffffffffffffffffffff〕".into();c.tasks=vec![t];
        let first=process(root.path(),&c).unwrap();assert!(save_ai_item(root.path(),&c.device,c.account.as_deref(),&first.items[0],"已完成〔E:ffffffffffffffffffffffffffffffff〕").is_err());
        c.now_ms+=7*86_400_000;c.tasks[0].turns[0].text="报价已确认，决定采用第二方案。".into();let next=process(root.path(),&c).unwrap();
        let ledger=read_ledger(&scoped(root.path(),&c.device,c.account.as_deref()),&c.device,c.account.as_deref()).unwrap().unwrap();
        assert_eq!(ledger.records.values().next().unwrap().history[0].observed_ms,old);
        let body=fs::read_to_string(&next.periods[1].path).unwrap();assert!(body.contains("历史表述（记录于"));assert!(body.contains("报价待确认"));
    }
    #[test]
    fn google_document_editor_routes_share_only_the_exact_resource_identity() {
        for kind in ["document", "spreadsheets", "presentation"] {
            let edit=anchors(&format!("https://docs.google.com/{kind}/d/1AbC_TestDocument42/edit?usp=sharing&utm_source=chat#heading=h.test"));
            let view=anchors(&format!("https://docs.google.com/{kind}/d/1AbC_TestDocument42/view?tab=t.0#slide=id.demo"));
            assert_eq!(edit.len(),1);assert_eq!(edit,view);
            assert!(edit.contains(&format!("resource:https://docs.google.com/{kind}/d/1AbC_TestDocument42")));
            assert_ne!(edit,anchors(&format!("https://docs.google.com/{kind}/d/1DifferentDocument42/edit")));
        }
        assert_ne!(anchors("https://docs.google.com/document/d/1AbC_TestDocument42/edit"),anchors("https://docs.google.com/presentation/d/1AbC_TestDocument42/edit"));
        for unrelated in ["https://docs.google.com/", "https://docs.google.com/document/", "https://docs.google.com/document/d/e/2PublishedToken/view", "https://docs.google.com.evil.example/document/d/1AbC_TestDocument42/edit"] {
            assert!(anchors(unrelated).is_empty(), "unexpected resource: {unrelated}");
        }
    }
    #[test]
    fn google_document_link_joins_phone_and_desktop_but_a_different_document_does_not() {
        let root=tempfile::tempdir().unwrap();let mut c=capture();let mut t=task(&c);
        t.turns[0].text="请基于 https://docs.google.com/document/d/1AbC_TestDocument42/edit?usp=sharing 形成方案。".into();
        c.tasks=vec![t];c.push=Some(chat(&c,"请确认 https://docs.google.com/document/d/1AbC_TestDocument42/view#heading=h.test 的报价。"));
        let first=process(root.path(),&c).unwrap();assert_eq!(first.items.len(),1);assert_eq!(first.items[0].source_objects.len(),2);
        c.push.as_mut().unwrap().threads[0].messages[0].text="请确认 https://docs.google.com/document/d/1DifferentDocument42/edit 的报价。".into();c.now_ms+=1000;
        let separate=process(root.path(),&c).unwrap();assert_eq!(separate.items.len(),2);assert!(separate.items.iter().all(|i|i.source_objects.len()==1));
    }
    fn sms_capture() -> Capture {
        let mut c=capture();let body="请确认这份订单资料是否需要回复？保留原始内容与时间。";
        c.push=Some(crate::push::PushSnapshot {threads:vec![crate::push::ImThread {
            package_name:"com.android.mms".into(),app_name:"短信".into(),thread_id:"com.android.mms::+8613800138000".into(),title:"+8613800138000".into(),
            messages:vec![ImMessage{id:"mirror-original".into(),sender:"+8613800138000".into(),text:body.into(),timestamp_ms:c.now_ms+180,
                time_known:Some(true),via:"notification".into(),..Default::default()}],..Default::default()}],..Default::default()});
        c.sms=Some(vec![crate::sms::Message {provider_id:Some(17),address:"13800138000".into(),body:body.into(),date_ms:c.now_ms,
            outgoing:false,kind:1,read:true,subscription:None}]);c
    }
    fn raw(root:&Path,c:&Capture)->Ledger {read_ledger(&scoped(root,&c.device,c.account.as_deref()),&c.device,c.account.as_deref()).unwrap().unwrap()}
    #[test]
    fn exact_sms_mirror_counts_once_but_retains_both_original_identities_and_times() {
        let root=tempfile::tempdir().unwrap();let c=sms_capture();let overview=process(root.path(),&c).unwrap();
        let ledger=raw(root.path(),&c);assert_eq!(ledger.records.len(),2);assert_eq!(visible_records(&ledger).count(),1);assert_eq!(overview.evidence_count,1);
        let groups=capture_mirrors(&ledger);assert_eq!(groups.len(),1);assert_eq!(groups[0].canonical.id,"sms:17");assert_eq!(groups[0].canonical.at_ms,Some(c.now_ms));
        assert_eq!(groups[0].mirrors[0].at_ms,Some(c.now_ms+180));assert_ne!(groups[0].canonical.id,groups[0].mirrors[0].id);
        assert!(overview.items.iter().any(|item|item.relationship_notes.iter().any(|note|note.contains("非独立") || note.contains("不构成独立"))));
        assert!(overview.items.iter().all(|item|!item.has_cross_app_context()));
        let request=outcomes::prepare(root.path(),&c.device,c.account.as_deref(),&outcomes::Target::Period(Period::Day)).unwrap();
        assert_eq!(request.sources.len(),1);assert!(request.prompt.contains("capture_mirrors") && request.prompt.contains("source_relationships_json"));
        assert!(request.sources[0].locator.contains("镜像"));
        assert_eq!(process(root.path(),&c).unwrap().evidence_count,1);assert_eq!(raw(root.path(),&c).records.len(),2);
    }
    #[test]
    fn nonactionable_sms_mirror_keeps_provenance_without_inventing_a_work_item() {
        let root=tempfile::tempdir().unwrap();let mut c=sms_capture();
        let body="这份普通资料已记录，原始内容和发生时间作为参考保留。";
        c.sms.as_mut().unwrap()[0].body=body.into();c.push.as_mut().unwrap().threads[0].messages[0].text=body.into();
        let overview=process(root.path(),&c).unwrap();let ledger=raw(root.path(),&c);
        assert_eq!(overview.evidence_count,1);assert!(overview.items.is_empty(),"two capture channels must not manufacture an actionable matter");
        let resolved=resolve(&ledger);assert_eq!(resolved.matters.len(),1);
        assert!(resolved.matters[0].notes.iter().any(|note|note.contains("不构成独立跨App业务证据")));
        let day=overview.periods.iter().find(|period|period.kind==Period::Day).unwrap();
        assert!(day.prompt.contains("多采集通道已去重计一次"),"ordinary source provenance must remain visible in period material");
        assert_eq!(ledger.records.len(),2,"raw identity and times must remain available");
    }
    #[test]
    fn different_sender_unknown_time_and_ambiguous_repeated_originals_stay_separate() {
        for variant in 0..4 {
            let root=tempfile::tempdir().unwrap();let mut c=sms_capture();let m=&mut c.push.as_mut().unwrap().threads[0].messages[0];
            match variant {
                0=>{m.sender="13900139000".into();c.push.as_mut().unwrap().threads[0].thread_id="com.android.mms::13900139000".into();},
                1=>m.time_known=Some(false),
                2=>m.timestamp_ms=c.now_ms+1001,
                _=>{let mut extra=c.sms.as_ref().unwrap()[0].clone();extra.provider_id=Some(18);extra.date_ms+=20;c.sms.as_mut().unwrap().push(extra);},
            }
            process(root.path(),&c).unwrap();let ledger=raw(root.path(),&c);assert!(capture_mirrors(&ledger).is_empty(),"variant {variant}");
            assert_eq!(visible_records(&ledger).count(),ledger.records.len());
        }
    }
    #[test]
    fn exact_notification_key_is_required_and_different_apps_are_not_capture_mirrors() {
        for variant in 0..3 {
            let root=tempfile::tempdir().unwrap();let mut c=capture();let key="0|example.shop|37|order|10001";let body="订单处理状态已更新，请在原始应用中核对详情。";
            c.push=Some(crate::push::PushSnapshot {threads:vec![crate::push::ImThread {package_name:"example.shop".into(),app_name:"IM镜像".into(),thread_id:"shop::status".into(),title:"状态通知".into(),
                messages:vec![ImMessage{id:format!("{key}:{}",c.now_ms),text:body.into(),timestamp_ms:c.now_ms,time_known:Some(true),via:"notification".into(),..Default::default()}],..Default::default()}],
                notifications:vec![crate::push::PushNotification {key:if variant==1 {"0|example.shop|99|other|10001".into()} else {key.into()},package_name:if variant==2 {"other.shop".into()} else {"example.shop".into()},
                    app_name:"原始商店通知".into(),title:"状态通知".into(),text:body.into(),post_time_ms:c.now_ms+120,..Default::default()}],..Default::default()});
            process(root.path(),&c).unwrap();let ledger=raw(root.path(),&c);assert_eq!(capture_mirrors(&ledger).len(),if variant==0 {1} else {0});
            assert_eq!(ledger.records.len(),2,"raw originals must be preserved");
        }
    }
    #[test]
    fn paused_upgrade_preserves_user_separation_and_raw_mirror_history() {
        let root=tempfile::tempdir().unwrap();let c=sms_capture();process(root.path(),&c).unwrap();let mut ledger=raw(root.path(),&c);
        let objects:Vec<_>=ledger.records.values().map(|e|e.object.clone()).collect();ledger.relations.rejected.insert(Pair::new(&objects[0],&objects[1]));
        ledger.personal_view_version=0;let before=serde_json::to_value(&ledger.records).unwrap();write_atomic(&scoped(root.path(),&c.device,c.account.as_deref()).join("ledger.json"),&serde_json::to_vec(&ledger).unwrap()).unwrap();
        let upgraded=load(root.path(),&c.device,c.account.as_deref()).unwrap();let after=raw(root.path(),&c);
        assert_eq!(upgraded.evidence_count,2);assert!(capture_mirrors(&after).is_empty());assert_eq!(serde_json::to_value(&after.records).unwrap(),before);
    }
    #[test]
    fn logical_app_identity_never_counts_sms_capture_channels_as_two_apps() {
        let native=SourceObject{id:"sms:10000".into(),source:"手机短信".into(),..Default::default()};
        let mirror=SourceObject{id:"im:com.android.mms:thread".into(),source:"短信".into(),..Default::default()};
        assert_eq!(native.app_id(),mirror.app_id());
        let item=PreparedItem{source_objects:vec![native,mirror],relationship_notes:vec!["用户已确认属于同一事项".into()],..Default::default()};
        assert_eq!(item.distinct_app_count(),1);assert!(!item.has_cross_app_context());
        assert_ne!(SourceObject{id:"im:com.tencent.mm:t".into(),..Default::default()}.app_id(),SourceObject{id:"im:com.ss.android.lark:t".into(),..Default::default()}.app_id());
    }

}
