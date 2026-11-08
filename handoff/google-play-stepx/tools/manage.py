from common import baseline_matches, DEVICE_KEY
#!/usr/bin/env python3
"""Explicit workflow steps; invoking --help or preflight never installs anything."""
import argparse
import base64
from contextlib import contextmanager, redirect_stdout
from datetime import datetime
import hashlib
import importlib.util
import io
import json
import re
import shutil
import subprocess
import sys
import time
import urllib.request
import zipfile
from common import ADB, CONFIG, ROOT, STATE, DOWNLOADS, guard, init_state, private_write, root_json, run, save_json, sdk_tool, sdk_command, sha256, shell, verified_fingerprint, operation_lock, OperationBusy
from host_runtime import file_lock

from source_integrity import python_source_sha256

PACKAGES = ['com.google.android.gsf', 'com.google.android.gms', 'com.android.vending']
MONITOR_REVISION = 'google-login-v2-recheck-1'
INSTALL_LABELS = dict(zip(PACKAGES, ['Google 服务框架', 'Google Play 服务', 'Google Play 商店']))
INSTALL_GUIDANCE = '请在手机「设置 → 开发者选项」开启「USB 安装」后重试；程序不会自动修改安装总开关'

# Read-only coverage from the reviewed GmsProbe permission groups embedded in
# module/inject.py (SHA-256 279305289749acada3f9951c816d7e9f00367dce4331d4396c04554e4685fbf8).
# This does not authorize or grant anything; it detects stale/legacy receipts.
REVIEWED_PERMISSIONS = {
    'com.google.android.gms': (
        'android.permission.ACCESS_BROADCAST_RESPONSE_STATS',
        'android.permission.ACCESS_CACHE_FILESYSTEM',
        'android.permission.ACCESS_CONTEXT_HUB',
        'android.permission.ACCESS_FPS_COUNTER',
        'android.permission.ACCESS_NETWORK_CONDITIONS',
        'android.permission.ACCESS_VIBRATOR_STATE',
        'android.permission.ACTIVITY_EMBEDDING',
        'android.permission.ALLOCATE_AGGRESSIVE',
        'android.permission.BACKUP',
        'android.permission.BLUETOOTH_PRIVILEGED',
        'android.permission.BROADCAST_CLOSE_SYSTEM_DIALOGS',
        'android.permission.CALL_AUDIO_INTERCEPTION',
        'android.permission.CALL_PRIVILEGED',
        'android.permission.CAPTURE_AUDIO_HOTWORD',
        'android.permission.CAPTURE_AUDIO_OUTPUT',
        'android.permission.CHANGE_COMPONENT_ENABLED_STATE',
        'android.permission.CHANGE_DEVICE_IDLE_TEMP_WHITELIST',
        'android.permission.CHECK_REMOTE_LOCKSCREEN',
        'android.permission.COMPANION_APPROVE_WIFI_CONNECTIONS',
        'android.permission.CONFIGURE_FACTORY_RESET_PROTECTION',
        'android.permission.CONNECTIVITY_USE_RESTRICTED_NETWORKS',
        'android.permission.CONTROL_DISPLAY_SATURATION',
        'android.permission.CONTROL_INCALL_EXPERIENCE',
        'android.permission.CONTROL_KEYGUARD_SECURE_NOTIFICATIONS',
        'android.permission.DISPATCH_PROVISIONING_MESSAGE',
        'android.permission.DOMAIN_VERIFICATION_AGENT',
        'android.permission.DUMP',
        'android.permission.EMERGENCY_INSTALL_PACKAGES',
        'android.permission.FORCE_STOP_PACKAGES',
        'android.permission.GET_APP_OPS_STATS',
        'android.permission.GET_ROLE_HOLDERS',
        'android.permission.INSTALL_LOCATION_TIME_ZONE_PROVIDER_SERVICE',
        'android.permission.INTENT_FILTER_VERIFICATION_AGENT',
        'android.permission.INTERACT_ACROSS_USERS',
        'android.permission.INVOKE_CARRIER_SETUP',
        'android.permission.LOCAL_MAC_ADDRESS',
        'android.permission.LOCATION_BYPASS',
        'android.permission.LOCATION_HARDWARE',
        'android.permission.LOCK_DEVICE',
        'android.permission.LOG_COMPAT_CHANGE',
        'android.permission.MANAGE_ADVANCED_PROTECTION_MODE',
        'android.permission.MANAGE_FACTORY_RESET_PROTECTION',
        'android.permission.MANAGE_GAME_ACTIVITY',
        'android.permission.MANAGE_GAME_MODE',
        'android.permission.MANAGE_INTRUSION_DETECTION_STATE',
        'android.permission.MANAGE_ROLLBACKS',
        'android.permission.MANAGE_SECURE_LOCK_DEVICE',
        'android.permission.MANAGE_SOUND_TRIGGER',
        'android.permission.MANAGE_SUBSCRIPTION_PLANS',
        'android.permission.MANAGE_TIME_AND_ZONE_DETECTION',
        'android.permission.MANAGE_USB',
        'android.permission.MANAGE_USERS',
        'android.permission.MANAGE_VOICE_KEYPHRASES',
        'android.permission.MANAGE_WIFI_INTERFACES',
        'android.permission.MANAGE_WIFI_NETWORK_SELECTION',
        'android.permission.MASTER_CLEAR',
        'android.permission.MEDIA_CONTENT_CONTROL',
        'android.permission.MODIFY_AUDIO_ROUTING',
        'android.permission.MODIFY_DAY_NIGHT_MODE',
        'android.permission.MODIFY_DEFAULT_AUDIO_EFFECTS',
        'android.permission.MODIFY_NETWORK_ACCOUNTING',
        'android.permission.MODIFY_PHONE_STATE',
        'android.permission.NOTIFY_PENDING_SYSTEM_UPDATE',
        'android.permission.OBSERVE_GRANT_REVOKE_PERMISSIONS',
        'android.permission.OBSERVE_ROLE_HOLDERS',
        'android.permission.OVERRIDE_WIFI_CONFIG',
        'android.permission.PACKAGE_USAGE_STATS',
        'android.permission.PREPARE_FACTORY_RESET',
        'android.permission.PROVIDE_DEFAULT_ENABLED_CREDENTIAL_SERVICE',
        'android.permission.PROVIDE_REMOTE_CREDENTIALS',
        'android.permission.PROVIDE_RESOLVER_RANKER_SERVICE',
        'android.permission.PROVIDE_TRUST_AGENT',
        'android.permission.READ_COMPAT_CHANGE_CONFIG',
        'android.permission.READ_DREAM_STATE',
        'android.permission.READ_DROPBOX_DATA',
        'android.permission.READ_INTRUSION_DETECTION_STATE',
        'android.permission.READ_LOGS',
        'android.permission.READ_NETWORK_USAGE_HISTORY',
        'android.permission.READ_OEM_UNLOCK_STATE',
        'android.permission.READ_PRIVILEGED_PHONE_STATE',
        'android.permission.READ_SYSTEM_GRAMMATICAL_GENDER',
        'android.permission.READ_WIFI_CREDENTIAL',
        'android.permission.REAL_GET_TASKS',
        'android.permission.REBOOT',
        'android.permission.RECEIVE_DATA_ACTIVITY_CHANGE',
        'android.permission.RECOVERY',
        'android.permission.RECOVER_KEYSTORE',
        'android.permission.REMOTE_DISPLAY_PROVIDER',
        'android.permission.RENOUNCE_PERMISSIONS',
        'android.permission.REQUEST_COMPANION_PROFILE_COMPUTER',
        'android.permission.REQUEST_COMPANION_SELF_MANAGED',
        'android.permission.RESET_PASSWORD',
        'android.permission.SATELLITE_COMMUNICATION',
        'android.permission.SCHEDULE_PRIORITIZED_ALARM',
        'android.permission.SCORE_NETWORKS',
        'android.permission.SEND_SAFETY_CENTER_UPDATE',
        'android.permission.SEND_SMS_NO_CONFIRMATION',
        'android.permission.SETUP_FSVERITY',
        'android.permission.SET_BIOMETRIC_DIALOG_ADVANCED',
        'android.permission.SET_LOW_POWER_STANDBY_PORTS',
        'android.permission.SET_TIME',
        'android.permission.SET_TIME_ZONE',
        'android.permission.START_ACTIVITIES_FROM_BACKGROUND',
        'android.permission.START_TASKS_FROM_RECENTS',
        'android.permission.STATUS_BAR',
        'android.permission.SUBSCRIBE_TO_KEYGUARD_LOCKED_STATE',
        'android.permission.SUBSTITUTE_NOTIFICATION_APP_NAME',
        'android.permission.SUBSTITUTE_SHARE_TARGET_APP_NAME_AND_ICON',
        'android.permission.TETHER_PRIVILEGED',
        'android.permission.THREAD_NETWORK_PRIVILEGED',
        'android.permission.TURN_SCREEN_ON',
        'android.permission.UPDATE_APP_OPS_STATS',
        'android.permission.UPDATE_DEVICE_STATS',
        'android.permission.UPDATE_FONTS',
        'android.permission.USER_ACTIVITY',
        'android.permission.UWB_PRIVILEGED',
        'android.permission.WRITE_EMBEDDED_SUBSCRIPTIONS',
        'android.permission.WRITE_GSERVICES',
        'android.permission.WRITE_SECURE_SETTINGS',
        'android.permission.WRITE_VERIFICATION_STATE_E2EE_CONTACT_KEYS',
    ),
    'com.android.vending': (
        'android.permission.ALLOCATE_AGGRESSIVE',
        'android.permission.BACKUP',
        'android.permission.BATTERY_STATS',
        'android.permission.CHANGE_COMPONENT_ENABLED_STATE',
        'android.permission.CHANGE_DEVICE_IDLE_TEMP_WHITELIST',
        'android.permission.CHANGE_OVERLAY_PACKAGES',
        'android.permission.CLEAR_APP_CACHE',
        'android.permission.CONNECTIVITY_INTERNAL',
        'android.permission.DELETE_PACKAGES',
        'android.permission.DUMP',
        'android.permission.FORCE_STOP_PACKAGES',
        'android.permission.GET_ACCOUNTS_PRIVILEGED',
        'android.permission.GET_APP_OPS_STATS',
        'android.permission.INSTALL_PACKAGES',
        'android.permission.INTERACT_ACROSS_USERS',
        'android.permission.LOADER_USAGE_STATS',
        'android.permission.MANAGE_CLOUDSEARCH',
        'android.permission.MANAGE_ROLLBACKS',
        'android.permission.MANAGE_USERS',
        'android.permission.OVERRIDE_COMPAT_CHANGE_CONFIG_ON_RELEASE_BUILD',
        'android.permission.PACKAGE_USAGE_STATS',
        'android.permission.PACKAGE_VERIFICATION_AGENT',
        'android.permission.READ_PRIVILEGED_PHONE_STATE',
        'android.permission.READ_RUNTIME_PROFILES',
        'android.permission.READ_SYSTEM_GRAMMATICAL_GENDER',
        'android.permission.REAL_GET_TASKS',
        'android.permission.REBOOT',
        'android.permission.SEND_DEVICE_CUSTOMIZATION_READY',
        'android.permission.SEND_SAFETY_CENTER_UPDATE',
        'android.permission.SEND_SMS_NO_CONFIRMATION',
        'android.permission.SET_PREFERRED_APPLICATIONS',
        'android.permission.START_ACTIVITIES_FROM_BACKGROUND',
        'android.permission.STATUS_BAR',
        'android.permission.SUBSTITUTE_NOTIFICATION_APP_NAME',
        'android.permission.UPDATE_DEVICE_STATS',
        'android.permission.WRITE_SECURE_SETTINGS',
    ),
}


