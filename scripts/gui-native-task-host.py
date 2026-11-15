#!/usr/bin/env python3
"""Bounded research runner for the original SDK's fixed Settings test.

Start only prepares the independent host. A signed research companion controls
the task separately. No original process is stopped, no signal is sent, and
neither file cleanup nor host exit is represented as complete GUI rollback.
"""
import argparse
import base64
import gzip
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT / 'scripts') not in sys.path:
    sys.path.insert(0, str(ROOT / 'scripts'))
from host_runtime import file_lock
DEX = ROOT / 'desktop/.runtime/native-gui-research/stepx-native-gui-task-host.dex'
# Fixed-test gate and artifact reviewed by the coordinating task; never a general executor.
# Disabled after the original MMKV native reader crashed the disposable host.
# Keep collection/recovery available; review a new artifact before any new start.
DEX_SHA256 = 'UNREVIEWED'
SCHEMA = 'stepx.native-gui.task-host.v1'
MAIN = 'stepx.NativeGuiTaskHost'
SOCKET = 'stepx-native-gui-test-v1'
PROVIDER = 'content://com.stepx.companion.app.native_gui_task_probe'
STATUS_METHOD = 'status'
REMOTE_DIR = '/data/user/0/com.stepos.ability/code_cache'
REMOTE_ALIASES = (REMOTE_DIR, '/data/data/com.stepos.ability/code_cache')
APP_PROCESS = '/system/bin/app_process64'
APP_PROCESS_SHA256 = '5acb9827648f9adba44ae2bb16f81ef006ba5564dd81c2a2efd854dccb140bae'
MAX_OUTPUT_BYTES = 128 * 1024


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


recovery = module('task_host_recovery', 'gui-native-backup.py')
readiness = module('task_host_readiness', 'gui-native-readiness.py')


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def read_payload():
    require(re.fullmatch(r'[0-9a-f]{64}', DEX_SHA256) is not None,
            'Task host DEX is not yet reviewed; launch is disabled')
    payload = DEX.read_bytes()
    require(payload.startswith(b'dex\n') and len(payload) <= 512 * 1024
            and hashlib.sha256(payload).hexdigest() == DEX_SHA256,
            'Task host DEX differs from the reviewed artifact')
    return payload


def validate_record(record):
    require(record.get('schema') == SCHEMA
            and record.get('serial') == recovery.SERIAL
            and record.get('fingerprint') in recovery.ALLOWED_FINGERPRINTS
            and record.get('ability_sha256') in recovery.ALLOWED_APK_SHA256S
            and record.get('main') == MAIN and record.get('socket') == SOCKET,
            'Unknown task host identity or firmware')
    require(re.fullmatch(r'[0-9a-f]{32}', record.get('id', '')) is not None,
            'Invalid task host record identifier')
    require(re.fullmatch(r'[0-9a-f]{64}', record.get('signer', '')) is not None
            and re.fullmatch(r'[0-9a-f]{64}', record.get('dex_sha256', '')) is not None
            and type(record.get('companion_uid')) is int
            and 10000 <= record['companion_uid'] < 100000,
            'Invalid companion or DEX identity')
    require(re.fullmatch(r'[0-9a-f-]{36}', record.get('boot_id', '')) is not None,
            'Invalid boot identity')
    for key, suffix in (('dex_name', '.dex'), ('result_name', '.result.jsonl')):
        require(record.get(key) == 'stepx-native-gui-task-' + record['id'] + suffix,
                'Remote file is outside the unique host scope')
    require(isinstance(record.get('backup'), str) and Path(record['backup']).is_absolute(),
            'Recovery directory is not absolute')
    require(all(type(record.get(key)) is int and record[key] > 1
                for key in ('system_server_pid', 'ability_pid')), 'Original process inventory is missing')
    canonical = record.get('code_cache_realpath')
    require(canonical in REMOTE_ALIASES or (canonical is None and record.get('status') == 'preparing'
                                          and 'process' not in record),
            'Unreviewed code_cache directory alias')
    return record


def same_process(left, right):
    return isinstance(left, dict) and isinstance(right, dict) and left == right


def validate_process(value, record):
    require(isinstance(value, dict) and type(value.get('pid')) is int and value['pid'] > 1,
            'Host PID is not verified')
    full = [APP_PROCESS, '/system/bin', MAIN, '--fixed-settings-test',
            record['signer'], str(record['companion_uid'])]
    require(value.get('argv') in (full, [MAIN])
            and value.get('uids') == [1000] * 4 and value.get('gids') == [1000] * 4
            and value.get('context') == 'u:r:system_app:s0'
            and value.get('boot_id') == record['boot_id']
            and str(value.get('start_ticks', '')).isdigit(), 'Host process identity mismatch')


def file_matches(found, expected):
    return (isinstance(found, dict) and isinstance(expected, dict)
            and all(found.get(k) == expected.get(k) for k in ('dev', 'ino', 'uid', 'gid', 'mode', 'nlink'))
            and found.get('uid') == 1000 and found.get('gid') == 1000
            and found.get('nlink') == 1 and found.get('mode') in (0o400, 0o600))


