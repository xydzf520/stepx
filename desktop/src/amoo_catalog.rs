//! Secret-free capability discovery for the phone's OpenClaw runtime.
//!
//! This runtime and Amoo's AgentBus cloud sessions are separate environments.
//! A model in the runtime's catalog is not proof of credentials or cloud support.
use crate::{adb::Adb, device::Device, openclaw::GatewayWs};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashSet, time::Duration};

const SOURCE: &str = "手机 OpenClaw";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CatalogAvailability {
    pub available: bool,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Provider-qualified ID; bare model IDs may collide between providers.
    pub id: String,
    pub name: String,
    pub provider: String,
    pub capabilities: Vec<String>,
    pub current: bool,
    pub available: bool,
    pub availability_reason: String,
    pub context_window: Option<u64>,
    pub image_input: bool,
    pub source: String,
    #[serde(default)]
    pub verified_at_ms: Option<i64>,
    #[serde(default)]
    pub verified_account_scope: Option<String>,
}

impl ModelInfo {
    /// Recheck on every render/action: an in-memory catalog can outlive its proof
    /// or the account that originally passed the permission check.
    pub fn is_verified_for(&self, scope: Option<&str>) -> bool {
        self.is_verified_at(scope, chrono::Utc::now().timestamp_millis())
    }
    fn is_verified_at(&self, scope: Option<&str>, now: i64) -> bool {
        self.available
            && scope.is_some()
            && self.verified_account_scope.as_deref() == scope
            && self
                .verified_at_ms
                .is_some_and(|at| now >= at && now - at < MODEL_PROOF_TTL_MS)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillInstaller {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub installed: bool,
    pub enabled: bool,
    pub eligible: bool,
    pub installers: Vec<SkillInstaller>,
    pub source: String,
    pub install_reference: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToolInfo {
    pub id: String,
    pub name: String,
    pub group: String,
    pub source: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub model: String,
    pub source: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AiAppInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub installed: bool,
    pub installable: bool,
    pub source: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkCatalog {
    pub models: Vec<ModelInfo>,
    pub skills: Vec<SkillInfo>,
    pub agents: Vec<AgentInfo>,
    pub ai_apps: Vec<AiAppInfo>,
    pub models_status: CatalogAvailability,
    pub skills_status: CatalogAvailability,
    pub agents_status: CatalogAvailability,
    pub ai_apps_status: CatalogAvailability,
    /// Cloud Amoo sessions do not currently expose a verified model override API.
    pub can_select_model: bool,
    pub gateway_version: String,
    pub tools: Vec<ToolInfo>,
    pub tools_status: CatalogAvailability,
}

fn status(available: bool, message: impl Into<String>) -> CatalogAvailability {
    CatalogAvailability {
        available,
        message: message.into(),
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

#[cfg(test)]
fn model_ref(provider: &str, id: &str) -> String {
    if id.starts_with(&format!("{provider}/")) {
        id.into()
    } else {
        format!("{provider}/{id}")
    }
}

fn primary_model(value: &Value) -> String {
    value
        .as_str()
        .or_else(|| value["primary"].as_str())
        .unwrap_or("")
        .into()
}

/// Only consumes non-secret model metadata. The config object is never retained.
#[cfg(test)]
fn parse_models(payload: &Value, config: Option<&Value>) -> Result<Vec<ModelInfo>> {
    let rows = payload["models"].as_array().context("手机未返回模型目录")?;
    let current = config
        .map(|c| primary_model(&c["agents"]["defaults"]["model"]))
        .unwrap_or_default();
    let allowed = config.and_then(|c| c["agents"]["defaults"]["models"].as_object());
    let mut configured: std::collections::BTreeMap<String, Value> =
        std::collections::BTreeMap::new();
    if let Some(providers) = config.and_then(|c| c["models"]["providers"].as_object()) {
        for (provider, settings) in providers {
            if let Some(models) = settings["models"].as_array() {
                for model in models {
                    let id = text(model, "id");
                    if !id.is_empty() {
                        let mut row = model.clone();
                        row["provider"] = json!(provider);
                        configured.insert(model_ref(provider, id), row);
                    }
                }
            }
        }
    }
    let mut all: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    for row in rows {
        let (provider, id) = (text(row, "provider"), text(row, "id"));
        if provider.is_empty() || id.is_empty() {
            continue;
        }
        all.insert(model_ref(provider, id), row.clone());
    }
    // A custom phone proxy model need not occur in the upstream public catalog.
    for (id, row) in &configured {
        all.insert(id.clone(), row.clone());
    }
    let mut models = Vec::with_capacity(all.len());
    for (id, row) in all {
        let provider = text(&row, "provider").to_string();
        let configured_here = configured.contains_key(&id);
        let allowed_here =
            allowed.is_none_or(|models| models.is_empty() || models.contains_key(&id));
        let available = configured_here && allowed_here;
        let image_input = row["input"]
            .as_array()
            .is_some_and(|i| i.iter().any(|v| v == "image"));
        let mut capabilities = vec!["文字".into()];
        if image_input {
            capabilities.push("图片输入".into());
        }
        if row["reasoning"].as_bool() == Some(true) {
            capabilities.push("推理".into());
        }
        let reason = if config.is_none() {
            "未能读取手机模型配置，调用能力未确认"
        } else if !configured_here {
            "目录模型；手机尚未配置此模型的调用路由"
        } else if !allowed_here {
            "手机模型许可列表未启用此模型"
        } else {
            "手机 OpenClaw 已配置；云 Amoo 对话仍使用手机自动选择"
        };
        models.push(ModelInfo {
            current: id == current,
            id,
            name: text(&row, "name").to_string(),
            provider,
            capabilities,
            available,
            availability_reason: reason.into(),
            context_window: row["contextWindow"].as_u64(),
            image_input,
            source: SOURCE.into(),
            verified_at_ms: None,
            verified_account_scope: None,
        });
    }
    models.sort_by(|a, b| {
        b.available
            .cmp(&a.available)
            .then(b.current.cmp(&a.current))
            .then(a.provider.cmp(&b.provider))
            .then(a.name.cmp(&b.name))
            .then(a.id.cmp(&b.id))
    });
    Ok(models)
}

fn parse_skills(payload: &Value) -> Result<Vec<SkillInfo>> {
    let rows = payload["skills"].as_array().context("手机未返回技能目录")?;
    let mut seen = HashSet::new();
    let mut skills = Vec::new();
    for row in rows {
        let id = text(row, "skillKey");
        if id.is_empty() || !seen.insert(id.to_string()) {
            continue;
        }
        let installers = row["install"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let id = text(item, "id");
                        if id.is_empty() {
                            return None;
                        }
                        Some(SkillInstaller {
                            id: id.into(),
                            label: text(item, "label").into(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        skills.push(SkillInfo {
            id: id.into(),
            name: text(row, "name").into(),
            description: text(row, "description").into(),
            installed: true,
            enabled: row["disabled"].as_bool() != Some(true),
            eligible: row["eligible"].as_bool() == Some(true),
            installers,
            source: SOURCE.into(),
            install_reference: String::new(),
        });
    }
    Ok(skills)
}

fn phone_skills(ws: &mut GatewayWs, payload: &Value) -> Result<Vec<SkillInfo>> {
    let mut skills = parse_skills(payload)?;
    let rows = payload["skills"].as_array().context("手机未返回技能目录")?;
    // Origin enrichment is optional for old/other phone runtimes. Installed
    // state always comes from skills.status, not from cached market results.
    if let Ok(origins) = phone_skill_script(
        ws,
        json!({
            "action":"origins", "workspace":text(payload,"workspaceDir"),
            "files":rows.iter().map(|r| text(r,"filePath")).collect::<Vec<_>>()
        }),
    ) {
        for skill in &mut skills {
            if let Some(row) = rows.iter().find(|r| text(r, "skillKey") == skill.id) {
                let origin = &origins[text(row, "filePath")];
                if !text(origin, "reference").is_empty() {
                    skill.install_reference = text(origin, "reference").into();
                    skill.source = format!(
                        "ClawHub {} · {} · 已安装到手机",
                        skill.install_reference,
                        text(origin, "version")
                    );
                }
            }
        }
    }
    Ok(skills)
}

fn parse_agents(payload: &Value) -> Result<Vec<AgentInfo>> {
    Ok(payload["agents"]
        .as_array()
        .context("手机未返回智能体目录")?
        .iter()
        .filter_map(|row| {
            let id = text(row, "id");
            if id.is_empty() {
                return None;
            }
            let name = [text(row, "name"), text(&row["identity"], "name"), id]
                .into_iter()
                .find(|v| !v.is_empty())
                .unwrap_or(id);
            Some(AgentInfo {
                id: id.into(),
                name: name.into(),
                description: "手机 OpenClaw 智能体，与云 Amoo 会话独立".into(),
                model: primary_model(&row["model"]),
                source: SOURCE.into(),
            })
        })
        .collect())
}

fn connect(adb: &Adb, device: &Device) -> Result<GatewayWs> {
    GatewayWs::connect(adb, device)
}

fn populate_models(adb: &Adb, device: &Device, catalog: &mut WorkCatalog) {
    match verified_cloud_models(adb, device) {
        Ok(models) => {
            catalog.models_status = status(
                true,
                if models.is_empty() {
                    "当前账号尚无有效模型权限验证。点击验证模型，仅保留实际可调用的手机自动路由。"
                        .into()
                } else {
                    format!("已验证 {} 个手机 Amoo 云端路由；当前版本由手机自动选模，未开放指定模型接口。",models.len())
                },
            );
            catalog.models = models;
        }
        Err(_) => {
            catalog.models_status =
                status(false, "无法核实当前账号的模型权限，请连接手机后重新验证。")
        }
    }
}

pub fn fetch(adb: &Adb, device: &Device) -> Result<WorkCatalog> {
    // Catalog refresh never sends a model request. Explicit permission checks
    // and successful user turns populate account-scoped evidence below.
    match connect(adb, device) {
        Ok(mut ws) => fetch_with_ws(adb, device, &mut ws),
        Err(_) => {
            let mut catalog = WorkCatalog::default();
            populate_models(adb, device, &mut catalog);
            catalog.skills_status = status(false, "手机技能服务暂不可用");
            catalog.agents_status = status(false, "手机智能体服务暂不可用");
            catalog.tools_status = status(false, "手机工具服务暂不可用");
            Ok(catalog)
        }
    }
}

fn fetch_with_ws(adb: &Adb, device: &Device, ws: &mut GatewayWs) -> Result<WorkCatalog> {
    let mut catalog = WorkCatalog::default();
    populate_models(adb, device, &mut catalog);
    catalog.gateway_version = ws.gateway_version.clone();
    let skill_status = ws.call("skills.status", json!({}));
    match skill_status.and_then(|p| phone_skills(ws, &p)) {
        Ok(skills) => {
            catalog.skills_status = status(
                true,
                format!(
                    "手机 OpenClaw 已安装 {} 个技能。此目录与手机 Amoo 云技能市场独立。",
                    skills.len()
                ),
            );
            catalog.skills = skills;
        }
        Err(error) => catalog.skills_status = status(false, format!("技能读取失败：{error}")),
    }
    match ws
        .call("agents.list", json!({}))
        .and_then(|p| parse_agents(&p))
    {
        Ok(agents) => {
            catalog.agents_status = status(
                true,
                "手机 OpenClaw 智能体；不代表云 Amoo 当前使用的智能体。",
            );
            catalog.agents = agents;
        }
        Err(error) => catalog.agents_status = status(false, format!("智能体读取失败：{error}")),
    }
    catalog.ai_apps_status = status(
        false,
        "当前手机未开放 AI 应用目录和后台安装接口。可在 Studio 创建应用，或在应用管理中安装 APK。",
    );
    match ws.call("tools.catalog", json!({})) {
        Ok(value) => {
            if let Some(groups) = value["groups"].as_array() {
                for group in groups {
                    if let Some(tools) = group["tools"].as_array() {
                        for tool in tools {
                            if !text(tool, "id").is_empty() {
                                catalog.tools.push(ToolInfo {
                                    id: text(tool, "id").into(),
                                    name: text(tool, "label").into(),
                                    group: text(group, "label").into(),
                                    source: SOURCE.into(),
                                });
                            }
                        }
                    }
                }
                catalog.tools_status = status(
                    true,
                    "手机 OpenClaw 工具目录；实际执行还取决于模型、权限和服务配置。",
                );
            } else {
                catalog.tools_status = status(false, "手机未返回工具目录");
            }
        }
        Err(error) => catalog.tools_status = status(false, format!("工具目录暂不可用：{error}")),
    }
    Ok(catalog)
}

fn safe_registry_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && value != "."
        && value != ".."
}

fn parse_market(payload: &Value) -> Result<Vec<SkillInfo>> {
    let rows = payload["results"]
        .as_array()
        .context("手机未返回技能市场搜索结果")?;
    let mut skills = Vec::new();
    for row in rows {
        let owner = [text(row, "ownerHandle"), text(&row["owner"], "handle")]
            .into_iter()
            .find(|x| !x.is_empty())
            .unwrap_or("");
        let slug = text(row, "slug");
        if !safe_registry_component(owner) || !safe_registry_component(slug) {
            continue;
        }
        let reference = format!("@{owner}/{slug}");
        let installable =
            row["install"]["kind"] == "clawhub" && row["trust"]["installability"] == "installable";
        skills.push(SkillInfo {
            id: format!("clawhub:{reference}"),
            name: text(row, "displayName").into(),
            description: text(row, "summary").into(),
            installed: false,
            enabled: false,
            eligible: false,
            installers: if installable {
                vec![SkillInstaller {
                    id: "clawhub".into(),
                    label: format!("安装到手机 · {owner}"),
                }]
            } else {
                vec![]
            },
            source: format!("ClawHub · {owner} · 手机 OpenClaw"),
            install_reference: reference,
        });
    }
    Ok(skills)
}

pub fn search_skills(adb: &Adb, device: &Device, query: &str) -> Result<Vec<SkillInfo>> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 160 {
        bail!("请输入 1–160 字的技能关键词");
    }
    let mut ws = connect(adb, device)?;
    let mut market = parse_market(&ws.call("skills.search", json!({"query":query,"limit":40}))?)?;
    let status = ws.call("skills.status", json!({}))?;
    let installed = phone_skills(&mut ws, &status)?;
    for item in &mut market {
        if let Some(skill) = installed.iter().find(|skill| {
            !skill.install_reference.is_empty() && skill.install_reference == item.install_reference
        }) {
            item.id = skill.id.clone();
            item.installed = true;
            item.enabled = skill.enabled;
            item.eligible = skill.eligible;
            item.installers.clear();
            item.source = skill.source.clone();
        }
    }
    Ok(market)
}

fn market_identity(reference: &str) -> Result<(&str, &str)> {
    let (owner, slug) = reference
        .strip_prefix('@')
        .and_then(|r| r.split_once('/'))
        .context("市场技能缺少明确发布者")?;
    if !safe_registry_component(owner) || !safe_registry_component(slug) {
        bail!("市场技能引用无效");
    }
    Ok((owner, slug))
}

fn phone_skill_script(ws: &mut GatewayWs, args: Value) -> Result<Value> {
    let result = ws.call_with_timeout(
        "stepclaw.system.run",
        json!({"command":["python3","-c",include_str!("amoo_skills.py"),serde_json::to_string(&args)?],"timeoutMs":55000}),
        Duration::from_secs(60),
    ).context("手机技能处理结果未确认，请先刷新目录")?;
    let output: Value =
        serde_json::from_str(text(&result, "stdout")).context("手机未返回有效的技能处理结果")?;
    if result["exitCode"].as_i64() != Some(0) || output["ok"] == false {
        bail!("{}", output["error"].as_str().unwrap_or("手机技能处理失败"));
    }
    Ok(output)
}

#[derive(Clone, Debug, Deserialize)]
pub struct SkillTextFile {
    pub path: String,
    pub content: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct SkillBundle {
    pub name: String,
    pub files: Vec<SkillTextFile>,
    pub omitted: usize,
}

/// Reads the selected installed skill and bounded text references. Scripts are
/// not exported or executed; the desktop harness may use the instructions.
pub fn export_skill(adb: &Adb, device: &Device, id: &str) -> Result<SkillBundle> {
    let mut ws = connect(adb, device)?;
    let status = ws.call("skills.status", json!({}))?;
    let skill = status["skills"]
        .as_array()
        .context("手机未返回技能目录")?
        .iter()
        .find(|s| text(s, "skillKey") == id)
        .context("手机目录中没有此技能，请刷新")?;
    let output = phone_skill_script(
        &mut ws,
        json!({
            "action":"export", "name":text(skill,"name"), "filePath":text(skill,"filePath"),
            "allowedRoots":[text(&status,"managedSkillsDir"), format!("{}/skills",text(&status,"workspaceDir")), "/usr/local/lib/node_modules/openclaw/skills"]
        }),
    )?;
    serde_json::from_value(output).context("手机技能内容格式无效")
}

/// Explicit user operation on the phone runtime; verified by reading status again.
pub fn set_skill_enabled(
    adb: &Adb,
    device: &Device,
    id: &str,
    enabled: bool,
) -> Result<WorkCatalog> {
    let mut ws = connect(adb, device)?;
    let skills = parse_skills(&ws.call("skills.status", json!({}))?)?;
    if !skills.iter().any(|skill| skill.id == id) {
        bail!("技能不在手机当前目录中，请刷新后重试");
    }
    let response = ws.call("skills.update", json!({"skillKey":id,"enabled":enabled}))?;
    if response["ok"].as_bool() == Some(false) {
        bail!("手机未接受技能状态更改");
    }
    let after = parse_skills(&ws.call("skills.status", json!({}))?)?;
    if !after
        .iter()
        .any(|skill| skill.id == id && skill.enabled == enabled)
    {
        bail!("手机技能状态尚未确认，请刷新目录");
    }
    fetch_with_ws(adb, device, &mut ws)
}

/// Runs only an installer declared by this phone's skill status response.
pub fn install_skill(
    adb: &Adb,
    device: &Device,
    id: &str,
    installer_id: &str,
) -> Result<WorkCatalog> {
    let mut ws = connect(adb, device)?;
    let skills = parse_skills(&ws.call("skills.status", json!({}))?)?;
    if let Some(reference) = id.strip_prefix("clawhub:") {
        if installer_id != "clawhub" {
            bail!("技能安装方式与来源不匹配");
        }
        let (owner, slug) = market_identity(reference)?;
        let candidates =
            parse_market(&ws.call("skills.search", json!({"query":slug,"limit":100}))?)?;
        if !candidates.iter().any(|skill| {
            skill.install_reference == reference
                && skill.installers.iter().any(|i| i.id == "clawhub")
        }) {
            bail!("技能来源或可安装状态已变化，请重新搜索");
        }
        let status = ws.call("skills.status", json!({}))?;
        let result = phone_skill_script(
            &mut ws,
            json!({
                "action":"install", "owner":owner, "slug":slug,
                "workspace":text(&status,"workspaceDir"),
                "existingNames":skills.iter().map(|s| &s.name).collect::<Vec<_>>()
            }),
        )?;
        if result["ok"] != true || text(&result, "owner") != owner || text(&result, "slug") != slug
        {
            bail!("手机未确认正确来源的技能安装结果");
        }
        let installed = ws.call("skills.status", json!({}))?;
        if !installed["skills"].as_array().is_some_and(|rows| {
            rows.iter()
                .any(|s| text(s, "filePath") == text(&result, "filePath"))
        }) {
            bail!("手机文件已安装，但技能目录未加载，请刷新后确认");
        }
        return fetch_with_ws(adb, device, &mut ws);
    }
    let skill = skills
        .iter()
        .find(|s| s.id == id)
        .context("技能不在当前手机目录中")?;
    if !skill.installers.iter().any(|i| i.id == installer_id) {
        bail!("此安装方式并非手机技能声明的安装方式");
    }
    let result = ws
        .call_with_timeout(
            "skills.install",
            json!({"name":skill.name,"installId":installer_id,"timeoutMs":30000}),
            Duration::from_secs(40),
        )
        .context("技能安装结果尚未确认，请先刷新目录，不要重复安装")?;
    if result["ok"].as_bool() != Some(true) {
        bail!("技能安装未获得成功回执，请刷新目录确认");
    }
    fetch_with_ws(adb, device, &mut ws)
}

/// Create a new gateway agent only after an explicit user action. The workspace
/// is a phone path, never the desktop's selected working directory.
pub fn create_agent(adb: &Adb, device: &Device, name: &str) -> Result<WorkCatalog> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        bail!("请输入 1–80 字的智能体名称");
    }
    let mut ws = connect(adb, device)?;
    let before = parse_agents(&ws.call("agents.list", json!({}))?)?;
    if before.iter().any(|agent| agent.name == name) {
        bail!("同名智能体已存在");
    }
    let workspace = format!(
        "~/.openclaw/workspace-desktop-{:016x}",
        rand::random::<u64>()
    );
    let result = ws.call("agents.create", json!({"name":name,"workspace":workspace}))?;
    let id = text(&result, "agentId");
    if result["ok"].as_bool() != Some(true) || id.is_empty() {
        bail!("智能体创建未获得成功回执");
    }
    let after = parse_agents(&ws.call("agents.list", json!({}))?)?;
    if !after.iter().any(|agent| agent.id == id) {
        bail!("创建结果尚未出现在手机目录中，请刷新确认");
    }
    fetch_with_ws(adb, device, &mut ws)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unconfigured_catalog_does_not_imply_available() {
        let models = parse_models(&json!({"models":[{"id":"shared","name":"One","provider":"a"},{"id":"shared","name":"Two","provider":"b"}]}),Some(&json!({"models":{"providers":{"a":{"apiKey":"must-not-leak","models":[{"id":"shared","name":"Local"}]}}},"agents":{"defaults":{"model":"a/shared"}}}))).unwrap();
        assert_eq!(models.len(), 2);
        assert!(models[0].available && models[0].current);
        assert_eq!(models[0].id, "a/shared");
        assert!(!models[1].available);
        assert!(!serde_json::to_string(&models)
            .unwrap()
            .contains("must-not-leak"));
    }
    #[test]
    fn custom_configured_model_is_merged_and_allowlist_enforced() {
        let models = parse_models(&json!({"models":[]}),Some(&json!({"models":{"providers":{"local":{"models":[{"id":"auto","input":["text","image"]},{"id":"other"}]}}},"agents":{"defaults":{"models":{"local/auto":{}}}}}))).unwrap();
        assert_eq!(models.len(), 2);
        assert!(models[0].available && models[0].image_input);
        assert!(!models[1].available);
    }
    #[test]
    fn missing_config_never_claims_model_is_callable() {
        let models = parse_models(&json!({"models":[{"id":"x","provider":"p"}]}), None).unwrap();
        assert!(!models[0].available);
    }
    #[test]
    fn skill_installers_are_only_server_declared_choices() {
        let skills = parse_skills(&json!({"skills":[{"skillKey":"a","name":"Alpha","eligible":false,"disabled":true,"install":[{"id":"brew","label":"Install dependency"},{}]},{"skillKey":"b","name":"Beta","eligible":true}]})).unwrap();
        assert!(!skills[0].enabled);
        assert_eq!(skills[0].installers.len(), 1);
        assert!(skills[1].installed && skills[1].enabled && skills[1].installers.is_empty());
    }
    #[test]
    fn marketplace_never_drops_publisher_or_enables_untrusted_source() {
        let skills=parse_market(&json!({"results":[{"slug":"doc","ownerHandle":"writer","displayName":"Docs","install":{"kind":"clawhub"},"trust":{"installability":"installable"}},{"slug":"doc","ownerHandle":"someone-else","install":{"kind":"github"},"trust":{"installability":"installable"}},{"slug":"../escape","ownerHandle":"writer"}]})).unwrap();
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].id, "clawhub:@writer/doc");
        assert_eq!(skills[0].install_reference, "@writer/doc");
        assert_eq!(skills[0].installers.len(), 1);
        assert!(skills[1].installers.is_empty());
        assert!(!skills[0].installed);
    }
}

/// Permission evidence expires; refreshing a catalog never silently spends tokens.
const MODEL_PROOF_TTL_MS: i64 = 30 * 60 * 1000;
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ModelProof {
    account_scope: String,
    server_version: String,
    checked_at_ms: i64,
    model: ModelInfo,
}
fn model_proof_path(scope: &str) -> Result<std::path::PathBuf> {
    if scope.len() != 64 || !scope.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("模型验证账号标识无效")
    }
    Ok(crate::resources::user_state()
        .join("model-permissions")
        .join(format!("{scope}.json")))
}
fn proof_is_current(proof: &ModelProof, scope: &str, version: &str, now: i64) -> bool {
    proof.account_scope == scope
        && proof.model.is_verified_at(Some(scope), now)
        && proof.server_version == version
        && proof.model.available
        && !proof.model.id.is_empty()
        && now >= proof.checked_at_ms
        && now - proof.checked_at_ms < MODEL_PROOF_TTL_MS
}
fn save_model_proof(proof: &ModelProof) -> Result<()> {
    use std::io::Write;
    let path = model_proof_path(&proof.account_scope)?;
    let parent = path.parent().context("模型验证目录无效")?;
    std::fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&serde_json::to_vec(proof)?)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|_| anyhow::anyhow!("无法保存当前账号模型验证结果"))?;
    Ok(())
}

/// Read-only. A positive result belongs to this phone's currently logged-in
/// account, this server version, and a recent successful real inference.
pub fn verified_cloud_models(adb: &Adb, device: &Device) -> Result<Vec<ModelInfo>> {
    let Some(scope) = crate::amoo_usage::account_scope(adb, device)? else {
        return Ok(vec![]);
    };
    let path = model_proof_path(&scope)?;
    let Ok(bytes) = std::fs::read(path) else {
        return Ok(vec![]);
    };
    let Ok(proof) = serde_json::from_slice::<ModelProof>(&bytes) else {
        return Ok(vec![]);
    };
    let bus = crate::amoo::AgentBus::connect(adb, device)?;
    if proof_is_current(
        &proof,
        &scope,
        &bus.version,
        chrono::Utc::now().timestamp_millis(),
    ) {
        Ok(vec![proof.model])
    } else {
        Ok(vec![])
    }
}

pub fn invalidate_verified_models(adb: &Adb, device: &Device) -> Result<()> {
    if let Some(scope) = crate::amoo_usage::account_scope(adb, device)? {
        let path = model_proof_path(&scope)?;
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Caller must supply a new successful reply from the current live account;
/// arbitrary historical catalog entries are never permission evidence.
pub fn record_verified_cloud_model(
    adb: &Adb,
    device: &Device,
    model: &str,
    provider: &str,
    server_version: &str,
    context_limit: Option<u64>,
) -> Result<ModelInfo> {
    let scope = crate::amoo_usage::account_scope(adb, device)?
        .context("未读取到唯一的手机 Amoo 账号，无法保存模型权限")?;
    if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
        bail!("手机未返回有效的真实模型标识")
    }
    let verified = ModelInfo {
        id: format!("amoo-auto/{model}"),
        name: model.into(),
        provider: if provider.is_empty() {
            "Amoo".into()
        } else {
            provider.into()
        },
        capabilities: vec!["文字对话（已实测）".into()],
        current: true,
        available: true,
        availability_reason:
            "当前账号实际推理成功；跟随手机自动路由。验证有效期 30 分钟，可随时重新验证。".into(),
        context_window: context_limit,
        image_input: false,
        source: "手机 Amoo 云端（实际推理验证）".into(),
        verified_at_ms: Some(chrono::Utc::now().timestamp_millis()),
        verified_account_scope: Some(scope.clone()),
    };
    save_model_proof(&ModelProof {
        account_scope: scope,
        server_version: server_version.into(),
        checked_at_ms: chrono::Utc::now().timestamp_millis(),
        model: verified.clone(),
    })?;
    Ok(verified)
}

/// Explicit user-requested check: one minimal inference, no tools, one temporary
/// session. Invalid credentials, errors and timeouts never produce visible models.
pub fn verify_models(adb: &Adb, device: &Device) -> Result<Vec<ModelInfo>> {
    use std::time::Instant;
    let scope = crate::amoo_usage::account_scope(adb, device)?.context("请先登录手机 Amoo 账号")?;
    invalidate_verified_models(adb, device)?;
    let mut bus = crate::amoo::AgentBus::connect(adb, device)?;
    let key = bus.create_session()?;
    let result = (|| -> Result<(String, String, Option<u64>)> {
        let receipt = bus.send(&key, "模型权限验证：请只回复 OK，不要调用工具。")?;
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let history = bus.call_with_timeout(
                "chat.history",
                json!({"sessionId":key,"limit":20}),
                Duration::from_secs(5),
            )?;
            for row in history["messages"].as_array().into_iter().flatten() {
                if row["runId"] != receipt.run_id || row["role"] != "assistant" {
                    continue;
                }
                if matches!(
                    row["stopReason"].as_str(),
                    Some("error" | "ERROR" | "failed")
                ) {
                    bail!("当前账号的模型请求失败，未显示此模型")
                }
                if !matches!(
                    row["stopReason"].as_str(),
                    Some("STOP" | "stop" | "end_turn")
                ) || row["text"].as_str().is_none_or(|s| s.trim().is_empty())
                {
                    continue;
                }
                if crate::amoo_usage::account_scope(adb, device)?.as_deref() != Some(&scope) {
                    bail!("模型验证期间手机账号已改变，请重新验证")
                }
                let model = row["responseModel"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| row["model"].as_str())
                    .context("手机完成推理但未返回模型标识")?;
                let usage = crate::amoo_usage::parse_message(row);
                if usage.output_tokens.is_none_or(|tokens| tokens == 0) || row["isError"] == true {
                    bail!("手机未提供成功推理用量证据，未显示未经确认的模型")
                }
                return Ok((
                    model.to_string(),
                    text(row, "provider").to_string(),
                    usage.context_limit,
                ));
            }
            if Instant::now() >= deadline {
                bail!("模型验证超时，未显示未经确认的模型")
            }
            std::thread::sleep(Duration::from_millis(750));
        }
    })();
    let cleanup = bus.call("sessions.delete", json!({"key":key}));
    if cleanup.is_err() {
        invalidate_verified_models(adb, device)?;
        bail!("模型验证会话清理未确认，请刷新会话列表")
    }
    let (model, provider, context_limit) = result?;
    if crate::amoo_usage::account_scope(adb, device)?.as_deref() != Some(&scope) {
        bail!("模型验证期间手机账号已改变，请重新验证")
    }
    record_verified_cloud_model(adb, device, &model, &provider, &bus.version, context_limit)
        .map(|model| vec![model])
}

#[cfg(test)]
mod model_permission_tests {
    use super::*;
    #[test]
    fn proof_expires_and_never_crosses_account_or_server() {
        let p = ModelProof {
            account_scope: "a".into(),
            server_version: "1".into(),
            checked_at_ms: 100,
            model: ModelInfo {
                id: "model".into(),
                available: true,
                verified_at_ms: Some(100),
                verified_account_scope: Some("a".into()),
                ..Default::default()
            },
        };
        assert!(proof_is_current(&p, "a", "1", 101));
        assert!(!proof_is_current(&p, "b", "1", 101));
        assert!(!proof_is_current(&p, "a", "2", 101));
        assert!(!proof_is_current(&p, "a", "1", 99));
        assert!(!proof_is_current(&p, "a", "1", 100 + MODEL_PROOF_TTL_MS));
    }
    #[test]
    fn in_memory_catalog_must_recheck_account_and_age() {
        let model = ModelInfo {
            available: true,
            verified_at_ms: Some(100),
            verified_account_scope: Some("account-a".into()),
            ..Default::default()
        };
        assert!(model.is_verified_at(Some("account-a"), 101));
        assert!(!model.is_verified_at(None, 101));
        assert!(!model.is_verified_at(Some("account-b"), 101));
        assert!(!model.is_verified_at(Some("account-a"), 100 + MODEL_PROOF_TTL_MS));
        assert!(!ModelInfo {
            available: true,
            ..Default::default()
        }
        .is_verified_at(Some("account-a"), 101));
    }
    #[test]
    fn proof_paths_require_digest_and_never_use_raw_account_id() {
        assert!(model_proof_path("../secret").is_err());
        assert!(model_proof_path("user@example.com").is_err());
        assert!(model_proof_path(&"a".repeat(64)).is_ok());
    }
}
