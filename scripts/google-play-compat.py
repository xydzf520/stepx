#!/usr/bin/env python3
"""Fixed-device Google compatibility repair, with private, scoped recovery.

Public interface: status | apply | rollback; STEPX_SERIAL and STEPX_ADB select
the existing USB transport. This is not device certification or an APK patch.
Never print worker proofs, cache paths, account names or exception messages.
"""

import argparse
import contextlib
import copy
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import stat
import subprocess
import sys
import tempfile
import time


SCRIPTS = Path(__file__).resolve().parent
ROOT = SCRIPTS.parent
for _candidate_scripts in (SCRIPTS, ROOT.parent.parent.parent / "scripts"):
    if (_candidate_scripts / "host_runtime.py").is_file():
        if str(_candidate_scripts) not in sys.path:
            sys.path.insert(0, str(_candidate_scripts))
        break
import host_runtime

SERIAL = os.environ.get("STEPX_DEVICE_SERIAL") or os.environ.get("STEPX_SERIAL") or ""
FORMAT = 1
MAX_PRIVATE_BYTES = 8 * 1024 * 1024
_O_NOFOLLOW = getattr(os, "O_NOFOLLOW", 0)
_O_NONBLOCK = getattr(os, "O_NONBLOCK", 0)
_O_BINARY = getattr(os, "O_BINARY", 0)