# Shared by the emitted collect worker and local unit tests. Never forward raw
# stdout, exception messages, task events, screen text, account data, or images.
RESULT_FILTER = r'''
def filtered_result(raw, record):
 if len(raw)>MAX_OUTPUT_BYTES:raise RuntimeError('result_too_large')
 lines=[line for line in raw.decode('utf-8','strict').splitlines() if line.strip()]
 if not lines:raise RuntimeError('result_missing')
 value=json.loads(lines[-1])
 if not isinstance(value,dict) or value.get('event')!='finished':raise RuntimeError('final_event_missing')
 if value.get('pid')!=record['process']['pid'] or value.get('host_uid')!=1000:raise RuntimeError('final_process_mismatch')
 if value.get('target_package')!='com.stepos.ability' or value.get('socket')!='stepx-native-gui-test-v1':raise RuntimeError('final_target_mismatch')
 if value.get('companion_uid')!=record['companion_uid']:raise RuntimeError('final_companion_mismatch')
 if type(value.get('ok')) is not bool:raise RuntimeError('final_ok_missing')
 scope=value.get('cleanup_scope')
 if scope not in ('host_process_resources','temporary_storage_only','no_initialization'):raise RuntimeError('unknown_cleanup_scope')
 if type(value.get('cleanup_verified')) is not bool and not (scope=='no_initialization' and value.get('cleanup_verified') is None and value['ok'] is False):raise RuntimeError('final_cleanup_missing')
 if value.get('global_rollback_verified') is not False:raise RuntimeError('unsupported_global_rollback_claim')
 result={k:value[k] for k in ('event','pid','host_uid','target_package','socket','companion_uid','ok','cleanup_verified','global_rollback_verified')}
 for key in ('storage_removed','storage_preserved','adapter_closed','database_closed','llm_unbound','sdk_released','clipboard_restored','timed_out','execution_verified','companion_signature_verified','account_accessed','memory_db_closed','task_memory_closed','listener_closed','cloud_client_initialized','cloud_client_released','mmkv_root_isolated','mmkv_released','sdk_initialized','mdi_released','model_service_disconnected_after_release','memory_db_open','memory_db_path_verified','test_invocation_started','owned_task_cleared','local_account_present','content_providers_released','companion_binder_published','companion_binder_unregistered','owned_upload_cancel_requested','settings_launch_accepted','isolated_prompt_overrides_disabled','query_cleanup_verified','mdi_factory_released'):
  if key in value:
   if type(value[key]) is not bool:raise RuntimeError('invalid_result_boolean')
   result[key]=value[key]
 for key in ('tasks_started','active_task_count','action_events','host_deadline_ms','native_task_count_before_release','mdi_task_count_before_release','test_action_events','tasks_started_automatically','provider_refs_acquired','provider_refs_outstanding','providers_returned'):
  if key in value:
   if type(value[key]) is not int or value[key]<0 or value[key]>1000000:raise RuntimeError('invalid_result_count')
   result[key]=value[key]
 if 'settings_launch_result' in value:
  if type(value['settings_launch_result']) is not int or not -10000<=value['settings_launch_result']<=10000:raise RuntimeError('invalid_settings_launch_result')
  result['settings_launch_result']=value['settings_launch_result']
 result['cleanup_scope']=scope
 if 'phase' in value:
  if value['phase'] not in ('memory_database','cloud_logger','local_account','native_gui_initialization','original_device_identifier','cloud_client_initialization','model_observation','companion_control_endpoint','ready'):raise RuntimeError('unknown_phase')
  result['phase']=value['phase']
 if 'error_frames' in value:
  frames=value['error_frames']
  if not isinstance(frames,list) or len(frames)>5:raise RuntimeError('invalid_error_frames')
  pattern=r'[A-Za-z_$][A-Za-z0-9_.$]*\.[A-Za-z_$<>][A-Za-z0-9_$<>]*\((?:[A-Za-z0-9_$.-]+\.(?:java|kt):[0-9]+|Native Method|Unknown Source(?::[0-9]+)?)\)'
  result['error_frames']=[frame for frame in frames if isinstance(frame,str) and len(frame)<=240 and re.fullmatch(pattern,frame)]
  result['error_frames_filtered']=len(frames)-len(result['error_frames'])
 if 'provider_error_category' in value:
  if not isinstance(value['provider_error_category'],str) or not re.fullmatch(r'[A-Za-z0-9_]{0,100}',value['provider_error_category']):raise RuntimeError('invalid_provider_category')
  result['provider_error_category']=value['provider_error_category']
 if 'guard_code' in value:
  if not isinstance(value['guard_code'],str) or not re.fullmatch(r'native_query_[a-z_]{1,90}',value['guard_code']):raise RuntimeError('invalid_guard_code')
  result['guard_code']=value['guard_code']
 if 'test_terminal' in value:
  if value['test_terminal'] not in ('','complete','completed','finish','finished','success','error','failed','failure','cancel','cancelled','stop','stopped','app_not_found'):raise RuntimeError('unknown_terminal_state')
  result['test_terminal']=value['test_terminal']
 if 'model_preferences' in value:
  item=value['model_preferences']
  if not isinstance(item,dict) or item.get('status') not in ('absent','no_saved_selection','invalid_selection','unregistered_selection','saved_selection','inspection_failed'):raise RuntimeError('invalid_model_preferences')
  clean={'status':item['status']}
  for key in ('permission_verified','source_modified','source_exists','saved_selection_present','matches_static_default','custom_model','stable_copy_verified','read_only_copy','snapshot_removed'):
   if key in item:
    if type(item[key]) is not bool:raise RuntimeError('invalid_model_preference_boolean')
    clean[key]=item[key]
  if clean.get('permission_verified') is not False or clean.get('source_modified') is not False:raise RuntimeError('invalid_model_preference_claim')
  if 'current_model_id' in item:
   model=item['current_model_id']
   if not isinstance(model,str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/+:-]{0,159}',model) or '://' in model or '..' in model:raise RuntimeError('invalid_model_preference_id')
   clean['current_model_id']=model
  if 'model_cell_type' in item:
   if item['model_cell_type'] not in ('StepPlanTreeModelCell','DoubaoTarsModelCell','DoubaoSeedModelCell','GeminiModelCell','StepGelabModelCell','GlmAtuoModelCell','GLMModelCell','MiMoModelCell','LocalModelCell','LocalCloudModelCell'):raise RuntimeError('invalid_model_preference_cell')
   clean['model_cell_type']=item['model_cell_type']
  if 'error_class' in item:
   if not isinstance(item['error_class'],str) or not re.fullmatch(r'[A-Za-z0-9_.$]{1,220}',item['error_class']):raise RuntimeError('invalid_model_preference_error')
   clean['error_class']=item['error_class']
  result['model_preferences']=clean
 if 'event_diagnostics' in value:
  item=value['event_diagnostics']
  if not isinstance(item,dict):raise RuntimeError('invalid_event_diagnostics')
  clean={}
  for key in ('installed','removed'):
   if key in item:
    if type(item[key]) is not bool:raise RuntimeError('invalid_event_observer_boolean')
    clean[key]=item[key]
  for key in ('received_count','forwarded_count','observation_failure_count'):
   if key in item:
    if type(item[key]) is not int or not 0<=item[key]<=1000000:raise RuntimeError('invalid_event_observer_count')
    clean[key]=item[key]
  events=item.get('events',[])
  if not isinstance(events,list) or len(events)>12:raise RuntimeError('invalid_event_observer_events')
  clean['events']=[]
  for event in events:
   if not isinstance(event,dict) or event.get('stage') not in ('AGENT_CALL_RECEIVED','EVENT_OBSERVATION_ERROR'):raise RuntimeError('invalid_event_observer_stage')
   entry={'stage':event['stage']}
   for key in ('frame_type','message_type','header_meta_count','raw_body_length','completion_body_length','error_reason_field_length','error_schema_candidate_code'):
    if key in event:
     if type(event[key]) is not int or not -(2**31)<=event[key]<2**31:raise RuntimeError('invalid_event_numeric_field')
     entry[key]=event[key]
   for key in ('command_matches_handler','module_is_client_agent','module_is_gui','module_empty','callback_cache_present','completion_proto_parse_ok','body_inspection_skipped','wire_scan_complete'):
    if key in event:
     if type(event[key]) is not bool:raise RuntimeError('invalid_event_boolean_field')
     entry[key]=event[key]
   for key in ('error_class','completion_parse_error_class'):
    if key in event:
     if not isinstance(event[key],str) or not re.fullmatch(r'[A-Za-z0-9_.$]{1,220}',event[key]):raise RuntimeError('invalid_event_error_class')
     entry[key]=event[key]
   if 'reason_classification_candidate' in event:
    category=event['reason_classification_candidate']
    if category not in ('unauthorized','forbidden','invalid_parameter','rate_limited','rpc_unimplemented','model_unavailable','route_unavailable','server_error','unclassified'):raise RuntimeError('invalid_reason_classification')
    entry['reason_classification_candidate']=category
   if 'reason_keyword_candidates' in event:
    keywords=event['reason_keyword_candidates']
    if not isinstance(keywords,dict):raise RuntimeError('invalid_reason_keywords')
    entry['reason_keyword_candidates']={}
    for key in ('model','not_found','unknown_route','invalid_parameter','unauthorized','forbidden','unimplemented','unknown_service','unavailable','connection_refused','no_healthy_upstream','routing_timeout','rate_limited','server_error'):
     if key in keywords:
      if type(keywords[key]) is not bool:raise RuntimeError('invalid_reason_keyword')
      entry['reason_keyword_candidates'][key]=keywords[key]
   resources=('agent','task','session','conversation','handler','config','processor','service','endpoint','instance','resource','namespace','cluster','tenant','deployment','app','device','account','user','permission','token','key','model')
   config_fields=('mentions_default_model_id','mentions_gui','mentions_invoke','mentions_client_agent','mentions_agent_call','grpc_not_found')
   for group,allowed in (('reason_resource_candidates',resources),('reason_missing_phrase_candidates',tuple(key+'_not_found' for key in resources)),('reason_known_config_candidates',config_fields)):
    if group in event:
     candidates=event[group]
     if not isinstance(candidates,dict):raise RuntimeError('invalid_reason_candidates')
     entry[group]={}
     for key in allowed:
      if key in candidates:
       if type(candidates[key]) is not bool:raise RuntimeError('invalid_reason_candidate')
       entry[group][key]=candidates[key]
   codes=event.get('numeric_header_codes',{})
   if not isinstance(codes,dict):raise RuntimeError('invalid_header_codes')
   entry['numeric_header_codes']={}
   for key in ('code','error_code','status','status_code','errorCode','statusCode'):
    if key in codes:
     if type(codes[key]) is not int or not -999999999<=codes[key]<=999999999:raise RuntimeError('invalid_header_code')
     entry['numeric_header_codes'][key]=codes[key]
   fields=event.get('wire_fields',[])
   if not isinstance(fields,list) or len(fields)>8:raise RuntimeError('invalid_wire_fields')
   entry['wire_fields']=[]
   for field in fields:
    if not isinstance(field,dict) or type(field.get('field')) is not int or not 0<field['field']<=536870911 or type(field.get('wire')) is not int or not 0<=field['wire']<=7:raise RuntimeError('invalid_wire_field')
    entry['wire_fields'].append({'field':field['field'],'wire':field['wire']})
   clean['events'].append(entry)
  result['event_diagnostics']=clean
 if 'model_diagnostics' in value:
  item=value['model_diagnostics']
  if not isinstance(item,dict):raise RuntimeError('invalid_model_diagnostics')
  clean={}
  for key in ('installed','removed','task_override_coverage_verified'):
   if key in item:
    if type(item[key]) is not bool:raise RuntimeError('invalid_model_observer_boolean')
    clean[key]=item[key]
  for key in ('request_call_count','request_generated_count','service_chat_count','service_response_count','parse_action_count','error_count'):
   if key in item:
    if type(item[key]) is not int or not 0<=item[key]<=1000000:raise RuntimeError('invalid_model_observer_count')
    clean[key]=item[key]
  events=item.get('events',[])
  if not isinstance(events,list) or len(events)>24:raise RuntimeError('invalid_model_observer_events')
  clean['events']=[]
  for event in events:
   if not isinstance(event,dict) or not re.fullmatch(r'(REQUEST_GENERATION|SERVICE_CHAT|ACTION_PARSE)_(ENTER|RETURN|SUSPENDED|ERROR|ASYNC_RETURN|ASYNC_ERROR)',event.get('stage','')):raise RuntimeError('invalid_model_observer_stage')
   entry={'stage':event['stage']}
   if 'error_class' in event:
    if not isinstance(event['error_class'],str) or not re.fullmatch(r'[A-Za-z0-9_.$]{1,220}',event['error_class']):raise RuntimeError('invalid_model_observer_error')
    entry['error_class']=event['error_class']
   frames=event.get('error_frames',[])
   if not isinstance(frames,list) or len(frames)>5:raise RuntimeError('invalid_model_observer_frames')
   if frames:entry['error_frames']=[frame for frame in frames if isinstance(frame,str) and len(frame)<=460 and re.fullmatch(r'[A-Za-z0-9_.$<>-]+:[-0-9]{1,10}',frame)]
   clean['events'].append(entry)
  result['model_diagnostics']=clean
 for group,bools,counts in (
  ('mdi_registry_before_aux_cleanup',('ok','read_only','atomic_snapshot','query_reference_present','watcher_reference_present','watcher_observer_present','watcher_timer_present','factory_initialized'),('task_count','query_count','watcher_count','settings_task_count','other_count')),
  ('auxiliary_cleanup',('ok','complete','query_present','callback_invoked','native_tasks_empty','mdi_tasks_empty','query_cleared','display_gone','watcher_present','watcher_reset_invoked','watcher_cleared','watcher_display_gone','cleanup_pending','mdi_factory_release_attempted','mdi_factory_released'),('query_display_id','watcher_display_id'))):
  if group in value:
   item=value[group]
   if not isinstance(item,dict):raise RuntimeError('invalid_diagnostic_group')
   clean={}
   for key in bools:
    if key in item:
     if type(item[key]) is not bool:raise RuntimeError('invalid_diagnostic_boolean')
     clean[key]=item[key]
   for key in counts:
    if key in item:
     if type(item[key]) is not int or not -1<=item[key]<100000000:raise RuntimeError('invalid_diagnostic_count')
     clean[key]=item[key]
   for key in ('error_type','watcher_error','callback_error','mdi_factory_error'):
    if key in item:
     if not isinstance(item[key],str) or not re.fullmatch(r'[A-Za-z0-9_.$]{0,160}',item[key]):raise RuntimeError('invalid_diagnostic_error')
     clean[key]=item[key]
   result[group]=clean
 for key in ('error_class','error','error_type','cleanup_error_class','task_count_error','cloud_release_error','model_release_error','mdi_release_error','memory_release_error','mmkv_release_error','storage_cleanup_error','finalization_error','provider_release_error','binder_release_error','upload_release_error'):
  if key in value and value[key] is not None:
   if not isinstance(value[key],str) or not re.fullmatch(r'[A-Za-z0-9_.$]{1,160}',value[key]):raise RuntimeError('invalid_error_class')
   result[key]=value[key]
 return result
'''
exec(RESULT_FILTER)


