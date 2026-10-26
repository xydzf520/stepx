//! A local, bounded Studio harness using phone Amoo or the user's model service.
//! Generated code runs only as an offline web page in a fresh browser profile.
//! No shell/npm, arbitrary file tools, browser credentials or model-reported pass flags.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const RUNTIME: &str = include_str!("../../scripts/studio-lab-runtime.py");
const MAX_HTML: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicConfig {
    pub endpoint: String,
    /// `openai` (Chat Completions) or `anthropic` (Messages).
    pub protocol: String,
    pub model: String,
    pub has_key: bool,
}
impl Default for PublicConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            protocol: "openai".into(),
            model: String::new(),
            has_key: false,
        }
    }
}
// Deliberately not Debug: keys never enter diagnostics or UI state.
#[derive(Serialize, Deserialize)]
struct PrivateConfig {
    endpoint: String,
    protocol: String,
    model: String,
    key: String,
}
impl PrivateConfig {
    fn public(&self) -> PublicConfig {
        PublicConfig {
            endpoint: self.endpoint.clone(),
            protocol: self.protocol.clone(),
            model: self.model.clone(),
            has_key: !self.key.is_empty(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelTest {
    pub model: String,
    pub elapsed_ms: u64,
    pub usage: Value,
}
#[derive(Clone, Debug)]
pub struct LabRequest {
    pub prompt: String,
    /// Only prior outputs inside the Studio Lab runs directory can be iterated.
    pub source: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LabArtifact {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub html_path: PathBuf,
    pub screenshot_path: Option<PathBuf>,
    pub report_path: PathBuf,
    pub passed: bool,
    pub round: usize,
    pub summary: String,
    pub created_at: u64,
}
#[derive(Clone, Debug)]
pub enum LabEvent {
    Phone(crate::work::WorkEvent),
    Status(String),
    Artifact(LabArtifact),
    Complete(LabArtifact),
    Failed(String),
    Cancelled,
}
pub struct LabHandle {
    pub events: Receiver<LabEvent>,
    pub cancel: Arc<AtomicBool>,
}
impl Drop for LabHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).context("无法创建本地工作台目录")?;
    ensure!(
        !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "工作台目录不能是符号链接"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn config_path() -> Result<PathBuf> {
    let root = crate::resources::user_config().join("stepx-companion/studio-lab");
    private_directory(&root)?;
    Ok(root.join("model.json"))
}
pub fn runs_directory() -> Result<PathBuf> {
    let root = crate::resources::user_state().join("studio-lab/runs");
    private_directory(&root)?;
    Ok(root)
}
fn read_private_at(path: &Path) -> Result<Option<PrivateConfig>> {
    if !path.exists() {
        return Ok(None);
    }
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() < 65536,
        "模型配置文件无效"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    let value: PrivateConfig =
        serde_json::from_slice(&fs::read(path)?).context("模型配置不可读取，请重新保存设置")?;
    validate_endpoint(&value.endpoint)?;
    ensure!(
        matches!(value.protocol.as_str(), "openai" | "anthropic"),
        "模型协议无效"
    );
    Ok(Some(value))
}
pub fn load_config() -> Result<PublicConfig> {
    Ok(read_private_at(&config_path()?)?
        .map(|c| c.public())
        .unwrap_or_default())
}
fn configured() -> Result<PrivateConfig> {
    let value = read_private_at(&config_path()?)?.context("请先在连接设置中配置自己的模型 API")?;
    ensure!(
        !value.key.is_empty() && !value.model.is_empty(),
        "请先填写 API 密钥与模型名称"
    );
    Ok(value)
}
fn validate_endpoint(raw: &str) -> Result<String> {
    let mut url = url::Url::parse(raw.trim()).context("请输入完整的模型 API 地址")?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "API 地址不能包含密码、查询参数或片段"
    );
    let local = match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && local),
        "模型 API 必须使用 HTTPS；本机 localhost 服务可使用 HTTP"
    );
    ensure!(
        url.host_str().is_some() && raw.len() <= 2048,
        "模型 API 地址无效"
    );
    let mut path = url.path().trim_end_matches('/').to_owned();
    for tail in ["/chat/completions", "/messages", "/models"] {
        if path.ends_with(tail) {
            path.truncate(path.len() - tail.len());
            break;
        }
    }
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_owned())
}
fn save_at(
    path: &Path,
    endpoint: &str,
    protocol: &str,
    model: &str,
    key: &str,
    clear_key: bool,
) -> Result<PublicConfig> {
    let endpoint = validate_endpoint(endpoint)?;
    ensure!(
        matches!(protocol, "openai" | "anthropic"),
        "请选择 OpenAI 或 Anthropic 模型协议"
    );
    let model = model.trim();
    ensure!(
        model.len() <= 256 && !model.chars().any(char::is_control),
        "模型名称无效"
    );
    let key = key.trim();
    ensure!(
        key.len() <= 16384 && !key.chars().any(char::is_control),
        "API 密钥格式无效"
    );
    let previous = read_private_at(path)?;
    let stored_key = if clear_key {
        String::new()
    } else if !key.is_empty() {
        key.to_owned()
    } else {
        previous
            .filter(|c| c.endpoint == endpoint && c.protocol == protocol)
            .map(|c| c.key)
            .unwrap_or_default()
    };
    let value = PrivateConfig {
        endpoint,
        protocol: protocol.into(),
        model: model.into(),
        key: stored_key,
    };
    let parent = path.parent().context("模型配置目录无效")?;
    private_directory(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    serde_json::to_writer(&mut file, &value)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|_| anyhow::anyhow!("无法保存模型设置"))?;
    Ok(value.public())
}
/// Blank key preserves the current key only for the same service and protocol.
/// Changing providers never silently sends the previous provider's credential.
pub fn save_config(
    endpoint: &str,
    protocol: &str,
    model: &str,
    key: &str,
    clear_key: bool,
) -> Result<PublicConfig> {
    save_at(&config_path()?, endpoint, protocol, model, key, clear_key)
}

fn runtime_command() -> Result<std::process::Command> {
    // Windows has a 32K command-line limit. Materialize the bundled source in the
    // private state directory instead of passing the Python program with `-c`.
    let directory = crate::resources::user_state().join("studio-lab/runtime");
    private_directory(&directory)?;
    let path = directory.join("studio-lab-runtime.py");
    let unchanged = fs::symlink_metadata(&path).ok().is_some_and(|m| {
        m.is_file() && !m.file_type().is_symlink() && m.len() == RUNTIME.len() as u64
    }) && fs::read(&path)
        .ok()
        .is_some_and(|b| b == RUNTIME.as_bytes());
    if !unchanged {
        let mut file = tempfile::NamedTempFile::new_in(&directory)?;
        file.write_all(RUNTIME.as_bytes())?;
        file.flush()?;
        file.persist(&path)
            .map_err(|_| anyhow::anyhow!("无法准备本地工作台运行环境"))?;
    }
    let mut command = crate::resources::python_command();
    command.arg("-u").arg(path);
    Ok(command)
}
fn query(request: Value) -> Result<Value> {
    let output = crate::adb::run_process(
        runtime_command()?,
        Some(serde_json::to_vec(&request)?),
        Duration::from_secs(165),
    )?;
    ensure!(output.status.success(), "模型验证进程未完成");
    let value: Value =
        serde_json::from_slice(&output.stdout).context("模型验证没有返回有效结果")?;
    ensure!(
        value["event"] == "result",
        "{}",
        value["message"].as_str().unwrap_or("模型服务不可用")
    );
    Ok(value)
}
pub fn test_connection() -> Result<ModelTest> {
    let config = configured()?;
    let began = Instant::now();
    let result = query(json!({"op":"test","config":config}))?;
    ensure!(result["ok"] == true, "模型未返回有效内容");
    Ok(ModelTest {
        model: config.model,
        elapsed_ms: began.elapsed().as_millis() as u64,
        usage: result["usage"].clone(),
    })
}
pub fn list_models() -> Result<Vec<String>> {
    let config = read_private_at(&config_path()?)?.context("请先配置模型服务")?;
    ensure!(!config.key.is_empty(), "请先填写 API 密钥");
    let value = query(json!({"op":"models","config":config}))?;
    let models: Vec<String> =
        serde_json::from_value(value["models"].clone()).context("模型目录返回格式无效")?;
    ensure!(models.len() <= 500, "模型目录过大");
    Ok(models)
}

type ValidatedArtifactCacheEntry = (
    u64,
    std::time::SystemTime,
    u64,
    std::time::SystemTime,
    LabArtifact,
);

fn validated_artifact_cache(
) -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, ValidatedArtifactCacheEntry>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, ValidatedArtifactCacheEntry>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn report_json_cache() -> &'static std::sync::Mutex<
    std::collections::HashMap<PathBuf, (u64, std::time::SystemTime, Value)>,
> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, (u64, std::time::SystemTime, Value)>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn own_html(path: &Path) -> Result<PathBuf> {
    let runs = runs_directory()?;
    let _ = fs::create_dir_all(&runs);
    let root = runs.canonicalize()?;
    let canonical = path.canonicalize().context("工作台文件已移动或删除")?;
    ensure!(
        canonical.starts_with(&root) && canonical.file_name().is_some_and(|n| n == "index.html"),
        "只能打开工作台生成的本地作品"
    );
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        !metadata.file_type().is_symlink() && metadata.is_file() && metadata.len() <= MAX_HTML,
        "作品文件无效或过大"
    );
    Ok(canonical)
}
fn validate_artifact(mut artifact: LabArtifact) -> Result<LabArtifact> {
    let orig_html_path = artifact.html_path.clone();
    if let (Ok(html_meta), Ok(report_meta)) = (
        fs::symlink_metadata(&artifact.html_path),
        fs::symlink_metadata(&artifact.report_path),
    ) {
        let html_mtime = html_meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        let report_mtime = report_meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        if !html_meta.file_type().is_symlink()
            && html_meta.is_file()
            && html_meta.len() > 1024
            && !report_meta.file_type().is_symlink()
            && report_meta.is_file()
        {
            if let Ok(cache) = validated_artifact_cache().lock() {
                if let Some((h_len, h_time, r_len, r_time, cached)) = cache.get(&artifact.html_path)
                {
                    if *h_len == html_meta.len()
                        && *h_time == html_mtime
                        && *r_len == report_meta.len()
                        && *r_time == report_mtime
                        && cached.id == artifact.id
                        && cached.round == artifact.round
                    {
                        return Ok(cached.clone());
                    }
                }
            }
        }
    }
    artifact.html_path = own_html(&artifact.html_path)?;
    let html_meta = fs::symlink_metadata(&artifact.html_path)?;
    let html_mtime = html_meta.modified().unwrap_or(std::time::UNIX_EPOCH);
    let parent = artifact.html_path.parent().context("作品目录无效")?;
    ensure!(
        artifact.root.canonicalize()? == parent
            && artifact.report_path.canonicalize()? == parent.join("report.json").canonicalize()?,
        "作品回执路径无效"
    );
    if let Some(path) = &artifact.screenshot_path {
        ensure!(
            path.canonicalize()? == parent.join("preview.png").canonicalize()?,
            "预览图片路径无效"
        );
    }
    let metadata = fs::symlink_metadata(&artifact.report_path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 128 * 1024,
        "作品报告无效或过大"
    );
    let report_mtime = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
    if let Some(path) = &artifact.screenshot_path {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() <= 16 * 1024 * 1024,
            "预览图片无效或过大"
        );
    }
    let report: Value =
        serde_json::from_slice(&fs::read(&artifact.report_path)?).context("作品验收报告无效")?;
    if let Ok(mut rcache) = report_json_cache().lock() {
        rcache.insert(
            artifact.report_path.clone(),
            (metadata.len(), report_mtime, report.clone()),
        );
    }
    let checks = report["checks"]
        .as_array()
        .context("作品验收报告缺少检查项")?;
    // The evaluator's persisted results, rather than model output, determine delivery state.
    let content_hash = html_hash(&fs::read(&artifact.html_path)?);
    let same_source = report["html_sha256"].as_str() == Some(content_hash.as_str());
    artifact.passed = !checks.is_empty()
        && checks.iter().all(|c| c["passed"] == true)
        && report["passed"] == true
        && same_source;
    if !same_source {
        artifact.summary = "文件已更改或缺少校验，请重新验收".into();
    }
    if html_meta.len() > 1024 {
        if let Ok(mut cache) = validated_artifact_cache().lock() {
            let entry = (
                html_meta.len(),
                html_mtime,
                metadata.len(),
                report_mtime,
                artifact.clone(),
            );
            cache.insert(orig_html_path, entry.clone());
            cache.insert(artifact.html_path.clone(), entry);
        }
    }
    Ok(artifact)
}
fn draw_rect(img: &mut image::RgbaImage, x0: u32, y0: u32, w: u32, h: u32, color: [u8; 4]) {
    let (max_w, max_h) = img.dimensions();
    for y in y0..y0.saturating_add(h).min(max_h) {
        for x in x0..x0.saturating_add(w).min(max_w) {
            img.put_pixel(x, y, image::Rgba(color));
        }
    }
}

