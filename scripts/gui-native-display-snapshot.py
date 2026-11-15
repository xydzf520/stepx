#!/usr/bin/env python3
"""Research-only readback of existing system_server native GUI redirect state.

The Java operation is read-only. The audited transport temporarily attaches the
android.ui thread and retains its small code arena until reboot; it is not a
zero-impact dumpsys. No redirect setter, task or endpoint is installed.
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
spec = importlib.util.spec_from_file_location('display_snapshot_permissions', ROOT / 'scripts/companion-permissions.py')
permissions = importlib.util.module_from_spec(spec)
spec.loader.exec_module(permissions)
DEX = ROOT / 'desktop/.runtime/native-gui-research/stepx-native-gui-display-snapshot.dex'
DEX_SHA256 = '1e5832039b1ad7341ebc505a39fb195ad673877e647bb7c462987102cd4c4781'
IMAGE_HASHES = {
    '/system/framework/services.jar': 'bfd925847906475a512c0b7bdec616ae955617a2a4f9cc0a2f7892f1b38b6cb1',
    '/system_ext/framework/stepos-services.jar': '488f32dd32462d307e8d41fb99a935ea00135ed4b007c050841fe83ac8fe9aa5',
}
SNAPSHOT_FIELDS = {'version', 'mode', 'pid', 'uid', 'read_only', 'ok',
                   'initialized_wms_singleton', 'redirect_ref_count', 'redirect_component',
                   'stable_observation', 'atomic_snapshot', 'control_packages',
                   'control_package_count', 'ability_control_allowed'}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def validate_snapshot(value, pid):
    require(type(value) is dict and set(value) == SNAPSHOT_FIELDS, 'Unexpected snapshot schema')
    require(value['version'] == 1 and value['mode'] == 'native_gui_display_snapshot', 'Wrong snapshot protocol')
    require(type(value['pid']) is int and value['pid'] == pid and value['uid'] == 1000, 'Wrong snapshot process')
    for name in ('read_only', 'ok', 'initialized_wms_singleton', 'stable_observation'):
        require(value[name] is True, 'Snapshot failed or changed during observation')
    require(value['atomic_snapshot'] is False, 'Invalid snapshot consistency claim')
    require(type(value['redirect_ref_count']) is int and 0 <= value['redirect_ref_count'] <= 10000,
            'Invalid redirect counter')
    component = value['redirect_component']
    require(component is None or type(component) is str and re.fullmatch(r'[A-Za-z0-9_.$]{1,200}/[A-Za-z0-9_.$]{1,240}', component),
            'Invalid redirect component')
    packages = value['control_packages']
    require(type(packages) is list and len(packages) <= 64 and
            all(type(item) is str and re.fullmatch(r'[A-Za-z0-9_.]{1,200}', item) for item in packages),
            'Invalid package set')
    require(packages == sorted(set(packages)) and type(value['control_package_count']) is int
            and value['control_package_count'] == len(packages), 'Invalid package set count')
    require(type(value['ability_control_allowed']) is bool and
            value['ability_control_allowed'] == ('com.stepos.ability' in packages), 'Invalid package membership')
    return value


def build_worker(dex, identity):
    require(dex.startswith(b'dex\n') and len(dex) <= 64 * 1024 and hashlib.sha256(dex).hexdigest() == DEX_SHA256,
            'Unreviewed display snapshot DEX')
    source = read_python_source(permissions.WORKFLOW / 'module/inject.py').decode('utf-8')
    require(hashlib.sha256(source.encode()).hexdigest() == permissions.LOADER_SHA256, 'Audited loader changed')
    source, count = re.subn(r'dex=base64\.b64decode\([^\n]+?\);dexp=', 'dex=base64.b64decode(SNAPSHOT_DEX);dexp=', source)
    require(count == 1 and source.count("cstr('stepx.GmsProbe')") == 1, 'Unsupported loader template')
    source = source.replace("cstr('stepx.GmsProbe')", "cstr('stepx.NativeGuiDisplaySnapshot')")
    old = "assert open('/proc/'+str(pid)+'/comm').read().strip()=='system_server'"
    require(source.count(old) == 1, 'Missing original process guard')
    source = source.replace(old, old + "\nassert open('/proc/'+str(pid)+'/stat').read().rsplit(')',1)[1].split()[19]==SNAPSHOT_START\n"
                            + "assert open('/proc/sys/kernel/random/boot_id').read().strip()==SNAPSHOT_BOOT")
    require('stepx.GmsProbe' not in source and 'ZGV4C' not in source, 'Original mutation payload remains')
    code = (f"SNAPSHOT_DEX={base64.b64encode(dex).decode()!r}\n"
            f"SNAPSHOT_START={identity['start_ticks']!r}\nSNAPSHOT_BOOT={identity['boot_id']!r}\n"
            f"os.environ['STEPX_SYSTEM_SERVER_PID']={str(identity['pid'])!r}\n" + source)
    compile(code, 'native-gui-display-snapshot-worker', 'exec')
    return code


def process_identity():
    pid = permissions.shell('pidof', 'system_server')
    require(re.fullmatch(r'[1-9][0-9]*', pid), 'Expected exactly one system_server')
    stat = permissions.shell('cat', '/proc/' + pid + '/stat')
    ticks = stat.rsplit(')', 1)[1].split()[19]
    boot = permissions.shell('cat', '/proc/sys/kernel/random/boot_id')
    require(ticks.isdigit() and re.fullmatch(r'[0-9a-f-]{36}', boot), 'Invalid process lifetime identity')
    return {'pid': int(pid), 'start_ticks': ticks, 'boot_id': boot}


def compare(previous, current):
    require(previous.get('schema') == current.get('schema') == 'stepx_native_gui_display_snapshot_v1', 'Wrong comparison schema')
    require(previous.get('process') == current.get('process') and previous.get('image_hashes') == current.get('image_hashes'),
            'Snapshots do not belong to the same verified system_server')
    old = validate_snapshot(previous.get('snapshot'), current['process']['pid'])
    new = validate_snapshot(current.get('snapshot'), current['process']['pid'])
    return {key: old[key] == new[key] for key in ('redirect_ref_count', 'redirect_component', 'control_packages')}


def capture():
    permissions.guard(native=True)
    require(python_source_sha256(permissions.WORKFLOW / 'tools/root_worker.py') == permissions.WORKER_SHA256,
            'Audited bounded worker changed')
    for path, expected in IMAGE_HASHES.items():
        require(permissions.shell('sha256sum', path).split()[0] == expected, 'Original system service image changed')
    identity = process_identity()
    code = build_worker(DEX.read_bytes(), identity)
    proof = permissions.run_bounded_android_worker(code, identity['pid'])
    require(proof.get('detached') is True and proof.get('context') == 'u:r:system_server:s0'
            and proof.get('target_pid') == identity['pid'], 'Snapshot transport did not detach from the expected process')
    require(all(proof.get(key) is True for key in ('startup_properties_restored', 'system_server_unchanged',
                'launcher_unchanged', 'root_bridge_healthy', 'google_permission_baseline_unchanged')),
            'Snapshot transport invariants failed')
    require(process_identity() == identity, 'system_server lifetime changed during snapshot')
    raw = proof.get('java_result', '')
    require(type(raw) is str and len(raw.encode()) <= 16384, 'Invalid snapshot output size')
    snapshot = validate_snapshot(json.loads(raw), identity['pid'])
    return {'schema': 'stepx_native_gui_display_snapshot_v1', 'process': identity,
            'image_hashes': IMAGE_HASHES, 'dex_sha256': DEX_SHA256, 'snapshot': snapshot,
            'detached': True, 'startup_properties_restored': proof.get('startup_properties_restored') is True,
            'system_server_unchanged': proof.get('system_server_unchanged') is True,
            'global_rollback_verified': False, 'transport_arena_retained_until_reboot': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path, help='New private proof file; never overwritten')
    parser.add_argument('--compare', type=Path, help='Earlier proof from the same running system_server')
    args = parser.parse_args()
    open_flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, 'O_NOFOLLOW', 0) | getattr(os, 'O_BINARY', 0)
    fd = os.open(args.output, open_flags, 0o600)
    with os.fdopen(fd, 'wb') as output:
        try:
            proof = capture()
            if args.compare:
                proof['matches_previous'] = compare(json.loads(args.compare.read_text(encoding='utf-8')), proof)
            output.write((json.dumps(proof, ensure_ascii=False, indent=2) + '\n').encode('utf-8'))
            output.flush(); os.fsync(output.fileno())
            print(json.dumps(proof, ensure_ascii=False))
        except BaseException as error:
            failure = {'ok': False, 'error_type': type(error).__name__, 'global_rollback_verified': False}
            output.write((json.dumps(failure, ensure_ascii=False) + '\n').encode('utf-8'))
            output.flush(); os.fsync(output.fileno())
            raise


if __name__ == '__main__':
    main()
