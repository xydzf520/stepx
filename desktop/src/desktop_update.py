"""GitHub portable updater. Stdlib only; never runs on the UI thread.
Download/extract into private state; install using an isolated Python runtime after
old process exits. Keep a complete rollback and require a new-process receipt.
"""
import contextlib, hashlib, json, os, re, shutil, stat, subprocess, sys, tarfile, threading, time, urllib.error, urllib.parse, urllib.request, uuid, zipfile
from pathlib import Path, PurePosixPath

REPO = 'https://github.com/xydzf520/stepx'
API = 'https://api.github.com/repos/xydzf520/stepx/releases/latest'
MAX_PACKAGE = 1024 * 1024 * 1024
MAX_EXPANDED = 4 * MAX_PACKAGE
OFFICIAL_GITHUB_HOSTS = (
    'github.com',
    'api.github.com',
    'release-assets.githubusercontent.com',
    'objects.githubusercontent.com',
)
MIRROR_PREFIXES = (
    'https://ghfast.top/',
    'https://gh-proxy.com/',
)
# Community reverse proxies may ONLY carry binary release archives after SHA-256
# has been retrieved over direct TLS from the trusted GitHub origin.
PINNED_MIRROR_TARGETS = (
    REPO + '/releases/download/',
    'https://github.com/starred-ai/stepx/releases/download/',
)
STALL_WINDOW_SECS = 15.0
STALL_MIN_BYTES = 64 * 1024
HEARTBEAT_INTERVAL_SECS = 5.0
_EMIT_LOCK = threading.Lock()

def emit(kind, **data):
    line = json.dumps(dict(kind=kind, **data), ensure_ascii=False)
    with _EMIT_LOCK:
        print(line, flush=True)

@contextlib.contextmanager
def heartbeat(phase, received=0, total=0, interval=HEARTBEAT_INTERVAL_SECS):
    stop_event = threading.Event()
    def _worker():
        while not stop_event.wait(interval):
            emit('progress', phase=phase, received=received, total=total, speed=0)
    t = threading.Thread(target=_worker, name=f'stepx-updater-hb-{phase}', daemon=True)
    t.start()
    try:
        yield
    finally:
        stop_event.set()
        t.join(timeout=1.0)

def atomic_json(path, data):
    path = Path(path); tmp = path.with_suffix('.tmp')
    tmp.write_text(json.dumps(data, ensure_ascii=False), encoding='utf-8')
    os.replace(tmp, path)

def _valid_https_parts(p):
    return (
        p.scheme == 'https'
        and not p.username
        and not p.password
        and p.port in (None, 443)
        and not p.fragment
        and '\\' not in p.path
        and '..' not in PurePosixPath(p.path or '/').parts
    )

def _matches_pinned_target(inner):
    for target in PINNED_MIRROR_TARGETS:
        if target.endswith('/'):
            if inner.startswith(target):
                return True
        elif inner == target or inner.startswith(target + '?'):
            return True
    return False

def _is_checksum_or_metadata_path(p):
    path_low = (p.path or '').lower()
    return p.hostname == 'api.github.com' or path_low.endswith('.sha256') or path_low.endswith('.sha256sum')

def trusted_url(url, allow_mirrors=True):
    if not isinstance(url, str) or any(ch.isspace() for ch in url):
        raise ValueError('更新链接不是可信的 HTTPS 地址')
    p = urllib.parse.urlsplit(url)
    if not _valid_https_parts(p):
        raise ValueError('更新链接不是可信的 HTTPS 地址')
    if p.hostname in OFFICIAL_GITHUB_HOSTS:
        return url
    if not allow_mirrors:
        raise ValueError('元数据与校验摘要仅允许直连可信 HTTPS 源站')
    for prefix in MIRROR_PREFIXES:
        if url.startswith(prefix):
            inner = url[len(prefix):]
            inner_p = urllib.parse.urlsplit(inner)
            if (
                _valid_https_parts(inner_p)
                and inner_p.hostname == 'github.com'
                and not _is_checksum_or_metadata_path(inner_p)
                and _matches_pinned_target(inner)
            ):
                return url
            raise ValueError('镜像链接未指向受信任的官方二进制发布包地址')
    raise ValueError('更新链接不属于 GitHub')

def trusted_direct_url(url):
    return trusted_url(url, allow_mirrors=False)

