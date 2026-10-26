#!/usr/bin/env python3
"""Load a bounded inspect-only adapter into the pinned original GUI process.

Requires an independently verified backup. No APK, permission, model, credential,
chat database or display setting is changed. An idle watchdog closes the probe.
This research tool does not claim native GUI execution has been enabled.
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
ABILITY = 'com.stepos.ability'
ABILITY_APK = '/system/priv-app/ability/ability.apk'
ABILITY_SHA256 = 'c9f56f3078df96d8d726422b1c622a1c19fe8c7dec8cadd8537d9a6ecf513ac4'
SOCKET = 'stepx-native-gui-v1'
PROBE_DEX_SHA256 = '2a53e3d0707e43bbf7851f4a93e092f327f26c115adde04dcc3d3d88077f8a38'
STATE = Path(os.environ.get('XDG_STATE_HOME', Path.home() / '.local/state')) / 'stepx-companion/native-gui'


def process_identity():
    pid = permissions.shell('pidof', ABILITY)
    if not re.fullmatch(r'[1-9][0-9]*', pid):
        raise RuntimeError('Expected exactly one running native Ability process')
    command = permissions.shell('cat', '/proc/' + pid + '/cmdline').rstrip('\0')
    if command != ABILITY:
        raise RuntimeError('Ability process identity mismatch')
    stat = permissions.shell('cat', '/proc/' + pid + '/stat')
    start = stat.rsplit(')', 1)[1].split()[19]
    if not start.isdigit():
        raise RuntimeError('Unable to verify Ability process start time')
    boot = permissions.shell('cat', '/proc/sys/kernel/random/boot_id')
    return {'pid': int(pid), 'start_ticks': start, 'boot_id': boot}


def inspect(apk):
    if not apk:
        raise RuntimeError('Provide the trusted local StepX companion APK with --apk')
    companion = permissions.inspect(apk)
    if not companion.get('signature_verified'):
        raise RuntimeError('Install the signed StepX companion first')
    paths = permissions.shell('pm', 'path', ABILITY).splitlines()
    if paths != ['package:' + ABILITY_APK]:
        raise RuntimeError('Unsupported Ability package location')
    digest = permissions.shell('sha256sum', ABILITY_APK).split()[0]
    if digest != ABILITY_SHA256:
        raise RuntimeError('Native Ability version changed; review its ABI before adaptation')
    identity = process_identity()
    sockets = permissions.shell('cat', '/proc/net/unix')
    present = any(line.split()[-1:] == ['@' + SOCKET] for line in sockets.splitlines())
    return {'status': 'probe_present' if present else 'not_installed', 'installed': present, 'execution_enabled': False,
            'ability_package': ABILITY, 'ability_sha256': digest,
            'companion_uid': companion['uid'], 'signature_verified': True,
            'socket': SOCKET, 'scope': 'current_ability_process', 'reboot_verified': False,
            **identity}


def read_reviewed_probe():
    payload = (ROOT / 'desktop/assets/stepx-native-gui.dex').read_bytes()
    if hashlib.sha256(payload).hexdigest() != PROBE_DEX_SHA256:
        raise RuntimeError('Only the reviewed inspect-only native GUI probe DEX is allowed')
    return payload


def preflight(apk):
    result = inspect(apk)
    permissions.guard(native=True)
    expected_serial = os.environ.get('STEPX_DEVICE_SERIAL') or permissions.AUTHORIZED_SERIAL
    if permissions.shell('getprop', 'ro.serialno') != expected_serial:
        raise RuntimeError('Physical device identity mismatch')
    payload = read_reviewed_probe()
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
            "out['phase']='guard_cmdline'\nassert open('/proc/'+str(pid)+'/cmdline','rb').read().rstrip(b'\\x00')==b'com.stepos.ability'\n"
            "out['phase']='guard_start'\nassert open('/proc/'+str(pid)+'/stat').read().rsplit(')',1)[1].split()[19]==LEASE_START\n"
            "out['phase']='guard_boot'\nassert open('/proc/sys/kernel/random/boot_id').read().strip()==LEASE_BOOT\n"
            "out['phase']='guard_context'\nassert open('/proc/'+str(pid)+'/attr/current').read().rstrip('\\x00').strip()=='u:r:system_app:s0'\n"
            "out['phase']='guard_uid'\nassert [int(x) for x in re.search(r'^Uid:\\s+(.*)$',open('/proc/'+str(pid)+'/status').read(),re.M).group(1).split()]==[1000]*4\n"
            "out['phase']='guard_apk'\nassert hashlib.sha256(open(LEASE_APK,'rb').read()).hexdigest()==LEASE_APK_SHA256\n"
            "out['phase']='native_image_guard'",
        "memfd=os.open('/proc/'+str(pid)+'/mem',os.O_RDWR)":
            "out['phase']='open_target_memory'\nmemfd=os.open('/proc/'+str(pid)+'/mem',os.O_RDWR)\nout['phase']='target_memory_opened'",
        "if open(p+'/comm').read().strip()=='android.ui':tids.append(int(n))":
            "if int(n)==pid:tids.append(int(n))",
        "cstr('stepx.GmsProbe')": "cstr('stepx.NativeGuiAdapter')",
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
LEASE_BOOT={identity['boot_id']!r}
LEASE_APK={ABILITY_APK!r}
LEASE_APK_SHA256={ABILITY_SHA256!r}
'''
    code = 'import re\n' + prefix + source
    compile(code, 'stepx-amoo-background-worker', 'exec')
    return code


def save(path, value):
    STATE.mkdir(parents=True, exist_ok=True, mode=0o700); STATE.chmod(0o700)
    temporary = path.with_suffix('.new')
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump(value, stream, indent=2); stream.flush(); os.fsync(stream.fileno())
    os.replace(temporary, path)


def apply(apk, backup, retry_reviewed=False):
    spec = importlib.util.spec_from_file_location('gui_native_backup', ROOT / 'scripts/gui-native-backup.py')
    recovery = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(recovery)
    backup_proof = recovery.verify_backup(recovery.Adb(permissions.ADB[0], permissions.ADB[-1]), backup)
    if not backup_proof['system_server_unchanged']:
        raise RuntimeError('System server changed since backup; create a fresh recovery snapshot')
    if not backup_proof['original_process_still_alive']:
        raise RuntimeError('Ability process changed since backup; create a fresh recovery snapshot')
    result = preflight(apk)
    if any(result[key] != backup_proof['current_process'][key] for key in ('pid', 'start_ticks', 'boot_id')):
        raise RuntimeError('Ability changed after backup verification; create a fresh recovery snapshot')
    if result['installed']:
        # Do not inject another arena, change signing policy, or refresh a task lease.
        return {**result, 'status': 'already_present', 'injected': False}
    key = hashlib.sha256((result['boot_id'] + ':' + str(result['pid']) + ':' + result['start_ticks']).encode()).hexdigest()[:24]
    record = STATE / (key + '.json')
    if record.exists() and not retry_reviewed:
        raise RuntimeError('Adapter already attempted in this process; inspect it before an explicit --retry-reviewed')
    cert = permissions.signer(apk)
    dex = read_reviewed_probe()
    code = build_worker(dex, cert, result['companion_uid'], result)
    # Ensure no PID replacement occurred between inspection and bounded injection.
    if process_identity() != {name: result[name] for name in ('pid', 'start_ticks', 'boot_id')}:
        raise RuntimeError('Ability restarted before apply; run preflight again')
    server_pid = int(permissions.shell('pidof', 'system_server'))
    attempt = {**result, 'attempted_at': int(time.time()), 'status': 'attempting', 'backup': str(backup.resolve())}
    save(record, attempt)
    try:
        # The existing bounded system-service worker can inspect this protected
        # process. The target remains the pinned ability PID, never system_server.
        proof = permissions.run_bounded_android_worker(code, server_pid, worker_context='system_server')
    except Exception as error:
        proof_path = permissions.STATE / 'last-worker-result.json'
        proof = json.loads(proof_path.read_text()) if proof_path.is_file() else {}
        save(record, {**attempt, 'status': 'worker_failed', 'error': str(error), 'worker_proof': proof})
        raise
    save(record, {**attempt, 'status': 'worker_returned', 'worker_proof': proof})
    answer = json.loads(proof.get('java_result', '{}'))
    if proof.get('context') != 'u:r:system_server:s0' or proof.get('detached') is not True or answer.get('ok') is not True:
        save(record, {**attempt, 'worker_proof': proof, 'status': 'failed', 'detached': proof.get('detached'),
                      'error': answer.get('error') or proof.get('error') or 'No Java result'})
        raise RuntimeError(answer.get('error') or proof.get('error') or 'Ability adapter did not validate')
    if answer.get('pid') != result['pid'] or answer.get('uid') != result['companion_uid'] or not answer.get('signature_verified'):
        raise RuntimeError('Unexpected Amoo adapter identity response')
    current = inspect(apk)
    if not current['installed'] or current['pid'] != result['pid'] or current['start_ticks'] != result['start_ticks']:
        raise RuntimeError('Amoo adapter endpoint could not be verified after apply')
    verified = {**result, **answer, 'status': 'probe_installed', 'backup': str(backup.resolve()), 'execution_enabled': False, 'injected': True, 'detached': True,
                'startup_properties_restored': proof['startup_properties_restored'],
                'system_server_unchanged': proof['system_server_unchanged'],
                'google_permission_baseline_unchanged': True, 'root_bridge_healthy': True,
                'screen_state_changed': False, 'account_data_changed': False, 'worker_proof': proof}
    save(record, verified)
    return verified


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['inspect', 'preflight', 'apply'])
    parser.add_argument('--apk', type=Path, required=True)
    parser.add_argument('--backup', type=Path, help='Verified recovery directory required for apply')
    parser.add_argument('--retry-reviewed', action='store_true', help='Explicitly retry one reviewed failed process injection')
    args = parser.parse_args()
    if args.command == 'inspect': result = inspect(args.apk)
    elif args.command == 'preflight': result = preflight(args.apk)
    else:
        if not args.backup: raise RuntimeError('--backup is required before loading the probe')
        result = apply(args.apk, args.backup, args.retry_reviewed)
    print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    try: main()
    except Exception as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}, ensure_ascii=False))
        sys.exit(1)
