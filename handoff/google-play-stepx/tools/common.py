"""Host helpers. All device operations are pinned to one explicitly selected serial."""
import hashlib
import atexit
from contextlib import contextmanager, ExitStack
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import threading
import tempfile

ROOT = Path(__file__).resolve().parents[1]
for _extra_path in (ROOT.parents[1] / 'scripts', ROOT / 'tools'):
    if str(_extra_path) not in sys.path:
        sys.path.insert(0, str(_extra_path))
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


def _is_wireless_transport(value):
    return ':' in value or '._adb-' in value


CONFIG = json.loads((ROOT / 'target.json').read_text(encoding='utf-8'))
ALLOWED_FINGERPRINTS = tuple(dict.fromkeys(CONFIG.get('fingerprints') or [
    CONFIG['fingerprint'],
    'StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/12:user/release-keys',
    'StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys',
]))
SERIAL = os.environ.get('STEPX_SERIAL') or os.environ.get('STEPX_DEVICE_SERIAL') or CONFIG.get('serial', '')
DEVICE_SERIAL = os.environ.get('STEPX_DEVICE_SERIAL', '')
EXPECTED_HW_SERIAL = DEVICE_SERIAL or (SERIAL if SERIAL and not _is_wireless_transport(SERIAL) else '')
_INITIAL_SERIAL = SERIAL
_active_transport = SERIAL
_failover_from = None


def _current_transport():
    if SERIAL != _INITIAL_SERIAL and _failover_from != SERIAL:
        return SERIAL
    return _active_transport


class _AdbCommand:
    """Dynamic sequence proxy reading live [_resolve_adb(), '-s', _active_transport]."""

    def _items(self):
        return [_resolve_adb(), '-s', _current_transport()]

    def __iter__(self):
        return iter(self._items())

    def __getitem__(self, index):
        return self._items()[index]

    def __len__(self):
        return 3

    def __add__(self, other):
        return self._items() + list(other)

    def __radd__(self, other):
        return list(other) + self._items()

    def __repr__(self):
        return repr(self._items())

    def __eq__(self, other):
        if isinstance(other, (list, tuple, _AdbCommand)):
            return self._items() == list(other)
        return NotImplemented

    def copy(self):
        return list(self._items())


ADB = _AdbCommand()

DEVICE_KEY = DEVICE_SERIAL or (SERIAL if not _is_wireless_transport(SERIAL) else '')
STATE = user_state_root() / 'google-play/devices' / hashlib.sha256(DEVICE_KEY.encode()).hexdigest()[:24] / 'state'
DOWNLOADS = user_state_root() / 'google-play/downloads'
_operation_scope = threading.local()
_DROP_MARKERS = (
    'offline',
    'not found',
    'closed',
    'protocol fault',
    'connection reset',
    'cannot connect',
    'no devices/emulators found',
    'broken pipe',
)
_STALE_WORKER_PATH_RE = re.compile(
    r'^(?:/(?:data/local/tmp|tmp)/stepx-gms-w[A-Za-z0-9._-]*\.py|/usr/local/lib/stepx-gms-worker-dispatch\.py)$'
)


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


def _is_transport_drop(exc):
    if isinstance(exc, subprocess.TimeoutExpired):
        return True
    if isinstance(exc, subprocess.CalledProcessError):
        detail = ((exc.stderr or '') + '\n' + (exc.stdout or '')).lower()
        return any(marker in detail for marker in _DROP_MARKERS)
    return False


def _is_idempotent_adb_args(args):
    if not isinstance(args, (list, tuple)) or len(args) < 4 or args[1] != '-s':
        return False
    sub = [str(x) for x in args[3:]]
    if sub == ['get-state'] or sub[:1] == ['pull'] or sub[:2] == ['forward', 'tcp:0']:
        return True
    if sub[0] in ('shell', 'exec-out') and len(sub) >= 2:
        try:
            tokens = shlex.split(sub[1]) if len(sub) == 2 else sub[1:]
        except ValueError:
            tokens = sub[1:]
        if not tokens:
            return False
        head = tokens[0]
        if head in ('getprop', 'getenforce', 'sha256sum', 'cat', 'ls', 'ps', 'pidof', 'date', 'stat', 'readlink'):
            return True
        if head == 'dumpsys' and not ({'set', 'reset', 'enable', 'disable'} & set(tokens[1:])):
            return True
        if tokens[:2] in (['pm', 'path'], ['pm', 'list'], ['settings', 'get']):
            return True
    return False


