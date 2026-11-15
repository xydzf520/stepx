#!/usr/bin/env python3
"""Build a portable package from the native release binary and verified upstream tools."""
import argparse, hashlib, os, platform, plistlib, posixpath, re, shutil, stat, struct, subprocess, sys, tarfile, tempfile, urllib.parse, urllib.request, zipfile
from pathlib import Path
sys.dont_write_bytecode = True
from public_release_policy import audit_bundle, content_problems, manifest, runtime_plan, safe_read
from windows_crt import package_windows_crt, verify_manifest
from release_download import download_verified_tool

ROOT = Path(__file__).resolve().parents[1]
TOOLS = {
    'linux-x86_64': ('scrcpy-linux-x86_64-v5.0.1.tar.gz', '9f969d30cc574816077edecda65719c1c70b0aee1df1ba5fa00779ac07b8fd6e', 620992699),
    'macos-arm64': ('scrcpy-macos-aarch64-v5.0.1.tar.gz', '33611e51977a8289e2e124b220f727895181ef2eb9060e0987a4dc2db9cf0d6b', 620992787),
    'macos-x86_64': ('scrcpy-macos-x86_64-v5.0.1.tar.gz', '31a5467a9e907f162b093267cbbab1276b15c6e709c2ea8159566a8d165beb50', 620992832),
    'windows-x86_64': ('scrcpy-win64-v5.0.1.zip', 'b12a2c4ee8be317422451fc7dcf8ee20a71b5ea7ef9ad73ddd825a25316227a5', 620992965),
}
SINGLE_MACHO_MAGICS = frozenset({0xfeedface, 0xcefaedfe, 0xfeedfacf, 0xcffaedfe})
FAT_MACHO_MAGICS = frozenset({0xcafebabe, 0xbebafeca, 0xcafebabf, 0xbfbafeca})
LC_VERSION_MIN_MACOSX = 0x24
LC_BUILD_VERSION = 0x32
PLATFORM_MACOS = 1


def macos_info_plist(version: str) -> dict:
    return {
        'CFBundleInfoDictionaryVersion': '6.0',
        'CFBundleName': 'StepX Companion',
        'CFBundleDisplayName': 'StepX Companion',
        'CFBundleIdentifier': 'io.github.xydzf520.stepx',
        'CFBundleExecutable': 'stepx-companion',
        'CFBundlePackageType': 'APPL',
        'CFBundleShortVersionString': version,
        'CFBundleVersion': version,
        'LSMinimumSystemVersion': '11.0',
        'NSHighResolutionCapable': True,
        'NSSupportsAutomaticGraphicsSwitching': True,
        'NSMicrophoneUsageDescription': '用于语音输入',
        'NSCameraUsageDescription': '用于扫码连接',
        'NSLocalNetworkUsageDescription': '用于在局域网发现并连接您授权的手机。',
        'NSBonjourServices': [
            '_adb-tls-pairing._tcp',
            '_adb-tls-connect._tcp',
            '_adb._tcp',
            '_airplay._tcp',
            '_googlecast._tcp',
            '_raop._tcp',
        ],
        'CFBundleIconFile': 'StepX.icns',
    }


def is_macho_file(path: Path) -> bool:
    if path.is_symlink() or not path.is_file():
        return False
    try:
        with path.open('rb') as handle:
            hdr = handle.read(8)
    except OSError:
        return False
    if len(hdr) < 8:
        return False
    magic = int.from_bytes(hdr[:4], 'big')
    if magic in SINGLE_MACHO_MAGICS:
        return True
    if magic in FAT_MACHO_MAGICS:
        endian = 'big' if hdr[:4] in (b'\xca\xfe\xba\xbe', b'\xca\xfe\xba\xbf') else 'little'
        return 1 <= int.from_bytes(hdr[4:8], endian) <= 20
    return False


def macho_binaries(root: Path) -> list[Path]:
    """Return regular non-symlink Mach-O files under root sorted deepest-first."""
    root = Path(root)
    matches = [path for path in root.rglob('*') if is_macho_file(path)]
    return sorted(matches, key=lambda p: (-len(p.relative_to(root).parts), p.relative_to(root).as_posix()))


