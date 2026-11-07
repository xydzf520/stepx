//! Studio handoff: official links, a local notebook, and a phone-owned library.
//! Phone account authorization is scoped to the official Amoo works API; it is
//! not a browser login and never exports an account token to the desktop.
use crate::{adb::Adb, device::Device};
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LinkKind {
    Experience,
    Project,
    Preview,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Work {
    pub id: String,
    pub name: String,
    pub url: String,
    pub category: String,
    pub kind: LinkKind,
    pub updated: i64,
}

/// Only the published link formats observed on the official site. Never forward
/// credentials, private builder query strings, arbitrary intents, or local URLs.
pub fn parse_link(raw: &str) -> Result<(String, LinkKind)> {
    let raw = raw.trim();
    ensure!(
        raw.len() <= 2048 && !raw.chars().any(|c| c.is_control() || c == '\\'),
        "链接格式无效"
    );
    let mut u = url::Url::parse(raw).context("请粘贴 Studio 的完整 HTTPS 链接")?;
    ensure!(
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.port().is_none(),
        "只支持官方 HTTPS 链接，不接受账号或非标准端口"
    );
    let host = u.host_str().unwrap_or("");
    ensure!(
        ["studio.stepx.com", "studio-api.stepx.com"].contains(&host),
        "请使用 studio.stepx.com 或 studio-api.stepx.com 的官方作品链接"
    );
    let parts: Vec<_> = u.path().trim_matches('/').split('/').collect();
    let ident = |s: &str| {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
    };
    let kind = match parts.as_slice() {
        ["t", id] if host == "studio-api.stepx.com" && ident(id) => LinkKind::Experience,
        ["builder", typ, id]
            if host == "studio.stepx.com"
                && ["app", "agent", "plugin"].contains(typ)
                && id.starts_with("p_")
                && ident(id) =>
        {
            LinkKind::Project
        }
        ["preview", id] if host == "studio.stepx.com" && id.starts_with("p_") && ident(id) => {
            LinkKind::Preview
        }
        _ => bail!("未识别此地址。请从作品的“真机测试 / 生成体验二维码”复制链接，或保存构建器地址"),
    };
    // Known navigation-only parameters are safe to discard. Reject auth/signature
    // parameters rather than silently converting a secret URL into a share card.
    ensure!(
        u.query_pairs().all(|(k, _)| k == "tab"),
        "链接含有授权或未知参数，请重新复制官方体验链接"
    );
    let canonical_path = u.path().trim_end_matches('/').to_owned();
    u.set_path(&canonical_path);
    u.set_query(None);
    u.set_fragment(None);
    Ok((u.to_string(), kind))
}
impl Work {
    pub fn new(name: &str, raw: &str, category: &str) -> Result<Self> {
        let name = name.trim();
        ensure!(
            !name.is_empty() && name.chars().count() <= 60 && !name.chars().any(char::is_control),
            "作品名称需为 1–60 个字符"
        );
        ensure!(
            ["应用", "智能体", "技能"].contains(&category),
            "作品类型无效"
        );
        let (url, kind) = parse_link(raw)?;
        Ok(Self {
            id: format!("studio-{:032x}", rand::random::<u128>()),
            name: name.into(),
            url,
            category: category.into(),
            kind,
            updated: chrono::Utc::now().timestamp(),
        })
    }
    pub fn shareable(&self) -> bool {
        self.kind == LinkKind::Experience
    }
}

pub fn directory() -> Result<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
    };
    let path = base
        .context("无法定位用户配置目录")?
        .join("stepx-companion/studio");
    std::fs::create_dir_all(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}