WORKER_COMMON = r'''
def checked(condition,code):
 if not condition:raise RuntimeError(code)
def boot():return open('/proc/sys/kernel/random/boot_id').read().strip()
def metadata(st):
 checked(stat.S_ISREG(st.st_mode) and st.st_nlink==1,'file_type_or_links')
 return dict(dev=st.st_dev,ino=st.st_ino,uid=st.st_uid,gid=st.st_gid,mode=stat.S_IMODE(st.st_mode),nlink=st.st_nlink)
def match_file(found,expected):
 return found==expected and found['uid']==1000 and found['gid']==1000 and found['nlink']==1
def observed(pid):
 root='/proc/'+str(pid)
 # A permission failure is NOT proof that the host exited. Only a visible proc
 # directory disappearing can authorize collection, and boot must still match.
 try:os.stat(root)
 except FileNotFoundError:return None
 with open(root+'/stat') as f:fields=f.read().rsplit(')',1)[1].split()
 checked(len(fields)>19 and fields[19].isdigit(),'process_stat')
 if fields[0]=='Z':raise RuntimeError('process_zombie_pending_reap')
 with open(root+'/status') as f:status=f.read()
 ids={}
 for label in ('Uid','Gid'):
  found=re.search(r'^'+label+r':\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*$',status,re.M)
  checked(found is not None,'process_credentials')
  ids[label.lower()+'s']=[int(x) for x in found.groups()]
 with open(root+'/cmdline','rb') as f:argv=[x.decode('utf-8','strict') for x in f.read(4096).split(b'\0') if x]
 with open(root+'/attr/current') as f:context=f.read().rstrip('\0\n')
 return dict(pid=pid,start_ticks=fields[19],boot_id=boot(),argv=argv,context=context,**ids)
def process_valid(value):
 full=[APP_PROCESS,'/system/bin',MAIN,'--fixed-settings-test',RECORD['signer'],str(RECORD['companion_uid'])]
 return value is not None and value['argv'] in (full,[MAIN]) and value['uids']==[1000]*4 and value['gids']==[1000]*4 and value['context']=='u:r:system_app:s0' and value['boot_id']==RECORD['boot_id']
checked(boot()==RECORD['boot_id'],'boot_changed')
checked(callable(globals().get('publish')),'agentbus_supervisor_required')
ability_sha=hashlib.sha256(open('/system/priv-app/ability/ability.apk','rb').read()).hexdigest()
checked(ability_sha in ALLOWED_ABILITY_SHA256S and ability_sha==ABILITY_SHA256,'ability_changed')
checked(hashlib.sha256(open(APP_PROCESS,'rb').read()).hexdigest()==APP_PROCESS_SHA256,'app_process_changed')
directory=os.open(REMOTE_DIR,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
directory_stat=os.fstat(directory)
checked(directory_stat.st_uid==1000 and stat.S_ISDIR(directory_stat.st_mode),'code_cache_owner')
canonical=os.path.realpath(REMOTE_DIR)
checked(canonical in REMOTE_ALIASES,'code_cache_symlink')
if RECORD.get('code_cache_realpath') is not None:
 checked(canonical==RECORD['code_cache_realpath'],'code_cache_alias_changed')
out['code_cache_realpath']=canonical;publish(out)
'''


