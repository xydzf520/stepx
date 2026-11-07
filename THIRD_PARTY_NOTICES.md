# Third-Party Notices / 第三方组件说明

StepX Companion 的项目源码按 [`AGPL-3.0-only`](LICENSE) 发布。项目使用或集成的第三方组件遵循各自原始许可证；下文另行标明 Windows 运行库等专有可再分发组件，不将其标为 AGPL 或开源软件。

StepX Companion source code is released under [`AGPL-3.0-only`](LICENSE). Third-party components remain governed by their own licenses. Proprietary redistributable components, including the Windows runtime below, are identified separately and are not relicensed under the project's license.

---

## 1. 移动端与内嵌第三方组件 / Bundled & Mobile Third-Party Components

| 组件 / Component | 用途 / Purpose | 许可证 / License | 仓库内许可与说明路径 / Local License & Notice Path |
| --- | --- | --- | --- |
| **GELab-Zero** (`stepfun-ai/gelab-zero`) | 移动端 GUI 观察/动作解析格式参考与默认技能提示词适配（固定修订 `7b619f6f67d2b1101021fc453cb27bd48a29e4f2`） | MIT License | [`desktop/companion-app/third_party/gelab-zero/LICENSE`](desktop/companion-app/third_party/gelab-zero/LICENSE)<br>[`desktop/companion-app/third_party/gelab-zero/NOTICE.md`](desktop/companion-app/third_party/gelab-zero/NOTICE.md) |
| **ZXing Core** (`com.google.zxing:core:3.5.4`) | 手机端 StepX 伴侣本地二维码解析（配对与局域网连接），不上传相机帧 | Apache License 2.0 | [`desktop/companion-app/libs/ZXing-LICENSE.txt`](desktop/companion-app/libs/ZXing-LICENSE.txt)<br>[`desktop/companion-app/libs/ZXing-source.txt`](desktop/companion-app/libs/ZXing-source.txt) |

### GELab-Zero (MIT License)
- **Upstream**: https://github.com/stepfun-ai/gelab-zero
- **Copyright**: Copyright (c) 2025 stepfun-ai
- **License File**: [`desktop/companion-app/third_party/gelab-zero/LICENSE`](desktop/companion-app/third_party/gelab-zero/LICENSE)
- **Notes**: Android 侧适配层仅复用 `copilot_tools/parser_0920_summary_adv.py`（MIT）中的 `default_skill` 与制表符动作格式约定，不在 Android 上运行上游 Python/ADB 运行时。详见 [`desktop/companion-app/third_party/gelab-zero/NOTICE.md`](desktop/companion-app/third_party/gelab-zero/NOTICE.md)。

### ZXing Core (`zxing-core-3.5.4.jar`, Apache License 2.0)
- **Upstream**: https://github.com/zxing/zxing/tree/zxing-3.5.4
- **License File**: [`desktop/companion-app/libs/ZXing-LICENSE.txt`](desktop/companion-app/libs/ZXing-LICENSE.txt)
- **Source & Checksum Metadata**: [`desktop/companion-app/libs/ZXing-source.txt`](desktop/companion-app/libs/ZXing-source.txt)

---

## 2. 外部运行时工具与编解码组件 / Runtime Tools & Codec Components

| 组件 / Component | 用途 / Purpose | 许可证 / License | 上游项目 / Upstream |
| --- | --- | --- | --- |
| **scrcpy** | 桌面端设备画面投屏与低延迟键鼠控制运行时工具 | Apache License 2.0 | https://github.com/Genymobile/scrcpy |
| **adb (Android Debug Bridge)** | Android SDK Platform-Tools 授权设备通信、文件互传与调试通道运行时工具 | Apache License 2.0 | https://developer.android.com/tools/releases/platform-tools |
| **OpenH264 (`openh264`)** | H.264 视频流软解码支持（Cisco OpenH264 编解码库及 Rust 绑定） | BSD-2-Clause License | https://github.com/cisco/openh264<br>https://crates.io/crates/openh264 |

- **scrcpy & adb**: 作为外部子进程工具调用，遵循 Apache License 2.0。源码仓库不直接修改其内部实现；发布包随附运行时工具时保留其各自的 `NOTICE` 与许可证文件。
- **openh264**: Copyright (c) 2013, Cisco Systems, All rights reserved. 按 BSD 2-Clause "Simplified" License 授权使用。

### Microsoft Visual C++ Runtime（仅 Windows / Windows only）

Windows 安装包在主程序同目录附带其实际需要的 Microsoft Visual C++ x64 发行版运行库，避免要求用户另装开发工具。版权归 Microsoft Corporation 所有；这些 DLL 是专有的可再分发运行库，按适用的 Visual Studio 软件许可条款及 [Visual Studio 2022 Distributable Code / REDIST List](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution) 分发，不适用本项目 AGPL 许可证。REDIST List 允许从 Visual Studio 的 `VC/Redist` 子目录分发指定运行库，明确排除 `debug_nonredist`。

