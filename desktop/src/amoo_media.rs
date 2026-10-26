//! Native Amoo media transport and verified deliverables.
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::{fs, io::Read, path::PathBuf};
/// Maximum total attachment bytes in one native request (base64 fits its 16 MiB frame).
pub const MAX_ATTACHMENT_BYTES: u64 = 8 * 1024 * 1024;
pub fn mime_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        "csv" => "text/csv",
        "txt" | "md" => "text/plain",
        "json" => "application/json",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}
pub fn prepare_attachments(paths: &[PathBuf]) -> Result<Vec<Value>> {
    if paths.len() > 8 {
        bail!("每次最多上传 8 个附件")
    }
    let mut total = 0u64;
    let mut out = Vec::new();
    for path in paths {
        let meta =
            fs::metadata(path).with_context(|| format!("无法读取附件 {}", path.display()))?;
        if !meta.is_file() {
            bail!("附件必须是普通文件")
        }
        total = total.checked_add(meta.len()).context("附件过大")?;
        if total > MAX_ATTACHMENT_BYTES {
            bail!("每次附件总大小最多 8 MiB")
        }
        let mut data = Vec::new();
        fs::File::open(path)?
            .take(MAX_ATTACHMENT_BYTES + 1)
            .read_to_end(&mut data)?;
        if data.len() as u64 != meta.len() {
            bail!("附件已变化，请重新选择")
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .context("附件名称必须为 UTF-8")?;
        let mime = mime_type(path);
        let mut dimensions = None;
        if mime.starts_with("image/") {
            let (format, actual) = match image::guess_format(&data) {
                Ok(fmt @ image::ImageFormat::Png) => (fmt, "image/png"),
                Ok(fmt @ image::ImageFormat::Jpeg) => (fmt, "image/jpeg"),
                Ok(fmt @ image::ImageFormat::WebP) => (fmt, "image/webp"),
                Ok(fmt @ image::ImageFormat::Gif) => (fmt, "image/gif"),
                _ => bail!("图片格式无效，请选择 PNG、JPEG、WebP 或 GIF 图片"),
            };
            if actual != mime {
                bail!("图片扩展名与实际内容不一致，请重新保存为正确格式后上传")
            }
            let (width, height) =
                image::ImageReader::with_format(std::io::Cursor::new(&data), format)
                    .into_dimensions()
                    .context("无法读取图片尺寸，图片可能已损坏")?;
            if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 64 * 1024 * 1024
            {
                bail!("图片尺寸过大或无效，请缩小至 6400 万像素以内")
            }
            dimensions = Some((width, height));
        }
        let mut attachment = json!({"type":if mime.starts_with("image/"){"image"}else{"file"},"mime":mime,"fileName":name,"size":data.len(),"content":STANDARD.encode(&data)});
        if let Some((width, height)) = dimensions {
            attachment["width"] = json!(width);
            attachment["height"] = json!(height);
        }
        out.push(attachment);
    }
    Ok(out)
}

pub fn image_request(prompt: &str) -> String {
    format!("{}\n<amoo_work_image_instructions>\n请真正完成图片创作并交付图片文件，不要只给提示词或说任务已下发。使用当前 Amoo 的 text_to_image 能力；若该工具仅供 task-bot 使用，创建 image 类型任务，并将参考附件传给该任务。必须等待任务实际完成，再交付原图附件或工具真实返回的下载链接；不能将“已下发任务”作为最终交付。保留用户要求的数量、比例、文字与参考图约束；未指定时先交付一张。不要编造文件名或 URL，失败时说明实际原因。服务自动添加的标识如实保留和说明，不能声称已去除服务水印。\n</amoo_work_image_instructions>",prompt.trim())
}

#[derive(Clone, Debug)]
pub struct DownloadedArtifact {
    pub path: PathBuf,
    pub bytes: u64,
    pub mime: String,
}

/// Exact returned reference identity for a private, account-scoped cache. Keeping
/// every URL byte avoids aliasing distinct objects by dropping query parameters.
pub fn artifact_cache_key(url: &str) -> String {
    artifact_content_key(url.as_bytes())
}

/// Content-addressed artifact storage. No file names, signed URLs or account
/// credentials are embedded in the on-disk object name.
pub fn artifact_content_key(bytes: &[u8]) -> String {
    crate::device::sha256(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn download_failure(info: Option<&Value>) -> &'static str {
    match info.and_then(|value| value["error"].as_str()) {
        Some("expired") => "此图片未缓存，下载链接已过期。可在手机 Amoo 重新获取后同步，或将保存的原图添加到工作资料；新收到的图片会自动缓存到本机",
        Some("access_denied") => "附件服务拒绝下载，请确认手机账号仍有访问此文件的权限",
        Some("not_found") => "附件已不存在或已被远端删除",
        Some("rate_limited") => "附件服务请求过于频繁，请稍后重试",
        Some("service_unavailable") => "附件服务暂时不可用，请稍后重试",
        Some("timeout") => "附件下载超时，请检查电脑网络后重试",
        Some("network") => "无法连接附件服务，请检查电脑网络后重试",
        Some("invalid_image") => "附件服务返回的内容不是有效图片，无法预览",
        Some("empty") => "附件服务返回了空文件",
        Some("too_large") => "附件超过 32 MiB 下载限制",
        Some("invalid_url" | "unsafe_redirect") => "附件下载链接或跳转地址无效",
        _ => "附件下载失败，请稍后重试",
    }
}

fn download_http_loopback(url: &str, dest: &std::path::Path) -> Result<()> {
    use std::io::Write;
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    const MAX_BYTES: usize = 8 * 1024 * 1024;
    let parsed = url::Url::parse(url).map_err(|_| anyhow::anyhow!("invalid_url"))?;
    if parsed.scheme() != "http" || parsed.host_str() != Some("127.0.0.1") {
        bail!("invalid_url");
    }
    let port = parsed
        .port()
        .ok_or_else(|| anyhow::anyhow!("invalid_url"))?;
    let mut path_and_query = parsed.path().to_string();
    if path_and_query.is_empty() {
        path_and_query.push('/');
    }
    if let Some(q) = parsed.query() {
        path_and_query.push('?');
        path_and_query.push_str(q);
    }
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
        .map_err(|_| anyhow::anyhow!("network"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(25)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(25)));
    let req = format!(
        "GET {path_and_query} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUser-Agent: StepX Desktop\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|_| anyhow::anyhow!("network"))?;
    stream.flush().map_err(|_| anyhow::anyhow!("network"))?;

    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        let n = stream
            .read(&mut tmp)
            .map_err(|_| anyhow::anyhow!("network"))?;
        if n == 0 {
            bail!("http");
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 16384 {
            bail!("http");
        }
    };
    let header_str = String::from_utf8_lossy(&buf[..header_end - 4]);
    let mut lines = header_str.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("http"))?;
    if status_code == 404 || status_code == 410 {
        bail!("expired");
    }
    if status_code != 200 {
        bail!("http");
    }
    let mut chunked = false;
    let mut content_length: Option<usize> = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim().to_ascii_lowercase();
            if k == "transfer-encoding" && v.contains("chunked") {
                chunked = true;
            } else if k == "content-length" {
                content_length = v.parse::<usize>().ok();
            }
        }
    }
    if let Some(cl) = content_length {
        if cl > MAX_BYTES {
            bail!("too_large");
        }
    }
    let body = if chunked {
        let mut raw_body: Vec<u8> = buf.drain(header_end..).collect();
        loop {
            if raw_body.len() > MAX_BYTES + 65536 {
                bail!("too_large");
            }
            let n = stream
                .read(&mut tmp)
                .map_err(|_| anyhow::anyhow!("network"))?;
            if n == 0 {
                break;
            }
            raw_body.extend_from_slice(&tmp[..n]);
        }
        let mut out = Vec::new();
        let mut pos = 0usize;
        loop {
            let Some(rel_crlf) = raw_body[pos..].windows(2).position(|w| w == b"\r\n") else {
                bail!("http");
            };
            let size_line = std::str::from_utf8(&raw_body[pos..pos + rel_crlf])
                .map_err(|_| anyhow::anyhow!("http"))?;
            let size_hex = size_line.split(';').next().unwrap_or("").trim();
            let chunk_size =
                usize::from_str_radix(size_hex, 16).map_err(|_| anyhow::anyhow!("http"))?;
            pos += rel_crlf + 2;
            if chunk_size == 0 {
                break;
            }
            if out.len() + chunk_size > MAX_BYTES {
                bail!("too_large");
            }
            if pos + chunk_size + 2 > raw_body.len() {
                bail!("http");
            }
            out.extend_from_slice(&raw_body[pos..pos + chunk_size]);
            pos += chunk_size;
            if &raw_body[pos..pos + 2] != b"\r\n" {
                bail!("http");
            }
            pos += 2;
        }
        out
    } else {
        let mut out: Vec<u8> = buf.drain(header_end..).collect();
        if out.len() > MAX_BYTES {
            bail!("too_large");
        }
        while content_length.is_none_or(|cl| out.len() < cl) {
            let n = stream
                .read(&mut tmp)
                .map_err(|_| anyhow::anyhow!("network"))?;
            if n == 0 {
                break;
            }
            if out.len() + n > MAX_BYTES {
                bail!("too_large");
            }
            out.extend_from_slice(&tmp[..n]);
        }
        if let Some(cl) = content_length {
            if out.len() < cl {
                bail!("http");
            }
            out.truncate(cl);
        }
        out
    };
    if body.is_empty() {
        bail!("empty");
    }
    let mut file = fs::File::create(dest)?;
    file.write_all(&body)?;
    file.flush()?;
    Ok(())
}