def recover_active_transport(expected_hw=None):
    """Scan online ADB transports and switch _active_transport only to verified EXPECTED_HW_SERIAL."""
    global _active_transport, _failover_from, EXPECTED_HW_SERIAL
    if expected_hw is not None:
        hw = expected_hw.strip()
    elif SERIAL != _INITIAL_SERIAL:
        hw = (DEVICE_SERIAL or ('' if _is_wireless_transport(SERIAL) else SERIAL)).strip()
    else:
        hw = (EXPECTED_HW_SERIAL or DEVICE_SERIAL or ('' if _is_wireless_transport(SERIAL) else SERIAL)).strip()
    if not hw or hw.startswith('-') or any(c.isspace() for c in hw):
        return None
    adb_bin = _resolve_adb()
    try:
        devices_res = subprocess.run(
            [adb_bin, 'devices'],
            text=True, capture_output=True, timeout=5, encoding='utf-8', **subprocess_options(),
        )
    except (OSError, subprocess.SubprocessError):
        return None
    online = []
    for line in (devices_res.stdout or '').splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith('List of devices'):
            continue
        parts = stripped.split()
        if len(parts) >= 2 and parts[1] == 'device':
            online.append(parts[0])
    ordered = [c for c in online if c == hw] + [c for c in online if c != hw]
    for cand in ordered:
        try:
            probe = subprocess.run(
                [adb_bin, '-s', cand, 'shell', 'getprop', 'ro.serialno'],
                text=True, capture_output=True, timeout=5, encoding='utf-8', **subprocess_options(),
            )
        except (OSError, subprocess.SubprocessError):
            continue
        if probe.returncode == 0 and probe.stdout.strip() == hw:
            _active_transport = cand
            _failover_from = SERIAL
            EXPECTED_HW_SERIAL = hw
            os.environ['STEPX_SERIAL'] = cand
            os.environ['STEPX_DEVICE_SERIAL'] = hw
            return cand
    return None


def run(args, **kwargs):
    timeout = kwargs.pop('timeout', 40)
    opts = {**subprocess_options(), **kwargs}
    cmd = list(args) if isinstance(args, _AdbCommand) else args
    try:
        return subprocess.run(cmd, text=True, capture_output=True, check=True,
                              timeout=timeout, encoding='utf-8', **opts)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as exc:
        if isinstance(cmd, (list, tuple)) and len(cmd) >= 4 and cmd[1] == '-s' and _is_transport_drop(exc):
            recovered = recover_active_transport()
            if recovered and _is_idempotent_adb_args(cmd):
                replayed = [cmd[0], '-s', recovered] + list(cmd[3:])
                return subprocess.run(replayed, text=True, capture_output=True, check=True,
                                      timeout=timeout, encoding='utf-8', **opts)
        raise


def shell(*args):
    # adb shell joins arguments; quote empty values and metacharacters explicitly.
    return run(ADB + ['shell', shlex.join(str(x) for x in args)]).stdout.strip()


def guard(native=False):
    global EXPECTED_HW_SERIAL, _active_transport
    # ADB transport names (USB, IP:port or mDNS) are not hardware identities.
    # Wireless operations must carry the physical serial selected by the desktop.
    if not SERIAL or SERIAL.startswith('-') or any(c.isspace() for c in SERIAL):
        raise RuntimeError('请先选择已授权的手机连接')
    wireless = _is_wireless_transport(SERIAL)
    expected = DEVICE_SERIAL or ('' if wireless else SERIAL)
    if not expected:
        raise RuntimeError('无线连接缺少设备身份，请重新选择手机后重试')
    try:
        state = run(ADB + ['get-state']).stdout.strip()
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as exc:
        if not (_is_transport_drop(exc) and recover_active_transport(expected)):
            raise
        state = run(ADB + ['get-state']).stdout.strip()
    if state != 'device':
        if state != 'unauthorized' and recover_active_transport(expected):
            state = run(ADB + ['get-state']).stdout.strip()
        if state != 'device':
            raise RuntimeError('手机连接未授权或已离线，请重新连接')
    if shell('getprop', 'ro.serialno') != expected:
        if not os.environ.get('STEPX_SERIAL') and not os.environ.get('STEPX_DEVICE_SERIAL'):
            raise RuntimeError('连接的手机与已选设备不一致，请重新选择设备（命令行运行请设置环境变量 STEPX_SERIAL=<设备序列号>）')
        raise RuntimeError('连接的手机与已选设备不一致，请重新选择设备')
    EXPECTED_HW_SERIAL = expected
    if _failover_from != SERIAL:
        _active_transport = SERIAL
    actual_fp = shell('getprop', 'ro.build.fingerprint')
    if actual_fp not in ALLOWED_FINGERPRINTS:
        raise WorkflowError('unsupported_firmware', '当前手机固件尚未通过 Google 安装适配验证。请先更新手机系统或桌面客户端，再重新检测；重复安装不会解决，现有数据已保留。')
    actual_sdk = shell('getprop', 'ro.build.version.sdk')
    if actual_sdk != CONFIG['sdk'] or shell('getprop', 'ro.product.cpu.abilist') != CONFIG['abi']:
        raise WorkflowError('unsupported_firmware', '当前系统版本或处理器架构不在已验证的 Google 安装范围；请更新客户端后重新检测。')
    if shell('getenforce') != 'Enforcing':
        raise WorkflowError('unsupported_firmware', '手机系统安全状态与已验证配置不同，已停止安装和自动修复。')
    if native:
        for path, expected_hash in CONFIG['native_hashes'].items():
            if shell('sha256sum', path).split()[0] != expected_hash:
                raise WorkflowError('unsupported_firmware', '手机系统关键库与已验证固件不一致，已停止安装。请更新客户端适配后重新检测。')
    init_state()