pub fn load() -> Result<Vec<Work>> {
    load_at(&directory()?.join("works.json"))
}
fn load_at(path: &Path) -> Result<Vec<Work>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    ensure!(
        std::fs::metadata(path)?.len() <= 1024 * 1024,
        "作品记录过大，未加载"
    );
    let works: Vec<Work> =
        serde_json::from_slice(&std::fs::read(path)?).context("作品记录损坏，已保留原文件")?;
    ensure!(works.len() <= 200, "作品超过 200 条");
    for w in &works {
        let checked = Work::new(&w.name, &w.url, &w.category)?;
        ensure!(
            checked.kind == w.kind && valid_id(&w.id),
            "作品记录无效，已保留原文件"
        );
    }
    Ok(works)
}
pub fn save(work: Work) -> Result<Vec<Work>> {
    save_at(&directory()?.join("works.json"), work)
}
pub fn save_all(works: &[Work]) -> Result<()> {
    let path = directory()?.join("works.json");
    let tmp = tempfile::NamedTempFile::new_in(path.parent().context("作品目录无效")?)?;
    std::fs::write(tmp.path(), serde_json::to_vec_pretty(works)?)?;
    tmp.as_file().sync_all()?;
    tmp.persist(&path).map_err(|e| e.error)?;
    Ok(())
}
fn save_at(path: &Path, mut work: Work) -> Result<Vec<Work>> {
    let mut all = load_at(path)?;
    if let Some(old) = all.iter().find(|w| w.url == work.url) {
        work.id = old.id.clone();
    }
    all.retain(|w| w.url != work.url);
    ensure!(all.len() < 200, "作品库已达 200 条，请先整理历史记录");
    all.insert(0, work);
    let tmp = tempfile::NamedTempFile::new_in(path.parent().context("作品目录无效")?)?;
    std::fs::write(tmp.path(), serde_json::to_vec_pretty(&all)?)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(all)
}
pub fn qr(work: &Work) -> Result<PathBuf> {
    ensure!(
        valid_id(&work.id) && parse_link(&work.url)?.1 == LinkKind::Experience,
        "无效的分享作品"
    );
    ensure!(
        work.shareable(),
        "编辑地址需要账号访问，不能作为分享二维码。请先生成官方体验链接"
    );
    let code = qrcode::QrCode::new(work.url.as_bytes())?;
    let quiet = 4;
    let scale = 8;
    let width = code.width();
    let mut img = image::RgbImage::from_pixel(
        ((width + quiet * 2) * scale) as u32,
        ((width + quiet * 2) * scale) as u32,
        image::Rgb([255, 255, 255]),
    );
    for y in 0..width {
        for x in 0..width {
            if code[(x, y)] == qrcode::Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        img.put_pixel(
                            ((x + quiet) * scale + dx) as u32,
                            ((y + quiet) * scale + dy) as u32,
                            image::Rgb([24, 24, 27]),
                        );
                    }
                }
            }
        }
    }
    let path = directory()?.join(format!("share-{}.png", work.id));
    let tmp = tempfile::NamedTempFile::new_in(path.parent().context("二维码目录无效")?)?;
    img.save_with_format(tmp.path(), image::ImageFormat::Png)?;
    tmp.persist(&path).map_err(|e| e.error)?;
    Ok(path)
}
fn valid_id(id: &str) -> bool {
    id.strip_prefix("studio-")
        .is_some_and(|v| v.len() == 32 && v.bytes().all(|c| c.is_ascii_hexdigit()))
}

pub fn phone_call(adb: &Adb, device: &Device, request: Value) -> Result<Value> {
    device.verify(adb)?;
    phone_call_unverified(adb, device, request)
}

