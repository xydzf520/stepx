//! Verified HTML delivery and narrow Studio source synchronization. Website
//! credentials stay inside the phone's own WebView; local apps need no account.
use crate::{adb::Adb, device::Device, studio};
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;
const CHUNK_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalReceipt {
    pub id: String,
    pub name: String,
    pub sha256: String,
    pub size: usize,
    pub offline: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StudioModel {
    pub id: String,
    pub name: String,
    #[serde(rename = "isDefault", default)]
    pub is_default: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloudReceipt {
    pub project_id: String,
    pub editor_url: String,
    pub local_id: String,
    pub files: Vec<String>,
    pub model: String,
    #[serde(default)]
    pub commit: String,
}
fn local_id(id: &str) -> bool {
    id.strip_prefix("local-").is_some_and(|s| {
        s.len() == 32
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
fn project_id(id: &str) -> bool {
    id.starts_with("p_")
        && id.len() <= 128
        && id.len() > 2
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn hash(bytes: &[u8]) -> String {
    crate::device::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn validate_html(name: &str, bytes: &[u8]) -> Result<()> {
    ensure!(
        !name.trim().is_empty()
            && name.chars().count() <= 60
            && !name.chars().any(char::is_control),
        "作品名称需为 1–60 个字符"
    );
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_HTML_BYTES,
        "离线作品最大支持 2 MB"
    );
    let text = std::str::from_utf8(bytes)
        .context("作品需要使用 UTF-8 编码")?
        .to_ascii_lowercase();
    ensure!(
        text.contains("<html") && text.contains("</html>"),
        "请提供完整的 HTML 作品"
    );
    Ok(())
}
pub fn install_local(adb: &Adb, device: &Device, name: &str, html: &[u8]) -> Result<LocalReceipt> {
    validate_html(name, html)?;
    device.verify(adb)?;
    let digest = hash(html);
    let begin = studio::phone_call_unverified(
        adb,
        device,
        json!({"op":"local-begin","name":name.trim(),"size":html.len(),"sha256":digest}),
    )?;
    let upload = begin["upload"]
        .as_str()
        .filter(|s| local_id(s))
        .context("手机未返回有效上传编号")?
        .to_string();
    let result = (|| {
        let mut offset = 0;
        for chunk in html.chunks(CHUNK_BYTES) {
            let r = studio::phone_call_unverified(
                adb,
                device,
                json!({"op":"local-chunk","upload":upload,"offset":offset,"data":STANDARD.encode(chunk)}),
            )?;
            offset += chunk.len();
            ensure!(
                r["upload"] == upload && r["offset"].as_u64() == Some(offset as u64),
                "手机分片确认不一致"
            );
        }
        let raw = studio::phone_call_unverified(
            adb,
            device,
            json!({"op":"local-commit","upload":upload}),
        )?;
        let receipt: LocalReceipt = serde_json::from_value(raw).context("手机作品保存回执无效")?;
        ensure!(
            receipt.id == upload
                && receipt.sha256 == digest
                && receipt.size == html.len()
                && receipt.offline,
            "手机未通过作品完整性校验"
        );
        Ok(receipt)
    })();
    if result.is_err() {
        let _ =
            studio::phone_call_unverified(adb, device, json!({"op":"local-abort","upload":upload}));
    }
    result
}
pub fn local_status(adb: &Adb, device: &Device) -> Result<Value> {
    studio::phone_call(adb, device, json!({"op":"local-status"}))
}
pub fn local_list(adb: &Adb, device: &Device) -> Result<Vec<LocalReceipt>> {
    let raw = studio::phone_call(adb, device, json!({"op":"local-list"}))?;
    let all: Vec<LocalReceipt> = serde_json::from_value(raw["items"].clone())?;
    ensure!(
        all.len() <= 40
            && all
                .iter()
                .all(|r| local_id(&r.id) && r.offline && r.size <= MAX_HTML_BYTES),
        "手机作品列表无效"
    );
    Ok(all)
}
pub fn open_local(adb: &Adb, device: &Device, id: &str) -> Result<Value> {
    ensure!(local_id(id), "作品编号无效");
    device.verify(adb)?;
    let token = format!("{:032x}", rand::random::<u128>());
    let out = adb.shell(
        &device.transport,
        &[
            "am",
            "start",
            "-W",
            "-f",
            "0x14000000",
            "-n",
            "com.stepx.companion.app/.StudioLocalActivity",
            "--es",
            "id",
            id,
            "--es",
            "openToken",
            &token,
        ],
    )?;
    ensure!(
        out.contains("Status: ok") && !out.contains("Error:") && !out.contains("Exception"),
        "手机未能打开本地作品"
    );
    // A loaded document is not a functional-test verdict; the UI keeps the
    // distinction between delivery, page readiness and actual interaction QA.
    std::thread::sleep(std::time::Duration::from_millis(30));
    for _ in 0..40 {
        let status = studio::phone_call_unverified(adb, device, json!({"op":"local-status"}))?;
        if status["lastOpen"] == id
            && status["openToken"] == token
            && status["readyAt"].as_u64().unwrap_or(0) > 0
        {
            ensure!(
                status["errorCount"].as_u64().unwrap_or(0) == 0,
                "作品已保存，但手机页面运行异常；请返回电脑检查后重新发送"
            );
            return Ok(status);
        }
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
    bail!("手机已打开窗口，但作品页面尚未确认载入；请在手机检查")
}

fn scope_ok(scope: &str) -> bool {
    scope.len() == 64 && scope.bytes().all(|c| c.is_ascii_hexdigit())
}
fn cloud_phone_call(adb: &Adb, device: &Device, request: Value) -> Result<Value> {
    device.verify(adb)?;
    let encoded = STANDARD.encode(serde_json::to_vec(&request)?);
    ensure!(encoded.len() <= 32768, "官网操作请求过长");
    let command = [
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
    ]
    .iter()
    .map(|s| crate::adb::shell_quote(s))
    .collect::<Vec<_>>()
    .join(" ");
    let out = adb.run_timeout(
        &["-s", &device.transport, "shell", &command],
        std::time::Duration::from_secs(60),
    )?;
    let raw = out
        .split("result=")
        .nth(1)
        .context("手机尚不支持此 Studio 操作，请更新伴侣")?
        .split(|c: char| !c.is_ascii_alphanumeric() && !"+/=".contains(c))
        .next()
        .unwrap_or("");
    let result: Value = serde_json::from_slice(&STANDARD.decode(raw).context("官网操作回执无效")?)?;
    ensure!(
        result["ok"] == true,
        "{}",
        result["error"].as_str().unwrap_or("官网操作未完成")
    );
    Ok(result)
}
pub fn cloud_request(
    adb: &Adb,
    device: &Device,
    scope: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<String> {
    ensure!(scope_ok(scope), "官网账号未同步，请先连接 Studio");
    let mut q = json!({"op":"web-api","scope":scope,"method":method,"path":path});
    if let Some(body) = body {
        q["body"] = Value::String(serde_json::to_string(&body)?);
    }
    if method == "POST" && path.ends_with("/device-tests") {
        let id = format!("{:032x}", rand::random::<u128>());
        q["requestId"] = json!(format!(
            "{}-{}-{}-{}-{}",
            &id[..8],
            &id[8..12],
            &id[12..16],
            &id[16..20],
            &id[20..]
        ));
    }
    let raw = cloud_phone_call(adb, device, q)?;
    ensure!(
        raw["status"]
            .as_u64()
            .is_some_and(|s| (200..300).contains(&s)),
        "Studio 未确认该操作"
    );
    raw["text"]
        .as_str()
        .map(str::to_owned)
        .context("Studio 返回数据无效")
}
pub fn cloud_models(adb: &Adb, device: &Device, scope: &str) -> Result<Vec<StudioModel>> {
    let text = cloud_request(adb, device, scope, "GET", "/api/chat-models", None)?;
    let raw: Value = serde_json::from_str(&text)?;
    let models: Vec<StudioModel> = serde_json::from_value(raw["items"].clone())?;
    ensure!(
        !models.is_empty()
            && models.len() <= 100
            && models.iter().all(|m| !m.id.is_empty()
                && m.id.len() <= 160
                && !m.name.is_empty()
                && !m.id.chars().any(char::is_control)),
        "官网模型目录无效"
    );
    Ok(models)
}
pub fn cloud_set_model(
    adb: &Adb,
    device: &Device,
    scope: &str,
    project: &str,
    model: &str,
) -> Result<Value> {
    ensure!(project_id(project), "官网项目编号无效");
    ensure!(
        cloud_models(adb, device, scope)?
            .iter()
            .any(|m| m.id == model),
        "所选模型当前不在官网可用目录中"
    );
    cloud_request(
        adb,
        device,
        scope,
        "PATCH",
        &format!("/api/projects/{project}"),
        Some(json!({"model":model})),
    )?;
    let readback: Value = serde_json::from_str(&cloud_request(
        adb,
        device,
        scope,
        "GET",
        &format!("/api/projects/{project}"),
        None,
    )?)?;
    let actual = if readback["project"].is_object() {
        &readback["project"]
    } else {
        &readback
    };
    ensure!(
        actual["id"] == project && actual["model"] == model,
        "官网尚未确认所选模型，请重新同步项目"
    );
    Ok(readback)
}
pub fn cloud_files(adb: &Adb, device: &Device, scope: &str, project: &str) -> Result<Vec<String>> {
    ensure!(project_id(project), "官网项目编号无效");
    let text = cloud_request(
        adb,
        device,
        scope,
        "GET",
        &format!("/api/projects/{project}/files"),
        None,
    )?;
    ensure!(!text.trim_start().starts_with('<'), "官网没有返回源码目录");
    let paths: Vec<_> = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    ensure!(
        paths.len() <= 10000 && paths.iter().all(|p| valid_source_path(p)),
        "官网源码目录含无效路径"
    );
    Ok(paths)
}
pub fn valid_source_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 1024
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.contains(':')
        && !p.chars().any(char::is_control)
        && p.split('/').all(|x| x != ".." && x != ".")
}
fn fixed_app_package(project: &str, name: &str) -> String {
    let code = include_str!("../assets/studio-package-build.cjs");
    let build = format!("node -e {}", crate::adb::shell_quote(code));
    let preview = format!(
        "node -e {}",
        crate::adb::shell_quote(include_str!("../assets/studio-package-preview.cjs"))
    );
    json!({"name":"companion-local-app","version":"0.1.0","private":true,"scripts":{"build":build,"build:client":build,"dev":preview,"preview":preview},"companionManifest":{"applicationId":format!("ai_{project}"),"name":name,"version":"0.1.0","versionCode":1,"entry":"web/index.html","description":"本地验证后同步的离线 HTML 作品","orientation":"portrait","fullscreen":false,"permissions":[]}}).to_string()
}
fn cloud_source_hash(adb: &Adb, device: &Device, scope: &str, project: &str) -> Result<String> {
    let value = cloud_phone_call(
        adb,
        device,
        json!({"op":"web-api","scope":scope,"method":"GET","path":format!("/api/projects/{project}/file?path=index.html"),"responseDigest":true}),
    )?;
    ensure!(value["status"] == 200, "官网未返回源码文件");
    value["sha256"]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_owned)
        .context("官网未确认源码完整性，请更新伴侣后重试")
}
/// Save only Companion-created project edits; the phone broker enforces this
/// boundary. Studio device tests build a committed checkpoint, not dirty files.
pub fn cloud_save_source(adb: &Adb, device: &Device, scope: &str, project: &str) -> Result<String> {
    ensure!(project_id(project), "官网项目编号无效");
    let sessions: Value = serde_json::from_str(&cloud_request(
        adb,
        device,
        scope,
        "GET",
        &format!("/api/projects/{project}/sessions"),
        None,
    )?)?;
    let list = sessions["sessions"]
        .as_array()
        .context("官网未返回项目会话")?;
    let session = list
        .iter()
        .find(|s| s["status"] == "active" && s["branch"] == "")
        .or_else(|| if list.len() == 1 { list.first() } else { None })
        .and_then(|s| s["id"].as_str())
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .context("官网项目缺少明确的工作会话，请打开编辑器后重试")?;
    let command = |action: &str| -> Result<Value> {
        let id = format!("{:032x}", rand::random::<u128>());
        let request = format!(
            "{}-{}-{}-{}-{}",
            &id[..8],
            &id[8..12],
            &id[12..16],
            &id[16..20],
            &id[20..]
        );
        let value: Value = serde_json::from_str(&cloud_request(
            adb,
            device,
            scope,
            "POST",
            &format!("/api/projects/{project}/workspace/command"),
            Some(json!({"sessionId":session,"command":{"requestId":request,"action":action}})),
        )?)?;
        let result = &value["result"];
        ensure!(
            result["requestId"] == request
                && result["error"].as_str() == Some("")
                && result["status"]["dirty"].is_boolean()
                && result["status"]["busy"].is_boolean(),
            "官网未确认源码检查点操作"
        );
        Ok(result["status"].clone())
    };
    let before = command("status")?;
    ensure!(
        before["busy"] == false,
        "官网正在生成或保存版本，请稍后再同步"
    );
    if before["dirty"] == true {
        command("save_checkpoint")?;
    }
    let after = command("status")?;
    ensure!(
        after["busy"] == false && after["dirty"] == false,
        "源码已写入，但官网尚未确认保存的版本"
    );
    let sha = after["headSha"]
        .as_str()
        .filter(|s| matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .context("官网未返回保存版本编号")?;
    Ok(sha.into())
}
pub fn cloud_sync_new(
    adb: &Adb,
    device: &Device,
    scope: &str,
    name: &str,
    html: &[u8],
    model: &str,
) -> Result<CloudReceipt> {
    validate_html(name, html)?;
    ensure!(
        cloud_models(adb, device, scope)?
            .iter()
            .any(|m| m.id == model),
        "所选模型当前不在官网可用目录中"
    );
    let local = install_local(adb, device, name, html)?;
    let client_id = format!("companion-{:032x}", rand::random::<u128>());
    let raw = cloud_request(
        adb,
        device,
        scope,
        "POST",
        "/api/projects",
        Some(
            json!({"name":name.trim(),"projectType":"web","builderType":"app","productType":"app","model":model,"description":"在本机创作和验证的独立 HTML 作品","source":"unknown","clientRequestId":client_id}),
        ),
    )?;
    let created: Value = serde_json::from_str(&raw)?;
    let project = created["project"]["id"]
        .as_str()
        .filter(|s| project_id(s))
        .context("官网创建结果缺少项目编号，请刷新项目列表确认")?
        .to_string();
    let editor = format!("https://studio.stepx.com/builder/app/{project}");
    let sync = (|| {
        // Studio initializes a fresh app's repository/session when its official
        // editor connects. Reserving a sandbox alone leaves files at HTTP 503.
        // Reuse the explicit phone-session handoff, never ask the cloud model to
        // generate or replace this locally verified source.
        studio::open_editor(adb, device, &editor)?;
        cloud_request(
            adb,
            device,
            scope,
            "POST",
            &format!("/api/projects/{project}/sandbox/ensure"),
            None,
        )?;
        let mut ready = cloud_files(adb, device, scope, &project);
        for _ in 0..8 {
            if ready.is_ok() {
                break;
            }
            let retryable = ready
                .as_ref()
                .err()
                .is_some_and(|e| e.to_string().contains("HTTP 503"));
            if !retryable {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
            ready = cloud_files(adb, device, scope, &project);
        }
        let files = ready?;
        let bare = files
            .iter()
            .all(|p| matches!(p.as_str(), ".gitignore" | ".gitattributes"));
        // This function only reaches the project it just created. Existing
        // customer projects are never overwritten by local delivery.
        if !bare && !files.iter().any(|p| p == "index.html") {
            bail!("新项目使用了不同的源码模板，已保留项目；请从官网继续编辑");
        }
        if files.iter().any(|p| p == "index.html") {
            cloud_source_hash(adb, device, scope, &project)?;
        } else {
            cloud_request(
                adb,
                device,
                scope,
                "POST",
                &format!("/api/projects/{project}/file?path=index.html&kind=file"),
                None,
            )?;
        }
        let response = cloud_phone_call(
            adb,
            device,
            json!({"op":"web-api","scope":scope,"method":"PUT","path":format!("/api/projects/{project}/file?path=index.html"),"bodyLocalId":local.id}),
        )?;
        ensure!(
            response["status"]
                .as_u64()
                .is_some_and(|s| (200..300).contains(&s)),
            "官网未确认 HTML 写入"
        );
        let actual = cloud_source_hash(adb, device, scope, &project)?;
        ensure!(actual == local.sha256, "官网源码回读校验不一致");
        let mut written = vec!["index.html".into()];
        if bare {
            // A bare new app gets the fixed Studio package layout. Generated
            // HTML is copied as data; no model-supplied lifecycle commands.
            let package = fixed_app_package(&project, name);
            cloud_request(
                adb,
                device,
                scope,
                "POST",
                &format!("/api/projects/{project}/file?path=package.json&kind=file"),
                None,
            )?;
            let response = cloud_phone_call(
                adb,
                device,
                json!({"op":"web-api","scope":scope,"method":"PUT","path":format!("/api/projects/{project}/file?path=package.json"),"body":package}),
            )?;
            ensure!(
                response["status"]
                    .as_u64()
                    .is_some_and(|s| (200..300).contains(&s)),
                "官网未确认构建配置写入"
            );
            let actual = cloud_request(
                adb,
                device,
                scope,
                "GET",
                &format!("/api/projects/{project}/file?path=package.json"),
                None,
            )?;
            ensure!(actual == package, "官网构建配置回读校验不一致");
            written.push("package.json".into());
        }
        let commit = cloud_save_source(adb, device, scope, &project)?;
        Ok(CloudReceipt {
            project_id: project.clone(),
            editor_url: editor.clone(),
            local_id: local.id,
            files: written,
            model: model.into(),
            commit,
        })
    })();
    sync.with_context(|| format!("项目已创建，可继续编辑：{editor}"))
}
/// Submit an official test build. A ready/success receipt means that a test
/// package was built, not that its preview loaded or its phone UI was verified.
pub fn cloud_device_test(adb: &Adb, device: &Device, scope: &str, project: &str) -> Result<Value> {
    ensure!(project_id(project), "官网项目编号无效");
    let text = cloud_request(
        adb,
        device,
        scope,
        "POST",
        &format!("/api/projects/{project}/device-tests"),
        None,
    )?;
    let v: Value = serde_json::from_str(&text)?;
    ensure!(
        v["projectId"] == project
            && v["id"].is_string()
            && v["reviewStatus"].is_string()
            && v["syncStatus"].is_string(),
        "官网真机测试回执无效"
    );
    Ok(v)
}
pub fn cloud_test_status(adb: &Adb, device: &Device, scope: &str, project: &str) -> Result<Value> {
    ensure!(project_id(project), "官网项目编号无效");
    let text = cloud_request(
        adb,
        device,
        scope,
        "GET",
        &format!("/api/projects/{project}/device-tests"),
        None,
    )?;
    Ok(serde_json::from_str(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delivery_boundaries() {
        assert!(validate_html("计数器", b"<!doctype html><html><body>ok</body></html>").is_ok());
        assert!(validate_html("ok", &[0xff]).is_err());
        assert!(validate_html("\n", b"<html></html>").is_err());
        assert!(validate_html("ok", &vec![b'a'; MAX_HTML_BYTES + 1]).is_err());
        assert_eq!(
            hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        for id in ["../x", "local-a", "local-gggggggggggggggggggggggggggggggg"] {
            assert!(!local_id(id));
        }
    }
    #[test]
    fn source_paths_do_not_escape() {
        for p in ["index.html", "src/main.tsx", "assets/icon.png"] {
            assert!(valid_source_path(p));
        }
        for p in ["/etc/passwd", "../a", "x/../a", "C:/x", "x\\y", "a\n"] {
            assert!(!valid_source_path(p));
        }
    }
    #[test]
    fn chunk_request_fits_provider_limit() {
        let q = json!({"op":"local-chunk","upload":"local-0123456789abcdef0123456789abcdef","offset":MAX_HTML_BYTES,"data":STANDARD.encode(vec![0;CHUNK_BYTES])});
        assert!(STANDARD.encode(serde_json::to_vec(&q).unwrap()).len() < 32768);
    }
    #[test]
    fn app_name_cannot_change_cloud_build_command() {
        let normal: Value = serde_json::from_str(&fixed_app_package("p_a", "Hello")).unwrap();
        let quoted: Value =
            serde_json::from_str(&fixed_app_package("p_a", "'; echo surprise; #")).unwrap();
        assert_eq!(normal["scripts"], quoted["scripts"]);
        assert_eq!(quoted["companionManifest"]["name"], "'; echo surprise; #");
        assert!(normal.get("dependencies").is_none());
    }
}