def init_state():
    # Recovery evidence belongs to a hardware identity, not an IP/ADB port.
    for legacy in (user_state_root() / 'google-play/state', ROOT / 'state'):
        if STATE.exists(): break
        try: baseline = json.loads((legacy / 'baseline.json').read_text(encoding='utf-8'))
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
    raw = (result.stdout or '').lstrip()
    if not raw:
        raise RuntimeError('No valid root-operation proof; inspect the bridge locally')
    try:
        proof = json.JSONDecoder().raw_decode(raw)[0]
        if not isinstance(proof, dict):
            raise ValueError('Operation proof must be a JSON object')
        return proof
    except ValueError:
        raise RuntimeError('No valid root-operation proof; inspect the bridge locally') from None


def heal_stale_zygote_prop(shell_fn=None, root_fn=None):
    """Clear stale /data/local/tmp/stepx-gms-w*.py or /tmp/stepx-gms-w*.py when no active worker exists."""
    sh = shell if shell_fn is None else shell_fn
    rj = root_json if root_fn is None else root_fn
    current = sh('getprop', 'persist.openclaw.zygote.path')
    if not current or not _STALE_WORKER_PATH_RE.fullmatch(current):
        return current
    ps_out = sh('ps', '-A', '-o', 'PID,PPID,S,NAME,ARGS')
    worker_alive = False
    parent_pid = None
    for line in ps_out.splitlines()[1:]:
        parts = line.split(None, 4)
        if len(parts) < 4:
            continue
        if parts[3] == 'openclaw_env' and parts[1] == '1' and parts[0].isdigit():
            parent_pid = int(parts[0])
        rest = parts[3] + (' ' + parts[4] if len(parts) > 4 else '')
        if not parts[2].startswith('Z') and 'stepx-gms-w' in rest:
            worker_alive = True
    file_exists = True
    if current.startswith('/data/local/tmp/'):
        file_exists = sh('sh', '-c', 'test -f ' + shlex.quote(current) + ' && echo 1 || echo 0') == '1'
    elif current == '/usr/local/lib/stepx-gms-worker-dispatch.py':
        try:
            probe = rj(
                'import json,os,time;p="/tmp/stepx-gms-worker-active";'
                'ok=os.path.isfile(p) and (time.time()-os.lstat(p).st_mtime)<90;'
                'print(json.dumps({"exists":ok}))'
            )
            file_exists = bool(probe.get('exists'))
        except Exception:
            file_exists = False
        if file_exists:
            return current
    elif current.startswith('/tmp/'):
        try:
            probe = rj('import json,os;print(json.dumps({"exists":os.path.isfile(' + repr(current) + ')}))')
            file_exists = bool(probe.get('exists'))
        except Exception:
            file_exists = False
    if worker_alive and file_exists:
        return current
    try:
        sh('setprop', 'persist.openclaw.zygote.path', '')
    except Exception:
        pass
    try:
        sh('sh', '-c', 'rm -f /data/local/tmp/stepx-gms-w*')
    except Exception:
        pass
    after = sh('getprop', 'persist.openclaw.zygote.path')
    if after or current.startswith('/tmp/') or current == '/usr/local/lib/stepx-gms-worker-dispatch.py':
        try:
            cleanup_code = (
                'import glob,json,os,socket,struct\n'
                'for item in glob.glob("/tmp/stepx-gms-w*"):\n'
                ' try:\n'
                '  if not os.path.islink(item): os.unlink(item)\n'
                ' except OSError: pass\n'
            )
            if after and parent_pid is not None:
                prop_sock = f'/proc/{parent_pid}/root/dev/socket/property_service'
                cleanup_code += (
                    f'p={prop_sock!r}\n'
                    'for name in ("persist.openclaw.zygote.path","sys.openclaw.env.start"):\n'
                    ' s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.settimeout(5);s.connect(p)\n'
                    ' n=name.encode();s.sendall(struct.pack("<II",0x20001,len(n))+n+struct.pack("<I",0));s.recv(4);s.close()\n'
                    f'for item in glob.glob("/proc/{parent_pid}/root/data/local/tmp/stepx-gms-w*"):\n'
                    ' try:\n'
                    '  if not os.path.islink(item): os.unlink(item)\n'
                    ' except OSError: pass\n'
                )
            cleanup_code += 'print(json.dumps({"cleared":True}))\n'
            rj(cleanup_code)
        except Exception:
            pass
        after = sh('getprop', 'persist.openclaw.zygote.path')
    return after