def candidate_urls(url, allow_mirrors=False):
    trusted_url(url, allow_mirrors=allow_mirrors)
    if any(url.startswith(prefix) for prefix in MIRROR_PREFIXES):
        if not allow_mirrors:
            raise ValueError('元数据与校验摘要禁止通过第三方代理获取')
        return [url]
    if not allow_mirrors:
        return [url]
    p = urllib.parse.urlsplit(url)
    if p.hostname == 'github.com' and not _is_checksum_or_metadata_path(p):
        out = [url]
        for prefix in MIRROR_PREFIXES:
            mirrored = prefix + url
            try:
                trusted_url(mirrored, allow_mirrors=True)
                out.append(mirrored)
            except ValueError:
                pass
        return out
    return [url]

class Redirect(urllib.request.HTTPRedirectHandler):
    def __init__(self, allow_mirrors=False):
        super().__init__()
        self.allow_mirrors = allow_mirrors

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        trusted_url(newurl, allow_mirrors=self.allow_mirrors)
        return super().redirect_request(req,fp,code,msg,headers,newurl)

def open_url(url, headers=None, timeout=15):
    is_mirror = any(isinstance(url, str) and url.startswith(prefix) for prefix in MIRROR_PREFIXES)
    trusted_url(url, allow_mirrors=is_mirror)
    is_api = url == API
    req_headers = {
        'User-Agent':'StepX-Companion-Updater',
        'Accept':'application/vnd.github+json' if is_api else 'application/octet-stream',
        'X-GitHub-Api-Version':'2022-11-28',
    }
    if headers:
        req_headers.update(headers)
    return urllib.request.build_opener(Redirect(allow_mirrors=is_mirror)).open(
        urllib.request.Request(url, headers=req_headers),
        timeout=timeout,
    )

def check(platform):
    last_err = None
    for cand in candidate_urls(API, allow_mirrors=False):
        try:
            trusted_direct_url(cand)
            with open_url(cand) as response:
                raw = response.read(2*1024*1024+1)
            if len(raw)>2*1024*1024:
                raise ValueError('版本信息超过大小限制')
            return parse_release(json.loads(raw),platform)
        except Exception as err:
            last_err = err
    raise last_err

def parse_release(d, platform):
    tag=d.get('tag_name',''); version=tag.removeprefix('v')
    if d.get('draft') or d.get('prerelease'): raise ValueError('未发布正式版本')
    if not re.fullmatch(r'\d+\.\d+\.\d+',version): raise ValueError('正式版本号无效')
    if d.get('html_url') != REPO+'/releases/tag/'+tag: raise ValueError('版本来源不匹配')
    if platform not in ('linux-x86_64','macos-arm64','macos-x86_64','windows-x86_64'):
        raise ValueError('暂未提供适用于当前系统的安装包')
    name=f'StepX-Companion-{version}-{platform}'+('.tar.gz' if platform.startswith('linux') else '.zip')
    assets=[a for a in d.get('assets',[]) if a.get('name')==name and a.get('state','uploaded')=='uploaded']
    if len(assets)!=1: raise ValueError('此版本尚未提供当前系统的完整安装包')
    a=assets[0]; url=REPO+'/releases/download/'+tag+'/'+name
    if a.get('browser_download_url')!=url: raise ValueError('安装包地址不匹配')
    size=a.get('size',0)
    if not 0<size<=MAX_PACKAGE: raise ValueError('安装包大小无效')
    digest=a.get('digest') or ''
    checksum_url=None
    if not re.fullmatch(r'sha256:[a-fA-F0-9]{64}',digest):
        checksum=[c for c in d.get('assets',[]) if c.get('name')==name+'.sha256' and c.get('browser_download_url')==url+'.sha256']
        if len(checksum)!=1: raise ValueError('发布包缺少 SHA-256 校验信息')
        checksum_url=url+'.sha256'; digest=''
    return dict(version=version,tag=tag,url=url,name=name,size=size,sha256=digest.removeprefix('sha256:').lower(),checksum_url=checksum_url,
                notes=str(d.get('body') or '此版本未提供更新说明。')[:16000],page=d['html_url'],platform=platform)

def cancelled(job):
    if (job/'cancel').exists(): raise InterruptedError('下载已取消')

def digest_file(path):
    h=hashlib.sha256()
    with open(path,'rb') as f:
        for b in iter(lambda:f.read(1024*1024),b''): h.update(b)
    return h.hexdigest()

def member_path(root,name):
    if '\\' in name or ':' in name or '\x00' in name: raise ValueError('更新包含无效文件路径')
    p=PurePosixPath(name)
    if p.is_absolute() or '..' in p.parts: raise ValueError('更新包含越界文件路径')
    dest=root.joinpath(*p.parts)
    if not dest.resolve().is_relative_to(root.resolve()): raise ValueError('更新包含越界文件路径')
    return dest