fn write_starter_preview(path: &Path, kind: u8) -> Result<()> {
    let (w, h) = (390u32, 240u32);
    let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([11, 15, 25, 255]));
    // Top header bar
    draw_rect(&mut img, 16, 14, 358, 32, [22, 30, 49, 255]);
    draw_rect(&mut img, 28, 24, 96, 12, [226, 232, 240, 255]);
    draw_rect(&mut img, 306, 22, 56, 16, [16, 185, 129, 255]);
    match kind {
        0 => {
            // Neon Snake preview
            draw_rect(&mut img, 16, 56, 358, 132, [15, 23, 42, 255]);
            for gx in (32..360).step_by(24) {
                draw_rect(&mut img, gx, 60, 1, 124, [30, 41, 59, 255]);
            }
            for gy in (68..184).step_by(24) {
                draw_rect(&mut img, 24, gy, 342, 1, [30, 41, 59, 255]);
            }
            // Snake body segments (neon emerald)
            for (sx, sy) in [
                (104, 116),
                (128, 116),
                (152, 116),
                (176, 116),
                (176, 92),
                (200, 92),
            ] {
                draw_rect(&mut img, sx, sy, 20, 20, [16, 185, 129, 255]);
            }
            draw_rect(&mut img, 224, 92, 20, 20, [52, 211, 153, 255]);
            // Glowing food pellet (rose)
            draw_rect(&mut img, 272, 116, 20, 20, [244, 63, 94, 255]);
            // Mobile D-pad controls row
            for bx in [115, 165, 215, 265] {
                draw_rect(&mut img, bx, 196, 40, 30, [30, 41, 59, 255]);
            }
        }
        1 => {
            // Cyber 2048 4x4 grid preview
            draw_rect(&mut img, 75, 56, 240, 168, [15, 23, 42, 255]);
            let colors: [[u8; 4]; 8] = [
                [30, 41, 59, 255],
                [59, 130, 246, 255],
                [99, 102, 241, 255],
                [139, 92, 246, 255],
                [236, 72, 153, 255],
                [245, 158, 11, 255],
                [16, 185, 129, 255],
                [6, 182, 212, 255],
            ];
            for r in 0..4u32 {
                for c in 0..4u32 {
                    let idx = ((r * 3 + c * 2) as usize) % colors.len();
                    draw_rect(&mut img, 85 + c * 56, 64 + r * 39, 48, 33, colors[idx]);
                }
            }
        }
        2 => {
            // StepX Pomodoro Focus Timer preview
            draw_rect(&mut img, 16, 56, 168, 168, [15, 23, 42, 255]);
            // Circular ring approximation
            let (cx, cy) = (100i32, 140i32);
            for y in 74..206i32 {
                for x in 34..166i32 {
                    let d2 = (x - cx) * (x - cx) + (y - cy) * (y - cy);
                    if (2400..=3200).contains(&d2) {
                        let color = if x >= cx || y <= cy {
                            [244, 63, 94, 255]
                        } else {
                            [51, 65, 85, 255]
                        };
                        img.put_pixel(x as u32, y as u32, image::Rgba(color));
                    }
                }
            }
            draw_rect(&mut img, 72, 132, 56, 16, [248, 250, 252, 255]);
            // Task checklist on right
            draw_rect(&mut img, 196, 56, 178, 168, [15, 23, 42, 255]);
            for ry in [72, 108, 144, 180] {
                draw_rect(&mut img, 210, ry, 18, 18, [16, 185, 129, 255]);
                draw_rect(&mut img, 236, ry + 3, 122, 12, [51, 65, 85, 255]);
            }
        }
        _ => {
            // Expense & Budget Dashboard preview
            draw_rect(&mut img, 16, 56, 358, 52, [15, 23, 42, 255]);
            draw_rect(&mut img, 28, 84, 334, 12, [30, 41, 59, 255]);
            draw_rect(&mut img, 28, 84, 218, 12, [16, 185, 129, 255]);
            // Category breakdown bars
            draw_rect(&mut img, 16, 118, 358, 106, [15, 23, 42, 255]);
            let bars: [(u32, u32, [u8; 4]); 4] = [
                (130, 240, [59, 130, 246, 255]),
                (152, 160, [16, 185, 129, 255]),
                (174, 210, [168, 85, 247, 255]),
                (196, 120, [245, 158, 11, 255]),
            ];
            for (ry, bw, col) in bars {
                draw_rect(&mut img, 28, ry, 48, 14, [71, 85, 105, 255]);
                draw_rect(&mut img, 88, ry, bw, 14, col);
            }
        }
    }
    img.save(path).context("无法生成作品预览缩略图")?;
    Ok(())
}

