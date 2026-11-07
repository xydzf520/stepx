# Neo Agent Touch 本机适配层

这是一份独立编写的 Android IPC 适配层；规划、视觉理解、任务队列、截图、原生手势和接管控制调用 **JiayuStudio 的 Neo Agent Touch**。上游固定为 `a5a6b6aa28dece90831d0630214f514d23ef443e`（0.1.16 Beta）。本目录不包含上游实现和 APK。

上游当前没有给出允许再分发的开源许可。构建脚本把上游下载到本机私有目录，保留原作者与版权信息，重定位为 `com.stepx.companion.neo`。产物仅用于用户本机适配测试，不加入本仓库、安装包资源或公开 Release；正式分发前需解决上游授权。

## 构建

先用项目自己的构建流程生成伴侣签名，再执行：

```sh
python3 scripts/neo-agent-touch/build.py \
  --embedded --jdk "$JAVA_HOME" --sdk "$ANDROID_SDK_ROOT"
```

脚本只编译，不安装或修改手机。可用 `--upstream` 指向固定提交的干净本地 checkout。默认输出在 `~/.local/state/stepx-companion/neo-agent-build/`，`build-receipt.json` 记录提交与 APK SHA-256。签名复用 `~/.config/stepx-companion/signing/companion.p12`；不生成另一份身份。上游 APK 的自动升级入口被换为适配版本说明，避免原包覆盖/签名冲突。原作者页面仍可访问。

### 集成到伴侣同一个 APK

当前使用 `--embedded` 把固定版本的 Neo 规划器、执行器与伴侣源代码一起编译成 `stepx-companion-neo-embedded-local.apk`。包名、版本、签名和唯一启动图标沿用伴侣，安装更新不会更换伴侣的数据身份。上游源码仅进入私有构建目录，产物也不会覆盖公开资源里的 APK。不带 `--embedded` 是早期独立适配测试模式，不是当前单 App 交付方式。

- 内部类仍为 `com.stepx.companion.neo.*`，所有组件用完整类名注册。
- 内部 provider 为 `com.stepx.companion.app.neo.control`，与早期独立测试版的 authority 不冲突。
- 主配置页为伴侣自己的 `NeoSettingsActivity`；Neo 原生高级页是内部 Activity，没有第二个启动图标。
- Neo 资源、assets、通用 preferences 和报告文件分别加 `neo_`、`neo/`、`neo-` 前缀，避免覆盖伴侣的 Studio、传输、配对和设置数据。双方使用同一资源 ID 表，生成各自的 R 包。
- 嵌入模式不会导入旧独立应用的任务或私有密钥；需在伴侣中配置已授权的模型与应用范围。旧测试版可保留到整合验收完成后再移除。

此模式的无障碍组件为 `com.stepx.companion.app/com.stepx.companion.neo.ControlService`，输入法为 `com.stepx.companion.app/com.stepx.companion.neo.AgentInputMethodService`。它们属于伴侣 APK，不依赖第二个 Agent 应用。首次启用仍需用户授权，无 root 或系统签名要求。

## IPC 契约（版本 1）

- 单 APK authority：`com.stepx.companion.app.neo.control`。早期独立适配包使用 `com.stepx.companion.neo.control`；以下示例使用当前单 APK 模式。
- `ContentResolver.call(uri, "request", null, Bundle)`，Bundle 内 `request` 为 JSON 字符串。
- ADB 控制也可用 `content call --uri content://com.stepx.companion.app.neo.control --method request --arg <base64-json>`。
- 返回 Bundle 的 `result` 为 UTF-8 JSON 的 Base64。`ok` 表示接口是否接受/读到结果，不代表任务完成。
- 所有入口先核对 Binder UID。允许自身 UID、ADB shell UID 2000，以及**精确包名** `com.stepx.companion.app` 且与当前 APK 签名一致的单包 UID。其他应用、shared UID、仅有 DUMP 权限的应用和 root UID 均不能通过此应用接口。没有公网端口和任意 shell 工具。
- Manifest 的 provider 未设单一权限，因为需要兼容 shell 与同签名伴侣两类调用；`call/openFile` 自行强制上述身份检查。其它 CRUD 均不开放。