def extract(archive, dest, job):
    total=0; count=0; links=[]
    def reserve(name,size):
        nonlocal total,count
        total+=size;count+=1
        if total>MAX_EXPANDED or count>60000: raise ValueError('解压超过安全限制')
        cancelled(job)
        return member_path(dest,name)
    # Do not use extractall: prohibit links/devices and constrain every path.
    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as z:
            for m in z.infolist():
                out=reserve(m.filename,m.file_size); mode=m.external_attr>>16
                if stat.S_ISLNK(mode):
                    if m.file_size>4096:raise ValueError('链接路径过长')
                    links.append((out,z.read(m).decode('utf-8'),False));continue
                if m.is_dir(): out.mkdir(parents=True,exist_ok=True); continue
                out.parent.mkdir(parents=True,exist_ok=True)
                with z.open(m) as src,open(out,'wb') as dst: shutil.copyfileobj(src,dst)
                out.chmod(0o755 if mode&0o111 else 0o644)
    else:
        with tarfile.open(archive,'r:gz') as t:
            for m in t:
                out=reserve(m.name,m.size)
                if m.isdir(): out.mkdir(parents=True,exist_ok=True);continue
                if m.issym() or m.islnk():links.append((out,m.linkname,m.islnk()));continue
                if not m.isfile(): raise ValueError('更新包含不支持的特殊文件')
                out.parent.mkdir(parents=True,exist_ok=True)
                with t.extractfile(m) as src,open(out,'wb') as dst: shutil.copyfileobj(src,dst)
                out.chmod(0o755 if m.mode&0o111 else 0o644)

    # Runtime archives include relative Python/library links. Validate and create last,
    # so no subsequent archive member can write through a symlink.
    for out,name,hard in links:
        if not name or '\\' in name or ':' in name or PurePosixPath(name).is_absolute():raise ValueError('更新链接越界')
        link=(dest/name) if hard else (out.parent/name)
        if not link.resolve().is_relative_to(dest.resolve()):raise ValueError('更新链接越界')
        if out.exists() or out.is_symlink():raise ValueError('更新包含重复路径')
        out.parent.mkdir(parents=True,exist_ok=True)
        if hard:os.link(link,out)
        else:out.symlink_to(name)

def package_layout(root, platform):
    if platform.startswith('macos'):
        bundle=root/'StepX Companion.app'
        return bundle, bundle/'Contents/MacOS/stepx-companion',bundle/'Contents/Resources'
    return root,root/('stepx-companion.exe' if platform.startswith('windows') else 'stepx-companion'),root

def verify_payload(root,release):
    payload,exe,res=package_layout(root,release['platform'])
    for p in [exe,res/'scripts',res/'desktop/assets/stepx-companion.apk',res/'runtime/python',res/'tools/bin']:
        if not p.exists(): raise ValueError('安装包不完整：'+p.name)
    if release['platform'].startswith('macos'):
        import plistlib
        info=plistlib.loads((payload/'Contents/Info.plist').read_bytes())
        if info.get('CFBundleShortVersionString')!=release['version']:raise ValueError('安装包版本不匹配')
    return payload,exe

def tree_fingerprint(dest):
    entries=[]
    for cur,dirs,files in os.walk(dest,followlinks=False):
        dirs.sort();files.sort()
        base=Path(cur)
        for name in dirs+files:
            p=base/name;rel=p.relative_to(dest).as_posix();st=p.lstat()
            if stat.S_ISLNK(st.st_mode):entries.append([rel,'L',os.readlink(p)])
            elif stat.S_ISDIR(st.st_mode):entries.append([rel,'D',st.st_mode&0o777])
            else:entries.append([rel,'F',st.st_size,st.st_mtime_ns,st.st_ino])
    return entries

def staging_root(job):
    job = Path(job)
    return job.parent if job.name.startswith('job-') else job

def part_cache_path(release, job):
    version = str(release['version'])
    asset = str(release['name'])
    if not re.fullmatch(r'[A-Za-z0-9._-]+', version) or not re.fullmatch(r'[A-Za-z0-9._-]+', asset):
        raise ValueError('更新缓存文件名无效')
    cache_dir = staging_root(job) / 'cache'
    cache_dir.mkdir(parents=True, exist_ok=True)
    return cache_dir / f'{version}-{asset}.part'

def _allow_unsigned_local_fixture(release):
    return (
        isinstance(release, dict)
        and release.get('allow_unsigned_fixture') is True
        and os.environ.get('STEPX_ALLOW_UNSIGNED_TEST_FIXTURE') == '1'
    )

