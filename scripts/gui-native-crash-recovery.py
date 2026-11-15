#!/usr/bin/env python3
"""Audit then reclaim an exited research host with an exactly empty result file.

This separate path never creates a final event or relaxes normal collection.
It proves only ownership and deletion of staged files/private host storage. A
crash, empty output, or missing endpoint does not prove that no task ever ran.
"""
import argparse
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
from host_runtime import file_lock
spec = importlib.util.spec_from_file_location('crash_storage', ROOT / 'scripts/gui-native-host-storage.py')
storage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(storage)
runner = storage.runner


def crash_evidence(record):
    runner.validate_record(record)
    runner.validate_process(record.get('process'), record)
    proof = record.get('start_proof', {})
    runner.require(proof.get('operation') == 'task_host_start' and proof.get('host_spawned') is True
                   and proof.get('detached') is True and proof.get('phase') == 'host_detached'
                   and proof.get('worker_exit_code') == 0 and 'operation_error' not in proof and 'error' not in proof
                   and proof.get('process') == record['process'] and proof.get('files') == record.get('files')
                   and proof.get('code_cache_realpath') == record['code_cache_realpath'],
                   'Complete matching detached-start proof is required')
    runner.require(set(record.get('files', {})) == {'dex', 'result'}, 'Both staged inode records are required')
    for key, mode in (('dex', 0o400), ('result', 0o600)):
        value = record['files'][key]
        runner.require(runner.file_matches(value, value) and value['mode'] == mode, 'Invalid staged inode metadata')
    prior = record.get('collect_proof', {})
    runner.require(prior.get('operation') == 'task_host_collect' and prior.get('host_exited') is True
                   and prior.get('worker_exit_code') == 0 and prior.get('operation_error') == 'RuntimeError'
                   and not prior.get('final') and not prior.get('removed') and 'error' not in prior
                   and not prior.get('owned_files_removed'),
                   'An exited host with failed collection and no removed files is required')


