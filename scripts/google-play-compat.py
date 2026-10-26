#!/usr/bin/env python3
"""Fixed-device Google compatibility repair, with private, scoped recovery.

Public interface: status | apply | rollback; STEPX_SERIAL and STEPX_ADB select
the existing USB transport. This is not device certification or an APK patch.
Never print worker proofs, cache paths, account names or exception messages.
"""

import argparse
import contextlib
import datetime
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import stat
import sys


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = Path(__file__).resolve().parent
SERIAL = os.environ.get("STEPX_DEVICE_SERIAL") or os.environ.get("STEPX_SERIAL") or ""
FORMAT = 1
MAX_PRIVATE_BYTES = 8 * 1024 * 1024
SCOPES = {
    "gms": {
        "package": "com.google.android.gms", "uid": 10194,
        "domain": "u:r:gmscore_app:s0:c512,c768",
        "apk_sha256": "0a0af2a2c44c459a0dded52cb7e176704d640ae738980d12360697cf96c8da4e",
    },
    "play": {
        "package": "com.android.vending", "uid": 10185,
        "domain": "u:r:untrusted_app:s0:c185,c256,c512,c768",
        "apk_sha256": "5d062e4acbe631a0c3683cd0654637ec5724cd18890e1a272be834622d2660b3",
    },
}
MODULE_FILES = {
    "filesm": "google_play_compat_files.py",
    "dbm": "google_play_compat_db.py",
    "snapm": "google_play_compat_snapshot.py",
    "storem": "google_play_compat_store_cache.py",
    "storedbm": "google_play_compat_store_db.py",
}
RECEIPT_ID = re.compile(r"compat-[0-9]{8}T[0-9]{6}-[0-9a-f]{16}\Z")


class RepairError(RuntimeError):
    def __init__(self, code, message):
        super().__init__(code)
        self.code, self.public_message = code, message


def _require(condition, code, message):
    if not condition:
        raise RepairError(code, message)


def _private_directory(path):
    path = Path(path)
    if not path.exists():
        path.mkdir(parents=True, mode=0o700, exist_ok=True)
    info = path.lstat()
    _require(stat.S_ISDIR(info.st_mode) and info.st_uid == os.getuid()
             and stat.S_IMODE(info.st_mode) == 0o700,
             "private_state_required", "兼容补丁的恢复目录权限不正确，未修改手机。")
    return path


def _read_private(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        info = os.fstat(stream.fileno())
        _require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1
                 and info.st_uid == os.getuid() and stat.S_IMODE(info.st_mode) == 0o600
                 and info.st_size <= MAX_PRIVATE_BYTES,
                 "invalid_receipt", "本地恢复记录不完整或权限不正确，未覆盖手机数据。")
        raw = stream.read(MAX_PRIVATE_BYTES + 1)
    _require(len(raw) <= MAX_PRIVATE_BYTES, "invalid_receipt", "本地恢复记录超过限制。")
    return json.loads(raw)


def _save_private(path, value, replace=False):
    """Exclusive journals; only the small current-operation pointer may replace."""
    path = Path(path)
    raw = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    _require(len(raw) <= MAX_PRIVATE_BYTES, "receipt_too_large", "恢复记录超过限制，未开始修改。")
    temporary = path.with_name(".tmp-" + secrets.token_hex(12)) if replace else path
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        if replace:
            os.replace(temporary, path)
        parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        if replace:
            try:
                temporary.unlink()
            except FileNotFoundError:
                pass


def state_directory():
    base = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state"))
    _require(base.is_absolute(), "invalid_state_path", "XDG_STATE_HOME 必须是绝对路径。")
    return _private_directory(base / "stepx-google-compat" / SERIAL)