def resolve_expected_sha256(release):
    expected = (release.get('sha256') or '').strip().lower()
    if expected:
        if not re.fullmatch(r'[a-f0-9]{64}', expected):
            raise ValueError('发布包 SHA-256 格式无效')
        return expected
    checksum_url = release.get('checksum_url')
    if not checksum_url:
        if _allow_unsigned_local_fixture(release):
            return ''
        raise ValueError('发布包缺少 SHA-256 校验信息')
    last_err = None
    for cand in candidate_urls(checksum_url, allow_mirrors=False):
        try:
            trusted_direct_url(cand)
            with open_url(cand) as r:
                checksum = r.read(4097).decode('ascii')
            parts = checksum.strip().split()
            if (
                len(parts) != 2
                or parts[1].lstrip('*') != release['name']
                or not re.fullmatch(r'[a-fA-F0-9]{64}', parts[0])
            ):
                raise ValueError('发布校验文件无效')
            return parts[0].lower()
        except Exception as err:
            last_err = err
    raise last_err

def _open_candidate_url(cand, headers=None, allow_mirrors=False):
    trusted_url(cand, allow_mirrors=allow_mirrors)
    if headers is not None:
        return open_url(cand, headers=headers)
    return open_url(cand)

def _content_range_matches(response, offset):
    headers = getattr(response, 'headers', None)
    if not headers or not hasattr(headers, 'get'):
        return True
    cr = headers.get('Content-Range') or headers.get('content-range')
    if not cr:
        return True
    return str(cr).strip().startswith(f'bytes {offset}-')

def _stream_candidate(cand, cache_part, total_size, job, overall_start, allow_mirrors=False):
    offset = cache_part.stat().st_size if cache_part.is_file() else 0
    if offset > total_size:
        cache_part.unlink(missing_ok=True)
        offset = 0
    req_headers = {'Range': f'bytes={offset}-'} if offset > 0 else None
    try:
        ctx = _open_candidate_url(cand, headers=req_headers, allow_mirrors=allow_mirrors)
    except urllib.error.HTTPError as err:
        if err.code == 416:
            cache_part.unlink(missing_ok=True)
            if offset > 0:
                ctx = _open_candidate_url(cand, allow_mirrors=allow_mirrors)
                offset = 0
            else:
                raise
        else:
            raise
    with ctx as response:
        status = getattr(response, 'status', None)
        if status is None and hasattr(response, 'getcode'):
            status = response.getcode()
        status = status or 200
        if status == 416:
            cache_part.unlink(missing_ok=True)
            raise ValueError('断点范围无效，已重置缓存请重试')
        resuming = offset > 0 and status == 206 and _content_range_matches(response, offset)
        target_file = cache_part if (resuming or offset == 0) else cache_part.with_suffix('.part.fresh')
        mode = 'ab' if resuming else 'wb'
        received = offset if resuming else 0
        downloaded_this_attempt = 0
        last_emit = 0.0
        window_start = time.monotonic()
        window_bytes = 0
        try:
            with open(target_file, mode) as out:
                while True:
                    cancelled(job)
                    chunk = response.read(64 * 1024)
                    now = time.monotonic()
                    if not chunk:
                        break
                    received += len(chunk)
                    downloaded_this_attempt += len(chunk)
                    window_bytes += len(chunk)
                    if received > total_size:
                        out.close()
                        target_file.unlink(missing_ok=True)
                        if target_file == cache_part:
                            cache_part.unlink(missing_ok=True)
                        raise ValueError('下载大小与发布信息不匹配')
                    out.write(chunk)
                    if now - last_emit >= 0.15:
                        elapsed = max(now - overall_start, 0.1)
                        emit(
                            'progress',
                            phase='download',
                            received=received,
                            total=total_size,
                            speed=int(downloaded_this_attempt / elapsed),
                        )
                        last_emit = now
                    if now - window_start >= STALL_WINDOW_SECS:
                        if window_bytes < STALL_MIN_BYTES:
                            out.flush()
                            os.fsync(out.fileno())
                            raise TimeoutError('下载连接停滞（低于 64 KB / 15s），已保留断点进度')
                        window_start = now
                        window_bytes = 0
                out.flush()
                os.fsync(out.fileno())
        finally:
            if target_file != cache_part and target_file.is_file():
                if target_file.stat().st_size >= offset:
                    os.replace(target_file, cache_part)
                else:
                    target_file.unlink(missing_ok=True)
    final_size = cache_part.stat().st_size if cache_part.is_file() else 0
    if final_size != total_size:
        raise ValueError('下载未完成，请重试')