def worker_prefix(record):
    validate_record(record)
    values = dict(RECORD=record, APP_PROCESS=APP_PROCESS, MAIN=MAIN, REMOTE_DIR=REMOTE_DIR,
                  REMOTE_ALIASES=REMOTE_ALIASES,
                  APP_PROCESS_SHA256=APP_PROCESS_SHA256,
                  ABILITY_SHA256=record['ability_sha256'],
                  ALLOWED_ABILITY_SHA256S=recovery.ALLOWED_APK_SHA256S,
                  MAX_OUTPUT_BYTES=MAX_OUTPUT_BYTES)
    return ''.join(key + '=' + repr(value) + '\n' for key, value in values.items()) + WORKER_COMMON


def build_start_worker(record, payload, environment):
    require(hashlib.sha256(payload).hexdigest() == record['dex_sha256'], 'Worker DEX hash mismatch')
    compressed = gzip.compress(payload, compresslevel=9, mtime=0)
    return worker_prefix(record) + '\nPAYLOAD=' + repr(base64.b64encode(compressed).decode()) + '\nENV=' + repr(environment) + '\n' + r'''
import gzip
out['operation']='task_host_start'
out['files']={}
out['host_spawned']=False
publish(out)
try:
 data=gzip.decompress(base64.b64decode(PAYLOAD))
 checked(hashlib.sha256(data).hexdigest()==RECORD['dex_sha256'],'dex_payload_changed')
 for key,name,mode in (('dex',RECORD['dex_name'],0o400),('result',RECORD['result_name'],0o600)):
  fd=os.open(name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,mode,dir_fd=directory)
  try:
   os.fchmod(fd,mode)
   info=metadata(os.fstat(fd))
   out['files'][key]=info;publish(out)
   checked(info['uid']==1000 and info['gid']==1000 and info['mode']==mode,'created_file_owner')
   if key=='dex':
    pos=0
    while pos<len(data):pos+=os.write(fd,data[pos:])
    os.fsync(fd)
  finally:os.close(fd)
 out['phase']='files_created';publish(out)
 dexfd=os.open(RECORD['dex_name'],os.O_RDONLY|os.O_NOFOLLOW,dir_fd=directory)
 try:
  checked(match_file(metadata(os.fstat(dexfd)),out['files']['dex']),'dex_inode_changed')
  checked(hashlib.sha256(os.read(dexfd,len(data)+1)).hexdigest()==RECORD['dex_sha256'],'dex_readback')
 finally:os.close(dexfd)
 resultfd=os.open(RECORD['result_name'],os.O_WRONLY|os.O_NOFOLLOW,dir_fd=directory)
 try:
  checked(match_file(metadata(os.fstat(resultfd)),out['files']['result']),'result_inode_changed')
  def child_identity():
   signal.alarm(0)
   os.setresgid(1000,1000,1000);os.setresuid(1000,1000,1000)
  # No PIPE/communicate: this worker exits quickly while the host has its own
  # fixed deadline. stderr is discarded; stdout contains the host's JSON only.
  child=subprocess.Popen([APP_PROCESS,'/system/bin',MAIN,'--fixed-settings-test',RECORD['signer'],str(RECORD['companion_uid'])],env=ENV,preexec_fn=child_identity,start_new_session=True,stdin=subprocess.DEVNULL,stdout=resultfd,stderr=subprocess.DEVNULL,close_fds=True)
  out['host_spawned']=True;out['spawned_pid']=child.pid;publish(out)
  deadline=time.monotonic()+4
  current=None
  while time.monotonic()<deadline:
   current=observed(child.pid)
   if process_valid(current):break
   if child.poll() is not None:break
   time.sleep(.05)
  checked(process_valid(current),'host_identity_unverified')
  out['process']=current;out['phase']='host_detached';out['detached']=True;publish(out)
 finally:os.close(resultfd)
except BaseException as error:
 out['operation_error']=type(error).__name__
 out['cleanup_verified']=False
finally:
 os.close(directory)
 publish(out)
'''


