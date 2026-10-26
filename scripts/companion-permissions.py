#!/usr/bin/env python3
"""Manage fixed StepX伴侣 permission groups on the validated StepX firmware.

Reuses the existing Google workflow's bounded RootBridge transport and audited
native loader, but never runs its Google Java payload or changes Google state.
No boot hook or system/privileged package identity is installed.
"""
import argparse
import base64
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = Path(os.environ.get('STEPX_GOOGLE_HOME', ROOT / 'handoff/google-play-stepx')).resolve()
sys.path.insert(0, str(WORKFLOW / 'tools'))
from common import ADB, CONFIG, guard, root_json, shell
from host_runtime import file_lock, sdk_tool, sdk_command, java_environment, subprocess_options
from companion_apk import ensure_update_preserves_runtime, signer_digests
from companion_loader import adapt as adapt_native_loader

PACKAGE = 'com.stepx.companion.app'
PERMISSION = 'com.stepos.permission.MANAGE_BACK_SCREEN'
CONNECTION_PERMISSIONS = ('android.permission.MANAGE_DEBUGGING', 'android.permission.WRITE_SECURE_SETTINGS')
CONNECTION_SCOPE = 'connection_permissions_v1'
WAKE_PERMISSIONS = ('com.stepos.cloud.push.permission.MESSAGE',)
WAKE_OWNER = 'com.stepos.pushservice'
WAKE_SCOPE = 'wake_permission_v1'
AUTHORIZED_SERIAL = os.environ.get('STEPX_SERIAL') or os.environ.get('STEPX_DEVICE_SERIAL') or CONFIG.get('serial', '')
LOADER_SHA256 = '279305289749acada3f9951c816d7e9f00367dce4331d4396c04554e4685fbf8'
WORKER_SHA256 = '71be97798c805fc803b8eef47f9188aadd0c17f23a2b12ac607e8fa242ed188a'
STATE = Path(os.environ.get('XDG_STATE_HOME', Path.home() / '.local/state')) / 'stepx-companion/permissions'


def tool(name):
    return sdk_tool(name)


def run(args, timeout=30):
    result = subprocess.run(args, text=True, encoding='utf-8', capture_output=True,
                            timeout=timeout, env=java_environment(), **subprocess_options())
    if result.returncode:
        name = Path(args[0]).name
        detail = result.stderr.strip()[:1500] if name.lower() in ('java', 'java.exe', 'aapt', 'aapt.exe') else ''
        raise RuntimeError('Companion helper command failed: ' + name + (': ' + detail if detail else ''))
    return result.stdout


def signer(apk):
    if not apk.is_file():
        raise RuntimeError('Companion APK is missing')
    if (ROOT / 'runtime/android-build-tools/lib/apk-identity.jar').is_file():
        package = run(sdk_command('apk-identity') + [str(apk)]).strip()
    else:
        # Source checkouts may use a developer SDK instead of the shipped runtime.
        badging = run(sdk_command('aapt') + ['dump', 'badging', str(apk)])
        match = re.search(r"^package: name='([^']+)'", badging, re.M)
        package = match.group(1) if match else ''
    if package != PACKAGE:
        raise RuntimeError('APK is not StepX伴侣')
    result = run(sdk_command('apksigner') + ['verify', '--print-certs', str(apk)])
    matches = signer_digests(result)
    if len(matches) != 1:
        raise RuntimeError('Expected exactly one verified APK signer')
    return next(iter(matches))


def inspect(apk=None, preserve_embedded=False):
    # This command deliberately has no RootBridge call and does not start the app.
    if ADB[-1] != AUTHORIZED_SERIAL:
        raise RuntimeError('Companion adaptation is scoped to the authorized StepX phone')
    guard()
    dump = shell('dumpsys', 'package', PACKAGE)
    installed = re.search(r'Package \[' + re.escape(PACKAGE) + r'\]', dump) is not None
    uid_match = re.search(r'Package \[' + re.escape(PACKAGE) + r'\][^\n]*\n\s*(?:appId|userId)=(\d+)', dump)
    uid = int(uid_match.group(1)) if uid_match else None
    if installed and (uid is None or uid < 10000 or uid >= 100000):
        raise RuntimeError('Unable to verify companion user 0 UID')
    granted = re.search(r'^\s*' + re.escape(PERMISSION) + r': granted=true\b', dump, re.M) is not None
    result = {'package': PACKAGE, 'permission': PERMISSION, 'installed': installed,
              'granted': granted, 'uid': uid, 'scope': 'current_device_state', 'reboot_verified': False}
    connection = {name: re.search(r'^\s*' + re.escape(name) + r': granted=true\b', dump, re.M) is not None
                  for name in CONNECTION_PERMISSIONS}
    result.update(connection_permissions=connection, connection_permissions_granted=all(connection.values()))
    wake = {name: re.search(r'^\s*' + re.escape(name) + r': granted=true\b', dump, re.M) is not None
            for name in WAKE_PERMISSIONS}
    result.update(wake_permissions=wake, wake_permissions_granted=all(wake.values()))
    if apk:
        cert = signer(apk)
        if installed:
            package_paths = shell('pm', 'path', PACKAGE).splitlines()
            if len(package_paths) != 1 or not package_paths[0].startswith('package:/data/app/'):
                raise RuntimeError('Unexpected companion package location')
            with tempfile.TemporaryDirectory(prefix='stepx-companion-signature-') as temp:
                local = Path(temp) / 'installed.apk'
                run(ADB + ['pull', package_paths[0][8:], str(local)])
                if signer(local) != cert:
                    raise RuntimeError('Installed companion has a different signing certificate')
                if preserve_embedded:
                    result.update(ensure_update_preserves_runtime(apk, local))
        result['signature_verified'] = installed
    return result


def update_signer(apk, installed, trusted_apk=None):
    cert = signer(apk)
    if trusted_apk:
        if cert != signer(trusted_apk):
            raise RuntimeError('伴侣资源 APK 与客户端内置签名基线不一致，已阻止安装；请使用同签名完整安装包')
    elif not installed:
        raise RuntimeError('首次安装缺少客户端内置签名基线，未安装；请从 desktop 安装伴侣')
    return cert


