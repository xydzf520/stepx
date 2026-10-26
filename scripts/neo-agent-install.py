#!/usr/bin/env python3
"""Local, certificate-pinned Neo setup. No root, arbitrary grants, or app data access."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
from host_runtime import file_lock, java_environment, subprocess_options, user_state_root
from companion_apk import apk_identity, apk_manifest, embedded_runtime, signer_digests

PACKAGE = 'com.stepx.companion.app'
COMPANION = PACKAGE
NEO_NAMESPACE = 'com.stepx.companion.neo'
ACCESS = PACKAGE + '/' + NEO_NAMESPACE + '.ControlService'
IME = PACKAGE + '/' + NEO_NAMESPACE + '.AgentInputMethodService'
AUTHORITY = PACKAGE + '.neo.control'
SCOPE = 'embedded-neo-v1'
ROOT = Path(__file__).resolve().parents[1]
STATE = user_state_root() / 'neo-agent-embedded'


def canonical(component):
    if '/' not in component:
        return component
    package, name = component.split('/', 1)
    return package + '/' + (package + name if name.startswith('.') else name)


def components(value):
    return [x for x in value.split(':') if x and x != 'null']


def contains(value, item):
    return canonical(item) in [canonical(x) for x in components(value)]


def add_component(value, item):
    parts = components(value)
    if not contains(value, item):
        parts.append(item)
    return ':'.join(parts)


def remove_component(value, item):
    return ':'.join(x for x in components(value) if canonical(x) != canonical(item))


def run(args, timeout=30):
    p = subprocess.run([str(x) for x in args], capture_output=True, text=True,
                       encoding='utf-8', timeout=timeout, env=java_environment(), **subprocess_options())
    if p.returncode:
        detail = (p.stdout + '\n' + p.stderr).strip()
        # Never dump shell environment, model configuration, or arbitrary package state.
        codes = re.findall(r'INSTALL_FAILED_[A-Z_]+', detail)
        if codes:
            raise RuntimeError('安装失败：' + codes[-1] + '。请在手机确认安装后重试；已有数据未清除。')
        raise RuntimeError('本地工具未完成：' + Path(str(args[0])).name)
    return p.stdout.strip()



def private_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    path.parent.chmod(0o700)
    temp = path.with_suffix('.tmp')
    with os.fdopen(os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), 'w') as f:
        json.dump(value, f, ensure_ascii=False, indent=2)
        f.flush(); os.fsync(f.fileno())
    os.replace(temp, path)


class Installer:
    def __init__(self):
        serial = os.environ.get('STEPX_SERIAL', '')
        physical = os.environ.get('STEPX_DEVICE_SERIAL', '')
        if not serial or not physical or serial.startswith('-') or any(c.isspace() for c in serial):
            raise RuntimeError('请先选择并授权手机连接')
        self.adb = [os.environ.get('STEPX_ADB', 'adb'), '-s', serial]
        if run(self.adb + ['get-state']) != 'device' or self.shell('getprop', 'ro.serialno') != physical:
            raise RuntimeError('当前连接与选中的手机不一致')
        if int(self.shell('getprop', 'ro.build.version.sdk')) < 30:
            raise RuntimeError('Neo 需要 Android 11 或更新系统')
        self.identity = {'device': hashlib.sha256(physical.encode()).hexdigest(),
                         'fingerprint': self.shell('getprop', 'ro.build.fingerprint')}
        self.folder = STATE / self.identity['device'][:24]
        self.folder.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.folder.chmod(0o700)
        self.receipt = self.folder / 'setup.json'

    def shell(self, *args):
        return run(self.adb + ['shell', shlex.join(str(a) for a in args)])

    def setting(self, name):
        return self.shell('settings', '--user', '0', 'get', 'secure', name)

    def put_setting(self, name, value):
        if value == 'null':
            self.shell('settings', '--user', '0', 'delete', 'secure', name)
        else:
            self.shell('settings', '--user', '0', 'put', 'secure', name, value)

    def installed_path(self, package):
        probe = subprocess.run(self.adb + ['shell', shlex.join(['pm', 'path', '--user', '0', package])],
                               capture_output=True, text=True, timeout=30, **subprocess_options())
        if probe.returncode in (0, 1) and not probe.stdout.strip() and not probe.stderr.strip():
            return None
        if probe.returncode:
            raise RuntimeError('无法核对手机应用安装路径')
        lines = probe.stdout.strip().splitlines()
        bases = [x[8:] for x in lines if x.startswith('package:/data/app/') and x.endswith('/base.apk')]
        if len(bases) != 1:
            raise RuntimeError('应用安装位置异常，未改变权限')
        return bases[0]

    def installed_uid(self):
        dump = self.shell('dumpsys', 'package', PACKAGE)
        match = re.search(r'Package \[' + re.escape(PACKAGE) + r'\][^\n]*\n\s*(?:appId|userId)=(\d+)', dump)
        if not match or not 10000 <= int(match.group(1)) < 100000:
            raise RuntimeError('伴侣必须使用 Android 用户 0 的独立应用 UID')
        uid = int(match.group(1))
        lines = self.shell('pm', 'list', 'packages', '--user', '0', '--uid', str(uid)).splitlines()
        packages = [line.split(' uid:', 1)[0] for line in lines]
        if packages != ['package:' + PACKAGE]:
            raise RuntimeError('伴侣共享 UID，未进行权限适配')
        return uid

    def installed_cert(self, package, backup=None):
        path = self.installed_path(package)
        if not path:
            return None
        with tempfile.TemporaryDirectory(prefix='stepx-neo-signature-') as temp:
            local = Path(temp) / 'base.apk'
            run(self.adb + ['pull', path, local])
            actual, cert = apk_identity(local)
            if actual != package:
                raise RuntimeError('手机 APK 包名不匹配')
            if backup:
                shutil.copyfile(local, backup); backup.chmod(0o600)
            return cert

    def has_embedded_runtime(self):
        remote = self.installed_path(PACKAGE)
        if not remote: return False
        with tempfile.TemporaryDirectory(prefix='stepx-embedded-inspect-') as temp:
            local = Path(temp) / 'base.apk'
            run(self.adb + ['pull', remote, local])
            return embedded_runtime(apk_manifest(local))

    def doze(self):
        return any(len(parts := line.split(',')) == 3 and parts[1] == PACKAGE
                   for line in self.shell('dumpsys', 'deviceidle', 'whitelist').splitlines())

    def freezer(self):
        text = self.shell('dumpsys', 'activity', 'freeze', 'whitelist')
        result, section = {}, None
        for line in text.splitlines():
            match = re.fullmatch(r'\s*(cloud|user|local) freeze white apps:\s*', line)
            if match:
                section = match.group(1); result[section] = False
            elif line.strip().startswith('freeze white app:') and section is not None:
                result[section] |= line.split(':', 1)[1].strip() == PACKAGE
        return result if set(result) == {'cloud', 'user', 'local'} else None

    def status(self, verify=False):
        path = self.installed_path(PACKAGE)
        dump = self.shell('dumpsys', 'package', PACKAGE) if path else ''
        version = re.search(r'versionName=([^\n]+)', dump)
        # Only the phone's Neo status endpoint can confirm a connected live executor.
        recovery = False
        if self.receipt.is_file():
            try: recovery = json.loads(self.receipt.read_text()).get('phase') != 'restored'
            except (ValueError, OSError): recovery = True
        runtime_available = False
        signature_verified = False
        if path:
            with tempfile.TemporaryDirectory(prefix='stepx-embedded-inspect-') as temp:
                local = Path(temp) / 'base.apk'
                run(self.adb + ['pull', path, local])
                runtime_available = embedded_runtime(apk_manifest(local))
                if verify:
                    actual, cert = apk_identity(local)
                    if actual != PACKAGE:
                        raise RuntimeError('手机 APK 包名不匹配')
                    signature_verified = cert is not None
        out = {'checked': True, 'companion_installed': bool(path), 'installed': runtime_available, 'version': version.group(1).strip() if version else '',
               'accessibility_enabled': contains(self.setting('enabled_accessibility_services'), ACCESS),
               'ime_enabled': contains(self.setting('enabled_input_methods'), IME), 'doze_exempt': self.doze(),
               'oem_freezer_exempt': self.freezer(), 'recovery_available': recovery}
        if verify and path:
            out['signature_verified'] = signature_verified
        return out

    def save(self, value):
        private_json(self.receipt, value)

    def validate_receipt(self, value):
        if value.get('schema') != 2 or value.get('scope') != SCOPE or value.get('identity') != self.identity or value.get('package') != PACKAGE:
            raise RuntimeError('恢复记录与当前手机不匹配')
        cert = self.installed_cert(PACKAGE)
        if cert and cert != value.get('signer'):
            raise RuntimeError('Neo 签名已变化，保留恢复记录并停止')
        if cert and value.get('uid') is not None and self.installed_uid() != value['uid']:
            raise RuntimeError('Neo UID 已变化，保留恢复记录并停止')

    def restore_settings(self, value):
        self.validate_receipt(value)
        before = value['before']; errors = []
        def attempt(action):
            try: action()
            except Exception as error: errors.append(str(error))
        # Remove only our component; retain unrelated services added since installation.
        if value.get('access_added'):
            current = self.setting('enabled_accessibility_services')
            next_value = remove_component(current, ACCESS)
            if current != next_value:
                attempt(lambda: self.put_setting('enabled_accessibility_services', next_value or 'null'))
        if value.get('master_changed') and self.setting('accessibility_enabled') == '1':
            current_set = {canonical(x) for x in components(self.setting('enabled_accessibility_services'))}
            original_set = {canonical(x) for x in components(before['enabled_accessibility_services'])}
            if current_set == original_set:
                attempt(lambda: self.put_setting('accessibility_enabled', before['accessibility_enabled']))
        if value.get('ime_added'):
            if canonical(self.setting('default_input_method')) == canonical(IME):
                old = before['default_input_method']
                if old in ('', 'null') or not contains(self.setting('enabled_input_methods'), old):
                    errors.append('原输入法不可用，请在手机选择键盘后重试恢复')
                else:
                    attempt(lambda: self.shell('ime', 'set', '--user', '0', old))
            if canonical(self.setting('default_input_method')) != canonical(IME):
                attempt(lambda: self.shell('ime', 'disable', '--user', '0', IME))
        # Shared companion battery/OEM exemptions belong to companion setup, never this module.
        # Read back each owned change, never claim success merely from command exit codes.
        if value.get('access_added') and contains(self.setting('enabled_accessibility_services'), ACCESS):
            errors.append('无障碍服务尚未恢复')
        if value.get('ime_added') and contains(self.setting('enabled_input_methods'), IME):
            errors.append('输入法尚未恢复')
        value['restore_errors'] = errors
        value['phase'] = 'restore_pending' if errors else 'restored'
        self.save(value)
        if errors:
            raise RuntimeError('；'.join(dict.fromkeys(errors)))
        return self.status(verify=True)

    def restore(self):
        if not self.receipt.is_file():
            raise RuntimeError('没有本机创建的恢复记录，未更改手机')
        value = json.loads(self.receipt.read_text())
        self.validate_receipt(value)
        if value.get('phase') == 'restored': return self.status(verify=True)
        return self.restore_settings(value)

    def install(self, apk, enable_ime):
        # Snapshot the source before validation to avoid source-file replacement during install.
        with tempfile.TemporaryDirectory(prefix='stepx-neo-local-') as temp:
            staged = Path(temp) / 'neo.apk'
            if not apk.is_file() or apk.stat().st_size > 200 * 1024 * 1024:
                raise RuntimeError('请选择不超过 200 MiB 的本地 APK')
            shutil.copyfile(apk, staged)
            package, cert = apk_identity(staged)
            if package != PACKAGE:
                raise RuntimeError('请选择内置 GUI Agent 的 StepX伴侣 APK：包名必须为 ' + PACKAGE)
            if not embedded_runtime(apk_manifest(staged)):
                raise RuntimeError('该 APK 未内置完整 GUI Agent 服务、输入法、控制接口与设置页，未安装')
            companion_cert = self.installed_cert(COMPANION)
            if not companion_cert:
                bundled = ROOT / 'desktop/assets/stepx-companion.apk'
                bundled_package, companion_cert = apk_identity(bundled)
                if bundled_package != COMPANION: raise RuntimeError('内置伴侣签名基线无效')
            if cert != companion_cert:
                raise RuntimeError('APK 必须与 StepX伴侣使用同一签名，未安装或修改权限')
            previous = None
            if self.receipt.is_file():
                previous = json.loads(self.receipt.read_text())
                self.validate_receipt(previous)
                if previous.get('phase') == 'restore_pending':
                    raise RuntimeError('上次恢复未完成，请先撤销权限适配后重试')
            # Retain the original permission baseline across repeated updates.
            active = previous and previous.get('phase') in ('ready', 'configured')
            existing = self.installed_cert(PACKAGE, self.folder / 'previous.apk')
            if existing and existing != cert:
                raise RuntimeError('已安装 Neo 签名不同，未覆盖应用')
            value = previous if active else {'schema': 2, 'scope': SCOPE, 'package': PACKAGE, 'identity': self.identity,
                    'signer': cert, 'created_at': int(time.time()), 'before': {
                    'enabled_accessibility_services': self.setting('enabled_accessibility_services'),
                    'accessibility_enabled': self.setting('accessibility_enabled'),
                    'enabled_input_methods': self.setting('enabled_input_methods'),
                    'default_input_method': self.setting('default_input_method'),
                    'doze': self.doze(), 'freezer': self.freezer()}}
            value.update(phase='prepared', had_apk=bool(existing), apk_sha256=hashlib.sha256(staged.read_bytes()).hexdigest())
            self.save(value)
            try:
                output = run(self.adb + ['install', '-r', '--user', '0', staged], timeout=180)
                if 'Success' not in output or self.installed_cert(PACKAGE) != cert or not self.has_embedded_runtime():
                    raise RuntimeError('APK 安装尚未通过签名回读')
                value['uid'] = self.installed_uid(); self.save(value)
                old = self.setting('enabled_accessibility_services')
                value['access_added'] = value.get('access_added', False) or not contains(old, ACCESS)
                value['master_changed'] = value.get('master_changed', False) or self.setting('accessibility_enabled') != '1'
                self.save(value)
                self.put_setting('enabled_accessibility_services', add_component(old, ACCESS))
                self.put_setting('accessibility_enabled', '1')
                if enable_ime and not contains(self.setting('enabled_input_methods'), IME):
                    value['ime_added'] = True; self.save(value)
                    self.shell('ime', 'enable', '--user', '0', IME)
                status = self.status()
                if not status['accessibility_enabled'] or enable_ime and not status['ime_enabled']:
                    raise RuntimeError('内置 GUI Agent 无障碍或输入法配置未通过回读')
                value['phase'] = 'configured'; self.save(value)
                return {**status, 'signature_verified': True}
            except Exception as error:
                restore_error = ''
                try:
                    self.restore_settings(value)
                    restore_error = '；本次权限配置已恢复，已安装的 APK 与数据保留'
                except Exception as recovery: restore_error = '；恢复待处理：' + str(recovery)
                # Never uninstall existing versions, clear data, or silently downgrade them.
                if existing:
                    restore_error += '；已有应用数据保留，安装前 APK 已备份；若版本已更新，不自动卸载降级'
                raise RuntimeError(str(error) + restore_error) from None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['inspect', 'install', 'restore'])
    parser.add_argument('--apk', type=Path)
    parser.add_argument('--enable-ime', action='store_true')
    args = parser.parse_args()
    installer = Installer()
    with file_lock(installer.folder / 'operation.lock'):
        if args.command == 'inspect': result = installer.status(verify=True)
        elif args.command == 'restore': result = installer.restore()
        else:
            if not args.apk or not args.apk.is_file(): raise RuntimeError('请先选择本地适配 APK')
            result = installer.install(args.apk, args.enable_ime)
        print(json.dumps({'ok': True, **result}, ensure_ascii=False))


if __name__ == '__main__':
    try: main()
    except Exception as error:
        print(json.dumps({'ok': False, 'error': str(error)}, ensure_ascii=False)); sys.exit(1)