def _slice_macos_minos(data: bytes, label: str) -> tuple[int, int, int]:
    if len(data) < 28:
        raise ValueError(f'{label}: truncated Mach-O slice')
    raw_magic = data[:4]
    if raw_magic == b'\xfe\xed\xfa\xce':
        endian, header_size = '>', 28
    elif raw_magic == b'\xce\xfa\xed\xfe':
        endian, header_size = '<', 28
    elif raw_magic == b'\xfe\xed\xfa\xcf':
        endian, header_size = '>', 32
    elif raw_magic == b'\xcf\xfa\xed\xfe':
        endian, header_size = '<', 32
    else:
        raise ValueError(f'{label}: invalid single-arch Mach-O magic')
    if len(data) < header_size:
        raise ValueError(f'{label}: truncated Mach-O header')
    ncmds, sizeofcmds = struct.unpack_from(endian + 'II', data, 16)
    pos = header_size
    end = pos + sizeofcmds
    if end > len(data):
        raise ValueError(f'{label}: Mach-O load commands exceed file size')
    versions = []
    for _ in range(ncmds):
        if pos + 8 > end:
            raise ValueError(f'{label}: truncated Mach-O load command')
        cmd, cmdsize = struct.unpack_from(endian + 'II', data, pos)
        if cmdsize < 8 or pos + cmdsize > end:
            raise ValueError(f'{label}: invalid Mach-O load command size')
        if cmd == LC_BUILD_VERSION:
            if cmdsize < 24:
                raise ValueError(f'{label}: truncated LC_BUILD_VERSION')
            platform_id, raw_minos = struct.unpack_from(endian + 'II', data, pos + 8)
            if platform_id == PLATFORM_MACOS:
                versions.append((raw_minos >> 16, (raw_minos >> 8) & 0xff, raw_minos & 0xff))
        elif cmd == LC_VERSION_MIN_MACOSX:
            if cmdsize < 16:
                raise ValueError(f'{label}: truncated LC_VERSION_MIN_MACOSX')
            raw_minos, = struct.unpack_from(endian + 'I', data, pos + 8)
            versions.append((raw_minos >> 16, (raw_minos >> 8) & 0xff, raw_minos & 0xff))
        pos += cmdsize
    if not versions:
        raise RuntimeError(f'{label}: missing LC_BUILD_VERSION / LC_VERSION_MIN_MACOSX load command')
    return max(versions)


def verify_macos_minos(path: Path, max_minos: tuple[int, ...] = (11, 0)) -> list[tuple[int, int, int]]:
    """Verify Mach-O LC_BUILD_VERSION / LC_VERSION_MIN_MACOSX minos <= max_minos."""
    path = Path(path)
    data = path.read_bytes()
    if len(data) < 8:
        raise ValueError(f'{path.name}: not a Mach-O executable')
    raw_magic = data[:4]
    magic = int.from_bytes(raw_magic, 'big')
    slices = []
    if magic in FAT_MACHO_MAGICS:
        endian = '>' if raw_magic in (b'\xca\xfe\xba\xbe', b'\xca\xfe\xba\xbf') else '<'
        fat64 = raw_magic in (b'\xca\xfe\xba\xbf', b'\xbf\xba\xfe\xca')
        count, = struct.unpack_from(endian + 'I', data, 4)
        if not (1 <= count <= 20):
            raise ValueError(f'{path.name}: invalid Fat Mach-O architecture count')
        stride = 32 if fat64 else 20
        if len(data) < 8 + count * stride:
            raise ValueError(f'{path.name}: truncated Fat Mach-O header')
        for idx in range(count):
            entry = 8 + idx * stride
            offset, size = struct.unpack_from(endian + ('QQ' if fat64 else 'II'), data, entry + 8)
            if offset + size > len(data) or size == 0:
                raise ValueError(f'{path.name}: Fat Mach-O slice outside file')
            slices.append(data[offset:offset + size])
    elif magic in SINGLE_MACHO_MAGICS:
        slices.append(data)
    else:
        raise ValueError(f'{path.name}: expected Mach-O executable')
    limit = (*max_minos, 0, 0)[:3]
    versions = [_slice_macos_minos(chunk, path.name) for chunk in slices]
    for version in versions:
        if version > limit:
            rendered = '.'.join(map(str, version[:2] if version[2] == 0 else version))
            target = '.'.join(map(str, limit[:2] if limit[2] == 0 else limit))
            raise RuntimeError(f'{path.name}: macOS minos {rendered} exceeds {target}')
    return versions


