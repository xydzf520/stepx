# 阶跃伴侣 · StepX Companion

原生 Rust 跨平台桌面客户端（支持 **macOS / Windows / Linux**），连接已授权调试的 StepX 手机，复用相邻 `facloud` 仓库中的 `facloud_ui`。界面完全采用 GPU 原生渲染，不依赖浏览器或 Web 服务。

底部状态栏显示 Cargo 包版本和编译时间（UTC+8，精确到秒）。时间由 `build.rs` 固化进二进制，不随启动时间变化；`--version` / `-V` 可读取同一信息。可复现构建支持 `SOURCE_DATE_EPOCH`。

## 跨平台启动与安装

仓库布局：`stepx/` 与 `facloud/` 位于同一父目录。本次使用的 FaCloud 版本为 `d46ee1a1721d519d5718476da9c92fd3c63a04a7`。未修改 FaCloud 源码。

### Linux / macOS

从 `stepx` 根目录运行：

```bash
# Linux (Debian / Ubuntu) 或 macOS：准备本地 ADB 与 scrcpy 工具链到 .tools
scripts/setup-desktop-tools.sh

# 运行原生客户端（支持 --light / --dark / --pair-qr / --page <name>）
scripts/run-desktop.sh

# 构建并安装到当前用户（Linux 生成桌面入口，macOS 生成 ~/Applications/StepX Companion.app）
scripts/install-desktop.sh
```

- **macOS**：自动匹配 `PingFang SC` / `SF Mono` 等系统字体，支持 `⌘1`–`⌘6` 导航、`⌘R` 刷新、`⌘M` 实时控制、`⌘F` 搜索、`⌘C`/`⌘V`/`⌘A` 编辑快捷键，并支持从 Homebrew (`/opt/homebrew/bin`)、Android SDK (`~/Library/Android/sdk/platform-tools`) 或 `.app` 包内 `Contents/Resources/tools/bin` 自动发现 `adb` 与 `scrcpy`。
- **Linux**：自动匹配 `Noto Sans CJK SC` / `JetBrains Mono` 等字体；USB 若提示 `no permissions`，需要系统管理员安装 Android udev 规则；文件选择使用系统原生对话框（Zenity 或 KDialog）。x86_64 安装脚本自动打包官方 scrcpy 5.0.1（客户端、服务端和 ADB），下载固定版本并校验 SHA-256，无需 sudo。

### Windows (PowerShell)

```powershell
# 下载官方 Android Platform-Tools 到 .tools\bin
.\scripts\setup-desktop-tools.ps1

# 启动原生客户端
.\scripts\run-desktop.ps1

# 安装到当前用户并创建开始菜单快捷方式
.\scripts\install-desktop.ps1
```

- **Windows**：自动匹配 `Microsoft YaHei UI` / `Cascadia Mono` / `Consolas` 字体，后台执行 `adb.exe` 与 `scrcpy.exe` 自动启用 `CREATE_NO_WINDOW` 避免黑框闪烁，并支持从 `%LOCALAPPDATA%\Android\Sdk\platform-tools`、WinGet、Scoop、Chocolatey 自动发现工具。

## 界面

设计规范见 [DESIGN.md](DESIGN.md)。代码位于 `src/ui/`：`theme.rs`（设计令牌）、`widgets.rs`（组件）、`shell.rs`（侧栏/标题栏/状态栏）、`pages/`（每页一个文件）、`actions.rs`（交互与后台任务）。

| 页面 | 内容 |
|---|---|
| 概览 | 设备信息；电量、温度、存储；按键；截图预览与保存；投屏 |
| Amoo | 后台同步真实会话；后台新建、文字发送与回复刷新；中文输入；读取屏幕文字并插入问题 |
| Amoo Work | 手机工作会话、桌面文件协作、执行过程与交付文件；真实模型/技能/智能体目录；技能市场和本地工作流程 |
| 短信 | 会话列表与详情；搜索、筛选；选择 SIM 卡；发送前确认；存为草稿；查询发送状态 |
| 文件 | 常用位置、路径导航、搜索、排序；上传、下载、新建文件夹、重命名；回收站与撤销 |
| 应用 | 搜索；用户应用 / 全部；打开应用；安装 APK |
| 连接设备 | 已发现设备；扫码 / 配对码 / IP 地址配对；已记住的设备 |