CRASH_OPERATIONS = r'''
def audit_staged():
 found={}
 for key,name in (('dex',RECORD['dex_name']),('result',RECORD['result_name'])):
  fd=os.open(name,os.O_RDONLY|os.O_NOFOLLOW,dir_fd=directory)
  try:
   st=os.fstat(fd);meta=metadata(st)
   checked(match_file(meta,RECORD['files'][key]),'staged_inode_changed')
   checked(0<st.st_size<=512*1024 if key=='dex' else st.st_size==0,'staged_dex_size' if key=='dex' else 'result_not_empty')
   digest=hashlib.sha256()
   if key=='dex':
    remaining=st.st_size
    while remaining:
     part=os.read(fd,min(8192,remaining));checked(bool(part),'staged_short_read')
     digest.update(part);remaining-=len(part)
   else:checked(os.read(fd,1)==b'','result_not_empty')
   end=os.fstat(fd)
   checked(match_file(metadata(end),meta) and end.st_size==st.st_size and end.st_mtime_ns==st.st_mtime_ns and end.st_ctime_ns==st.st_ctime_ns,'staged_changed_during_audit')
   sha=digest.hexdigest()
   checked(sha==(RECORD['dex_sha256'] if key=='dex' else hashlib.sha256(b'').hexdigest()),'staged_digest_mismatch')
   found[key]=dict(metadata=meta,size=st.st_size,sha256=sha,mtime_ns=st.st_mtime_ns,ctime_ns=st.st_ctime_ns)
  finally:os.close(fd)
 return found
def audit_private_storage():
 names=[name for name in os.listdir(directory) if re.fullmatch(storage_pattern,name)]
 checked(len(names)<=128,'too_many_host_directories')
 candidates=[]
 for name in names:
  candidate=audit_candidate(name)
  if candidate is not None:
   checked(candidate['ownership_binding']=='pid_start_boot' and 'start_ticks' in candidate['owner'],'crash_owner_start_ticks_required')
   candidates.append(candidate)
 checked(len(candidates)==1,'crash_storage_unique_owner_required')
 return candidates[0]
def verify_removed():
 candidate=RECORD['crash_audit_proof']['storage']
 for name in (RECORD['dex_name'],RECORD['result_name'],candidate['name']):
  try:os.stat(name,dir_fd=directory,follow_symlinks=False)
  except FileNotFoundError:pass
  else:raise RuntimeError('removed_crash_path_reappeared')
 out['owned_storage_removed']=True;out['owned_staged_removed']=True
 out['owned_root']=candidate['root'];out['removed_staged']={'dex':True,'result':True}
 out['verification_only']=True;out['status']='crash_files_removed'
out['operation']='task_host_crash_cleanup' if DO_CLEANUP else 'task_host_crash_audit'
out['removed_staged']={}
out['final_event_available']=False
out['execution_verified']=False
out['no_task_started_verified']=False
out['global_rollback_verified']=False
try:
 checked(EXTERNALLY_EXITED,'exit_not_independently_verified')
 checked(observed(RECORD['process']['pid']) is None,'crash_host_still_live')
 out['host_exited']=True;publish(out)
 if ALREADY_REMOVED:verify_removed()
 else:
  staged=audit_staged()
  candidate=audit_private_storage()
  if not DO_CLEANUP:
   out['staged']=staged;out['storage']=candidate;out['storage_count']=1
   out['status']='crash_audited'
  else:
   pinned=RECORD['crash_audit_proof']
   checked(staged==pinned['staged'],'staged_changed_since_crash_audit')
   checked(candidate==pinned['storage'],'storage_changed_since_crash_audit')
   # All original staged files and the entire owner-bound tree are validated
   # before the first removal. The existing helper checks each inode again.
   remove_pinned_tree(candidate)
   out['owned_storage_removed']=True;out['owned_root']=candidate['root'];publish(out)
   for key,name in (('dex',RECORD['dex_name']),('result',RECORD['result_name'])):
    st=os.stat(name,dir_fd=directory,follow_symlinks=False)
    expected=pinned['staged'][key]
    checked(match_file(metadata(st),expected['metadata']) and st.st_size==expected['size']
            and st.st_mtime_ns==expected['mtime_ns'] and st.st_ctime_ns==expected['ctime_ns'],'staged_changed_before_unlink')
    os.unlink(name,dir_fd=directory)
    out['removed_staged'][key]=True;publish(out)
   out['owned_staged_removed']=True;out['status']='crash_files_removed'
except BaseException as error:
 out['operation_error']=type(error).__name__
 out['operation_error_code']=str(error) if re.fullmatch(r'[a-z0-9_]{1,100}',str(error)) else 'unclassified'
 out['cleanup_verified']=False
finally:
 os.close(directory);publish(out)
'''


def build_worker(record, externally_exited, cleanup=False):
    crash_evidence(record)
    runner.require(type(externally_exited) is bool, 'Independent process observation is required')
    if cleanup:
        proof = record.get('crash_audit_proof', {})
        runner.require(proof.get('operation') == 'task_host_crash_audit' and proof.get('status') == 'crash_audited'
                       and proof.get('host_exited') is True and proof.get('storage_count') == 1
                       and proof.get('worker_exit_code') == 0 and 'operation_error' not in proof and 'error' not in proof
                       and set(proof.get('staged', {})) == {'dex', 'result'} and isinstance(proof.get('storage'), dict),
                       'Successful complete crash audit is required before cleanup')
    prior = record.get('crash_cleanup_proof', {})
    removed = cleanup and prior.get('owned_staged_removed') is True
    if removed:
        runner.require(prior.get('operation') == 'task_host_crash_cleanup' and prior.get('status') == 'crash_files_removed'
                       and prior.get('owned_storage_removed') is True and prior.get('removed_staged') == {'dex': True, 'result': True}
                       and prior.get('host_exited') is True and prior.get('worker_exit_code') == 0
                       and prior.get('owned_root') == record['crash_audit_proof']['storage'].get('root')
                       and 'operation_error' not in prior and 'error' not in prior,
                       'Partial crash deletion requires review; automatic verification is not authorized')
    code = runner.worker_prefix(record) + '\nEXTERNALLY_EXITED=' + repr(externally_exited) + '\nDO_CLEANUP=' + repr(cleanup) + '\nALREADY_REMOVED=' + repr(removed) + '\n' + storage.STORAGE_HELPERS + CRASH_OPERATIONS
    runner.require('CODE' not in code and 'RESULT' not in code, 'Worker conflicts with transport placeholders')
    compile(code, '<crash-recovery-worker>', 'exec')
    return code