def preflight(apk, trusted_apk=None):
    if not apk:
        raise RuntimeError('A verified local companion APK is required')
    summary = inspect(apk, preserve_embedded=True)
    cert = update_signer(apk, summary['installed'], trusted_apk)
    guard(native=True)
    if shell('getprop', 'sys.boot_completed') != '1' or 'RUNNING_UNLOCKED' not in shell('dumpsys', 'user'):
        raise RuntimeError('Unlock the StepX phone before companion installation')
    # RootBridge is read-only here: just verifies the existing authorized runtime.
    proof = root_json("import os,json; print(json.dumps({'root_available':os.geteuid()==0}))")
    if proof.get('root_available') is not True:
        raise RuntimeError('Existing StepX RootBridge is not available')
    dex = ROOT / 'desktop/assets/stepx-companion-permissions.dex'
    if not dex.is_file() or not dex.read_bytes().startswith(b'dex\n'):
        raise RuntimeError('Build the companion permission helper before installing')
    summary.update(status='ready', firmware_verified=True, root_available=True,
                   local_signature_verified=bool(cert), boot_hook_installed=False,
                   usb_install_master_enabled=shell('settings', 'get', 'secure', 'usb_install_item_switch') == 'switch:1')
    return summary


def private_json(path, value):
    STATE.mkdir(parents=True, exist_ok=True, mode=0o700)
    STATE.chmod(0o700)
    temp = path.with_suffix('.new')
    fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump(value, stream, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temp, path)


INSTALL_KEY = 'usb_install_item_' + PACKAGE
INSTALL_VALUE = 'StepX伴侣:1'


def install_gate_path():
    return STATE / ('install-gate-' + hashlib.sha256(ADB[-1].encode()).hexdigest()[:24] + '.json')


def prepare_install(apk, trusted_apk=None):
    proof = preflight(apk, trusted_apk)
    if not proof['usb_install_master_enabled']:
        raise RuntimeError('手机的 USB 安装总开关未开启；未修改全局开关')
    path = install_gate_path()
    if path.exists():
        raise RuntimeError('上次安装的单应用授权待恢复，请先执行 finish-install')
    previous = shell('settings', 'get', 'secure', INSTALL_KEY)
    baseline = {'package': PACKAGE, 'key': INSTALL_KEY, 'serial': ADB[-1],
                'fingerprint': CONFIG['fingerprint'], 'signer': signer(apk),
                'previous': None if previous == 'null' else previous,
                'temporary': INSTALL_VALUE, 'created_at': int(time.time()),
                'global_switch_changed': False}
    # Persist recovery information before changing the sole package-scoped key.
    private_json(path, baseline)
    shell('settings', 'put', 'secure', INSTALL_KEY, INSTALL_VALUE)
    if shell('settings', 'get', 'secure', INSTALL_KEY) != INSTALL_VALUE:
        raise RuntimeError('StepX伴侣 USB 安装授权回读失败；请执行 finish-install 恢复')
    return {'status': 'prepared', 'package': PACKAGE, 'temporary_install_allowed': True,
            'global_switch_changed': False, 'restore_required': True}


def finish_install():
    if ADB[-1] != AUTHORIZED_SERIAL:
        raise RuntimeError('Companion adaptation is scoped to the authorized StepX phone')
    guard()
    path = install_gate_path()
    if not path.exists():
        return {'status': 'unchanged', 'package': PACKAGE, 'restore_required': False}
    baseline = json.loads(path.read_text())
    if baseline.get('package') != PACKAGE or baseline.get('key') != INSTALL_KEY or baseline.get('serial') != ADB[-1] or baseline.get('fingerprint') != CONFIG['fingerprint'] or baseline.get('temporary') != INSTALL_VALUE:
        raise RuntimeError('USB installation recovery baseline mismatch')
    previous = baseline.get('previous')
    if previous is not None and (not isinstance(previous, str) or len(previous) > 512):
        raise RuntimeError('Invalid previous USB installation setting')
    current = shell('settings', 'get', 'secure', INSTALL_KEY)
    expected = 'null' if previous is None else previous
    if current not in (INSTALL_VALUE, expected):
        raise RuntimeError('StepX伴侣 USB 安装设置已被其他操作修改，未覆盖；保留恢复记录')
    if current != expected:
        if previous is None:
            shell('settings', 'delete', 'secure', INSTALL_KEY)
        else:
            shell('settings', 'put', 'secure', INSTALL_KEY, previous)
    if shell('settings', 'get', 'secure', INSTALL_KEY) != expected:
        raise RuntimeError('StepX伴侣 USB 安装设置尚未恢复；保留恢复记录')
    path.unlink()
    return {'status': 'restored', 'package': PACKAGE, 'restore_required': False,
            'global_switch_changed': False}


def baseline_path(cert, uid):
    return STATE / (hashlib.sha256((ADB[-1] + cert + ':' + str(uid)).encode()).hexdigest()[:24] + '.json')


def validate_baseline(prior, cert, uid):
    if prior and (prior.get('signer') != cert or prior.get('fingerprint') != CONFIG['fingerprint'] or prior.get('uid') != uid):
        raise RuntimeError('Companion permission baseline or UID mismatch')


def freeze_lists():
    """Read OEM lists, not AOSP cached-process freezer state or deviceidle."""
    text = shell('dumpsys', 'activity', 'freeze', 'whitelist')
    lists, section = {}, None
    for line in text.splitlines():
        header = re.fullmatch(r'\s*(cloud|user|local) freeze white apps:\s*', line)
        if header:
            section = header.group(1)
            lists[section] = []
        elif line.strip().startswith('freeze white app:'):
            if section is None:
                raise RuntimeError('Unrecognized OEM freezer list format')
            lists[section].append(line.split(':', 1)[1].strip())
    if set(lists) != {'cloud', 'user', 'local'}:
        raise RuntimeError('This firmware does not expose the verified OEM freezer interface')
    return lists


def frozen_uid(uid):
    text = shell('dumpsys', 'activity', 'freeze', 'freezeInfo')
    match = re.search(r'^UidStats \{uid: ' + str(uid) + r', pkgs: \{([^}]+)\}, currentState: (frozen|unfrozen),', text, re.M)
    if not match:
        return None  # The app has not started in this system_server session.
    if match.group(1).strip() != PACKAGE:
        raise RuntimeError('Companion UID is shared; refusing UID-wide freezer adaptation')
    return match.group(2) == 'frozen'


def runtime_identity(cert, uid):
    boot = shell('cat', '/proc/sys/kernel/random/boot_id')
    pid = shell('pidof', 'system_server')
    if not re.fullmatch(r'[0-9a-f-]{36}', boot) or not re.fullmatch(r'[1-9][0-9]*', pid):
        raise RuntimeError('Unable to verify the current OEM freezer runtime')
    identity = {'serial': ADB[-1], 'fingerprint': CONFIG['fingerprint'], 'package': PACKAGE,
                'signer': cert, 'uid': uid, 'boot_id': boot, 'system_server_pid': int(pid)}
    key = hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()[:24]
    return identity, STATE / ('runtime-' + key + '.json')


