#!/usr/bin/env python3
"""Read-only, exact-firmware evidence for a process-scoped native GUI adapter.

This utility never installs an APK, starts/stops an Android process, sets a
property, or writes a phone file. A backup is not evidence that rollback works:
verify reports immutable-file restoration and process identity separately.
The adapter's own shutdown/task cleanup must also be verified by its controller.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
from datetime import datetime, timezone

SERIAL = os.environ.get('STEPX_DEVICE_SERIAL') or os.environ.get('STEPX_SERIAL') or ''
FINGERPRINT = 'StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys'
PACKAGE = 'com.stepos.ability'
APK = '/system/priv-app/ability/ability.apk'
APK_SHA256 = 'c9f56f3078df96d8d726422b1c622a1c19fe8c7dec8cadd8537d9a6ecf513ac4'
PINNED_FILES = {
    APK: APK_SHA256,
    '/apex/com.android.runtime/lib64/bionic/libc.so':
        '76a754389a8242328cec7c1fed868289a27eb843d49175e1e0ada83f9b60f6da',
    '/apex/com.android.art/lib64/libart.so':
        '5d6a675fc9931595266623b7b682ec5c513128739f8194b64db289b3fddb0836',
}
STARTUP_PROPERTIES = ('persist.openclaw.zygote.path', 'sys.openclaw.env.start')
SCHEMA = 'stepx.native-gui.backup.v1'


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def private_json(path, value):
    path = Path(path)
    temporary = path.with_name(path.name + '.tmp')
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, 'w', encoding='utf-8') as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2, sort_keys=True)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


class Adb:
    def __init__(self, executable, transport):
        # Never let a default/first attached device become the target.
        require(bool(transport), 'An explicit ADB transport is required')
        self.command = [executable, '-s', transport]
        self.transport = transport

    def run(self, *arguments, timeout=30):
        result = subprocess.run(self.command + list(arguments), capture_output=True,
                                timeout=timeout, check=False)
        require(result.returncode == 0,
                'Read-only ADB command failed: ' + result.stderr.decode('utf-8', 'replace')[:600])
        return result.stdout.decode('utf-8', 'strict').strip()

    def shell(self, *arguments):
        return self.run('shell', shlex.join(arguments))

    def pull(self, remote, local):
        require(remote == APK, 'Only the pinned original APK may be pulled')
        self.run('pull', remote, str(local), timeout=90)


def read_identity(adb):
    values = {key: adb.shell('getprop', key) for key in
              ('ro.serialno', 'ro.build.fingerprint', 'ro.build.version.sdk', 'ro.product.cpu.abi')}
    require(values['ro.serialno'] == SERIAL, 'Physical device serial does not match authorization')
    require(values['ro.build.fingerprint'] == FINGERPRINT, 'Firmware fingerprint changed')
    require(values['ro.build.version.sdk'] == '36', 'Android SDK changed')
    require(values['ro.product.cpu.abi'] == 'arm64-v8a', 'Native ABI changed')
    values['selinux'] = adb.shell('getenforce')
    require(values['selinux'] == 'Enforcing', 'SELinux must remain Enforcing')
    return values


def file_evidence(adb, path):
    require(path in PINNED_FILES, 'Unexpected file outside audited patch scope')
    sha = adb.shell('sha256sum', path).split()[0]
    require(sha == PINNED_FILES[path], 'Pinned original file changed: ' + path)
    fields = adb.shell('stat', '-c', '%u:%g:%a:%s:%Y', path).split(':')
    require(len(fields) == 5 and all(re.fullmatch(r'[0-9]+', x) for x in fields),
            'Could not read file ownership/mode/size/mtime: ' + path)
    context = adb.shell('ls', '-Zd', path).split()[0]
    require(re.fullmatch(r'[a-z_]+:[a-z_]+:[a-z0-9_]+:s[0-9]+(?:[:.,a-z0-9-]+)?', context),
            'Could not read file SELinux label: ' + path)
    return dict(sha256=sha, uid=int(fields[0]), gid=int(fields[1]), mode=fields[2],
                bytes=int(fields[3]), mtime=int(fields[4]), selinux=context)


def process_evidence(adb, name, boot_id):
    require(name in (PACKAGE, 'system_server'), 'Unexpected native process target')
    # pidof exits 1 if absent; use a shell branch with a fixed allowlisted name.
    pids = adb.shell('sh', '-c', 'pidof ' + shlex.quote(name) + ' || true').split()
    if not pids:
        require(name != 'system_server', 'system_server is absent')
        return dict(present=False, name=name, boot_id=boot_id)
    require(len(pids) == 1 and pids[0].isdigit() and int(pids[0]) > 1,
            'Expected exactly one native process: ' + name)
    pid = int(pids[0])
    base = '/proc/' + str(pid)
    command = adb.shell('cat', base + '/cmdline').rstrip('\x00')
    require(command == name, 'Process command line does not match: ' + name)
    stat = adb.shell('cat', base + '/stat')
    require(')' in stat, 'Cannot parse process start time')
    fields = stat.rsplit(')', 1)[1].split()
    require(len(fields) > 19 and fields[19].isdigit(), 'Cannot parse process start time')
    status = adb.shell('cat', base + '/status')
    uid = re.search(r'^Uid:\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*$', status, re.M)
    gid = re.search(r'^Gid:\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*$', status, re.M)
    require(uid is not None and gid is not None, 'Cannot read process credentials')
    context = adb.shell('cat', base + '/attr/current').rstrip('\x00')
    require(context.startswith('u:r:') and ':s0' in context, 'Cannot read process SELinux domain')
    return dict(present=True, name=name, pid=pid, start_ticks=fields[19], boot_id=boot_id,
                uids=[int(x) for x in uid.groups()], gids=[int(x) for x in gid.groups()],
                selinux=context)


def capture(adb):
    identity = read_identity(adb)
    require(adb.shell('pm', 'path', PACKAGE).splitlines() == ['package:' + APK],
            'Ability package location changed or has split/updated APKs')
    files = {path: file_evidence(adb, path) for path in PINNED_FILES}
    properties = {key: adb.shell('getprop', key) for key in STARTUP_PROPERTIES}
    require(not any(properties.values()), 'A root worker is already using startup properties')
    boot_id = adb.shell('cat', '/proc/sys/kernel/random/boot_id')
    require(re.fullmatch(r'[a-f0-9-]{36}', boot_id), 'Cannot read boot identity')
    return dict(identity=identity, files=files, startup_properties=properties,
                ability=process_evidence(adb, PACKAGE, boot_id),
                system_server=process_evidence(adb, 'system_server', boot_id))


def create_backup(adb, directory):
    directory = Path(directory).absolute()
    # Existence is an error; never replace an earlier recovery package.
    directory.mkdir(mode=0o700, parents=False, exist_ok=False)
    try:
        before = capture(adb)
        original = directory / 'ability.original.apk'
        adb.pull(APK, original)
        original.chmod(0o600)
        require(digest(original) == APK_SHA256, 'Pulled APK does not match pinned original SHA-256')
        after = capture(adb)
        require(before == after, 'Phone identity/files/process changed during backup; retry before patching')
        manifest = dict(schema=SCHEMA, captured_utc=datetime.now(timezone.utc).isoformat(),
                        adb_transport=adb.transport, snapshot=before,
                        original_apk='ability.original.apk', original_sha256=APK_SHA256,
                        scope='process-only; no APK/partition replacement',
                        rollback_verified=False)
        private_json(directory / 'manifest.json', manifest)
        return dict(backup=str(directory), original_sha256=APK_SHA256,
                    process_present=before['ability']['present'],
                    immutable_files_verified=True, rollback_verified=False)
    except Exception as error:
        private_json(directory / 'FAILED.json', dict(error=str(error), usable=False))
        raise


def load_backup(directory):
    directory = Path(directory)
    require(not (directory / 'FAILED.json').exists(), 'Incomplete backup cannot authorize a patch')
    manifest = json.loads((directory / 'manifest.json').read_text())
    require(manifest.get('schema') == SCHEMA, 'Unknown backup schema')
    require(manifest.get('original_apk') == 'ability.original.apk', 'Backup APK filename changed')
    require(manifest.get('original_sha256') == APK_SHA256, 'Backup APK hash changed')
    require(digest(directory / 'ability.original.apk') == APK_SHA256, 'Local original APK is corrupt')
    baseline = manifest['snapshot']
    require(baseline['identity'] == {
        'ro.serialno': SERIAL, 'ro.build.fingerprint': FINGERPRINT,
        'ro.build.version.sdk': '36', 'ro.product.cpu.abi': 'arm64-v8a', 'selinux': 'Enforcing'},
        'Backup belongs to a different device/firmware')
    require(set(baseline['files']) == set(PINNED_FILES), 'Backup file scope changed')
    for path, sha in PINNED_FILES.items():
        require(baseline['files'][path]['sha256'] == sha, 'Backup native image hash changed')
    require(baseline['startup_properties'] == {key: '' for key in STARTUP_PROPERTIES},
            'Backup startup baseline is not idle')
    return manifest


def same_process(left, right):
    return (left.get('present') is True and right.get('present') is True
            and all(left.get(key) == right.get(key)
                    for key in ('name', 'pid', 'start_ticks', 'boot_id')))


def verify_backup(adb, directory):
    manifest = load_backup(directory)
    baseline = manifest['snapshot']
    now = capture(adb)
    for field in ('identity', 'files', 'startup_properties'):
        require(now[field] == baseline[field], 'Baseline mismatch after patch: ' + field)
    original_process_alive = same_process(baseline['ability'], now['ability'])
    same_system_server = same_process(baseline['system_server'], now['system_server'])
    return dict(immutable_files_verified=True, startup_properties_restored=True,
                original_process_still_alive=original_process_alive,
                target_process_replaced=(baseline['ability']['present'] and not original_process_alive),
                system_server_unchanged=same_system_server,
                current_process=now['ability'], rollback_verified=False,
                remaining_proof='Controller must verify adapter stopped, owned tasks cleaned up, '
                                'and original GUI behavior restored; file/socket checks alone are insufficient.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adb', default='adb')
    parser.add_argument('--transport', required=True)
    sub = parser.add_subparsers(dest='action', required=True)
    sub.add_parser('backup').add_argument('--output', type=Path, required=True)
    sub.add_parser('verify').add_argument('--backup', type=Path, required=True)
    args = parser.parse_args()
    adb = Adb(args.adb, args.transport)
    result = create_backup(adb, args.output) if args.action == 'backup' else verify_backup(adb, args.backup)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (RuntimeError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(json.dumps(dict(ok=False, error=str(error)), ensure_ascii=False), file=sys.stderr)
        sys.exit(1)
