#!/usr/bin/env python3
"""Install an explicit, process-scoped Amoo lease endpoint on the verified StepX phone.

Only `apply` mutates the AgentBus process; preflight/inspect do not wake its UI.
No APK, credential, chat DB, display state, or system power policy is modified.
The signed StepX companion owns short leases and can remove the adapter with
the socket `shutdown` command. A native AgentBus restart also removes it.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('companion_permissions', ROOT / 'scripts/companion-permissions.py')
permissions = importlib.util.module_from_spec(spec)
spec.loader.exec_module(permissions)
AGENTBUS = 'com.stepos.agentbus'
AGENTBUS_APK = '/system/priv-app/AgentBus/AgentBus.apk'
AGENTBUS_SHA256 = '50a1bbd6af20d1cceff335e2336cff04e08d3718cf4cad581a86954c354f2a48'
ALLOWED_AGENTBUS_SHA256S = (
    AGENTBUS_SHA256,
    'e89a65e1a153e87fdec850caffe9a19db4ed70244754e847e0c512c540a902f8',
)
SOCKET = 'stepx-amoo-lease-v1'
STATE = Path(os.environ.get('XDG_STATE_HOME', Path.home() / '.local/state')) / 'stepx-companion/amoo-background'


def process_identity():
    pid = permissions.shell('pidof', AGENTBUS)
    if not re.fullmatch(r'[1-9][0-9]*', pid):
        raise RuntimeError('Expected exactly one running native AgentBus process')
    command = permissions.shell('cat', '/proc/' + pid + '/cmdline').rstrip('\0')
    if command != AGENTBUS:
        raise RuntimeError('AgentBus process identity mismatch')
    stat = permissions.shell('cat', '/proc/' + pid + '/stat')
    start = stat.rsplit(')', 1)[1].split()[19]
    if not start.isdigit():
        raise RuntimeError('Unable to verify AgentBus process start time')
    boot = permissions.shell('cat', '/proc/sys/kernel/random/boot_id')
    return {'pid': int(pid), 'start_ticks': start, 'boot_id': boot}


def inspect(apk):
    if not apk:
        raise RuntimeError('Provide the trusted local StepX companion APK with --apk')
    companion = permissions.inspect(apk)
    if not companion.get('signature_verified'):
        raise RuntimeError('Install the signed StepX companion first')
    paths = permissions.shell('pm', 'path', AGENTBUS).splitlines()
    if paths != ['package:' + AGENTBUS_APK]:
        raise RuntimeError('Unsupported AgentBus package location')
    digest = permissions.shell('sha256sum', AGENTBUS_APK).split()[0]
    if digest not in ALLOWED_AGENTBUS_SHA256S:
        raise RuntimeError('Native AgentBus version changed; review its ABI before adaptation')
    identity = process_identity()
    sockets = permissions.shell('cat', '/proc/net/unix')
    present = any(line.split()[-1:] == ['@' + SOCKET] for line in sockets.splitlines())
    return {'status': 'present' if present else 'not_installed', 'installed': present,
            'agentbus_package': AGENTBUS, 'agentbus_sha256': digest,
            'companion_uid': companion['uid'], 'signature_verified': True,
            'socket': SOCKET, 'scope': 'current_agentbus_process', 'reboot_verified': False,
            **identity}


def preflight(apk):
    result = inspect(apk)
    permissions.guard(native=True)
    dex = ROOT / 'desktop/assets/stepx-amoo-background.dex'
    payload = dex.read_bytes()
    if not payload.startswith(b'dex\n') or len(payload) > 128 * 1024:
        raise RuntimeError('Invalid Amoo background adapter DEX')
    result.update(dex_sha256=hashlib.sha256(payload).hexdigest(), native_images_verified=True)
    return result


def build_worker(dex, cert, uid, identity):
    source = (permissions.WORKFLOW / 'module/inject.py').read_text()
    if hashlib.sha256(source.encode()).hexdigest() != permissions.LOADER_SHA256:
        raise RuntimeError('Audited native loader changed; review required')
    source, count = re.subn(r'dex=base64\.b64decode\([^\n]+?\);dexp=',
                           'dex=base64.b64decode(LEASE_DEX);dexp=', source)
    if count != 1:
        raise RuntimeError('Unsupported native DEX template')
    substitutions = {
        'pid=int(os.environ["STEPX_SYSTEM_SERVER_PID"])': 'pid=LEASE_PID',
        "assert open('/proc/'+str(pid)+'/comm').read().strip()=='system_server'":
            "assert open('/proc/'+str(pid)+'/cmdline','rb').read().rstrip(b'\\x00')==b'com.stepos.agentbus'\n"
            "assert open('/proc/'+str(pid)+'/stat').read().rsplit(')',1)[1].split()[19]==LEASE_START\n"
            "assert hashlib.sha256(open(LEASE_APK,'rb').read()).hexdigest()==LEASE_APK_SHA256",
        "memfd=os.open('/proc/'+str(pid)+'/mem',os.O_RDWR)":
            "out['phase']='open_target_memory'\nmemfd=os.open('/proc/'+str(pid)+'/mem',os.O_RDWR)\nout['phase']='target_memory_opened'",
        "if open(p+'/comm').read().strip()=='android.ui':tids.append(int(n))":
            "if int(n)==pid:tids.append(int(n))",
        "cstr('stepx.GmsProbe')": "cstr('stepx.AgentBusLease')",
        "run=method(clazz,'run','()Ljava/lang/String;',True);answer=jni(116,clazz,run,0);":
            "run=method(clazz,'run','(Ljava/lang/String;I)Ljava/lang/String;',True);"
            "signer=jni(167,cstr(LEASE_SIGNER));answer=jni(116,clazz,run,argsv(signer,LEASE_UID));",
    }
    for old, new in substitutions.items():
        if source.count(old) != 1:
            raise RuntimeError('Unsupported native adapter template: ' + old[:40])
        source = source.replace(old, new)
    if 'stepx.GmsProbe' in source or 'ZGV4C' in source or 'STEPX_SYSTEM_SERVER_PID' in source:
        raise RuntimeError('Previous Google payload or system_server target was not removed')
    prefix = f'''LEASE_DEX={base64.b64encode(dex).decode()!r}
LEASE_SIGNER={cert!r}
LEASE_UID={uid!r}
LEASE_PID={identity['pid']!r}
LEASE_START={identity['start_ticks']!r}
LEASE_APK={AGENTBUS_APK!r}
LEASE_APK_SHA256={identity.get('agentbus_sha256', AGENTBUS_SHA256)!r}
'''
    code = prefix + source
    compile(code, 'stepx-amoo-background-worker', 'exec')
    return code


def save(path, value):
    STATE.mkdir(parents=True, exist_ok=True, mode=0o700); STATE.chmod(0o700)
    temporary = path.with_suffix('.new')
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump(value, stream, indent=2); stream.flush(); os.fsync(stream.fileno())
    os.replace(temporary, path)


def apply(apk, retry_reviewed=False):
    result = preflight(apk)
    if result['installed']:
        # Do not inject another arena, change signing policy, or refresh a task lease.
        return {**result, 'status': 'already_present', 'injected': False}
    key = hashlib.sha256((result['boot_id'] + ':' + str(result['pid']) + ':' + result['start_ticks']).encode()).hexdigest()[:24]
    record = STATE / (key + '.json')
    if record.exists() and not retry_reviewed:
        raise RuntimeError('Adapter already attempted in this process; inspect it before an explicit --retry-reviewed')
    cert = permissions.signer(apk)
    dex = (ROOT / 'desktop/assets/stepx-amoo-background.dex').read_bytes()
    code = build_worker(dex, cert, result['companion_uid'], result)
    # Ensure no PID replacement occurred between inspection and bounded injection.
    if process_identity() != {name: result[name] for name in ('pid', 'start_ticks', 'boot_id')}:
        raise RuntimeError('AgentBus restarted before apply; run preflight again')
    server_pid = int(permissions.shell('pidof', 'system_server'))
    save(record, {**result, 'attempted_at': int(time.time()), 'status': 'attempting'})
    proof = permissions.run_bounded_android_worker(code, server_pid, worker_context='agentbus')
    answer = json.loads(proof.get('java_result', '{}'))
    if proof.get('context') != 'u:r:system_app:s0' or proof.get('detached') is not True or answer.get('ok') is not True:
        save(record, {**result, 'status': 'failed', 'detached': proof.get('detached'),
                      'error': answer.get('error') or proof.get('error') or 'No Java result'})
        raise RuntimeError(answer.get('error') or proof.get('error') or 'AgentBus adapter did not validate')
    if answer.get('pid') != result['pid'] or answer.get('uid') != result['companion_uid'] or not answer.get('signature_verified'):
        raise RuntimeError('Unexpected Amoo adapter identity response')
    current = inspect(apk)
    if not current['installed'] or current['pid'] != result['pid'] or current['start_ticks'] != result['start_ticks']:
        raise RuntimeError('Amoo adapter endpoint could not be verified after apply')
    verified = {**result, **answer, 'status': 'installed', 'injected': True, 'detached': True,
                'startup_properties_restored': proof['startup_properties_restored'],
                'system_server_unchanged': proof['system_server_unchanged'],
                'google_permission_baseline_unchanged': True, 'root_bridge_healthy': True,
                'screen_state_changed': False, 'account_data_changed': False}
    save(record, verified)
    return verified


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['inspect', 'preflight', 'apply'])
    parser.add_argument('--apk', type=Path, required=True)
    parser.add_argument('--retry-reviewed', action='store_true', help='Explicitly retry one reviewed failed process injection')
    args = parser.parse_args()
    if args.command == 'inspect': result = inspect(args.apk)
    elif args.command == 'preflight': result = preflight(args.apk)
    else: result = apply(args.apk, args.retry_reviewed)
    print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    try: main()
    except Exception as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}, ensure_ascii=False))
        sys.exit(1)