def preflight():
    guard(native=True)
    result = {
        'fingerprint': verified_fingerprint(),
        'boot_completed': shell('getprop', 'sys.boot_completed'),
        'user_unlocked': 'RUNNING_UNLOCKED' in shell('dumpsys', 'user'),
        'avb': shell('getprop', 'ro.boot.verifiedbootstate'),
        'bootloader_locked': shell('getprop', 'ro.boot.flash.locked'),
        'selinux': shell('getenforce'),
        'google_packages': {p: bool(shell('pm', 'list', 'packages', '--user', '0', p)) for p in PACKAGES},
        'usb_install_master_enabled': shell('settings', 'get', 'secure', 'usb_install_item_switch') == 'switch:1',
    }
    result['install_guidance'] = '' if result['usb_install_master_enabled'] else INSTALL_GUIDANCE
    result['resume_install'] = resumable_install(result['fingerprint'])
    print(json.dumps(result, indent=2))
    return result


def resumable_install(fingerprint=None):
    try:
        baseline=json.loads((STATE/'baseline.json').read_text())
        receipt=json.loads((STATE/'install-progress.json').read_text())
        return (baseline_matches(baseline) and receipt.get('serial')==DEVICE_KEY
                and receipt.get('fingerprint')==baseline.get('fingerprint')==(fingerprint or verified_fingerprint())
                and not any(p in baseline.get('packages',[]) for p in PACKAGES)
                and not (STATE/'boot-install.json').exists())
    except (OSError,ValueError): return False


