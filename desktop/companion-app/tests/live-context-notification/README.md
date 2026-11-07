# 真机上下文通知测试 App

独立包名 `com.stepx.context.verification`，用于让真实 Android 通知经过伴侣的生产监听、同步与跨 App 关联流程。它只发布固定的验收资料，标记由启动参数提供。没有网络权限，也不读取短信、联系人、文件或账号；不属于伴侣正式 APK。

`--es scenario changes` 发布预算、日期、负责人发生变更的读书会通知；`--es scenario newsletter` 发布没有项目标记的无关资讯，用来验证不会把资讯误当成方案或待办。默认 `legacy` 保留旧的预算变更场景。三个场景使用不同通知 ID，同一场景与 marker 更新同一条通知，不接受任意通知正文。

构建脚本只构建、签名和校验测试 APK，不调用 ADB，不安装应用、不改真机：

```bash
bash desktop/companion-app/tests/live-context-notification/build.sh
```

需要 Python 3、JDK 和已有 Android SDK。JDK 按 `STEPX_JDK`、`JAVA_HOME`、PATH 中的 `javac` 查找；SDK 按 `ANDROID_SDK_ROOT`、`ANDROID_HOME` 和当前用户常见 SDK 目录查找。默认使用 Android 36、Build Tools 36.0.0 与 `cmdline-tools/latest/lib/r8.jar`；可用 `STEPX_ANDROID_API`、`STEPX_ANDROID_BUILD_TOOLS`、`STEPX_ANDROID_JAR`、`STEPX_R8_JAR` 覆盖。脚本不下载依赖。

产物为 `.build/stepx-context-verification.apk`，版本及权限记录在 `.build/apk-metadata.txt`。本地随机测试签名与密码保存在被 Git 忽略的 `.build/signing/`；保留这两份文件才能使用相同签名升级。它不使用伴侣的正式签名。

安装与发送通知由真机验收人员单独执行，例如对已授权的测试机：

```bash
adb -s STEPXTEST8DB89E4B8927 install -r desktop/companion-app/tests/live-context-notification/.build/stepx-context-verification.apk
adb -s STEPXTEST8DB89E4B8927 shell pm grant com.stepx.context.verification android.permission.POST_NOTIFICATIONS
adb -s STEPXTEST8DB89E4B8927 shell am start -n com.stepx.context.verification/.MainActivity --es marker STEPX-E2E-20261029
```

Activity 要求调用方持有 `android.permission.DUMP`，供 ADB shell 启动；不是面向普通 App 的通知注入入口。marker 只接受 8–80 个 ASCII 字母、数字或连字符，同一 marker 更新同一条测试通知。手机讨论使用相同的 `[项目: STEPX-E2E-20261029]` 标记，随后由 `verify_context_roundtrip` 读取并关联。

熄屏验证使用独立的 `ScreenOffReceiver`，同样要求调用者具有 `DUMP` 权限，无需打开 Activity。它不使用唤醒锁，通知使用独立 MIN 级别静默通道，不请求声音、振动、全屏或环境光提示。验收仍需记录实际电源状态，不能仅凭配置宣称全程熄屏。

```bash
adb -s DEVICE shell am broadcast --include-stopped-packages -n com.stepx.context.verification/.ScreenOffReceiver --es marker STEPX-VERIFY-SCREEN-OFF --ei sequence 1
# 仅撤销这一条测试通知，不删除其他通知或用户数据：
adb -s DEVICE shell am broadcast -n com.stepx.context.verification/.ScreenOffReceiver --es marker STEPX-VERIFY-SCREEN-OFF --ei sequence 1 --es op clear
```

序号仅接受 1–99，正文固定且明确标记为模拟，不能指定任意正文或伪装其他 App。`STEPX-VERIFY-` 是生产上下文过滤器已保留的验收标记，不应进入真实方案与总结。该模式证明真实 Android 通知监听和桌面同步链路，不能证明微信服务器投递或微信免打扰设置正常。

必须继续保留生产代码对 `com.android.shell` 等调试通知的过滤，不能为通过测试而放宽。测试通知确实进入生产链路，只证明这条标记数据的闭环，不代表所有 App 或完整历史均可读取。验收结束后的应用卸载由验收人员决定，构建脚本不会清理手机资料。

后台回复验收可使用 `--ez reply true`，通知会附带指向本 App 非导出 Receiver 的 RemoteInput。回复只写入本 App 私有文件，不发送给联系人或服务器。测试 APK 开启 debuggable，仅供 `adb run-as` 读取这份本地回执，正式伴侣不受影响。

```bash
adb -s DEVICE shell am broadcast --include-stopped-packages -n com.stepx.context.verification/.ScreenOffReceiver --es marker STEPX-VERIFY-CHAT-REPLY --ei sequence 21 --ez reply true
# 手机保持熄屏；例程只允许向这个固定测试包和会话发送固定测试正文。
cargo run --manifest-path desktop/Cargo.toml --example verify_chat_background -- DEVICE local-fixture-reply
```

不带 `local-fixture-reply` 时只做后台读取及能力核对。此验收证明 Android 后台回复通道与本地接收回执，不代表微信服务器或所有 App 都支持后台发送。