| op | 输入 | 返回/行为 |
| --- | --- | --- |
| `config_info` 或只读 `config` | 无其它配置字段 | endpoint/model/protocol/vision/maxSteps/hasKey；永不返回 key |
| `config` | endpoint、model、protocol（openai/anthropic/gemini）、key、maxSteps（4–40）、userConfirmed=true | 写入 Android Keystore 加密 Vault；空 key 仅在相同服务与协议下保留旧 key |
| `apps` 只读 | 无 packages | configured、apps（package/name/category/enabled 等），不传图标 |
| `apps` 写入 | packages（明确允许的包名数组）、userConfirmed=true | **替换**完整允许范围，并将该范围的截图授权绑定到当前模型 endpoint |
| `start` | requestId、goal、可选 session、可选 verify | 后台模型规划 → Neo TaskWorkflow 校验 → Neo 执行器；立即返回 accepted，必须继续轮询 |
| `status` | 无 | 当前 requestId/runId、state、detail、ready、accessibilityConnected、configComplete、scopeApproved、executorBusy、step、modelCalls、screenshots、actions、workflow、verification、config |
| `pause` / `stop` | 当前 requestId | 暂停/停止规划或当前拥有的运行任务；旧 ID 不可操作新任务 |
| `resume` | 当前 requestId、userConfirmed=true | 用户明确交还控制后调用 Neo 恢复；锁屏、未核对动作、执行上限继续由 Neo 拒绝 |

`apps` 支持 `mode: "all" | "custom"`。默认全部模式只从 PackageManager 的已启用、可启动应用目录生成范围，不采用通配包名，也不包含伴侣自身。新应用在下一任务开始前加入；范围仍绑定已确认的模型接收地址。`custom` 使用明确的 `packages`，任务期间两种模式均不可更改。已就绪的旧伴侣配置由手机宿主一次性迁移到全部模式；明确保存过的自定义选择不再自动扩大。只读桥接 `status` 不修改配置，宿主迁移是单独的应用范围请求。

### Companion 浮钮

`CompanionControlBar` 替代上游橙色条的展示层，执行仍调用原生暂停、继续、停止。默认 72 dp 圆形浮钮，点击展开黑白灰控制面板，可拖动、收起；提示取自实际规划/观察阶段或候选动作名称，`finish` 只显示核验，不宣告任务完成。控件仍参与截图遮挡、路径排除、动作避让与位置恢复。

`host_ui` 是同 UID 宿主专用消息，不提供给 Amoo MCP 或 ADB。任务页可见控制栏每次状态刷新续期 5 秒；只有前台窗口确属伴侣才隐藏重复悬浮控件，离开、失去心跳或进程重建均恢复。没有修改原执行器的完成与输入策略。

`verify` 示例：`{"package":"com.jingdong.app.mall","texts":["我的订单","全部"]}`。该条件必须来自调用方要验收的目标，而不是模型自行编造。适配层在模型 finish 时、Neo 刚通过原始观察有效性校验的同一主线程回调里读回目标包可见非密码控件，保留窗口、遮挡、观察编号和采样时间；只在同一 runId、执行代次、工作流阶段自然结束后消费这份证明，不在随后 status 查询时用新页面补判成功。一次最终页面条件只能验收单步任务，不能把多步骤中前面的 reported 一并升级。它是屏幕条件验收，不证明账号信息或所有业务结果正确；目标标签应有区分度，避免把上一页的入口文字当成目标已打开。

状态含 `idle/planning/running/paused/paused_planning/reported/completed/needs_input/failed/stopped/interrupted`。只有 `completed=true` 且 `goal_verified=true` 是 Neo 原生证据或明确屏幕条件通过。`reported` 表示模型报告完成、需要核对，`terminal=true` 只表示本轮不再自动执行。不能把 accepted、running、reported 当作成功。

`ready` 需要无障碍已连接、模型本地配置合法、已选应用和截图接收端授权均就绪；它不表示远端模型 API 已经调用验证。`accessibilityConnected` 单独表示看屏服务连接。`executorBusy` 包括 Neo 自身会话的运行/暂停任务，避免切换到另一个执行器时出现双任务。

当前拥有任务的 `status.diagnostics` 可提供最近 3000 字脱敏记录、错误类型、边界和输入法状态。`input.candidate` 只记录 type 候选的脱敏截断文本，`input.searchPolicy` 记录真正传入原搜索策略的目标、候选和判定；不放宽搜索范围。`modelResponse` 只含结束原因、输出预算、字符数和 Token 计数，不含模型正文、推理正文、截图或密钥。诊断只在本进程保留，任务归属不匹配时不返回。

明确的“搜索框中输入‘电池’”句式在提取目标前规范化为搜索指令，避免把“框输入”混入搜索词。只处理完整引用的搜索编辑器句式，保留引用内容及后续型号、容量、颜色等全部规格；不修改原 `SearchQueryPolicy`，无关搜索词和缺失/替换规格仍被拒绝。