def build_collect_worker(record, externally_exited):
    validate_process(record.get('process'), record)
    require(set(record.get('files', {})) == {'dex', 'result'}, 'Owned file evidence is incomplete')
    for key, mode in (('dex', 0o400), ('result', 0o600)):
        require(file_matches(record['files'][key], record['files'][key])
                and record['files'][key]['mode'] == mode, 'Invalid owned file metadata')
    require(type(externally_exited) is bool, 'External exit evidence is missing')
    return worker_prefix(record) + RESULT_FILTER + '\nEXTERNALLY_EXITED=' + repr(externally_exited) + '\n' + r'''
out['operation']='task_host_collect'
out['removed']={}
try:
 current=observed(RECORD['process']['pid'])
 if current is not None:
  checked(current==RECORD['process'],'host_identity_changed')
  out['status']='pending';out['process']=current
 else:
  # hidepid may make an existing process invisible in this worker's domain.
  # An independent ADB-shell ps snapshot must also prove absence first.
  checked(EXTERNALLY_EXITED,'exit_not_independently_verified')
  out['host_exited']=True
  # Verify both owned inodes before reading or unlinking either one.
  for key,name in (('dex',RECORD['dex_name']),('result',RECORD['result_name'])):
   found=metadata(os.stat(name,dir_fd=directory,follow_symlinks=False))
   checked(match_file(found,RECORD['files'][key]),'owned_file_changed')
  fd=os.open(RECORD['result_name'],os.O_RDONLY|os.O_NOFOLLOW,dir_fd=directory)
  try:
   checked(match_file(metadata(os.fstat(fd)),RECORD['files']['result']),'result_inode_changed')
   checked(os.fstat(fd).st_size<=MAX_OUTPUT_BYTES,'result_too_large')
   raw=b''
   while len(raw)<=MAX_OUTPUT_BYTES:
    part=os.read(fd,min(8192,MAX_OUTPUT_BYTES+1-len(raw)))
    if not part:break
    raw+=part
   out['final']=filtered_result(raw,RECORD)
  finally:os.close(fd)
  publish(out)
  # A result without a final bounded JSON remains for investigation; no arbitrary
  # stdout or guessed process exit is accepted as rollback evidence.
  for key,name in (('dex',RECORD['dex_name']),('result',RECORD['result_name'])):
   checked(match_file(metadata(os.stat(name,dir_fd=directory,follow_symlinks=False)),RECORD['files'][key]),'unlink_identity_changed')
   os.unlink(name,dir_fd=directory)
   out['removed'][key]=True;publish(out)
  out['status']='collected';out['owned_files_removed']=True
except BaseException as error:
 out['operation_error']=type(error).__name__
 out['cleanup_verified']=False
finally:
 os.close(directory)
 publish(out)
'''