/// Save a returned artifact through the phone's local blob server or its real HTTPS URL.
/// Opaque blob identifiers and cloud filesystem paths are never guessed into URLs.
pub fn download_artifact(
    adb: &crate::adb::Adb,
    device: &crate::device::Device,
    attachment: &crate::openclaw::RealAttachment,
    output: &std::path::Path,
) -> Result<DownloadedArtifact> {
    use std::time::Duration;
    device.verify(adb)?;
    if output.exists() {
        bail!("目标文件已存在，请另选保存名称")
    }
    let parent = output.parent().context("请选择有效保存目录")?;
    if !parent.is_dir() {
        bail!("保存目录不存在")
    }
    let raw = if !attachment.url.is_empty() {
        &attachment.url
    } else {
        &attachment.blob
    };
    let mut url = raw.replace("\\u0026", "&");
    if url.chars().any(char::is_control) {
        bail!("附件下载链接无效")
    }
    let mut forwarded = None;
    if let Some(rest) = url.strip_prefix("http://127.0.0.1:") {
        let (port, path) = rest.split_once('/').context("手机附件链接无效")?;
        let port: u16 = port.parse().context("手机附件端口无效")?;
        let sha = path.strip_prefix("blob/").context("不是手机 Blob 附件")?;
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("手机附件标识无效")
        }
        let local = adb
            .run(&[
                "-s",
                &device.transport,
                "forward",
                "tcp:0",
                &format!("tcp:{port}"),
            ])?
            .trim()
            .to_string();
        let local_port: u16 = local.parse().context("无法转发手机附件")?;
        url = format!("http://127.0.0.1:{local_port}/{path}");
        forwarded = Some(format!("tcp:{local_port}"));
    } else if !url.starts_with("https://") {
        bail!("手机尚未返回可下载的 HTTPS 附件链接")
    }
    let download = (|| -> Result<DownloadedArtifact> {
        let temp = tempfile::NamedTempFile::new_in(parent)?;
        let (bytes, mime) = if url.starts_with("http://127.0.0.1:") {
            if let Err(err) = download_http_loopback(&url, temp.path()) {
                let code = err.to_string();
                bail!(download_failure(Some(&json!({"error": code}))))
            }
            let bytes = fs::metadata(temp.path())?.len();
            if bytes == 0 {
                bail!("附件内容为空")
            }
            let mime = if !attachment.mime.is_empty() {
                attachment.mime.clone()
            } else {
                mime_type(output).to_string()
            };
            (bytes, mime)
        } else {
            let payload = json!({"url":url,"output":temp.path(),"image":attachment.kind=="image"||attachment.mime.starts_with("image/")});
            let mut command = crate::resources::python_command();
            command.args(["-c", include_str!("amoo_download.py")]);
            let result = crate::adb::run_process(
                command,
                Some(serde_json::to_vec(&payload)?),
                Duration::from_secs(70),
            )?;
            let info = serde_json::from_slice::<Value>(&result.stdout).ok();
            if !result.status.success() || info.as_ref().is_some_and(|info| info["ok"] == false) {
                bail!(download_failure(info.as_ref()))
            }
            let info = info.context("附件下载返回无效")?;
            let bytes = info["bytes"].as_u64().context("附件内容为空")?;
            if bytes == 0 {
                bail!("附件内容为空")
            }
            (
                bytes,
                info["mime"].as_str().unwrap_or(&attachment.mime).into(),
            )
        };
        if attachment.kind == "image" || attachment.mime.starts_with("image/") {
            verify_downloaded_image(temp.path())?;
        }
        temp.persist_noclobber(output)
            .map_err(|_| anyhow::anyhow!("保存失败：文件可能已存在"))?;
        Ok(DownloadedArtifact {
            path: output.to_path_buf(),
            bytes,
            mime,
        })
    })();
    if let Some(port) = forwarded {
        let _ = adb.run(&["-s", &device.transport, "forward", "--remove", &port]);
    }
    download
}

