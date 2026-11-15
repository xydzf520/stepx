#!/usr/bin/env python3
"""Build a device-local recovery snapshot operation. --run executes it once."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from build_payload import compile_java, replace_dex
from common import ROOT, STATE, guard, init_state, private_write, shell, subprocess_options, user_state_root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', action='store_true')
    parser.add_argument('--snapshot-id', type=int, default=int(time.time()))
    parser.add_argument(
        '--out',
        type=Path,
        default=user_state_root() / 'google-play-stepx/snapshots',
        help='Output root directory outside read-only application bundles',
    )
    args = parser.parse_args()
    guard(native=True)
    assert 'RUNNING_UNLOCKED' in shell('dumpsys', 'user'), 'Unlock phone before CE snapshot'
    assert 0 < args.snapshot_id < 2**31
    init_state()
    folder = Path(args.out) / ('snapshot-' + str(args.snapshot_id))
    folder.mkdir(parents=True, exist_ok=False, mode=0o700)
    source = (ROOT / 'snapshot-java/GmsProbe.java').read_text(encoding='utf-8').replace('1792601000', str(args.snapshot_id))
    (folder / 'GmsProbe.java').write_text(source, encoding='utf-8')
    dex = compile_java(folder / 'GmsProbe.java', folder)
    operation = replace_dex((ROOT / 'module/inject.py').read_text(encoding='utf-8'), dex).replace('def wait_stop(timeout=5):', 'def wait_stop(timeout=45):')
    discover = '''lib.setfsuid(0)
candidates=[]
for item in os.listdir('/proc'):
 if item.isdigit():
  try:
   if open('/proc/'+item+'/comm').read().strip()=='system_server':candidates.append(int(item))
  except OSError:pass
assert len(candidates)==1
os.environ['STEPX_SYSTEM_SERVER_PID']=str(candidates[0])
'''
    tail = (ROOT / 'tools/snapshot_tail.py.txt').read_text(encoding='utf-8').replace('SNAPSHOT_ID', str(args.snapshot_id))
    operation = discover + operation + '\n' + tail
    compile(operation, 'snapshot-operation.py', 'exec')
    path = folder / 'operation.py'
    private_write(path, operation)
    print('Prepared snapshot operation:', path, flush=True)
    if args.run:
        env = os.environ.copy()
        env['STEPX_WORKER_TIMEOUT'] = '60'
        result = STATE / ('account-snapshot-' + str(args.snapshot_id) + '.json')
        subprocess.run(
            [sys.executable, str(ROOT / 'tools/root_worker.py'), str(path), str(result)],
            env=env, check=True, **subprocess_options(),
        )
        proof = json.loads(result.read_text(encoding='utf-8'))
        assert not proof.get('error'), proof.get('error')
        assert proof.get('detached') and proof.get('java_result', '').count('SNAPSHOT_OK ') == 3
        assert len(proof.get('account_database_snapshots', [])) == 2
        print('Three application snapshots and two device-local account DB copies confirmed.')
    else:
        print('Prepared only. No system_server injection or device snapshot was executed.')


if __name__ == '__main__':
    main()