def require_current_baseline(baseline):
    if not baseline_matches(baseline):
        raise RuntimeError('Wrong device baseline; recovery files were preserved')
    if baseline.get('fingerprint') != verified_fingerprint():
        raise RuntimeError('安装基线记录的固件与当前手机不同；保留备份，不能直接覆盖或沿用旧基线')


def install_gate_path(package):
    if package not in INSTALL_LABELS:
        raise RuntimeError('USB installation permission is limited to the three verified Google packages')
    return STATE / ('usb-install-' + package + '.json')


def finish_install_gate(package):
    path = install_gate_path(package)
    if not path.exists():
        return
    baseline = json.loads(path.read_text())
    key = 'usb_install_item_' + package
    temporary = INSTALL_LABELS[package] + ':1'
    if (not baseline_matches(baseline) or baseline.get('fingerprint') != verified_fingerprint()
            or baseline.get('package') != package or baseline.get('key') != key
            or baseline.get('temporary') != temporary):
        raise RuntimeError('USB 安装恢复记录不匹配，未修改设置或删除记录')
    previous = baseline.get('previous')
    if previous is not None and (not isinstance(previous, str) or len(previous) > 512):
        raise RuntimeError('Invalid saved USB installation setting')
    expected = 'null' if previous is None else previous
    current = shell('settings', 'get', 'secure', key)
    if current not in (temporary, expected):
        raise RuntimeError('USB 安装设置已被其他操作修改，未覆盖；请核对后重试')
    if current != expected:
        if previous is None:
            shell('settings', 'delete', 'secure', key)
        else:
            shell('settings', 'put', 'secure', key, previous)
    if shell('settings', 'get', 'secure', key) != expected:
        raise RuntimeError('USB 安装设置恢复未通过回读，已保留恢复记录')
    path.unlink()


@contextmanager
def temporary_install_gate(package):
    path = install_gate_path(package)
    finish_install_gate(package)  # Recover an interrupted attempt before a new write.
    if shell('settings', 'get', 'secure', 'usb_install_item_switch') != 'switch:1':
        raise RuntimeError(INSTALL_GUIDANCE)
    key = 'usb_install_item_' + package
    previous = shell('settings', 'get', 'secure', key)
    if len(previous) > 512:
        raise RuntimeError('USB installation setting exceeds the recovery bound')
    temporary = INSTALL_LABELS[package] + ':1'
    save_json(path.name, {'serial': DEVICE_KEY, 'fingerprint': verified_fingerprint(),
              'package': package, 'key': key, 'temporary': temporary,
              'previous': None if previous == 'null' else previous,
              'global_switch_changed': False})
    try:
        shell('settings', 'put', 'secure', key, temporary)
        if shell('settings', 'get', 'secure', key) != temporary:
            raise RuntimeError('Google 单应用安装授权回读失败')
        yield
    finally:
        finish_install_gate(package)


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
    with operation_lock():
        return _prepare()


def _prepare():
    current = preflight()
    assert current['user_unlocked'], 'Unlock the phone first'
    if (STATE/'baseline.json').exists():
        assert resumable_install(current['fingerprint']), '现有安装基线需要核对，未覆盖备份或账号数据'
        assert sha256(STATE/'ubuntu_zygote.stock')==CONFIG['stock_sha256'], '原启动文件备份校验失败'
        print('Resuming the same verified installation; original baseline retained.')
        return
    assert not any(current['google_packages'].values()), 'Google packages already exist; use maintenance workflow'
    all_packages = shell('pm', 'list', 'packages', '-u')
    assert all('package:' + p + '\n' not in all_packages + '\n' for p in PACKAGES), 'Prior Google package state exists'
    assert re.findall(r'UserInfo\{(\d+):', shell('pm', 'list', 'users')) == ['0'], 'Only user 0 was validated'
    init_state()
    assert not (STATE / 'baseline.json').exists(), 'Do not overwrite an existing baseline'
    baseline = {'serial': DEVICE_KEY, 'fingerprint': current['fingerprint'],
                'packages': [x.removeprefix('package:') for x in shell('pm', 'list', 'packages').splitlines()],
                'google_core': shell('settings', 'get', 'global', 'cn_gms_core'),
                'autofill': shell('settings', 'get', 'secure', 'autofill_service'),
                'startup_path': shell('getprop', 'persist.openclaw.zygote.path'),
                'start_trigger': shell('getprop', 'sys.openclaw.env.start')}
    assert baseline['startup_path'] == baseline['start_trigger'] == '', 'Non-default startup properties'
    lines = shell('ps', '-A', '-o', 'PID,PPID,NAME').splitlines()
    parent = next(int(x.split()[0]) for x in lines if x.split()[-1] == 'openclaw_env' and x.split()[1] == '1')
    code = '''import os,json,hashlib,base64,stat
h=HOST
assert not os.path.lexists('/usr/local/lib/stepx-gms') and not os.path.lexists('/var/lib/stepx-gms'), 'Adapter already exists'
def read_regular(path):
 fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW)
 with os.fdopen(fd,'rb') as f:
  assert stat.S_ISREG(os.fstat(fd).st_mode), 'Expected regular boot artifact'
  return f.read()
def digest(path):return hashlib.sha256(read_regular(path)).hexdigest()
p='/usr/local/bin/ubuntu_zygote';stock=p+'.stepx-stock-20261021';data=read_regular(p)
if os.path.lexists(stock):
 assert digest(stock)==STOCK_SHA, 'Existing stock backup differs; preserve and stop'
 if hashlib.sha256(data).hexdigest()!=STOCK_SHA:
  companion='/usr/local/lib/stepx-companion';config=json.loads(read_regular(companion+'/config.json'))
  assert config.get('boot_mode')=='standalone' and config.get('hook_path')==p, 'Unknown companion bootstrap'
  assert config.get('serial')==DEVICE and config.get('fingerprint')==FINGERPRINT, 'Companion bootstrap identity differs'
  assert digest(p)==config.get('hook_sha256'), 'Companion launcher changed'
  for name in ('chain','boot','grant'):
   assert digest(companion+'/'+name+'.py')==config[name+'_sha256'], 'Companion artifact changed'
 data=read_regular(stock)
r={'stock':base64.b64encode(data).decode(),'stock_sha256':hashlib.sha256(data).hexdigest(),'files':{},'skipped_files':{}}
for path in ['/data/system/packages.xml','/data/misc_de/0/apexdata/com.android.permission/access.abx']:
 try:
  with open(h+path,'rb') as f:r['files'][path]=base64.b64encode(f.read()).decode()
 except PermissionError:r['skipped_files'][path]='permission_denied'
 except FileNotFoundError:r['skipped_files'][path]='not_present'
print(json.dumps(r))
'''.replace('HOST', repr('/proc/' + str(parent) + '/root')).replace('STOCK_SHA', repr(CONFIG['stock_sha256'])).replace('DEVICE', repr(DEVICE_KEY)).replace('FINGERPRINT', repr(current['fingerprint']))
    proof = root_json(code)
    assert proof['stock_sha256'] == CONFIG['stock_sha256'], 'Unexpected factory launcher'
    private_write(STATE / 'ubuntu_zygote.stock', base64.b64decode(proof['stock']))
    for path, encoded in proof['files'].items():
        private_write(STATE / ('before-' + path.rsplit('/', 1)[-1]), base64.b64decode(encoded))
    baseline['metadata_backups'] = {path: 'saved' for path in proof['files']}
    baseline['metadata_backups'].update(proof.get('skipped_files', {}))
    save_json('baseline.json', baseline)
    save_json('install-progress.json', {'serial':DEVICE_KEY,'fingerprint':current['fingerprint'],'completed':[]})
    print('Original launcher verified and saved privately. No Google APK installed yet.')
    if proof.get('skipped_files'):
        print('系统限制了部分只读诊断备份；已记录缺项，不修改 SELinux。原启动文件的完整备份已通过校验。')


