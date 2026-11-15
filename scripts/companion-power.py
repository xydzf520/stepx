#!/usr/bin/env python3
"""Manage only StepX伴侣's Android Doze exemption, retaining its original state.

This is separate from the OEM freezer adapter. No global idle mode, timeout,
other application or battery policy is changed. Restore removes only a user
whitelist entry which this tool previously added and verified.
"""
import argparse
from contextlib import contextmanager
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT / 'scripts') not in sys.path:
    sys.path.insert(0, str(ROOT / 'scripts'))
from host_runtime import migrate_legacy_state, optional_tool_home, user_state_root
spec = importlib.util.spec_from_file_location('companion_permissions', ROOT / 'scripts/companion-permissions.py')
permissions = importlib.util.module_from_spec(spec)
spec.loader.exec_module(permissions)
PACKAGE = 'com.stepx.companion.app'
_DEFAULT_STATE = user_state_root() / 'power'
STATE = _DEFAULT_STATE
SCOPE = 'companion_doze_user_whitelist_v1'


def _ensure_state_dir():
    if STATE == _DEFAULT_STATE and not os.environ.get('STEPX_STATE_DIR') and not STATE.exists():
        home = optional_tool_home()
        if home is not None:
            legacy_base = Path(os.environ.get('XDG_STATE_HOME') or (home / '.local/state'))
            legacy = legacy_base / 'stepx-companion/power'
            if legacy != STATE and legacy.exists():
                migrate_legacy_state(legacy, STATE)
    STATE.mkdir(parents=True, exist_ok=True, mode=0o700)
    if STATE.is_symlink() or not STATE.is_dir():
        raise RuntimeError('Doze 恢复记录目录无效')
    if os.name != 'nt':
        STATE.chmod(0o700)
    return STATE


def parse_whitelist(text):
    """AOSP deviceidle emits scope,package,appId; unknown output fails closed."""
    result = {kind: {} for kind in ('system-excidle', 'system', 'user')}
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        match = re.fullmatch(r'(system-excidle|system|user),([A-Za-z0-9_.]+),([0-9]+)', line)
        if not match or match[2] in result[match[1]]:
            raise RuntimeError('Android Doze 白名单返回了无法识别的状态')
        result[match[1]][match[2]] = int(match[3])
    return result


def whitelist():
    return parse_whitelist(permissions.shell('cmd', 'deviceidle', 'whitelist'))


def target_state(lists, uid):
    for entries in lists.values():
        if PACKAGE in entries and entries[PACKAGE] != uid:
            raise RuntimeError('StepX伴侣 Doze 白名单 UID 与当前安装不一致')
    return {kind: PACKAGE in entries for kind, entries in lists.items()}


def verified_identity(apk):
    if not apk:
        raise RuntimeError('请提供已验证的 StepX伴侣 APK')
    app = permissions.inspect(apk)
    if not app.get('installed') or not app.get('signature_verified'):
        raise RuntimeError('请先安装签名一致的 StepX伴侣')
    return {'scope': SCOPE, 'package': PACKAGE, 'serial': permissions.shell('getprop', 'ro.serialno'),
            'fingerprint': permissions.CONFIG['fingerprint'], 'signer': permissions.signer(apk), 'uid': app['uid']}


def state_path(identity):
    digest = hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()[:24]
    return STATE / (digest + '.json')


def load_state(path, identity):
    if not path.exists():
        return None
    if path.is_symlink() or path.stat().st_size > 32768:
        raise RuntimeError('Doze 恢复记录无效')
    previous = json.loads(path.read_text(encoding='utf-8'))
    if previous.get('identity') != identity or previous.get('phase') not in ('prepared', 'applied', 'revoked'):
        raise RuntimeError('Doze 恢复记录与当前设备或安装不匹配')
    before = previous.get('before', {})
    if set(before) != {'system-excidle', 'system', 'user'} or any(type(v) is not bool for v in before.values()):
        raise RuntimeError('Doze 原状态记录不完整')
    if type(previous.get('added_by_tool')) is not bool:
        raise RuntimeError('Doze 恢复记录缺少授权来源')
    if previous['added_by_tool'] and (before['system'] or before['user']):
        raise RuntimeError('不能把预先存在的电池豁免标记为本工具添加')
    return previous


def save_state(path, value):
    _ensure_state_dir()
    with tempfile.NamedTemporaryFile(mode='w', encoding='utf-8', dir=STATE, delete=False) as file:
        temporary = Path(file.name)
        os.chmod(temporary, 0o600)
        json.dump(value, file, ensure_ascii=False, indent=2)
        file.flush()
        os.fsync(file.fileno())
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


@contextmanager
def locked():
    _ensure_state_dir()
    with permissions.file_lock(STATE / 'power.lock'):
        yield


def result(identity, current, previous, status):
    return {'status': status, 'package': PACKAGE, 'uid': identity['uid'], 'scope': SCOPE,
            'doze_exempt': current['user'] or current['system'],
            'doze_user_exempt': current['user'], 'doze_system_exempt': current['system'],
            'doze_except_idle_exempt': current['system-excidle'],
            'doze_added_by_tool': bool(previous and previous['added_by_tool'] and previous['phase'] == 'applied'),
            'global_policy_changed': False, 'oem_freezer_changed': False}


def unrelated(lists):
    return {kind: {name: uid for name, uid in entries.items() if name != PACKAGE}
            for kind, entries in lists.items()}


def operate(apk, operation):
    if operation not in ('inspect', 'apply', 'revoke-owned'):
        raise RuntimeError('未知的 Doze 操作')
    identity = verified_identity(apk)
    with locked():
        path = state_path(identity)
        previous = load_state(path, identity)
        before_lists = whitelist()
        current = target_state(before_lists, identity['uid'])
        if operation == 'inspect':
            return result(identity, current, previous, 'inspected')
        if operation == 'apply':
            if previous is None or previous['phase'] == 'revoked':
                previous = {'identity': identity, 'before': current, 'phase': 'prepared',
                            'added_by_tool': not (current['user'] or current['system']), 'created_at': int(time.time())}
                save_state(path, previous)  # Recovery always precedes the single-package write.
            if not current['user'] and not current['system']:
                permissions.shell('cmd', 'deviceidle', 'whitelist', '+' + PACKAGE)
                after_lists = whitelist()
                current = target_state(after_lists, identity['uid'])
                if not current['user'] or unrelated(after_lists) != unrelated(before_lists):
                    raise RuntimeError('Doze 单应用豁免回读未通过；已保留恢复记录')
            previous['phase'] = 'applied'
            save_state(path, previous)
            return result(identity, current, previous, 'applied')
        if not previous or not previous['added_by_tool'] or previous['phase'] == 'revoked':
            return result(identity, current, previous, 'unchanged')
        if previous['phase'] != 'applied':
            raise RuntimeError('上次 Doze 修改未完成核验，不能确定归属；保留记录供检查')
        if current['user']:
            permissions.shell('cmd', 'deviceidle', 'whitelist', '-' + PACKAGE)
            after_lists = whitelist()
            current = target_state(after_lists, identity['uid'])
            if current['user'] or unrelated(after_lists) != unrelated(before_lists):
                raise RuntimeError('Doze 单应用豁免未恢复；已保留恢复记录')
        previous['phase'] = 'revoked'
        save_state(path, previous)
        return result(identity, current, previous, 'revoked')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['inspect', 'apply', 'revoke-owned'])
    parser.add_argument('--apk', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(operate(args.apk, args.command), ensure_ascii=False))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}, ensure_ascii=False))
        print(str(error), file=sys.stderr)
        sys.exit(1)
