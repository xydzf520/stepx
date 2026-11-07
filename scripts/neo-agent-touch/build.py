#!/usr/bin/env python3
"""Build the original StepX adapter against a pinned private Neo source checkout.

Downloaded upstream code and resulting APK stay outside the public repository.
This script does not install the APK, grant phone permissions, or contact a model.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET
import zipfile
from embedded import assemble
from instrument import instrument

COMMIT = 'a5a6b6aa28dece90831d0630214f514d23ef443e'
REPOSITORY = 'https://github.com/JiayuStudio/stepx-neo-lab.git'
ORIGINAL = 'studio.jiayu.neoagent'
PACKAGE = 'com.stepx.companion.neo'
ROOT = Path(__file__).resolve().parents[2]
ANDROID = '{http://schemas.android.com/apk/res/android}'
ET.register_namespace('android', ANDROID[1:-1])


def run(args, **kwargs):
    subprocess.run([str(x) for x in args], check=True, **kwargs)


def tool(name, directory):
    path = directory / ('bin' if name in ('javac', 'java', 'jar') else '') / (name + ('.exe' if os.name == 'nt' else ''))
    if not path.is_file():
        raise RuntimeError(f'Missing build tool: {path}')
    return path


def rewrite_method(source, signature, body):
    start = source.index(signature)
    opening = source.index('{', start)
    # This fixed upstream method uses ordinary Java strings and no text blocks.
    depth, quote, escaped, closing = 1, None, False, opening + 1
    while depth:
        c = source[closing]
        if quote:
            if escaped: escaped = False
            elif c == '\\': escaped = True
            elif c == quote: quote = None
        elif c in ('"', "'"): quote = c
        elif c == '{': depth += 1
        elif c == '}': depth -= 1
        closing += 1
    return source[:opening + 1] + body + source[closing - 1:]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', type=Path, help='Existing clean checkout at the pinned commit; otherwise fetch into private runtime')
    parser.add_argument('--runtime-dir', type=Path, default=Path.home() / '.local/state/stepx-companion/neo-agent-build')
    parser.add_argument('--jdk', type=Path, default=os.environ.get('STEPX_JDK') or os.environ.get('JAVA_HOME'))
    parser.add_argument('--sdk', type=Path, default=os.environ.get('ANDROID_SDK_ROOT') or os.environ.get('ANDROID_HOME'))
    parser.add_argument('--signing-dir', type=Path, default=Path(os.environ.get('XDG_CONFIG_HOME', Path.home()/'.config')) / 'stepx-companion/signing')
    parser.add_argument('--embedded', action='store_true', help='Build a single private StepX Companion APK containing the Neo runtime')
    args = parser.parse_args()
    if not args.jdk or not args.sdk:
        parser.error('Set --jdk/JAVA_HOME and --sdk/ANDROID_SDK_ROOT')
    if args.embedded:
        run([sys.executable,Path(__file__).with_name('test_embedded.py')])
    runtime = args.runtime_dir.expanduser().resolve()
    if runtime == ROOT or ROOT in runtime.parents:
        raise RuntimeError('The private build must be outside the public checkout')
    runtime.mkdir(parents=True, exist_ok=True, mode=0o700)
    if os.name != 'nt': runtime.chmod(0o700)
    upstream = args.upstream
    if upstream is None:
        upstream = runtime / 'upstream'
        if not (upstream / '.git').exists():
            upstream.mkdir(exist_ok=True)
            run(['git', 'init', upstream], stdout=subprocess.DEVNULL)
            run(['git', '-C', upstream, 'remote', 'add', 'origin', REPOSITORY])
        run(['git', '-C', upstream, 'fetch', '--depth=1', 'origin', COMMIT])
        run(['git', '-C', upstream, 'checkout', '--detach', COMMIT], stdout=subprocess.DEVNULL)
    head = subprocess.check_output(['git', '-C', str(upstream), 'rev-parse', 'HEAD'], text=True).strip()
    if head != COMMIT:
        raise RuntimeError('Upstream commit does not match the reviewed source')
    clean = subprocess.check_output(['git', '-C', str(upstream), 'status', '--porcelain', '--', 'guides/neo-agent-touch/source'], text=True)
    if clean.strip():
        raise RuntimeError('Upstream source has local modifications; use the exact reviewed commit')
    source = upstream / 'guides/neo-agent-touch/source'
    stage = runtime / ('build-' + COMMIT[:12] + ('-embedded' if args.embedded else ''))
    if stage.exists(): shutil.rmtree(stage)
    shutil.copytree(source, stage, ignore=shutil.ignore_patterns('out', 'dist', 'tests', '.git'))
    # Relocate only our private build, preserving the original upstream checkout.
    for base in ('src', 'release-src', 'res', 'assets'):
        for path in (stage / base).rglob('*'):
            if path.is_file() and path.suffix in ('.java', '.xml', '.html', '.js', '.json', '.css'):
                path.write_text(path.read_text('utf-8').replace(ORIGINAL, PACKAGE), 'utf-8')
    manifest = ET.parse(stage / 'AndroidManifest.xml')
    root = manifest.getroot(); root.set('package', PACKAGE)
    app = root.find('application')
    app.set(ANDROID+'label', 'Neo Agent Touch · StepX适配')
    # Our adapter has a separate package/signature; official APK updates cannot
    # replace it. Preserve upstream authorship and offer the original release page.
    updater = stage / 'src/studio/jiayu/neoagent/AppUpdater.java'
    text = updater.read_text('utf-8')
    body = 'message="此为 StepX 本地适配版；请通过伴侣更新，原作者发布页保留供参考";phase="idle";busy=false;candidate=null;emit();'
    for method in ('public void check()', 'public void downloadAndInstall()'):
        text = rewrite_method(text, method, body)
    updater.write_text(text, 'utf-8')
    activity = stage / 'src/studio/jiayu/neoagent/MainActivity.java'
    text = activity.read_text('utf-8')
    anchor = 'void takeover(){'
    if text.count(anchor) != 1:
        raise RuntimeError('Upstream takeover entry changed; review before building')
    activity.write_text(text.replace(anchor, anchor+'CompanionBridgeRuntime.phoneTakeover();', 1), 'utf-8')
    for permission in list(root.findall('uses-permission')):
        if permission.get(ANDROID+'name') in ('android.permission.RECORD_AUDIO', 'android.permission.REQUEST_INSTALL_PACKAGES'):
            root.remove(permission)
    for provider in app.findall('provider'):
        authority = provider.get(ANDROID+'authorities')
        if authority: provider.set(ANDROID+'authorities', authority.replace(ORIGINAL, PACKAGE))
    queries = root.find('queries')
    ET.SubElement(queries, 'package', {ANDROID+'name':'com.stepx.companion.app'})
    ET.SubElement(app, 'provider', {ANDROID+'name':'.CompanionBridgeProvider', ANDROID+'authorities':PACKAGE+'.control', ANDROID+'exported':'true'})
    service = ET.SubElement(app, 'service', {ANDROID+'name':'.CompanionMdiService', ANDROID+'exported':'false', ANDROID+'foregroundServiceType':'specialUse'})
    ET.SubElement(service,'property',{ANDROID+'name':'android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE',ANDROID+'value':'Explicit user-started virtual-screen task with pause and stop controls; no unattended restart.'})
    manifest.write(stage / 'AndroidManifest.xml', encoding='utf-8', xml_declaration=True)
    adapter = ROOT / 'desktop/neo-agent-adapter'
    for path in (adapter / 'src').rglob('*.java'):
        shutil.copy2(path, stage / 'src/studio/jiayu/neoagent' / path.name)
    instrument(stage)
    notice = ('Neo Agent Touch by JiayuStudio\nUpstream: '+REPOSITORY+'\nCommit: '+COMMIT+'\n'
              'Original authors retain their copyright. This is a private StepX compatibility build.\n'
              'StepX adapter source is maintained separately. This APK is not an official Neo release.\n')
    (stage/'assets/STEPX_ADAPTER_NOTICE.txt').write_text(notice, 'utf-8')
    apk_package = assemble(stage, ROOT/'desktop/companion-app', PACKAGE) if args.embedded else PACKAGE
    out = stage / 'out'
    for directory in ('classes', 'dex', 'generated', 'tests'): (out/directory).mkdir(parents=True, exist_ok=True)
    sdk, jdk = args.sdk.resolve(), args.jdk.resolve()
    bt = sdk/'build-tools/36.0.0'; android = sdk/'platforms/android-36/android.jar'
    r8 = sdk/'cmdline-tools/latest/lib/r8.jar'
    javac, java = tool('javac',jdk), tool('java',jdk)
    run([tool('aapt2',bt),'compile','--dir',stage/'res','-o',out/'compiled.zip'])
    assets = out/'assets'
    shutil.copytree(stage/'assets', assets, ignore=shutil.ignore_patterns('voice'))
    extra_resources = ['--extra-packages',PACKAGE] if args.embedded else []
    run([tool('aapt2',bt),'link','-o',out/'unsigned.apk','-I',android,'--manifest',stage/'AndroidManifest.xml','--java',out/'generated',*extra_resources,'-A',assets,out/'compiled.zip'])
    sources = [p for p in (stage/'src').rglob('*.java') if p.name != 'OfflineVoice.java']
    sources += list((stage/'release-src').rglob('*.java')) + list((out/'generated').rglob('*.java'))
    dependencies = []
    if args.embedded:
        sources += list((stage/'companion-src').rglob('*.java'))
        zxing = stage/'companion-libs/zxing-core-3.5.4.jar'
        if hashlib.sha256(zxing.read_bytes()).hexdigest() != '71de5d89341b5fcf5dd89da7f44e84d825d0e084cdf3ec77c9abe26b0f0ceb13':
            raise RuntimeError('Companion ZXing dependency hash mismatch')
        dependencies.append(zxing)
    compile_args = ['-encoding','UTF-8','-source','8','-target','8','-bootclasspath',str(android.parent/'core-for-system-modules.jar')+os.pathsep+str(android)]
    if dependencies:
        compile_args += ['-classpath',os.pathsep.join(map(str,dependencies))]
    compile_args += ['-d',out/'classes',*sources]
    # An argument file avoids Windows' CreateProcess command length limit.
    argfile = out/'compile.args'
    argfile.write_text('\n'.join('"'+str(x).replace('\\','\\\\').replace('"','\\"')+'"' for x in compile_args)+'\n','utf-8')
    run([javac,'@'+str(argfile)])
    tests = list((adapter/'tests').glob('*.java'))
    run([javac,'-encoding','UTF-8','-source','8','-target','8','-classpath',out/'classes','-d',out/'tests',*tests])
    run([java,'-cp',str(out/'classes')+os.pathsep+str(out/'tests'),PACKAGE+'.BridgePolicyTest'])
    run([java,'-cp',str(out/'classes')+os.pathsep+str(out/'tests'),PACKAGE+'.CompanionModelPolicyTest'])
    run([java,'-cp',str(out/'classes')+os.pathsep+str(out/'tests'),PACKAGE+'.CompanionSearchTargetTest'])
    run([java,'-cp',str(out/'classes')+os.pathsep+str(out/'tests'),PACKAGE+'.CompanionCompletionRulesTest'])
    run([java,'-cp',str(out/'classes')+os.pathsep+str(out/'tests'),PACKAGE+'.CompanionMdiCoordinatesTest'])
    if args.embedded:
        host_tests = list((stage/'companion-tests').glob('*.java'))
        classpath = os.pathsep.join(map(str,[out/'classes',android,*dependencies]))
        run([javac,'-encoding','UTF-8','-source','11','-target','11','-classpath',classpath,'-d',out/'tests',*host_tests])
        for test in ('GameCoreTest','GuiTaskStateTest','GelabActionTest','GuiRetryPolicyTest','NeoPolicyTest','McpRoutePolicyTest','OfficialGuiSkillPolicyTest','AmooLeaseTest','PairingCodeTest','StudioWebPolicyTest','StudioLocalPolicyTest'):
            run([java,'-cp',str(out/'tests')+os.pathsep+classpath,apk_package+'.'+test])
    run([tool('jar',jdk),'cf',out/'classes.jar','-C',out/'classes','.'])
    run([java,'-cp',r8,'com.android.tools.r8.D8','--release','--min-api','33' if args.embedded else '30','--lib',android,'--output',out/'dex',out/'classes.jar',*dependencies])
    with zipfile.ZipFile(out/'unsigned.apk','a',zipfile.ZIP_DEFLATED) as archive:
        for dex in (out/'dex').glob('*.dex'): archive.write(dex,dex.name)
    run([tool('zipalign',bt),'-f','4',out/'unsigned.apk',out/'aligned.apk'])
    key, password = args.signing_dir/'companion.p12', args.signing_dir/'companion.pass'
    if not key.is_file() or not password.is_file():
        raise RuntimeError('Build StepX Companion first; this adapter must use its existing signing identity')
    apk = runtime/('stepx-companion-neo-embedded-local.apk' if args.embedded else 'stepx-neo-agent-local.apk')
    env = dict(os.environ, JAVA_HOME=str(jdk), PATH=str(jdk/'bin')+os.pathsep+os.environ.get('PATH',''))
    signer = bt/('apksigner.bat' if os.name == 'nt' else 'apksigner')
    pending_apk=apk.with_suffix('.pending.apk')
    run([signer,'sign','--v4-signing-enabled','false','--ks',key,'--ks-key-alias','companion','--ks-pass','file:'+str(password),'--out',pending_apk,out/'aligned.apk'],env=env)
    run([signer,'verify',pending_apk],env=env)
    pending_apk.replace(apk)
    digest = hashlib.sha256(apk.read_bytes()).hexdigest()
    metadata = dict(package=apk_package,authority=('com.stepx.companion.app.neo.control' if args.embedded else PACKAGE+'.control'),enginePackage=PACKAGE,embedded=args.embedded,commit=COMMIT,upstreamVersion='0.1.16-beta.1',adapterVersion=1,sha256=digest,apk=str(apk),privateBuild=True)
    (runtime/('build-receipt-embedded.json' if args.embedded else 'build-receipt.json')).write_text(json.dumps(metadata,ensure_ascii=False,indent=2)+'\n','utf-8')
    print(json.dumps(metadata,ensure_ascii=False))

if __name__ == '__main__':
    try: main()
    except (RuntimeError, subprocess.CalledProcessError) as failure:
        sys.exit(str(failure))