def install_apks():
    guard(native=True)
    with operation_lock(), file_lock(STATE / 'install.lock'):
        return _install_apks()


def _install_apks():
    # Restore every interrupted package gate before validating this attempt.
    # Missing downloads or a newer already-installed APK must not leave a
    # different package's temporary installation permission enabled forever.
    for package in PACKAGES:
        finish_install_gate(package)
    baseline = json.loads((STATE / 'baseline.json').read_text())
    require_current_baseline(baseline)
    assert all(p not in baseline['packages'] for p in PACKAGES)
    assert resumable_install(), '缺少本次安装记录；未覆盖已有 Google 组件'
    check_apks()
    for package in PACKAGES:
        meta = next(x for x in CONFIG['apks'] if x['package'] == package)
        if shell('pm','list','packages','--user','0',package):
            paths=shell('pm','path','--user','0',package).splitlines()
            assert len(paths)==1 and paths[0].startswith('package:/data/app/'), '已有 Google 组件位置不同，未覆盖'
            current=shell('sha256sum',paths[0][8:]).split()[0]
            assert current==meta['sha256'], '已有 Google 组件版本不同，已保留；请使用检测/修复'
            print('Preserved verified installed package: '+package,flush=True)
            continue
        # These provider packages must be discoverable by clients that assume a
        # preinstalled GMS stack. This does not mark them SYSTEM/PRIVILEGED.
        with temporary_install_gate(package):
            result = run(ADB + ['install', '--user', '0', '--force-queryable', str(DOWNLOADS / meta['filename'])], timeout=180)
            assert 'Success' in result.stdout, 'Installation not confirmed: ' + package
        paths=shell('pm','path','--user','0',package).splitlines()
        assert len(paths)==1 and paths[0].startswith('package:/data/app/') and shell('sha256sum',paths[0][8:]).split()[0]==meta['sha256'], 'Installed APK read-back mismatch'
        receipt=json.loads((STATE/'install-progress.json').read_text())
        receipt['completed']=list(dict.fromkeys(receipt.get('completed',[])+[package]))
        save_json('install-progress.json',receipt)
        print('Installed and verified ' + package, flush=True)
    shell('settings', 'put', 'global', 'cn_gms_core', '1')
    for package in PACKAGES:
        shell('pm', 'grant', package, 'android.permission.INTERACT_ACROSS_USERS')
    shell('pm', 'grant', 'com.google.android.gms', 'android.permission.WRITE_SECURE_SETTINGS')
    for package in ['com.google.android.gms', 'com.android.vending']:
        shell('cmd', 'appops', 'set', package, 'SYSTEM_EXEMPT_FROM_POWER_RESTRICTIONS', 'allow')
    print('APKs installed. Do not log in before adapter verification and an unlogged reboot test.')
    print('登录后 Play 服务或商店可能自动更新；若提示组件版本变化或启动闪退，请使用「修复 Google 服务」重新校验权限，账号和应用数据会保留。')


def current_permission_checks(checks, dumps=None):
    """An old boot receipt is not evidence after a Play services update."""
    if dumps is None:
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


def requested_permissions(dump):
    lines = dump.splitlines()
    for index, line in enumerate(lines):
        if line.strip() != 'requested permissions:':
            continue
        depth = len(line) - len(line.lstrip())
        names = set()
        for value in lines[index + 1:]:
            if value.strip() and len(value) - len(value.lstrip()) <= depth:
                break
            name = value.strip()
            if re.fullmatch(r'[A-Za-z0-9_.]+', name):
                names.add(name)
        return names
    return None


def permission_coverage(java_result, dumps, registered=None):
    """Check current requests against the reviewed list, including old receipts
    that never recorded SKIP lines. No permissions are granted by status().
    """
    checked = set()
    for line in java_result.splitlines():
        match = re.fullmatch(r'check (\S+) (\S+)=0', line)
        if match:
            checked.add((match[1], match[2]))
    uncovered, unknown = [], []
    for package, allowed in REVIEWED_PERMISSIONS.items():
        requested = requested_permissions(dumps.get(package, ''))
        if requested is None:
            unknown.append(package)
            continue
        required = set(allowed) & requested
        if registered is not None:
            required &= registered
        uncovered.extend(package + ':' + permission for permission in required
                         if (package, permission) not in checked)
    return sorted(uncovered), unknown


def registered_permissions(raw):
    """Unavailable Android API permissions are not missing app grants.

    Require a recognizable complete platform inventory before excluding any
    requested permission. Never trust the old receipt's SKIP line alone.
    """
    names = {line.strip().removeprefix('permission:') for line in raw.splitlines()
             if line.strip().startswith('permission:')}
    if ('All Permissions:' not in raw or len(names) < 100
            or not {'android.permission.INTERNET', 'android.permission.MANAGE_USERS',
                    'android.permission.INSTALL_PACKAGES'} <= names):
        raise RuntimeError('Current platform permission definitions could not be verified')
    return names


def component_versions(dumps):
    # dumpsys is already needed for grants. Avoid hashing large APKs every poll.
    return {package: {key: (match[1].strip() if (match := re.search(pattern, data, re.M)) else None)
                      for key, pattern in [('versionCode', r'^\s*versionCode=(\d+)'),
                                           ('versionName', r'^\s*versionName=([^\r\n]+)'),
                                           ('uid', r'^\s*(?:appId|userId)=(\d+)'),
                                           ('lastUpdateTime', r'^\s*lastUpdateTime=([^\r\n]+)')]}
            for package, data in dumps.items()}