def runtime_status(uid):
    lists = freeze_lists()
    return {'background_exempt': any(PACKAGE in values for values in lists.values()),
            'background_frozen': frozen_uid(uid) if uid is not None else None,
            'background_scope': 'current_system_server', 'reboot_verified': False}


@contextmanager
def runtime_lock():
    STATE.mkdir(parents=True, exist_ok=True, mode=0o700)
    STATE.chmod(0o700)
    with file_lock(STATE / 'runtime.lock'):
        yield


def ensure_runtime(apk, initial=None):
    """One package-scoped exemption, with at most one thaw; never a keepalive."""
    with runtime_lock():
        return _ensure_runtime(apk, initial)


def _ensure_runtime(apk, initial=None):
    initial = initial or inspect(apk)
    if not apk or not initial.get('installed') or not initial.get('signature_verified'):
        raise RuntimeError('Install the verified companion APK before enabling background operation')
    cert, uid = signer(apk), initial['uid']
    identity, path = runtime_identity(cert, uid)
    before = freeze_lists()
    frozen = frozen_uid(uid)  # Also rejects a shared UID before any mutation.
    prior = json.loads(path.read_text()) if path.exists() else None
    if prior and prior.get('identity') != identity:
        raise RuntimeError('Companion background baseline does not match this runtime')
    added = False
    if not any(PACKAGE in values for values in before.values()):
        if not prior:
            prior = {'identity': identity, 'lists_before': before, 'added_by_tool': True,
                     'created_at': int(time.time()), 'state': 'prepared'}
            private_json(path, prior)  # Recovery precedes the single-package write.
        shell('dumpsys', 'activity', 'freeze', 'whiteAdd', PACKAGE)
        after = freeze_lists()
        if after['cloud'].count(PACKAGE) != 1 or after['local'].count(PACKAGE) != 1:
            raise RuntimeError('StepX伴侣 后台豁免回读失败；保留恢复记录')
        for section in before:
            if [p for p in after[section] if p != PACKAGE] != [p for p in before[section] if p != PACKAGE]:
                raise RuntimeError('OEM freezer lists changed concurrently; inspect before retrying')
        prior['state'] = 'applied'
        private_json(path, prior)
        added = True
    thawed = False
    if frozen is True:
        shell('dumpsys', 'activity', 'freeze', 'doUnfreeze', str(uid))
        for attempt in range(12):
            if frozen_uid(uid) is False:
                thawed = True
                break
            time.sleep(.1)
        if not thawed:
            raise RuntimeError('StepX伴侣 单应用解冻尚未通过回读')
    status = runtime_status(uid)
    if not status['background_exempt'] or status['background_frozen'] is True:
        raise RuntimeError('StepX伴侣 后台运行适配未通过校验')
    return {**initial, **status, 'status': 'runtime_ready', 'background_added': added,
            'background_thawed': thawed, 'background_global_settings_changed': False}


def revoke_runtime(apk, initial):
    with runtime_lock():
        return _revoke_runtime(apk, initial)


def _revoke_runtime(apk, initial):
    identity, path = runtime_identity(signer(apk), initial['uid'])
    if not path.exists():
        return {'background_reverted': False, 'background_owned': False}
    prior = json.loads(path.read_text())
    if prior.get('identity') != identity or prior.get('added_by_tool') is not True:
        raise RuntimeError('Companion background recovery baseline mismatch')
    baseline, current = prior['lists_before'], freeze_lists()
    if any(PACKAGE in values for values in baseline.values()):
        raise RuntimeError('Refusing to revoke a preexisting OEM freezer exemption')
    if current['user'].count(PACKAGE) or any(current[s].count(PACKAGE) > 1 for s in ('cloud', 'local')):
        raise RuntimeError('OEM freezer exemption changed externally; recovery record retained')
    if any(PACKAGE in values for values in current.values()):
        shell('dumpsys', 'activity', 'freeze', 'whiteRemove', PACKAGE)
        after = freeze_lists()
        if any(PACKAGE in values for values in after.values()):
            raise RuntimeError('StepX伴侣 后台豁免尚未恢复；保留恢复记录')
        for section in current:
            if after[section] != [p for p in current[section] if p != PACKAGE]:
                raise RuntimeError('OEM freezer lists changed during recovery; inspect before retrying')
    path.unlink()
    return {'background_reverted': True, 'background_owned': True}



def agentbus_worker_source(source):
    """Keep result I/O in the Ubuntu parent; child domain changes are observable.

    The audited transport still restores both startup properties and executes
    its original launcher. Only the bounded worker is adapted; no global policy
    or installed launcher changes are made.
    """
    if hashlib.sha256(source.encode()).hexdigest() != WORKER_SHA256:
        raise RuntimeError('Bounded root worker changed; review the AgentBus domain adapter')
    start = source.index("src=\"\"\"") if 'src=\"\"\"' in source else source.index("src=" + "'" * 3)
    end_marker = "'" * 3 + ".replace('signal.alarm(25)'"
    end = source.index(end_marker, start)
    template = AGENTBUS_WORKER_TEMPLATE
    source = source[:start] + "src=" + "'" * 3 + template + source[end:]
    substitutions = {
        "result='/data/system/stepx-gms-worker-'+tag+'.json'":
            "result='/tmp/stepx-background-worker-'+tag+'.json'",
        "repr('/proc/'+str(parent)+'/root'+result)": "repr(result)",
    }
    for old, new in substitutions.items():
        if source.count(old) != 1:
            raise RuntimeError('Unsupported bounded root worker transport template')
        source = source.replace(old, new)
    compile(source, 'agentbus-bounded-worker', 'exec')
    return source