def download(release,job):
    job=Path(job); job.mkdir(parents=True,exist_ok=True)
    cancelled(job)
    expected=resolve_expected_sha256(release)
    allow_unsigned=_allow_unsigned_local_fixture(release)
    if not expected and not allow_unsigned:
        raise ValueError('发布包缺少可信 SHA-256 摘要，拒绝下载')
    total_size=int(release['size'])
    if not 0 < total_size <= MAX_PACKAGE:
        raise ValueError('安装包大小无效')
    cache_part=part_cache_path(release,job)
    archive=job/'package.part'
    start=time.monotonic()
    initial_offset=cache_part.stat().st_size if cache_part.is_file() else 0
    if initial_offset > total_size:
        cache_part.unlink(missing_ok=True)
        initial_offset = 0
    emit('progress',phase='download',received=initial_offset,total=total_size,speed=0)
    try:
        verified = False
        if initial_offset == total_size and expected:
            emit('progress',phase='verify',received=total_size,total=total_size,speed=0)
            with heartbeat('verify', received=total_size, total=total_size):
                actual_initial = digest_file(cache_part)
            if actual_initial == expected:
                verified = True
            else:
                cache_part.unlink(missing_ok=True)
        if not verified:
            last_err = None
            sha_err = None
            stall_err = None
            trunc_err = None
            allow_mirrors = bool(expected)
            for cand in candidate_urls(release['url'], allow_mirrors=allow_mirrors):
                cancelled(job)
                cur_offset = cache_part.stat().st_size if cache_part.is_file() else 0
                emit('progress',phase='download',received=cur_offset,total=total_size,speed=0)
                try:
                    _stream_candidate(cand, cache_part, total_size, job, start, allow_mirrors=allow_mirrors)
                    emit('progress',phase='verify',received=total_size,total=total_size,speed=0)
                    with heartbeat('verify', received=total_size, total=total_size):
                        actual = digest_file(cache_part)
                    if expected and actual != expected:
                        cache_part.unlink(missing_ok=True)
                        sha_err = ValueError('SHA-256 校验失败，未安装此文件')
                        last_err = sha_err
                        continue
                    if not expected:
                        expected = actual
                    verified = True
                    break
                except InterruptedError:
                    raise
                except TimeoutError as err:
                    stall_err = err
                    last_err = err
                except Exception as err:
                    if isinstance(err, ValueError) and '下载未完成' in str(err):
                        trunc_err = err
                    last_err = err
            if not verified:
                raise (sha_err or stall_err or trunc_err or last_err or ValueError('下载未完成，请重试'))
        cancelled(job)
        shutil.copy2(cache_part, archive)
        dest=job/'unpacked';dest.mkdir()
        emit('progress',phase='extract',received=total_size,total=total_size,speed=0)
        with heartbeat('extract', received=total_size, total=total_size):
            extract(archive,dest,job)
            root=dest/release['name'].removesuffix('.tar.gz').removesuffix('.zip')
            payload,exe=verify_payload(root,release)
            fp=tree_fingerprint(dest)
        # Verified bytes are retained to re-check before preparing installation.
        atomic_json(job/'unpacked.json',dict(sha256=expected,files=fp))
        atomic_json(job/'ready.json',dict(release=release,sha256=expected,payload=str(payload),exe=str(exe)))
        cancelled(job)
        emit('ready',job=str(job))
    except BaseException:
        archive.unlink(missing_ok=True)
        (job/'unpacked.json').unlink(missing_ok=True)
        shutil.rmtree(job/'unpacked',ignore_errors=True)
        raise

