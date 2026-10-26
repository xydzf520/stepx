//! Verified Android input-method setup. Never disable the old keyboard before readback.
use crate::{adb::Adb, device::Device, resources};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
pub const PACKAGE: &str = "org.fcitx.fcitx5.android";
const SOGOU: &str = "com.sohu.inputmethod.sogou.oem";
const APK: &str = "org.fcitx.fcitx5.android-0.1.3-0-g048f581c-arm64-v8a-release.apk";
const HASH: &str = "8e5de1036aea1f55895b39b4fc25a2d563e7b0663fa2708c86598f81768b106d";
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Status {
    pub current: String,
    pub installed: bool,
    pub active: bool,
    pub sogou_installed: bool,
    pub sogou_disabled: bool,
    pub checked: bool,
}
fn component(list: &str, package: &str) -> Result<String> {
    list.lines()
        .map(str::trim)
        .find(|line| {
            line.split_once('/')
                .is_some_and(|(p, c)| p == package && !c.is_empty())
                && !line.contains(char::is_whitespace)
        })
        .map(str::to_owned)
        .context("手机未注册该输入法服务，请在手机上完成首次初始化后重试")
}
fn has_package(list: &str, package: &str) -> bool {
    list.lines()
        .any(|s| s.trim() == format!("package:{package}"))
}
pub fn inspect(adb: &Adb, d: &Device) -> Result<Status> {
    d.verify(adb)?;
    let combined = adb.script(
        &d.transport,
        "settings --user 0 get secure default_input_method; echo '<<<STEPX_ENABLED>>>'; settings --user 0 get secure enabled_input_methods; echo '<<<STEPX_PKGS>>>'; pm list packages --user 0; echo '<<<STEPX_DISABLED>>>'; pm list packages -d --user 0",
    )?;
    let (current, rest) = combined
        .split_once("<<<STEPX_ENABLED>>>")
        .unwrap_or((&combined, ""));
    let (enabled, rest) = rest.split_once("<<<STEPX_PKGS>>>").unwrap_or((rest, ""));
    let (packages, disabled) = rest
        .split_once("<<<STEPX_DISABLED>>>")
        .unwrap_or((rest, ""));
    Ok(Status {
        active: current.trim().starts_with(&format!("{PACKAGE}/"))
            && has_package(packages, PACKAGE)
            && !has_package(disabled, PACKAGE)
            && enabled
                .trim()
                .split(':')
                .any(|s| s.split(';').next() == Some(current.trim())),
        current: current.trim().into(),
        installed: has_package(packages, PACKAGE),
        sogou_installed: has_package(packages, SOGOU),
        sogou_disabled: has_package(disabled, SOGOU),
        checked: true,
    })
}
fn verified_apk() -> Result<PathBuf> {
    let cache = resources::user_state().join("downloads/input-method");
    std::fs::create_dir_all(&cache)?;
    let bundled = resources::root().join("apks").join(APK);
    let cached = cache.join(APK);
    let path = if bundled.is_file() { bundled } else { cached };
    let script = r#"import hashlib,pathlib,sys,urllib.request,os
p=pathlib.Path(sys.argv[1]); expected=sys.argv[2]
def digest(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for b in iter(lambda:f.read(1048576),b''):h.update(b)
    return h.hexdigest()
if not p.exists():
    temp=p.with_suffix('.download')
    try:
        url='https://github.com/fcitx5-android/fcitx5-android/releases/download/0.1.3/'+p.name
        with urllib.request.urlopen(urllib.request.Request(url,headers={'User-Agent':'StepX-Companion'}),timeout=30) as r,temp.open('wb') as f:
            size=0
            while True:
                b=r.read(1048576)
                if not b:break
                size+=len(b)
                if size>160*1024*1024:raise ValueError('download too large')
                f.write(b)
        if digest(temp)!=expected:raise ValueError('APK checksum mismatch')
        os.replace(temp,p)
    finally:temp.unlink(missing_ok=True)
if digest(p)!=expected:raise ValueError('APK checksum mismatch')
"#;
    let mut cmd = resources::python_command();
    cmd.args(["-c", script]).arg(&path).arg(HASH);
    let output = crate::adb::run_process(cmd, None, Duration::from_secs(180))?;
    ensure!(
        output.status.success(),
        "输入法下载或校验失败，请检查网络后重试；手机输入法未更改"
    );
    Ok(path)
}
fn set_and_verify(adb: &Adb, d: &Device, ime: &str) -> Result<()> {
    let q = crate::adb::shell_quote(ime);
    let combined = adb.script(
        &d.transport,
        &format!(
            "ime enable --user 0 {q} >/dev/null && ime set --user 0 {q} >/dev/null && settings --user 0 get secure default_input_method && echo '<<<STEPX_ENABLED>>>' && settings --user 0 get secure enabled_input_methods"
        ),
    )?;
    let (readback, enabled) = combined
        .split_once("<<<STEPX_ENABLED>>>")
        .unwrap_or((&combined, ""));
    ensure!(
        readback.trim() == ime
            && enabled
                .trim()
                .split(':')
                .any(|s| s.split(';').next() == Some(ime)),
        "手机未确认默认输入法切换，尚未停用原输入法"
    );
    Ok(())
}
pub fn install_and_enable(adb: &Adb, d: &Device) -> Result<Status> {
    let before = inspect(adb, d)?;
    let backup = resources::user_state().join("input-method-backups");
    std::fs::create_dir_all(&backup)?;
    let name: String = d
        .serial
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let receipt = backup.join(format!("{name}.json"));
    if !receipt.exists() {
        std::fs::write(&receipt, serde_json::to_vec_pretty(&before)?)?;
    }
    if !before.installed {
        let abi = adb.shell(&d.transport, &["getprop", "ro.product.cpu.abilist"])?;
        ensure!(
            abi.split(',').any(|a| a.trim() == "arm64-v8a"),
            "此安装包适用于 ARM64 手机；当前设备架构不匹配，未修改输入法"
        );
        let apk = verified_apk()?;
        let result = adb.run_timeout(
            &["-s", &d.transport, "install", "-r", &apk.to_string_lossy()],
            Duration::from_secs(120),
        )?;
        ensure!(
            result.lines().any(|s| s.trim() == "Success"),
            "手机未确认输入法安装成功：{}",
            result.trim()
        );
    }
    adb.shell(&d.transport, &["pm", "enable", "--user", "0", PACKAGE])?;
    let imes = adb.shell(&d.transport, &["ime", "list", "-a", "-s", "--user", "0"])?;
    let ime = component(&imes, PACKAGE)?;
    if let Err(error) = set_and_verify(adb, d, &ime) {
        if before.current.contains('/') {
            if let Err(restore_error) = set_and_verify(adb, d, &before.current) {
                return Err(error.context(format!(
                    "切换未完成，原输入法未停用；恢复默认输入法也未能确认：{restore_error}"
                )));
            }
        }
        return Err(error);
    }
    // Only the OEM Sogou package; keep Android's secure input method intact.
    if before.sogou_installed && !before.sogou_disabled {
        adb.shell(&d.transport, &["pm", "disable-user", "--user", "0", SOGOU])
            .context("小企鹅已启用，但搜狗停用失败；请刷新后在手机输入法设置中处理")?;
    }
    let after = inspect(adb, d)?;
    ensure!(
        after.active && (!after.sogou_installed || after.sogou_disabled),
        "最终状态未通过核对，请刷新检查；未删除输入法数据"
    );
    Ok(after)
}
pub fn restore_sogou(adb: &Adb, d: &Device) -> Result<Status> {
    d.verify(adb)?;
    adb.shell(
        &d.transport,
        &["cmd", "package", "install-existing", "--user", "0", SOGOU],
    )?;
    adb.shell(&d.transport, &["pm", "enable", "--user", "0", SOGOU])?;
    let ime = component(
        &adb.shell(&d.transport, &["ime", "list", "-a", "-s", "--user", "0"])?,
        SOGOU,
    )?;
    set_and_verify(adb, d, &ime)?;
    inspect(adb, d)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discover_component_instead_of_guessing_class() {
        assert_eq!(
            component(
                "other/.Ime\norg.fcitx.fcitx5.android/.input.FcitxInputMethodService\n",
                PACKAGE
            )
            .unwrap(),
            format!("{PACKAGE}/.input.FcitxInputMethodService")
        );
        assert!(component("org.fcitx.fcitx5.android.fake/.Ime", PACKAGE).is_err());
        assert!(!has_package(
            "package:org.fcitx.fcitx5.android.fake",
            PACKAGE
        ));
    }
}
