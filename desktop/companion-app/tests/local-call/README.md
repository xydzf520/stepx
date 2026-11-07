# 本机 Telecom 来电验收

独立测试包 `com.stepx.callverify`，不属于正式伴侣。只在本机注册测试 PhoneAccount 并创建固定 `+15550163` 的来电；没有网络权限、不经过 SIM、不拨打真实号码。Android Telecom、电话状态监听、伴侣推送和桌面控制均使用生产链路。它能验证控制闭环，不能代替运营商真实来电、蓝牙音频或所有厂商固件的验收。

构建使用已有 Android SDK/JDK，独立生成并保存测试签名，不借用伴侣正式签名，不自动安装：

```bash
bash desktop/companion-app/tests/local-call/build.sh
adb -s DEVICE install -r -g desktop/companion-app/tests/local-call/.build/stepx-local-call-verification.apk
adb -s DEVICE shell am broadcast -n com.stepx.callverify/.Control --es op register
adb -s DEVICE shell cmd telecom set-phone-account-enabled com.stepx.callverify/.CallService stepx-local 0
adb -s DEVICE shell am broadcast -n com.stepx.callverify/.Control --es op incoming
```

Receiver 受系统 DUMP 权限保护，仅供 ADB shell 调用；来电号码和姓名固定，不接受任意号码。桌面收到来电后，点击接听、结束通话，通过测试包私有回执核对，而非仅看按钮文字：

```bash
adb -s DEVICE shell run-as com.stepx.callverify cat files/state.txt
```

回执应按 `ringing → active → ended` 变化，直接拒接为 `rejected`。同时检查桌面不抢输入焦点，来电与通话卡片尺寸一致，结束后移除来电卡片，新消息仍能同步。测试包 `debuggable` 仅为了读取这些本地回执，不改变正式伴侣配置。

在测试来电响铃时发送 `--es op expire`，只结束测试服务自身的待接连接并写入 `missed` 回执，用于核对真实 Telecom 未接记录→桌面未接提醒→回拨确认页。它不能结束其他服务的真实通话。

回拨验收到号码与 SIM 确认页即取消，不要拨打测试号码。验收结束后先结束本机测试通话，再注销测试账号和卸载测试包。若系统通话记录出现测试号码，只删除精确匹配 `+15550163` 的验收记录，保留其他记录：

```bash
adb -s DEVICE shell am broadcast -n com.stepx.callverify/.Control --es op unregister
adb -s DEVICE uninstall com.stepx.callverify
```