电话和短信页只展示当前手机数据，不混入示例联系人、会话、验证码或通话记录。读取失败会在页面明确提示；草稿、发送失败和排队中的短信保留真实状态。SIM 选择使用手机返回的订阅 ID；短信经确认后发送，只有系统回执成功才显示成功，超时显示“状态未确认”并提供查询入口，不自动重发或补写手机记录。拨号及新建联系人交给手机界面完成，保存后刷新可读到实际结果。

概览和文件页的存储数据来自手机 `/data` 数据分区，使用 `df -h` 的容量与使用率；它与包含系统分区的标称容量口径不同。点击“刷新状态”或重新进入概览会更新数据，读取不到时显示未知。Amoo 等输入框启用系统输入法，显示中文组合文字并上报候选窗光标位置；选词期间的回车不会触发发送。标题栏按钮调用原生最小化、最大化/还原和关闭接口，并固定保留窗口控制区域。

未连接设备时，依赖设备的页面显示统一的“未连接设备”空状态。浅色、深色、跟随系统可在侧栏底部切换。

### 文字选择与电脑朗读

Amoo、Amoo Work 和简洁对话支持拖选正文后复制；输入框支持局部选择、全选、剪切、粘贴，以及 Shift + 方向键扩展选择。阅读历史消息或选择文字时，新回复不会抢走阅读位置。

回复底部的扬声器按钮在电脑上朗读，再次点击停止。朗读不上传正文、不消耗模型 Token；macOS / Windows 使用系统语音，Linux 需要 `espeak-ng` 和中文语音数据。手机 Amoo 的官方 TTS 接口目前未返回可用音频，本版使用本机语音，不声称复用了手机音色。

滚轮采用短缓动，触控板保留系统惯性；在设置中启用「减少动态效果」后直接滚动。发布运行使用 `cargo build --release`，避免调试构建额外的布局与渲染开销。排查性能时可临时设置 `STEPX_PERF_TRACE=1`，日志仅记录阶段耗时与连接复用状态，不记录会话正文或账号信息。

### 快捷键（macOS 使用 `⌘`，Windows / Linux 使用 `Ctrl`）

- `⌘1`–`⌘5`：切换页面；`⌘6`：连接设备
- `⌘R` / `F5`：刷新当前页
- `⌘F`：聚焦搜索；`⌘M`：投屏；`⌘T`：切换外观
- 文件页：`↑` `↓` 选择，`Enter` 打开，`Backspace` 上一级，`Delete` 删除，`F2` 重命名，`⌘N` 新建文件夹
- `⌘C` 复制选中文字，`⌘X` 剪切、`⌘V` 粘贴、`⌘A` 全选输入框；`Shift` + 方向键扩展选择
- `Tab` 切换输入框，`Enter` 提交，`Esc` 取消

## 新手机与新电脑配对

- **USB**：开启开发者选项与 USB 调试，插上数据线，在手机确认“允许”。首次只有一台已授权 StepX 时自动连接，多台设备时手动选择。
- **二维码**：双方连接同一 Wi-Fi。电脑打开“连接设备”，选择“扫码”；手机在 StepX伴侣“连接”页扫描或选择二维码图片，确认电脑身份，并按系统提示确认当前 Wi-Fi。二维码有效期两分钟，不保存配对口令。伴侣需要已安装且当前会话的安全扫码服务就绪。已真机验证二维码图片在手机本地解码 → Android 配对 → TLS 连接，并核对同一硬件身份；该结果不等于物理相机取景扫码已验收。
- **六位配对码**：手机打开“使用配对码配对”弹窗，同一网络只发现一台待配对手机时，电脑仅输入六位码即可。多台手机或自动发现失败时，再填写弹窗中的 IP 和配对端口。
- **后续重连**：记住硬件序列号，通过 ADB 的 mDNS/TLS 发现在线连接地址并重新验证硬件身份。
- **撤销信任**：电脑中的“忘记”或“移除”仅删除本地设备记录。手机上的“已配对设备 → 忘记”才会撤销该电脑的无线调试权限。