AGENTBUS_WORKER_TEMPLATE = r'''#!/usr/bin/python3
import os,sys,json,socket,struct,ctypes,mmap,time,signal,base64,hashlib,binascii,stat,errno,re,subprocess,select,sqlite3,gzip,zlib,fcntl
lib=ctypes.CDLL(None,use_errno=True)
host='/proc/'+str(os.getppid())+'/root'
def prop(name,value):
 s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.settimeout(3)
 try:
  s.connect(host+'/dev/socket/property_service');n=name.encode();v=value.encode();s.sendall(struct.pack('<II',0x20001,len(n))+n+struct.pack('<I',len(v))+v)
  r=s.recv(4)
  if len(r)!=4 or struct.unpack('<I',r)[0]!=0:raise RuntimeError('property restore failed')
 finally:s.close()
prop('persist.openclaw.zygote.path','');prop('sys.openclaw.env.start','')
# Keep the existing boot adapters out of this diagnostic operation. File locks
# survive fork in the worker; the factory launcher closes its copies on exec.
boot_locks=[];lock_error=None
try:
 for path in ('/var/lib/stepx-gms/lock','/var/lib/stepx-companion/lock'):
  if not os.path.isdir(os.path.dirname(path)):continue
  fd=os.open(path,os.O_RDWR|os.O_CREAT,0o600);boot_locks.append(fd)
  fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
except OSError:lock_error='Existing boot restoration is active; no diagnostic executed'
shared=mmap.mmap(-1,256*1024,flags=mmap.MAP_SHARED|mmap.MAP_ANONYMOUS)
json.dumps({'warmup':True});hashlib.sha256(b'warmup');base64.b64decode('AA==')
def publish(value):
 data=json.dumps(value).encode()
 if len(data)>len(shared)-8:raise RuntimeError('worker proof exceeds fixed buffer')
 shared[4:4+len(data)]=data;shared[:4]=len(data).to_bytes(4,'little')
def snapshot():
 length=int.from_bytes(shared[:4],'little')
 return json.loads(shared[4:4+length]) if 0<length<=len(shared)-8 else {}
def save(value):
 fd=os.open(RESULT,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
 try:
  data=json.dumps(value).encode();written=0
  while written<len(data):written+=os.write(fd,data[written:])
  os.fsync(fd)
 finally:os.close(fd)
child=os.fork()
if child:os.execv('/usr/local/bin/ubuntu_zygote',['/usr/local/bin/ubuntu_zygote'])
# Supervisor retains Ubuntu namespace/domain and owns result I/O. The original
# launcher proceeds independently, as in the audited bounded transport.
out={'configuration_restored':True,'phase':'supervisor_start'}
try:
 os.setsid()
 fd=os.open(host+'/sys/fs/cgroup/cgroup.procs',os.O_WRONLY);os.write(fd,str(os.getpid()).encode());os.close(fd)
 rootfd=os.open(host,os.O_RDONLY|os.O_DIRECTORY)
 publish(out)
 worker=os.fork()
 if worker==0:
  def expired(a,b):raise TimeoutError('worker deadline')
  signal.signal(signal.SIGALRM,expired);signal.alarm(25)
  try:
   out['phase']='android_namespace';publish(out)
   os.fchdir(rootfd);os.chroot('.');os.chdir('/')
   out['phase']='worker_identity';publish(out)
   os.setresgid(1000,0,0);os.setresuid(1000,0,0);lib.setfsgid(1000);lib.setfsuid(1000)
   out['phase']='worker_domain';publish(out)
   fd=os.open('/proc/self/attr/current',os.O_WRONLY);os.write(fd,b'u:r:system_app:s0');os.close(fd)
   out.update(uid=os.getuid(),euid=os.geteuid(),context=open('/proc/self/attr/current').read().strip('\\x00\\n'),phase='payload')
   publish(out)
   if lock_error:raise RuntimeError(lock_error)
   exec(CODE,globals(),locals())
  except BaseException as error:out['error']=repr(error)
  finally:
   signal.alarm(0);publish(out);os._exit(0)
 os.close(rootfd)
 deadline=time.monotonic()+33
 while True:
  waited,status=os.waitpid(worker,os.WNOHANG)
  if waited:break
  if time.monotonic()>deadline:
   os.kill(worker,signal.SIGKILL);os.waitpid(worker,0)
   out=snapshot();out.update(error='Worker hard deadline',hard_deadline=True);save(out);os._exit(1)
  time.sleep(.02)
 out=snapshot();out['child_status']=status
 if status or 'context' not in out:out.setdefault('error','Worker exited before verified context')
except BaseException as error:out['error']=repr(error)
save(out);os._exit(0)
'''


def verify_system_server_stable(target_pid, seconds=20):
    """A successful native return does not exclude a delayed ART/GC crash."""
    until=time.monotonic()+seconds
    while True:
        if shell('pidof','system_server')!=str(target_pid):
            return False
        if time.monotonic()>=until:return True
        time.sleep(1)