SCOPES = {
    "gms": {
        "package": "com.google.android.gms", "uid": 10194,
        "domain": "u:r:gmscore_app:s0:c512,c768",
        "apk_sha256": "0a0af2a2c44c459a0dded52cb7e176704d640ae738980d12360697cf96c8da4e",
        "cert_sha256": "5f2391277b1dbd489000467e4c2fa6af802430080457dce2f618992e9dfb5402",
    },
    "play": {
        "package": "com.android.vending", "uid": 10185,
        "domain": "u:r:untrusted_app:s0:c185,c256,c512,c768",
        "apk_sha256": "5d062e4acbe631a0c3683cd0654637ec5724cd18890e1a272be834622d2660b3",
        "cert_sha256": "7ce83c1b71f3d572fed04c8d40c5cb10ff75e6d87d9df6fbd53f0468c2905053",
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


def _is_wireless_serial(value):
    return ":" in value or "_adb-tls-connect._tcp" in value or "._adb-" in value


def _load_target_config():
    for candidate in (ROOT / "handoff/google-play-stepx/target.json", ROOT.parent / "target.json"):
        if candidate.is_file():
            try:
                return json.loads(candidate.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                return {}
    return {}


def _target_cert_pins():
    target = _load_target_config()
    by_package = {}
    packages_cfg = target.get("packages")
    if isinstance(packages_cfg, dict):
        for pkg, info in packages_cfg.items():
            if isinstance(info, dict):
                cert = info.get("cert_sha256") or info.get("signer_sha256")
                if isinstance(cert, str) and re.fullmatch(r"[0-9a-fA-F]{64}", cert):
                    by_package[pkg] = cert.lower()
    elif isinstance(packages_cfg, list):
        for info in packages_cfg:
            if isinstance(info, dict) and isinstance(info.get("package"), str):
                cert = info.get("cert_sha256") or info.get("signer_sha256")
                if isinstance(cert, str) and re.fullmatch(r"[0-9a-fA-F]{64}", cert):
                    by_package[info["package"]] = cert.lower()
    for info in target.get("apks") or ():
        if isinstance(info, dict) and isinstance(info.get("package"), str):
            cert = info.get("cert_sha256") or info.get("signer_sha256")
            if isinstance(cert, str) and re.fullmatch(r"[0-9a-fA-F]{64}", cert):
                by_package.setdefault(info["package"], cert.lower())
    return {
        scope: by_package.get(defaults["package"], defaults["cert_sha256"]).lower()
        for scope, defaults in SCOPES.items()
    }


def _private_directory(path):
    path = Path(path)
    if not path.exists():
        path.mkdir(parents=True, mode=0o700, exist_ok=True)
    info = path.lstat()
    if os.name == "nt":
        _require(stat.S_ISDIR(info.st_mode) and not path.is_symlink(),
                 "private_state_required", "兼容补丁的恢复目录权限不正确，未修改手机。")
    else:
        _require(stat.S_ISDIR(info.st_mode) and info.st_uid == os.getuid()
                 and stat.S_IMODE(info.st_mode) == 0o700,
                 "private_state_required", "兼容补丁的恢复目录权限不正确，未修改手机。")
    return path


def _read_private(path):
    path = Path(path)
    if os.name == "nt" and path.is_symlink():
        raise OSError("Refusing symlink receipt")
    descriptor = os.open(path, os.O_RDONLY | _O_NOFOLLOW | _O_NONBLOCK | _O_BINARY)
    with os.fdopen(descriptor, "rb") as stream:
        info = os.fstat(stream.fileno())
        if os.name == "nt":
            valid = stat.S_ISREG(info.st_mode) and info.st_size <= MAX_PRIVATE_BYTES
        else:
            valid = (stat.S_ISREG(info.st_mode) and info.st_nlink == 1
                     and info.st_uid == os.getuid() and stat.S_IMODE(info.st_mode) == 0o600
                     and info.st_size <= MAX_PRIVATE_BYTES)
        _require(valid, "invalid_receipt", "本地恢复记录不完整或权限不正确，未覆盖手机数据。")
        raw = stream.read(MAX_PRIVATE_BYTES + 1)
    _require(len(raw) <= MAX_PRIVATE_BYTES, "invalid_receipt", "本地恢复记录超过限制。")
    return json.loads(raw)


def _save_private(path, value, replace=False):
    """Exclusive journals; only the small current-operation pointer may replace."""
    path = Path(path)
    if path.is_symlink():
        raise OSError("Refusing symlink receipt")
    raw = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    _require(len(raw) <= MAX_PRIVATE_BYTES, "receipt_too_large", "恢复记录超过限制，未开始修改。")
    temporary = path.with_name(".tmp-" + secrets.token_hex(12)) if replace else path
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | _O_NOFOLLOW | _O_BINARY, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        if replace:
            os.replace(temporary, path)
        if os.name != "nt" and hasattr(os, "O_DIRECTORY"):
            parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | _O_NOFOLLOW)
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
    selected = os.environ.get("STEPX_SERIAL", SERIAL)
    _require(not _is_wireless_serial(selected) and not _is_wireless_serial(SERIAL),
             "usb_required", "此兼容补丁目前需要通过 USB 连接已验证的 StepX 手机。")
    _require(bool(SERIAL) and bool(re.fullmatch(r"[A-Za-z0-9._-]+", SERIAL)) and SERIAL not in {".", ".."},
             "unsupported_device", "此兼容补丁仅适用于当前已验证的 StepX 手机。")
    if os.environ.get("XDG_STATE_HOME"):
        _require(Path(os.environ["XDG_STATE_HOME"]).is_absolute(),
                 "invalid_state_path", "XDG_STATE_HOME 必须是绝对路径。")
    base = host_runtime.user_state_root()
    _require(base.is_absolute(), "invalid_state_path", "状态目录必须是绝对路径。")
    target = base / "google-play-compat" / SERIAL
    if SERIAL and not target.exists() and os.name != "nt" and not os.environ.get("STEPX_STATE_DIR"):
        legacy_root = Path(os.environ.get("XDG_STATE_HOME") or (Path.home() / ".local/state"))
        legacy = legacy_root / "stepx-google-compat" / SERIAL
        if legacy.exists():
            host_runtime.migrate_legacy_state(legacy, target)
    return _private_directory(target)


@contextlib.contextmanager
def operation_lock(directory):
    stack = contextlib.ExitStack()
    try:
        stack.enter_context(host_runtime.file_lock(directory / "operation.lock", timeout=15))
    except RuntimeError:
        raise RepairError("operation_in_progress", "兼容补丁正在检查或处理，请稍后重试。") from None
    try:
        yield
    finally:
        stack.close()


def _valid_receipt_pins(pins):
    if not isinstance(pins, dict) or set(pins) != set(SCOPES):
        return False
    for scope, defaults in SCOPES.items():
        entry = pins.get(scope)
        if not isinstance(entry, dict) or entry.get("package") != defaults["package"]:
            return False
        uid = entry.get("uid")
        if type(uid) is not int or not (10000 <= uid < 100000):
            return False
        domain = entry.get("domain")
        if not isinstance(domain, str) or not domain.startswith("u:r:"):
            return False
    return True


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
             and manifest.get("receipt_id") == identifier and _valid_receipt_pins(manifest.get("pins")),
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
        source_path = SCRIPTS / MODULE_FILES[name]
        if not source_path.is_file():
            source_path = ROOT.parent.parent.parent / "scripts" / MODULE_FILES[name]
        source = source_path.read_text(encoding="utf-8")
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


def _resolve_selinux_domain(module, scope, package, uid, dump, ps_lines):
    app_offset = uid - 10000
    user_label = f"u0_a{app_offset}"
    for line in ps_lines:
        parts = line.split()
        if len(parts) < 3:
            continue
        ctx, owner, proc_name = parts[0], parts[1], parts[-1]
        if owner == user_label and (proc_name == package or proc_name.startswith(package + ":")):
            if re.fullmatch(r"u:r:[A-Za-z0-9_]+:s0(?::[A-Za-z0-9_,]+)?", ctx):
                return ctx
    seinfo_match = re.search(r"\bseInfo=([^\s]+)", dump, re.I)
    seinfo = seinfo_match.group(1) if seinfo_match else ""
    domain_type = "gmscore_app" if (scope == "gms" or "gmscore" in seinfo) else "untrusted_app"
    mls = None
    try:
        ls_out = module.shell("ls", "-Zd", "/data/user/0/" + package)
        mls_match = re.search(r"\bu:object_r:[A-Za-z0-9_]+:(s0(?::[A-Za-z0-9_,]+)?)\b", ls_out)
        if mls_match:
            mls = mls_match.group(1)
    except Exception:
        mls = None
    if not mls:
        if domain_type == "gmscore_app":
            mls = "s0:c512,c768"
        else:
            mls = f"s0:c{app_offset & 0xff},c{256 + (app_offset >> 8)},c512,c768"
    return f"u:r:{domain_type}:{mls}"


class Transport:
    accepts_action = True

    def __init__(self):
        self.module = None
        self.last_proof = None
        self.scopes = copy.deepcopy(SCOPES)

    def _verify_package_signer(self, package, paths, apk_hashes, expected_cert, defaults):
        if (len(paths) == 1 and apk_hashes == [defaults["apk_sha256"]]
                and expected_cert == defaults["cert_sha256"]):
            return
        with tempfile.TemporaryDirectory(prefix="stepx-compat-cert-") as scratch:
            apksigner = host_runtime.sdk_command("apksigner")
            env = host_runtime.java_environment()
            opts = host_runtime.subprocess_options()
            for idx, (remote, expected_hash) in enumerate(zip(paths, apk_hashes)):
                local = Path(scratch) / f"{idx}.apk"
                self.module.run(self.module.ADB + ["pull", remote, str(local)], timeout=180)
                pulled_hash = hashlib.sha256(local.read_bytes()).hexdigest()
                if pulled_hash != expected_hash:
                    raise RepairError("unsupported_apk", "Google 组件正在后台更新，请等待更新完成后重试。")
                verify = subprocess.run(
                    apksigner + ["verify", "--print-certs", str(local)],
                    capture_output=True, text=True, encoding="utf-8", timeout=90, env=env, **opts,
                )
                digests = {
                    s.lower()
                    for s in re.findall(
                        r"^(?:Signer #\d+|Signer \([^)]+\)|V\d+(?:\.\d+)? Signer(?: #\d+)?|V\d+(?:\.\d+)? Signer \([^)]+\)):? certificate SHA-256 digest: ([0-9a-fA-F]{64})\s*$",
                        verify.stdout or "",
                        re.M,
                    )
                }
                if verify.returncode != 0 or digests != {expected_cert}:
                    label = "Google Play 服务" if package == SCOPES["gms"]["package"] else "Google Play 商店"
                    raise RepairError("unsupported_apk", label + "签名证书与已验证 Google 官方证书不匹配，兼容补丁未执行。")

    def check(self, action="status"):
        selected = os.environ.get("STEPX_SERIAL", SERIAL)
        _require(not _is_wireless_serial(selected), "usb_required", "此兼容补丁目前需要通过 USB 连接已验证的 StepX 手机。")
        _require(bool(selected) and selected == SERIAL, "unsupported_device", "此兼容补丁仅适用于当前已验证的 StepX 手机。")
        os.environ["STEPX_SERIAL"] = selected
        perm_path = SCRIPTS / "companion-permissions.py"
        if not perm_path.is_file():
            perm_path = ROOT.parent.parent.parent / "scripts/companion-permissions.py"
        spec = importlib.util.spec_from_file_location("stepx_compat_permissions", perm_path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.module = module
        try:
            module.guard(native=True)
        except Exception:
            raise RepairError("unsupported_firmware", "手机系统版本、原生文件或 USB 连接与已验证环境不符，未修改手机。") from None
        _require(module.shell("getprop", "ro.serialno") == SERIAL,
                 "unsupported_device", "连接到的手机不是当前已验证的 StepX 手机。")
        cert_pins = _target_cert_pins()
        try:
            ps_lines = module.shell("ps", "-AZ").splitlines()
        except Exception:
            ps_lines = []
        resolved = {}
        for scope, defaults in SCOPES.items():
            package = defaults["package"]
            dump = module.shell("dumpsys", "package", package)
            uid_match = re.search(r"\b(?:appId|userId)=([0-9]+)\b", dump)
            _require(bool(uid_match), "unsupported_package_identity", "Google 组件身份与已验证安装不符，未修改手机。")
            uid = int(uid_match.group(1))
            _require(10000 <= uid < 100000, "unsupported_package_identity", "Google 组件身份与已验证安装不符，未修改手机。")
            domain = _resolve_selinux_domain(module, scope, package, uid, dump, ps_lines)
            paths = [
                line[8:]
                for line in module.shell("pm", "path", "--user", "0", package).splitlines()
                if line.startswith("package:/") and line.endswith(".apk")
            ]
            if action != "rollback":
                _require(bool(paths), "unsupported_apk", "Google 组件安装结构不匹配，未修改手机。")
                base_paths = [p for p in paths if p.endswith("/base.apk")]
                _require(len(base_paths) == 1 or len(paths) == 1, "unsupported_apk", "Google 组件安装结构不匹配，未修改手机。")
                apk_hashes = []
                for apk_path in paths:
                    parts = module.shell("sha256sum", apk_path).split()
                    _require(bool(parts) and bool(re.fullmatch(r"[0-9a-fA-F]{64}", parts[0])),
                             "unsupported_apk", "无法读取 Google 组件安装包校验值，未修改手机。")
                    apk_hashes.append(parts[0].lower())
                try:
                    self._verify_package_signer(package, paths, apk_hashes, cert_pins[scope], defaults)
                except RepairError:
                    raise
                except Exception:
                    version = re.search(r"\bversionName=([A-Za-z0-9(). _-]{1,100})", dump)
                    label = "Google Play 服务" if package == SCOPES["gms"]["package"] else "Google Play 商店"
                    raise RepairError(
                        "unsupported_apk",
                        label + ("（" + version[1].strip() + "）" if version else "") + "签名校验未完成，兼容补丁未执行。",
                    ) from None
                primary_sha = apk_hashes[paths.index(base_paths[0])] if base_paths else apk_hashes[0]
            else:
                primary_sha = ""
            resolved[scope] = {
                "package": package,
                "uid": uid,
                "domain": domain,
                "apk_sha256": primary_sha,
                "cert_sha256": cert_pins[scope],
            }
        self.scopes = resolved

    def stop_apps(self):
        # The existing Android shell path was verified on the pinned firmware.
        for scope in ("play", "gms"):
            result = self.module.shell("am", "force-stop", "--user", "0", self.scopes[scope]["package"])
            _require(not result.strip(), "stop_failed", "Google 组件未能停止，未开始修改。")

    def run(self, scope, action, manifest=None):
        module = self.module
        settings = self.scopes[scope]
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


def execute(action, directory, transport, force_close_receipt=False):
    receipt = current_receipt(directory)
    if getattr(transport, "accepts_action", False):
        transport.check(action)
    else:
        transport.check()
    if action == "status":
        scopes = _status(transport, directory, receipt)
        return _reply(scopes, receipt, "兼容补丁已生效；这不改变 Google 的设备认证。" if _active(scopes)
                      else "已支持当前系统和 Google 组件版本；兼容补丁尚未完整生效。")
    if action == "rollback":
        if receipt is None:
            return _reply(_status(transport), None, "没有需要撤销的兼容补丁记录。")
        transport.stop_apps()
        try:
            scopes = _rollback(transport, directory, receipt)
        except RepairError as error:
            if error.code != "rollback_conflict" or not force_close_receipt:
                raise
            try:
                record_event(directory, receipt["receipt_id"], "receipt_force_closed")
            except Exception:
                pass
            closed_receipt = copy.deepcopy(receipt)
            closed_receipt["closed"] = True
            closed_receipt["force_closed_at"] = int(time.time())
            _save_private(directory / "current.json", closed_receipt, replace=True)
            return _reply(_status(transport), None, "已关闭冲突的兼容补丁恢复记录并保留当前手机数据。")
        return _reply(scopes, None, "已撤销本次兼容补丁；请重新打开 Google Play。")
    superseded_receipt_id = None
    if receipt is not None:
        scopes = _status(transport)
        if _active(scopes):
            return _reply(scopes, receipt, "兼容补丁已生效，沿用原恢复记录，无需重复安装。")
        superseded_receipt_id = receipt["receipt_id"]
        record_event(directory, superseded_receipt_id, "receipt_superseded")
        superseded_receipt = copy.deepcopy(receipt)
        superseded_receipt["closed"] = True
        superseded_receipt["superseded_at"] = int(time.time())
        _save_private(directory / "current.json", superseded_receipt, replace=True)
    transport.stop_apps()
    prepared = {}
    for scope in SCOPES:
        proof = _checked(transport.run(scope, "prepare"))
        prepared[scope] = {key: proof[key] for key in ("file_baseline", "db_baseline")}
    identifier = "compat-" + datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S-") + secrets.token_hex(8)
    _private_directory(directory / identifier)
    pins = copy.deepcopy(getattr(transport, "scopes", SCOPES))
    receipt = {"format": FORMAT, "receipt_id": identifier, "serial": SERIAL,
               "pins": pins, "scopes": prepared}
    if superseded_receipt_id is not None:
        receipt["superseded_receipt_id"] = superseded_receipt_id
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
    parser.add_argument("--force-close-receipt", action="store_true",
                        help="Close a conflicted rollback receipt while preserving current device data")
    arguments = parser.parse_args(argv)
    directory = None
    try:
        selected = os.environ.get("STEPX_SERIAL", SERIAL)
        _require(not _is_wireless_serial(selected) and not _is_wireless_serial(SERIAL),
                 "usb_required", "此兼容补丁目前需要通过 USB 连接已验证的 StepX 手机。")
        _require(arguments.action == "rollback" or not arguments.force_close_receipt,
                 "invalid_operation", "--force-close-receipt 仅可用于 rollback。")
        directory = state_directory()
        with operation_lock(directory):
            result = execute(arguments.action, directory, Transport(),
                             force_close_receipt=arguments.force_close_receipt)
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