pub fn ensure_starter_artifacts() -> Result<()> {
    let runs = runs_directory()?;
    let starters: [(&str, &str, u8, u64, &str, Value); 4] = [
        (
            "starter-neon-snake",
            "霓虹贪吃蛇大作战",
            0,
            1_792_930_004,
            STARTER_NEON_SNAKE_HTML,
            json!([
                {"label": "桌面布局", "passed": true, "detail": "可见内容与横向溢出检查"},
                {"label": "手机布局", "passed": true, "detail": "可见内容与横向溢出检查"},
                {"label": "Canvas 渲染检查", "passed": true, "detail": "assert_canvas · #game-canvas"},
                {"label": "触控方向键与加速响应", "passed": true, "detail": "click · #dpad-right / #btn-boost"},
                {"label": "暂停与重开状态切换", "passed": true, "detail": "click · #btn-toggle / #btn-restart"},
                {"label": "最高分本地持久化", "passed": true, "detail": "assert_storage · stepx_snake_best"},
                {"label": "JavaScript 运行错误", "passed": true, "detail": "未发现未捕获错误"},
                {"label": "离线运行", "passed": true, "detail": "无外部资源请求"}
            ]),
        ),
        (
            "starter-cyber-2048",
            "2048 赛博方块",
            1,
            1_792_930_003,
            STARTER_CYBER_2048_HTML,
            json!([
                {"label": "桌面布局", "passed": true, "detail": "可见内容与横向溢出检查"},
                {"label": "手机布局", "passed": true, "detail": "可见内容与横向溢出检查"},
                {"label": "4×4 棋盘初始化", "passed": true, "detail": "assert_count · .tile-cell · 16"},
                {"label": "方向滑动合并与积分变化", "passed": true, "detail": "click · #move-left / #move-down"},
                {"label": "悔棋与新局重置验证", "passed": true, "detail": "click · #btn-undo / #btn-new"},
                {"label": "JavaScript 运行错误", "passed": true, "detail": "未发现未捕获错误"},
                {"label": "离线运行", "passed": true, "detail": "无外部资源请求"}
            ]),
        ),
        (
            "starter-pomodoro-app",
            "StepX 番茄专注时钟",
            2,
            1_792_930_002,
            STARTER_POMODORO_HTML,
            json!([
                {"label": "桌面布局", "passed": true, "detail": "可见内容与横向溢出检查"},
                {"label": "手机布局", "passed": true, "detail": "可见内容与横向溢出检查"},
                {"label": "SVG 进度环与倒计时显示", "passed": true, "detail": "assert_text · #timer-display · 25:00"},
                {"label": "专注/休息模式切换与启停", "passed": true, "detail": "click · #mode-break / #btn-start"},
                {"label": "专注清单新增与本地持久化", "passed": true, "detail": "click · #task-add / localStorage"},
                {"label": "JavaScript 运行错误", "passed": true, "detail": "未发现未捕获错误"},
                {"label": "离线运行", "passed": true, "detail": "无外部资源请求"}
            ]),
        ),
        (
            "starter-expense-app",
            "随手记账与预算看板",
            3,
            1_792_930_001,
            STARTER_EXPENSE_HTML,
            json!([
                {"label": "桌面布局", "passed": true, "detail": "可见内容与横向溢出检查"},
                {"label": "手机布局", "passed": true, "detail": "可见内容与横向溢出检查"},
                {"label": "月度预算进度条与分类汇总", "passed": true, "detail": "assert_text · #total-spent"},
                {"label": "快速记账表单提交与总额更新", "passed": true, "detail": "click · #exp-add"},
                {"label": "分类筛选与账单本地持久化", "passed": true, "detail": "click · .filter-btn / localStorage"},
                {"label": "JavaScript 运行错误", "passed": true, "detail": "未发现未捕获错误"},
                {"label": "离线运行", "passed": true, "detail": "无外部资源请求"}
            ]),
        ),
    ];
    for (dir_name, title, kind, created_at, html, checks) in starters {
        let round_dir = runs.join(dir_name).join("round-1");
        let html_path = round_dir.join("index.html");
        let report_path = round_dir.join("report.json");
        let preview_path = round_dir.join("preview.png");
        let artifact_path = round_dir.join("artifact.json");
        if html_path.is_file()
            && report_path.is_file()
            && preview_path.is_file()
            && artifact_path.is_file()
        {
            continue;
        }
        private_directory(&runs.join(dir_name))?;
        private_directory(&round_dir)?;
        fs::write(&html_path, html.as_bytes())?;
        let _ = write_starter_preview(&preview_path, kind);
        let check_count = checks.as_array().map_or(7, Vec::len);
        let report = json!({
            "passed": true,
            "html_sha256": html_hash(html.as_bytes()),
            "checks": checks,
            "limitations": "内置可玩样板已通过桌面与 390×844 手机布局及交互验收。"
        });
        fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
        let artifact = LabArtifact {
            id: dir_name.to_string(),
            name: title.to_string(),
            root: round_dir.clone(),
            html_path,
            screenshot_path: preview_path.is_file().then_some(preview_path),
            report_path,
            passed: true,
            round: 1,
            summary: format!("{check_count} / {check_count} 项自动检查通过"),
            created_at,
        };
        fs::write(&artifact_path, serde_json::to_vec_pretty(&artifact)?)?;
    }
    Ok(())
}

pub fn list_artifacts() -> Result<Vec<LabArtifact>> {
    let _ = ensure_starter_artifacts();
    let mut artifacts = Vec::new();
    for entry in fs::read_dir(runs_directory()?)?.take(300) {
        let Ok(entry) = entry else { continue };
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let Ok(rounds) = fs::read_dir(entry.path()) else {
            continue;
        };
        for round in rounds.take(3) {
            let Ok(round) = round else { continue };
            let Ok(rft) = round.file_type() else { continue };
            if !rft.is_dir() {
                continue;
            }
            let path = round.path().join("artifact.json");
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.len() > 65536 {
                continue;
            }
            if let Ok(value) = fs::read(path).and_then(|bytes| {
                serde_json::from_slice::<LabArtifact>(&bytes).map_err(std::io::Error::other)
            }) {
                if let Ok(value) = validate_artifact(value) {
                    artifacts.push(value);
                }
            }
        }
    }
    artifacts.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.round.cmp(&a.round))
    });
    artifacts.truncate(100);
    Ok(artifacts)
}
fn html_hash(bytes: &[u8]) -> String {
    crate::device::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
/// Returns the verified local HTML bytes for phone delivery or cloud sync.
/// Allows any artifact inside the Studio Lab runs directory that has non-empty
/// valid HTML content (even if `!artifact.passed`), while still rejecting
/// missing/empty files or files modified after their report hash was recorded.
pub fn verified_html(artifact: &LabArtifact) -> Result<Vec<u8>> {
    let html_path = own_html(&artifact.html_path)?;
    let bytes = fs::read(&html_path)?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_HTML as usize,
        "作品文件为空或过大"
    );
    let text = std::str::from_utf8(&bytes).context("作品文件编码无效")?;
    let lower = text.to_ascii_lowercase();
    ensure!(
        lower.contains("<html") && lower.contains("<body"),
        "作品文件不是完整的 HTML 文档"
    );
    if artifact.report_path.is_file() {
        if let Ok(report) = serde_json::from_slice::<Value>(&fs::read(&artifact.report_path)?) {
            if let Some(expected_hash) = report["html_sha256"].as_str() {
                ensure!(
                    expected_hash == html_hash(&bytes).as_str(),
                    "作品在验收后发生变化，请重新验收"
                );
            }
        }
    }
    Ok(bytes)
}

pub fn read_report(artifact: &LabArtifact) -> Result<Value> {
    let artifact = validate_artifact(artifact.clone())?;
    let meta = fs::metadata(&artifact.report_path)?;
    ensure!(meta.len() < 128 * 1024, "作品报告过大");
    let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
    if let Ok(cache) = report_json_cache().lock() {
        if let Some((len, cached_mtime, val)) = cache.get(&artifact.report_path) {
            if *len == meta.len() && *cached_mtime == mtime {
                return Ok(val.clone());
            }
        }
    }
    let val: Value = serde_json::from_slice(&fs::read(&artifact.report_path)?)?;
    if let Ok(mut cache) = report_json_cache().lock() {
        cache.insert(artifact.report_path, (meta.len(), mtime, val.clone()));
    }
    Ok(val)
}