def verify_os_signature(stage, rel_exe, platform):
    allow_adhoc = os.environ.get('STEPX_ALLOW_ADHOC_SIGNATURE') == '1'
    if platform.startswith('macos'):
        subprocess.run(['codesign', '--verify', '--deep', '--strict', str(stage)], check=True, timeout=60, capture_output=True, text=True)
        dv = subprocess.run(['codesign', '-dv', '--verbose=4', str(stage)], check=True, timeout=30, capture_output=True, text=True)
        details = (dv.stdout or '') + '\n' + (dv.stderr or '')
        if not allow_adhoc:
            if 'TeamIdentifier=not set' in details or 'Signature=adhoc' in details:
                raise ValueError('macOS 安装包缺少有效的 Developer ID 签名（拒绝 ad-hoc 签名）')
            sp = subprocess.run(['spctl', '--assess', '--type', 'execute', '--verbose=2', str(stage)], check=False, timeout=60, capture_output=True, text=True)
            if sp.returncode != 0:
                raise ValueError('macOS Gatekeeper (spctl) 签名验证未通过')
    elif platform.startswith('windows'):
        if sys.platform != 'win32' and os.environ.get('STEPX_FORCE_AUTHENTICODE_CHECK') != '1':
            return
        ps = shutil.which('powershell.exe') or shutil.which('pwsh.exe') or shutil.which('powershell')
        if not ps:
            if os.environ.get('STEPX_REQUIRE_AUTHENTICODE') == '1':
                raise ValueError('无法调用 PowerShell 校验 Windows 安装包签名')
            return
        exe_path = Path(stage) / rel_exe
        ps_cmd = (
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; "
            "$s = Get-AuthenticodeSignature -LiteralPath $env:STEPX_VERIFY_EXE; "
            "@{ Status = [string]$s.Status; StatusMessage = [string]$s.StatusMessage; "
            "HasCert = ($null -ne $s.SignerCertificate) } | ConvertTo-Json -Compress"
        )
        env = os.environ.copy()
        env['STEPX_VERIFY_EXE'] = str(exe_path)
        kwargs = {'creationflags': 0x08000000} if os.name == 'nt' else {}
        proc = subprocess.run([ps, '-NoProfile', '-NonInteractive', '-Command', ps_cmd], check=True, timeout=60, capture_output=True, text=True, env=env, **kwargs)
        info = json.loads((proc.stdout or '').strip() or '{}')
        status = str(info.get('Status') or '')
        has_cert = bool(info.get('HasCert'))
        if os.environ.get('STEPX_REQUIRE_AUTHENTICODE') == '1' and status == 'NotSigned':
            raise ValueError('Windows 安装包缺少 Authenticode 数字签名')
        if has_cert or (status and status != 'NotSigned'):
            if status != 'Valid':
                msg = str(info.get('StatusMessage') or status)
                raise ValueError(f'Windows Authenticode 签名校验未通过：{msg}')

def installation_target(executable,platform):
    exe=Path(executable).resolve()
    if platform.startswith('macos'):
        if exe.parent.name!='MacOS' or exe.parent.parent.name!='Contents':raise ValueError('请先将应用移到“应用程序”目录再更新')
        target=exe.parent.parent.parent
        if not target.name.endswith('.app'):raise ValueError('无法识别应用安装位置')
    else:
        target=exe.parent
        if not (target/'desktop/assets/stepx-companion.apk').is_file() or not (target/'scripts').is_dir():
            raise ValueError('当前是开发构建，请使用完整安装包进行更新')
    if target==Path.home() or len(target.parts)<3 or '/AppTranslocation/' in str(target) or str(target).startswith('/Volumes/'):
        raise ValueError('请先将应用安装到本机可写目录后再更新')
    return target

def _prune_old_dirs(entries, keep_paths, keep_latest=1):
    keep_resolved = set()
    for kp in keep_paths:
        if kp is None:
            continue
        keep_resolved.add(Path(kp))
        try:
            keep_resolved.add(Path(kp).resolve())
        except OSError:
            pass
    candidates = []
    for p in entries:
        try:
            if p in keep_resolved or p.resolve() in keep_resolved:
                continue
            if p.is_dir() and not p.is_symlink():
                candidates.append((p.stat().st_mtime_ns, p))
        except OSError:
            continue
    candidates.sort(key=lambda item: item[0], reverse=True)
    for _, old_dir in candidates[max(0, keep_latest):]:
        shutil.rmtree(old_dir, ignore_errors=True)

def prune_old_backups_and_jobs(target, latest_backup, current_job):
    target = Path(target)
    latest_backup = Path(latest_backup) if latest_backup else None
    current_job = Path(current_job) if current_job else None
    parent = target.parent
    if parent.is_dir():
        mac = target.name.endswith('.app')
        stem = target.stem if mac else target.name
        prefix = f'{stem}.backup-'
        backups = [
            p for p in parent.iterdir()
            if p.name.startswith(prefix) and (not mac or p.name.endswith('.app'))
        ]
        # latest_backup is already kept via keep_paths (count=1), so keep_latest=0 removes all older backups.
        _prune_old_dirs(backups, [latest_backup, target], keep_latest=0 if latest_backup else 1)
        stale_stages = [p for p in parent.iterdir() if p.name.startswith('.stepx-stage-')]
        _prune_old_dirs(stale_stages, [target, latest_backup], keep_latest=0)
    if current_job is not None:
        root = staging_root(current_job)
        if root.is_dir() and current_job.name.startswith('job-'):
            keep_jobs = [current_job]
            for pointer_name in ('last-ready.json', 'last-job.json'):
                ptr = root / pointer_name
                if ptr.is_file():
                    try:
                        ref = json.loads(ptr.read_text(encoding='utf-8')).get('job')
                        if ref:
                            keep_jobs.append(Path(ref))
                    except Exception:
                        pass
            jobs = [p for p in root.iterdir() if p.name.startswith('job-')]
            _prune_old_dirs(jobs, keep_jobs, keep_latest=0)