@contextlib.contextmanager
def operation_lock(directory):
    descriptor = os.open(directory / "operation.lock",
                         os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w") as stream:
        try:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RepairError("operation_in_progress", "兼容补丁正在检查或处理，请稍后重试。") from None
        yield


def current_receipt(directory):
    pointer = directory / "current.json"
    if not pointer.exists():
        return None
    current = _read_private(pointer)
    identifier = current.get("receipt_id", "")
    _require(bool(RECEIPT_ID.fullmatch(identifier)), "invalid_receipt", "恢复记录标识无效。")
    if current.get("closed") is True:
        return None
    path = _private_directory(directory / identifier)
    manifest = _read_private(path / "manifest.json")
    _require(manifest.get("format") == FORMAT and manifest.get("serial") == SERIAL
             and manifest.get("receipt_id") == identifier and manifest.get("pins") == SCOPES,
             "invalid_receipt", "恢复记录不属于当前已验证的兼容补丁。")
    return manifest


def record_event(directory, identifier, phase, proof=None):
    _require(bool(RECEIPT_ID.fullmatch(identifier)), "invalid_receipt", "恢复记录标识无效。")
    event = {"phase": phase, "time": datetime.datetime.now(datetime.timezone.utc).isoformat()}
    if proof is not None:
        event["proof"] = proof
    _save_private(directory / identifier / (secrets.token_hex(12) + ".json"), event)


def _module_loader(scope):
    names = ("filesm", "dbm", "snapm") if scope == "gms" else ("filesm", "storem", "storedbm")
    lines = ["import types"]
    for name in names:
        source = (SCRIPTS / MODULE_FILES[name]).read_text()
        # Separate globals are essential: helpers have overlapping names and
        # dataclasses resolve their declaring module through sys.modules.
        module = "stepx_compat_" + name
        lines.append(f"{name}=types.ModuleType({module!r});sys.modules[{module!r}]={name}")
        lines.append(f"exec({source!r},{name}.__dict__)")
    return "\n".join(lines) + "\n"


WORKER_COMMON = r'''
package=filesm.PACKAGES[scope]
base='/data/user/0/'+package
db_path=base+'/databases/phenotype.db'
def discover():
 if scope=='gms':
  fallback=base+'/files/phenotype/shared/com.google.android.gms.gmscompliance.pb'
  parent=base+'/app_phenotype_shared'
  found=[]
  # Only the audited shared directory and the anonymous account hash qualify.
  for salt in sorted(os.listdir(parent)):
   if not re.fullmatch(r'[A-Za-z0-9_=-]{1,80}',salt):continue
   folder=parent+'/'+salt
   if not stat.S_ISDIR(os.lstat(folder).st_mode):continue
   target=folder+'/0dPD7nD8E2yOLxoYWJqw_g==.pb'
   if os.path.lexists(target):found.append(target)
  if not 1<=len(found)<=4:raise RuntimeError('Unsupported primary snapshot layout')
  return [fallback]+found
 parent=base+'/files'
 found=sorted(parent+'/'+name for name in os.listdir(parent) if name.startswith('experiment-flags-regular-'))
 if not 1<=len(found)<=5:raise RuntimeError('Unsupported Play cache layout')
 return found
def compact(baseline):
 return {'format':baseline['format'],'scope':baseline['scope'],
         'entries':[{k:v for k,v in e.items() if k!='data_zlib_b64'} for e in baseline['entries']]}
def db_read():
 connection=sqlite3.connect('file:'+db_path+'?mode=ro',uri=True)
 try:return (dbm if scope=='gms' else storedbm).plan(connection)
 finally:connection.close()
def inspect_files(baseline,db=None):
 entries=baseline['entries']
 if scope=='play' and isinstance(db,dict) and db.get('state',{}).get('account_id',0)>0:
  scoped=[e for e in entries if not e['path'].endswith('/experiment-flags-regular-null-account')]
  if scoped:entries=scoped
 checks=[]
 for entry in entries:
  raw=filesm._original(entry)
  if scope=='gms':
   fn=snapm.inspect_fallback if entry['path'].endswith('/com.google.android.gms.gmscompliance.pb') else snapm.inspect_primary
   checks.append(fn(raw)['effective_value'] is False)
  else:checks.append(storem.inspect_cache(raw)['empty'] is True)
 return bool(checks) and all(checks)
def db_active(plan):
 if scope=='gms':
  row=plan.get('override')
  return bool(row is not None and row['value']==0 and any(
   x['experiment_state_id']==plan['config']['committed_experiment_state_id'] for x in plan['links']))
 return storedbm.is_active(plan)
def summary(baseline,db):
 return {'cache_active':inspect_files(baseline,db),'db_active':db_active(db),'cache_count':len(baseline['entries'])}
def read_journal(path):
 filesm._path(scope,path,filesm.ROOT_BASE,journal=True)
 raw,metadata=filesm._read(path,filesm.ROOT_BASE)
 with filesm._directory(base+'/files') as parent:owner=filesm._metadata(parent,filesm.ROOT_BASE)
 if metadata['mode']!=0o600 or any(metadata[k]!=owner[k] for k in ('uid','gid','selinux_b64')):
  raise RuntimeError('Invalid journal ownership')
 return json.loads(raw)
def rollback_owned():
 errors=[];results={}
 if os.path.lexists(cache_journal):
  try:results['cache']=filesm.rollback(scope,cache_journal)
  except BaseException:errors.append('cache_conflict')
 else:
  try:
   if compact(filesm.prepare(scope,discover()))!=expected:raise RuntimeError('Missing cache recovery journal')
   results['cache']={'already_restored':True}
  except BaseException:errors.append('missing_cache_journal')
 if os.path.lexists(db_journal):
  try:
   receipt=read_journal(db_journal)
   connection=sqlite3.connect(db_path)
   try:results['db']=(dbm if scope=='gms' else storedbm).rollback(connection,receipt)
   finally:connection.close()
  except BaseException:errors.append('db_conflict')
 else:
  try:
   if db_read()!=expected_db:raise RuntimeError('Missing database recovery journal')
   results['db']={'already_restored':True}
  except BaseException:errors.append('missing_db_journal')
 out['recovery']={'errors':errors,'results':results}
 publish(out)
 if errors:raise RuntimeError('Scoped rollback preserved external changes')
 return results
'''


def worker_source(scope, action, manifest=None):
    _require(scope in SCOPES and action in ("status", "prepare", "apply", "rollback"),
             "invalid_operation", "不支持的兼容补丁操作。")
    source = _module_loader(scope) + f"scope={scope!r}\n" + WORKER_COMMON
    if action in ("status", "prepare"):
        source += "\nbaseline=filesm.prepare(scope,discover());db=db_read()\n"
        source += "out['summary']=summary(baseline,db)\n"
        if action == "prepare":
            source += "prepared={'file_baseline':baseline,'db_baseline':db}\n"
            source += "if len(json.dumps(prepared).encode())>200000:raise RuntimeError('Recovery baseline exceeds transport limit')\n"
            source += "out.update(prepared)\n"
        return source
    identifier = manifest["receipt_id"]
    _require(bool(RECEIPT_ID.fullmatch(identifier)), "invalid_receipt", "恢复记录标识无效。")
    source += f"\ncache_journal=base+'/files/stepx-google-compat/{identifier}-cache.json'\n"
    source += f"db_journal=base+'/files/stepx-google-compat/{identifier}-db.json'\n"
    baseline = manifest["scopes"][scope]["file_baseline"]
    expected = {"format": baseline["format"], "scope": baseline["scope"],
                "entries": [{key: value for key, value in entry.items() if key != "data_zlib_b64"}
                            for entry in baseline["entries"]]}
    source += f"expected={expected!r}\nexpected_db={manifest['scopes'][scope]['db_baseline']!r}\n"
    if action == "rollback":
        return source + (
            "rollback_owned()\n"
            "baseline=filesm.prepare(scope,discover());db=db_read()\n"
            "if compact(baseline)!=expected or db!=expected_db:raise RuntimeError('Rollback readback differs from original')\n"
            "out['summary']=dict(summary(baseline,db),rolled_back=True)\n")
    source += r'''
baseline=filesm.prepare(scope,discover())
if compact(baseline)!=expected or db_read()!=expected_db:raise RuntimeError('Fresh baseline differs from private backup')
if os.path.lexists(cache_journal) or os.path.lexists(db_journal):raise RuntimeError('Recovery journal already exists')
patched=[]
for entry in baseline['entries']:
 raw=filesm._original(entry)
 if scope=='gms':
  fn=snapm.patch_fallback if entry['path'].endswith('/com.google.android.gms.gmscompliance.pb') else snapm.patch_primary
 else:fn=storem.patch_cache
 patched.append(fn(raw))
connection=sqlite3.connect(db_path)
try:
 receipt=(dbm if scope=='gms' else storedbm).apply(connection,expected_db,
  before_commit=lambda r:filesm._write_journal(scope,db_journal,r,filesm.ROOT_BASE))
 out['db_receipt']=receipt;publish(out)
finally:connection.close()
try:
 out['cache_result']=filesm.apply(scope,baseline,patched,cache_journal)
 out['summary']=summary(filesm.prepare(scope,discover()),db_read())
 if not all(out['summary'][k] for k in ('cache_active','db_active')):raise RuntimeError('Applied state readback failed')
except BaseException:
 rollback_owned()
 raise
'''
    return source


class Transport:
    def __init__(self):
        self.module = None
        self.last_proof = None

    def check(self):
        selected = os.environ.get("STEPX_SERIAL", SERIAL)
        _require(":" not in selected, "usb_required", "此兼容补丁目前需要通过 USB 连接已验证的 StepX 手机。")
        _require(selected == SERIAL, "unsupported_device", "此兼容补丁仅适用于当前已验证的 StepX 手机。")
        os.environ["STEPX_SERIAL"] = selected
        spec = importlib.util.spec_from_file_location("stepx_compat_permissions", SCRIPTS / "companion-permissions.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.module = module
        try:
            module.guard(native=True)
        except Exception:
            raise RepairError("unsupported_firmware", "手机系统版本、原生文件或 USB 连接与已验证环境不符，未修改手机。") from None
        _require(module.shell("getprop", "ro.serialno") == SERIAL,
                 "unsupported_device", "连接到的手机不是当前已验证的 StepX 手机。")
        for settings in SCOPES.values():
            package = settings["package"]
            dump = module.shell("dumpsys", "package", package)
            _require(bool(re.search(r"\bappId=" + str(settings["uid"]) + r"\b", dump)),
                     "unsupported_package_identity", "Google 组件身份与已验证安装不符，未修改手机。")
            paths = [line[8:] for line in module.shell("pm", "path", package).splitlines()
                     if line.startswith("package:") and line.endswith("/base.apk")]
            _require(len(paths) == 1, "unsupported_apk", "Google 组件安装结构不匹配，未修改手机。")
            digest = module.shell("sha256sum", paths[0]).split()
            version = re.search(r"\bversionName=([A-Za-z0-9(). _-]{1,100})", dump)
            label = "Google Play 服务" if package == SCOPES["gms"]["package"] else "Google Play 商店"
            _require(bool(digest) and digest[0] == settings["apk_sha256"], "unsupported_apk",
                     label + ("（" + version[1].strip() + "）" if version else "")
                     + "与已测试版本不匹配，兼容补丁未执行。")

    def stop_apps(self):
        # The existing Android shell path was verified on the pinned firmware.
        for scope in ("play", "gms"):
            result = self.module.shell("am", "force-stop", "--user", "0", SCOPES[scope]["package"])
            _require(not result.strip(), "stop_failed", "Google 组件未能停止，未开始修改。")

    def run(self, scope, action, manifest=None):
        module = self.module
        settings = SCOPES[scope]
        self.last_proof = None
        original = module.AGENTBUS_WORKER_TEMPLATE
        identity = "os.setresgid(1000,0,0);os.setresuid(1000,0,0);lib.setfsgid(1000);lib.setfsuid(1000)"
        _require(original.count(identity) == 1 and original.count("b'u:r:system_app:s0'") == 1,
                 "transport_changed", "底层恢复工具已变化，需要重新核对兼容性。")
        uid = settings["uid"]
        adapted = original.replace(identity,
            f"os.setresgid({uid},0,0);os.setresuid({uid},0,0);lib.setfsgid({uid});lib.setfsuid({uid})")
        adapted = adapted.replace("b'u:r:system_app:s0'", repr(settings["domain"].encode()))
        adapted = adapted.replace("import os,sys,json,socket",
            "import types,dataclasses,typing,contextlib,secrets\nimport os,sys,json,socket", 1)
        source = worker_source(scope, action, manifest)
        source_hash = hashlib.sha256(source.encode()).hexdigest()
        result_path = module.STATE / "last-worker-result.json"
        try:
            previous_stat = result_path.stat()
            previous_stamp = (previous_stat.st_ino, previous_stat.st_mtime_ns, previous_stat.st_size)
        except FileNotFoundError:
            previous_stamp = None
        module.AGENTBUS_WORKER_TEMPLATE = adapted
        try:
            with module.runtime_lock():
                proof = module.run_bounded_android_worker(source,
                    int(module.shell("pidof", "system_server")), worker_context="agentbus",
                    preserve_verified_result=lambda value: setattr(self, "last_proof", value))
            self.last_proof = proof
        except Exception:
            # Preserve this call's result before any recovery worker can replace
            # the shared last-result file. Never mistake a stale proof for it.
            try:
                current_stat = result_path.stat()
                stamp = (current_stat.st_ino, current_stat.st_mtime_ns, current_stat.st_size)
                if stamp != previous_stamp:
                    saved = _read_private(result_path)
                    if saved.get("worker_code_sha256") == source_hash:
                        self.last_proof = saved
            except Exception:
                pass
            raise
        finally:
            module.AGENTBUS_WORKER_TEMPLATE = original
        _require(proof.get("uid") == uid and proof.get("euid") == 0
                 and proof.get("context", "").rstrip("\x00\n") == settings["domain"],
                 "worker_identity_mismatch", "无法确认操作范围与 Google 组件身份，恢复记录已保留。")
        return proof


def _checked(proof):
    _require(isinstance(proof, dict) and not proof.get("error"), "scoped_operation_failed",
             "兼容补丁检查或修改未通过；恢复记录已保留，不会覆盖新产生的数据。")
    return proof


def _exception_diagnostic(transport, error):
    diagnostic = {"exception_class": type(error).__name__}
    if isinstance(error, RepairError):
        diagnostic["error_code"] = error.code
    proof = getattr(transport, "last_proof", None)
    if proof is not None:
        diagnostic["worker_proof"] = proof
    return diagnostic


def _scope_status(proof):
    summary = proof.get("summary", {})
    _require(all(type(summary.get(key)) is bool for key in ("cache_active", "db_active")),
             "invalid_worker_result", "无法确认兼容补丁当前状态。")
    return {key: summary[key] for key in ("cache_active", "db_active")}


def _status(transport, directory=None, receipt=None):
    result = {}
    for scope in SCOPES:
        try:
            proof = transport.run(scope, "status")
            if directory is not None and receipt is not None:
                # Save first: _checked and summary validation may throw.
                record_event(directory, receipt["receipt_id"], scope + "_post_status_result", proof)
            result[scope] = _scope_status(_checked(proof))
        except Exception as error:
            if directory is not None and receipt is not None:
                record_event(directory, receipt["receipt_id"], scope + "_post_status_exception",
                             _exception_diagnostic(transport, error))
            raise
        # Whitelist public fields; never pass worker results directly to stdout.
    return result


def _active(scopes):
    return bool(scopes) and all(value is True for state in scopes.values() for value in state.values())


def _reply(scopes, receipt, message):
    response = {"ok": True, "supported": True, "active": _active(scopes),
                "can_rollback": receipt is not None, "message": message,
                "scopes": scopes, "official_certification_changed": False}
    if receipt:
        response["receipt_id"] = receipt["receipt_id"]
    return response


def _rollback(transport, directory, receipt):
    failures = []
    scopes = {}
    for scope in reversed(tuple(SCOPES)):
        try:
            proof = transport.run(scope, "rollback", receipt)
            record_event(directory, receipt["receipt_id"], scope + "_rollback", proof)
            scopes[scope] = _scope_status(_checked(proof))
            _require(proof.get("summary", {}).get("rolled_back") is True,
                     "rollback_readback_failed", "无法确认兼容补丁已经撤销。")
        except Exception as error:
            try:
                record_event(directory, receipt["receipt_id"], scope + "_rollback_exception",
                             _exception_diagnostic(transport, error))
            except Exception:
                pass
            failures.append(scope)
    if failures:
        raise RepairError("rollback_conflict", "部分 Google 数据已有后续变化，已保留这些数据和恢复记录；未强制覆盖。")
    _save_private(directory / "current.json", {"receipt_id": receipt["receipt_id"], "closed": True}, replace=True)
    return scopes


def execute(action, directory, transport):
    receipt = current_receipt(directory)
    transport.check()
    if action == "status":
        scopes = _status(transport, directory, receipt)
        return _reply(scopes, receipt, "兼容补丁已生效；这不改变 Google 的设备认证。" if _active(scopes)
                      else "已支持当前系统和 Google 组件版本；兼容补丁尚未完整生效。")
    if action == "rollback":
        if receipt is None:
            return _reply(_status(transport), None, "没有需要撤销的兼容补丁记录。")
        transport.stop_apps()
        scopes = _rollback(transport, directory, receipt)
        return _reply(scopes, None, "已撤销本次兼容补丁；请重新打开 Google Play。")
    if receipt is not None:
        scopes = _status(transport)
        if _active(scopes):
            return _reply(scopes, receipt, "兼容补丁已生效，沿用原恢复记录，无需重复安装。")
        raise RepairError("recovery_required", "已有恢复记录且状态发生变化，请先撤销原兼容补丁再重新应用。")
    transport.stop_apps()
    prepared = {}
    for scope in SCOPES:
        proof = _checked(transport.run(scope, "prepare"))
        prepared[scope] = {key: proof[key] for key in ("file_baseline", "db_baseline")}
    identifier = "compat-" + datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S-") + secrets.token_hex(8)
    _private_directory(directory / identifier)
    receipt = {"format": FORMAT, "receipt_id": identifier, "serial": SERIAL,
               "pins": SCOPES, "scopes": prepared}
    _save_private(directory / identifier / "manifest.json", receipt)
    _save_private(directory / "current.json", {"receipt_id": identifier, "closed": False}, replace=True)
    try:
        scopes = {}
        for scope in SCOPES:
            record_event(directory, identifier, scope + "_apply_started")
            proof = transport.run(scope, "apply", receipt)
            record_event(directory, identifier, scope + "_apply_result", proof)
            scopes[scope] = _scope_status(_checked(proof))
        _require(_active(scopes), "readback_failed", "兼容补丁写入后的状态不一致。")
    except Exception as error:
        # The original failure must survive subsequent rollback workers and
        # their shared last-result file. This is private recovery evidence only.
        try:
            record_event(directory, identifier, "apply_failed_before_rollback",
                         _exception_diagnostic(transport, error))
        except Exception:
            pass  # Recovery must still run if the host disk becomes unavailable.
        try:
            _rollback(transport, directory, receipt)
        except Exception:
            raise RepairError("apply_recovery_required", "兼容补丁未完成，部分恢复需继续核对；原始备份和恢复记录已保留。") from None
        raise RepairError("apply_rolled_back", "兼容补丁未完成，已撤销本次修改并保留恢复记录。") from None
    return _reply(scopes, receipt, "兼容补丁已应用并回读确认；请重新打开 Google Play。设备认证状态不作更改。")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("status", "apply", "rollback"))
    arguments = parser.parse_args(argv)
    directory = None
    try:
        directory = state_directory()
        with operation_lock(directory):
            result = execute(arguments.action, directory, Transport())
    except Exception as error:
        can_rollback = False
        if directory is not None:
            try:
                can_rollback = current_receipt(directory) is not None
            except Exception:
                pass
        result = {"ok": False, "supported": False, "active": False,
                  "can_rollback": can_rollback,
                  "error": error.code if isinstance(error, RepairError) else "compatibility_check_failed",
                  "message": error.public_message if isinstance(error, RepairError)
                  else "兼容补丁检查未完成；手机数据与私有恢复记录不会被强制覆盖。"}
    print(json.dumps(result, ensure_ascii=False, separators=(",", ":")))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
