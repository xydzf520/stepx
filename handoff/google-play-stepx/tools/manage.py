#!/usr/bin/env python3
"""Explicit workflow steps; invoking --help or preflight never installs anything."""
import argparse
import base64
import json
import re
import shutil
import subprocess
import sys
import time
import urllib.request
import zipfile
from common import ADB, CONFIG, ROOT, STATE, DOWNLOADS, guard, init_state, private_write, root_json, run, save_json, sdk_tool, sdk_command, sha256, shell

PACKAGES = ['com.google.android.gsf', 'com.google.android.gms', 'com.android.vending']


def preflight():
    guard(native=True)
    result = {
        'fingerprint': shell('getprop', 'ro.build.fingerprint'),
        'boot_completed': shell('getprop', 'sys.boot_completed'),
        'user_unlocked': 'RUNNING_UNLOCKED' in shell('dumpsys', 'user'),
        'avb': shell('getprop', 'ro.boot.verifiedbootstate'),
        'bootloader_locked': shell('getprop', 'ro.boot.flash.locked'),
        'selinux': shell('getenforce'),
        'google_packages': {p: bool(shell('pm', 'list', 'packages', '--user', '0', p)) for p in PACKAGES},
    }
    print(json.dumps(result, indent=2))
    return result


def fetch(archive=None):
    folder = DOWNLOADS
    folder.mkdir(parents=True, exist_ok=True)
    meta = CONFIG['archive']
    if archive:
        path = __import__('pathlib').Path(archive).resolve()
    else:
        path = folder / meta['name']
        if not path.exists():
            temp = path.with_suffix('.partial')
            with urllib.request.urlopen(meta['url'], timeout=60) as source, open(temp, 'wb') as target:
                shutil.copyfileobj(source, target)
            assert sha256(temp) == meta['sha256'], 'Archive digest mismatch'
            temp.replace(path)
    assert sha256(path) == meta['sha256'], 'Archive digest mismatch'
    with zipfile.ZipFile(path) as z:
        for entry in CONFIG['apks']:
            dest = folder / entry['filename']
            dest.write_bytes(z.read(entry['zip_path']))
    check_apks()


def check_apks():
    for entry in CONFIG['apks']:
        path = DOWNLOADS / entry['filename']
        assert sha256(path) == entry['sha256'], 'APK digest mismatch: ' + entry['package']
        result = run(sdk_command('apksigner') + ['verify', '--print-certs', str(path)])
        assert 'Signer #1 certificate SHA-256 digest: ' + entry['signer_sha256'] in result.stdout
        print(entry['package'] + ': APK digest and signer OK')


def prepare():
    current = preflight()
    assert current['user_unlocked'], 'Unlock the phone first'
    assert not any(current['google_packages'].values()), 'Google packages already exist; use maintenance workflow'
    all_packages = shell('pm', 'list', 'packages', '-u')
    assert all('package:' + p + '\n' not in all_packages + '\n' for p in PACKAGES), 'Prior Google package state exists'
    assert re.findall(r'UserInfo\{(\d+):', shell('pm', 'list', 'users')) == ['0'], 'Only user 0 was validated'
    init_state()
    assert not (STATE / 'baseline.json').exists(), 'Do not overwrite an existing baseline'
    baseline = {'serial': ADB[-1], 'fingerprint': CONFIG['fingerprint'],
                'packages': [x.removeprefix('package:') for x in shell('pm', 'list', 'packages').splitlines()],
                'google_core': shell('settings', 'get', 'global', 'cn_gms_core'),
                'autofill': shell('settings', 'get', 'secure', 'autofill_service'),
                'startup_path': shell('getprop', 'persist.openclaw.zygote.path'),
                'start_trigger': shell('getprop', 'sys.openclaw.env.start')}
    assert baseline['startup_path'] == baseline['start_trigger'] == '', 'Non-default startup properties'
    lines = shell('ps', '-A', '-o', 'PID,PPID,NAME').splitlines()
    parent = next(int(x.split()[0]) for x in lines if x.split()[-1] == 'openclaw_env' and x.split()[1] == '1')
    code = '''import os,json,hashlib,base64
h=HOST
assert not os.path.exists('/usr/local/lib/stepx-gms'), 'Adapter already exists'
assert not os.path.exists('/usr/local/bin/ubuntu_zygote.stepx-stock-20261021'), 'Stock backup already exists'
p='/usr/local/bin/ubuntu_zygote';data=open(p,'rb').read()
r={'stock':base64.b64encode(data).decode(),'stock_sha256':hashlib.sha256(data).hexdigest(),'files':{}}
for path in ['/data/system/packages.xml','/data/misc_de/0/apexdata/com.android.permission/access.abx']:
 r['files'][path]=base64.b64encode(open(h+path,'rb').read()).decode()
print(json.dumps(r))
'''.replace('HOST', repr('/proc/' + str(parent) + '/root'))
    proof = root_json(code)
    assert proof['stock_sha256'] == CONFIG['stock_sha256'], 'Unexpected factory launcher'
    private_write(STATE / 'ubuntu_zygote.stock', base64.b64decode(proof['stock']))
    for path, encoded in proof['files'].items():
        private_write(STATE / ('before-' + path.rsplit('/', 1)[-1]), base64.b64decode(encoded))
    save_json('baseline.json', baseline)
    print('Original launcher and package/permission metadata saved privately. No Google APK installed yet.')