def clean_tar_info(info: tarfile.TarInfo) -> tarfile.TarInfo:
    """Strip host UID/GID and username/groupname from Linux release tarballs."""
    info.uid = 0
    info.gid = 0
    info.uname = ''
    info.gname = ''
    return info


def sign_macos_bundle(base: Path, binary_path: Path, resource_dir: Path, tools_dir: Path, tool_env: dict) -> list[Path]:
    subprocess.run(['xattr','-cr',str(base/'StepX Companion.app')],check=True)
    nested_machos = macho_binaries(resource_dir)
    for macho in [*nested_machos, binary_path]:
        mode = macho.stat().st_mode
        if not (mode & stat.S_IWUSR):
            macho.chmod(mode | stat.S_IWUSR)
    for macho in nested_machos:
        subprocess.run(['codesign', '--force', '--sign', '-', str(macho)], check=True)
    subprocess.run(['codesign','--force','--deep','--sign','-',str(base/'StepX Companion.app')],check=True)
    subprocess.run([str(binary_path),'--check-runtime'],check=True,env=verification_environment(),timeout=45)
    subprocess.run([str(tools_dir/'scrcpy'),'--version'],check=True,env=tool_env,timeout=30)
    for macho in nested_machos:
        subprocess.run(['codesign', '--verify', '--strict', str(macho)], check=True)
    subprocess.run(['codesign','--verify','--deep','--strict',str(base/'StepX Companion.app')],check=True)
    return nested_machos

def verification_environment():
    """Never let validation mutate the staged runtime or an inherited cache path."""
    env = os.environ.copy()
    env.pop('PYTHONPYCACHEPREFIX', None)
    env.update(PYTHONDONTWRITEBYTECODE='1', PYTHONUTF8='1', PYTHONIOENCODING='utf-8')
    return env

def verify_runtime(python, resources):
    # -B takes effect before Python imports encodings/site, which happens before
    # the verifier can set an environment variable or inspect the staged tree.
    subprocess.run([str(python), '-B', '-X', 'utf8',
                    str(ROOT/'scripts/verify-packaged-runtime.py'), str(resources)],
                   check=True, env=verification_environment(), timeout=180)

def package_document(rel, data, version, packaged_paths):
    """Keep bundled links local; pin omitted documentation/media to this release.

    The repository README is also shipped offline. Its screenshots, longer
    guides and separately downloadable videos must not become dead local links.
    Source documentation stays unchanged; only the new package copy is rendered.
    """
    if not rel.lower().endswith('.md'):
        return data
    tag = urllib.parse.quote('v' + version, safe='')

    def destination(value):
        angled = value.startswith('<') and value.endswith('>')
        raw = value[1:-1] if angled else value
        parts = urllib.parse.urlsplit(raw)
        if parts.scheme or parts.netloc or not parts.path or parts.path.startswith('/'):
            return value
        resolved = posixpath.normpath(posixpath.join(
            posixpath.dirname(rel), urllib.parse.unquote(parts.path)))
        if resolved in packaged_paths:
            return value
        if resolved == '..' or resolved.startswith('../'):
            raise ValueError(f'{rel}: documentation link leaves the source tree')
        encoded = urllib.parse.quote(resolved, safe='/')
        if resolved.startswith('docs/videos/') and resolved.lower().endswith('.mp4'):
            # A maintenance release can reuse an older product demonstration.
            # Its filename records the release that actually owns the asset.
            recorded = re.fullmatch(r'StepX-(?:product|\d+\.\d+\.\d+-to)-(\d+\.\d+\.\d+)\.mp4',
                                    posixpath.basename(resolved))
            media_tag = 'v' + recorded[1] if recorded else tag
            url = ('https://github.com/xydzf520/stepx/releases/download/' + media_tag +
                   '/' + urllib.parse.quote(posixpath.basename(resolved), safe=''))
        elif resolved.lower().endswith(('.png', '.jpg', '.jpeg', '.gif', '.webp', '.svg')):
            url = 'https://raw.githubusercontent.com/xydzf520/stepx/' + tag + '/' + encoded
        else:
            url = 'https://github.com/xydzf520/stepx/blob/' + tag + '/' + encoded
        if parts.query:
            url += '?' + parts.query
        if parts.fragment:
            url += '#' + parts.fragment
        return '<' + url + '>' if angled else url

    # Preserve labels, optional titles and reference-link syntax. Do not rewrite
    # examples inside fenced code blocks.
    inline = re.compile(r'(!?\[[^\]\n]*\]\()(<[^>\n]+>|[^\s)]+)')
    reference = re.compile(r'^(\[[^\]\n]+\]:[ \t]*)(<[^>\n]+>|[^\s]+)', re.MULTILINE)
    chunks = re.split(r'(^[ \t]*```[^\n]*\n.*?^[ \t]*```[^\n]*$|^[ \t]*~~~[^\n]*\n.*?^[ \t]*~~~[^\n]*$)',
                      data.decode('utf-8'), flags=re.MULTILINE | re.DOTALL)
    for index in range(0, len(chunks), 2):
        for pattern in (inline, reference):
            chunks[index] = pattern.sub(lambda m: m[1] + destination(m[2]), chunks[index])
    return ''.join(chunks).encode('utf-8')