def run_bounded_android_worker(code, target_pid, preserve_verified_result=None, worker_context='system_server'):
    if worker_context not in ('system_server', 'agentbus'):
        raise RuntimeError('Unsupported bounded worker context')
    quarantine=STATE/'native-quarantine.json'
    if 'pt(0x4206,t)' in code and quarantine.exists():
        previous=json.loads(quarantine.read_text())
        if previous.get('target_pid')==target_pid and previous.get('boot_id')==shell('cat','/proc/sys/kernel/random/boot_id'):
            raise RuntimeError('Previous native operation failed in this system process; automatic retry is disabled')
    properties = {key: shell('getprop', key) for key in ('persist.openclaw.zygote.path', 'sys.openclaw.env.start')}
    if any(properties.values()):
        raise RuntimeError('Another StepClaw startup operation is active; companion permission was not changed')
    expected_launcher = hashlib.sha256((WORKFLOW / 'module/launcher.py').read_bytes()).hexdigest()
    before = root_json("import os,json,hashlib; print(json.dumps({'launcher':hashlib.sha256(open('/usr/local/bin/ubuntu_zygote','rb').read()).hexdigest(),'root_available':os.geteuid()==0}))")
    if before.get('launcher') != expected_launcher or before.get('root_available') is not True:
        raise RuntimeError('Existing StepClaw/Google launcher differs from the verified installation')
    google_before = {}
    for package in ('com.google.android.gms', 'com.android.vending'):
        dump = shell('dumpsys', 'package', package)
        google_before[package] = 'android.permission.MANAGE_USERS: granted=true' in dump
    device_started = int(shell('date', '+%s'))
    with tempfile.TemporaryDirectory(prefix='stepx-companion-grant-') as directory:
        operation, output = Path(directory) / 'operation.py', Path(directory) / 'result.json'
        operation.write_text(code); operation.chmod(0o600)
        worker_path = WORKFLOW / 'tools/root_worker.py'
        if worker_context == 'agentbus':
            source = agentbus_worker_source(worker_path.read_text())
            worker_path = Path(directory) / 'agentbus-worker.py'
            worker_path.write_text('import sys\nsys.path.insert(0, ' + repr(str(WORKFLOW / 'tools')) + ')\n' + source)
            worker_path.chmod(0o600)
        worker = subprocess.run([sys.executable, str(worker_path), str(operation), str(output)],
            text=True, capture_output=True, timeout=130)
        # Save the native result before any postcondition can throw. A successful
        # grant's before_flags remain recoverable even if an unrelated guard fails.
        try:
            proof = json.loads(output.read_text()) if output.is_file() else {'error': 'Worker returned no result file'}
        except (ValueError, OSError) as error:
            proof = {'error': 'Worker result could not be parsed: ' + type(error).__name__}
        proof.update(worker_exit_code=worker.returncode, worker_code_sha256=hashlib.sha256(code.encode()).hexdigest(),
                     expected_system_server_pid=target_pid, google_permission_before=google_before,
                     device_started_epoch=device_started, worker_context=worker_context, validation_stage='worker_returned')
        private_json(STATE / 'last-worker-result.json', proof)
        # Worker restores both startup properties before entering Android context.
        # Verify independently even when execution or grant fails.
        restored = {key: shell('getprop', key) for key in properties} == properties
        unchanged = int(shell('pidof', 'system_server')) == target_pid
        proof.update(startup_properties_restored=restored, system_server_unchanged=unchanged,
                     validation_stage='startup_properties')
        private_json(STATE / 'last-worker-result.json', proof)
        if not restored or not unchanged:
            raise RuntimeError('Companion worker did not preserve startup properties/system_server; inspect before retrying')
        if proof.get('calls',0)>0:
            native_fault=bool(proof.get('error') or proof.get('jni_cleanup_error'))
            stable=verify_system_server_stable(target_pid)
            proof.update(system_server_stable=stable,stability_seconds=20,validation_stage='stability')
            private_json(STATE / 'last-worker-result.json',proof)
            if native_fault or not stable:
                private_json(quarantine,{'target_pid':target_pid,'boot_id':shell('cat','/proc/sys/kernel/random/boot_id'),'error':proof.get('error','Delayed system process restart')})
                raise RuntimeError('Native operation failed or Android restarted during validation; result saved and automatic retry disabled')
        after = root_json("import os,json,hashlib; print(json.dumps({'launcher':hashlib.sha256(open('/usr/local/bin/ubuntu_zygote','rb').read()).hexdigest(),'root_available':os.geteuid()==0}))")
        if after != before:
            raise RuntimeError('Companion worker could not verify original StepClaw launcher/runtime')
        proof.update(launcher_unchanged=True, root_bridge_healthy=True, validation_stage='launcher_verified')
        private_json(STATE / 'last-worker-result.json', proof)
        if worker.returncode or not output.is_file():
            raise RuntimeError('Bounded companion permission worker failed; startup properties were restored; worker proof was saved')
        if preserve_verified_result is not None:
            preserve_verified_result(proof)
        google_after = {package: 'android.permission.MANAGE_USERS: granted=true' in shell('dumpsys', 'package', package) for package in google_before}
        proof.update(google_permission_after=google_after, google_permission_baseline_unchanged=google_after == google_before,
                     validation_stage='google_baseline')
        private_json(STATE / 'last-worker-result.json', proof)
        if google_after != google_before:
            # This is diagnostic only. Never relax the guard or restore/modify
            # Google's independently owned permission state.
            try:
                repair = root_json("import json,os,time; p='/var/lib/stepx-gms/last.json'; d=json.load(open(p)) if os.path.isfile(p) else {}; print(json.dumps({'status':d.get('status'),'unix_time':d.get('unix_time'),'target_pid':d.get('target_pid'),'detached':d.get('detached'),'observed_epoch':int(time.time()),'manage_users':[l for l in d.get('java_result','').splitlines() if ' android.permission.MANAGE_USERS before=' in l]}))")
            except Exception:
                repair = {'status': 'unavailable'}
            proof['google_boot_repair'] = repair
            private_json(STATE / 'last-worker-result.json', proof)
            if (repair.get('status') == 'success' and repair.get('target_pid') == target_pid and
                    device_started <= repair.get('unix_time', 0) <= repair.get('observed_epoch', 0)):
                raise RuntimeError('原有 Google 开机修复在本次操作期间更新了 Google 权限；伴侣执行结果与恢复基线已保存。请待修复稳定后重试；未撤销或覆盖 Google 权限')
            raise RuntimeError('Google permission baseline changed during companion worker；已保存 worker 执行结果与权限前后状态，请核实后重试，未更改 Google 恢复状态')
        proof.update(validation_stage='complete')
        private_json(STATE / 'last-worker-result.json', proof)
        return proof


def inject(cert, grant, restore_flags, expected_uid):
    source = (WORKFLOW / 'module/inject.py').read_text()
    if hashlib.sha256(source.encode()).hexdigest() != LOADER_SHA256:
        raise RuntimeError('Native loader changed; review the companion adapter before reuse')
    source = adapt_native_loader(source)
    dex = (ROOT / 'desktop/assets/stepx-companion-permissions.dex').read_bytes()
    if not dex.startswith(b'dex\n') or len(dex) > 64 * 1024:
        raise RuntimeError('Invalid companion permission DEX')
    source, count = re.subn(r'dex=base64\.b64decode\([^\n]+?\);dexp=',
        'dex=base64.b64decode(COMPANION_DEX);dexp=', source)
    if count != 1 or "cstr('stepx.GmsProbe')" not in source:
        raise RuntimeError('Unsupported native loader template')
    source = source.replace("cstr('stepx.GmsProbe')", "cstr('stepx.CompanionGrant')")
    old = "run=method(clazz,'run','()Ljava/lang/String;',True);answer=jni(116,clazz,run,0);"
    new = "run=method(clazz,'run','(Ljava/lang/String;ZII)Ljava/lang/String;',True);signer=jni(167,cstr(COMPANION_SIGNER));answer=jni(116,clazz,run,argsv(signer,COMPANION_GRANT,COMPANION_RESTORE,COMPANION_UID));"
    if source.count(old) != 1:
        raise RuntimeError('Unsupported native entry point')
    source = source.replace(old, new)
    if 'stepx.GmsProbe' in source or 'ZGV4C' in source:
        raise RuntimeError('Google payload was not removed')
    target_pid = int(shell('pidof', 'system_server'))
    # Reuse the installed workflow's bounded init-context worker. It restores
    # the temporary launch properties before entering this operation and leaves
    # the existing launcher/Google adapter bytes untouched.
    prefix = f'''COMPANION_DEX={base64.b64encode(dex).decode()!r}
COMPANION_SIGNER={cert!r}
COMPANION_GRANT={1 if grant else 0}
COMPANION_RESTORE={restore_flags!r}
COMPANION_UID={expected_uid!r}
os.environ['STEPX_SYSTEM_SERVER_PID']={str(target_pid)!r}
{source}
'''
    compile(prefix, 'companion-permission-worker', 'exec')
    def preserve_grant_baseline(proof):
        if not grant or proof.get('detached') is not True:
            return
        answer = json.loads(proof.get('java_result', '{}'))
        if (answer.get('status') != 'success' or answer.get('package') != PACKAGE or
                answer.get('permission') != PERMISSION or answer.get('uid') != expected_uid or
                answer.get('signature_verified') is not True or answer.get('granted') is not True or
                type(answer.get('before_flags')) is not int):
            return
        path = baseline_path(cert, expected_uid)
        prior = json.loads(path.read_text()) if path.exists() else None
        validate_baseline(prior, cert, expected_uid)
        if prior is None:
            private_json(path, {'signer': cert, 'fingerprint': CONFIG['fingerprint'],
                                'before_flags': answer['before_flags'], 'uid': expected_uid})
    proof = run_bounded_android_worker(prefix, target_pid, preserve_grant_baseline)
    result = json.loads(proof.get('java_result', '{}'))
    if proof.get('detached') is not True or result.get('status') != 'success':
        raise RuntimeError(result.get('error') or proof.get('error') or 'Companion permission verification failed')
    if result.get('package') != PACKAGE or result.get('permission') != PERMISSION or result.get('signature_verified') is not True or result.get('uid') != expected_uid:
        raise RuntimeError('Unexpected permission grant response')
    if grant and result.get('granted') is not True:
        raise RuntimeError('Companion backscreen grant did not pass read-back')
    result.update(target_pid=target_pid, detached=True, reboot_verified=False,
                  startup_properties_restored=proof["startup_properties_restored"],
                  system_server_unchanged=proof["system_server_unchanged"],
                  google_permission_baseline_unchanged=True, launcher_unchanged=True, root_bridge_healthy=True)
    return result