def phone_component_identity_matches(phone, versions, timezone=None):
    for package, prefix in [('com.google.android.gms', 'gms'), ('com.android.vending', 'play'),
                            ('com.google.android.gsf', 'gsf')]:
        current = versions.get(package, {})
        if str(phone.get(prefix + '_version')) != current.get('versionCode'):
            return False
        if str(phone.get(prefix + '_uid')) != current.get('uid'):
            return False
        # Both values are produced on the phone in its local timezone. Never
        # interpret dumpsys timestamps using the desktop's possibly different zone.
        if phone.get(prefix + '_updated_local') != current.get('lastUpdateTime'):
            if not timezone:
                return False
            try:
                from zoneinfo import ZoneInfo
                current_seconds = int(datetime.strptime(current.get('lastUpdateTime', ''), '%Y-%m-%d %H:%M:%S')
                                      .replace(tzinfo=ZoneInfo(timezone)).timestamp())
                if current_seconds != phone[prefix + '_updated_at'] // 1000:
                    return False
            except (ValueError, KeyError, TypeError):
                return False
    return True


def runtime_summary(adapter, boot_id, pid, dumps, previous=None, registered=None):
    java_result = adapter.get('java_result', '')
    checks = [x for x in java_result.splitlines() if x.startswith('check ')]
    summary = {k: adapter.get(k) for k in ['status', 'target_pid', 'unix_time', 'detached', 'error']}
    summary.update(current_boot_matches=adapter.get('identity') == boot_id,
                   current_pid_matches=str(adapter.get('target_pid')) == pid,
                   permission_checks=len(checks), all_checked_granted=bool(checks) and all(x.endswith('=0') for x in checks))
    missing = current_permission_checks(checks, dumps)
    uncovered, unknown = permission_coverage(java_result, dumps, registered)
    versions = component_versions(dumps)
    receipt = hashlib.sha256(json.dumps({k: adapter.get(k) for k in
        ('identity', 'target_pid', 'unix_time', 'java_result')}, sort_keys=True).encode()).hexdigest()
    same_receipt = previous and previous.get('receipt_sha256') == receipt
    def previous_version_matches(package):
        old = previous.get('versions', {}).get(package, {})
        # Older desktop observations predate the UID field. Introducing this
        # extra read-back must not itself look like an app update.
        return all(old.get(key) == value for key, value in versions[package].items()
                   if key != 'uid' or key in old)
    changed = [package for package in PACKAGES if same_receipt and not previous_version_matches(package)]
    summary.update(current_permissions_granted=bool(checks) and not missing and not uncovered and not unknown,
                   missing_permissions=missing, newly_declared_permissions=uncovered,
                   permission_coverage_unknown=unknown,
                   updated_components=changed, component_versions=versions,
                   receipt_sha256=receipt)
    summary['permission_coverage_verified'] = registered is not None and not unknown and not uncovered and bool(checks)
    ready = (summary['status'] == 'success' and summary['detached'] is True
             and summary['current_boot_matches'] and summary['current_pid_matches']
             and (len(checks) >= 120 or summary['permission_coverage_verified']) and summary['all_checked_granted']
             and summary['current_permissions_granted'] and not changed and not unknown
             and all(value['versionCode'] for value in versions.values()))
    summary['needs_repair'] = not ready
    if not ready:
        summary['status'] = 'needs_repair'
        summary['repair_command'] = 'manage.py repair-runtime'
        summary['message'] = ('Google 组件已更新或新增权限尚未重新校验；请点击「修复 Google 服务」，账号和应用数据会保留'
                              if changed or uncovered else 'Google 当前运行权限或启动回执未通过校验；请点击「修复 Google 服务」')
    return summary


