"""Host helpers. All device operations are pinned to one explicitly selected serial."""
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
CONFIG = json.loads((ROOT / 'target.json').read_text())
SERIAL = os.environ.get('STEPX_SERIAL') or os.environ.get('STEPX_DEVICE_SERIAL') or CONFIG.get('serial', '')
DEVICE_SERIAL = os.environ.get('STEPX_DEVICE_SERIAL', '')
ADB = [os.environ.get('STEPX_ADB', shutil.which('adb') or 'adb'), '-s', SERIAL]


sys.path.insert(0, str(ROOT.parents[1] / 'scripts'))
from host_runtime import sdk_tool, sdk_command, subprocess_options, user_state_root, migrate_legacy_state

STATE = user_state_root() / 'google-play/state'
DOWNLOADS = user_state_root() / 'google-play/downloads'


def run(args, **kwargs):
    return subprocess.run(args, text=True, capture_output=True, check=True,
                          timeout=kwargs.pop('timeout', 40), encoding='utf-8', **subprocess_options(), **kwargs)


def shell(*args):
    # adb shell joins arguments; quote empty values and metacharacters explicitly.
    return run(ADB + ['shell', shlex.join(str(x) for x in args)]).stdout.strip()


def guard(native=False):
    # ADB transport names (USB, IP:port or mDNS) are not hardware identities.
    # Wireless operations must carry the physical serial selected by the desktop.
    if not SERIAL or SERIAL.startswith('-') or any(c.isspace() for c in SERIAL):
        raise RuntimeError('请先选择已授权的手机连接')
    wireless = ':' in SERIAL or '._adb-tls-connect._tcp' in SERIAL
    expected = DEVICE_SERIAL or ('' if wireless else SERIAL)
    if not expected:
        raise RuntimeError('无线连接缺少设备身份，请重新选择手机后重试')
    if run(ADB + ['get-state']).stdout.strip() != 'device':
        raise RuntimeError('手机连接未授权或已离线，请重新连接')
    if shell('getprop', 'ro.serialno') != expected:
        raise RuntimeError('连接的手机与已选设备不一致，请重新选择设备')
    assert shell('getprop', 'ro.build.fingerprint') == CONFIG['fingerprint'], 'Unsupported firmware'
    assert shell('getprop', 'ro.build.version.sdk') == CONFIG['sdk'], 'Unsupported SDK'
    assert shell('getprop', 'ro.product.cpu.abilist') == CONFIG['abi'], 'Unsupported ABI'
    assert shell('getenforce') == 'Enforcing', 'Unexpected SELinux state'
    if native:
        for path, expected in CONFIG['native_hashes'].items():
            assert shell('sha256sum', path).split()[0] == expected, 'Native image mismatch: ' + path
    init_state()


def init_state():
    migrate_legacy_state(ROOT / 'state', STATE)
    STATE.mkdir(parents=True, mode=0o700, exist_ok=True)
    STATE.chmod(0o700)


def private_write(path, data):
    path = Path(path)
    if isinstance(data, str):
        data = data.encode()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, 'wb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    path.chmod(0o600)


def save_json(name, value):
    init_state()
    private_write(STATE / name, json.dumps(value, indent=2, ensure_ascii=False) + '\n')


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def root_json(code):
    result = run([sys.executable, str(ROOT / 'tools/root_bridge.py')], input=code, timeout=45)
    # The apt hook deliberately exits before apt update; its exit 100 is expected.
    # Success is established by the operation's structured proof, never that exit code.
    try:
        return json.JSONDecoder().raw_decode(result.stdout.lstrip())[0]
    except ValueError:
        raise RuntimeError('No valid root-operation proof; inspect the bridge locally') from None