def permissions_and_adb():
    permissions = module('task_host_permissions', 'companion-permissions.py')
    require(permissions.ADB[-1] == recovery.SERIAL, 'Task host requires the exact authorized USB serial')
    return permissions, recovery.Adb(permissions.ADB[0], recovery.SERIAL)


def strict_baseline(adb, backup):
    checked = recovery.verify_backup(adb, backup)
    require(checked['original_process_still_alive'] and checked['system_server_unchanged'],
            'Original native process changed; retain evidence and review')
    return checked


def runtime_environment(permissions, record):
    result = dict(CLASSPATH=REMOTE_DIR + '/' + record['dex_name'], ANDROID_ROOT='/system',
                  ANDROID_DATA='/data', PATH='/system/bin', ANDROID_ART_ROOT='/apex/com.android.art',
                  ANDROID_I18N_ROOT='/apex/com.android.i18n', ANDROID_TZDATA_ROOT='/apex/com.android.tzdata')
    for key in ('BOOTCLASSPATH', 'DEX2OATBOOTCLASSPATH', 'SYSTEMSERVERCLASSPATH'):
        value = permissions.shell('printenv', key)
        require(value and all(part.startswith('/') for part in value.split(':')),
                'Android runtime classpath is unavailable')
        result[key] = value
    return result


def external_process(adb, record):
    """Read-only shell view avoids treating worker hidepid ENOENT as exit."""
    pid = record['process']['pid']
    rows = adb.shell('ps', '-A', '-o', 'PID').splitlines()
    require(rows and rows[0].strip() == 'PID', 'Process inventory is unavailable')
    require(all(row.strip().isdigit() for row in rows[1:] if row.strip()), 'Unexpected process inventory')
    pids = {int(row.strip()) for row in rows[1:] if row.strip()}
    require(record['system_server_pid'] in pids and record['ability_pid'] in pids,
            'Independent process inventory does not see original system processes')
    if pid not in pids:
        return None
    root = '/proc/' + str(pid)
    fields = adb.shell('cat', root + '/stat').rsplit(')', 1)[1].split()
    require(len(fields) > 19 and fields[19].isdigit() and fields[0] != 'Z',
            'Host is not a verifiable live process')
    status = adb.shell('cat', root + '/status')
    ids = {}
    for label in ('Uid', 'Gid'):
        match = re.search(r'^' + label + r':\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*$', status, re.M)
        require(match is not None, 'Host credentials are unavailable')
        ids[label.lower() + 's'] = [int(value) for value in match.groups()]
    value = dict(pid=pid, start_ticks=fields[19],
                 boot_id=adb.shell('cat', '/proc/sys/kernel/random/boot_id'),
                 argv=[part for part in adb.shell('cat', root + '/cmdline').split('\0') if part],
                 context=adb.shell('cat', root + '/attr/current').rstrip('\0\n'), **ids)
    validate_process(value, record)
    require(same_process(value, record['process']), 'Host PID was reused or its identity changed')
    return value


