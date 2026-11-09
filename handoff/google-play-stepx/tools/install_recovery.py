"""Read-only adoption checks for an explicit, non-destructive install retry.

Presence is not provenance. Verify every installed base/split APK before any
missing package is installed. A repair snapshot never pretends to be the
original pre-Google baseline and cannot authorize account-data removal.
"""
import re
import json
import tempfile
from pathlib import Path
from common import ADB, CONFIG, STATE, run, shell, sdk_command, sha256
from workflow_errors import WorkflowError

PACKAGES = tuple(entry['package'] for entry in CONFIG['apks'])


def package_inventory():
    lines = shell('pm', 'list', 'packages', '--user', '0').splitlines()
    if not lines or any(not re.fullmatch(r'package:[A-Za-z0-9_.]+', line) for line in lines):
        raise WorkflowError('incomplete_inventory', '手机包列表读取未完成，请保持连接后重新检测；不能据此判断 Google 尚未安装。', True)
    return {package: 'package:' + package in lines for package in PACKAGES}


def digest_on_phone(path):
    value = shell('sha256sum', path).split()
    if not value or not re.fullmatch('[0-9a-fA-F]{64}', value[0]):
        raise WorkflowError('package_unreadable', '无法完整读取已有 Google 安装包。请保持手机连接后重新检测；尚未覆盖已有组件。', True)
    return value[0].lower()


def verify_installed_package(package):
    meta = next(entry for entry in CONFIG['apks'] if entry['package'] == package)
    paths = shell('pm', 'path', '--user', '0', package).splitlines()
    if not paths or len(paths) > 64 or any(not p.startswith('package:/') for p in paths):
        raise WorkflowError('package_unreadable', '已有 Google 组件的安装路径读取不完整，请重新检测。', True)
    files = [p[8:] for p in paths]
    if len(set(files)) != len(files) or any('\n' in p or '\r' in p or '..' in Path(p).parts for p in files):
        raise WorkflowError('package_unreadable', 'Google 组件的安装路径异常，已停止恢复。')
    hashes = [digest_on_phone(path) for path in files]
    # The reviewed archive hash already binds package identity and signer.
    if len(files) != 1 or hashes[0] != meta['sha256']:
        STATE.mkdir(parents=True, exist_ok=True, mode=0o700)
        with tempfile.TemporaryDirectory(prefix='.verify-apk-', dir=STATE) as temp:
            for i, (remote, expected) in enumerate(zip(files, hashes)):
                local = Path(temp) / f'{i}.apk'
                run(ADB + ['pull', remote, str(local)], timeout=180)
                if sha256(local) != expected or digest_on_phone(remote) != expected:
                    raise WorkflowError('package_changed', 'Google 组件正在更新，请等待更新结束后继续；未覆盖已有版本。', True)
                result = run(sdk_command('apksigner') + ['verify', '--print-certs', str(local)], timeout=90)
                signers = re.findall(r'^Signer #\d+ certificate SHA-256 digest: ([0-9a-fA-F]{64})\s*$', result.stdout, re.M)
                if [s.lower() for s in signers] != [meta['signer_sha256']]:
                    raise WorkflowError('untrusted_package', '已有 Google 组件的签名不在已验证范围，已停止安装和授权。请核对组件来源；账号和应用保持原样。')
    # Do not accept a package generation that changed while it was inspected.
    if (shell('pm', 'path', '--user', '0', package).splitlines() != paths
            or [digest_on_phone(path) for path in files] != hashes):
        raise WorkflowError('package_changed', 'Google 组件正在更新，请等待更新结束后继续。', True)
    return {'verified': True, 'sha256': hashes, 'signer_sha256': meta['signer_sha256'],
            'reviewed_archive': len(files) == 1 and hashes[0] == meta['sha256']}


def verify_existing(inventory=None):
    inventory = package_inventory() if inventory is None else inventory
    return {package: verify_installed_package(package) for package in PACKAGES if inventory[package]}


def check_recovery_receipt(receipt, fingerprint, device_key):
    if (receipt.get('kind') != 'recovery_snapshot' or receipt.get('serial') != device_key
            or receipt.get('fingerprint') != fingerprint or not isinstance(receipt.get('packages'), list)):
        raise WorkflowError('recovery_identity', '恢复安装记录与当前设备或固件不一致，未覆盖记录或已有数据。')


def check_progress(receipt, fingerprint, device_key):
    if (receipt.get('serial') != device_key or receipt.get('fingerprint') != fingerprint
            or not isinstance(receipt.get('completed'), list)
            or any(p not in PACKAGES for p in receipt['completed'])):
        raise WorkflowError('recovery_identity', '安装进度记录与当前设备或固件不一致，已保留记录，请重新检测。')


def require_fresh_rollback(baseline, fingerprint, device_key):
    """Legacy CLI removal must prove a fresh, still-unlogged installation.

    Called under the device operation lock, before touching hooks or packages.
    Missing/corrupt receipts and unavailable account reads are never consent.
    """
    from health import account_count
    if (STATE / 'recovery-baseline.json').exists() or (STATE / 'recovery-boot-install.json').exists():
        raise WorkflowError('preserve_existing', '此设备已进行保留数据的恢复安装，不能沿用旧基线卸载组件；请使用保留应用的修复。')
    if (baseline.get('serial') != device_key or baseline.get('fingerprint') != fingerprint
            or not isinstance(baseline.get('packages'), list)
            or any(p in baseline['packages'] for p in PACKAGES)):
        raise WorkflowError('recovery_identity', '无法确认这是首次安装，已停止卸载并保留数据。')
    try:
        receipt = json.loads((STATE / 'install-progress.json').read_text())
    except (OSError, ValueError):
        raise WorkflowError('recovery_identity', '首次安装记录缺失或损坏，不能据此卸载已有 Google 组件。') from None
    check_progress(receipt, fingerprint, device_key)
    existing = verify_existing()
    if any(p not in receipt['completed'] or not proof['reviewed_archive'] for p, proof in existing.items()):
        raise WorkflowError('preserve_existing', '检测到非本次安装或已经更新的组件，停止卸载，保留已有版本。')
    if account_count(shell('dumpsys', 'account')) != 0:
        raise WorkflowError('preserve_existing', '已有 Google 账号或账号状态无法确认，已停止卸载和数据清理。')


def plan(preflight, owned):
    inventory = preflight['google_packages']
    if set(inventory) != set(PACKAGES) or any(type(v) is not bool for v in inventory.values()):
        raise WorkflowError('incomplete_inventory', 'Google 组件检测不完整，请重新检测。', True)
    installed = [p for p in PACKAGES if inventory[p]]
    missing = [p for p in PACKAGES if not inventory[p]]
    mode = 'existing' if not missing and owned else (
        'fresh' if not installed and not owned and not preflight.get('retained_packages') else 'recovery')
    if preflight.get('resume_install'):
        mode = 'fresh'
    return {'mode': mode, 'installed': installed, 'missing': missing, 'managed_runtime': owned,
            'preserve_existing': True, 'requires_install_switch': bool(missing)}