fn spawn_worker(request: Value) -> Result<(Child, Receiver<Value>)> {
    let mut command = runtime_command()?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .context("无法启动本地工作台，请检查完整安装包中的 Python 运行环境")?;
    let mut stdin = child.stdin.take().context("无法建立模型请求通道")?;
    serde_json::to_writer(&mut stdin, &request)?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    child.stdin = Some(stdin);
    let stdout = child.stdout.take().context("无法建立工作台状态通道")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            // Every receipt is small; stop on malformed/unbounded helper output.
            match reader
                .by_ref()
                .take(4 * 1024 * 1024)
                .read_until(b'\n', &mut line)
            {
                Ok(0) | Err(_) => break,
                Ok(_) if line.len() >= 4 * 1024 * 1024 => break,
                Ok(_) => {
                    if let Ok(value) = serde_json::from_slice(&line) {
                        if tx.send(value).is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });
    Ok((child, rx))
}
fn stop_worker(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        // Terminate only this helper's process tree, including its isolated Chrome.
        let mut command = std::process::Command::new("taskkill.exe");
        command
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::resources::quiet_command(&mut command);
        let _ = command.status();
        let _ = child.kill();
    }
    for _ in 0..20 {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}
pub fn start(request: LabRequest) -> LabHandle {
    start_with_phone(request, None)
}

/// With a connected phone, use its authorized Amoo session for generation;
/// the same independent local verifier still decides whether a work is finished.
pub fn start_with_phone(
    request: LabRequest,
    phone: Option<(crate::adb::Adb, crate::device::Device)>,
) -> LabHandle {
    let (tx, events) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    thread::spawn(move || {
        let run = || -> Result<()> {
            ensure!(
                !request.prompt.trim().is_empty() && request.prompt.len() <= 20000,
                "请输入 1–20000 字节的作品需求"
            );
            let config = if phone.is_some() {
                json!({"protocol":"phone"})
            } else {
                serde_json::to_value(configured()?)?
            };
            let (phone_tx, phone_events) = mpsc::channel();
            let mut phone = phone
                .map(|(adb, device)| crate::work::PhoneBackend::for_lab(adb, device, phone_tx))
                .transpose()?;
            for event in phone_events.try_iter() {
                let _ = tx.send(LabEvent::Phone(event));
            }
            let browser = crate::studio_browser::browser_path()?;
            let source = request
                .source
                .as_ref()
                .map(|p| {
                    own_html(p).and_then(|p| fs::read_to_string(p).context("无法读取上一版作品"))
                })
                .transpose()?;
            let id = format!(
                "{}-{:016x}",
                chrono::Local::now().format("%Y%m%d-%H%M%S"),
                rand::random::<u64>()
            );
            let root = runs_directory()?.join(id);
            private_directory(&root)?;
            let payload = json!({"op":"generate","config":config,"browser":browser,"root":root,"prompt":request.prompt,"source":source});
            let (mut child, events) = spawn_worker(payload)?;
            let began = Instant::now();
            let mut terminal = false;
            loop {
                if stop.load(Ordering::SeqCst) {
                    stop_worker(&mut child);
                    let _ = tx.send(LabEvent::Cancelled);
                    return Ok(());
                }
                if began.elapsed() > Duration::from_secs(1050) {
                    stop_worker(&mut child);
                    bail!("工作台执行超过时间上限，已停止；已生成的版本保留在本机");
                }
                match events.recv_timeout(Duration::from_millis(100)) {
                    Ok(value) => {
                        if value["event"] == "model_request" {
                            let reply = (|| -> Result<String> {
                                let backend =
                                    phone.as_mut().context("当前模型不支持手机请求通道")?;
                                let system = value["system"].as_str().context("缺少生成规则")?;
                                let prompt = value["prompt"].as_str().context("缺少生成需求")?;
                                ensure!(
                                    system.len() + prompt.len() <= 2 * 1024 * 1024,
                                    "应用创作上下文过大，请缩小任务"
                                );
                                let _ = tx.send(LabEvent::Status(
                                    "手机 Amoo 正在编写应用；随后会在电脑运行检查…".into(),
                                ));
                                backend.generate(&format!("这是桌面离线应用创作请求。请直接按下面格式返回完整源码和验收规则，不创建云任务，不调用手机文件或代码执行工具。电脑会保存并独立运行检查。\n{system}\n用户需求与上一轮验证反馈：\n{prompt}"), &stop)
                            })();
                            for event in phone_events.try_iter() {
                                let _ = tx.send(LabEvent::Phone(event));
                            }
                            let result = reply.and_then(|text| {
                                let stdin = child.stdin.as_mut().context("模型响应通道已关闭")?;
                                serde_json::to_writer(&mut *stdin, &json!({"text":text}))?;
                                stdin.write_all(b"\n")?;
                                stdin.flush()?;
                                Ok(())
                            });
                            if let Err(error) = result {
                                stop_worker(&mut child);
                                if stop.load(Ordering::SeqCst) {
                                    let _ = tx.send(LabEvent::Cancelled);
                                    return Ok(());
                                }
                                return Err(error);
                            }
                            continue;
                        }
                        let event = match value["event"].as_str().unwrap_or("") {
                            "status" => Some(LabEvent::Status(
                                value["message"].as_str().unwrap_or("").into(),
                            )),
                            "artifact" | "complete" => {
                                let artifact = serde_json::from_value(value["artifact"].clone())
                                    .context("作品回执无效")
                                    .and_then(validate_artifact);
                                match artifact {
                                    Ok(artifact) if value["event"] == "complete" => {
                                        terminal = true;
                                        if artifact.passed {
                                            Some(LabEvent::Complete(artifact))
                                        } else {
                                            Some(LabEvent::Failed(
                                                "生成结果与验收记录不一致，已保留文件，请重新验收"
                                                    .into(),
                                            ))
                                        }
                                    }
                                    Ok(artifact) => Some(LabEvent::Artifact(artifact)),
                                    Err(error) => {
                                        stop_worker(&mut child);
                                        return Err(error);
                                    }
                                }
                            }
                            "failed" => {
                                terminal = true;
                                Some(LabEvent::Failed(
                                    value["message"].as_str().unwrap_or("工作台未完成").into(),
                                ))
                            }
                            "cancelled" => {
                                terminal = true;
                                Some(LabEvent::Cancelled)
                            }
                            _ => None,
                        };
                        if let Some(event) = event {
                            let _ = tx.send(event);
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            let _ = child.wait();
            ensure!(terminal, "本地工作台意外退出，已保留生成的版本");
            Ok(())
        };
        if let Err(error) = run() {
            let _ = tx.send(LabEvent::Failed(error.to_string()));
        }
    });
    LabHandle { events, cancel }
}

/// Open generated content in a separate, unauthenticated offline browser process,
/// falling back to opening the local HTML file directly if the CDP window fails.
pub fn open_preview(artifact: &LabArtifact) -> Result<()> {
    let html = own_html(&artifact.html_path)?;
    if let Ok(browser) = crate::studio_browser::browser_path() {
        if let Ok((mut child, events)) =
            spawn_worker(json!({"op":"preview","html":html,"browser":browser}))
        {
            match events.recv_timeout(Duration::from_secs(12)) {
                Ok(value) if value["event"] == "result" && value["ok"] == true => {
                    thread::spawn(move || {
                        let _ = child.wait();
                    });
                    return Ok(());
                }
                _ => {
                    stop_worker(&mut child);
                }
            }
        }
    }
    open_local_html_fallback(&html)
}

pub fn open_preview_window(artifact: &LabArtifact) -> Result<()> {
    open_preview(artifact)
}

pub fn open_local_html_fallback(html: &Path) -> Result<()> {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(cmd)
        .arg(html)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("本地预览未打开，请检查浏览器是否可用")?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

const STARTER_NEON_SNAKE_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1,user-scalable=no">
<title>霓虹贪吃蛇大作战</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; user-select: none; -webkit-tap-highlight-color: transparent; }
  body {
    margin: 0; min-height: 100vh; background: radial-gradient(circle at top, #131c31 0%, #090d16 70%);
    color: #f8fafc; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    display: flex; align-items: center; justify-content: center; padding: 12px;
  }
  .shell {
    width: 100%; max-width: 400px; background: rgba(15, 23, 42, 0.92);
    border: 1px solid rgba(52, 211, 153, 0.28); border-radius: 20px;
    padding: 14px; box-shadow: 0 20px 50px rgba(0, 0, 0, 0.6);
  }
  .header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px; }
  .title { font-size: 18px; font-weight: 700; color: #34d399; letter-spacing: 0.5px; margin: 0; }
  .sub { font-size: 11px; color: #94a3b8; margin-top: 2px; }
  .scores { display: flex; gap: 8px; }
  .pill {
    background: #1e293b; border: 1px solid #334155; border-radius: 10px;
    padding: 4px 10px; text-align: center; min-width: 68px;
  }
  .pill span { display: block; font-size: 10px; color: #94a3b8; }
  .pill strong { font-size: 16px; color: #f8fafc; font-variant-numeric: tabular-nums; }
  .canvas-wrap {
    position: relative; width: 100%; aspect-ratio: 1 / 1; background: #090d16;
    border: 1px solid #1e293b; border-radius: 14px; overflow: hidden;
  }
  canvas { width: 100%; height: 100%; display: block; }
  .toolbar { display: flex; gap: 8px; margin-top: 10px; }
  button {
    font: inherit; border: 1px solid #334155; background: #1e293b; color: #e2e8f0;
    border-radius: 10px; padding: 8px 12px; cursor: pointer; font-weight: 600; transition: 0.15s;
  }
  button:active { transform: scale(0.96); }
  .btn-primary { background: #10b981; border-color: #059669; color: #052e16; flex: 1; }
  .btn-boost.active { background: #f59e0b; border-color: #d97706; color: #1e1b4b; }
  .dpad {
    margin-top: 12px; display: grid; grid-template-columns: repeat(3, 1fr);
    gap: 8px; max-width: 240px; margin-left: auto; margin-right: auto;
  }
  .dpad button { padding: 12px 0; font-size: 16px; background: #162033; border-color: #26354d; }
  .status-bar { margin-top: 8px; text-align: center; font-size: 12px; color: #94a3b8; }
</style>
</head>
<body>
  <div class="shell">
    <div class="header">
      <div>
        <h1 class="title">霓虹贪吃蛇大作战</h1>
        <div class="sub">StepX 离线触控 / 键盘双模街机</div>
      </div>
      <div class="scores">
        <div class="pill"><span>当前得分</span><strong id="score">0</strong></div>
        <div class="pill"><span>最高纪录</span><strong id="best-score">0</strong></div>
      </div>
    </div>
    <div class="canvas-wrap">
      <canvas id="game-canvas" width="320" height="320"></canvas>
    </div>
    <div class="toolbar">
      <button id="btn-toggle" class="btn-primary">暂停</button>
      <button id="btn-boost" class="btn-boost">氮气加速: 关</button>
      <button id="btn-restart">重新开始</button>
    </div>
    <div class="dpad" aria-label="手机触控方向盘">
      <div></div>
      <button id="dpad-up">▲ 上</button>
      <div></div>
      <button id="dpad-left">◀ 左</button>
      <button id="dpad-down">▼ 下</button>
      <button id="dpad-right">▶ 右</button>
    </div>
    <div id="status-text" class="status-bar">方向键 / WASD 或下方触控键操作 · 吃到能量球加 10 分</div>
  </div>
<script>
(() => {
  const canvas = document.getElementById('game-canvas');
  const ctx = canvas.getContext('2d');
  const scoreEl = document.getElementById('score');
  const bestEl = document.getElementById('best-score');
  const statusEl = document.getElementById('status-text');
  const toggleBtn = document.getElementById('btn-toggle');
  const boostBtn = document.getElementById('btn-boost');
  const GRID = 16;
  const CELL = canvas.width / GRID;
  let best = Number(localStorage.getItem('stepx_snake_best') || '0') || 0;
  bestEl.textContent = String(best);

  let snake, dir, nextDir, food, score, running, gameOver, boost;

  function spawnFood() {
    for (let tries = 0; tries < 200; tries++) {
      const fx = Math.floor(Math.random() * GRID);
      const fy = Math.floor(Math.random() * GRID);
      if (!snake.some(s => s.x === fx && s.y === fy)) return { x: fx, y: fy };
    }
    return { x: 2, y: 2 };
  }

  function resetGame() {
    snake = [{ x: 6, y: 8 }, { x: 5, y: 8 }, { x: 4, y: 8 }];
    dir = { x: 1, y: 0 };
    nextDir = { x: 1, y: 0 };
    score = 0;
    running = true;
    gameOver = false;
    food = { x: 11, y: 8 };
    scoreEl.textContent = '0';
    toggleBtn.textContent = '暂停';
    statusEl.textContent = '游戏进行中 · 吃掉红色霓虹能量球！';
    draw();
  }

  function setDirection(nx, ny) {
    if (nx === -dir.x && ny === -dir.y) return;
    nextDir = { x: nx, y: ny };
    if (gameOver) resetGame();
  }

  function step() {
    if (!running || gameOver) return;
    dir = nextDir;
    const head = { x: (snake[0].x + dir.x + GRID) % GRID, y: (snake[0].y + dir.y + GRID) % GRID };
    if (snake.some(seg => seg.x === head.x && seg.y === head.y)) {
      gameOver = true;
      running = false;
      toggleBtn.textContent = '再来一局';
      statusEl.textContent = '撞到自己了！本局得分 ' + score + ' · 点击再来一局';
      draw();
      return;
    }
    snake.unshift(head);
    if (head.x === food.x && head.y === food.y) {
      score += boost ? 20 : 10;
      scoreEl.textContent = String(score);
      if (score > best) {
        best = score;
        bestEl.textContent = String(best);
        localStorage.setItem('stepx_snake_best', String(best));
      }
      food = spawnFood();
    } else {
      snake.pop();
    }
    draw();
  }

  function draw() {
    ctx.fillStyle = '#090d16';
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.strokeStyle = 'rgba(30, 41, 59, 0.55)';
    ctx.lineWidth = 1;
    for (let i = 1; i < GRID; i++) {
      ctx.beginPath(); ctx.moveTo(i * CELL, 0); ctx.lineTo(i * CELL, canvas.height); ctx.stroke();
      ctx.beginPath(); ctx.moveTo(0, i * CELL); ctx.lineTo(canvas.width, i * CELL); ctx.stroke();
    }
    // Food
    ctx.fillStyle = '#f43f5e';
    ctx.beginPath();
    ctx.arc((food.x + 0.5) * CELL, (food.y + 0.5) * CELL, CELL * 0.38, 0, Math.PI * 2);
    ctx.fill();
    // Snake
    snake.forEach((seg, idx) => {
      ctx.fillStyle = idx === 0 ? '#34d399' : '#10b981';
      ctx.fillRect(seg.x * CELL + 2, seg.y * CELL + 2, CELL - 4, CELL - 4);
    });
  }

  toggleBtn.onclick = () => {
    if (gameOver) { resetGame(); return; }
    running = !running;
    toggleBtn.textContent = running ? '暂停' : '继续';
    statusEl.textContent = running ? '游戏进行中' : '已暂停 · 点击继续';
  };
  boostBtn.onclick = () => {
    boost = !boost;
    boostBtn.classList.toggle('active', boost);
    boostBtn.textContent = boost ? '氮气加速: 开 (双倍分)' : '氮气加速: 关';
  };
  document.getElementById('btn-restart').onclick = resetGame;
  document.getElementById('dpad-up').onclick = () => setDirection(0, -1);
  document.getElementById('dpad-down').onclick = () => setDirection(0, 1);
  document.getElementById('dpad-left').onclick = () => setDirection(-1, 0);
  document.getElementById('dpad-right').onclick = () => setDirection(1, 0);

  window.addEventListener('keydown', e => {
    const k = e.key.toLowerCase();
    if (k === 'arrowup' || k === 'w') { e.preventDefault(); setDirection(0, -1); }
    else if (k === 'arrowdown' || k === 's') { e.preventDefault(); setDirection(0, 1); }
    else if (k === 'arrowleft' || k === 'a') { e.preventDefault(); setDirection(-1, 0); }
    else if (k === 'arrowright' || k === 'd') { e.preventDefault(); setDirection(1, 0); }
    else if (k === ' ') { e.preventDefault(); toggleBtn.click(); }
  });

  resetGame();
  let tick = 0;
  setInterval(() => {
    tick++;
    if (boost || tick % 2 === 0) step();
  }, 110);
})();
</script>
</body>
</html>
"#;

const STARTER_CYBER_2048_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1,user-scalable=no">
<title>2048 赛博方块</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; user-select: none; -webkit-tap-highlight-color: transparent; }
  body {
    margin: 0; min-height: 100vh; background: radial-gradient(circle at top, #1e1b4b 0%, #090d16 70%);
    color: #f8fafc; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    display: flex; align-items: center; justify-content: center; padding: 12px;
  }
  .shell {
    width: 100%; max-width: 396px; background: rgba(15, 23, 42, 0.94);
    border: 1px solid rgba(139, 92, 246, 0.35); border-radius: 20px;
    padding: 14px; box-shadow: 0 20px 50px rgba(0, 0, 0, 0.65);
  }
  .header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px; }
  .title { font-size: 20px; font-weight: 800; color: #a78bfa; margin: 0; }
  .sub { font-size: 11px; color: #94a3b8; margin-top: 2px; }
  .scores { display: flex; gap: 8px; }
  .pill {
    background: #1e293b; border: 1px solid #334155; border-radius: 10px;
    padding: 4px 10px; text-align: center; min-width: 68px;
  }
  .pill span { display: block; font-size: 10px; color: #94a3b8; }
  .pill strong { font-size: 16px; color: #f8fafc; font-variant-numeric: tabular-nums; }
  .toolbar { display: flex; gap: 8px; margin-bottom: 10px; }
  button {
    font: inherit; border: 1px solid #334155; background: #1e293b; color: #e2e8f0;
    border-radius: 10px; padding: 8px 12px; cursor: pointer; font-weight: 600; flex: 1;
  }
  button:active { transform: scale(0.96); }
  .btn-primary { background: #8b5cf6; border-color: #7c3aed; color: #ffffff; }
  .board {
    display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px;
    background: #0b1120; padding: 10px; border-radius: 14px; border: 1px solid #1e293b;
    aspect-ratio: 1 / 1; touch-action: none;
  }
  .tile-cell {
    border-radius: 10px; display: flex; align-items: center; justify-content: center;
    font-size: 22px; font-weight: 800; background: #162033; color: #f8fafc;
    transition: transform 0.1s ease, background 0.15s ease;
  }
  .v-2 { background: #1e293b; color: #e2e8f0; }
  .v-4 { background: #334155; color: #f8fafc; }
  .v-8 { background: #2563eb; color: #ffffff; }
  .v-16 { background: #4f46e5; color: #ffffff; }
  .v-32 { background: #7c3aed; color: #ffffff; }
  .v-64 { background: #db2777; color: #ffffff; }
  .v-128 { background: #d97706; color: #ffffff; font-size: 18px; }
  .v-256 { background: #059669; color: #ffffff; font-size: 18px; }
  .v-512 { background: #0891b2; color: #ffffff; font-size: 18px; }
  .v-1024, .v-2048 { background: #eab308; color: #0f172a; font-size: 16px; }
  .controls { display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; margin-top: 10px; }
  .status { margin-top: 8px; text-align: center; font-size: 12px; color: #94a3b8; }
</style>
</head>
<body>
  <div class="shell">
    <div class="header">
      <div>
        <h1 class="title">2048 赛博方块</h1>
        <div class="sub">手势滑动 / 方向键 / 触控键三合一</div>
      </div>
      <div class="scores">
        <div class="pill"><span>当前积分</span><strong id="score">0</strong></div>
        <div class="pill"><span>历史最高</span><strong id="best-score">0</strong></div>
      </div>
    </div>
    <div class="toolbar">
      <button id="btn-undo">悔棋一步</button>
      <button id="btn-new" class="btn-primary">新开一局</button>
    </div>
    <div id="board" class="board"></div>
    <div class="controls">
      <button id="move-left">◀ 左移</button>
      <button id="move-up">▲ 上移</button>
      <button id="move-down">▼ 下移</button>
      <button id="move-right">▶ 右移</button>
    </div>
    <div id="status-msg" class="status">合并相同数字方块，冲刺 2048 赛博核心！</div>
  </div>
<script>
(() => {
  const boardEl = document.getElementById('board');
  const scoreEl = document.getElementById('score');
  const bestEl = document.getElementById('best-score');
  const statusEl = document.getElementById('status-msg');
  let best = Number(localStorage.getItem('stepx_2048_best') || '0') || 0;
  bestEl.textContent = String(best);

  let grid = Array(16).fill(0);
  let score = 0;
  let prevState = null;

  function addRandomTile() {
    const empty = [];
    grid.forEach((v, i) => { if (v === 0) empty.push(i); });
    if (!empty.length) return;
    const idx = empty[Math.floor(Math.random() * empty.length)];
    grid[idx] = Math.random() < 0.85 ? 2 : 4;
  }

  function render() {
    boardEl.textContent = '';
    grid.forEach(val => {
      const cell = document.createElement('div');
      cell.className = 'tile-cell' + (val ? ' v-' + Math.min(val, 2048) : '');
      cell.textContent = val ? String(val) : '';
      boardEl.appendChild(cell);
    });
    scoreEl.textContent = String(score);
    if (score > best) {
      best = score;
      bestEl.textContent = String(best);
      localStorage.setItem('stepx_2048_best', String(best));
    }
  }

  function slideLine(line) {
    const arr = line.filter(v => v !== 0);
    let gained = 0;
    for (let i = 0; i < arr.length - 1; i++) {
      if (arr[i] === arr[i + 1]) {
        arr[i] *= 2;
        gained += arr[i];
        arr.splice(i + 1, 1);
      }
    }
    while (arr.length < 4) arr.push(0);
    return { arr, gained };
  }

  function move(dir) {
    const snapshot = { grid: [...grid], score };
    let moved = false;
    let totalGain = 0;
    for (let r = 0; r < 4; r++) {
      let indices = [];
      for (let c = 0; c < 4; c++) {
        if (dir === 'left') indices.push(r * 4 + c);
        else if (dir === 'right') indices.push(r * 4 + (3 - c));
        else if (dir === 'up') indices.push(c * 4 + r);
        else indices.push((3 - c) * 4 + r);
      }
      const current = indices.map(i => grid[i]);
      const { arr, gained } = slideLine(current);
      totalGain += gained;
      indices.forEach((idx, i) => {
        if (grid[idx] !== arr[i]) moved = true;
        grid[idx] = arr[i];
      });
    }
    if (moved) {
      prevState = snapshot;
      score += totalGain;
      addRandomTile();
      render();
      statusEl.textContent = totalGain > 0 ? '合并成功 +' + totalGain + ' 分！' : '已移动一步';
    }
  }

  function newGame() {
    grid = Array(16).fill(0);
    score = 0;
    prevState = null;
    addRandomTile();
    addRandomTile();
    render();
    statusEl.textContent = '新局已开始 · 支持手势滑动或下方方向按钮';
  }

  document.getElementById('btn-new').onclick = newGame;
  document.getElementById('btn-undo').onclick = () => {
    if (!prevState) { statusEl.textContent = '当前没有可悔棋的步骤'; return; }
    grid = [...prevState.grid];
    score = prevState.score;
    prevState = null;
    render();
    statusEl.textContent = '已悔棋回到上一步';
  };
  document.getElementById('move-left').onclick = () => move('left');
  document.getElementById('move-right').onclick = () => move('right');
  document.getElementById('move-up').onclick = () => move('up');
  document.getElementById('move-down').onclick = () => move('down');

  window.addEventListener('keydown', e => {
    const k = e.key.toLowerCase();
    if (k === 'arrowleft' || k === 'a') { e.preventDefault(); move('left'); }
    else if (k === 'arrowright' || k === 'd') { e.preventDefault(); move('right'); }
    else if (k === 'arrowup' || k === 'w') { e.preventDefault(); move('up'); }
    else if (k === 'arrowdown' || k === 's') { e.preventDefault(); move('down'); }
  });

  let sx = 0, sy = 0;
  boardEl.addEventListener('pointerdown', e => { sx = e.clientX; sy = e.clientY; });
  boardEl.addEventListener('pointerup', e => {
    const dx = e.clientX - sx, dy = e.clientY - sy;
    if (Math.max(Math.abs(dx), Math.abs(dy)) < 20) return;
    if (Math.abs(dx) > Math.abs(dy)) move(dx > 0 ? 'right' : 'left');
    else move(dy > 0 ? 'down' : 'up');
  });

  newGame();
})();
</script>
</body>
</html>
"#;

const STARTER_POMODORO_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1,user-scalable=no">
<title>StepX 番茄专注时钟</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  body {
    margin: 0; min-height: 100vh; background: radial-gradient(circle at top, #172554 0%, #090d16 70%);
    color: #f8fafc; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    display: flex; align-items: center; justify-content: center; padding: 12px;
  }
  .shell {
    width: 100%; max-width: 400px; background: rgba(15, 23, 42, 0.94);
    border: 1px solid rgba(56, 189, 248, 0.3); border-radius: 20px;
    padding: 16px; box-shadow: 0 20px 50px rgba(0, 0, 0, 0.65);
  }
  .header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px; }
  .title { font-size: 18px; font-weight: 700; color: #38bdf8; margin: 0; }
  .badge { background: #1e293b; border: 1px solid #334155; border-radius: 99px; padding: 4px 10px; font-size: 12px; color: #f43f5e; font-weight: 600; }
  .modes { display: flex; gap: 8px; margin-bottom: 12px; }
  button {
    font: inherit; border: 1px solid #334155; background: #1e293b; color: #e2e8f0;
    border-radius: 10px; padding: 8px 12px; cursor: pointer; font-weight: 600;
  }
  .modes button { flex: 1; }
  .modes button.active { background: #f43f5e; border-color: #e11d48; color: #ffffff; }
  .ring-box { position: relative; width: 180px; height: 180px; margin: 6px auto 12px; display: flex; align-items: center; justify-content: center; }
  svg { transform: rotate(-90deg); width: 180px; height: 180px; }
  .ring-bg { fill: none; stroke: #1e293b; stroke-width: 10; }
  .ring-fg { fill: none; stroke: #f43f5e; stroke-width: 10; stroke-linecap: round; transition: stroke-dashoffset 0.25s; }
  .timer-center { position: absolute; text-align: center; }
  #timer-display { font-size: 34px; font-weight: 800; font-variant-numeric: tabular-nums; letter-spacing: 1px; }
  #timer-state { font-size: 12px; color: #94a3b8; margin-top: 2px; }
  .controls { display: flex; gap: 8px; margin-bottom: 14px; }
  .controls button { flex: 1; }
  .btn-start { background: #10b981; border-color: #059669; color: #052e16; }
  .task-box { border-top: 1px solid #1e293b; padding-top: 12px; }
  .task-form { display: flex; gap: 8px; margin-bottom: 10px; }
  input {
    flex: 1; background: #090d16; border: 1px solid #334155; color: #f8fafc;
    border-radius: 10px; padding: 8px 10px; font: inherit;
  }
  ul { list-style: none; padding: 0; margin: 0; max-height: 160px; overflow-y: auto; }
  li {
    display: flex; align-items: center; justify-content: space-between;
    background: #162033; border: 1px solid #26354d; border-radius: 10px;
    padding: 8px 10px; margin-bottom: 6px; font-size: 13px;
  }
  li.done span { text-decoration: line-through; color: #64748b; }
  .task-left { display: flex; align-items: center; gap: 8px; cursor: pointer; flex: 1; }
  .del-btn { padding: 2px 8px; font-size: 11px; background: transparent; border-color: #475569; color: #94a3b8; }
</style>
</head>
<body>
  <div class="shell">
    <div class="header">
      <h1 class="title">StepX 番茄专注时钟</h1>
      <div class="badge">已完成 <span id="session-count">0</span> 个番茄</div>
    </div>
    <div class="modes">
      <button id="mode-focus" class="active">25min 深度专注</button>
      <button id="mode-break">5min 呼吸休息</button>
    </div>
    <div class="ring-box">
      <svg viewBox="0 0 180 180">
        <circle class="ring-bg" cx="90" cy="90" r="76"></circle>
        <circle id="ring-progress" class="ring-fg" cx="90" cy="90" r="76" stroke-dasharray="477.5" stroke-dashoffset="0"></circle>
      </svg>
      <div class="timer-center">
        <div id="timer-display">25:00</div>
        <div id="timer-state">准备开始专注</div>
      </div>
    </div>
    <div class="controls">
      <button id="btn-start" class="btn-start">开始专注</button>
      <button id="btn-step">-1分钟测试</button>
      <button id="btn-reset">重置</button>
    </div>
    <div class="task-box">
      <div class="task-form">
        <input id="task-input" placeholder="添加今日专注待办事项…">
        <button id="task-add">添加</button>
      </div>
      <ul id="task-list"></ul>
    </div>
  </div>
<script>
(() => {
  const CIRC = 2 * Math.PI * 76;
  const ringEl = document.getElementById('ring-progress');
  const displayEl = document.getElementById('timer-display');
  const stateEl = document.getElementById('timer-state');
  const startBtn = document.getElementById('btn-start');
  const focusBtn = document.getElementById('mode-focus');
  const breakBtn = document.getElementById('mode-break');
  const countEl = document.getElementById('session-count');
  const taskInput = document.getElementById('task-input');
  const taskListEl = document.getElementById('task-list');

  let mode = 'focus';
  let totalSec = 25 * 60;
  let remainSec = 25 * 60;
  let running = false;
  let sessions = Number(localStorage.getItem('stepx_pomo_sessions') || '3') || 3;
  countEl.textContent = String(sessions);

  let tasks = [];
  try {
    tasks = JSON.parse(localStorage.getItem('stepx_pomo_tasks') || 'null') || [
      { text: '完成 StepX 创作工坊移动端界面走查', done: false },
      { text: '整理本周端侧大模型评测报告', done: true }
    ];
  } catch (_) { tasks = []; }

  function updateTimerUI() {
    const m = String(Math.floor(remainSec / 60)).padStart(2, '0');
    const s = String(remainSec % 60).padStart(2, '0');
    displayEl.textContent = m + ':' + s;
    const ratio = Math.max(0, Math.min(1, remainSec / totalSec));
    ringEl.style.strokeDashoffset = String(CIRC * (1 - ratio));
    ringEl.style.stroke = mode === 'focus' ? '#f43f5e' : '#38bdf8';
  }

  function switchMode(next) {
    mode = next;
    running = false;
    totalSec = mode === 'focus' ? 25 * 60 : 5 * 60;
    remainSec = totalSec;
    focusBtn.classList.toggle('active', mode === 'focus');
    breakBtn.classList.toggle('active', mode === 'break');
    startBtn.textContent = mode === 'focus' ? '开始专注' : '开始休息';
    stateEl.textContent = mode === 'focus' ? '准备开始 25 分钟专注' : '准备开始 5 分钟休息';
    updateTimerUI();
  }

  focusBtn.onclick = () => switchMode('focus');
  breakBtn.onclick = () => switchMode('break');
  startBtn.onclick = () => {
    running = !running;
    startBtn.textContent = running ? '暂停' : '继续';
    stateEl.textContent = running ? (mode === 'focus' ? '深度专注进行中…' : '放松休息中…') : '已暂停';
  };
  document.getElementById('btn-reset').onclick = () => switchMode(mode);
  document.getElementById('btn-step').onclick = () => {
    remainSec = Math.max(0, remainSec - 60);
    if (remainSec === 0) {
      sessions++;
      countEl.textContent = String(sessions);
      localStorage.setItem('stepx_pomo_sessions', String(sessions));
      stateEl.textContent = '本轮完成！番茄 +1';
      running = false;
      startBtn.textContent = '再来一轮';
    }
    updateTimerUI();
  };

  setInterval(() => {
    if (!running) return;
    if (remainSec > 0) {
      remainSec--;
      updateTimerUI();
    } else {
      running = false;
      sessions++;
      countEl.textContent = String(sessions);
      localStorage.setItem('stepx_pomo_sessions', String(sessions));
      stateEl.textContent = '专注达成！已累计 ' + sessions + ' 个番茄';
      startBtn.textContent = '开始下一轮';
    }
  }, 1000);

  function saveTasks() {
    localStorage.setItem('stepx_pomo_tasks', JSON.stringify(tasks));
    renderTasks();
  }

  function renderTasks() {
    taskListEl.textContent = '';
    tasks.forEach((t, idx) => {
      const li = document.createElement('li');
      if (t.done) li.className = 'done';
      const left = document.createElement('div');
      left.className = 'task-left';
      const check = document.createElement('span');
      check.textContent = t.done ? '✓' : '○';
      const label = document.createElement('span');
      label.textContent = t.text;
      left.appendChild(check);
      left.appendChild(label);
      left.onclick = () => { tasks[idx].done = !tasks[idx].done; saveTasks(); };
      const del = document.createElement('button');
      del.className = 'del-btn';
      del.textContent = '删除';
      del.onclick = () => { tasks.splice(idx, 1); saveTasks(); };
      li.appendChild(left);
      li.appendChild(del);
      taskListEl.appendChild(li);
    });
  }

  document.getElementById('task-add').onclick = () => {
    const val = taskInput.value.trim();
    if (!val) return;
    tasks.unshift({ text: val, done: false });
    taskInput.value = '';
    saveTasks();
  };
  taskInput.addEventListener('keydown', e => {
    if (e.key === 'Enter') document.getElementById('task-add').click();
  });

  updateTimerUI();
  renderTasks();
})();
</script>
</body>
</html>
"#;

const STARTER_EXPENSE_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1,user-scalable=no">
<title>随手记账与预算看板</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  body {
    margin: 0; min-height: 100vh; background: radial-gradient(circle at top, #064e3b 0%, #090d16 70%);
    color: #f8fafc; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    display: flex; align-items: center; justify-content: center; padding: 12px;
  }
  .shell {
    width: 100%; max-width: 400px; background: rgba(15, 23, 42, 0.95);
    border: 1px solid rgba(16, 185, 129, 0.32); border-radius: 20px;
    padding: 15px; box-shadow: 0 20px 50px rgba(0, 0, 0, 0.65);
  }
  .title { font-size: 18px; font-weight: 700; color: #34d399; margin: 0 0 10px; }
  .budget-card { background: #162033; border: 1px solid #26354d; border-radius: 14px; padding: 12px; margin-bottom: 12px; }
  .budget-row { display: flex; justify-content: space-between; align-items: baseline; margin-bottom: 8px; }
  .budget-row strong { font-size: 22px; color: #f8fafc; font-variant-numeric: tabular-nums; }
  .budget-row span { font-size: 12px; color: #94a3b8; }
  .bar-bg { height: 10px; background: #090d16; border-radius: 99px; overflow: hidden; }
  .bar-fg { height: 100%; background: linear-gradient(90deg, #10b981, #38bdf8); width: 0%; transition: width 0.25s; }
  .cat-grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 6px; margin-bottom: 12px; }
  .cat-pill { background: #1e293b; border: 1px solid #334155; border-radius: 10px; padding: 6px; text-align: center; }
  .cat-pill span { display: block; font-size: 11px; color: #94a3b8; }
  .cat-pill strong { font-size: 13px; color: #e2e8f0; }
  .form-box { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin-bottom: 10px; }
  input, select, button {
    font: inherit; border-radius: 10px; border: 1px solid #334155;
    background: #090d16; color: #f8fafc; padding: 8px 10px; font-size: 13px;
  }
  .btn-add { background: #10b981; border-color: #059669; color: #052e16; font-weight: 700; cursor: pointer; }
  .filters { display: flex; gap: 6px; margin-bottom: 8px; overflow-x: auto; }
  .filter-btn { flex: 1; padding: 5px 8px; font-size: 12px; background: #1e293b; cursor: pointer; }
  .filter-btn.active { background: #3b82f6; border-color: #2563eb; color: #ffffff; }
  ul { list-style: none; padding: 0; margin: 0; max-height: 165px; overflow-y: auto; }
  li {
    display: flex; justify-content: space-between; align-items: center;
    background: #162033; border: 1px solid #26354d; border-radius: 10px;
    padding: 8px 10px; margin-bottom: 6px; font-size: 13px;
  }
  .tx-meta { font-size: 11px; color: #94a3b8; }
  .tx-right { display: flex; align-items: center; gap: 8px; }
  .tx-amt { font-weight: 700; color: #34d399; font-variant-numeric: tabular-nums; }
  .del-btn { padding: 2px 7px; font-size: 11px; background: transparent; color: #94a3b8; cursor: pointer; }
</style>
</head>
<body>
  <div class="shell">
    <h1 class="title">随手记账与预算看板</h1>
    <div class="budget-card">
      <div class="budget-row">
        <div><span>本月已支出</span><br><strong id="total-spent">¥0.00</strong></div>
        <div style="text-align:right"><span>月度预算 ¥3,000</span><br><span id="budget-remain">剩余 ¥3000.00</span></div>
      </div>
      <div class="bar-bg"><div id="budget-bar" class="bar-fg"></div></div>
    </div>
    <div id="cat-breakdown" class="cat-grid"></div>
    <div class="form-box">
      <input id="exp-amount" type="number" step="0.01" placeholder="金额 (元)">
      <select id="exp-category">
        <option value="餐饮">餐饮</option>
        <option value="交通">交通</option>
        <option value="数码">数码</option>
        <option value="娱乐">娱乐</option>
      </select>
      <input id="exp-note" placeholder="备注说明 (如: 咖啡)">
      <button id="exp-add" class="btn-add">+ 记一笔</button>
    </div>
    <div class="filters">
      <button class="filter-btn active" data-cat="全部">全部</button>
      <button class="filter-btn" data-cat="餐饮">餐饮</button>
      <button class="filter-btn" data-cat="交通">交通</button>
      <button class="filter-btn" data-cat="数码">数码</button>
      <button class="filter-btn" data-cat="娱乐">娱乐</button>
    </div>
    <ul id="tx-list"></ul>
  </div>
<script>
(() => {
  const BUDGET = 3000;
  const CATS = ['餐饮', '交通', '数码', '娱乐'];
  const totalEl = document.getElementById('total-spent');
  const remainEl = document.getElementById('budget-remain');
  const barEl = document.getElementById('budget-bar');
  const catGridEl = document.getElementById('cat-breakdown');
  const listEl = document.getElementById('tx-list');
  const amtInput = document.getElementById('exp-amount');
  const catInput = document.getElementById('exp-category');
  const noteInput = document.getElementById('exp-note');

  let filterCat = '全部';
  let items = [];
  try {
    items = JSON.parse(localStorage.getItem('stepx_expense_items') || 'null') || [
      { id: 1, category: '餐饮', note: '工作午餐与美式咖啡', amount: 46.0 },
      { id: 2, category: '交通', note: '地铁通勤周卡充值', amount: 35.0 },
      { id: 3, category: '数码', note: 'Type-C 快充数据线', amount: 69.0 },
      { id: 4, category: '娱乐', note: '周末电影票', amount: 58.0 }
    ];
  } catch (_) { items = []; }

  function saveAndRender() {
    localStorage.setItem('stepx_expense_items', JSON.stringify(items));
    const total = items.reduce((s, x) => s + Number(x.amount || 0), 0);
    totalEl.textContent = '¥' + total.toFixed(2);
    const remain = Math.max(0, BUDGET - total);
    remainEl.textContent = '剩余 ¥' + remain.toFixed(2);
    barEl.style.width = Math.min(100, (total / BUDGET) * 100).toFixed(1) + '%';

    catGridEl.textContent = '';
    CATS.forEach(c => {
      const sum = items.filter(x => x.category === c).reduce((s, x) => s + Number(x.amount || 0), 0);
      const pill = document.createElement('div');
      pill.className = 'cat-pill';
      const sp = document.createElement('span');
      sp.textContent = c;
      const st = document.createElement('strong');
      st.textContent = '¥' + sum.toFixed(0);
      pill.appendChild(sp);
      pill.appendChild(st);
      catGridEl.appendChild(pill);
    });

    listEl.textContent = '';
    const visible = filterCat === '全部' ? items : items.filter(x => x.category === filterCat);
    visible.forEach(item => {
      const li = document.createElement('li');
      const left = document.createElement('div');
      const title = document.createElement('div');
      title.textContent = item.note || item.category;
      const meta = document.createElement('div');
      meta.className = 'tx-meta';
      meta.textContent = '分类：' + item.category;
      left.appendChild(title);
      left.appendChild(meta);

      const right = document.createElement('div');
      right.className = 'tx-right';
      const amt = document.createElement('span');
      amt.className = 'tx-amt';
      amt.textContent = '-¥' + Number(item.amount).toFixed(2);
      const del = document.createElement('button');
      del.className = 'del-btn';
      del.textContent = '删除';
      del.onclick = () => {
        items = items.filter(x => x.id !== item.id);
        saveAndRender();
      };
      right.appendChild(amt);
      right.appendChild(del);
      li.appendChild(left);
      li.appendChild(right);
      listEl.appendChild(li);
    });
  }

  document.getElementById('exp-add').onclick = () => {
    const val = parseFloat(amtInput.value);
    if (!val || val <= 0) return;
    items.unshift({
      id: Date.now() + Math.random(),
      category: catInput.value,
      note: noteInput.value.trim() || (catInput.value + '支出'),
      amount: val
    });
    amtInput.value = '';
    noteInput.value = '';
    saveAndRender();
  };

  document.querySelectorAll('.filter-btn').forEach(btn => {
    btn.onclick = () => {
      filterCat = btn.getAttribute('data-cat') || '全部';
      document.querySelectorAll('.filter-btn').forEach(b => b.classList.toggle('active', b === btn));
      saveAndRender();
    };
  });

  saveAndRender();
})();
</script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_do_not_accept_inline_credentials_or_remote_plaintext() {
        for bad in [
            "http://example.com/v1",
            "https://key@example.com/v1",
            "https://example.com/v1?key=secret",
            "https://example.com/#key",
            "file:///tmp/a",
            "http://localhost.evil/v1",
        ] {
            assert!(validate_endpoint(bad).is_err(), "{bad}");
        }
        assert_eq!(
            validate_endpoint("https://api.example.com/v1/chat/completions/").unwrap(),
            "https://api.example.com/v1"
        );
        assert!(validate_endpoint("http://127.0.0.1:8080/v1").is_ok());
        assert!(validate_endpoint("http://[::1]:8080/v1").is_ok());
    }
    #[test]
    fn edited_html_cannot_reuse_a_previous_pass_report() {
        let directory = tempfile::tempdir_in(runs_directory().unwrap()).unwrap();
        let root = directory.path().to_path_buf();
        let html = root.join("index.html");
        let report = root.join("report.json");
        let original = b"<!doctype html><html><body>tested</body></html>";
        fs::write(&html, original).unwrap();
        fs::write(&report, serde_json::to_vec(&json!({"passed":true,"html_sha256":html_hash(original),"checks":[{"passed":true}]})).unwrap()).unwrap();
        let artifact = LabArtifact {
            id: "test".into(),
            name: "test".into(),
            root,
            html_path: html.clone(),
            screenshot_path: None,
            report_path: report,
            passed: true,
            round: 1,
            summary: String::new(),
            created_at: 0,
        };
        assert_eq!(verified_html(&artifact).unwrap(), original);
        fs::write(&html, b"<!doctype html><html><body>edited</body></html>").unwrap();
        assert!(verified_html(&artifact).is_err());
        assert!(!validate_artifact(artifact).unwrap().passed);
    }
    #[test]
    fn key_stays_private_and_is_not_reused_after_changing_provider() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.json");
        let public = save_at(
            &path,
            "https://api.example.com/v1",
            "openai",
            "a",
            "private-test-key",
            false,
        )
        .unwrap();
        assert!(public.has_key);
        assert!(!serde_json::to_string(&public)
            .unwrap()
            .contains("private-test-key"));
        save_at(
            &path,
            "https://api.example.com/v1",
            "openai",
            "b",
            "",
            false,
        )
        .unwrap();
        assert_eq!(
            read_private_at(&path).unwrap().unwrap().key,
            "private-test-key"
        );
        save_at(
            &path,
            "https://other.example.com/v1",
            "openai",
            "b",
            "",
            false,
        )
        .unwrap();
        assert!(!read_private_at(&path).unwrap().unwrap().public().has_key);
        save_at(
            &path,
            "https://other.example.com/v1",
            "openai",
            "b",
            "new-key",
            false,
        )
        .unwrap();
        save_at(
            &path,
            "https://other.example.com/v1",
            "openai",
            "b",
            "",
            true,
        )
        .unwrap();
        assert!(!read_private_at(&path).unwrap().unwrap().public().has_key);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn starter_artifacts_and_unpassed_delivery_work() {
        ensure_starter_artifacts().unwrap();
        let list = list_artifacts().unwrap();
        for id in [
            "starter-neon-snake",
            "starter-cyber-2048",
            "starter-pomodoro-app",
            "starter-expense-app",
        ] {
            let found = list.iter().find(|a| a.id == id).expect(id);
            assert!(found.passed);
            assert!(found.html_path.is_file());
            assert!(found.screenshot_path.as_ref().is_some_and(|p| p.is_file()));
            assert!(!verified_html(found).unwrap().is_empty());
        }
        let directory = tempfile::tempdir_in(runs_directory().unwrap()).unwrap();
        let root = directory.path().to_path_buf();
        let html = root.join("index.html");
        let report = root.join("report.json");
        let content = b"<!doctype html><html><body>playable draft</body></html>";
        fs::write(&html, content).unwrap();
        fs::write(
            &report,
            serde_json::to_vec(&json!({"passed":false,"html_sha256":html_hash(content),"checks":[{"passed":false}]})).unwrap(),
        )
        .unwrap();
        let draft = LabArtifact {
            id: "draft".into(),
            name: "draft".into(),
            root,
            html_path: html,
            screenshot_path: None,
            report_path: report,
            passed: false,
            round: 1,
            summary: String::new(),
            created_at: 0,
        };
        assert_eq!(verified_html(&draft).unwrap(), content);
    }
}
