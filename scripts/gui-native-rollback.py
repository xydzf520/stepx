#!/usr/bin/env python3
"""Stop only the recorded inspect-only native GUI adapter, then verify evidence.

Uses the signed companion's fixed DUMP-protected provider. It never kills a
process or modifies an APK, model, permission, boot configuration or partition.
Closing a listener leaves the in-memory DEX allocated until Ability restarts.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


def load_module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


recovery = load_module('native_gui_recovery', 'gui-native-backup.py')
patcher = load_module('native_gui_patcher', 'gui-native-patch.py')
PROVIDER = 'content://com.stepx.companion.app.native_gui_probe'


def provider(adb, operation):
    recovery.require(operation in ('status', 'shutdown'), 'Unexpected rollback operation')
    text = adb.shell('content', 'call', '--uri', PROVIDER, '--method', operation)
    marker = 'result='
    offset = text.find(marker)
    recovery.require(offset >= 0, 'Companion provider did not return adapter JSON: ' + text[:500])
    answer, _ = json.JSONDecoder().raw_decode(text[offset + len(marker):].lstrip())
    recovery.require(isinstance(answer, dict) and answer.get('version') == 1,
                     'Unexpected native GUI response schema')
    recovery.require(answer.get('ok') is True, 'Native GUI operation failed: ' + str(answer.get('error', 'unknown')))
    return answer


def load_record(path):
    value = json.loads(Path(path).read_text(encoding='utf-8'))
    recovery.require(value.get('ability_package') == recovery.PACKAGE
                     and value.get('ability_sha256') in recovery.ALLOWED_APK_SHA256S
                     and value.get('socket') == patcher.SOCKET
                     and value.get('scope') == 'current_ability_process',
                     'Installation record does not identify the pinned adapter')
    recovery.require(type(value.get('pid')) is int and value['pid'] > 1
                     and str(value.get('start_ticks', '')).isdigit()
                     and isinstance(value.get('boot_id'), str)
                     and type(value.get('companion_uid')) is int,
                     'Installation record lacks exact process/companion identity')
    recovery.require(isinstance(value.get('backup'), str) and value['backup'],
                     'Installation record lacks a recovery snapshot')
    return value


def same_target(record, current):
    return current.get('present') is True and all(
        record.get(key) == current.get(key) for key in ('pid', 'start_ticks', 'boot_id'))


def socket_present(adb):
    text = adb.shell('cat', '/proc/net/unix')
    return any(line.split()[-1:] == ['@' + patcher.SOCKET] for line in text.splitlines())


def verify_probe_identity(answer, record):
    expected_hashes = {record['ability_sha256']} if 'ability_sha256' in record else set(recovery.ALLOWED_APK_SHA256S)
    recovery.require(answer.get('pid') == record['pid']
                     and answer.get('uid') == record['companion_uid']
                     and answer.get('target') == recovery.PACKAGE
                     and answer.get('socket') == patcher.SOCKET
                     and answer.get('apk_sha256') in recovery.ALLOWED_APK_SHA256S
                     and answer.get('apk_sha256') in expected_hashes,
                     'Endpoint is not the recorded native GUI adapter')
    recovery.require(answer.get('mode') == 'inspect_only'
                     and answer.get('execution_enabled') is False,
                     'This rollback tool only supports the inspect-only adapter; task cleanup requires its controller')


def rollback(adb, record_path, trusted_apk):
    record_path = Path(record_path)
    record = load_record(record_path)
    companion = patcher.permissions.inspect(trusted_apk)
    recovery.require(companion.get('signature_verified') is True
                     and companion.get('uid') == record['companion_uid'],
                     'Signed companion identity changed since adapter installation')
    before = recovery.verify_backup(adb, record['backup'])
    recovery.require(before['system_server_unchanged'], 'system_server changed; preserve evidence for review')
    process_matches = same_target(record, before['current_process'])
    shutdown = None
    if process_matches and socket_present(adb):
        status = provider(adb, 'status')
        verify_probe_identity(status, record)
        # Narrow the race after IPC: never issue stop after PID/start/boot changed.
        fresh = recovery.capture(adb)
        recovery.require(same_target(record, fresh['ability']), 'Ability changed before shutdown; no stop sent')
        shutdown = provider(adb, 'shutdown')
        recovery.require(shutdown.get('running') is False
                         and shutdown.get('execution_enabled') is False
                         and shutdown.get('rollback') == 'process_adapter_closed',
                         'Adapter did not acknowledge scoped shutdown')
    elif not process_matches and socket_present(adb):
        raise RuntimeError('A different process now owns a GUI endpoint; refusing to stop it')
    deadline = time.monotonic() + 5
    while socket_present(adb):
        recovery.require(time.monotonic() < deadline, 'Adapter listener remains after shutdown')
        time.sleep(0.1)
    after = recovery.verify_backup(adb, record['backup'])
    recovery.require(after['system_server_unchanged'], 'system_server changed during rollback')
    result = dict(status='probe_closed', endpoint_absent=True, shutdown_acknowledged=shutdown is not None,
                  original_apk_and_metadata_unchanged=True, startup_properties_restored=True,
                  system_server_unchanged=True, execution_enabled=False,
                  target_process_replaced=after['target_process_replaced'],
                  in_memory_dex_may_remain=same_target(record, after['current_process']),
                  native_behavior_regression_verified=False,
                  rollback_scope='inspect-only listener; no native method/settings were modified',
                  remaining_proof='Run the original native GUI request to verify unchanged factory behavior.')
    patcher.save(record_path, {**record, 'status': 'probe_closed', 'rollback': result})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adb', default=patcher.permissions.ADB[0])
    parser.add_argument('--transport', required=True)
    parser.add_argument('--record', type=Path, required=True)
    parser.add_argument('--apk', type=Path, required=True)
    args = parser.parse_args()
    recovery.require(args.transport == patcher.permissions.ADB[-1],
                     'Rollback transport must match the companion signer verification transport')
    result = rollback(recovery.Adb(args.adb, args.transport), args.record, args.apk)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps(dict(status='rollback_unverified', error=str(error)), ensure_ascii=False), file=sys.stderr)
        sys.exit(1)