构建仅从当前 Visual Studio 安装对应的 `VC/Redist/MSVC/<version>/x64/Microsoft.VC*.CRT` 发行目录复制原样 DLL，不从任意 PATH、系统目录、第三方软件或调试目录收集文件。具体文件、版本、SHA-256、相对来源路径及依赖闭包记录在 Windows 包根目录的 `windows-crt.json`。仅实际导入的文件及其递归依赖随包分发；不会复制整个 Visual Studio 安装。

The Windows package includes only the unmodified x64 release CRT DLLs required by the main executable, deployed app-locally. Copyright Microsoft Corporation. These proprietary redistributable runtime libraries are governed by the applicable Visual Studio license terms and Microsoft REDIST List above, not by the StepX source license. The package's `windows-crt.json` records the exact runtime files, versions, hashes, relative Visual Studio source and dependency closure. Debug/non-redistributable files are excluded. Windows 10/11 operating-system libraries are provided by Windows and are not copied into the package.

---

## 3. Rust 生态依赖项 / Rust Ecosystem Dependencies

桌面客户端（`desktop/Cargo.toml`）基于 Rust 构建，使用了以下主要开源 crate（均遵循 MIT License、Apache License 2.0 或 BSD 类宽松开源许可）：

| Crate | 版本 / Version | 许可证 / License | 用途说明 / Purpose |
| --- | --- | --- | --- |
| `serde` / `serde_json` | `1.x` | MIT OR Apache-2.0 | 数据结构序列化与 JSON 协议编解码 |
| `chrono` | `0.4` | MIT OR Apache-2.0 | 日期、时间与时间戳格式化处理 |
| `image` | `0.25` | MIT OR Apache-2.0 | PNG / JPEG / WebP / GIF 图像解码与缩放处理 |
| `qrcode` | `0.14` | MIT OR Apache-2.0 | 配对二维码与局域网观看链接二维码生成 |
| `mdns-sd` | `0.21.6` | MIT OR Apache-2.0 | 局域网 mDNS / DNS-SD 设备发现与广播 |
| `winit` | `0.29` (patched) | Apache-2.0 | 跨平台原生窗口创建与输入事件循环 |
| `openh264` | `0.8.1` | BSD-2-Clause | H.264 视频帧解码封装 |
| `anyhow` | `1.x` | MIT OR Apache-2.0 | 错误上下文传播与诊断格式化 |
| `rand` | `0.8` | MIT OR Apache-2.0 | 随机数生成（配对码与临时会话标识） |
| `tempfile` | `3.x` | MIT OR Apache-2.0 | 安全临时文件与原子写入暂存 |
| `quick-xml` | `0.37` | MIT | XML 清单与结构解析 |
| `base64` | `0.22` | MIT OR Apache-2.0 | Base64 编解码（IPC 载荷与图标资源） |
| `url` | `2.x` | MIT OR Apache-2.0 | URL 解析与安全校验 |
| `async-channel` | `2.5` | Apache-2.0 OR MIT | 异步任务与 UI 线程消息通道 |
| `global-hotkey` | `0.7` | Apache-2.0 OR MIT | 桌面全局快捷键注册 |
| `tray-icon` | `0.26.1` | Apache-2.0 OR MIT | 系统托盘图标与菜单支持 |
| `native-dialog` | `0.7` | MIT | 原生文件选择与保存对话框 |
| `pathfinder_geometry` / `pathfinder_simd` | `0.5.1` | Apache-2.0 OR MIT | 2D 几何计算与 GPU 矢量渲染支撑 |
| `libc` / `libloading` / `x11rb` | `0.2` / `0.8` / `0.13.2` | MIT OR Apache-2.0 | 平台系统调用、动态库加载与 Linux X11 交互 |
| `embed-resource` | `3.0` | MIT | Windows 可执行文件图标与清单资源嵌入 |

---

## 4. 未打包的外部可选集成说明 / Unbundled Optional Upstream Integrations

- **Neo Agent Touch（仅本机适配层）**：仓库内 [`desktop/neo-agent-adapter/`](desktop/neo-agent-adapter/README.md) 仅包含 StepX Companion 独立编写的 Android IPC 适配层源码与构建脚本，**不包含** JiayuStudio 的 Neo Agent Touch 上游源码或预编译 APK。任何本机组合构建均需用户自行在私有目录拉取并遵循原作者版权要求，不随公开仓库或标准安装包再分发。
- **Google Play / GMS 组件**：仓库及公开构建不内置任何 Google 专有 APK，详见 [`NOTICE`](NOTICE)。

## Android 官方连接教程截图

`desktop/assets/connection-guide/` 中的版本号、USB 调试、电脑授权和无线调试图片来自 Google / Android Developers 的实机文档截图，依 [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) 使用。来源为 [连接 Android 设备](https://developer.android.com/codelabs/basic-android-kotlin-compose-connect-device) 和 [在硬件设备上运行](https://developer.android.com/studio/run/device)。部分图片仅裁切周边空白或无关设置项，未生成或修改系统控件；中文提示在客户端单独绘制。逐图网址和裁切坐标见 [来源清单](desktop/assets/connection-guide/sources.json)。Android 及相关产品标识仍归原权利人所有；引用不表示 Google 为本项目背书。