fn verify_downloaded_image(path: &std::path::Path) -> Result<()> {
    use image::GenericImageView as _;
    let decoded = image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()
        .context("下载的图片不完整或已损坏，请重试")?;
    let (width, height) = decoded.dimensions();
    anyhow::ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 64 * 1024 * 1024,
        "下载的图片尺寸过大或无效"
    );
    Ok(())
}

#[derive(Clone, Debug)]
pub struct UploadedAttachment {
    pub name: String,
    pub mime: String,
    pub bytes: u64,
    pub device_path: String,
    pub sha256: String,
    /// URL returned by the phone's authenticated upload ability, never guessed.
    pub url: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug)]
pub struct UploadedAttachments {
    pub items: Vec<UploadedAttachment>,
    pub directory: String,
}
impl UploadedAttachments {
    pub fn message(&self, prompt: &str) -> String {
        if self.items.is_empty() {
            return prompt.into();
        }
        let entries:Vec<Value>=self.items.iter().map(|a|json!({"id":"","url":a.url,"size":a.bytes,"devicePath":a.device_path,"filename":a.name,"mimeType":a.mime,"sha256":a.sha256})).collect();
        format!("{}\n<amoo_work_upload_instructions>图片已通过原生图片附件提交，请直接看图分析，不要用 exec、OCR 或读取二进制来替代看图。其他类型附件已上传到手机；devicePath 是手机路径，可用手机文件读取能力处理。只处理所列附件，不要搜索其他目录。</amoo_work_upload_instructions>\n<extra>{}</extra>",prompt.trim(),json!({"attachments":entries}))
    }