def prepare(job,executable,pid):
    job=Path(job); ready=json.loads((job/'ready.json').read_text(encoding='utf-8'));r=ready['release']
    total_size = int(r.get('size') or 0)
    emit('progress', phase='prepare', received=0, total=total_size, speed=0)
    with heartbeat('prepare', received=0, total=total_size):
        if digest_file(job/'package.part')!=ready['sha256']:raise ValueError('暂存包已变化，请重新下载')
        dest=job/'unpacked'
        root=dest/r['name'].removesuffix('.tar.gz').removesuffix('.zip')
        payload=newexe=None
        marker=job/'unpacked.json'
        if dest.is_dir() and marker.is_file():
            try:
                info=json.loads(marker.read_text(encoding='utf-8'))
                if info.get('sha256')==ready['sha256'] and info.get('files')==tree_fingerprint(dest):
                    payload,newexe=verify_payload(root,r)
            except Exception:
                payload=newexe=None
        if payload is None or newexe is None:
            # Discard extracted files and re-extract from verified archive (no staging TOCTOU).
            shutil.rmtree(dest,ignore_errors=True); dest.mkdir()
            extract(job/'package.part',dest,job)
            payload,newexe=verify_payload(root,r)
            atomic_json(marker,dict(sha256=ready['sha256'],files=tree_fingerprint(dest)))
        target=installation_target(executable,r['platform'])
        mac=r['platform'].startswith('macos')
        stem=target.stem if mac else target.name; suffix='.app' if mac else ''
        ts=f'{int(time.time()*1000)}-{uuid.uuid4().hex[:8]}'
        stage=target.with_name(f'.stepx-stage-{ts}{suffix}')
        backup=target.with_name(f'{stem}.backup-{ts}{suffix}')
        # Stage in target.parent on the same filesystem so os.replace is atomic.
        stage.mkdir()
        try:
            # Preserve unrecognised files in portable installs, but replace all shipped resources.
            shutil.copytree(target,stage,dirs_exist_ok=True,symlinks=True)
            for p in payload.iterdir():
                out=stage/p.name
                if out.is_symlink() or out.is_file():out.unlink()
                elif out.exists():shutil.rmtree(out)
                if p.is_dir():shutil.copytree(p,out,symlinks=True)
                else:shutil.copy2(p,out)
            rel=newexe.relative_to(payload)
            verify_os_signature(stage, rel, r['platform'])
            # Copy Python out of the install tree: Windows cannot rename an in-use runtime.
            _,_,resources=package_layout(root,r['platform'])
            runtime=job/'installer-python'
            shutil.rmtree(runtime, ignore_errors=True)
            shutil.copytree(resources/'runtime/python',runtime,symlinks=True)
            python=runtime/('python.exe' if r['platform'].startswith('windows') else 'bin/python3')
            config=dict(target=str(target),stage=str(stage),backup=str(backup),relative_exe=str(rel),pid=pid,version=r['version'],nonce=uuid.uuid4().hex,job=str(job))
            atomic_json(job/'install.json',config)
            helper=job/'installer.py';shutil.copy2(Path(__file__),helper)
            kwargs={'creationflags':0x08000000|0x00000200} if os.name=='nt' else {'start_new_session':True}
            with open(job/'installer.log','ab') as log:
                subprocess.Popen([str(python),'-B','-X','utf8',str(helper),'install',str(job/'install.json')],stdin=subprocess.DEVNULL,stdout=log,stderr=log,cwd=job,**kwargs)
            deadline=time.monotonic()+15
            while not (job/'installer-ready').exists():
                if time.monotonic()>deadline:raise TimeoutError('更新安装器未能启动，客户端保持运行')
                time.sleep(.1)
        except BaseException:
            shutil.rmtree(stage,ignore_errors=True);raise
    emit('prepared',job=str(job))