pub(crate) fn phone_call_unverified(adb: &Adb, device: &Device, request: Value) -> Result<Value> {
    let encoded = STANDARD.encode(serde_json::to_vec(&request)?);
    ensure!(encoded.len() <= 32768, "作品请求过长");
    let out = adb.shell(
        &device.transport,
        &[
            "content",
            "call",
            "--user",
            "0",
            "--uri",
            "content://com.stepx.companion.app.studio",
            "--method",
            "request",
            "--arg",
            &encoded,
        ],
    )?;
    let raw = out
        .split("result=")
        .nth(1)
        .context("手机尚不支持作品库，请从小工具安装新版 StepX伴侣")?
        .split(|c: char| !c.is_ascii_alphanumeric() && !"+/=".contains(c))
        .next()
        .unwrap_or("");
    let result: Value = serde_json::from_slice(&STANDARD.decode(raw).context("手机作品回执无效")?)?;
    ensure!(
        result["ok"] == true,
        "{}",
        result["error"].as_str().unwrap_or("手机操作未完成")
    );
    Ok(result)
}
pub fn send(adb: &Adb, device: &Device, work: &Work) -> Result<Value> {
    ensure!(work.shareable(), "请发送官方体验链接；编辑地址仅保存在电脑，技能和智能体请在官方“真机测试”后到 Amoo“我的作品”安装");
    let receipt = phone_call(adb, device, json!({"op":"save", "work":work}))?;
    ensure!(
        receipt["saved"] == work.id && receipt["url"] == work.url,
        "手机未确认保存此作品"
    );
    Ok(receipt)
}
pub fn open_phone(adb: &Adb, device: &Device, mode: &str, id: Option<&str>) -> Result<String> {
    device.verify(adb)?;
    let mut args = vec!["am", "start", "-W", "-f", "0x14000000"];
    match mode {
        "library" => args.extend(["-n", "com.stepx.companion.app/.StudioActivity"]),
        "website" => args.extend(["-n", "com.stepx.companion.app/.StudioWebActivity"]),
        "account" => args.extend(["-n", "com.stepx.companion.app/.StudioAccountActivity"]),
        "open" | "pin" => args.extend([
            "-n",
            "com.stepx.companion.app/.StudioActivity",
            "--es",
            "op",
            mode,
            "--es",
            "id",
            id.context("缺少作品编号")?,
        ]),
        "amoo" => args.extend([
            "-a",
            "android.intent.action.VIEW",
            "-d",
            "stepos://amoo/testspace",
        ]),
        _ => bail!("未知手机操作"),
    }
    let out = adb.shell(&device.transport, &args)?;
    ensure!(
        !out.contains("Error:") && !out.contains("Exception") && out.contains("Status: ok"),
        "手机未打开入口：{}",
        out.trim()
    );
    Ok(match mode {
        "account" => "已打开手机账号授权，完成后自动同步作品",
        "website" => "已打开手机 Studio 官方登录，完成后自动同步官网项目",
        "pin" => "已打开手机桌面图标确认，请在手机上完成添加",
        "amoo" => "已打开 Amoo 我的作品，请查看测试产物并确认安装",
        "open" => "已发送打开请求；网页是否可用以手机画面为准",
        _ => "已打开手机作品库，断开电脑后仍可使用",
    }
    .into())
}

#[derive(Clone, Debug, Deserialize)]
pub struct CloudWork {
    pub id: String,
    pub name: String,
    pub category: String,
    pub url: String,
    pub description: String,
    pub version: String,
    pub installed: i64,
    pub updated: String,
    #[serde(default)]
    pub source: String,
}
impl CloudWork {
    pub fn experience(&self) -> Option<Work> {
        let work = Work::new(&self.name, &self.url, &self.category).ok()?;
        work.shareable().then_some(work)
    }
    pub fn editor(&self) -> Option<String> {
        if !self.id.starts_with("p_") {
            return None;
        }
        let kind = match self.category.as_str() {
            "应用" => "app",
            "智能体" => "agent",
            "技能" => "plugin",
            _ => return None,
        };
        let url = format!("https://studio.stepx.com/builder/{kind}/{}", self.id);
        parse_link(&url).ok().map(|v| v.0)
    }
}
#[derive(Clone, Debug, Default)]
pub struct CloudLibrary {
    pub authorized: bool,
    pub scope: String,
    pub items: Vec<CloudWork>,
    pub totals: [usize; 3],
    pub synced_at: i64,
    pub name: String,
}
pub fn open_editor(adb: &Adb, device: &Device, target: &str) -> Result<String> {
    let (url, kind) = parse_link(target)?;
    ensure!(kind == LinkKind::Project, "请选择官网项目编辑入口");
    let mut handoff = phone_call(adb, device, json!({"op":"web-session","url":url}))?;
    let result = crate::studio_browser::open_editor(&url, &handoff);
    // Do not retain session material in UI state, files, or receipts.
    if let Some(cookies) = handoff.get_mut("cookies").and_then(Value::as_array_mut) {
        for c in cookies {
            c["value"] = json!("");
        }
    }
    result?;
    Ok("已验证手机账号并打开对应 Studio 项目".into())
}