    /// Native AgentBus Attachment uses `blob`, not inline `content`. Preserve the
    /// real URL in both the model input and history metadata (for later preview).
    pub fn wire_attachments(&self) -> Result<Vec<Value>> {
        self.items
            .iter()
            .filter(|a| a.mime.starts_with("image/"))
            .map(|a| {
                if !a.url.starts_with("https://") || a.width == 0 || a.height == 0 {
                    bail!("图片尚未完成上传，消息未发送")
                }
                Ok(
                    json!({"type":"image","blob":a.url,"mime":a.mime,"size":a.bytes,
                "sha256":a.sha256,"width":a.width,"height":a.height,"filename":a.name}),
                )
            })
            .collect()
    }
}
/// Upload immutable snapshots into a unique phone inbox, then verify their SHA-256.
/// The native `<extra>.attachments[].devicePath` channel lets Amoo's phone tools
/// consume the actual files without exporting account credentials to the desktop.
pub fn upload_attachments(
    adb: &crate::adb::Adb,
    device: &crate::device::Device,
    paths: &[PathBuf],
) -> Result<UploadedAttachments> {
    upload_attachments_with_bus(adb, device, paths, None)
}

pub fn upload_attachments_with_bus(
    adb: &crate::adb::Adb,
    device: &crate::device::Device,
    paths: &[PathBuf],
    bus: Option<&mut crate::amoo::AgentBus>,
) -> Result<UploadedAttachments> {
    use crate::adb::shell_quote;
    use std::io::Write;
    if bus.is_none() {
        device.verify(adb)?;
    }
    // Reuse byte/count validation; this reads only explicitly selected local files.
    let prepared = prepare_attachments(paths)?;
    if prepared.is_empty() {
        return Ok(UploadedAttachments {
            items: vec![],
            directory: String::new(),
        });
    }
    let directory = format!("/sdcard/Download/Amoo Work/{:032x}", rand::random::<u128>());
    adb.shell(&device.transport, &["mkdir", "-p", "--", &directory])?;
    let result = (|| -> Result<UploadedAttachments> {
        let mut items = Vec::new();
        for (index, attachment) in prepared.iter().enumerate() {
            let name = attachment["fileName"].as_str().context("附件名称无效")?;
            if name.chars().any(char::is_control) || name == "." || name == ".." {
                bail!("附件名称包含不支持的字符")
            }
            let data = STANDARD.decode(attachment["content"].as_str().context("附件快照无效")?)?;
            let mut snapshot = tempfile::NamedTempFile::new()?;
            snapshot.write_all(&data)?;
            snapshot.as_file().sync_all()?;
            let sha = artifact_content_key(&data);
            let remote = format!("{directory}/{index:02}-{name}");
            adb.transfer(
                &device.transport,
                "push",
                snapshot.path(),
                std::path::Path::new(&remote),
            )?;
            items.push(UploadedAttachment {
                name: name.into(),
                mime: attachment["mime"]
                    .as_str()
                    .unwrap_or("application/octet-stream")
                    .into(),
                bytes: data.len() as u64,
                device_path: remote,
                sha256: sha,
                url: String::new(),
                width: attachment["width"].as_u64().unwrap_or(0) as u32,
                height: attachment["height"].as_u64().unwrap_or(0) as u32,
            });
        }
        if !items.is_empty() {
            let quoted_paths = items
                .iter()
                .map(|item| shell_quote(&item.device_path))
                .collect::<Vec<_>>()
                .join(" ");
            let actual = adb.script(&device.transport, &format!("sha256sum {quoted_paths}"))?;
            let mut actual_sums = actual
                .lines()
                .filter_map(|line| line.split_whitespace().next());
            for item in &items {
                if actual_sums.next() != Some(item.sha256.as_str()) {
                    bail!("附件上传校验失败，文件未发送给 Amoo")
                }
            }
        }
        let image_paths = items
            .iter()
            .filter(|a| a.mime.starts_with("image/"))
            .map(|a| a.device_path.clone())
            .collect::<Vec<_>>();
        if !image_paths.is_empty() {
            let urls = upload_native_images(adb, device, &image_paths, bus)?;
            for item in &mut items {
                if let Some(url) = urls.get(&item.device_path) {
                    item.url = url.clone();
                }
            }
        }
        Ok(UploadedAttachments {
            items,
            directory: directory.clone(),
        })
    })();
    if result.is_err() {
        let _ = adb.shell(&device.transport, &["rm", "-rf", "--", &directory]);
    }
    result
}