def verify_baseline(adb, record):
    before = runner.strict_baseline(adb, record['backup'])
    runner.require(adb.shell('cat', '/proc/sys/kernel/random/boot_id') == record['boot_id'], 'Original boot is required')
    runner.require(runner.external_process(adb, record) is None, 'Recorded PID is still live or has been reused')
    runner.require(runner.readiness.display_baseline(adb) == record['display_baseline'], 'Original display baseline is required')
    runner.require(not any(row.split()[-1:] == ['@' + runner.SOCKET]
                           for row in adb.shell('cat', '/proc/net/unix').splitlines()), 'Research socket is still present')
    return dict(original_files_verified=before['immutable_files_verified'], original_process_unchanged=True,
                system_server_unchanged=True, boot_unchanged=True, host_absent_in_shell=True,
                display_baseline_restored=True, socket_absent=True, global_rollback_verified=False)


def run(record_path, cleanup=False):
    record_path, record = runner.load_record(record_path)
    crash_evidence(record)
    permissions, adb = runner.permissions_and_adb()
    preconditions = verify_baseline(adb, record)
    record['crash_recovery_preconditions'] = preconditions
    runner.recovery.private_json(record_path, record)
    permissions.guard(native=True)
    operation = 'crash_cleanup' if cleanup else 'crash_audit'
    proof = runner.preserve_worker(permissions, record_path, record, build_worker(record, True, cleanup), operation)
    runner.require('operation_error' not in proof and 'error' not in proof,
                   'Crash recovery unverified; retained proof records any partial deletion')
    postconditions = verify_baseline(adb, record)
    if cleanup:
        runner.require(proof.get('status') == 'crash_files_removed' and proof.get('owned_storage_removed') is True
                       and proof.get('owned_staged_removed') is True and proof.get('removed_staged') == {'dex': True, 'result': True},
                       'Owned crash file removal is incomplete')
    else:
        runner.require(proof.get('status') == 'crash_audited', 'Crash audit is incomplete')
    result = dict(ok=True, status=proof['status'], record=str(record_path), host_exited=True,
                  final_event_available=False, execution_verified=False, no_task_started_verified=False,
                  global_rollback_verified=False, rollback_verified=False, postconditions=postconditions)
    if cleanup:
        result.update(owned_storage_removed=True, owned_staged_removed=True, verification_only=proof.get('verification_only') is True)
    else:
        result.update(storage_count=1, result_bytes=proof['staged']['result']['size'],
                      file_count=len(proof['storage']['entries']), bytes=proof['storage']['bytes'])
    record[operation] = result
    runner.recovery.private_json(record_path, record)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('crash-audit', 'crash-cleanup'))
    parser.add_argument('--record', type=Path, required=True)
    args = parser.parse_args()
    with file_lock(args.record.parent / '.native-gui-task-host.lock'):
        print(json.dumps(run(args.record, args.command == 'crash-cleanup'), ensure_ascii=False))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps(dict(ok=False, error=str(error), rollback_verified=False), ensure_ascii=False), file=sys.stderr)
        raise SystemExit(1)
