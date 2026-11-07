"""Package host-side installers with no dependency on a developer SDK path."""
import argparse, hashlib, os, platform, shutil, subprocess, tarfile, tempfile, urllib.request, zipfile
from pathlib import Path

PYTHON_URL='https://www.python.org/ftp/python/3.13.13/python-3.13.13-embed-amd64.zip'
PYTHON_SHA256='8766a8775746235e23cf5aee5027ab1060bb981d93110577adcf3508aa0cbd55'


PYTHON_UNIX={
 'macos-arm64':('aarch64-apple-darwin','8dc60610a82f1e73be7faf964383e02520f95f6e39592a5c37c3809f5a7b798e'),
 'macos-x86_64':('x86_64-apple-darwin','ce9c748c8f7525c0052247edb2435fdb1cac89c77e687e0e199eaf2b827668c2'),
 'linux-x86_64':('x86_64-unknown-linux-gnu','f829adc9d888e76c8801b4799ed444188d35239fcae3a9808a99ec5cb0d8d92c'),
}

def enable_windows_utf8_launcher(executable):
    # Java's native Windows launcher otherwise converts arguments using the
    # machine ANSI code page, losing Chinese paths on non-Chinese installations.
    # This is an application-local setting; the user's system locale is unchanged.
    kits=Path(os.environ.get('ProgramFiles(x86)', r'C:\Program Files (x86)'))/'Windows Kits/10/bin'
    candidates=sorted(kits.glob('*/x64/mt.exe'),reverse=True)
    if not candidates:raise RuntimeError('Windows SDK manifest tool is required when packaging Java')
    with tempfile.TemporaryDirectory() as td:
        manifest=Path(td)/'java.manifest'
        subprocess.run([str(candidates[0]),'-nologo','-inputresource:'+str(executable)+';#1','-out:'+str(manifest)],check=True)
        import xml.etree.ElementTree as ET
        tree=ET.parse(manifest);root=tree.getroot()
        namespace='urn:schemas-microsoft-com:asm.v3'
        application=root.find('{'+namespace+'}application')
        if application is None:application=ET.SubElement(root,'{'+namespace+'}application')
        settings=application.find('{'+namespace+'}windowsSettings')
        if settings is None:settings=ET.SubElement(application,'{'+namespace+'}windowsSettings')
        tag='{http://schemas.microsoft.com/SMI/2019/WindowsSettings}activeCodePage'
        codepage=settings.find(tag)
        if codepage is None:codepage=ET.SubElement(settings,tag)
        codepage.text='UTF-8'
        tree.write(manifest,encoding='utf-8',xml_declaration=True)
        subprocess.run([str(candidates[0]),'-nologo','-manifest',str(manifest),'-outputresource:'+str(executable)+';#1'],check=True)

def build_runtime(root, windows):
    runtime=root/'runtime';runtime.mkdir(exist_ok=True)
    sdk=Path(os.environ.get('ANDROID_SDK_ROOT') or os.environ['ANDROID_HOME'])
    source=sdk/'build-tools/36.0.0'
    target=runtime/'android-build-tools';target.mkdir()
    (target/'lib').mkdir();shutil.copy2(source/'lib/apksigner.jar',target/'lib/apksigner.jar')
    for name in ['NOTICE.txt','source.properties']:
        if (source/name).is_file():shutil.copy2(source/name,target/name)
    java=Path(os.environ['JAVA_HOME'])
    # Native SDK aapt may require Rosetta on Apple Silicon and mishandle Windows
    # Unicode paths. Use the same official apksig parser on the bundled JVM.
    with tempfile.TemporaryDirectory() as directory:
        classes=Path(directory)/'classes';classes.mkdir()
        helper=Path(__file__).parent/'runtime/ApkIdentity.java'
        subprocess.run([str(java/'bin'/('javac.exe' if windows else 'javac')),
            '--release','11','-encoding','UTF-8','-cp',str(source/'lib/apksigner.jar'),
            '-d',str(classes),str(helper)],check=True)
        subprocess.run([str(java/'bin'/('jar.exe' if windows else 'jar')),
            '--create','--file',str(target/'lib/apk-identity.jar'),'-C',str(classes),'.'],check=True)
    if (java/'jmods').is_dir():
        subprocess.run([str(java/'bin'/('jlink.exe' if windows else 'jlink')),
            '--add-modules','java.base,java.logging,java.naming,java.xml,java.security.jgss,jdk.crypto.ec',
            '--strip-debug','--no-header-files','--no-man-pages','--output',str(runtime/'java')],check=True)
    else:
        raise RuntimeError('Packaging requires a JDK with jmods/jlink; arbitrary JAVA_HOME trees are never copied into releases')
    if windows:enable_windows_utf8_launcher(runtime/'java/bin/java.exe')
    if windows:
        with tempfile.TemporaryDirectory() as directory:
            archive=Path(directory)/'python.zip';urllib.request.urlretrieve(PYTHON_URL,archive)
            if hashlib.sha256(archive.read_bytes()).hexdigest()!=PYTHON_SHA256:raise RuntimeError('Python digest mismatch')
            with zipfile.ZipFile(archive) as z:z.extractall(runtime/'python')
        # Keep isolated search paths, adding only bundled application modules.
        (runtime/'python/python313._pth').write_text('python313.zip\n.\n../../scripts\n../../handoff/google-play-stepx/tools\nimport site\n',encoding='utf-8')
        # Every bundled helper and its descendants hide their own console.
        (runtime/'python/sitecustomize.py').write_text('''import subprocess, sys
sys.dont_write_bytecode = True
_original = subprocess.Popen
class QuietPopen(_original):
    def __init__(self, *args, **kwargs):
        kwargs['creationflags'] = kwargs.get('creationflags', 0) | subprocess.CREATE_NO_WINDOW
        super().__init__(*args, **kwargs)
subprocess.Popen = QuietPopen
''',encoding='utf-8')
    else:
        key=('macos-' if platform.system()=='Darwin' else 'linux-')+('arm64' if platform.machine() in ('arm64','aarch64') else 'x86_64')
        triple,digest=PYTHON_UNIX[key]
        url='https://github.com/astral-sh/python-build-standalone/releases/download/20261021/cpython-3.13.17%2B20261021-'+triple+'-install_only_stripped.tar.gz'
        with tempfile.TemporaryDirectory() as directory:
            archive=Path(directory)/'python.tar.gz';urllib.request.urlretrieve(url,archive)
            if hashlib.sha256(archive.read_bytes()).hexdigest()!=digest:raise RuntimeError('Python digest mismatch')
            with tarfile.open(archive) as t:t.extractall(runtime,filter='data')
    # Upstream distributions can include bytecode caches. Rebuildable caches
    # are not release data; only prune this newly constructed temporary tree.
    for cache in (runtime/'python').rglob('__pycache__'):
        shutil.rmtree(cache)
    return runtime

def bundle(root, windows):
    # A failed dependency download must leave the previously installed runtime intact.
    root.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.runtime-build-',dir=root) as td:
        work=Path(td);built=build_runtime(work,windows)
        destination=root/'runtime';previous=work/'previous'
        if destination.exists():destination.rename(previous)
        try:built.rename(destination)
        except BaseException:
            if previous.exists():previous.rename(destination)
            raise
    return root/'runtime'

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('root',type=Path);p.add_argument('--windows',action='store_true');a=p.parse_args();bundle(a.root,a.windows)
