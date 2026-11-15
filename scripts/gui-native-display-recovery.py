#!/usr/bin/env python3
"""One failed research run's stale Omni mapping recovery; never a general API.

Requires the collected, exited fixed-test host and its same-boot redirect proof.
Replays the original onDisplayRemoved only for already-dead display 90000001.
It never releases a display, moves a task, changes a counter or edits a map.
ARCHIVED, NOT RUN: the launcher returned to display 0 through its normal flow;
execution is disabled and the Java precondition no longer matches the phone.
Run at most once, then obtain independent display/task/redirect readback. A
failure is not permission to retry. The bounded transport retains its arena.
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

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT / 'scripts') not in sys.path:
    sys.path.insert(0, str(ROOT / 'scripts'))
from source_integrity import python_source_sha256, read_python_source
spec = importlib.util.spec_from_file_location('owned_display_snapshot', ROOT / 'scripts/gui-native-display-snapshot.py')
snapshot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(snapshot)
permissions = snapshot.permissions
DEX = ROOT / 'desktop/.runtime/native-gui-research/stepx-native-gui-display-recovery.dex'
DEX_SHA256 = '13ae058fd9d4a202ebaa225bea1e23dfaf9cfb198527d90e71e0a56d65abb556'
OWNER_ID = 'd0cf1f25c3b94bcdb88513b033bd0fd6'
EXECUTION_ENABLED = False
SUCCESS_FIELDS = {
    'version', 'mode', 'pid', 'uid', 'read_only', 'owned_display_id', 'target_uid',
    'target_package', 'ok', 'scheduled', 'callback_done', 'preconditions_verified',
    'operation_count', 'replayed_original_on_display_removed', 'global_rollback_verified',
    'current_before', 'previous_before', 'current_after', 'previous_after',
    'unrelated_map_unchanged', 'redirect_unchanged', 'control_packages_unchanged',
    'ime_focus_unchanged', 'actual_display_absent',
}


def require(value, message):
    if not value:
        raise RuntimeError(message)


def validate_owner(owner, redirect):
    require(type(owner) is dict and owner.get('schema') == 'stepx.native-gui.task-host.v1'
            and owner.get('id') == OWNER_ID, 'Only the reviewed failed test can be recovered')
    identity = redirect.get('process', {})
    require(redirect.get('schema') == 'stepx_native_gui_display_snapshot_v1'
            and identity.get('pid') == owner.get('system_server_pid')
            and identity.get('boot_id') == owner.get('boot_id')
            and redirect.get('image_hashes') == snapshot.IMAGE_HASHES,
            'Owner and redirect evidence do not belong to the same original system_server')
    state = snapshot.validate_snapshot(redirect.get('snapshot'), identity['pid'])
    require(state['redirect_ref_count'] == 0 and state['redirect_component'] is None
            and state['control_packages'] == ['com.stepos.ability'], 'Redirect baseline is not idle')
    collected = owner.get('collect_proof', {})
    final = collected.get('final', {})
    process = owner.get('process', {})
    require(collected.get('host_exited') is True and collected.get('owned_files_removed') is True
            and collected.get('system_server_unchanged') is True, 'Failed host has not exited and been collected')
    require(process.get('boot_id') == identity['boot_id'] and process.get('context') == 'u:r:system_app:s0'
            and type(process.get('pid')) is int and process['pid'] == final.get('pid')
            and final.get('host_uid') == 1000 and final.get('target_package') == 'com.stepos.ability',
            'Wrong failed host identity')
    require(final.get('phase') == 'ready' and final.get('error_type') == 'SecurityException'
            and final.get('test_invocation_started') is True and final.get('test_terminal') == 'cancel'
            and final.get('owned_task_cleared') is True and final.get('native_task_count_before_release') == 0
            and final.get('mdi_task_count_before_release') == 1 and final.get('adapter_closed') is False
            and final.get('storage_preserved') is True and final.get('cleanup_verified') is False,
            'Failure evidence differs from the reviewed orphan mapping incident')
    return identity


def validate_result(value, pid):
    require(type(value) is dict and set(value) == SUCCESS_FIELDS, 'Recovery did not return the full success evidence')
    require(value['version'] == 1 and value['mode'] == 'native_gui_owned_display_recovery'
            and value['pid'] == pid and value['uid'] == 1000, 'Wrong recovery identity')
    require(value['owned_display_id'] == 90000001 and value['target_uid'] == 10124
            and value['target_package'] == 'com.android.launcher3' and value['operation_count'] == 1,
            'Wrong owned recovery target')
    require(value['read_only'] is False and value['global_rollback_verified'] is False,
            'Invalid recovery scope claim')
    for key in ('ok', 'scheduled', 'callback_done', 'preconditions_verified',
                'replayed_original_on_display_removed', 'unrelated_map_unchanged',
                'redirect_unchanged', 'control_packages_unchanged', 'ime_focus_unchanged', 'actual_display_absent'):
        require(value[key] is True, 'Recovery verification failed')
    require(value['current_before'] == 90000001 and value['previous_before'] == 90000000
            and value['current_after'] == -1 and value['previous_after'] == 90000001,
            'Original removal callback did not produce the expected mapping state')
    return value


def build_worker(dex, identity):
    require(dex.startswith(b'dex\n') and len(dex) <= 64 * 1024 and hashlib.sha256(dex).hexdigest() == DEX_SHA256,
            'Unreviewed one-off recovery DEX')
    source = read_python_source(permissions.WORKFLOW / 'module/inject.py').decode('utf-8')
    require(hashlib.sha256(source.encode()).hexdigest() == permissions.LOADER_SHA256, 'Audited loader changed')
    source, count = re.subn(r'dex=base64\.b64decode\([^\n]+?\);dexp=', 'dex=base64.b64decode(RECOVERY_DEX);dexp=', source)
    require(count == 1 and source.count("cstr('stepx.GmsProbe')") == 1, 'Unsupported loader template')
    source = source.replace("cstr('stepx.GmsProbe')", "cstr('stepx.NativeGuiDisplayRecovery')")
    old = "assert open('/proc/'+str(pid)+'/comm').read().strip()=='system_server'"
    require(source.count(old) == 1, 'Missing original process guard')
    source = source.replace(old, old + "\nassert open('/proc/'+str(pid)+'/stat').read().rsplit(')',1)[1].split()[19]==RECOVERY_START\n"
                            + "assert open('/proc/sys/kernel/random/boot_id').read().strip()==RECOVERY_BOOT")
    require('stepx.GmsProbe' not in source and 'ZGV4C' not in source, 'Original mutation payload remains')
    code = (f"RECOVERY_DEX={base64.b64encode(dex).decode()!r}\n"
            f"RECOVERY_START={identity['start_ticks']!r}\nRECOVERY_BOOT={identity['boot_id']!r}\n"
            f"os.environ['STEPX_SYSTEM_SERVER_PID']={str(identity['pid'])!r}\n" + source)
    compile(code, 'native-gui-owned-display-recovery-worker', 'exec')
    return code


def recover(owner, redirect):
    require(EXECUTION_ENABLED, 'Archived research recovery: the stale mapping already recovered normally')
    expected_identity = validate_owner(owner, redirect)
    permissions.guard(native=True)
    require(python_source_sha256(permissions.WORKFLOW / 'tools/root_worker.py') == permissions.WORKER_SHA256,
            'Audited bounded worker changed')
    for path, expected in snapshot.IMAGE_HASHES.items():
        require(permissions.shell('sha256sum', path).split()[0] == expected, 'Original system service image changed')
    identity = snapshot.process_identity()
    require(identity == expected_identity, 'system_server lifetime differs from failed test')
    # Even a reused host PID makes the evidence ambiguous: refuse before attach.
    permissions.shell('test', '!', '-e', '/proc/' + str(owner['process']['pid']))
    code = build_worker(DEX.read_bytes(), identity)
    proof = permissions.run_bounded_android_worker(code, identity['pid'])
    require(proof.get('detached') is True and proof.get('context') == 'u:r:system_server:s0'
            and proof.get('target_pid') == identity['pid'], 'Recovery transport did not detach correctly')
    require(all(proof.get(key) is True for key in ('startup_properties_restored', 'system_server_unchanged',
                'launcher_unchanged', 'root_bridge_healthy', 'google_permission_baseline_unchanged')),
            'Recovery transport invariants failed')
    require(snapshot.process_identity() == identity, 'system_server lifetime changed during recovery')
    raw = proof.get('java_result', '')
    require(type(raw) is str and len(raw.encode()) <= 16384, 'Invalid recovery output size')
    recovery = validate_result(json.loads(raw), identity['pid'])
    return {'schema': 'stepx_native_gui_owned_display_recovery_v1', 'owner_id': OWNER_ID,
            'process': identity, 'image_hashes': snapshot.IMAGE_HASHES, 'dex_sha256': DEX_SHA256,
            'recovery': recovery, 'detached': True, 'system_server_unchanged': True,
            'global_rollback_verified': False, 'requires_independent_readback': True,
            'transport_arena_retained_until_reboot': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--owner-proof', required=True, type=Path)
    parser.add_argument('--redirect-proof', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path, help='New private result; never overwritten')
    args = parser.parse_args()
    owner_bytes = args.owner_proof.read_bytes()
    redirect_bytes = args.redirect_proof.read_bytes()
    owner, redirect = json.loads(owner_bytes), json.loads(redirect_bytes)
    validate_owner(owner, redirect)
    open_flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, 'O_NOFOLLOW', 0) | getattr(os, 'O_BINARY', 0)
    fd = os.open(args.output, open_flags, 0o600)
    with os.fdopen(fd, 'wb') as output:
        try:
            proof = recover(owner, redirect)
            proof['owner_proof_sha256'] = hashlib.sha256(owner_bytes).hexdigest()
            proof['redirect_proof_sha256'] = hashlib.sha256(redirect_bytes).hexdigest()
            output.write((json.dumps(proof, ensure_ascii=False, indent=2) + '\n').encode('utf-8'))
            output.flush(); os.fsync(output.fileno())
            print(json.dumps(proof, ensure_ascii=False))
        except BaseException as error:
            proof = {'ok': False, 'error_type': type(error).__name__, 'owner_id': OWNER_ID,
                     'global_rollback_verified': False, 'requires_independent_readback': True,
                     'do_not_retry_without_review': True}
            output.write((json.dumps(proof, ensure_ascii=False) + '\n').encode('utf-8'))
            output.flush(); os.fsync(output.fileno())
            raise


if __name__ == '__main__':
    main()