def connection_baseline_path(cert, uid):
    # Deliberately separate from the legacy backscreen permission baseline.
    return STATE / ('connection-' + hashlib.sha256((ADB[-1] + cert + ':' + str(uid)).encode()).hexdigest()[:24] + '.json')


def validate_connection_flags(flags):
    if not isinstance(flags, dict) or set(flags) != set(CONNECTION_PERMISSIONS):
        raise RuntimeError('Exact fixed connection permission baseline is required')
    if any(type(value) is not int or not -(2**31) <= value < 2**31 for value in flags.values()):
        raise RuntimeError('Invalid connection permission baseline flags')


def inject_connection(cert, operation, before_flags, expected_uid):
    if operation not in ('inspect', 'grant', 'revoke'):
        raise RuntimeError('Unsupported connection permission operation')
    if operation != 'inspect':
        validate_connection_flags(before_flags)
    source = (WORKFLOW / 'module/inject.py').read_text()
    if hashlib.sha256(source.encode()).hexdigest() != LOADER_SHA256:
        raise RuntimeError('Native loader changed; review the connection permission adapter')
    source = adapt_native_loader(source)
    dex = (ROOT / 'desktop/assets/stepx-companion-permissions.dex').read_bytes()
    if not dex.startswith(b'dex\n') or len(dex) > 64 * 1024:
        raise RuntimeError('Invalid companion permission DEX')
    source, count = re.subn(r'dex=base64\.b64decode\([^\n]+?\);dexp=',
        'dex=base64.b64decode(CONNECTION_DEX);dexp=', source)
    if count != 1 or source.count("cstr('stepx.GmsProbe')") != 1:
        raise RuntimeError('Unsupported connection permission loader template')
    source = source.replace("cstr('stepx.GmsProbe')", "cstr('stepx.CompanionGrant')")
    old = "run=method(clazz,'run','()Ljava/lang/String;',True);answer=jni(116,clazz,run,0);"
    new = ("run=method(clazz,'runScoped','(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;I)Ljava/lang/String;',True);"
           "signer=jni(167,cstr(CONNECTION_SIGNER));operation=jni(167,cstr(CONNECTION_OPERATION));"
           "baseline=jni(167,cstr(CONNECTION_BASELINE));answer=jni(116,clazz,run,argsv(signer,operation,baseline,CONNECTION_UID));")
    if source.count(old) != 1:
        raise RuntimeError('Unsupported connection permission entry point')
    source = source.replace(old, new)
    if 'stepx.GmsProbe' in source or 'ZGV4C' in source:
        raise RuntimeError('Google payload was not removed from connection adapter')
    target_pid = int(shell('pidof', 'system_server'))
    code = f'''CONNECTION_DEX={base64.b64encode(dex).decode()!r}
CONNECTION_SIGNER={cert!r}
CONNECTION_OPERATION={operation!r}
CONNECTION_BASELINE={json.dumps(before_flags, separators=(',', ':'))!r}
CONNECTION_UID={expected_uid!r}
os.environ['STEPX_SYSTEM_SERVER_PID']={str(target_pid)!r}
{source}
'''
    compile(code, 'companion-connection-permission-worker', 'exec')
    proof = run_bounded_android_worker(code, target_pid)
    result = json.loads(proof.get('java_result', '{}'))
    if proof.get('detached') is not True or result.get('status') != 'success':
        raise RuntimeError(result.get('error') or proof.get('error') or 'Connection permission adapter failed')
    if (result.get('scope') != CONNECTION_SCOPE or result.get('operation') != operation or
            result.get('package') != PACKAGE or result.get('uid') != expected_uid or result.get('signature_verified') is not True or
            set(result.get('permissions', {})) != set(CONNECTION_PERMISSIONS)):
        raise RuntimeError('Unexpected connection permission adapter response')
    for name, detail in result['permissions'].items():
        if detail.get('owner') != 'android' or detail.get('declared') is not True or detail.get('signature_permission') is not True:
            raise RuntimeError('Connection permission identity was not verified')
        if operation != 'inspect':
            expected = ((before_flags[name] & ~2) | 4) if operation == 'grant' else before_flags[name]
            if detail.get('after_flags') != expected:
                raise RuntimeError('Connection permission flags did not match the exact requested baseline')
    if operation == 'grant' and result.get('all_granted') is not True:
        raise RuntimeError('Connection permission grant read-back failed')
    result.update(target_pid=target_pid, detached=True, reboot_verified=False,
                  startup_properties_restored=proof['startup_properties_restored'],
                  system_server_unchanged=proof['system_server_unchanged'],
                  google_permission_baseline_unchanged=True, root_bridge_healthy=True)
    return result