def provider_status(adb, record):
    text = adb.shell('content', 'call', '--uri', PROVIDER, '--method', STATUS_METHOD)
    require(len(text.encode()) <= 40 * 1024 and 'result=' in text,
            'Research companion did not provide authenticated host status')
    value, _ = json.JSONDecoder().raw_decode(text.split('result=', 1)[1].lstrip())
    require(isinstance(value, dict) and value.get('ok') is True and value.get('version') == 1
            and value.get('pid') == record['process']['pid']
            and value.get('uid') == record['companion_uid']
            and value.get('socket') == SOCKET and value.get('target') == recovery.PACKAGE
            and value.get('apk_sha256') in recovery.ALLOWED_APK_SHA256S
            and value.get('apk_sha256') == record['ability_sha256']
            and value.get('mode') == 'fixed_settings_about_test'
            and value.get('arbitrary_tasks_enabled') is False and value.get('running') is True,
            'Research socket is not the recorded fixed-test host')
    require(value.get('active_task_count') == 0 and not value.get('test'),
            'Host already contains a task; automatic start is forbidden')
    return {key: value[key] for key in ('ok', 'version', 'pid', 'uid', 'socket', 'target',
                                       'apk_sha256', 'mode', 'arbitrary_tasks_enabled', 'running', 'active_task_count')}


def preserve_worker(permissions, record_path, record, code, operation):
    def save(proof):
        record[operation + '_proof'] = proof
        if operation == 'start':
            for key in ('process', 'files', 'spawned_pid', 'host_spawned', 'code_cache_realpath'):
                if key in proof:
                    record[key] = proof[key]
        recovery.private_json(record_path, record)
    try:
        proof = permissions.run_bounded_android_worker(code, record['system_server_pid'], save,
                                                       worker_context='agentbus')
        save(proof)
        return proof
    except BaseException:
        # The shared worker stores its result even when a later transport guard
        # fails. Preserve only this invocation's matching proof, never an older one.
        last = permissions.STATE / 'last-worker-result.json'
        try:
            proof = json.loads(last.read_text(encoding='utf-8'))
            if proof.get('worker_code_sha256') == hashlib.sha256(code.encode()).hexdigest():
                save(proof)
        except (OSError, ValueError):
            pass
        raise


def start(backup, apk):
    payload = read_payload()
    permissions, adb = permissions_and_adb()
    backup = backup.resolve(strict=True)
    before = strict_baseline(adb, backup)
    manifest = recovery.load_backup(backup)
    baseline = readiness.display_baseline(adb)
    companion = permissions.inspect(apk)
    require(companion.get('signature_verified') is True, 'Signed companion identity is not verified')
    permissions.guard(native=True)
    require(not any(line.split()[-1:] == ['@' + SOCKET]
                    for line in adb.shell('cat', '/proc/net/unix').splitlines()),
            'A fixed-test socket already exists; do not replace another host')
    identity = uuid.uuid4().hex
    record = dict(schema=SCHEMA, id=identity, status='preparing', backup=str(backup),
                  serial=recovery.SERIAL,
                  fingerprint=manifest['snapshot']['identity']['ro.build.fingerprint'],
                  ability_sha256=manifest['original_sha256'], main=MAIN, socket=SOCKET,
                  signer=permissions.signer(apk), companion_uid=companion['uid'],
                  dex_sha256=DEX_SHA256, boot_id=before['current_process']['boot_id'],
                  ability_pid=before['current_process']['pid'],
                  system_server_pid=int(permissions.shell('pidof', 'system_server')),
                  dex_name='stepx-native-gui-task-' + identity + '.dex',
                  result_name='stepx-native-gui-task-' + identity + '.result.jsonl',
                  code_cache_realpath=None,
                  display_baseline=baseline, rollback_verified=False,
                  execution_verified=False, scope='independent_fixed_test_host')
    path = backup / ('task-host-' + identity + '.json')
    require(not path.exists(), 'Unique host record already exists')
    recovery.private_json(path, record)
    try:
        code = build_start_worker(record, payload, runtime_environment(permissions, record))
        proof = preserve_worker(permissions, path, record, code, 'start')
        require(proof.get('detached') is True and 'operation_error' not in proof and 'error' not in proof,
                'Host launch not verified; record retained')
        validate_process(record.get('process'), record)
        strict_baseline(adb, backup)
        require(external_process(adb, record) is not None, 'Host exited before endpoint verification')
        # The companion checks Unix SO_PEERCRED and the installed signer. Retry
        # only read-only status while the bounded host initializes its SDK.
        deadline = time.monotonic() + 12
        while True:
            try:
                record['socket_status'] = provider_status(adb, record)
                break
            except (RuntimeError, ValueError, KeyError):
                if time.monotonic() >= deadline:
                    raise RuntimeError('Host socket identity/readiness unverified; record retained') from None
                time.sleep(.25)
        require(external_process(adb, record) is not None, 'Host exited during endpoint verification')
        require(readiness.display_baseline(adb) == baseline, 'Host initialization changed idle display baseline')
        record['status'] = 'ready'
        recovery.private_json(path, record)
        return dict(ok=True, status='ready', record=str(path), process=record['process'],
                    tasks_started_automatically=0, execution_verified=False, rollback_verified=False)
    except BaseException as error:
        record.update(status='launch_unverified', error_class=type(error).__name__, rollback_verified=False)
        recovery.private_json(path, record)
        raise RuntimeError('Host launch unverified; preserve ' + str(path)) from None