def phone_recovery_summary(phone, boot_id, pid, dumps, registered, timezone=None):
    """Accept an in-place phone repair only after current independent read-back.

    Old boot last.json is not updated by the phone's package-update listener.
    Its stale permissions must not cause another unnecessary gateway restart.
    """
    if not isinstance(phone, dict) or phone.get('status') != 'ready':
        return None
    if (phone.get('boot_id') != boot_id or str(phone.get('system_server_pid')) != pid
            or phone.get('update_monitor') is not True or type(phone.get('checked_at')) is not int):
        return None
    versions = component_versions(dumps)
    for package, key in [('com.google.android.gms', 'gms_version'), ('com.android.vending', 'play_version')]:
        if str(phone.get(key)) != versions[package]['versionCode']:
            return None
    if type(phone.get('schema')) is int and phone.get('schema') == 2:
        if phone.get('signatures_verified') is not True or phone.get('permissions_verified') is not True:
            return None
        if (phone.get('registered') is not True or phone.get('domain_ready') is not True
                or not phone_component_identity_matches(phone, versions, timezone)):
            return None
        proof = phone.get('permission_result', '')
        if not isinstance(proof, str) or not proof.strip():
            return None
    else:
        # Legacy receipts lack the complete read-back and installation identities.
        # Keep them diagnostic only; never invent permission checks for an update.
        return None
    summary = runtime_summary({'status': 'success', 'identity': boot_id, 'target_pid': int(pid),
                               'detached': True, 'unix_time': phone['checked_at'] // 1000, 'java_result': proof},
                              boot_id, pid, dumps, registered=registered)
    if summary['needs_repair']:
        return None
    summary['recovery_source'] = 'phone_update_monitor'
    summary['phone_runtime_verified'] = True
    summary['detached'] = None  # This is current in-process proof, not a native-worker detach receipt.
    return summary


def status():
    guard()
    init_state()
    result = root_json('''import os,json
p='/var/lib/stepx-gms/last.json'
r={'boot_id':open('/proc/sys/kernel/random/boot_id').read().strip(),'adapter':json.load(open(p)) if os.path.exists(p) else None,'phone_recovery':None,'phone_recovery_read_state':'unavailable'}
try:
 parent=next(p for p in os.listdir('/proc') if p.isdigit() and os.path.exists('/proc/'+p+'/comm') and open('/proc/'+p+'/comm').read().strip()=='openclaw_env')
 q='/proc/'+parent+'/root/data/system/stepx-google-login-runtime.json'
 try:
  with open(q) as f:phone=json.load(f)
 except FileNotFoundError:r['phone_recovery_read_state']='missing'
 else:
  if not isinstance(phone,dict):raise ValueError('invalid phone recovery receipt')
  r.update(phone_recovery=phone,phone_recovery_read_state='readable')
except (OSError,ValueError,StopIteration):pass
print(json.dumps(r))''')
    adapter = result['adapter'] or {}
    save_json('adapter-status.json', result)
    observation = STATE / 'component-observation.json'
    previous = json.loads(observation.read_text()) if observation.exists() else None
    dumps = {package: shell('dumpsys', 'package', package) for package in PACKAGES}
    current_pid = shell('pidof', 'system_server')
    summary = runtime_summary(adapter, result['boot_id'], current_pid, dumps, previous)
    registered = None
    if summary['newly_declared_permissions'] or result.get('phone_recovery'):
        # Only needed for uncovered requests; steady receipts use the existing
        # three dumpsys reads, without repeated APK digest work.
        try:
            registered = registered_permissions(shell('pm', 'list', 'permissions'))
        except (RuntimeError, subprocess.SubprocessError):
            summary['permission_definitions_checked'] = False
            summary['message'] = '当前系统权限定义读取未完成；请重新检测，未修改 Google 权限或应用数据'
        else:
            summary = runtime_summary(adapter, result['boot_id'], current_pid, dumps, previous, registered)
            summary['permission_definitions_checked'] = True
    phone = result.get('phone_recovery') or {}
    timezone = None
    if phone.get('schema') == 2 and not phone_component_identity_matches(phone, component_versions(dumps)):
        # The phone's timezone can change after receipt creation. Only this slow
        # mismatch path needs another read; normal checks compare local strings.
        timezone = shell('getprop', 'persist.sys.timezone')
    if registered is not None:
        summary = phone_recovery_summary(phone, result['boot_id'], current_pid, dumps, registered, timezone) or summary
    summary['phone_recovery'] = {key: phone.get(key) for key in
        ('schema', 'status', 'checked_at', 'retry_after_ms', 'recovery_pending', 'attempt', 'boot_id', 'system_server_pid', 'reason', 'signatures_verified',
         'recheck_supported', 'runtime_revision', 'grant_authorized', 'error_code', 'identity_package')}
    summary['boot_id'] = result['boot_id']
    summary['phone_recovery_read_state'] = result['phone_recovery_read_state']
    supported_schema = type(phone.get('schema')) is int and phone.get('schema') in (1, 2)
    legacy_monitor = supported_schema and (phone.get('schema') == 1
        or (phone.get('schema') == 2 and phone.get('runtime_revision') is None))
    summary['monitor_upgrade_needed'] = (result['phone_recovery_read_state'] == 'missing'
        or (result['phone_recovery_read_state'] == 'readable' and legacy_monitor))
    summary['monitor_schema_unsupported'] = result['phone_recovery_read_state'] == 'readable' and (not supported_schema
        or (phone.get('schema') == 2 and phone.get('runtime_revision') not in (None, MONITOR_REVISION)))
    summary['phone_recovery_current_session'] = phone.get('boot_id') == result['boot_id'] and str(phone.get('system_server_pid')) == current_pid
    summary['phone_recovery_current_components'] = phone_component_identity_matches(phone, component_versions(dumps), timezone)
    if (phone.get('schema') == 2 and summary['phone_recovery_current_session']
            and phone.get('status') != 'ready'):
        summary.update(status='needs_repair', needs_repair=True,
                       message='手机 Google 恢复监听尚未通过权限、登录入口与链接验证；自动维护将等待或复核')
    summary['components_enabled'] = {}
    for package, data in dumps.items():
        user = re.search(r'^\s*User 0:([^\n]*)', data, re.M)
        value = user[1] if user else ''
        enabled = re.search(r'\benabled=(\d+)', value)
        summary['components_enabled'][package] = bool(user and 'installed=true' in value
            and 'hidden=false' in value and 'suspended=false' in value and enabled and enabled[1] in ('0', '1'))
    if not summary['needs_repair']:
        save_json(observation.name, {'receipt_sha256': summary['receipt_sha256'], 'versions': summary['component_versions']})
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    return summary


def owned_runtime():
    """Verify an existing reviewed installation; never adopt an unknown hook."""
    expected = {name: python_source_sha256(ROOT / 'module' / name) for name in ('boot.py', 'inject.py', 'launcher.py')}
    chain = python_source_sha256(ROOT.parents[1] / 'scripts/companion_boot/chain.py')
    code = '''import os,json,hashlib,stat
base='/usr/local/lib/stepx-gms';launcher='/usr/local/bin/ubuntu_zygote'
def read(path):
 fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW)
 with os.fdopen(fd,'rb') as f:
  if not stat.S_ISREG(os.fstat(fd).st_mode):raise ValueError('not regular')
  return f.read(1048577)
def sha(path):return hashlib.sha256(read(path)).hexdigest()
owned=False
try:
 manifest=json.loads(read(base+'/manifest.json'))
 boot=base+'/boot.py'
 if sha(boot)!=EXPECTED['boot.py']:
  c=json.loads(read('/usr/local/lib/stepx-companion/config.json'))
  if c.get('chain_sha256')!=CHAIN or sha(boot)!=CHAIN:raise ValueError('unknown chain')
  boot='/usr/local/lib/stepx-companion/previous-boot.py'
 owned=(manifest==EXPECTED and sha(boot)==EXPECTED['boot.py'] and sha(base+'/inject.py')==EXPECTED['inject.py'] and sha(launcher)==EXPECTED['launcher.py'] and sha(launcher+'.stepx-stock-20261021')==STOCK)
except (OSError,ValueError,KeyError):pass
print(json.dumps({'owned':owned}))
'''.replace('EXPECTED', repr(expected)).replace('CHAIN', repr(chain)).replace('STOCK', repr(CONFIG['stock_sha256']))
    return root_json(code).get('owned') is True


def autonomous_plan(summary, prior, owned, now):
    """Pure policy: phone listener first, persistent bounded desktop fallback."""
    def answer(state, message, delay=300, **extra):
        return {'state': state, 'message': message, 'retry_after_seconds': delay,
                'repair_allowed': False, **extra}
    if not owned:
        return answer('blocked', '未确认由 StepX 管理的 Google 适配；自动维护未修改手机'), prior
    if summary.get('phone_recovery_read_state') == 'unavailable':
        return answer('waiting', '手机恢复监听状态暂时无法读取，稍后复检；未修改手机', 60), prior
    if summary.get('monitor_schema_unsupported'):
        return answer('blocked', '手机恢复组件版本暂不支持，保留现有组件，请更新桌面端'), prior
    phone = summary.get('phone_recovery') or {}
    if summary.get('phone_recovery_current_session'):
        if phone.get('status') in ('blocked', 'disabled', 'not_installed', 'stopped'):
            message = {'unsupported_context': '当前固件或执行环境不支持 Google 自动维护，保留现有服务',
                       'signer_changed': 'Google 组件签名不在已验证范围，自动维护已暂停',
                       'uid_outside_user_zero': 'Google 组件安装用户与适配范围不符，自动维护已暂停'}.get(phone.get('error_code'),
                       '手机已暂停 Google 恢复或组件不符合适配条件；请查看详情')
            return answer('blocked', message), prior
        if summary.get('phone_recovery_current_components') and phone.get('recovery_pending') is True:
            until = (phone.get('checked_at', 0) + phone.get('retry_after_ms', 0)) / 1000 + 5
            if now < until:
                return answer('waiting', '手机正在原地恢复 Google 服务', max(5, min(180, int(until - now)))), prior
    upgrade = summary.get('monitor_upgrade_needed') is True
    if summary.get('status') == 'success' and summary.get('needs_repair') is False and not upgrade:
        return answer('healthy', 'Google 服务已校验，手机更新后会自动恢复'), {}
    if ((summary.get('needs_repair') is not True and not upgrade) or summary.get('permission_coverage_unknown')
            or summary.get('permission_definitions_checked') is False
            or not summary.get('component_versions')):
        return answer('waiting', '检测证据暂不完整，稍后自动复检；未修改手机', 60), prior
    versions = summary['component_versions']
    if not all(v.get('versionCode') for v in versions.values()):
        return answer('blocked', 'Google 组件信息不完整，请查看检测详情'), prior
    if not upgrade and not (summary.get('phone_recovery_current_session') and summary.get('phone_recovery_current_components')
            and phone.get('signatures_verified') is True and phone.get('recheck_supported') is True):
        return answer('needs_attention', '尚未确认当前组件的手机恢复监听与签名，请先修复 Google 服务以启用自动维护'), prior
    # A boot or a different receipt is not a new retry budget. Client restarts
    # and repeated gateway attempts for the same component generation are bounded.
    fault = hashlib.sha256(json.dumps({'versions': None if upgrade else versions, 'operation': 'upgrade_monitor' if upgrade else 'native_recheck',
                          'monitor_artifact': summary.get('monitor_artifact_sha256') if upgrade else None}, sort_keys=True).encode()).hexdigest()
    state = dict(prior) if prior.get('fault') == fault else {'fault': fault, 'attempts': 0, 'first_seen': now, 'next_retry': now + 45}
    if state.get('attempts', 0) >= (1 if upgrade else 3):
        return answer('needs_attention', '本版自动维护组件升级未通过校验，已停止重复激活，请查看原因' if upgrade
                      else '自动恢复已尝试 3 次，请查看具体原因；不会继续重复操作'), state
    if now < state.get('next_retry', 0):
        return answer('waiting', '先等待手机自动恢复，随后重新核验', max(1, int(state['next_retry'] - now))), state
    return answer('repair_needed', '准备更新既有 Google 自动维护组件' if upgrade else 'Google 更新后权限待恢复，准备后台修复', 0,
                  repair_allowed=True, fault=fault, operation='upgrade_monitor' if upgrade else 'native_recheck'), state


def _autonomous_status():
    guard()
    installed = shell('pm', 'list', 'packages', '--user', '0')
    if not all('package:' + package in installed.splitlines() for package in PACKAGES):
        return {'state': 'blocked', 'message': '未安装完整 Google 组件，自动维护不会安装应用', 'retry_after_seconds': 300, 'repair_allowed': False}
    disabled = shell('pm', 'list', 'packages', '-d', '--user', '0').splitlines()
    if any('package:' + package in disabled for package in PACKAGES):
        return {'state': 'blocked', 'message': 'Google 组件已被停用，保留你的设置', 'retry_after_seconds': 300, 'repair_allowed': False}
    with redirect_stdout(io.StringIO()):
        summary = status()
    if summary.get('monitor_upgrade_needed'):
        summary['monitor_artifact_sha256'] = sha256(ROOT.parents[1] / 'desktop/assets/stepx-companion-permissions.dex')
    if not all(summary.get('components_enabled', {}).get(package) is True for package in PACKAGES):
        return {'state': 'blocked', 'message': 'Google 组件未启用或已暂停，保留你的应用设置', 'retry_after_seconds': 300, 'repair_allowed': False}
    path = STATE / 'auto-recovery.json'
    prior = json.loads(path.read_text()) if path.exists() else {}
    decision, history = autonomous_plan(summary, prior, owned_runtime(), int(time.time()))
    if history != prior:
        save_json(path.name, history)
    decision['summary'] = summary
    return decision


def autonomous_status():
    try:
        with operation_lock():
            return _autonomous_status()
    except OperationBusy:
        return {'state': 'waiting', 'message': 'Google 安装或修复正在运行，自动维护稍后继续', 'retry_after_seconds': 30, 'repair_allowed': False}


def phone_idle_for_recovery():
    raw = shell('dumpsys', 'telephony.registry')
    values = re.findall(r'\bmCallState\s*=\s*(\d+)', raw)
    if not values or any(value != '0' for value in values):
        return False
    return 'RUNNING_UNLOCKED' in shell('dumpsys', 'user')


def companion_boot_module():
    script = ROOT.parents[1] / 'scripts/companion-boot.py'
    spec = importlib.util.spec_from_file_location('google_auto_companion_boot', script)
    boot = importlib.util.module_from_spec(spec); spec.loader.exec_module(boot)
    return boot


def companion_operation_lock():
    return operation_lock(companion_boot_module().STATE_LOCAL / 'operation.lock')


def request_native_recheck():
    # Only the already registered system-context observer receives this fixed,
    # DUMP-protected request. It cannot authorize grants or create a new watcher.
    # A broadcast response is not evidence that recovery completed.
    shell('am', 'broadcast', '--user', '0', '-a',
          'com.stepx.companion.action.RECHECK_GOOGLE_RUNTIME', '-p', 'android')


def upgrade_owned_monitor():
    """Activate the signed, already installed companion's new in-process observer.

    This does not install an APK or rebind an identity. The existing bounded
    worker preserves Android but may replace the Ubuntu gateway while acquiring
    its reviewed execution context. Upgrade is attempted only once per artifact.
    """
    boot = companion_boot_module()
    apk = ROOT.parents[1] / 'desktop/assets/stepx-companion.apk'
    with operation_lock(boot.STATE_LOCAL / 'operation.lock'):
        current = boot.status(apk)
        config = current.get('config') or {}
        if (not current.get('installed') or config.get('package') != boot.p.PACKAGE
                or config.get('serial') != boot.p.device_key() or config.get('boot_mode') != 'chained'
                or config.get('fingerprint') not in boot.p.ALLOWED_FINGERPRINTS):
            raise RuntimeError('No identity-bound managed companion; automatic monitor upgrade stopped')
        # Every active byte must still match the existing owned manifest before
        # any native activation. The installer repeats this guard transactionally.
        proof = root_json('''import os,json,hashlib,stat
b='/usr/local/lib/stepx-companion'
def read(path):
 fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW)
 with os.fdopen(fd,'rb') as f:
  if not stat.S_ISREG(os.fstat(fd).st_mode):raise ValueError('not regular')
  return f.read(1048577)
def sha(path):return hashlib.sha256(read(path)).hexdigest()
c=json.loads(read(b+'/config.json'))
ok=c==EXPECTED and all(sha(b+'/'+name)==c[key] for name,key in [('grant.py','grant_sha256'),('chain.py','chain_sha256'),('boot.py','boot_sha256')])
ok=ok and sha('/usr/local/lib/stepx-gms/boot.py')==c['chain_sha256'] and sha(b+'/previous-boot.py')==c['original_sha256']
print(json.dumps({'owned':ok}))
'''.replace('EXPECTED', repr(config)))
        if proof.get('owned') is not True:
            raise RuntimeError('Managed companion artifacts changed; automatic upgrade stopped')
        # Do not request rebind. Existing signer/UID plus exact observed baseline
        # are required; legacy baselines are measured read-only by this API.
        if not phone_idle_for_recovery():
            raise RuntimeError('Call began before monitor activation; deferred')
        result = boot.activate(apk, rebind=False)
        if result.get('status') != 'activated' or result.get('system_server_unchanged') is not True:
            raise RuntimeError('Monitor upgrade did not pass native read-back')
        google = result.get('google_login') or {}
        if (google.get('status') not in ('scheduled', 'needs_google_repair')
                or google.get('update_monitor') is not True or google.get('error')):
            raise RuntimeError('Phone Google monitor rejected activation: ' + str(google.get('error') or google.get('status') or 'missing proof'))


def repair_autonomous():
    try:
        # Acquire both gates before reserving an attempt. A manual companion
        # operation delays maintenance; it must not consume the repair budget.
        with operation_lock(), companion_operation_lock():
            decision = _autonomous_status()  # Never turn a failed read into a write.
            if not decision.get('repair_allowed'):
                return decision
            if not phone_idle_for_recovery():
                return {'state': 'waiting', 'message': '等待通话结束或首次解锁后自动维护', 'retry_after_seconds': 30, 'repair_allowed': False}
            guard(native=decision.get('operation') == 'upgrade_monitor')
            history = json.loads((STATE / 'auto-recovery.json').read_text())
            if history.get('fault') != decision['fault']:
                raise RuntimeError('Google recovery generation changed; nothing was activated')
            history['attempts'] = history.get('attempts', 0) + 1
            history['next_retry'] = int(time.time()) + min(1800, 30 * 2 ** (history['attempts'] - 1))
            # Durable reservation precedes native activation or a recheck request. A crash consumes
            # one attempt; it cannot silently reset the budget after restarting.
            save_json('auto-recovery.json', history)
            try:
                with redirect_stdout(io.StringIO()):
                    if decision.get('operation') == 'upgrade_monitor':
                        upgrade_owned_monitor()
                        time.sleep(3)  # Native observer initial check is delayed 1.5s.
                    else:
                        request_native_recheck()
                        return {'state': 'waiting', 'message': '已请求手机原地复检 Google 服务，稍后验证结果',
                                'retry_after_seconds': 5, 'repair_allowed': False, 'attempts': history['attempts']}
                    verified = status()
                if (decision.get('operation') == 'upgrade_monitor' and verified.get('monitor_upgrade_needed') is False
                        and verified.get('phone_recovery_current_session')):
                    # Native observer now owns its retry sequence. Do not start
                    # a gateway restart inside the same upgrade operation.
                    save_json('auto-recovery.json', {})
                    return {'state': 'healthy' if not verified.get('needs_repair') else 'waiting',
                            'message': 'Google 自动维护组件已更新，正在完成手机端复核',
                            'retry_after_seconds': 30, 'repair_allowed': False, 'summary': verified}
                if (verified.get('status') != 'success' or verified.get('needs_repair') is not False
                        or verified.get('monitor_upgrade_needed')):
                    raise RuntimeError('Google recovery read-back did not pass')
            except Exception as error:
                save_json('auto-recovery-last-error.json', {'operation': decision.get('operation', 'native_recheck'),
                          'attempt': history['attempts'], 'checked_at': int(time.time()),
                          'error_type': type(error).__name__, 'detail': str(error)[:2048]})
                upgrade = decision.get('operation') == 'upgrade_monitor'
                return {'state': 'needs_attention' if upgrade or history['attempts'] >= 3 else 'retrying',
                        'message': ('手机 Google 自动维护组件拒绝激活或未通过回读，已停止重复升级；原服务与数据已保留' if upgrade
                                    else 'Google 自动恢复未通过回读，账号和应用数据已保留'),
                        'retry_after_seconds': max(30, history['next_retry'] - int(time.time())),
                        'repair_allowed': False, 'attempts': history['attempts']}
            save_json('auto-recovery.json', {})
            return {'state': 'healthy', 'message': 'Google 更新后的权限已恢复并通过回读',
                    'retry_after_seconds': 300, 'repair_allowed': False, 'summary': verified}
    except OperationBusy:
        return {'state': 'waiting', 'message': 'Google 安装或修复正在运行，自动维护稍后继续', 'retry_after_seconds': 30, 'repair_allowed': False}


def repair_runtime():
    with operation_lock(), companion_operation_lock():
        return _repair_runtime()


def _repair_runtime():
    """Re-run the existing pinned adapter after a package update; keep boot hooks intact."""
    guard(native=True)
    assert 'RUNNING_UNLOCKED' in shell('dumpsys', 'user'), 'Unlock the phone first'
    assert shell('getprop', 'persist.openclaw.zygote.path') == '', 'Another startup operation is active'
    assert shell('getprop', 'sys.openclaw.env.start') == '', 'Another startup operation is active'
    expected = {name: python_source_sha256(ROOT / 'module' / name) for name in ('boot.py', 'inject.py')}
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
            assert summary['status'] == 'success' and not summary['needs_repair'], 'Current Google permissions did not recover'
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
    require_current_baseline(baseline)
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
    require_current_baseline(baseline)
    assert sha256(STATE / 'ubuntu_zygote.stock') == CONFIG['stock_sha256']
    assert not (STATE / 'boot-install.json').exists(), 'Existing recovery metadata must be reviewed, not overwritten'
    expected = {p.name: python_source_sha256(p) for p in (ROOT / 'module').glob('*.py')}
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
    for name in ['preflight', 'check-apks', 'prepare', 'install-apks', 'status', 'verify', 'certification', 'recover-adapter-metadata', 'repair-runtime', 'auto-status', 'repair-autonomous']:
        commands.add_parser(name)
    commands.add_parser('fetch').add_argument('--archive')
    commands.add_parser('adopt-backup').add_argument('directory')
    args = parser.parse_args()
    if args.command in ('auto-status', 'repair-autonomous'):
        result = autonomous_status() if args.command == 'auto-status' else repair_autonomous()
        print(json.dumps(result, ensure_ascii=False)); return
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
