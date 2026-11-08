<div align="center">
<img src="desktop/assets/stepx_companion_icon.png" width="88" alt="StepX Companion" />

# StepX Companion

### 以人为中心，让 AI 贯通终端与服务。

连接手机与电脑上的消息、文件、安排、工具与 AI 成果，让提醒及时到达，让工作与生活跨端继续。

Linux / macOS / Windows · Rust 原生客户端 · 面向 STEPX Neo / PZ261A

[下载安装](https://github.com/xydzf520/stepx/releases/latest) · [快速开始](docs/GETTING_STARTED.md) · [更新合集](docs/RELEASE_0_2_182_CUMULATIVE.md) · [反馈问题](https://github.com/xydzf520/stepx/issues)

</div>

面向 STEPX 用户的社区项目，与 STEPX、Amoo、Google 无官方隶属关系。普通 Android 不在本版支持范围；原厂专有能力仍依赖对应固件、权限和账号。

## 下载最新版 · 0.2.182

**2026-11-08 发布** · [最新版发布页与校验文件](https://github.com/xydzf520/stepx/releases/latest) · [0.2.60 → 0.2.182 更新合集](docs/RELEASE_0_2_182_CUMULATIVE.md)

| 电脑 | 下载 | 使用条件 |
| --- | --- | --- |
| Windows x64 | [下载安装包](https://github.com/xydzf520/stepx/releases/download/v0.2.182/StepX-Companion-0.2.182-windows-x86_64.zip) | 完整解压运行；必要时安装手机 USB 驱动 |
| macOS Apple Silicon | [下载安装包](https://github.com/xydzf520/stepx/releases/download/v0.2.182/StepX-Companion-0.2.182-macos-arm64.zip) | 适用于 Apple 芯片；未 Apple 公证 |
| macOS Intel | [下载安装包](https://github.com/xydzf520/stepx/releases/download/v0.2.182/StepX-Companion-0.2.182-macos-x86_64.zip) | 适用于 Intel 芯片；未 Apple 公证 |
| Linux x64 | [下载安装包](https://github.com/xydzf520/stepx/releases/download/v0.2.182/StepX-Companion-0.2.182-linux-x86_64.tar.gz) | Ubuntu 24.04 构建基线，glibc 2.39+ |

包内提供 ADB、scrcpy、Python、Java 与 APK 校验工具，请完整解压。手机高级能力依赖受支持固件及授权，客户端会核验设备与接口。

## 1. 首页成为工作与生活的日常入口

- **先看整个月，再处理今天的事。** 完整月历优先展示，工作、生活用两种颜色标记，日程、待处理事项和最近文件集中在手边。
- **按自己的习惯摆放。** 16 个模块可移动、缩放、隐藏；调整后可保存，也能撤销或取消。
- **大小窗口都能抓住重点。** 卡片随宽高调整内容，同排对齐；窄卡保留核心信息，宽卡展示更多内容。平时右键即可使用模块快捷操作。

![首页：月历、待处理事项与最近文件](https://raw.githubusercontent.com/xydzf520/stepx/v0.2.180/desktop/assets/release-notes/0.2.180/home.png)

## 2. 重要消息及时到，正在做的事不被打断

- **新消息与验证码优先显示，红包更醒目。** 同一会话合并提醒，重启或重连后减少旧消息重复弹出。
- **回复时保持稳定。** 弹窗固定尺寸，输入和中文组词期间保持当前消息，避免跳动、切卡和抢焦点。
- **提醒太多，一键收起。** 顶部直接提供「清空、菜单、关闭」；清空当前及排队的普通弹窗，消息历史、未读状态和之后的新提醒仍保留。也可按应用暂停提醒。

## 3. 实况胶囊，跟进真正还没办完的事

日程、取件、配送和未接来电集中展示，减少反复打开手机查看状态。

物流会区分**寄给别人、自己收件和信息不明**，按订单合并状态；已完成的事项及时收起，避免旧取件码再次提醒。规则先判断，开启 Agent 后再复核有歧义的信息；未接来电可直接进入回拨操作。

## 4. 最近文件与全局搜索，让资料跨端接着用

- **刚下载、生成或收到的文件，首页就能找到。** 手机和电脑的图片、文档、PDF 等合并显示，不限定来自哪款 AI 或办公软件。
- **找到后直接操作。** 打开文件、传到手机或存到电脑；「文件与扩容」默认进入最近文件。
- **少找一层入口。** 点开顶部搜索即可看到最近文件；输入关键词后，统一查找已同步消息、文件、Amoo 讨论、日程和联系人等内容，并可筛选手机或电脑来源。

![最近文件：查看来源、打开与双向传输](https://raw.githubusercontent.com/xydzf520/stepx/v0.2.180/desktop/assets/release-notes/0.2.180/files.png)

## 5. 手机熄屏，消息和来电仍能在电脑处理

- **纯无线后台同步更可靠。** 改善“显示在线但读取停滞”的恢复；消息读取与 AI 长任务分开，生成过程中也能继续同步。
- **来电有独立弹窗。** 显示号码与已识别联系人，提供手机接听、电脑接听、挂断和回拨入口，并显示实际使用的音频端。

已在支持固件的手机与 Linux 电脑组合上，完成纯无线、熄屏来电及双向音频验证。电脑通话音频的可用范围仍以具体设备和平台适配为准。

## 6. AI 从聊天延伸到整理资料与继续工作

结合已同步的讨论和资料，整理方案、待办及日／周／月回顾，保留原文依据和人工编辑，生成的成果可继续修改并传回手机。

Agent 有独立设置，覆盖找资料、拟回复、整理待办和日程草稿等常用场景；规则优先，必要时调用所选 AI 来源。失败时保留草稿与附件，部分响应后断线不自动重复提交，减少重复任务和成果丢失。

## 7. 数据多了，减少重复读取与等待

- **增量保存、索引与缓存配合。** 更新时尽量只处理变化的记录，长列表分批显示，首页最近文件按可见数量读取。
- **拖动和缩放更连贯。** 减少宽窗口重复布局计算，合并拖动刷新，让模块调整更稳定。
- **恢复与迁移更稳妥。** 改善 Windows 数据路径迁移、macOS 文件选择和保存窗口等问题；操作失败可查看原因，并对支持的操作重试。

同机合成数据测试中，2 万条记录修改 1,000 条，热快照写入中位数由 **44.054 ms 降至 8.545 ms**。这是该增量场景的测量结果，完整快照仍有成本；[查看性能测量范围](https://github.com/xydzf520/stepx/blob/v0.2.182/docs/PERFORMANCE.md)。

## 8. Google 服务更新后，自动检查与恢复

对已经通过 StepX 完成适配的 Google 服务，手机在组件更新后自动核验，按需恢复已支持的权限缺口；电脑连接后后台补检，保留账号和应用数据。

小工具内可查看处理状态、失败原因，立即检查或暂停电脑维护。旧监控首次升级会短暂重建手机网关；后续原生复检无需重建。网络、账号和设备认证问题会单独提示，不能由权限修复保证解决。

---

**原有能力继续保留：** 背屏创作与控制、投屏、相册、文件扩容、应用与商店。本轮同时改善了原生接口调用和操作后的状态核验。

## 视频：从连接到日常使用

 [完整产品介绍](https://github.com/xydzf520/stepx/releases/download/v0.2.180/StepX-product-0.2.180.mp4) · [累计更新演示](https://github.com/xydzf520/stepx/releases/download/v0.2.180/StepX-0.2.60-to-0.2.180.mp4)

图片与视频来自 0.2.180 的真实客户端和隔离演示数据；本页文字已合并至 0.2.182。

<details>
<summary>适用范围与详细更新记录</summary>

- 消息与搜索以已同步内容为准，通知不等于完整聊天历史；手机文件主要按名称检索，电脑可检索指定目录内支持的文本。
- AI 与后台操作依赖所选来源、账号权限和手机实际开放能力；原厂通用离线语音转写与合成尚未验证可用。
- 熄屏后台读取需要已有有效授权；重启后的首次解锁和系统安全确认仍需在手机完成。macOS 安装包尚未 Apple 公证。
- [功能、版本与日期索引](https://github.com/xydzf520/stepx/blob/v0.2.182/docs/FEATURE_HISTORY.md) · [完整逐版更新记录](https://github.com/xydzf520/stepx/blob/v0.2.182/CHANGELOG.md) · [0.2.182 修复验证详情](https://github.com/xydzf520/stepx/blob/v0.2.182/docs/RELEASE_0_2_182.md)

</details>

## 快速开始

1. 下载对应电脑的安装包，完整解压运行。
2. 按客户端引导开启手机开发者选项和 USB 调试，连接并授权。
3. 在「连接设备」选择手机，按需安装伴侣并完成权限引导。
4. 需要无线使用时，让手机和电脑连接同一局域网，完成扫码或数字配对。
5. 从首页查看安排和最近文件，或进入消息、Amoo、背屏与小工具。

[图文连接指南](docs/GETTING_STARTED.md) · [完整功能说明](docs/FEATURE_GUIDE.md) · [功能对应版本与日期](docs/FEATURE_HISTORY.md)

## 数据与隐私

设置、同步缓存和文件保存在当前电脑的用户目录。公开源码和安装包不包含开发机的聊天、数据库、设备记录、诊断日志或密钥。使用 AI 服务或向手机传输文件时，会按你启用的功能发送所需内容；提交问题日志前请先脱敏。

## 开发与贡献

```sh
git clone https://github.com/xydzf520/stepx.git
cd stepx
cargo build --release --locked --manifest-path desktop/Cargo.toml
```

UI 依赖已放在 `vendor/`。系统依赖、Android 构建和打包见 [构建指南](docs/BUILDING.md)。欢迎提交复现步骤、跨平台问题和 Pull Request。

## 许可与反馈

源码按 **AGPL-3.0-only** 发布，第三方组件遵循各自许可：[LICENSE](LICENSE) · [NOTICE](NOTICE) · [第三方说明](THIRD_PARTY_NOTICES.md)。品牌、标志和原厂素材权利归原权利人所有；源码许可不授予商标权。

仓库不包含账号密钥、个人手机备份或私有聊天记录。系统维护属于实验性能力，执行前请阅读界面的范围与恢复说明。

使用问题、建议与版权反馈可通过 [GitHub Issues](https://github.com/xydzf520/stepx/issues) 联系项目。
