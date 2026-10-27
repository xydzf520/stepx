"""APK identity and embedded-runtime checks shared by companion installation paths.

Only reads local APK metadata. No device operations or model configuration access.
"""
from pathlib import Path
import json
import re
import subprocess
from host_runtime import sdk_command, java_environment, subprocess_options

PACKAGE = 'com.stepx.companion.app'
NEO_NAMESPACE = 'com.stepx.companion.neo'
AUTHORITY = PACKAGE + '.neo.control'
ROOT = Path(__file__).resolve().parents[1]


def run(args, timeout=30):
    p = subprocess.run([str(x) for x in args], capture_output=True, text=True,
                       encoding='utf-8', timeout=timeout, env=java_environment(), **subprocess_options())
    if p.returncode:
        raise RuntimeError('APK 校验工具未完成：' + Path(str(args[0])).name)
    return p.stdout.strip()


def signer_digests(proof):
    # Android build-tools 37 labels signers by signing scheme; older tools use Signer #1.
    return {value.lower() for value in re.findall(
        r'^(?:Signer #\d+|V\d+(?:\.\d+)? Signer(?: #\d+)?):? certificate SHA-256 digest: ([a-fA-F0-9]{64})$',
        proof, re.M)}


def apk_package(path):
    """Read only the declared package; generic APK installs have no companion size cap."""
    if not path.is_file():
        raise RuntimeError('请选择有效的本地 APK 安装包')
    if (ROOT / 'runtime/android-build-tools/lib/apk-identity.jar').is_file():
        package = run(sdk_command('apk-identity') + [path])
    else:
        output = run(sdk_command('aapt') + ['dump', 'badging', path])
        match = re.search(r"^package: name='([^']+)'", output, re.M)
        package = match.group(1) if match else ''
    if not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*', package):
        raise RuntimeError('无法核对 APK 包名，尚未安装；请使用完整安装包')
    return package


def apk_identity(path):
    if not path.is_file() or path.stat().st_size > 200 * 1024 * 1024:
        raise RuntimeError('请选择不超过 200 MiB 的本地 APK')
    package = apk_package(path)
    proof = run(sdk_command('apksigner') + ['verify', '--print-certs', path])
    certs = signer_digests(proof)
    if len(certs) != 1:
        raise RuntimeError('APK 必须具有一个可验证的签名')
    return package, next(iter(certs))


def apk_manifest(path):
    if (ROOT / 'runtime/android-build-tools/lib/apk-identity.jar').is_file():
        try:
            value = json.loads(run(sdk_command('apk-identity') + [path, '--manifest']))
        except Exception as error:
            raise RuntimeError('APK 组件校验运行时需更新，请重新解压完整安装包') from error
        if not isinstance(value, dict) or not isinstance(value.get('components'), list):
            raise RuntimeError('APK 组件校验未返回有效数据')
        return value
    # Developer checkouts can use their SDK; shipped builds use the portable Java parser above.
    package, _ = apk_identity(path)
    xml = run(sdk_command('aapt') + ['dump', 'xmltree', path, 'AndroidManifest.xml'])
    result, active, indent = [], None, 0
    for line in xml.splitlines():
        element = re.match(r'(\s*)E: (\w+)', line)
        if element:
            depth = len(element.group(1))
            if active is not None and depth <= indent: active = None
            if element.group(2) in ('service', 'provider', 'activity'):
                active = {'type': element.group(2)}; indent = depth; result.append(active)
            continue
        if active is None or len(line) - len(line.lstrip()) != indent + 2: continue
        attr = re.match(r'\s*A: android:(name|authorities|permission|enabled)(?:\([^)]*\))?=(.*)', line)
        if not attr: continue
        raw = attr.group(2)
        text = re.match(r'"([^"\n]*)"', raw)
        if text: active[attr.group(1)] = text.group(1)
        elif attr.group(1) == 'enabled' and '(type 0x12)' in raw:
            active['enabled'] = not raw.strip().endswith('0x0')
    return {'package':package, 'components':result}


def embedded_runtime(manifest):
    if manifest.get('package') != PACKAGE: return False
    items = {}
    for component in manifest.get('components', []):
        name = component.get('name', '')
        if name.startswith('.'): name = PACKAGE + name
        elif '.' not in name: name = PACKAGE + '.' + name
        if component.get('enabled') is not False:
            items[(component.get('type'), name)] = component
    control = items.get(('service', NEO_NAMESPACE+'.ControlService'), {})
    keyboard = items.get(('service', NEO_NAMESPACE+'.AgentInputMethodService'), {})
    provider = items.get(('provider', NEO_NAMESPACE+'.CompanionBridgeProvider'), {})
    return (control.get('permission') == 'android.permission.BIND_ACCESSIBILITY_SERVICE'
            and keyboard.get('permission') == 'android.permission.BIND_INPUT_METHOD'
            and AUTHORITY in provider.get('authorities', '').split(';')
            and ('activity', PACKAGE+'.NeoSettingsActivity') in items)



def has_embedded_engine(manifest):
    """Protect even an older/partial runtime, regardless of whether it is enabled."""
    for component in manifest.get('components', []):
        name = component.get('name', '')
        if name.startswith(NEO_NAMESPACE + '.') or AUTHORITY in component.get('authorities', '').split(';'):
            return True
    return False


def validate_update_manifests(candidate, installed):
    if candidate.get('package') != PACKAGE or installed.get('package') != PACKAGE:
        raise RuntimeError('更新包或手机应用包名不符，未安装')
    if has_embedded_engine(installed) and not embedded_runtime(candidate):
        raise RuntimeError('手机已内置 GUI Agent，但当前更新包未包含完整引擎，已阻止覆盖。请在“小工具 → 内置 GUI Agent”选择同签名的完整 StepX伴侣 APK，或使用包含完整 APK 的安装目录。现有应用与数据未改变。')
    return {'installed_embedded_gui':has_embedded_engine(installed),
            'candidate_embedded_gui':embedded_runtime(candidate)}


def ensure_update_preserves_runtime(candidate_apk, installed_apk):
    return validate_update_manifests(apk_manifest(candidate_apk), apk_manifest(installed_apk))


if __name__ == '__main__':
    import argparse
    import sys
    parser = argparse.ArgumentParser(description='Read APK package identity without device operations')
    parser.add_argument('--package', type=Path, required=True)
    parser.add_argument('--verify-signature', action='store_true')
    args = parser.parse_args()
    try:
        package = apk_package(args.package)
        if args.verify_signature:
            proof = run(sdk_command('apksigner') + ['verify', '--print-certs', args.package])
            if not signer_digests(proof):
                raise RuntimeError('APK 签名校验未通过')
        print(json.dumps({'ok': True, 'package': package}, ensure_ascii=False))
    except Exception as error:
        print(json.dumps({'ok': False, 'error': str(error)}, ensure_ascii=False))
        sys.exit(1)
