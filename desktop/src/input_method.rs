//! Verified Android input-method setup. Never disable the old keyboard before readback.
use crate::{adb::Adb, device::Device, resources};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
pub const PACKAGE: &str = "org.fcitx.fcitx5.android";
const SOGOU: &str = "com.sohu.inputmethod.sogou.oem";
const APK: &str = "org.fcitx.fcitx5.android-0.1.3-0-g048f581c-arm64-v8a-release.apk";
const HASH: &str = "8e5de1036aea1f55895b39b4fc25a2d563e7b0663fa2708c86598f81768b106d";
const REMOTE_IME_RESTORE_FILE: &str = "/data/local/tmp/stepx-companion/ime-restore.txt";
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Status {
    pub current: String,
    pub installed: bool,
    pub active: bool,
    pub sogou_installed: bool,
    pub sogou_disabled: bool,
    pub checked: bool,
}
fn is_valid_ime_id(id: &str) -> bool {
    let Some((pkg, cls)) = id.split_once('/') else {
        return false;
    };
    if pkg.is_empty() || cls.is_empty() || cls.contains('/') {
        return false;
    }
    pkg.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_'))
        && cls
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '$' | '-'))
}
pub fn parse_ime_list(list: &str) -> Vec<String> {
    list.lines()
        .map(str::trim)
        .filter(|line| is_valid_ime_id(line))
        .map(str::to_owned)
        .collect()
}
fn component(list: &str, package: &str) -> Result<String> {
    parse_ime_list(list)
        .into_iter()
        .find(|line| line.split_once('/').is_some_and(|(p, _)| p == package))
        .context("手机未注册该输入法服务，请在手机上完成首次初始化后重试")
}
fn has_package(list: &str, package: &str) -> bool {
    list.lines()
        .any(|s| s.trim() == format!("package:{package}"))
}
pub fn inspect(adb: &Adb, d: &Device) -> Result<Status> {
    let transport = d.resolve_active_transport(adb)?;
    let combined = adb.script(
        &transport,
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
    let script = r#"import hashlib, os, pathlib, sys, urllib.error, urllib.request
p = pathlib.Path(sys.argv[1])
expected = sys.argv[2]
MAX_BYTES = 160 * 1024 * 1024

def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for b in iter(lambda: f.read(1048576), b''):
            h.update(b)
    return h.hexdigest()

if p.exists() and digest(p) != expected:
    p.unlink(missing_ok=True)

if not p.exists():
    p.with_suffix('.download').unlink(missing_ok=True)
    temp = p.with_suffix('.part')
    if temp.is_symlink() or (temp.exists() and temp.stat().st_size > MAX_BYTES):
        temp.unlink(missing_ok=True)
    base = 'https://github.com/fcitx5-android/fcitx5-android/releases/download/0.1.3/' + p.name
    urls = [
        'https://ghfast.top/' + base,
        'https://gh-proxy.com/' + base,
        base,
    ]
    last_err = None
    completed = False
    for url in urls:
        for attempt in range(2):
            offset = temp.stat().st_size if temp.exists() else 0
            headers = {'User-Agent': 'StepX-Companion', 'Accept-Encoding': 'identity'}
            if offset > 0:
                headers['Range'] = f'bytes={offset}-'
            try:
                with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=25) as r:
                    status = getattr(r, 'status', 200)
                    if status == 206 and offset > 0:
                        mode = 'ab'
                        size = offset
                    elif status == 200:
                        mode = 'wb'
                        size = 0
                    else:
                        raise ValueError(f'unexpected status {status}')
                    cl = r.headers.get('Content-Length')
                    expected_total = (size + int(cl)) if (cl and cl.isdigit()) else 0
                    if expected_total > MAX_BYTES:
                        temp.unlink(missing_ok=True)
                        raise ValueError('download too large')
                    with temp.open(mode) as f:
                        while True:
                            b = r.read(262144)
                            if not b:
                                break
                            size += len(b)
                            if size > MAX_BYTES:
                                f.close()
                                temp.unlink(missing_ok=True)
                                raise ValueError('download too large')
                            f.write(b)
                        f.flush()
                if expected_total and temp.stat().st_size < expected_total:
                    raise ValueError('incomplete download')
                if digest(temp) != expected:
                    temp.unlink(missing_ok=True)
                    raise ValueError('APK checksum mismatch')
                os.replace(temp, p)
                completed = True
                break
            except urllib.error.HTTPError as e:
                last_err = e
                if e.code == 416:
                    temp.unlink(missing_ok=True)
                    continue
                break
            except Exception as e:
                last_err = e
                break
        if completed:
            break
    if not completed:
        raise RuntimeError(str(last_err or 'APK download failed'))

if digest(p) != expected:
    p.unlink(missing_ok=True)
    raise ValueError('APK checksum mismatch')
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
fn set_and_verify(adb: &Adb, transport: &str, ime: &str) -> Result<()> {
    let q = crate::adb::shell_quote(ime);
    let combined = adb.script(
        transport,
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
fn backup_receipt_path(serial: &str) -> Result<PathBuf> {
    let backup = resources::user_state().join("input-method-backups");
    std::fs::create_dir_all(&backup)?;
    let name: String = serial
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    Ok(backup.join(format!("{name}.json")))
}
fn persist_remote_ime_restore_if_needed(adb: &Adb, transport: &str, current: &str) -> Result<()> {
    let trimmed = current.trim();
    if trimmed.is_empty()
        || trimmed.starts_with(&format!("{PACKAGE}/"))
        || !is_valid_ime_id(trimmed)
    {
        return Ok(());
    }
    let quoted_ime = crate::adb::shell_quote(trimmed);
    let quoted_path = crate::adb::shell_quote(REMOTE_IME_RESTORE_FILE);
    adb.script(
        transport,
        &format!(
            "mkdir -p /data/local/tmp/stepx-companion && chmod 700 /data/local/tmp/stepx-companion 2>/dev/null || true; if [ ! -s {quoted_path} ]; then printf '%s\\n' {quoted_ime} > {quoted_path}; fi"
        ),
    )?;
    Ok(())
}
fn read_persisted_restore_ime(adb: &Adb, transport: &str, serial: &str) -> Option<String> {
    let quoted_path = crate::adb::shell_quote(REMOTE_IME_RESTORE_FILE);
    if let Ok(raw) = adb.script(
        transport,
        &format!("cat {quoted_path} 2>/dev/null || true"),
    ) {
        let candidate = raw.trim();
        if is_valid_ime_id(candidate) && !candidate.starts_with(&format!("{PACKAGE}/")) {
            return Some(candidate.to_owned());
        }
    }
    if let Ok(receipt) = backup_receipt_path(serial) {
        if let Ok(bytes) = std::fs::read(receipt) {
            if let Ok(saved) = serde_json::from_slice::<Status>(&bytes) {
                let candidate = saved.current.trim();
                if is_valid_ime_id(candidate) && !candidate.starts_with(&format!("{PACKAGE}/")) {
                    return Some(candidate.to_owned());
                }
            }
        }
    }
    None
}
pub fn install_and_enable(adb: &Adb, d: &Device) -> Result<Status> {
    let mut transport = d.resolve_active_transport(adb)?;
    let before = inspect(adb, d)?;
    let receipt = backup_receipt_path(&d.serial)?;
    if !receipt.exists() {
        std::fs::write(&receipt, serde_json::to_vec_pretty(&before)?)?;
    }
    persist_remote_ime_restore_if_needed(adb, &transport, &before.current)?;
    if !before.installed {
        let abi = adb.shell(&transport, &["getprop", "ro.product.cpu.abilist"])?;
        ensure!(
            abi.split(',').any(|a| a.trim() == "arm64-v8a"),
            "此安装包适用于 ARM64 手机；当前设备架构不匹配，未修改输入法"
        );
        let apk = verified_apk()?;
        transport = d.resolve_active_transport(adb)?;
        let result = adb.run_timeout(
            &["-s", &transport, "install", "-r", &apk.to_string_lossy()],
            Duration::from_secs(120),
        )?;
        ensure!(
            result.lines().any(|s| s.trim() == "Success"),
            "手机未确认输入法安装成功：{}",
            result.trim()
        );
    }
    adb.shell(&transport, &["pm", "enable", "--user", "0", PACKAGE])?;
    let imes = adb.shell(&transport, &["ime", "list", "-a", "-s", "--user", "0"])?;
    let ime = component(&imes, PACKAGE)?;
    if let Err(error) = set_and_verify(adb, &transport, &ime) {
        if before.current.contains('/') {
            if let Err(restore_error) = set_and_verify(adb, &transport, &before.current) {
                return Err(error.context(format!(
                    "切换未完成，原输入法未停用；恢复默认输入法也未能确认：{restore_error}"
                )));
            }
        }
        return Err(error);
    }
    // Only the OEM Sogou package; keep Android's secure input method intact.
    if before.sogou_installed && !before.sogou_disabled {
        adb.shell(&transport, &["pm", "disable-user", "--user", "0", SOGOU])
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
    let transport = d.resolve_active_transport(adb)?;
    let persisted = read_persisted_restore_ime(adb, &transport, &d.serial);
    let target_pkg = persisted
        .as_deref()
        .and_then(|ime| ime.split_once('/').map(|(p, _)| p))
        .unwrap_or(SOGOU);
    let _ = adb.shell(
        &transport,
        &["cmd", "package", "install-existing", "--user", "0", target_pkg],
    );
    adb.shell(&transport, &["pm", "enable", "--user", "0", target_pkg])?;
    if target_pkg != SOGOU {
        let _ = adb.shell(
            &transport,
            &["cmd", "package", "install-existing", "--user", "0", SOGOU],
        );
        let _ = adb.shell(&transport, &["pm", "enable", "--user", "0", SOGOU]);
    }
    let imes = adb.shell(&transport, &["ime", "list", "-a", "-s", "--user", "0"])?;
    let ime = if let Some(saved_ime) = persisted.as_deref() {
        if parse_ime_list(&imes).iter().any(|item| item == saved_ime) {
            saved_ime.to_owned()
        } else {
            component(&imes, target_pkg).or_else(|_| component(&imes, SOGOU))?
        }
    } else {
        component(&imes, SOGOU)?
    };
    set_and_verify(adb, &transport, &ime)?;
    let quoted_path = crate::adb::shell_quote(REMOTE_IME_RESTORE_FILE);
    let _ = adb.script(&transport, &format!("rm -f {quoted_path} 2>/dev/null || true"));
    if let Ok(receipt) = backup_receipt_path(&d.serial) {
        let _ = std::fs::remove_file(receipt);
    }
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
        assert_eq!(
            component(
                "com.oem.keyboard/.service.ImeService$InnerService\n",
                "com.oem.keyboard"
            )
            .unwrap(),
            "com.oem.keyboard/.service.ImeService$InnerService"
        );
        assert_eq!(
            parse_ime_list(
                "com.oem.keyboard/.service.ImeService$InnerService\ninvalid line with spaces\n"
            ),
            vec!["com.oem.keyboard/.service.ImeService$InnerService".to_string()]
        );
        assert!(component("org.fcitx.fcitx5.android.fake/.Ime", PACKAGE).is_err());
        assert!(!has_package(
            "package:org.fcitx.fcitx5.android.fake",
            PACKAGE
        ));
    }
}