def install_apks():
    guard(native=True)
    baseline = json.loads((STATE / 'baseline.json').read_text())
    assert baseline['fingerprint'] == CONFIG['fingerprint']
    assert baseline.get('serial', CONFIG['serial']) == ADB[-1], 'Wrong device baseline'
    assert all(p not in baseline['packages'] for p in PACKAGES)
    assert all(not shell('pm', 'list', 'packages', '--user', '0', p) for p in PACKAGES), 'Do not overwrite an existing installation'
    check_apks()
    for package in PACKAGES:
        meta = next(x for x in CONFIG['apks'] if x['package'] == package)
        # These provider packages must be discoverable by clients that assume a
        # preinstalled GMS stack. This does not mark them SYSTEM/PRIVILEGED.
        result = run(ADB + ['install', '--user', '0', '--force-queryable', str(DOWNLOADS / meta['filename'])], timeout=180)
        assert 'Success' in result.stdout, 'Installation not confirmed: ' + package
        print('Installed ' + package, flush=True)
    shell('settings', 'put', 'global', 'cn_gms_core', '1')
    for package in PACKAGES:
        shell('pm', 'grant', package, 'android.permission.INTERACT_ACROSS_USERS')
    shell('pm', 'grant', 'com.google.android.gms', 'android.permission.WRITE_SECURE_SETTINGS')
    for package in ['com.google.android.gms', 'com.android.vending']:
        shell('cmd', 'appops', 'set', package, 'SYSTEM_EXEMPT_FROM_POWER_RESTRICTIONS', 'allow')
    print('APKs installed. Do not log in before adapter verification and an unlogged reboot test.')


def current_permission_checks(checks):
    """An old boot receipt is not evidence after a Play services update."""
    dumps = {package: shell('dumpsys', 'package', package) for package in PACKAGES}
    missing = []
    for check in checks:
        match = re.fullmatch(r'check (\S+) (\S+)=0', check)
        if not match or match[1] not in dumps:
            missing.append('unrecognized_receipt')
            continue
        permission = match[2]
        if not re.search(r'^\s*' + re.escape(permission) + r': granted=true(?:\s|,|$)', dumps[match[1]], re.M):
            missing.append(match[1] + ':' + permission)
    return missing


def status():
    guard()
    init_state()
    result = root_json("import os,json; p='/var/lib/stepx-gms/last.json'; print(json.dumps({'boot_id':open('/proc/sys/kernel/random/boot_id').read().strip(),'adapter':json.load(open(p)) if os.path.exists(p) else None}))")
    adapter = result['adapter']
    save_json('adapter-status.json', result)
    if not adapter:
        print('No adapter status. Unlock phone and verify StepClaw is running.'); return
    checks = [x for x in adapter.get('java_result', '').splitlines() if x.startswith('check ')]
    summary = {k: adapter.get(k) for k in ['status', 'target_pid', 'unix_time', 'detached', 'error']}
    summary.update(current_boot_matches=adapter.get('identity') == result['boot_id'],
                   current_pid_matches=str(adapter.get('target_pid')) == shell('pidof', 'system_server'),
                   permission_checks=len(checks), all_checked_granted=bool(checks) and all(x.endswith('=0') for x in checks))
    missing = current_permission_checks(checks)
    summary.update(current_permissions_granted=bool(checks) and not missing, missing_permissions=missing)
    print(json.dumps(summary, indent=2))
    assert summary['status'] == 'success' and summary['detached']
    assert summary['current_boot_matches'] and summary['current_pid_matches']
    assert len(checks) >= 120 and summary['all_checked_granted']
    assert summary['current_permissions_granted'], 'Current Google permissions differ from the saved receipt'
    return summary


