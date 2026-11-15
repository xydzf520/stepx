"""Package host-side installers with no dependency on a developer SDK path."""
import argparse, hashlib, os, platform, shutil, subprocess, tarfile, tempfile, urllib.request, zipfile
from pathlib import Path
from release_download import download_verified_tool

PYTHON_URL='https://www.python.org/ftp/python/3.13.13/python-3.13.13-embed-amd64.zip'
PYTHON_FALLBACK_URL='https://registry.npmmirror.com/-/binary/python/3.13.13/python-3.13.13-embed-amd64.zip'
WIN_PY_SHA256='8766a8775746235e23cf5aee5027ab1060bb981d93110577adcf3508aa0cbd55'
PYTHON_SHA256=WIN_PY_SHA256

SITE_CUSTOMIZE = '''import subprocess, sys
sys.dont_write_bytecode = True
_original = subprocess.Popen
class QuietPopen(_original):
    def __init__(self, *args, **kwargs):
        flags = kwargs.get('creationflags') or 0
        if not (flags & (subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_CONSOLE)):
            kwargs['creationflags'] = flags | subprocess.CREATE_NO_WINDOW
        super().__init__(*args, **kwargs)
subprocess.Popen = QuietPopen
'''


PYTHON_UNIX={
 'macos-arm64':('aarch64-apple-darwin','8dc60610a82f1e73be7faf964383e02520f95f6e39592a5c37c3809f5a7b798e',663544854),
 'macos-x86_64':('x86_64-apple-darwin','ce9c748c8f7525c0052247edb2435fdb1cac89c77e687e0e199eaf2b827668c2',663545760),
 'linux-x86_64':('x86_64-unknown-linux-gnu','f829adc9d888e76c8801b4799ed444188d35239fcae3a9808a99ec5cb0d8d92c',663545914),
}


def sort_mt_candidates(paths):
    return sorted(
        paths,
        key=lambda p: tuple(int(x) for x in p.parent.parent.name.split('.') if x.isdigit()),
        reverse=True,
    )


def strip_pe_authenticode(executable):
    """Remove PE Authenticode certificates invalidated after manifest edits."""
    import ctypes
    from ctypes import wintypes

    CERT_SECTION_TYPE_ANY = 0xFF
    GENERIC_READ_WRITE = 0x80000000 | 0x40000000
    FILE_SHARE_READ = 0x00000001
    OPEN_EXISTING = 3
    FILE_ATTRIBUTE_NORMAL = 0x00000080
    INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value

    kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel32.CreateFileW.argtypes = [
        wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
        ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE,
    ]
    kernel32.CreateFileW.restype = wintypes.HANDLE
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL

    imagehlp = ctypes.WinDLL('imagehlp', use_last_error=True)
    imagehlp.ImageEnumerateCertificates.argtypes = [
        wintypes.HANDLE, wintypes.WORD, ctypes.POINTER(wintypes.DWORD),
        ctypes.POINTER(wintypes.DWORD), wintypes.DWORD,
    ]
    imagehlp.ImageEnumerateCertificates.restype = wintypes.BOOL
    imagehlp.ImageRemoveCertificate.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    imagehlp.ImageRemoveCertificate.restype = wintypes.BOOL

    handle = kernel32.CreateFileW(
        str(executable), GENERIC_READ_WRITE, FILE_SHARE_READ,
        None, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, None,
    )
    if handle in (None, 0, INVALID_HANDLE_VALUE):
        raise OSError(ctypes.get_last_error(), f'CreateFileW failed for {executable}')
    try:
        count = wintypes.DWORD(0)
        if not imagehlp.ImageEnumerateCertificates(
            handle, CERT_SECTION_TYPE_ANY, ctypes.byref(count), None, 0
        ):
            raise OSError(ctypes.get_last_error(), f'ImageEnumerateCertificates failed for {executable}')
        for index in range(count.value - 1, -1, -1):
            if not imagehlp.ImageRemoveCertificate(handle, index):
                raise OSError(ctypes.get_last_error(), f'ImageRemoveCertificate({index}) failed for {executable}')
        return count.value
    finally:
        kernel32.CloseHandle(handle)


def download_windows_python(archive):
    archive = Path(archive)
    last_error = None
    for candidate in (PYTHON_URL, PYTHON_FALLBACK_URL):
        try:
            download_verified_tool(candidate, archive, WIN_PY_SHA256)
            return
        except Exception as error:
            last_error = error
            archive.unlink(missing_ok=True)
    raise last_error


def enable_windows_utf8_launcher(executable):
    # Java's native Windows launcher otherwise converts arguments using the
    # machine ANSI code page, losing Chinese paths on non-Chinese installations.
    # This is an application-local setting; the user's system locale is unchanged.
    kits=Path(os.environ.get('ProgramFiles(x86)', r'C:\Program Files (x86)'))/'Windows Kits/10/bin'
    candidates=sort_mt_candidates(kits.glob('*/x64/mt.exe'))
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
    strip_pe_authenticode(executable)