def connection_permissions(apk, operation):
    if operation not in ('inspect', 'grant', 'revoke'):
        raise RuntimeError('Unsupported connection permission operation')
    initial = inspect(apk)
    if not apk or not initial['installed'] or not initial.get('signature_verified'):
        raise RuntimeError('Install the verified companion APK before managing connection permissions')
    guard(native=True)
    cert = signer(apk)
    if operation == 'inspect':
        return inject_connection(cert, operation, {}, initial['uid'])
    state_path = connection_baseline_path(cert, initial['uid'])
    prior = json.loads(state_path.read_text()) if state_path.exists() else None
    if prior:
        validate_baseline(prior, cert, initial['uid'])
        allowed_serials = {s for s in (ADB[-1], os.environ.get('STEPX_DEVICE_SERIAL', '')) if s}
        if prior.get('scope') != CONNECTION_SCOPE or prior.get('serial') not in allowed_serials:
            raise RuntimeError('Connection permission baseline scope/device mismatch')
        validate_connection_flags(prior.get('before_flags'))
        if operation == 'grant' and initial.get('connection_permissions_granted') is True:
            # The signed installed package/UID and our exact restoration baseline
            # are already verified. Do not reinject a native worker just to grant
            # permissions that PackageManager confirms are still present.
            return {**initial, 'status': 'already_granted', 'scope': CONNECTION_SCOPE,
                    'operation': 'grant', 'all_granted': True, 'injected': False,
                    'baseline_preserved': True}
    elif operation == 'revoke':
        raise RuntimeError('No connection permission baseline available; refusing to invent flags')
    else:
        # Read native flags first and durably save recovery before any permission change.
        readback = inject_connection(cert, 'inspect', {}, initial['uid'])
        before_flags = {name: readback['permissions'][name]['before_flags'] for name in CONNECTION_PERMISSIONS}
        validate_connection_flags(before_flags)
        prior = {'signer': cert, 'fingerprint': CONFIG['fingerprint'], 'uid': initial['uid'],
                 'serial': ADB[-1], 'scope': CONNECTION_SCOPE, 'before_flags': before_flags,
                 'created_at': int(time.time())}
        private_json(state_path, prior)
    try:
        result = inject_connection(cert, operation, prior['before_flags'], initial['uid'])
    except RuntimeError as error:
        if operation != 'grant' or 'Permission changed since baseline' not in str(error):
            raise
        readback = inject_connection(cert, 'inspect', {}, initial['uid'])
        before_flags = {name: readback['permissions'][name]['before_flags'] for name in CONNECTION_PERMISSIONS}
        validate_connection_flags(before_flags)
        prior = {'signer': cert, 'fingerprint': CONFIG['fingerprint'], 'uid': initial['uid'],
                 'serial': ADB[-1], 'scope': CONNECTION_SCOPE, 'before_flags': before_flags,
                 'created_at': int(time.time())}
        private_json(state_path, prior)
        result = inject_connection(cert, operation, prior['before_flags'], initial['uid'])
    private_json(STATE / 'last-connection-result.json', result)
    return result


def wake_baseline_path(cert, uid):
    # Independent of both backscreen and connection recovery baselines.
    return STATE / ('wake-' + hashlib.sha256((ADB[-1] + cert + ':' + str(uid)).encode()).hexdigest()[:24] + '.json')


def validate_wake_flags(flags):
    if not isinstance(flags, dict) or set(flags) != set(WAKE_PERMISSIONS):
        raise RuntimeError('Exact fixed wake permission baseline is required')
    if any(type(value) is not int or not -(2**31) <= value < 2**31 for value in flags.values()):
        raise RuntimeError('Invalid wake permission baseline flags')


def inject_wake(cert, operation, before_flags, expected_uid):
    if operation not in ('inspect', 'grant', 'revoke'):
        raise RuntimeError('Unsupported wake permission operation')
    if operation != 'inspect':
        validate_wake_flags(before_flags)
    source = (WORKFLOW / 'module/inject.py').read_text()
    if hashlib.sha256(source.encode()).hexdigest() != LOADER_SHA256:
        raise RuntimeError('Native loader changed; review the wake permission adapter')
    source = adapt_native_loader(source)
    dex = (ROOT / 'desktop/assets/stepx-companion-permissions.dex').read_bytes()
    if not dex.startswith(b'dex\n') or len(dex) > 64 * 1024:
        raise RuntimeError('Invalid companion permission DEX')
    source, count = re.subn(r'dex=base64\.b64decode\([^\n]+?\);dexp=',
        'dex=base64.b64decode(WAKE_DEX);dexp=', source)
    if count != 1 or source.count("cstr('stepx.GmsProbe')") != 1:
        raise RuntimeError('Unsupported wake permission loader template')
    source = source.replace("cstr('stepx.GmsProbe')", "cstr('stepx.CompanionGrant')")
    old = "run=method(clazz,'run','()Ljava/lang/String;',True);answer=jni(116,clazz,run,0);"
    new = ("run=method(clazz,'runWake','(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;I)Ljava/lang/String;',True);"
           "signer=jni(167,cstr(WAKE_SIGNER));operation=jni(167,cstr(WAKE_OPERATION));"
           "baseline=jni(167,cstr(WAKE_BASELINE));answer=jni(116,clazz,run,argsv(signer,operation,baseline,WAKE_UID));")
    if source.count(old) != 1:
        raise RuntimeError('Unsupported wake permission entry point')
    source = source.replace(old, new)
    if 'stepx.GmsProbe' in source or 'ZGV4C' in source:
        raise RuntimeError('Google payload was not removed from wake adapter')
    target_pid = int(shell('pidof', 'system_server'))
    code = f'''WAKE_DEX={base64.b64encode(dex).decode()!r}
WAKE_SIGNER={cert!r}
WAKE_OPERATION={operation!r}
WAKE_BASELINE={json.dumps(before_flags, separators=(',', ':'))!r}
WAKE_UID={expected_uid!r}
os.environ['STEPX_SYSTEM_SERVER_PID']={str(target_pid)!r}
{source}
'''
    compile(code, 'companion-wake-permission-worker', 'exec')
    proof = run_bounded_android_worker(code, target_pid)
    result = json.loads(proof.get('java_result', '{}'))
    if proof.get('detached') is not True or result.get('status') != 'success':
        raise RuntimeError(result.get('error') or proof.get('error') or 'Wake permission adapter failed')
    validate_wake_response(result, operation, before_flags, expected_uid)
    result.update(target_pid=target_pid, detached=True, reboot_verified=False,
                  startup_properties_restored=proof['startup_properties_restored'],
                  system_server_unchanged=proof['system_server_unchanged'],
                  google_permission_baseline_unchanged=True, root_bridge_healthy=True)
    return result


def validate_wake_response(result, operation, before_flags, expected_uid):
    if (result.get('scope') != WAKE_SCOPE or result.get('operation') != operation or
            result.get('package') != PACKAGE or result.get('uid') != expected_uid or result.get('signature_verified') is not True or
            set(result.get('permissions', {})) != set(WAKE_PERMISSIONS)):
        raise RuntimeError('Unexpected wake permission adapter response')
    for name, detail in result['permissions'].items():
        if detail.get('owner') != WAKE_OWNER or detail.get('declared') is not True or detail.get('signature_permission') is not True:
            raise RuntimeError('Wake permission identity was not verified')
        if operation != 'inspect':
            expected = ((before_flags[name] & ~2) | 4) if operation == 'grant' else before_flags[name]
            if detail.get('after_flags') != expected:
                raise RuntimeError('Wake permission flags did not match the exact requested baseline')
    if operation == 'grant' and (result.get('all_granted') is not True or
            any(detail.get('granted') is not True for detail in result['permissions'].values())):
        raise RuntimeError('Wake permission grant read-back failed')