pub fn sync_web(adb: &Adb, device: &Device) -> Result<CloudLibrary> {
    parse_web_snapshot(phone_call(adb, device, json!({"op":"web-snapshot"}))?)
}
fn parse_web_snapshot(value: Value) -> Result<CloudLibrary> {
    if value["authorized"] != true {
        return Ok(CloudLibrary::default());
    }
    let scope = value["scope"]
        .as_str()
        .context("官网账号标识缺失")?
        .to_owned();
    ensure!(
        scope.len() == 64 && scope.bytes().all(|b| b.is_ascii_hexdigit()),
        "官网账号标识无效"
    );
    let items: Vec<CloudWork> =
        serde_json::from_value(value["items"].clone()).context("官网项目格式无效")?;
    ensure!(items.len() <= 500, "官网项目数量过大");
    let mut totals = [0; 3];
    let mut seen = std::collections::HashSet::new();
    for item in &items {
        let i = ["应用", "智能体", "技能"]
            .iter()
            .position(|c| *c == item.category)
            .context("官网项目类型无效")?;
        ensure!(
            item.editor().is_some()
                && item.source == "website"
                && item.name.chars().count() <= 120
                && item.description.len() <= 6000
                && item.url.is_empty(),
            "官网项目字段无效"
        );
        ensure!(seen.insert(&item.id), "官网返回重复项目");
        totals[i] += 1;
    }
    Ok(CloudLibrary {
        authorized: true,
        scope,
        items,
        totals,
        synced_at: chrono::Utc::now().timestamp(),
        name: value["name"]
            .as_str()
            .unwrap_or("Studio 账号")
            .chars()
            .take(80)
            .collect(),
    })
}

pub fn sync_cloud(adb: &Adb, device: &Device) -> Result<CloudLibrary> {
    device.verify(adb)?;
    sync_cloud_with(|request| phone_call_unverified(adb, device, request))
}
fn sync_cloud_with(mut call: impl FnMut(Value) -> Result<Value>) -> Result<CloudLibrary> {
    let status = call(json!({"op":"cloud-status"}))?;
    if status["authorized"] != true {
        return Ok(CloudLibrary::default());
    }
    let scope = status["scope"]
        .as_str()
        .context("手机账号标识缺失")?
        .to_owned();
    ensure!(
        scope.len() == 64 && scope.bytes().all(|b| b.is_ascii_hexdigit()),
        "手机账号标识无效"
    );
    let mut library = CloudLibrary {
        authorized: true,
        scope: scope.clone(),
        ..Default::default()
    };
    for (i, category) in ["应用", "智能体", "技能"].iter().enumerate() {
        let mut seen = std::collections::HashSet::new();
        for page in 1..=4 {
            let value = call(json!({"op":"cloud-page","category":category,"page":page}))?;
            ensure!(
                value["scope"] == scope && value["page"] == page,
                "同步期间手机账号已变化，请重试"
            );
            let total = value["total"].as_u64().context("作品数量缺失")? as usize;
            let items: Vec<CloudWork> =
                serde_json::from_value(value["items"].clone()).context("手机作品格式无效")?;
            ensure!(items.len() <= 50, "手机作品页过大");
            library.totals[i] = total;
            for item in items {
                ensure!(
                    item.category == *category
                        && !item.id.is_empty()
                        && item.id.len() <= 200
                        && item.name.chars().count() <= 120
                        && item.description.len() <= 6000,
                    "作品字段无效"
                );
                if seen.insert(item.id.clone()) {
                    library.items.push(item);
                }
            }
            if seen.len() >= total {
                break;
            }
            ensure!(
                seen.len() >= page as usize * 50,
                "官网作品分页不完整，请刷新重试"
            );
        }
    }
    let final_status = call(json!({"op":"cloud-status"}))?;
    ensure!(
        final_status["authorized"] == true && final_status["scope"] == scope,
        "手机账号已断开或切换，未保留同步结果"
    );
    library.synced_at = chrono::Utc::now().timestamp();
    Ok(library)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfficialGuiSkillStatus {
    pub scope: String,
    pub candidate_id: String,
    pub candidate_name: String,
    pub candidate_version: String,
    pub installed: bool,
}

pub fn new_gui_skill_request_id() -> String {
    let raw = rand::random::<u128>();
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        (raw >> 96) as u32,
        ((raw >> 80) & 0xffff) as u16,
        ((raw >> 64) & 0xffff) as u16,
        ((raw >> 48) & 0xffff) as u16,
        (raw & 0xffff_ffff_ffff) as u64,
    )
}