def pid_alive(pid):
    if os.name=='nt':
        import ctypes
        k=ctypes.WinDLL('kernel32',use_last_error=True)
        k.OpenProcess.restype=ctypes.c_void_p;k.OpenProcess.argtypes=[ctypes.c_uint32,ctypes.c_int,ctypes.c_uint32]
        k.CloseHandle.argtypes=[ctypes.c_void_p]
        k.WaitForSingleObject.argtypes=[ctypes.c_void_p,ctypes.c_uint32]
        h=k.OpenProcess(0x100000,False,pid)
        if not h:return False
        alive=k.WaitForSingleObject(h,0)==258;k.CloseHandle(h);return alive
    try:os.kill(pid,0);return True
    except ProcessLookupError:return False
    except PermissionError:return True

def launch(exe,job,nonce):
    env=os.environ.copy()
    for key in ('STEPX_RESOURCE_DIR','STEPX_PYTHON'):env.pop(key,None)
    kwargs={'creationflags':0x08000000} if os.name=='nt' else {'start_new_session':True}
    return subprocess.Popen([str(exe),'--update-receipt',str(job/'install.json'),'--update-nonce',nonce],cwd=exe.parent,env=env,
                            stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,**kwargs)

def replace_directory(source,destination):
    # Windows may release executable handles shortly after the process exits.
    for attempt in range(25):
        try:os.replace(source,destination);return
        except PermissionError:
            if attempt==24:raise
            time.sleep(.2)

def install(config,wait_seconds=40,ack_seconds=45):
    target=Path(config['target']);stage=Path(config['stage']);backup=Path(config['backup']);job=Path(config['job'])
    atomic_json(job/'status.json',dict(state='waiting',version=config['version']))
    (job/'installer-ready').touch()
    deadline=time.monotonic()+wait_seconds
    while pid_alive(config['pid']):
        if time.monotonic()>deadline:
            atomic_json(job/'status.json',dict(state='failed',message='客户端尚未退出，未替换任何文件'));return False
        time.sleep(.2)
    moved=False;installed=False;child=None
    try:
        atomic_json(job/'status.json',dict(state='installing',version=config['version']))
        # One directory rename per side; old files remain intact in backup.
        replace_directory(target,backup);moved=True
        replace_directory(stage,target);installed=True
        exe=target/config['relative_exe'];child=launch(exe,job,config['nonce'])
        deadline=time.monotonic()+ack_seconds
        while time.monotonic()<deadline:
            if child.poll() is not None:raise RuntimeError('新版本启动后提前退出')
            if (job/'ack.json').exists():
                ack=json.loads((job/'ack.json').read_text(encoding='utf-8'))
                if ack.get('nonce')==config['nonce'] and ack.get('version')==config['version']:
                    atomic_json(job/'status.json',dict(state='installed',version=config['version'],backup=str(backup)))
                    prune_old_backups_and_jobs(target, backup, job)
                    return True
            time.sleep(.2)
        raise RuntimeError('未收到新版启动确认')
    except BaseException as error:
        if child and child.poll() is None:
            if os.name=='nt':
                subprocess.run(['taskkill','/PID',str(child.pid),'/T','/F'],capture_output=True,timeout=15,creationflags=0x08000000)
            else:
                import signal
                try:os.killpg(child.pid,signal.SIGTERM)
                except ProcessLookupError:pass
            try:child.wait(timeout=10)
            except subprocess.TimeoutExpired:child.kill();child.wait(timeout=10)
        if installed and target.exists():
            try:
                replace_directory(target,stage)
            except OSError:
                shutil.rmtree(target, ignore_errors=True)
        if moved:
            replace_directory(backup,target)
        shutil.rmtree(stage, ignore_errors=True)
        atomic_json(job/'status.json',dict(state='rolled_back' if moved else 'failed',message=('更新失败，已恢复原版本：' if moved else '安装目录被占用或不可写，已保留原版本：')+str(error)))
        if (target/config['relative_exe']).is_file():launch(target/config['relative_exe'],job,config['nonce'])
        return False

def main():
    op=sys.argv[1]
    try:
        if op=='install': install(json.loads(Path(sys.argv[2]).read_text(encoding='utf-8')));return
        data=json.load(sys.stdin)
        if op=='check':emit('release',release=check(data['platform']))
        elif op=='download':download(data['release'],data['job'])
        elif op=='prepare':prepare(data['job'],data['executable'],data['pid'])
        else:raise ValueError('未知更新操作')
    except InterruptedError:emit('cancelled')
    except urllib.error.HTTPError as e:
        emit('error',message={403:'GitHub 请求受限，请稍后重试',404:'尚未发布可下载的正式版本',429:'GitHub 请求过于频繁，请稍后重试'}.get(e.code,'GitHub 下载失败，请稍后重试'))
    except Exception as e:emit('error',message=str(e)[:400])
if __name__=='__main__':main()