def repair_runtime():
    """Re-run the existing pinned adapter after a package update; keep boot hooks intact."""
    guard(native=True)
    assert 'RUNNING_UNLOCKED' in shell('dumpsys', 'user'), 'Unlock the phone first'
    assert shell('getprop', 'persist.openclaw.zygote.path') == '', 'Another startup operation is active'
    assert shell('getprop', 'sys.openclaw.env.start') == '', 'Another startup operation is active'
    expected = {name: sha256(ROOT / 'module' / name) for name in ('boot.py', 'inject.py')}
    code = '''import os,json,hashlib,fcntl,time
base='/usr/local/lib/stepx-gms';state='/var/lib/stepx-gms'
def sha(path):return hashlib.sha256(open(path,'rb').read()).hexdigest()
boot=base+'/boot.py'
if sha(boot)!=EXPECTED['boot.py']:
 c=json.load(open('/usr/local/lib/stepx-companion/config.json'))
 assert sha(boot)==c['chain_sha256'], 'Unknown boot hook'
 boot='/usr/local/lib/stepx-companion/previous-boot.py'
assert sha(boot)==EXPECTED['boot.py'] and sha(base+'/inject.py')==EXPECTED['inject.py'], 'Adapter changed'
lock=os.open(state+'/lock',os.O_RDWR|os.O_CREAT,0o600)
fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
last=state+'/last.json'
if os.path.exists(last):os.rename(last,state+'/maintenance-'+str(time.time_ns())+'.json')
os.close(lock)
print(json.dumps({'prepared':True}))
'''.replace('EXPECTED', repr(expected))
    proof = root_json(code)
    assert proof.get('prepared'), 'Adapter repair did not prepare'
    def trigger(value):
        assert value in ('', '1')
        parent = next(int(x.split()[0]) for x in shell('ps', '-A', '-o', 'PID,PPID,NAME').splitlines()
                      if x.split()[-1] == 'openclaw_env' and x.split()[1] == '1')
        def observed():
            actual = shell('getprop', 'sys.openclaw.env.start')
            if actual == value:
                return True
            # Factory startup consumes the trigger immediately on this firmware.
            return value == '1' and actual == '' and any(
                x.split()[-1] == 'openclaw_env' and x.split()[1] == '1'
                and int(x.split()[0]) != parent
                for x in shell('ps', '-A', '-o', 'PID,PPID,NAME').splitlines()[1:])
        request = '''import socket,struct,json
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);s.settimeout(3)
s.connect(PATH);n=b'sys.openclaw.env.start';v=VALUE.encode()
s.sendall(struct.pack('<II',0x20001,len(n))+n+struct.pack('<I',len(v))+v)
r=s.recv(4);s.close();assert len(r)==4 and struct.unpack('<I',r)[0]==0
print(json.dumps({'requested':True}))
'''.replace('PATH', repr('/proc/' + str(parent) + '/root/dev/socket/property_service')).replace('VALUE', repr(value))
        try:
            root_json(request)
        except (RuntimeError, subprocess.SubprocessError):
            # The start trigger terminates its own gateway; require native read-back.
            if not observed():
                raise
        assert observed(), 'Factory trigger read-back failed'

    # The factory init service supplies its reviewed startup context. Running
    # the same code inside an ordinary Ubuntu child cannot restore permissions.
    try:
        trigger('1')
        deadline = time.monotonic() + 55
        while time.monotonic() < deadline:
            time.sleep(3)
            try:
                value = root_json("import json,os;p='/var/lib/stepx-gms/last.json';print(json.dumps(json.load(open(p)) if os.path.exists(p) else {}))")
            except (RuntimeError, subprocess.SubprocessError):
                continue  # Gateway is restarting with the factory service.
            if not value or value.get('status') == 'started':
                continue
            assert value.get('status') == 'success' and value.get('detached'), 'Adapter repair failed; original hooks preserved'
            summary = status()
            assert summary['current_permissions_granted'], 'Current Google permissions did not recover'
            return
        raise RuntimeError('Adapter repair timed out; inspect its saved result before retrying')
    finally:
        if shell('getprop', 'sys.openclaw.env.start') == '1':
            trigger('')


