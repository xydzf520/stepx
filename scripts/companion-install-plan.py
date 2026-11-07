"""Select and back up a signed companion APK before any phone mutation."""
import argparse, hashlib, importlib.util, json, os, shutil, subprocess, tempfile
from pathlib import Path
from companion_apk import apk_identity, apk_manifest, installation_choice
from host_runtime import user_state_root, file_lock, subprocess_options
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('permissions', ROOT/'scripts/companion-permissions.py')
p = importlib.util.module_from_spec(spec); spec.loader.exec_module(p)

def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()

def installation_device():
    """Ordinary APK installation needs identity, not a privileged firmware adapter."""
    transport = p.ADB[-1]
    expected = os.environ.get('STEPX_DEVICE_SERIAL', '').strip()
    if not expected and ':' not in transport and not transport.startswith('adb-'):
        expected = transport
    if not expected or not transport or transport.startswith('-') or any(c.isspace() for c in transport):
        raise RuntimeError('安装缺少已选手机身份，请重新选择设备')
    if p.run(p.ADB + ['get-state']).strip() != 'device':
        raise RuntimeError('手机未授权或已离线，尚未安装')
    if p.shell('getprop', 'ro.serialno') != expected:
        raise RuntimeError('连接的手机与已选设备不一致，尚未安装')
    model = p.shell('getprop', 'ro.product.model').lower()
    product = p.shell('getprop', 'ro.product.device').lower()
    if 'stepx' not in model and 'pz261a' not in model and product != 'pandora':
        raise RuntimeError('这台手机不是已支持的 StepX，尚未安装')
    sdk = p.shell('getprop', 'ro.build.version.sdk')
    if not sdk.isdecimal() or int(sdk) < 33:
        raise RuntimeError('StepX伴侣需要 Android 13 或更新版本')
    fingerprint = p.shell('getprop', 'ro.build.fingerprint')
    if not fingerprint:
        raise RuntimeError('无法读取手机固件信息，尚未安装')
    return {'firmware': fingerprint,
            'system_adaptation_supported': fingerprint in p.ALLOWED_FINGERPRINTS}

def package_paths():
    # Android returns exit 1 with no output when this package is not installed.
    # Treat only that exact absence as a first install, never a transport error.
    probe = subprocess.run(p.ADB + ['shell', 'pm', 'path', '--user', '0', p.PACKAGE],
                           capture_output=True, text=True, timeout=30, **subprocess_options())
    if probe.returncode in (0, 1) and not probe.stdout.strip() and not probe.stderr.strip():
        return []
    if probe.returncode:
        raise RuntimeError('无法核对手机伴侣安装位置，尚未修改手机')
    return probe.stdout.strip().splitlines()

def plan(candidate, trusted, selected):
    device = installation_device()
    identity = apk_identity(candidate)
    if identity != apk_identity(trusted) or identity[0] != p.PACKAGE:
        raise RuntimeError('伴侣安装包与客户端签名不一致，未改动手机')
    manifest = apk_manifest(candidate)
    paths = package_paths()
    if not paths:
        shutil.copyfile(candidate, selected)
        return {'ok':True,'needs_install':True,'message':'首次安装 StepX伴侣', **device}
    if len(paths)!=1 or not paths[0].startswith('package:/data/app/'):
        raise RuntimeError('手机伴侣安装位置未适配，已保留原应用')
    with tempfile.TemporaryDirectory(prefix='stepx-install-plan-') as folder:
        installed = Path(folder)/'installed.apk'
        p.run(p.ADB+['pull',paths[0][8:],str(installed)])
        if apk_identity(installed) != identity:
            raise RuntimeError('手机伴侣签名不同，不能覆盖；已保留应用与账号数据')
        choice, message = installation_choice(manifest, apk_manifest(installed))
        original_hash=digest(installed)
        backup=user_state_root()/'companion-install'/hashlib.sha256(p.device_key().encode()).hexdigest()[:24]
        backup.mkdir(parents=True,exist_ok=True,mode=0o700)
        with file_lock(backup/'backup.lock'):
            target=backup/(original_hash+'.apk')
            if not target.exists():
                with target.open('xb') as out: out.write(installed.read_bytes())
                target.chmod(0o600)
            if digest(target)!=original_hash: raise RuntimeError('原安装包备份校验失败，未安装')
        shutil.copyfile(installed if choice=='installed' else candidate,selected)
        return {'ok':True,'needs_install':digest(selected)!=original_hash,'message':message,'backup_verified':True, **device}

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('apk','trusted-apk','selected-apk'): parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args()
    try: print(json.dumps(plan(args.apk,args.trusted_apk,args.selected_apk),ensure_ascii=False))
    except Exception as error:
        print(json.dumps({'ok':False,'error':str(error)},ensure_ascii=False));raise SystemExit(1)
