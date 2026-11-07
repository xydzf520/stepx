# 构建与打包

需要当前 stable Rust、Git；高级功能需要 Python 3.10+（标准库即可运行内置 Office XML 工具）。可用 `STEPX_PYTHON` 指定解释器，`STEPX_ADB` 指定 ADB，`STEPX_RESOURCE_DIR` 指定资源目录。

## Linux（Ubuntu 24.04）

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config clang libclang-dev libssl-dev libfontconfig1-dev libfreetype-dev libxkbcommon-dev libwayland-dev libxcb1-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev libgl1-mesa-dev libdbus-1-dev
cargo build --release --locked --manifest-path desktop/Cargo.toml
```

需要支持 Vulkan/OpenGL 的图形环境。其他 Linux 发行版可能需要对应动态库；便携包不是静态链接的 AppImage。正式 Linux 包在 Ubuntu 24.04 上构建，以 glibc 2.39 为基线；较新发行版本机构建的二进制可能要求更新的 glibc，不能直接作为该兼容基线的发布包。

## macOS

安装 Xcode（含 Metal 工具链）和 Command Line Tools，运行相同的 Cargo 命令。使用目标架构的原生机器构建；发布流程分别在 Intel 和 Apple Silicon runner 执行。

## Windows

安装 Rust MSVC 工具链、Visual Studio C++ Build Tools 和 LLVM / libclang，运行相同的 Cargo 命令。源代码构建时需要 Python、JDK 17 和 Android SDK build-tools 36.0.0；用户使用完整发布包时无需安装这些开发依赖。

## 验证

```bash
cargo test --locked --manifest-path desktop/Cargo.toml --no-default-features --lib --test contracts
python3 -m unittest discover -s desktop/tests -p 'test_*.py'
```

真机验证脚本不属于默认测试，不会在 CI 中连接手机。

## 发布包

```bash
python3 scripts/package-release.py --platform linux-x86_64 --version 0.2.177
```

平台可选 `linux-x86_64`、`macos-arm64`、`macos-x86_64`、`windows-x86_64`。版本号应与 `desktop/Cargo.toml` 一致。脚本验证官方 scrcpy 工具包的 SHA256，将可执行文件与运行资源封装到 `dist/`。通过 `.github/workflows/release.yml` 的手动运行，或向 main 提交版本 / 工作流变更，构建全部平台；源码检查、四个平台的测试、构建和打包检查全部通过后自动发布 GitHub Release，附安装包、校验文件、伴侣 APK 和两段介绍视频。任一平台失败会阻止发布；流程不会覆盖已经正式发布的同名版本。

## Android

Android 源码位于 `desktop/companion-app`，以及 bridge、权限与后台服务对应目录。`scripts/build-companion-app.sh` 等脚本需要 JDK、Android SDK、`ANDROID_HOME` 与 `JAVA_HOME`。发布包含本项目编译的伴侣 APK / DEX / JAR；私有签名密钥不在仓库中。自己构建的 APK 若签名不同，无法直接覆盖已安装版本；保留数据前先自行备份。

## UI 截图

`cargo build --features ui-capture --manifest-path desktop/Cargo.toml` 后可使用 `--demo --capture-ui /absolute/path.png --page overview`。演示数据仅用于截图构建；普通发布构建不启用演示模式，截图不代表连接真实设备的验收记录。

发布打包还需要 JAVA_HOME 与 ANDROID_HOME（或 ANDROID_SDK_ROOT）。脚本用 jlink 生成最小 Java 运行时，编译 APK 身份校验助手并复制官方 apksig 校验库，并下载经固定 SHA256 校验的 Python。打包检查会清空 SDK/Python 环境，在中文、空格目录运行包内 ADB、Python、APK 签名校验和文件锁，不连接手机。macOS 仍未做 Apple Developer 公证。

APK 包名通过 Android 官方 apksig 的解析器读取，签名仍用 apksigner 验证；发布包不依赖原生 aapt，也不要求 Apple Silicon 用户为该流程安装 Rosetta。

应用生成验收使用已安装的 Chrome / Edge / Chromium 独立临时会话，不读取个人浏览器配置。CI 通过 `python scripts/ci-platform-smoke.py` 验证浏览器键入、点击、结果与隔离策略。

### 应用商店联网验收

默认回归使用内置目录与本机 HTTP 夹具，验证搜索一致性、去重、下载中断与续传。第三方商店目录、华为下载地址和真实 APK 下载属于显式联网验收，避免将外部服务的临时变化误判为本地构建失败。可在隔离的用户配置与缓存目录中运行 `cargo test --locked --manifest-path desktop/Cargo.toml --no-default-features --lib app_store::tests::test_live_ -- --ignored --test-threads=1`；其中 APK 验收会下载超过 100 MB 的数据并需要 Android 签名校验工具。联网失败应单独记录，不能据此宣称对应商店已验证可用。