## 配置与验证

`STEPX_ADB`、`STEPX_SCRCPY` 可覆盖工具路径；`STEPX_CONFIG_DIR` 可指定设备记录目录；`STEPX_RENDER_DRIVER` 可覆盖 scrcpy 渲染器；`STEPX_THEME=light|dark` 可指定默认主题。

```bash
cd desktop
cargo fmt --package stepx-companion -- --check
cargo test --no-default-features --lib --test contracts
cargo clippy --all-targets -- -D warnings
cargo run -- --doctor

# 显式真机检查：仅打印计数，不输出短信或界面正文
cargo run --no-default-features --example verify_phone -- <ADB设备标识> --transfer

# 开发截图必须启用框架的 COPY_SRC 测试功能
cargo run --features ui-capture -- --page overview --capture-ui /tmp/stepx-overview.png
# 使用示例数据检查各页面（仅 ui-capture 构建可用；STEPX_DEMO_ALT=1 显示确认条等状态）
cargo run --features ui-capture -- --demo --page sms --capture-ui /tmp/stepx-sms.png
```

## 小工具集合

左侧“小工具”是分类工具首页，提供六个入口：释放空间、Google Play、APK 安装、系统服务、截图与投屏、设备检测。首页不扫描存储、不安装应用；进入具体工具后才读取所需数据。分类切换、返回集合、处理中禁用、结果反馈与行内确认沿用桌面设计。离线仍可浏览集合，并显示连接入口。

Google Play 与系统服务管理从“应用”页迁入；“应用”页保留列表和指向 APK 工具的快捷入口。Google 状态读取实际包版本、当前 boot/PID 的适配结果和磁盘包标记；缺失、失败或旧状态不显示为就绪。首次安装复用 `handoff/google-play-stepx` 的固件、签名、基线校验，经用户行内确认后依次执行。Google 安装入口同时安装并授权 StepX伴侣；已存在的 Google 包保留，不重复覆盖安装，部分安装状态需按实际检测修复。首次 Google 系统适配完成后需手动重启、解锁并再次检测；不会自动重启或索取账号信息。该适配仅支持已验证的构建，不代表 Google 认证。

安装组件默认定位本仓库的交接目录，可用 `STEPX_GOOGLE_HOME` 指定位置；需要 Linux、Python 3 和交接文档列出的 SDK/JDK。系统服务操作只允许适配列表中的条目，禁止停用当前输入法，操作后重新读取状态，不将命令失败算作成功。截图需手动触发，预览后另存；投屏提供结束入口。设备检测可以复制实际读取结果。

投屏使用支持 `--keep-active` 的 scrcpy（验证版本 5.0.1），仅在会话期间保持手机活跃，适用于 USB 和无线连接，不写入手机亮度、息屏时间或充电常亮设置。Linux/macOS 收到首帧后才显示画面就绪；Windows 先显示进程已启动，需在独立窗口确认画面。缺少组件、旧版不兼容、启动超时和中途断连均显示失败原因；关闭投屏或桌面时回收子进程。Linux 默认软件解码以避免 VA-API 驱动兼容问题，窗口仍使用 GPU 渲染。可通过 `STEPX_SCRCPY` 指定外部组件。

