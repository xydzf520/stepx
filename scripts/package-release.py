#!/usr/bin/env python3
"""Build a portable package from the native release binary and verified upstream tools."""
import argparse, hashlib, http.client, os, platform, plistlib, posixpath, re, shutil, subprocess, sys, tarfile, tempfile, time, urllib.error, urllib.parse, urllib.request, zipfile
from pathlib import Path
sys.dont_write_bytecode = True
from public_release_policy import audit_bundle, content_problems, manifest, runtime_plan, safe_read
from windows_crt import package_windows_crt, verify_manifest

ROOT = Path(__file__).resolve().parents[1]
TOOLS = {
    'linux-x86_64': ('scrcpy-linux-x86_64-v5.0.1.tar.gz', '9f969d30cc574816077edecda65719c1c70b0aee1df1ba5fa00779ac07b8fd6e', 620992699),
    'macos-arm64': ('scrcpy-macos-aarch64-v5.0.1.tar.gz', '33611e51977a8289e2e124b220f727895181ef2eb9060e0987a4dc2db9cf0d6b', 620992787),
    'macos-x86_64': ('scrcpy-macos-x86_64-v5.0.1.tar.gz', '31a5467a9e907f162b093267cbbab1276b15c6e709c2ea8159566a8d165beb50', 620992832),
    'windows-x86_64': ('scrcpy-win64-v5.0.1.zip', 'b12a2c4ee8be317422451fc7dcf8ee20a71b5ea7ef9ad73ddd825a25316227a5', 620992965),
}

def download_verified_tool(url, destination, expected_sha256, *, asset_api_url=None):
    """Retry transient upstream failures without accepting partial/unverified tools."""
    destination = Path(destination)
    for attempt in range(3):
        partial = None
        try:
            request_url = asset_api_url if attempt and asset_api_url else url
            if attempt:
                parts = urllib.parse.urlsplit(request_url)
                query = urllib.parse.parse_qsl(parts.query, keep_blank_values=True)
                query.append(('stepx_retry', str(time.time_ns())))
                request_url = urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(query)))
            request = urllib.request.Request(request_url, headers={
                'User-Agent': 'StepX-release-packager', 'Accept': 'application/octet-stream'})
            started = time.monotonic()
            digest = hashlib.sha256()
            with urllib.request.urlopen(request, timeout=30) as response:
                with tempfile.NamedTemporaryFile(dir=destination.parent, suffix='.part', delete=False) as output:
                    partial = Path(output.name)
                    while True:
                        if time.monotonic() - started > 120:
                            raise TimeoutError('Tool archive download exceeded its time budget')
                        # At most one underlying read before checking the deadline again.
                        chunk = response.read1(64 * 1024)
                        if not chunk:
                            break
                        digest.update(chunk)
                        output.write(chunk)
            if digest.hexdigest() != expected_sha256:
                raise RuntimeError('Tool archive SHA256 mismatch')
            os.replace(partial, destination)
            return
        except urllib.error.HTTPError as error:
            if error.code not in {408, 429, 500, 502, 503, 504} or attempt == 2:
                raise
            print(f'Tool download returned HTTP {error.code}; retrying ({attempt + 2}/3)', flush=True)
        except (urllib.error.URLError, TimeoutError, ConnectionError, http.client.IncompleteRead):
            if attempt == 2:
                raise
            print(f'Tool download interrupted; retrying ({attempt + 2}/3)', flush=True)
        finally:
            if partial is not None:
                partial.unlink(missing_ok=True)
        time.sleep(attempt + 1)

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
            info={'CFBundleName':'StepX Companion','CFBundleDisplayName':'StepX Companion','CFBundleIdentifier':'io.github.xydzf520.stepx','CFBundleExecutable':'stepx-companion','CFBundlePackageType':'APPL','CFBundleShortVersionString':a.version,'CFBundleVersion':a.version,'NSHighResolutionCapable':True,'NSMicrophoneUsageDescription':'用于语音输入','NSCameraUsageDescription':'用于扫码连接'}
            info['NSLocalNetworkUsageDescription']='用于在局域网发现并连接您授权的手机。'
            info['NSBonjourServices']=['_adb-tls-pairing._tcp','_adb-tls-connect._tcp','_adb._tcp']
            iconset=stage/'StepX.iconset';iconset.mkdir()
            for size in [16,32,128,256,512]:
                for scale in [1,2]:
                    suffix='@2x' if scale==2 else ''
                    subprocess.run(['sips','-z',str(size*scale),str(size*scale),str(ROOT/'desktop/assets/stepx_companion_icon.png'),'--out',str(iconset/f'icon_{size}x{size}{suffix}.png')],check=True,stdout=subprocess.DEVNULL)
            subprocess.run(['iconutil','-c','icns',str(iconset),'-o',str(resource_dir/'StepX.icns')],check=True)
            info['CFBundleIconFile']='StepX.icns'
            (bundle/'Info.plist').write_bytes(plistlib.dumps(info))
        else:binary_dir=resource_dir=base
        binary_name='stepx-companion'+('.exe' if windows else '')
        shutil.copy2(ROOT/'desktop'/'target'/'release'/binary_name,binary_dir/binary_name)
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
        tools_dir=resource_dir/'tools/bin';shutil.copytree(source,tools_dir)
        subprocess.run([sys.executable,'-B',str(ROOT/'scripts/package-runtime.py'),str(resource_dir),*(['--windows'] if windows else [])],check=True,env=verification_environment())
        tool_env=verification_environment();tool_env['LD_LIBRARY_PATH']=str(tools_dir)+os.pathsep+tool_env.get('LD_LIBRARY_PATH','')
        for executable,arg in [(binary_dir/binary_name,'--version'),(tools_dir/('adb.exe' if windows else 'adb'),'version'),(tools_dir/('scrcpy.exe' if windows else 'scrcpy'),'--version')]:
            subprocess.run([str(executable),arg],check=True,env=tool_env,timeout=30)
        python=resource_dir/'runtime/python/python.exe' if windows else resource_dir/'runtime/python/bin/python3'
        verify_runtime(python, resource_dir)
        if mac:
            subprocess.run(['codesign','--force','--deep','--sign','-',str(base/'StepX Companion.app')],check=True)
            subprocess.run([str(binary_dir/binary_name),'--check-runtime'],check=True,env=verification_environment(),timeout=45)
            subprocess.run(['codesign','--verify','--deep','--strict',str(base/'StepX Companion.app')],check=True)
        issues=audit_bundle(resource_dir)
        if issues: raise RuntimeError('Package privacy verification failed:\n'+'\n'.join(issues))
        if windows: verify_manifest(binary_dir/binary_name)
        if a.platform.startswith('linux'):
            output=dist/(name+'.tar.gz')
            with tarfile.open(output,'w:gz') as t:t.add(base,arcname=name)
        else:
            output=dist/(name+'.zip')
            if mac:subprocess.run(['ditto','-c','-k','--sequesterRsrc','--keepParent',str(base),str(output)],check=True)
            else:shutil.make_archive(str(dist/name),'zip',stage,name)
        (dist/(output.name+'.sha256')).write_text(hashlib.sha256(output.read_bytes()).hexdigest()+'  '+output.name+'\n')
        print(output)
if __name__=='__main__':main()