def wake_permissions(apk, operation):
    if operation not in ('inspect', 'grant', 'revoke'):
        raise RuntimeError('Unsupported wake permission operation')
    initial = inspect(apk)
    if not apk or not initial['installed'] or not initial.get('signature_verified'):
        raise RuntimeError('Install the verified companion APK before managing wake permissions')
    guard(native=True)
    cert = signer(apk)
    if operation == 'inspect':
        return inject_wake(cert, operation, {}, initial['uid'])
    state_path = wake_baseline_path(cert, initial['uid'])
    prior = json.loads(state_path.read_text()) if state_path.exists() else None
    if prior:
        validate_baseline(prior, cert, initial['uid'])
        allowed_serials = {s for s in (ADB[-1], os.environ.get('STEPX_DEVICE_SERIAL', '')) if s}
        if (prior.get('scope') != WAKE_SCOPE or prior.get('serial') not in allowed_serials or
                prior.get('owner') != WAKE_OWNER):
            raise RuntimeError('Wake permission baseline scope/device mismatch')
        validate_wake_flags(prior.get('before_flags'))
        if operation == 'grant' and initial.get('wake_permissions_granted') is True:
            # The signed installed package/UID and our exact restoration baseline
            # are already verified. Do not reinject a native worker just to grant
            # permissions that PackageManager confirms are still present.
            return {**initial, 'status': 'already_granted', 'scope': WAKE_SCOPE,
                    'operation': 'grant', 'all_granted': True, 'injected': False,
                    'baseline_preserved': True}
    elif operation == 'revoke':
        raise RuntimeError('No wake permission baseline available; refusing to invent flags')
    else:
        # Read native flags first and durably save recovery before any permission change.
        readback = inject_wake(cert, 'inspect', {}, initial['uid'])
        before_flags = {name: readback['permissions'][name]['before_flags'] for name in WAKE_PERMISSIONS}
        validate_wake_flags(before_flags)
        prior = {'signer': cert, 'fingerprint': CONFIG['fingerprint'], 'uid': initial['uid'],
                 'serial': ADB[-1], 'scope': WAKE_SCOPE, 'owner': WAKE_OWNER, 'before_flags': before_flags,
                 'created_at': int(time.time())}
        private_json(state_path, prior)
    try:
        result = inject_wake(cert, operation, prior['before_flags'], initial['uid'])
    except RuntimeError as error:
        if operation != 'grant' or 'Permission changed since baseline' not in str(error):
            raise
        readback = inject_wake(cert, 'inspect', {}, initial['uid'])
        before_flags = {name: readback['permissions'][name]['before_flags'] for name in WAKE_PERMISSIONS}
        validate_wake_flags(before_flags)
        prior = {'signer': cert, 'fingerprint': CONFIG['fingerprint'], 'uid': initial['uid'],
                 'serial': ADB[-1], 'scope': WAKE_SCOPE, 'owner': WAKE_OWNER, 'before_flags': before_flags,
                 'created_at': int(time.time())}
        private_json(state_path, prior)
        result = inject_wake(cert, operation, prior['before_flags'], initial['uid'])
    private_json(STATE / 'last-wake-result.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['preflight', 'inspect', 'prepare-install', 'finish-install', 'grant', 'revoke', 'runtime',
                                           'inspect-connection', 'grant-connection', 'revoke-connection',
                                           'inspect-wake', 'grant-wake', 'revoke-wake'])
    parser.add_argument('--apk', type=Path)
    parser.add_argument('--trusted-apk', type=Path, help='Client embedded APK snapshot used as the signing trust anchor')
    args = parser.parse_args()
    if args.command.endswith('-wake'):
        print(json.dumps(wake_permissions(args.apk, args.command.split('-', 1)[0]), ensure_ascii=False)); return
    if args.command.endswith('-connection'):
        print(json.dumps(connection_permissions(args.apk, args.command.split('-', 1)[0]), ensure_ascii=False)); return
    if args.command == 'preflight':
        print(json.dumps(preflight(args.apk, args.trusted_apk), ensure_ascii=False)); return
    if args.command == 'prepare-install':
        print(json.dumps(prepare_install(args.apk, args.trusted_apk), ensure_ascii=False)); return
    if args.command == 'finish-install':
        print(json.dumps(finish_install(), ensure_ascii=False)); return
    initial = inspect(args.apk)
    if args.command == 'inspect':
        print(json.dumps({**initial, **runtime_status(initial['uid'])}, ensure_ascii=False)); return
    if args.command == 'runtime':
        print(json.dumps(ensure_runtime(args.apk, initial), ensure_ascii=False)); return
    if not args.apk or not initial['installed'] or not initial.get('signature_verified'):
        raise RuntimeError('Install the verified companion APK before granting permission')
    guard(native=True)
    cert = signer(args.apk)
    state_path = baseline_path(cert, initial['uid'])
    prior = json.loads(state_path.read_text()) if state_path.exists() else None
    if args.command == 'grant' and initial['granted']:
        if not prior:
            private_json(state_path, {'signer': cert, 'fingerprint': CONFIG['fingerprint'],
                                      'before_flags': 0, 'uid': initial['uid']})
        print(json.dumps({**ensure_runtime(args.apk, initial), 'status': 'already_granted'}, ensure_ascii=False)); return
    if args.command == 'revoke' and not prior:
        raise RuntimeError('No companion grant baseline available; refusing to invent permission flags')
    validate_baseline(prior, cert, initial['uid'])
    result = inject(cert, args.command == 'grant', 0 if prior is None else prior['before_flags'], initial['uid'])
    if args.command == 'grant' and not prior:
        private_json(state_path, {'signer': cert, 'fingerprint': CONFIG['fingerprint'],
                                 'before_flags': result['before_flags'], 'uid': result['uid']})
    if args.command == 'grant':
        runtime = ensure_runtime(args.apk, {**initial, 'granted': True})
        result.update({key: value for key, value in runtime.items() if key.startswith('background_')})
    else:
        result.update(revoke_runtime(args.apk, initial))
    private_json(STATE / 'last-result.json', result)
    print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}, ensure_ascii=False))
        print(str(error), file=sys.stderr)
        sys.exit(1)
