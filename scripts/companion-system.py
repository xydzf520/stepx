#!/usr/bin/env python3
"""Run only the signed companion's fixed Android pairing Binder service.

Current-device session service: shell UID, no root/native injection, no SELinux
changes, no Google launcher changes. Installation and stop are explicit. Reboot
persistence is intentionally reported false until separately implemented/tested.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('companion_permissions', ROOT / 'scripts/companion-permissions.py')
permissions = importlib.util.module_from_spec(spec); spec.loader.exec_module(permissions)
DEX = ROOT / 'desktop/assets/stepx-amoo-background.dex'
APK = ROOT / 'desktop/assets/stepx-companion.apk'
MAIN = 'stepx.PairingDaemon'
DIRECTORY = '/data/local/tmp/stepx-pairing'
REMOTE_DEX = DIRECTORY + '/service.dex'
REMOTE_LOG = DIRECTORY + '/service.log'
APP_PROCESS = '/system/bin/app_process64'
APP_PROCESS_SHA256 = '5acb9827648f9adba44ae2bb16f81ef006ba5564dd81c2a2efd854dccb140bae'
TOYBOX_SHA256 = '051ef75fe661eb65b0b1b204959717c6834a872a415a7a99991f8d08ccc250aa'
PAIRING_STATUS = '/sdcard/Android/data/com.stepx.companion.app/files/pairing.json'
STATE = Path(os.environ.get('XDG_STATE_HOME', Path.home() / '.local/state')) / 'stepx-companion/system-service'


def save(path, value):
    STATE.mkdir(parents=True, exist_ok=True, mode=0o700); STATE.chmod(0o700)
    temp = path.with_suffix('.new')
    fd = os.open(temp, os.O_CREAT | os.O_TRUNC | os.O_WRONLY, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump(value, stream, indent=2); stream.flush(); os.fsync(stream.fileno())
    os.replace(temp, path)


def record_path():
    return STATE / (hashlib.sha256(permissions.device_key().encode()).hexdigest()[:24] + '.json')


def system_session():
    pid = permissions.shell('pidof', 'system_server').strip()
    if not pid.isdigit() or int(pid) < 2:
        raise RuntimeError('Unable to verify Android system service session')
    stat = permissions.shell('cat', '/proc/' + pid + '/stat')
    start = stat.rsplit(')', 1)[1].split()[19]
    if not start.isdigit():
        raise RuntimeError('Invalid Android system service start time')
    return {'pid': int(pid), 'start_ticks': start}


def preflight(apk):
    companion = permissions.inspect(apk)
    if not companion.get('signature_verified'):
        raise RuntimeError('Install the signed StepX companion before preparing pairing')
    permissions.guard(native=True)
    payload = DEX.read_bytes()
    if not payload.startswith(b'dex\n') or len(payload) > 256 * 1024 or b'Lstepx/PairingDaemon;' not in payload:
        raise RuntimeError('Build the fixed companion pairing DEX first')
    for path, expected in ((APP_PROCESS, APP_PROCESS_SHA256), ('/system/bin/toybox', TOYBOX_SHA256)):
        if permissions.shell('sha256sum', path).split()[0] != expected:
            raise RuntimeError('Unreviewed Android service executable: ' + path)
    return {'service': 'stepx-pairing-v1', 'package': permissions.PACKAGE,
            'signer': permissions.signer(apk), 'companion_uid': companion['uid'],
            'dex_sha256': hashlib.sha256(payload).hexdigest(),
            'serial': permissions.device_key(), 'fingerprint': permissions.CONFIG['fingerprint'],
            'boot_id': permissions.shell('cat', '/proc/sys/kernel/random/boot_id'),
            'system_session': system_session(),
            'scope': 'current_device_session', 'reboot_verified': False,
            'native_injection': False, 'google_launcher_changed': False}


def process(pid):
    if type(pid) is not int or pid < 2:
        raise RuntimeError('Invalid service PID')
    root = '/proc/' + str(pid)
    try:
        cmdline = permissions.shell('cat', root + '/cmdline')
        stat = permissions.shell('cat', root + '/stat')
        status = permissions.shell('cat', root + '/status')
        context = permissions.shell('cat', root + '/attr/current').strip('\0\n')
        executable = permissions.shell('readlink', root + '/exe')
    except Exception:
        return None
    uid = re.search(r'^Uid:\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*$', status, re.M)
    if not uid or uid.groups() != ('2000',) * 4 or context != 'u:r:shell:s0' or executable != APP_PROCESS:
        raise RuntimeError('Pairing process identity does not match the fixed shell service')
    argv = [part for part in cmdline.split('\0') if part]
    start = stat.rsplit(')', 1)[1].split()[19]
    if not start.isdigit():
        raise RuntimeError('Invalid pairing process start time')
    return {'pid': pid, 'start_ticks': start, 'argv': argv, 'uid': 2000,
            'context': context, 'executable': executable}


def validate_launch_identity(found, identity):
    expected = [APP_PROCESS, '/system/bin', MAIN, identity['signer'], str(identity['companion_uid'])]
    # Some Android versions replace argv with the Java main's process name.
    # The PID still comes from our fixed shell launch, then its exact observed
    # argv/start time is saved and required for every later stop/reuse.
    if found is None or found['argv'] not in (expected, [MAIN]):
        raise RuntimeError('The started process did not match the fixed Java entry point')


def candidates():
    output = permissions.shell('ps', '-A', '-o', 'PID,ARGS')
    return [int(parts[0]) for line in output.splitlines() if len(parts := line.split()) > 1
            and parts[0].isdigit() and MAIN in parts[1:]]


def saved_service(identity):
    path = record_path()
    prior = json.loads(path.read_text()) if path.is_file() else None
    active = candidates()
    if not prior or prior.get('boot_id') != identity['boot_id']:
        if active:
            raise RuntimeError('A pairing process exists without this installation record; inspect before replacing it')
        return None
    for key in ('service', 'package', 'serial', 'fingerprint'):
        if prior.get(key) != identity.get(key):
            raise RuntimeError('Pairing service installation record does not match this device')
    saved = prior.get('process')
    if not saved:
        if active:
            raise RuntimeError('A previous service start has no verified identity; inspect before retrying')
        return None
    current = process(saved['pid'])
    if current is None:
        if active:
            raise RuntimeError('A different pairing process is already running')
        return None
    if current != saved or active != [saved['pid']]:
        raise RuntimeError('Pairing process changed; refusing to adopt or stop it')
    return prior


def status(apk):
    identity = preflight(apk); prior = saved_service(identity)
    return {**identity, 'status': 'running' if prior else 'not_running',
            'running': bool(prior), 'process': prior.get('process') if prior else None,
            'artifact_current': bool(prior and prior['dex_sha256'] == identity['dex_sha256'] and
                                     prior['signer'] == identity['signer'] and
                                     prior['companion_uid'] == identity['companion_uid'] and
                                     prior.get('system_session') == identity['system_session'])}


def start(apk):
    identity = preflight(apk); prior = saved_service(identity)
    if prior:
        if any(prior.get(k) != identity.get(k) for k in ('dex_sha256', 'signer', 'companion_uid', 'system_session')):
            raise RuntimeError('Stop the verified old pairing service before installing the new artifact')
        return {**identity, 'status': 'already_running', 'process': prior['process'], 'running': True}
    directory = shlex.quote(DIRECTORY)
    permissions.shell('sh', '-c', f'if [ -L {directory} ]; then exit 31; fi; '
        f'if [ -e {directory} ]; then [ "$(stat -c %u:%a {directory})" = "2000:700" ] || exit 32; '
        f'else umask 077; mkdir {directory}; fi')
    temporary = REMOTE_DEX + '.new'
    # No symlink following or arbitrary destination; this shell-owned private
    # directory contains only this service's artifact and diagnostics.
    permissions.shell('sh', '-c', 'test ! -L ' + shlex.quote(temporary) +
                      ' && test ! -L ' + shlex.quote(REMOTE_LOG) +
                      ' && rm -f ' + shlex.quote(temporary))
    permissions.run(permissions.ADB + ['push', str(DEX), temporary])
    if permissions.shell('sha256sum', temporary).split()[0] != identity['dex_sha256']:
        raise RuntimeError('Pairing DEX transfer did not pass SHA-256 read-back')
    permissions.shell('chmod', '400', temporary)
    permissions.shell('mv', '-f', temporary, REMOTE_DEX)
    if permissions.shell('stat', '-c', '%u:%a', REMOTE_DEX) != '2000:400':
        raise RuntimeError('Pairing DEX must be shell-owned and read-only before loading')
    started = int(permissions.shell('date', '+%s')) * 1000
    save(record_path(), {**identity, 'status': 'starting', 'started_at_ms': started})
    launch = ('umask 077; CLASSPATH=' + shlex.quote(REMOTE_DEX) + ' /system/bin/nohup ' +
              shlex.join([APP_PROCESS, '/system/bin', MAIN, identity['signer'], str(identity['companion_uid'])]) +
              ' >' + shlex.quote(REMOTE_LOG) + ' 2>&1 </dev/null & stepx_pairing_pid=$!; echo "$stepx_pairing_pid"')
    pid_text = permissions.shell('sh', '-c', launch)
    if not re.fullmatch(r'[1-9][0-9]*', pid_text):
        raise RuntimeError('Android did not return the pairing service PID')
    pid = int(pid_text)
    deadline = time.monotonic() + 25
    found = None
    while time.monotonic() < deadline:
        try:
            found = process(pid)
        except RuntimeError:
            # nohup may not have exec'd app_process yet. No process is adopted
            # or signalled until the final strict UID/domain/executable/argv check.
            found = None
            time.sleep(.1); continue
        if found:
            try: validate_launch_identity(found, identity)
            except RuntimeError:
                time.sleep(.1); continue
            break
        time.sleep(.1)
    validate_launch_identity(found, identity)
    save(record_path(), {**identity, 'status': 'started', 'started_at_ms': started, 'process': found})
    while time.monotonic() < deadline:
        if process(pid) != found:
            raise RuntimeError('Pairing service stopped before the companion accepted it; inspect service.log')
        try:
            readback = json.loads(permissions.shell('cat', PAIRING_STATUS))
            if (readback.get('pairingBridgeReady') is True and readback.get('pairingSupported') is True and
                    readback.get('updatedAtMs', 0) >= started):
                result = {**identity, 'status': 'ready', 'running': True, 'process': found,
                          'companion_accepted': True, 'started_at_ms': started}
                save(record_path(), result); return result
        except Exception:
            # A newly installed app may not have created its external status
            # file yet; adb reports a command failure, not FileNotFoundError.
            # Only this read is tolerated. Process identity checks stay strict.
            pass
        time.sleep(.3)
    raise RuntimeError('Pairing process started, but the signed companion did not confirm Binder readiness; inspect service.log')


def stop(apk):
    identity = preflight(apk); prior = saved_service(identity)
    if prior is None:
        return {**identity, 'status': 'not_running', 'running': False}
    found = prior['process']
    # Recheck PID/start time/argv/UID/domain immediately before the sole signal.
    if process(found['pid']) != found:
        raise RuntimeError('Pairing service changed before stop; nothing was signalled')
    permissions.shell('kill', '-TERM', str(found['pid']))
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        current = process(found['pid'])
        if current is None:
            result = {**identity, 'status': 'stopped', 'running': False}
            save(record_path(), result); return result
        if current != found:
            raise RuntimeError('PID was replaced after stop; no further signal sent')
        time.sleep(.1)
    raise RuntimeError('Pairing service did not exit after TERM; no forced kill or broader process action performed')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['preflight', 'status', 'start', 'stop'])
    parser.add_argument('--apk', type=Path, default=APK)
    args = parser.parse_args()
    STATE.mkdir(parents=True, exist_ok=True, mode=0o700); STATE.chmod(0o700)
    with permissions.file_lock(STATE / 'operation.lock'):
        result = {'preflight': preflight, 'status': status, 'start': start, 'stop': stop}[args.command](args.apk)
        print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    try: main()
    except Exception as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}, ensure_ascii=False)); sys.exit(1)