def stage_release_media(root, dist, version):
    """Maintenance releases may omit both new videos, never publish half a pair."""
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise ValueError('Invalid media release version')
    root, dist = Path(root), Path(dist)
    videos = [root/'docs/videos'/name for name in
              (f'StepX-product-{version}.mp4', f'StepX-0.2.60-to-{version}.mp4')]
    present = [path.exists() for path in videos]
    if any(present) and not all(present):
        raise ValueError('Release videos must include both product and update, or neither')
    inputs = [(root/'desktop/assets/stepx-companion.apk', 'StepX-companion.apk')]
    if all(present):
        inputs += [(path, path.name) for path in videos]
    if any(path.is_symlink() or not path.is_file() or path.stat().st_size == 0 for path, _ in inputs):
        raise ValueError('Release media is missing, empty, or a symlink')
    dist.mkdir(parents=True, exist_ok=True)
    checksums = []
    for source, name in inputs:
        target = dist/name
        shutil.copy2(source, target)
        checksums.append(hashlib.sha256(target.read_bytes()).hexdigest() + '  ' + name)
    (dist/'media-sha256.txt').write_text('\n'.join(checksums)+'\n', encoding='utf-8')
    return [name for _, name in inputs]


def main():
    p=argparse.ArgumentParser();p.add_argument('--platform',choices=TOOLS);p.add_argument('--version',required=True);p.add_argument('--stage-media-only',action='store_true');a=p.parse_args()
    if not all(c.isalnum() or c in '.-_' for c in a.version):p.error('Invalid version')
    import tomllib
    if tomllib.loads((ROOT/'desktop/Cargo.toml').read_text())['package']['version']!=a.version: p.error('Package version must match Cargo.toml')
    if a.stage_media_only:
        if a.platform:p.error('--stage-media-only does not take --platform')
        print('Release media:', ', '.join(stage_release_media(ROOT,ROOT/'dist',a.version)))
        return
    if not a.platform:p.error('--platform is required for native packaging')
    windows=a.platform.startswith('windows');mac=a.platform.startswith('macos')
    host_os={'Darwin':'macos','Windows':'windows','Linux':'linux'}.get(platform.system())
    host_arch={'AMD64':'x86_64','aarch64':'arm64'}.get(platform.machine(),platform.machine())
    if a.platform != f'{host_os}-{host_arch}': p.error('Use a native runner for the requested platform; cross-built packages cannot be runtime-verified here')
    policy=manifest(ROOT)
    resources = runtime_plan(ROOT)  # fail closed before staging any resource
    documents={rel:safe_read(ROOT,rel) for rel in policy['package_documents']}
    issues=[issue for rel,data in documents.items() for issue in content_problems(rel,data,policy)]
    if issues: raise RuntimeError('\n'.join(issues))
    packaged_paths = set(resources) | set(documents)
    documents = {rel: package_document(rel, data, a.version, packaged_paths)
                 for rel, data in documents.items()}
    dist=ROOT/'dist';dist.mkdir(exist_ok=True)
    name=f'StepX-Companion-{a.version}-{a.platform}'
    with tempfile.TemporaryDirectory(prefix='stepx-package-') as td:
        stage=Path(td);base=stage/name;base.mkdir()
        if mac:
            bundle=base/'StepX Companion.app'/'Contents';binary_dir=bundle/'MacOS';resource_dir=bundle/'Resources'
            binary_dir.mkdir(parents=True);resource_dir.mkdir()
            info=macos_info_plist(a.version)
            iconset=stage/'StepX.iconset';iconset.mkdir()
            for size in [16,32,128,256,512]:
                for scale in [1,2]:
                    suffix='@2x' if scale==2 else ''
                    subprocess.run(['sips','-z',str(size*scale),str(size*scale),str(ROOT/'desktop/assets/stepx_companion_icon.png'),'--out',str(iconset/f'icon_{size}x{size}{suffix}.png')],check=True,stdout=subprocess.DEVNULL)
            subprocess.run(['iconutil','-c','icns',str(iconset),'-o',str(resource_dir/'StepX.icns')],check=True)
            (bundle/'Info.plist').write_bytes(plistlib.dumps(info))
        else:binary_dir=resource_dir=base
        binary_name='stepx-companion'+('.exe' if windows else '')
        shutil.copy2(ROOT/'desktop'/'target'/'release'/binary_name,binary_dir/binary_name)
        if mac:
            verify_macos_minos(binary_dir/binary_name)
        if windows:
            crt = package_windows_crt(binary_dir/binary_name)
            print('App-local Visual C++ runtime:', ', '.join(crt['files']) or 'not required', flush=True)
        subprocess.run([sys.executable,'-B',str(ROOT/'scripts/stage-release-resources.py'),str(ROOT),str(resource_dir)],check=True,env=verification_environment())
        for rel,data in documents.items():
            target=resource_dir/rel;target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(data)
        filename,digest,asset_id=TOOLS[a.platform];download=stage/filename
        download_verified_tool('https://github.com/Genymobile/scrcpy/releases/download/v5.0.1/'+filename,download,digest,
                               asset_api_url=f'https://api.github.com/repos/Genymobile/scrcpy/releases/assets/{asset_id}')
        unpack=stage/'upstream';unpack.mkdir()
        if filename.endswith('.zip'):
            with zipfile.ZipFile(download) as z:z.extractall(unpack)
        else:
            with tarfile.open(download) as t:t.extractall(unpack,filter='data')
        roots=list(unpack.iterdir());source=roots[0] if len(roots)==1 and roots[0].is_dir() else unpack
        tools_dir=resource_dir/'tools/bin';shutil.copytree(source,tools_dir,symlinks=True)
        subprocess.run([sys.executable,'-B',str(ROOT/'scripts/package-runtime.py'),str(resource_dir),*(['--windows'] if windows else [])],check=True,env=verification_environment())
        tool_env=verification_environment();tool_env['LD_LIBRARY_PATH']=str(tools_dir)+os.pathsep+tool_env.get('LD_LIBRARY_PATH','')
        for executable,arg in [(binary_dir/binary_name,'--version'),(tools_dir/('adb.exe' if windows else 'adb'),'version'),(tools_dir/('scrcpy.exe' if windows else 'scrcpy'),'--version')]:
            subprocess.run([str(executable),arg],check=True,env=tool_env,timeout=30)
        python=resource_dir/'runtime/python/python.exe' if windows else resource_dir/'runtime/python/bin/python3'
        verify_runtime(python, resource_dir)
        if mac:
            sign_macos_bundle(base, binary_dir/binary_name, resource_dir, tools_dir, tool_env)
        issues=audit_bundle(resource_dir)
        if issues: raise RuntimeError('Package privacy verification failed:\n'+'\n'.join(issues))
        if windows: verify_manifest(binary_dir/binary_name)
        if a.platform.startswith('linux'):
            output=dist/(name+'.tar.gz')
            with tarfile.open(output,'w:gz') as t:t.add(base,arcname=name,filter=clean_tar_info)
        else:
            output=dist/(name+'.zip')
            if mac:subprocess.run(['ditto','-c','-k','--sequesterRsrc','--keepParent',str(base),str(output)],check=True)
            else:shutil.make_archive(str(dist/name),'zip',stage,name)
        (dist/(output.name+'.sha256')).write_text(hashlib.sha256(output.read_bytes()).hexdigest()+'  '+output.name+'\n')
        print(output)
if __name__=='__main__':main()
