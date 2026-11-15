#!/usr/bin/env python3
"""Audit, then remove only a recorded, exited research host's private storage.

The old owner format lacks process start ticks. Such directories require one
unique PID/boot match and marker creation within the recorded host lifetime.
The separate audit pins all inodes and owner bytes before explicit cleanup.
This is not global GUI rollback and never signals a process or launches a task.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT / 'scripts') not in sys.path:
    sys.path.insert(0, str(ROOT / 'scripts'))
from host_runtime import file_lock
spec = importlib.util.spec_from_file_location('storage_task_host', ROOT / 'scripts/gui-native-task-host.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

STORAGE_HELPERS = r'''
storage_pattern=r'stepx-gui-task-host-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'
uuid_pattern=r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'
def entry_meta(st):
 checked(st.st_uid==1000 and st.st_gid==1000,'storage_owner_changed')
 checked(stat.S_ISREG(st.st_mode) or stat.S_ISDIR(st.st_mode),'storage_special_file')
 kind='directory' if stat.S_ISDIR(st.st_mode) else 'file'
 special=stat.S_IMODE(st.st_mode)&0o7000
 checked(special in (0,0o2000) if kind=='directory' else special==0,'storage_special_mode')
 if kind=='file':checked(st.st_nlink==1,'storage_hardlink')
 checked(not stat.S_IMODE(st.st_mode)&0o022,'storage_writable_by_other')
 return dict(dev=st.st_dev,ino=st.st_ino,uid=st.st_uid,gid=st.st_gid,mode=stat.S_IMODE(st.st_mode),kind=kind,
             size=st.st_size,mtime_ns=st.st_mtime_ns,ctime_ns=st.st_ctime_ns)
def same_entry(found,expected):
 keys=('dev','ino','uid','gid','mode','kind') if expected['kind']=='directory' else tuple(expected)
 return all(found.get(key)==expected.get(key) for key in keys)
def validate_owner(owner,root_path,marker_stat):
 checked(isinstance(owner,dict) and set(owner).issubset({'pid','boot_id','root','nonce','start_ticks'}),'owner_schema')
 checked(owner.get('pid')==RECORD['process']['pid'] and owner.get('boot_id')==RECORD['boot_id'],'owner_process_mismatch')
 checked(owner.get('root')==root_path and re.fullmatch(uuid_pattern,owner.get('nonce','')),'owner_path_or_nonce')
 if 'start_ticks' in owner:
  checked(str(owner['start_ticks'])==RECORD['process']['start_ticks'],'owner_start_changed')
  return 'pid_start_boot'
 lower=RECORD['start_proof'].get('device_started_epoch')
 upper=RECORD['collect_proof'].get('device_started_epoch')
 checked(type(lower) is int and type(upper) is int and lower<=upper<=lower+900,'legacy_lifetime_missing')
 checked(lower<=marker_stat.st_ctime<=upper+2,'legacy_owner_outside_lifetime')
 return 'legacy_pid_boot_time_window'
def read_owner(rootfd):
 fd=os.open('owner.json',os.O_RDONLY|os.O_NOFOLLOW,dir_fd=rootfd)
 try:
  st=os.fstat(fd);meta=entry_meta(st)
  checked(meta['kind']=='file' and meta['mode']==0o600 and 0<st.st_size<=4096,'owner_file_invalid')
  raw=os.read(fd,4097);checked(len(raw)==st.st_size,'owner_file_changed')
  return json.loads(raw.decode('utf-8','strict')),hashlib.sha256(raw).hexdigest(),st
 finally:os.close(fd)
def inventory(rootfd,device):
 entries={};total=[0]
 def visit(fd,parent,depth):
  checked(depth<=8,'storage_too_deep')
  for name in sorted(os.listdir(fd)):
   checked(name not in ('.','..') and '/' not in name and len(name)<=255,'storage_name_invalid')
   relative=parent+'/'+name if parent else name
   meta=entry_meta(os.stat(name,dir_fd=fd,follow_symlinks=False))
   checked(meta['dev']==device,'storage_mount_changed')
   entries[relative]=meta;checked(len(entries)<=256,'storage_too_many_files')
   if meta['kind']=='directory':
    child=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd)
    try:
     checked(same_entry(entry_meta(os.fstat(child)),meta),'storage_directory_changed')
     visit(child,relative,depth+1)
    finally:os.close(child)
   else:
    total[0]+=meta['size'];checked(total[0]<=64*1024*1024,'storage_too_large')
 visit(rootfd,'',0)
 return entries,total[0]
def audit_candidate(name):
 checked(re.fullmatch(storage_pattern,name),'storage_name_outside_scope')
 st=os.stat(name,dir_fd=directory,follow_symlinks=False)
 checked(stat.S_ISDIR(st.st_mode),'storage_root_not_directory')
 rootfd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=directory)
 try:
  found=os.fstat(rootfd)
  checked((found.st_dev,found.st_ino)==(st.st_dev,st.st_ino),'storage_root_changed')
  owner,owner_sha,marker_stat=read_owner(rootfd)
  if owner.get('pid')!=RECORD['process']['pid'] or owner.get('boot_id')!=RECORD['boot_id']:return None
  # Diagnose only a matching owner; unrelated old research directories do not
  # inherit this run's strict root-mode rule. Never expose marker contents.
  out['storage_root_diagnostic']=dict(mode=stat.S_IMODE(found.st_mode),uid=found.st_uid,gid=found.st_gid,
      same_device=found.st_dev==os.fstat(directory).st_dev,owner_process_matches=True)
  publish(out)
  meta=entry_meta(found)
  checked(meta['kind']=='directory' and meta['mode'] in (0o700,0o2700) and meta['dev']==os.fstat(directory).st_dev,'storage_root_invalid')
  root_path=RECORD['code_cache_realpath']+'/'+name
  binding=validate_owner(owner,root_path,marker_stat)
  entries,total=inventory(rootfd,meta['dev'])
  checked('owner.json' in entries,'owner_missing')
  return dict(name=name,root=root_path,root_metadata=meta,owner=owner,owner_sha256=owner_sha,
              ownership_binding=binding,entries=entries,bytes=total)
 finally:os.close(rootfd)
def remove_pinned_tree(candidate):
 name=candidate['name']
 checked(audit_candidate(name)==candidate,'storage_changed_since_audit')
 rootfd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=directory)
 try:
  entries=candidate['entries']
  # The marker is last, so interrupted cleanup remains auditable and cannot
  # cause later cleanup to guess ownership from a directory name alone.
  ordered=sorted((path for path in entries if path!='owner.json'),key=lambda path:(-path.count('/'),entries[path]['kind']=='directory',path))
  for relative in ordered+['owner.json']:
   parts=relative.split('/');parentfd=os.dup(rootfd)
   try:
    prefix=''
    for component in parts[:-1]:
     prefix=prefix+'/'+component if prefix else component
     child=os.open(component,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=parentfd)
     checked(same_entry(entry_meta(os.fstat(child)),entries[prefix]),'cleanup_parent_changed')
     os.close(parentfd);parentfd=child
    found=entry_meta(os.stat(parts[-1],dir_fd=parentfd,follow_symlinks=False))
    checked(same_entry(found,entries[relative]),'cleanup_entry_changed')
    if found['kind']=='directory':os.rmdir(parts[-1],dir_fd=parentfd)
    else:os.unlink(parts[-1],dir_fd=parentfd)
    out['removed_entry_count']=out.get('removed_entry_count',0)+1;publish(out)
   finally:os.close(parentfd)
  checked(same_entry(entry_meta(os.stat(name,dir_fd=directory,follow_symlinks=False)),candidate['root_metadata']),'cleanup_root_changed')
  os.rmdir(name,dir_fd=directory)
 finally:os.close(rootfd)
'''


def build_worker(record, cleanup=False):
    runner.validate_process(record.get('process'), record)
    runner.collected_evidence(record)
    if cleanup:
        proof = record.get('storage_audit_proof', {})
        runner.require(proof.get('status') == 'storage_audited' and proof.get('host_exited') is True
                       and proof.get('storage_count') == 1 and isinstance(proof.get('storage'), dict)
                       and not proof.get('operation_error') and not proof.get('error'),
                       'A successful unique storage audit is required before cleanup')
    prior = record.get('storage_cleanup_proof', {})
    removed = cleanup and prior.get('owned_storage_removed') is True
    if removed:
        candidate = record['storage_audit_proof']['storage']
        runner.require(prior.get('operation') == 'task_host_storage_cleanup'
                       and prior.get('owned_root') == candidate.get('root')
                       and prior.get('owner_sha256') == candidate.get('owner_sha256')
                       and not prior.get('operation_error') and not prior.get('error'),
                       'Prior cleanup evidence does not match the audited storage')
    code = (runner.worker_prefix(record) + '\nDO_CLEANUP=' + repr(cleanup)
            + '\nALREADY_REMOVED=' + repr(removed) + '\n' + STORAGE_HELPERS + r'''
out['operation']='task_host_storage_cleanup' if DO_CLEANUP else 'task_host_storage_audit'
try:
 checked(observed(RECORD['process']['pid']) is None,'host_still_live')
 out['host_exited']=True;publish(out)
 if DO_CLEANUP:
  candidate=RECORD['storage_audit_proof']['storage']
  out['owned_root']=candidate['root'];out['owner_sha256']=candidate['owner_sha256'];publish(out)
  if ALREADY_REMOVED:
   try:os.stat(candidate['name'],dir_fd=directory,follow_symlinks=False)
   except FileNotFoundError:out['verification_only']=True
   else:raise RuntimeError('removed_storage_path_reappeared')
  else:remove_pinned_tree(candidate)
  out['status']='storage_removed';out['owned_storage_removed']=True
 else:
  candidates=[]
  names=[name for name in os.listdir(directory) if re.fullmatch(storage_pattern,name)]
  checked(len(names)<=128,'too_many_host_directories')
  for name in names:
   candidate=audit_candidate(name)
   if candidate is not None:candidates.append(candidate)
  checked(len(candidates)<=1,'storage_owner_ambiguous')
  out['storage_count']=len(candidates)
  if candidates:out['storage']=candidates[0]
  out['status']='storage_audited' if candidates else 'storage_absent'
except BaseException as error:
 out['operation_error']=type(error).__name__
 out['operation_error_code']=str(error) if re.fullmatch(r'[a-z0-9_]{1,100}',str(error)) else 'unclassified'
 out['cleanup_verified']=False
finally:
 os.close(directory);publish(out)
''')
    runner.require('CODE' not in code and 'RESULT' not in code, 'Worker conflicts with transport template placeholders')
    compile(code, '<host-storage-worker>', 'exec')
    return code


def run(record_path, cleanup=False):
    record_path, record = runner.load_record(record_path)
    runner.collected_evidence(record)
    permissions, adb = runner.permissions_and_adb()
    runner.strict_baseline(adb, record['backup'])
    runner.require(adb.shell('cat', '/proc/sys/kernel/random/boot_id') == record['boot_id'], 'Storage recovery requires the original boot')
    runner.require(runner.external_process(adb, record) is None, 'Storage owner has not exited')
    if cleanup:
        runner.require(runner.readiness.display_baseline(adb) == record['display_baseline'], 'Native displays have not returned to baseline')
        runner.require(not any(line.split()[-1:] == ['@' + runner.SOCKET]
                               for line in adb.shell('cat', '/proc/net/unix').splitlines()), 'Another host is active; storage cleanup deferred')
    permissions.guard(native=True)
    operation = 'storage_cleanup' if cleanup else 'storage_audit'
    proof = runner.preserve_worker(permissions, record_path, record, build_worker(record, cleanup), operation)
    runner.require(not proof.get('operation_error') and not proof.get('error'), 'Storage operation unverified; audit evidence retained')
    runner.strict_baseline(adb, record['backup'])
    if cleanup:
        runner.require(proof.get('owned_storage_removed') is True, 'Owned storage removal not verified')
        result = dict(ok=True, status='storage_removed', owned_storage_removed=True, record=str(record_path),
                      root=proof['owned_root'], rollback_verified=False, global_rollback_verified=False)
        record['storage_cleanup'] = result
    else:
        runner.require(proof.get('status') in ('storage_audited', 'storage_absent'), 'Storage audit did not finish')
        candidate = proof.get('storage', {})
        result = dict(ok=True, status=proof['status'], record=str(record_path), storage_count=proof['storage_count'],
                      root=candidate.get('root'), ownership_binding=candidate.get('ownership_binding'),
                      file_count=len(candidate.get('entries', {})), bytes=candidate.get('bytes', 0),
                      rollback_verified=False, global_rollback_verified=False)
    runner.recovery.private_json(record_path, record)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('audit', 'cleanup'))
    parser.add_argument('--record', type=Path, required=True)
    args = parser.parse_args()
    with file_lock(args.record.parent / '.native-gui-task-host.lock'):
        print(json.dumps(run(args.record, args.command == 'cleanup'), ensure_ascii=False))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps(dict(ok=False, error=str(error), rollback_verified=False), ensure_ascii=False), file=sys.stderr)
        raise SystemExit(1)