```bash
cargo run --example verify_tools -- <明确的StepX设备序列号> # 只读，不安装或停用
cargo run --example verify_mirror -- <明确的StepX设备序列号> # 打开投屏 75 秒，检查常亮及设置未变化
cargo run --features ui-capture -- --page tools --capture-ui /tmp/toolbox.png
cargo run --features ui-capture -- --page tools --tool google --capture-ui /tmp/google.png
```

### 释放手机空间

左侧“小工具”提供手机端文档压缩/还原、重复副本清理、大文件删除及系统缓存清理。操作前显示具体路径和行内确认，完成后显示手机可用空间的实际变化。文档以 `.gz` 替换原文件，使用前需要还原；压缩无收益、文件变化或校验失败时保留原文件，不导出到电脑。

系统透明应用压缩单独检测：当前 StepX 内核有 F2FS 压缩支持，但 `/data` 未开启相应分区特性，单独提升应用权限无法启用。检测复用 GMS 工程现有的固件校验和 root 通道，仅做读取。详见 [方案分析与实测](../docs/storage-compression-assessment-20261022.md)。

公开目录扫描有数量、时长及校验读取上限，界面显示跳过或截断结果。清理缓存由 Android 包管理器执行，不等同于清除应用数据。文件删除是永久删除，不经过回收站。

## StepX伴侣安装与授权

“小工具 → StepX伴侣”提供安装、修复、权限检测和文件互传；“连接设备”提供 USB 调试、电脑授权、安装伴侣、无线调试与配对的分步引导。手机端分为连接、文件、设置三个页面。文件通过 Android 系统文件选择器授权，采用独立传输目录和 SHA-256 回读校验，不申请所有文件访问权限。