/// Invoke the first-party upload ability on this phone. Its account credentials
/// stay inside Android; the desktop only receives the uploaded image references.
fn upload_native_images(
    adb: &crate::adb::Adb,
    device: &crate::device::Device,
    paths: &[String],
    bus: Option<&mut crate::amoo::AgentBus>,
) -> Result<std::collections::HashMap<String, String>> {
    // Upload precedes chat.send, so this connection owns a task lease for the
    // entire native upload. The phone routes directly to its local ability;
    // its screen-off edge WebSocket and cloud-account node lists are not needed.
    let response = match bus {
        Some(bus) => bus
            .upload_local_images(paths)
            .context("手机图片上传未完成，草稿已保留，请稍后重试")?,
        None => {
            let mut owned_bus = crate::amoo::AgentBus::connect(adb, device)
                .context("无法连接手机的图片上传服务，图片尚未发送")?;
            owned_bus
                .upload_local_images(paths)
                .context("手机图片上传未完成，草稿已保留，请稍后重试")?
        }
    };
    native_upload_results(&response, paths)
}

fn native_upload_results(
    response: &Value,
    paths: &[String],
) -> Result<std::collections::HashMap<String, String>> {
    if response["ok"] != true || response["payload"]["ok"] != true {
        bail!("手机图片上传失败，图片尚未交给 Amoo")
    }
    let rows = response["payload"]["content"]["uploaded"]
        .as_array()
        .context("手机未返回图片上传结果")?;
    if rows.len() != paths.len() {
        bail!("部分图片未完成上传，消息尚未发送")
    }
    let mut out = std::collections::HashMap::new();
    for row in rows {
        let path = row["sourcePath"].as_str().unwrap_or("");
        let path = path.strip_prefix("file://").unwrap_or(path);
        let url = row["url"].as_str().unwrap_or("");
        if row["success"] != true
            || !paths.iter().any(|p| p == path)
            || !url.starts_with("https://")
            || url.chars().any(char::is_control)
            || out.insert(path.to_string(), url.to_string()).is_some()
        {
            bail!("图片上传结果无效或不完整，消息尚未发送")
        }
    }
    Ok(out)
}

