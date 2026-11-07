#!/usr/bin/env python3
"""Inspect the original GUI class loader in a short-lived, UID-1000 service.

Research only: no GUI initialization, account access, task or persistent service.
Requires the fixed firmware, existing authorized root transport and a verified
backup. The only phone file created is a unique, read-only probe DEX in code_cache.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import uuid

ROOT = Path(__file__).resolve().parents[1]
DEX_SHA256 = 'c413bad64a74c13fa091317cbc371f851049d826d6c5e7251134dcdc219464a6'
DEX = ROOT / 'desktop/assets/stepx-native-gui-isolated-probe.dex'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def run(backup):
    permissions = module('permissions', ROOT / 'scripts/companion-permissions.py')
    recovery = module('recovery', ROOT / 'scripts/gui-native-backup.py')
    if permissions.ADB[-1] != permissions.AUTHORIZED_SERIAL:
        raise RuntimeError('Research launch requires the explicitly authorized USB serial')
    adb = recovery.Adb(permissions.ADB[0], permissions.ADB[-1])
    before = recovery.verify_backup(adb, backup)
    if not before['system_server_unchanged'] or not before['original_process_still_alive']:
        raise RuntimeError('Original process identity changed; create a fresh backup')
    permissions.guard(native=True)
    payload = DEX.read_bytes()
    if hashlib.sha256(payload).hexdigest() != DEX_SHA256:
        raise RuntimeError('Only the reviewed, initialization-free probe DEX is allowed')
    name = 'stepx-native-gui-probe-' + uuid.uuid4().hex + '.dex'
    remote = '/data/user/0/com.stepos.ability/code_cache/' + name
    runtime_env = dict(CLASSPATH=remote, ANDROID_ROOT='/system', ANDROID_DATA='/data',
                       PATH='/system/bin', ANDROID_ART_ROOT='/apex/com.android.art',
                       ANDROID_I18N_ROOT='/apex/com.android.i18n', ANDROID_TZDATA_ROOT='/apex/com.android.tzdata')
    for key in ('BOOTCLASSPATH', 'DEX2OATBOOTCLASSPATH', 'SYSTEMSERVERCLASSPATH'):
        value = permissions.shell('printenv', key)
        if not value or not all(part.startswith('/') for part in value.split(':')):
            raise RuntimeError('Android runtime classpath is unavailable: ' + key)
        runtime_env[key] = value
    code = 'PROBE_ENV=' + repr(runtime_env) + '\nPROBE_DEX=' + repr(base64.b64encode(payload).decode()) + '\n' + r'''
out['probe_only']=True
probe_path=PROBE_ENV['CLASSPATH']
probe_written=False
try:
 assert hashlib.sha256(open('/system/priv-app/ability/ability.apk','rb').read()).hexdigest() in ('c9f56f3078df96d8d726422b1c622a1c19fe8c7dec8cadd8537d9a6ecf513ac4','a7dbeaa721c13836bbb3990026af6c3d66bf164afc4c73f70bd453aa119fbca8')
 assert os.stat(os.path.dirname(probe_path)).st_uid==1000
 data=base64.b64decode(PROBE_DEX)
 assert hashlib.sha256(data).hexdigest()=='c413bad64a74c13fa091317cbc371f851049d826d6c5e7251134dcdc219464a6'
 fd=os.open(probe_path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o400)
 probe_written=True
 try:
  written=0
  while written<len(data):written+=os.write(fd,data[written:])
  os.fsync(fd)
 finally:os.close(fd)
 def child_identity():
  os.setresgid(1000,1000,1000);os.setresuid(1000,1000,1000)
 child=subprocess.Popen(['/system/bin/app_process64','/system/bin','stepx.NativeGuiIsolatedProbe'],env=PROBE_ENV,preexec_fn=child_identity,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
 out['isolated_pid']=child.pid
 try:stdout,stderr=child.communicate(timeout=12)
 except subprocess.TimeoutExpired:
  child.kill();stdout,stderr=child.communicate(timeout=2);out['timed_out']=True
 out['child_exit']=child.returncode
 out['isolated_result']=stdout[:16000]
 out['launch_error']=stderr[:1000]
except Exception as error:out['isolated_error']=str(error)
finally:
 if probe_written:
  # Delete only our O_EXCL-created file, never original application data.
  os.unlink(probe_path);out['probe_file_removed']=not os.path.exists(probe_path)
'''
    proof = permissions.run_bounded_android_worker(code, int(permissions.shell('pidof', 'system_server')),
                                                   worker_context='agentbus')
    after = recovery.verify_backup(adb, backup)
    proof['probe_remote_path'] = remote
    proof['probe_dex_sha256'] = DEX_SHA256
    proof['original_files_verified'] = after['immutable_files_verified']
    proof['original_ability_unchanged'] = after['original_process_still_alive']
    target = Path(backup) / (name + '.proof.json')
    recovery.private_json(target, proof)
    response = json.loads(proof.get('isolated_result', '{}'))
    if (proof.get('child_exit') != 0 or proof.get('probe_file_removed') is not True
            or not after['original_process_still_alive'] or not after['system_server_unchanged']
            or response.get('ok') is not True or response.get('uid') != 1000
            or response.get('mode') != 'isolated_classloader_probe'
            or response.get('tasks_started') != 0 or response.get('account_accessed') is not False
            or response.get('execution_verified') is not False):
        raise RuntimeError('Read-only probe failed validation; inspect ' + str(target))
    return {**proof, 'proof_file': str(target.resolve()), 'execution_verified': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--backup', type=Path, required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(run(args.backup), ensure_ascii=False))
    except Exception as error:
        print(json.dumps({'ok': False, 'error': str(error)}, ensure_ascii=False))
        raise SystemExit(1)


if __name__ == '__main__':
    main()