StepX伴侣当前版本为 **0.1.4（versionCode 5）**。安装流程会授予并回读用户 0 的相机、通知、通话状态权限；通过既有 StepX 受权适配授予背屏、无线配对、无线调试设置及固定 CloudWake 请求权限；检查网络、前台服务与 CPU 唤醒锁等全部 **15 项 Manifest 权限**。完整清单见[权限与适配说明](../docs/amoo-screen-off-lease.md#安装权限)。Android Doze 与厂商冻结豁免仅针对 `com.stepx.companion.app`，保存原状态，恢复时不移除用户原先已有的豁免。APK 安装成功、权限完整、扫码系统服务可用、Amoo 熄屏任务完成是不同的检测结果，界面分别展示。

2026-10-22 在已授权 PZ261A / Android 16 完成电脑发文件、手机 SAF 保存与选择、手机发回电脑和接收回执的双向实测。权限适配依赖当前已核验固件、已有 RootBridge 和一致的应用签名，不是任意 Android 的通用安装器。新增第 15 项为原生推送服务声明的签名权限 `com.stepos.cloud.push.permission.MESSAGE`，仅用于固定的本地 CloudWake 请求。[0.1.4 安装日志](../diagnostics/companion-background-20261022/install-final.log)及[最终回读](../diagnostics/companion-background-20261022/final-permission-readback.json)已确认全部 15 项为 `true`；AgentBus PID `8444` 和 system_server PID `2859` 与本轮前一致。

当前原生 Amoo 在熄屏后主动断开空闲云连接，CPU 唤醒锁本身不能覆盖该策略。兼容适配通过受签名权限保护的原生 CloudWake 入口，为真实任务申请短时连接窗口；Amoo 与 Work 独立持有 90 秒租约并续期，用户停止或断连后释放伴侣资源。已真机确认手机保持熄屏时完成 Amoo 图片附件上传与模型看图回复，以及 Work 使用手机模型汇总 CSV、实际写入并回读 521 字节的 `report.md`。[联合验收](../diagnostics/companion-background-20261022/screenoff-verification.json)两项退出码均为 0，25.5 秒内 13 次电源采样始终为 `Asleep`、HAL 非交互，常亮设置不变；[Work 电源采样](../diagnostics/companion-background-20261022/screenoff-work-verification.json)同时记录显示 OFF。[296.143 秒真机生命周期验收](../diagnostics/companion-background-20261022/lease-lifecycle.json)也已通过：持续租约 190.158 秒后的真实云端查询、独立任务释放、停止后不复活和 90 秒无续租释放均符合预期。发送广播不等于任务成功。原生进程注入实验未通过，生产安装、重试和连接流程不执行该实验。详见[熄屏连接实测与限制](../docs/amoo-screen-off-lease.md)。

任务结束后立即释放伴侣 CPU 唤醒锁；只在本轮原生窗口剩余时间内保留屏幕监听，不继续请求或延长连接窗口，避免短会话刷新反复等待。用户停止立即结束监听；真实亮屏事件会使窗口失效，快速亮→灭也会重新核验，服务重建不信任旧窗口。

扫码使用独立的 shell UID 2000 配对服务，经 `DUMP` 保护的 Provider 向伴侣交付 Binder，双方验证身份，服务只开放五种固定配对操作。[实测](../diagnostics/companion-background-20261022/pairing-live-fixed.log)已完成二维码图片真实解码 → Android 配对 → TLS 连接，并核对同一手机 `STEPXTEST8DB89E4B8927`；物理相机扫码尚未单独验收。该服务只覆盖当前手机启动会话，没有开机常驻；手机重启或服务退出后需保持已授权 USB 连接，在“小工具 → StepX伴侣”更新或修复。系统网络信任仍需手机确认，重启后的权限恢复和服务恢复尚未验收。

## Amoo 后台连接

Amoo Work 位于“智能与开发”，与 Amoo 页面共享 AgentBus 云会话。手机处理模式从后台发送与同步任务；桌面协作模式使用手机模型规划，通过有界工具循环读取用户选择的工作目录、检索资料、汇总 CSV，并生成 Markdown、HTML、SVG、DOCX 或 XLSX。可读取文本及 PDF/DOCX/XLSX/PPTX 的可提取文字；扫描 PDF 不冒充已完成 OCR。

桌面工具不执行模型返回的 shell/Python 或第三方技能脚本。导入的 SKILL.md 工作流程可指导模型使用固定文件工具。文件写入新的输出目录，任务事件保存于工作目录的 .amoo-work；再次选择该目录可恢复产物列表。停止操作取消后续桌面执行，已被手机接受的任务可能继续。只有成功落盘的文件计为产物，普通模型回复不算已生成文件。

模型、技能、智能体和工具目录通过手机 OpenClaw 的正式 WebSocket 接口读取，与 AgentBus 云会话区分。2026-10-22 实机返回 884 个去重模型目录项，仅配置 stepos/stepos-model-auto。目录不等于调用授权；当前未验证到云 Amoo 会话的模型覆盖接口，界面跟随手机默认路由，不伪造切换成功。

技能市场搜索保留发布者身份。针对旧网关不支持作者限定引用的情况，固定安装器通过 ClawHub 正式 ownerHandle API 下载指定作者的版本，逐文件校验官方 SHA-256 清单，非覆盖安装到手机并回读技能状态。可将已安装技能的 SKILL.md 与文本参考导入桌面工作目录供模型使用，不执行第三方技能脚本。真机已安装 @ivangdavila/word-docx 1.0.2，并验证模型实际读取此技能后生成 DOCX，LibreOffice Writer 可打开并导出 PDF。

“图片创作”模式调用手机实际图片能力；桌面跟踪 task_create 返回的子会话，解析最终 text_to_image 产物，不将父会话的任务下发回复当作完成。真实图片在后台下载并内联预览，可查看原图和另存为；下载校验图片格式、不覆盖已有文件。真机已生成并下载 1024×1024 PNG，手机前台保持不变。

手机模式可直接添加附件，最多 8 个、合计 8 MiB。文件通过已授权 ADB 上传到手机 /sdcard/Download/Amoo Work 的独立目录，逐个回读 SHA-256 校验，再沿原生 <extra> 附件元数据通道传给 Amoo。模型在手机后台读取实际文件；真机已验证读取 CSV 内随机校验码及正确合计。上传成功后保留手机副本供会话后续处理，发送结果未知时不自动重发或删除副本。这不是云 FileStorage 上传，不伪造云 fileId。桌面协作模式仍在所选工作目录处理文件。

图片附件（PNG/JPEG/WebP/GIF）通过当前手机 AgentBus 的固定 `node.invoke.request` 能力调用进行 `batch_file_upload_return_all` 后台上传，使用真实私有数据标记阻止 CLOUD 路由回退；并将返回的真实 HTTPS URL 作为 `chat.send.attachments[].blob` 提交，包含 MIME、尺寸、大小和 SHA-256。`content: base64` 不属于这个 AgentBus 的图片输入协议，不能用文件字节读取代替视觉理解。上传能力接收原始手机路径，兼容中文、空格、`#`、`%` 文件名；账号凭据保留在手机内。图片超过 6400 万像素、上传不完整或失败时保留草稿，不静默降级为文字附件。

本轮熄屏测试已确认[固定本地图片上传能力](../diagnostics/companion-background-20261022/local-upload-screenoff.log)返回真实成功回执并通过暂存文件校验；随后[完整图片任务](../diagnostics/companion-background-20261022/screenoff-amoo-photo.log)完成真实图片发送及模型回复，校验图中红、绿、蓝三种颜色，且手机始终未点亮。此结果由任务回复和持续电源采样共同确认，未将上传回执当作视觉识别成功。

手机处理和桌面协作均可直接看图；后者先提交图片，再由同一会话使用桌面工具交付文件。真机已验证随机图片数字、颜色、形状、空间顺序、多图区分、无附件追问，以及实际生成并回读 Markdown 报告，全程无需打开手机界面。可运行 `cargo run --locked --no-default-features --example verify_amoo_vision -- STEPXTEST8DB89E4B8927 --desktop` 复验（仅合成图片和自建临时会话）。参考图编辑尚未单独验收。手机任务等待中可“停止等待”，这只结束桌面等待，不冒充已取消手机任务。

工作页默认仅显示宽屏对话；会话列表与文件面板互斥展开，模型/技能/智能体/AI 应用位于能力中心。回复原生显示 Markdown 标题、粗体、列表、引用、代码块、表格和链接，内部工具协议与附件元数据不展示在正文。AI 应用页提供 APK 安装及 Studio 入口，手机当前没有已验证的统一 AI 应用目录接口。原生 Amoo 界面的即时刷新限制仍见下文。

验证命令（从 desktop 目录运行）：

    cargo test --locked --features ui-capture -- --test-threads=1
    cargo run --locked --features ui-capture --example verify_amoo_catalog -- DEVICE_SERIAL documents
    cargo run --locked --features ui-capture --example verify_work -- DEVICE_SERIAL --send
    cargo run --locked --features ui-capture --example verify_amoo_media -- DEVICE_SERIAL
    cargo run --locked --features ui-capture --example verify_amoo_media -- DEVICE_SERIAL --image
    cargo run --locked --features ui-capture --example verify_work_skill -- DEVICE_SERIAL --send
    python3 -m unittest discover -s tests -p 'amoo_skills_test.py'

真机写入测试只使用独立测试会话、合成文件和隔离工作目录，结束后清理自己的测试会话及派生任务。技能安装是持久操作，verify_skill_install 需显式提供作者引用；不会自动卸载用户技能。原生窗口可使用 --page work --work-tab models 查看模型目录；--work-format-fixture 仅在 ui-capture 构建中显示本地格式验收样例，不写手机。

工作台参考 [WorkBuddy 的任务与文件工作方式](https://www.workbuddy.cn/)、[模型配置](https://www.workbuddy.cn/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/Model)和[技能管理](https://www.workbuddy.cn/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/Skills-Market)，能力以手机接口及实机验证为准。

桌面直接使用原厂 Amoo 的 AgentBus 协议：通过当前已授权设备的 ADB 转发连接 `localabstract:agentbus`，按协议 3 交换带 4 字节大端长度的 JSON 帧。同步、切换会话、新建和文字发送不调用 Activity、按键或 UI 自动化，也不依赖手机打开 Amoo。无需额外安装系统应用或修改 GMS 权限。每次连接校验设备身份，连接结束移除转发；使用随机客户端 ID，避免覆盖手机 Amoo 的连接。

`src/amoo.rs` 提供 `sessions.list`、`chat.history`、`sessions.reset(reason=new)` 与 `chat.send`。请求 ID 跨连接随机生成；会话使用真实 key，不以标题或列表位置识别。发送从手机公开 OpenId provider 读取原生 Amoo 使用的设备信息，携带 `stepclaw` 上下文；设备信息读取失败则保留草稿，不发送缺失上下文的请求。只有收到有效回执才清除匹配草稿，超时不自动重发。

回复按回执的 `runId` 和最终 `stopReason` 判断完成，工具调用和中间文字不会提前结束等待。当前会话及仍在处理的会话约每 2.4 秒刷新，50 次后降为每 10 秒继续读取；切换页面、会话和临时读取错误不会丢弃跟踪。无待处理请求时，Amoo 页每 10 秒同步外部变化。未加载的会话保留缓存，新建空会话在服务端发布前保留在桌面列表；历史分页限制明确标注。

手机 Amoo 当前版本会跳过其他客户端请求的实时推送，只在自身恢复/亮屏等历史同步时补齐已有会话。桌面收到后台回执不代表手机本地界面已刷新；不能保证桌面新建会话立即出现在手机列表。桌面不修改手机私有数据库。已实机确认已有会话的请求和最终回复可经手机原生历史同步恢复。

无界面跨客户端实时落库需要手机组件支持。当前 APK 暴露接口的核对结果、手机端修改位置和验收条件见 [手机实时同步接入结论](../docs/amoo-native-realtime-sync.md)。

AgentBus 云会话当前支持文字对话、手机文件附件和图片产物展示；语音、云会话模型切换与记忆管理尚未接入。Amoo Work 的手机技能管理使用独立的 OpenClaw 接口，其目录与安装状态不代表云会话一定能够调用；导入的工作流程已可由桌面协作使用。需要实际操作手机应用的指令仍由手机执行，并可能要求系统授权或显示相应应用。

```bash
# 只读取会话，核对手机前台窗口不变，不输出聊天正文
cargo run --example verify_amoo -- <ADB设备标识>
# 新建独立测试会话，发送一次无工具测试，收到回复后仅删除测试会话
cargo run --example verify_amoo -- <ADB设备标识> --send
```

在 PZ261A / Android 16 上读取到 18 个真实会话；真机测试完成中文发送、真实回复、测试会话清理，手机前台窗口保持不变。一次测试遇到系统本地网络权限窗口变化，该次前台检查正确报错，未算作通过。普通单测不再自动读取个人浏览器凭据或连接真机。


### 快速呼出简洁对话

在「设置 → 通用 → 简洁对话快捷键」点击组合键录入新按键，点击「确认保存」并确认变更后生效。默认 Windows / Linux 为 `Ctrl + Alt + A`，macOS 为 `Command + Option + A`。快捷键呼出 Amoo 简洁窗口并定位输入框，不清空会话、附件或草稿；全局快捷键冲突会提示并保留原设置。Windows、macOS、X11 支持全局呼出，原生 Wayland 当前仅支持窗口内快捷键（设置页会标明）。

底栏的 Star 数据来自公开 GitHub 仓库，每次启动仅请求一次，网络失败时显示 GitHub 入口，点击可访问项目。文件列表单击选择、双击进入目录或取回文件查看；安装包等非预览类型保留保存对话框，不自动安装。Amoo Work 的文件单击预览、双击在默认应用打开。