针对严格匹配的 StepFun 官方 HTTPS endpoint、OpenAI 协议及 `step-3.7-flash`，采用官方 `reasoning_effort`（视觉单步 low、规划 medium）、至少 4096 的输出预算和 JSON 提示词对应的 `response_format=json_object`，其它模型不改参数。响应 `finish_reason=length` 会明确停止在输出截断状态，不把不完整 JSON 当动作；仍由 Neo 校验动作结构、授权与页面新鲜度。参数依据：[Step 3.7 文档](https://platform.stepfun.ai/docs/en/guides/models/step-3.7-flash)、[JSON 模式和截断处理](https://platform.stepfun.ai/docs/en/guides/developer/json-mode)。

同一当前 requestId 的重试返回现有状态；目标不同则拒绝。每次新任务使用新 UUID。Bridge 的任务归属独立于 Neo 原生聊天：外部停止/暂停不能影响另一个原生会话。原生“接管”按钮也会撤销仍在进行的外部规划，旧模型响应不能重新取得控制。

### 密钥传输

伴侣使用 Binder Bundle 向 config 写入，密钥不落日志或回执。ADB **不能在命令参数中传入 key**；需要本机调试时以标准输入流式写入：

```sh
adb -s DEVICE exec-in content write --uri content://com.stepx.companion.app.neo.control/config < PRIVATE_CONFIG_JSON
```

输入是完整 `{"op":"config",...}`，上限 32 KiB。流写入异步完成后通过 `config_info` 核对公开参数及 hasKey。不要把密钥写入聊天、构建日志、Git、CLI 参数或发布附件。

**Amoo MCP 只暴露 start/status/pause/resume/stop。config/apps 是伴侣设置管理入口，不能允许模型自行用 userConfirmed=true 扩大授权。**

## 权限与手机独立使用

必要的是 Internet、用户启用的 `ControlService` 无障碍（截图/读取可见内容/手势）、用户允许操作的应用、模型接收端与截图授权。中文输入使用无障碍编辑动作及 Android 30+ 键盘控制；Neo 专用输入法可由用户启用，退出时沿用 Neo 恢复逻辑。悬浮接管栏使用 Accessibility Overlay，不需要另授 SYSTEM_ALERT_WINDOW。

此闭环不要求 root、平台签名、系统 UID、安装包权限、短信/联系人/存储全盘权限。适配 release 移除了录音和安装包请求。电池不限制/后台运行白名单可提高长任务稳定性，但不用于跳过密码锁屏。root 或系统预装也不能替代任务授权、视觉模型能力与完成证据。

绑定的无障碍服务、规划调度与执行循环全部位于手机进程，由手机直接请求单独配置的视觉模型 API；模型权重不在手机离线推理，也不默认复用 Amoo 账号。电脑断开后不需要 desktop 代跑，网络与模型服务仍需可用。进程恢复使用 Neo 加密 checkpoint 并停在暂停状态；不会自动重放可能已执行的动作。登录、支付、系统安全设置、人工接管与 Neo 付款边界均保留。

## 本机验证

构建时运行 `BridgePolicyTest`，验证签名/UID/包名边界、协议规范化、任务 ID、终态和完成证据区分；随后编译 Android、D8、签名和验签。未完成或失败步骤不能通过调用方的屏幕条件升级为完成。此测试不等于多步骤真机验收，实际启动、看图操作、中文输入、暂停/继续/停止和 Amoo 调用需要安装后逐项测试。


## Amoo 原生 MCP 与后台虚拟屏（0.2.62）

在已适配的 StepX PZ261A / 172 固件，完整组合 APK 可使用伴侣的签名限定系统服务连接原厂 MDI。手机 APK 仍是普通应用 UID；系统服务只暴露约束后的任务 API，不提供任意 root shell。

1. 在手机伴侣配置支持图片的视觉模型，并确认应用范围。
2. 桌面「小工具 → GUI Agent」选择后台虚拟屏，再点击「连接 Amoo」，确认安装调用技能。
3. 等待 Amoo 在真实会话中完成技能文件写入和读回验证。需要原厂安全确认时由用户确认；不会自动同意。
4. 新建普通对话，发送“打开京东，我的订单”。调用链为 Amoo → 原生 `ability / stepx_gui_agent` MCP → 手机伴侣 → MDI。任务推理和执行均在手机端，电脑不逐步代点。

本轮冷启动实测自动执行 2 次点击，5 次看屏，完成后独立核验订单列表并释放虚拟屏；前台保持设置页。动态广告不再使稳定导航控件失效。京东订单页不提供文字控件树，因此采用独立截图验收并核对两次观察到的原文；不会凭执行器自述完成。

锁屏、登录、本人授权等交用户处理；后台失败不会自动降级到主屏。同一应用的 Activity 仍受 Android 多显示策略约束，不能保证用户前台正在操作同一个应用时互不影响。其他应用需要单独验收。原始证据含私人界面，仅保存在本机 diagnostics，不上传。