pub fn parse_gui_skill_status(value: &Value) -> Result<OfficialGuiSkillStatus> {
    let scope = value["scope"]
        .as_str()
        .context("手机账号标识缺失")?
        .to_owned();
    ensure!(
        scope.len() == 64 && scope.bytes().all(|b| b.is_ascii_hexdigit()),
        "手机账号标识无效"
    );
    let candidate = &value["candidate"];
    let candidate_id = candidate["id"].as_str().unwrap_or_default().to_owned();
    let candidate_name = candidate["name"].as_str().unwrap_or_default().to_owned();
    let candidate_version = candidate["version"].as_str().unwrap_or_default().to_owned();
    let install_failed = value.get("install").is_some_and(|ack| {
        ack["code"].as_i64() != Some(0) || ack["http"].as_i64().unwrap_or(200) != 200
    });
    let installed_skills = value["installedState"]["skills"]
        .as_array()
        .or_else(|| value["after"]["skills"].as_array());
    let installed = !install_failed
        && (installed_skills.is_some_and(|skills| {
            skills.iter().any(|s| {
                let sid = s["id"].as_str().unwrap_or("");
                let sname = s["name"].as_str().unwrap_or("");
                let sver = s["installVersion"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .or_else(|| s["version"].as_str().filter(|v| !v.is_empty()));
                let version_matches = match sver {
                    Some(v) if !candidate_version.is_empty() => v == candidate_version,
                    _ => true,
                };
                let identity_matches = (!candidate_id.is_empty() && sid == candidate_id)
                    || (!candidate_name.is_empty() && sname == candidate_name)
                    || sname.contains("GUI")
                    || sname.contains("手机屏幕");
                identity_matches && version_matches
            })
        }) || candidate["installed"].as_i64().unwrap_or(0) > 0);
    Ok(OfficialGuiSkillStatus {
        scope,
        candidate_id,
        candidate_name,
        candidate_version,
        installed,
    })
}

pub fn gui_skill_status(adb: &Adb, device: &Device) -> Result<OfficialGuiSkillStatus> {
    let raw = phone_call(adb, device, json!({"op": "gui-skill-status"}))?;
    parse_gui_skill_status(&raw)
}

pub fn gui_skill_install_with(
    mut call: impl FnMut(Value) -> Result<Value>,
) -> Result<OfficialGuiSkillStatus> {
    let status_val = call(json!({"op": "gui-skill-status"}))?;
    let status = parse_gui_skill_status(&status_val)?;
    if status.installed {
        return Ok(status);
    }
    ensure!(
        !status.candidate_id.is_empty() && !status.candidate_version.is_empty(),
        "官方技能市场未找到可安装的手机屏幕操控技能候选包"
    );
    let request_id = new_gui_skill_request_id();
    let install_val = call(json!({
        "op": "gui-skill-install",
        "scope": status.scope,
        "version": status.candidate_version,
        "testing": true,
        "requestId": request_id,
    }))?;
    if let Some(ack) = install_val.get("install") {
        let code = ack["code"].as_i64().unwrap_or(-1);
        let http = ack["http"].as_i64().unwrap_or(200);
        ensure!(
            code == 0 && http == 200,
            "官方技能安装未完成（HTTP {http}，代码 {code}）：{}",
            ack["message"].as_str().unwrap_or("未确认安装成功")
        );
    }
    let mut after = parse_gui_skill_status(&install_val)?;
    ensure!(
        after.scope == status.scope,
        "手机账号已变化，未确认技能安装结果"
    );
    if install_val["install"]["code"].as_i64() == Some(0)
        && install_val["install"]["http"].as_i64().unwrap_or(200) == 200
    {
        after.installed = true;
    }
    ensure!(after.installed, "官方技能市场安装未生效");
    Ok(after)
}

pub fn gui_skill_install(adb: &Adb, device: &Device) -> Result<OfficialGuiSkillStatus> {
    device.verify(adb)?;
    gui_skill_install_with(|request| phone_call_unverified(adb, device, request))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn website_sync_keeps_three_builders_and_rejects_invalid_catalogs() {
        let item = |id: &str, category: &str| json!({"id":id,"name":"草稿","category":category,"url":"","description":"","version":"","installed":-1,"updated":"","source":"website"});
        let snapshot = json!({"authorized":true,"scope":"a".repeat(64),"name":"测试","items":[item("p_app","应用"),item("p_agent","智能体"),item("p_skill","技能")]});
        let library = parse_web_snapshot(snapshot.clone()).unwrap();
        assert_eq!(library.totals, [1, 1, 1]);
        assert!(library.items[1]
            .editor()
            .unwrap()
            .contains("/builder/agent/"));
        assert!(library.items[2]
            .editor()
            .unwrap()
            .contains("/builder/plugin/"));
        assert!(library.items.iter().all(|p| p.experience().is_none()));
        let mut duplicate = snapshot.clone();
        duplicate["items"][1] = duplicate["items"][0].clone();
        assert!(parse_web_snapshot(duplicate).is_err());
        let mut bad = snapshot.clone();
        bad["items"][0]["id"] = json!("p_x/../../login");
        assert!(parse_web_snapshot(bad).is_err());
        let mut secret = snapshot;
        secret["items"][0]["url"] = json!("https://studio.stepx.com/?token=secret");
        assert!(parse_web_snapshot(secret).is_err());
        let off = parse_web_snapshot(json!({"authorized":false})).unwrap();
        assert!(!off.authorized && off.items.is_empty());
    }

    #[test]
    fn cloud_account_change_and_partial_catalog_fail_closed() {
        let scope = "a".repeat(64);
        let mut n = 0;
        assert!(sync_cloud_with(|q| {
            n += 1;
            if q["op"] == "cloud-status" {
                return Ok(
                    json!({"authorized":true,"scope":if n==1 {scope.clone()}else{"b".repeat(64)}}),
                );
            }
            Ok(json!({"scope":scope,"page":1,"total":0,"items":[]}))
        })
        .is_err());
        assert!(sync_cloud_with(|q| {
            if q["op"] == "cloud-status" {
                return Ok(json!({"authorized":true,"scope":scope}));
            }
            Ok(json!({"scope":scope,"page":1,"total":51,"items":[]}))
        })
        .is_err());
    }
    #[test]
    fn cloud_sync_pages_and_preserves_real_empty_categories() {
        let scope = "a".repeat(64);
        let mut pages = vec![];
        let result=sync_cloud_with(|q| {
            if q["op"]=="cloud-status" {return Ok(json!({"authorized":true,"scope":scope}));}
            let page=q["page"].as_u64().unwrap();let app=q["category"]=="应用";
            let items:Vec<_>=if app {(if page==1 {0..50}else{50..51}).map(|i|json!({"id":format!("p_{i}"),"name":"作品","category":"应用","url":"","description":"","version":"1","installed":0,"updated":""})).collect()} else{vec![]};
            pages.push((q["category"].clone(),page));
            Ok(json!({"scope":scope,"page":page,"total":if app{51}else{0},"items":items}))
        }).unwrap();
        assert_eq!(result.items.len(), 51);
        assert_eq!(result.totals, [51, 0, 0]);
        assert_eq!(pages.len(), 4);
        let mut called = 0;
        let empty = sync_cloud_with(|_| {
            called += 1;
            Ok(json!({"authorized":false}))
        })
        .unwrap();
        assert_eq!(called, 1);
        assert!(!empty.authorized);
        assert!(empty.items.is_empty());
    }
    #[test]
    fn official_link_boundary() {
        for raw in [
            "javascript:alert(1)",
            "https://studio-api.stepx.com.evil/t/abc",
            "https://x@studio-api.stepx.com/t/abc",
            "http://studio-api.stepx.com/t/abc",
            "https://studio-api.stepx.com/t/abc?token=secret",
            "https://studio-api.stepx.com:9000/t/abc",
            "https://studio.stepx.com/app/fake",
            "https://studio-api.stepx.com/t/a%2fb",
        ] {
            assert!(parse_link(raw).is_err(), "{raw}");
        }
        assert_eq!(
            parse_link("https://studio-api.stepx.com/t/test123")
                .unwrap()
                .1,
            LinkKind::Experience
        );
        let (url, kind) =
            parse_link("https://studio.stepx.com/builder/plugin/p_abc?tab=preview").unwrap();
        assert_eq!(kind, LinkKind::Project);
        assert!(!url.contains('?'));
    }
    #[test]
    fn upsert_preserves_identity_and_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("works.json");
        let a = Work::new("作品", "https://studio-api.stepx.com/t/test123", "应用").unwrap();
        let id = a.id.clone();
        save_at(&path, a).unwrap();
        let b = Work::new("改名", "https://studio-api.stepx.com/t/test123", "应用").unwrap();
        let all = save_at(&path, b.clone()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);
        std::fs::write(&path, b"broken").unwrap();
        assert!(save_at(&path, b).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
    }

    #[test]
    fn gui_skill_status_and_install_with_studio_provider_schema() {
        let scope = "f".repeat(64);
        let mut ops = Vec::new();
        let installed = gui_skill_install_with(|q| {
            let op = q["op"].as_str().unwrap_or("").to_string();
            ops.push(op.clone());
            if op == "gui-skill-status" {
                Ok(json!({
                    "ok": true,
                    "scope": scope,
                    "candidate": {
                        "id": "802",
                        "name": "stepx_gui_official",
                        "version": "1.2.0",
                        "installed": 0
                    },
                    "installedState": {
                        "http": 200,
                        "code": 0,
                        "skills": [{"id": "802", "name": "stepx_gui_official", "version": "1.0.0"}]
                    }
                }))
            } else {
                assert_eq!(op, "gui-skill-install");
                assert_eq!(q["scope"], scope);
                assert_eq!(q["version"], "1.2.0");
                assert_eq!(q["testing"], true);
                let req_id = q["requestId"].as_str().unwrap();
                let parts: Vec<&str> = req_id.split('-').collect();
                assert_eq!(
                    parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
                    vec![8, 4, 4, 4, 12],
                    "requestId must match OfficialGuiSkillPolicy UUID format: {req_id}"
                );
                assert!(parts
                    .iter()
                    .all(|p| p.bytes().all(|b| b.is_ascii_hexdigit())));
                Ok(json!({
                    "ok": true,
                    "scope": scope,
                    "candidate": {
                        "id": "802",
                        "name": "stepx_gui_official",
                        "version": "1.2.0",
                        "installed": 1
                    },
                    "install": { "http": 200, "code": 0 },
                    "after": {
                        "http": 200,
                        "code": 0,
                        "skills": [{"id": "802", "name": "stepx_gui_official", "version": "1.2.0"}]
                    }
                }))
            }
        })
        .unwrap();

        assert_eq!(ops, vec!["gui-skill-status", "gui-skill-install"]);
        assert!(installed.installed);
        assert_eq!(installed.candidate_id, "802");
        assert_eq!(installed.candidate_version, "1.2.0");

        // Reject when candidate.version is empty
        let empty_version_err = gui_skill_install_with(|_| {
            Ok(json!({
                "ok": true,
                "scope": scope,
                "candidate": { "id": "802", "name": "stepx_gui_official", "version": "", "installed": 0 },
                "installedState": { "http": 200, "code": 0, "skills": [] }
            }))
        });
        assert!(empty_version_err.is_err());

        // Reject when install.code is non-zero even if after.skills still lists a stale skill
        let failed_install_err = gui_skill_install_with(|q| {
            if q["op"] == "gui-skill-status" {
                Ok(json!({
                    "ok": true,
                    "scope": scope,
                    "candidate": { "id": "802", "name": "stepx_gui_official", "version": "1.2.0", "installed": 0 },
                    "installedState": { "http": 200, "code": 0, "skills": [] }
                }))
            } else {
                Ok(json!({
                    "ok": false,
                    "scope": scope,
                    "candidate": { "id": "802", "name": "stepx_gui_official", "version": "1.2.0", "installed": 0 },
                    "install": { "http": 500, "code": -1, "message": "server error" },
                    "after": {
                        "http": 200,
                        "code": 0,
                        "skills": [{"id": "802", "name": "stepx_gui_official", "version": "1.0.0"}]
                    }
                }))
            }
        });
        assert!(failed_install_err.is_err());
    }
}