def verify():
    guard()
    account_dump = shell('dumpsys', 'account')
    result = {'device_epoch': int(shell('date', '+%s')),
              'google_account_count': len(re.findall(r'Account \{[^\n]*type=com\.google\}', account_dump)),
              'system_server_pid': shell('pidof', 'system_server'), 'selinux': shell('getenforce'),
              'user_unlocked': 'RUNNING_UNLOCKED' in shell('dumpsys', 'user'),
              'google_core': shell('settings', 'get', 'global', 'cn_gms_core'),
              'versions': {}, 'test_apps': {}, 'fatal_events': []}
    for package in PACKAGES:
        data = shell('dumpsys', 'package', package)
        match = re.search(r'versionName=([^\n]+)', data)
        result['versions'][package] = match.group(1) if match else None
    for package in ['com.google.android.calculator', 'com.google.android.deskclock']:
        data = shell('dumpsys', 'package', package)
        result['test_apps'][package] = {key: (m.group(1) if (m := re.search(r'\b' + key + r'=([^\s]+)', data)) else None)
                                       for key in ['versionName', 'installerPackageName', 'initiatingPackageName']}
    logs = run(ADB + ['logcat', '-b', 'crash', '-d', '-v', 'epoch']).stdout
    for block in re.split(r'(?=\s*\d{10}\.\d+\s+\d+\s+\d+ E AndroidRuntime: FATAL EXCEPTION)', logs):
        process = re.search(r'Process: (com\.google\.android\.(?:gms|gsf)[^,\s]*|com\.android\.vending[^,\s]*),', block)
        stamp = re.search(r'(\d{10}\.\d+)', block)
        if 'FATAL EXCEPTION' in block and process and stamp:
            result['fatal_events'].append({'device_epoch': float(stamp.group(1)), 'process': process.group(1)})
    # Never persist the account dump, account names, or raw logs.
    save_json('verification-' + str(result['device_epoch']) + '.json', result)
    print(json.dumps(result, indent=2))


def adopt(source):
    from pathlib import Path
    guard()
    init_state()
    assert not (STATE / 'baseline.json').exists(), 'Existing baseline must not be overwritten'
    origin = Path(source).resolve()
    baseline = json.loads((origin / 'baseline.json').read_text())
    install = json.loads((origin / 'boot-install.json').read_text())
    assert baseline['fingerprint'] == CONFIG['fingerprint']
    assert baseline.get('serial', CONFIG['serial']) == ADB[-1], 'Backup belongs to a different device'
    assert sha256(origin / 'ubuntu_zygote.stock') == CONFIG['stock_sha256'] == install['original_sha256']
    current = root_json("import hashlib,json;print(json.dumps({'sha':hashlib.sha256(open('/usr/local/bin/ubuntu_zygote','rb').read()).hexdigest()}))")
    assert current['sha'] == install['launcher_sha256'], 'Installed launcher differs from recovery metadata'
    for name in ['baseline.json', 'boot-install.json', 'ubuntu_zygote.stock']:
        private_write(STATE / name, (origin / name).read_bytes())
    print('Existing private rollback metadata imported; nothing changed on phone.')


def recover_metadata():
    """Recover a lost install response only after exact read-back of a complete install."""
    guard(native=True)
    baseline = json.loads((STATE / 'baseline.json').read_text())
    assert baseline['fingerprint'] == CONFIG['fingerprint']
    assert baseline.get('serial', CONFIG['serial']) == ADB[-1]
    assert sha256(STATE / 'ubuntu_zygote.stock') == CONFIG['stock_sha256']
    assert not (STATE / 'boot-install.json').exists(), 'Existing recovery metadata must be reviewed, not overwritten'
    expected = {p.name: sha256(p) for p in (ROOT / 'module').glob('*.py')}
    code = '''import json,hashlib
def digest(p):return hashlib.sha256(open(p,'rb').read()).hexdigest()
base='/usr/local/lib/stepx-gms'
manifest=json.load(open(base+'/manifest.json'))
actual={n:digest('/usr/local/bin/ubuntu_zygote' if n=='launcher.py' else base+'/'+n) for n in ['launcher.py','boot.py','inject.py']}
print(json.dumps({'manifest':manifest,'actual':actual,'stock':digest('/usr/local/bin/ubuntu_zygote.stepx-stock-20261021')}))
'''
    proof = root_json(code)
    assert proof['manifest'] == proof['actual'] == expected, 'Partial or different install; manual review required'
    assert proof['stock'] == CONFIG['stock_sha256']
    save_json('boot-install.json', {'installed': True, 'original_sha256': proof['stock'],
                                 'launcher_sha256': expected['launcher.py'], 'manifest': expected,
                                 'recovered_from_exact_readback': True})
    print('Local rollback metadata recovered from verified device files; phone unchanged.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    for name in ['preflight', 'check-apks', 'prepare', 'install-apks', 'status', 'verify', 'certification', 'recover-adapter-metadata', 'repair-runtime']:
        commands.add_parser(name)
    commands.add_parser('fetch').add_argument('--archive')
    commands.add_parser('adopt-backup').add_argument('directory')
    args = parser.parse_args()
    if args.command == 'certification':
        from certification import inspect
        print(json.dumps(inspect(), ensure_ascii=False, indent=2))
        return
    if args.command == 'fetch': fetch(args.archive)
    elif args.command == 'adopt-backup': adopt(args.directory)
    else: {'preflight': preflight, 'check-apks': check_apks, 'prepare': prepare,
           'install-apks': install_apks, 'status': status, 'verify': verify,
           'recover-adapter-metadata': recover_metadata, 'repair-runtime': repair_runtime}[args.command]()


if __name__ == '__main__':
    main()