def resolve_jdk_with_jmods(windows):
    suffix='.exe' if windows else ''
    candidates=[]
    if os.environ.get('JAVA_HOME'):
        candidates.append(Path(os.environ['JAVA_HOME']))
    if not windows:
        if platform.system()=='Darwin' and Path('/usr/libexec/java_home').is_file():
            res=subprocess.run(['/usr/libexec/java_home'],capture_output=True,text=True)
            if res.returncode==0 and res.stdout.strip():
                candidates.append(Path(res.stdout.strip()))
        jvm_root=Path('/usr/lib/jvm')
        if jvm_root.is_dir():
            for entry in sorted(jvm_root.iterdir(),reverse=True):
                if entry.is_dir():candidates.append(entry)
    for candidate in candidates:
        if (candidate/'jmods').is_dir() and (candidate/'bin'/('jlink'+suffix)).is_file():
            return candidate
    raise RuntimeError('Packaging requires a JDK with jmods/jlink; arbitrary JAVA_HOME trees are never copied into releases')

def build_runtime(root, windows):
    runtime=root/'runtime';runtime.mkdir(exist_ok=True)
    sdk=Path(os.environ.get('ANDROID_SDK_ROOT') or os.environ['ANDROID_HOME'])
    source=sdk/'build-tools/36.0.0'
    target=runtime/'android-build-tools';target.mkdir()
    (target/'lib').mkdir();shutil.copy2(source/'lib/apksigner.jar',target/'lib/apksigner.jar')
    for name in ['NOTICE.txt','source.properties']:
        if (source/name).is_file():shutil.copy2(source/name,target/name)
    java=resolve_jdk_with_jmods(windows)
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
    subprocess.run([str(java/'bin'/('jlink.exe' if windows else 'jlink')),
        '--add-modules','java.base,java.logging,java.naming,java.xml,java.security.jgss,jdk.crypto.ec',
        '--strip-debug','--no-header-files','--no-man-pages','--output',str(runtime/'java')],check=True)
    if windows:enable_windows_utf8_launcher(runtime/'java/bin/java.exe')
    if windows:
        with tempfile.TemporaryDirectory() as directory:
            archive=Path(directory)/'python.zip'
            download_windows_python(archive)
            with zipfile.ZipFile(archive) as z:z.extractall(runtime/'python')
        # Keep isolated search paths, adding only bundled application modules.
        (runtime/'python/python313._pth').write_text('python313.zip\n.\n../../scripts\n../../handoff/google-play-stepx/tools\nimport site\n',encoding='utf-8')
        # Every bundled helper and its descendants hide their own console.
        (runtime/'python/sitecustomize.py').write_text(SITE_CUSTOMIZE,encoding='utf-8')
    else:
        key=('macos-' if platform.system()=='Darwin' else 'linux-')+('arm64' if platform.machine() in ('arm64','aarch64') else 'x86_64')
        triple,digest,asset_id=PYTHON_UNIX[key]
        url='https://github.com/astral-sh/python-build-standalone/releases/download/20261021/cpython-3.13.17%2B20261021-'+triple+'-install_only_stripped.tar.gz'
        with tempfile.TemporaryDirectory() as directory:
            archive=Path(directory)/'python.tar.gz'
            download_verified_tool(url,archive,digest,
                asset_api_url=f'https://api.github.com/repos/astral-sh/python-build-standalone/releases/assets/{asset_id}')
            with tarfile.open(archive) as t:t.extractall(runtime,filter='data')
    # Upstream distributions can include bytecode caches. Rebuildable caches
    # are not release data; only prune this newly constructed temporary tree.
    for cache in (runtime/'python').rglob('__pycache__'):
        shutil.rmtree(cache)
    stage_google_apks(runtime)
    return runtime

def stage_google_apks(runtime):
    import json
    cfg_path=Path(__file__).resolve().parents[1]/'handoff/google-play-stepx/target.example.json'
    cfg=json.loads(cfg_path.read_text(encoding='utf-8'))
    meta=cfg['archive']
    dest_dir=runtime/'google-play';dest_dir.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory() as directory:
        archive=Path(directory)/meta['name']
        download_verified_tool(meta['url'],archive,meta['sha256'])
        with zipfile.ZipFile(archive) as z:
            for entry in cfg['apks']:
                content=z.read(entry['zip_path'])
                if hashlib.sha256(content).hexdigest()!=entry['sha256']:
                    raise RuntimeError('Bundled Google APK digest mismatch: '+entry['package'])
                (dest_dir/entry['filename']).write_bytes(content)

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