def load_record(record_path):
    record_path = record_path.resolve(strict=True)
    record = validate_record(json.loads(record_path.read_text(encoding='utf-8')))
    require(record_path.parent == Path(record['backup']).resolve(strict=True),
            'Record is outside its recovery directory')
    require(record_path.name == 'task-host-' + record['id'] + '.json', 'Unexpected record filename')
    validate_process(record.get('process'), record)
    return record_path, record


def collected_evidence(record):
    proof = record.get('collect_proof', {})
    require(proof.get('operation') == 'task_host_collect' and proof.get('status') == 'collected'
            and proof.get('host_exited') is True and proof.get('owned_files_removed') is True
            and proof.get('removed') == {'dex': True, 'result': True}
            and proof.get('worker_exit_code') == 0
            and 'operation_error' not in proof and 'error' not in proof,
            'No complete owned-file collection proof is available')
    final = filtered_result(json.dumps(proof.get('final')).encode(), record)
    return proof, final


def finalize_collection(adb, record_path, record):
    """Retry observable postconditions only; no worker, file deletion or task IPC."""
    proof, final = collected_evidence(record)
    try:
        after = strict_baseline(adb, record['backup'])
        require(adb.shell('cat', '/proc/sys/kernel/random/boot_id') == record['boot_id'],
                'Phone rebooted; recovery verification requires review')
        require(external_process(adb, record) is None, 'Recorded host is still visible')
        baseline = readiness.display_baseline(adb)
        require(baseline == record['display_baseline'], 'Original idle display baseline was not restored')
        absent = not any(line.split()[-1:] == ['@' + SOCKET]
                         for line in adb.shell('cat', '/proc/net/unix').splitlines())
        require(absent, 'A task socket remains; global cleanup is unverified')
        result = dict(ok=final['ok'], status='collected', record=str(record_path),
                      final=final, owned_files_removed=True, host_exited=True,
                      original_files_verified=after['immutable_files_verified'],
                      original_process_unchanged=True, system_server_unchanged=True,
                      display_baseline_restored=True, endpoint_absent=True,
                      verification_only=True, global_rollback_verified=False,
                      rollback_verified=False, native_behavior_regression_verified=False)
        record.update(status='collected', collection=result)
        recovery.private_json(record_path, record)
        return result
    except Exception as error:
        record.update(status='collection_verification_pending',
                      finalization_error_class=type(error).__name__, rollback_verified=False)
        recovery.private_json(record_path, record)
        raise


def collect(record_path, finalize_only=False):
    record_path, record = load_record(record_path)
    permissions, adb = permissions_and_adb()
    # A prior worker may have already removed both files before an independent
    # display/transport check failed. Never run that destructive stage again.
    if finalize_only or record.get('collect_proof', {}).get('owned_files_removed') is True:
        return finalize_collection(adb, record_path, record)
    strict_baseline(adb, record['backup'])
    require(adb.shell('cat', '/proc/sys/kernel/random/boot_id') == record['boot_id'],
            'Phone rebooted; automatic collection is not authorized')
    permissions.guard(native=True)
    code = build_collect_worker(record, external_process(adb, record) is None)
    proof = preserve_worker(permissions, record_path, record, code, 'collect')
    if proof.get('status') == 'pending' and 'operation_error' not in proof and 'error' not in proof:
        record['status'] = 'pending'
        recovery.private_json(record_path, record)
        return dict(ok=True, status='pending', record=str(record_path),
                    process=record['process'], rollback_verified=False)
    require(proof.get('status') == 'collected' and proof.get('owned_files_removed') is True
            and proof.get('removed') == {'dex': True, 'result': True}
            and 'operation_error' not in proof and 'error' not in proof,
            'Collection unverified; preserve ' + str(record_path))
    return finalize_collection(adb, record_path, record)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    launch = commands.add_parser('start')
    launch.add_argument('--backup', type=Path, required=True)
    launch.add_argument('--apk', type=Path, required=True)
    gather = commands.add_parser('collect')
    gather.add_argument('--record', type=Path, required=True)
    gather.add_argument('--finalize-only', action='store_true', help='Read-only recovery verification after owned files were collected')
    args = parser.parse_args()
    backup = args.backup if args.command == 'start' else args.record.parent
    # Local lock only, preventing two invocations of this runner racing its record.
    with file_lock(backup / '.native-gui-task-host.lock'):
        print(json.dumps(start(args.backup, args.apk) if args.command == 'start'
                         else collect(args.record, args.finalize_only), ensure_ascii=False))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps(dict(ok=False, error=str(error), rollback_verified=False), ensure_ascii=False), file=sys.stderr)
        raise SystemExit(1)
