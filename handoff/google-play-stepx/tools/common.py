"""Host helpers. All device operations are pinned to one explicitly selected serial."""
import hashlib
import atexit
from contextlib import contextmanager, ExitStack
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import threading
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT.parents[1] / 'scripts'))
from host_runtime import sdk_tool, sdk_command, subprocess_options, user_state_root, migrate_legacy_state
from workflow_errors import WorkflowError


def _resolve_adb():
    env_adb = os.environ.get('STEPX_ADB')
    if env_adb:
        return env_adb
    adb_name = 'adb.exe' if os.name == 'nt' else 'adb'
    for rel in ('tools/bin', '.tools/bin'):
        candidate = ROOT.parents[1] / rel / adb_name
        if candidate.is_file():
            return str(candidate)
    return shutil.which('adb') or 'adb'


CONFIG = json.loads((ROOT / 'target.json').read_text())
ALLOWED_FINGERPRINTS = tuple(dict.fromkeys(CONFIG.get('fingerprints') or [
    CONFIG['fingerprint'],
    'StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/12:user/release-keys',
    'StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys',
]))
SERIAL = os.environ.get('STEPX_SERIAL') or os.environ.get('STEPX_DEVICE_SERIAL') or CONFIG.get('serial', '')
DEVICE_SERIAL = os.environ.get('STEPX_DEVICE_SERIAL', '')
ADB = [_resolve_adb(), '-s', SERIAL]

DEVICE_KEY = DEVICE_SERIAL or (SERIAL if ':' not in SERIAL and '._adb-' not in SERIAL else '')
STATE = user_state_root() / 'google-play/devices' / hashlib.sha256(DEVICE_KEY.encode()).hexdigest()[:24] / 'state'
DOWNLOADS = user_state_root() / 'google-play/downloads'
_operation_scope = threading.local()


class OperationBusy(RuntimeError):
    pass


@contextmanager
def operation_lock(path=None):
    """Nonblocking, process-wide and reentrant within one operation thread."""
    init_state()
    path = Path(path) if path is not None else STATE / 'operation.lock'
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    key = str(path.resolve())
    held = getattr(_operation_scope, 'held', None)
    if held is None:
        held = _operation_scope.held = set()
    if key in held:
        try: yield
        finally: pass
        return
    if path.is_symlink():
        raise RuntimeError('Invalid Google operation lock')
    fd = os.open(path, os.O_RDWR | os.O_CREAT | getattr(os, 'O_NOFOLLOW', 0), 0o600)
    with os.fdopen(fd, 'r+b') as stream:
        if os.name == 'nt':
            import msvcrt
            if os.fstat(fd).st_size == 0:
                stream.write(b'\0'); stream.flush()
            stream.seek(0)
            try: msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
            except OSError: raise OperationBusy('Google 安装或修复正在运行；自动维护已延期') from None
            unlock = lambda: (stream.seek(0), msvcrt.locking(fd, msvcrt.LK_UNLCK, 1))
        else:
            import fcntl
            try: fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError: raise OperationBusy('Google 安装或修复正在运行；自动维护已延期') from None
            unlock = lambda: fcntl.flock(fd, fcntl.LOCK_UN)
        held.add(key)
        try: yield
        finally:
            held.remove(key)
            unlock()


def hold_operation_lock():
    """Keep legacy top-level installation scripts locked through interpreter exit."""
    scope = ExitStack()
    scope.enter_context(operation_lock())
    atexit.register(scope.close)
    return scope


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
        if not os.environ.get('STEPX_SERIAL') and not os.environ.get('STEPX_DEVICE_SERIAL'):
            raise RuntimeError('连接的手机与已选设备不一致，请重新选择设备（命令行运行请设置环境变量 STEPX_SERIAL=<设备序列号>）')
        raise RuntimeError('连接的手机与已选设备不一致，请重新选择设备')
    actual_fp = shell('getprop', 'ro.build.fingerprint')
    if actual_fp not in ALLOWED_FINGERPRINTS:
        raise WorkflowError('unsupported_firmware', '当前手机固件尚未通过 Google 安装适配验证。请先更新手机系统或桌面客户端，再重新检测；重复安装不会解决，现有数据已保留。')
    actual_sdk = shell('getprop', 'ro.build.version.sdk')
    if actual_sdk != CONFIG['sdk'] or shell('getprop', 'ro.product.cpu.abilist') != CONFIG['abi']:
        raise WorkflowError('unsupported_firmware', '当前系统版本或处理器架构不在已验证的 Google 安装范围；请更新客户端后重新检测。')
    if shell('getenforce') != 'Enforcing':
        raise WorkflowError('unsupported_firmware', '手机系统安全状态与已验证配置不同，已停止安装和自动修复。')
    if native:
        for path, expected in CONFIG['native_hashes'].items():
            if shell('sha256sum', path).split()[0] != expected:
                raise WorkflowError('unsupported_firmware', '手机系统关键库与已验证固件不一致，已停止安装。请更新客户端适配后重新检测。')
    init_state()


def init_state():
    # Recovery evidence belongs to a hardware identity, not an IP/ADB port.
    for legacy in (user_state_root() / 'google-play/state', ROOT / 'state'):
        if STATE.exists(): break
        try: baseline = json.loads((legacy / 'baseline.json').read_text())
        except (OSError,ValueError): continue
        if baseline.get('serial') in (DEVICE_KEY, SERIAL) and DEVICE_KEY:
            migrate_legacy_state(legacy, STATE)
    STATE.mkdir(parents=True, mode=0o700, exist_ok=True)
    STATE.chmod(0o700)


def baseline_matches(value):
    return bool(DEVICE_KEY) and value.get('serial') in (DEVICE_KEY, SERIAL) and value.get('fingerprint') in ALLOWED_FINGERPRINTS


def verified_fingerprint():
    """The target profile lists compatible firmware; it is not device evidence."""
    actual = shell('getprop', 'ro.build.fingerprint')
    if actual not in ALLOWED_FINGERPRINTS:
        raise RuntimeError('Unsupported firmware; recovery metadata was not changed')
    return actual


def private_write(path, data):
    path = Path(path)
    if isinstance(data, str):
        data = data.encode()
    if path.is_symlink():
        raise RuntimeError('恢复记录路径异常，未覆盖现有记录')
    fd, temporary = tempfile.mkstemp(prefix='.' + path.name + '-', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        if os.name != 'nt':
            directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
            try: os.fsync(directory)
            finally: os.close(directory)
    finally:
        if os.path.exists(temporary): os.unlink(temporary)


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
    result = run([sys.executable, str(ROOT / 'tools/root_bridge.py')], input=code, timeout=150)
    # The apt hook deliberately exits before apt update; its exit 100 is expected.
    # Success is established by the operation's structured proof, never that exit code.
    try:
        proof = json.JSONDecoder().raw_decode(result.stdout.lstrip())[0]
        if not isinstance(proof, dict):
            raise ValueError('Operation proof must be a JSON object')
        return proof
    except ValueError:
        raise RuntimeError('No valid root-operation proof; inspect the bridge locally') from None