pub fn send_files(
    adb: &crate::adb::Adb,
    device: &crate::device::Device,
    key: &str,
    prompt: &str,
    paths: &[PathBuf],
) -> Result<crate::amoo::PendingReply> {
    let mut bus = crate::amoo::AgentBus::connect(adb, device)?;
    let uploaded = upload_attachments_with_bus(adb, device, paths, Some(&mut bus))?;
    // Keep uploaded bytes after an uncertain send: retrying must never lose data.
    let mut receipt = bus.send_with_attachments(
        key,
        &uploaded.message(prompt),
        &uploaded.wire_attachments()?,
    )?;
    receipt.text = prompt.to_string();
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloader_classifies_expiry_without_leaking_service_body() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        let cases: &[(&str, &str, &[u8], &str)] = &[
            (
                "403 Forbidden",
                "application/json",
                br#"{"Code":"AccessDenied","Message":"Request has expired; secret-credential"}"#,
                "expired",
            ),
            (
                "403 Forbidden",
                "application/json",
                br#"{"Code":"AccessDenied","Message":"Access denied; secret-credential"}"#,
                "access_denied",
            ),
            (
                "403 Forbidden",
                "application/xml",
                b"<Error><Code>AccessDenied</Code><Message>Request has expired; secret-credential</Message></Error>",
                "expired",
            ),
            (
                "404 Not Found",
                "text/plain",
                b"secret-credential",
                "not_found",
            ),
            (
                "200 OK",
                "text/html",
                b"<html>secret-credential</html>",
                "invalid_image",
            ),
            ("200 OK", "image/png", b"\x89PNG\r\n\x1a\nimage-bytes", ""),
        ];
        for (status, mime, body, expected) in cases {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let port = listener.local_addr().unwrap().port();
            let header = format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let body = body.to_vec();
            let server = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < deadline {
                    if let Ok((mut stream, _)) = listener.accept() {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(3)))
                            .unwrap();
                        let mut buffer = [0; 4096];
                        // Windows may deliver request headers in several packets.
                        // Closing with unread request bytes can reset the socket
                        // before the downloader receives the error response.
                        let mut request = Vec::new();
                        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                            let size = stream.read(&mut buffer).unwrap();
                            assert!(size > 0 && request.len() + size <= 16384);
                            request.extend_from_slice(&buffer[..size]);
                        }
                        stream.write_all(header.as_bytes()).unwrap();
                        stream.write_all(&body).unwrap();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                panic!("downloader did not connect");
            });
            let dir = tempfile::tempdir().unwrap();
            let output = dir.path().join("artifact.png");
            let payload = json!({"url":format!("http://127.0.0.1:{port}/blob/{}","a".repeat(64)),"output":output,"image":true});
            let mut command = crate::resources::python_command();
            command.args(["-c", include_str!("amoo_download.py")]);
            command.env("NO_PROXY", "127.0.0.1");
            command.env("no_proxy", "127.0.0.1");
            let result = crate::adb::run_process(
                command,
                Some(serde_json::to_vec(&payload).unwrap()),
                Duration::from_secs(12),
            )
            .unwrap();
            server.join().unwrap();
            assert!(!String::from_utf8_lossy(&result.stdout).contains("secret-credential"));
            assert!(result.stderr.is_empty());
            let info: Value = serde_json::from_slice(&result.stdout).unwrap();
            if expected.is_empty() {
                assert!(result.status.success());
                assert_eq!(info["ok"], true);
                assert_eq!(
                    info["bytes"].as_u64(),
                    Some(fs::metadata(output).unwrap().len())
                );
            } else {
                assert!(!result.status.success());
                assert_eq!(info["error"], *expected);
                assert!(!download_failure(Some(&info)).contains("同步手机后重试"));
            }
        }
    }

    #[test]
    fn artifact_cache_identity_keeps_the_complete_signed_reference() {
        let first = artifact_cache_key("https://example.test/file?version=1&signature=a");
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(
            first,
            artifact_cache_key("https://example.test/file?version=2&signature=a")
        );
        assert_ne!(
            first,
            artifact_cache_key("https://example.test/file?version=1&signature=b")
        );
    }

    #[test]
    fn image_delivery_rejects_valid_header_with_corrupt_body() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("image.png");
        fs::write(&image, b"\x89PNG\r\n\x1a\ntruncated").unwrap();
        assert!(verify_downloaded_image(&image).is_err());
        image::RgbaImage::from_pixel(3, 2, image::Rgba([20, 40, 80, 255]))
            .save(&image)
            .unwrap();
        verify_downloaded_image(&image).unwrap();
    }

    #[test]
    fn attachment_snapshot_preserves_bytes_and_unicode_filename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("销售 数据.csv");
        let bytes = "项目,数值\n桌面,43\n".as_bytes();
        fs::write(&path, bytes).unwrap();
        let prepared = prepare_attachments(&[path]).unwrap();
        assert_eq!(prepared[0]["mime"], "text/csv");
        assert_eq!(prepared[0]["fileName"], "销售 数据.csv");
        assert_eq!(
            STANDARD
                .decode(prepared[0]["content"].as_str().unwrap())
                .unwrap(),
            bytes
        );
    }
    #[test]
    fn upload_limits_reject_large_and_non_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(prepare_attachments(&[dir.path().to_path_buf()]).is_err());
        let path = dir.path().join("too-large.bin");
        fs::File::create(&path)
            .unwrap()
            .set_len(MAX_ATTACHMENT_BYTES + 1)
            .unwrap();
        assert!(prepare_attachments(&[path]).is_err());
        assert!(prepare_attachments(&vec![dir.path().to_path_buf(); 9]).is_err());
    }
    #[test]
    fn native_phone_metadata_contains_real_path_and_no_contents() {
        let uploaded = UploadedAttachments {
            directory: "/sdcard/Download/Amoo Work/own".into(),
            items: vec![UploadedAttachment {
                name: "sales.csv".into(),
                mime: "text/csv".into(),
                bytes: 66,
                device_path: "/sdcard/Download/Amoo Work/own/00-sales.csv".into(),
                sha256: "a".repeat(64),
                url: String::new(),
                width: 0,
                height: 0,
            }],
        };
        let message = uploaded.message("求合计");
        let raw = message
            .split_once("<extra>")
            .unwrap()
            .1
            .strip_suffix("</extra>")
            .unwrap();
        let value: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(
            value["attachments"][0]["devicePath"],
            uploaded.items[0].device_path
        );
        assert_eq!(value["attachments"][0]["id"], "");
        assert!(value["attachments"][0].get("content").is_none());
    }
    #[test]
    fn expected_document_mime_types_are_used() {
        assert_eq!(mime_type(PathBuf::from("IMG.JPG").as_path()), "image/jpeg");
        assert_eq!(
            mime_type(PathBuf::from("report.pdf").as_path()),
            "application/pdf"
        );
        assert!(mime_type(PathBuf::from("report.docx").as_path()).contains("wordprocessingml"));
        assert!(mime_type(PathBuf::from("report.xlsx").as_path()).contains("spreadsheetml"));
    }
    #[test]
    fn text_renamed_to_image_is_rejected_before_upload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.png");
        fs::write(&path, b"not image bytes").unwrap();
        assert!(prepare_attachments(&[path]).is_err());
    }
    #[test]
    fn native_image_input_requires_uploaded_blob_and_retains_history_reference() {
        let mut uploaded = UploadedAttachments {
            directory: "/sdcard/Download/Amoo Work/test".into(),
            items: vec![UploadedAttachment {
                name: "图片 #1%.png".into(),
                mime: "image/png".into(),
                bytes: 123,
                device_path: "/sdcard/Download/Amoo Work/test/00-图片 #1%.png".into(),
                sha256: "a".repeat(64),
                url: String::new(),
                width: 900,
                height: 540,
            }],
        };
        assert!(uploaded.wire_attachments().is_err());
        uploaded.items[0].url = "https://example.test/actual.png?signed=1&key=2".into();
        let wire = uploaded.wire_attachments().unwrap();
        assert_eq!(wire[0]["blob"], uploaded.items[0].url);
        assert_eq!(wire[0]["mime"], "image/png");
        assert_eq!(wire[0]["width"], 900);
        assert!(wire[0].get("content").is_none());
        assert!(uploaded
            .message("分析图片")
            .contains(&uploaded.items[0].url));
    }
    #[test]
    fn batch_upload_matches_by_source_and_rejects_partial_or_duplicate_results() {
        let paths: Vec<String> = vec!["/sdcard/a #1%.png".into(), "/sdcard/b.png".into()];
        let row = |path: &str| json!({"sourcePath":path,"url":"https://example.test/image.png","success":true});
        let response =
            |rows: Vec<Value>| json!({"ok":true,"payload":{"ok":true,"content":{"uploaded":rows}}});
        let valid = response(vec![row(&paths[1]), row(&paths[0])]);
        let parsed = native_upload_results(&valid, &paths).unwrap();
        assert!(parsed.contains_key(&paths[0]));
        assert_eq!(parsed.len(), 2);
        assert!(native_upload_results(&response(vec![row(&paths[0])]), &paths).is_err());
        assert!(
            native_upload_results(&response(vec![row(&paths[0]), row(&paths[0])]), &paths).is_err()
        );
        assert!(native_upload_results(
            &response(vec![row(&paths[0]), row("/sdcard/not-selected.png")]),
            &paths
        )
        .is_err());
        let mut failed = valid.clone();
        failed["payload"]["content"]["uploaded"][0]["success"] = json!(false);
        assert!(native_upload_results(&failed, &paths).is_err());
        let mut unsafe_url = valid;
        unsafe_url["payload"]["content"]["uploaded"][0]["url"] = json!("file:///tmp/image.png");
        assert!(native_upload_results(&unsafe_url, &paths).is_err());
    }
}
