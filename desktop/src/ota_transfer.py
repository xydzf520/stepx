"""Official firmware transfer only. Never invokes update_engine or modifies partitions."""
import base64, hashlib, json, os, pathlib, re, shlex, shutil, subprocess, sys, tempfile, time, urllib.error, urllib.parse, urllib.request, zipfile

MAX_SIZE = 12 * 1024**3
CHUNK = 1024 * 1024
class Cancelled(Exception): pass

def emit(event, **fields):
    print(json.dumps(dict(event=event, **fields), ensure_ascii=False), flush=True)

def check_cancel(req):
    if req.get("parent_pid") is not None and os.getppid() != req["parent_pid"]: raise Cancelled()
    if req.get('cancel') and os.path.exists(req['cancel']): raise Cancelled()

def validate_url(url, test_origin=None):
    p = urllib.parse.urlsplit(url)
    allowed_test = test_origin and p.scheme == 'http' and p.netloc == test_origin and p.hostname in ('127.0.0.1', 'localhost', '::1')
    if (p.scheme != 'https' and not allowed_test) or not p.hostname or p.username or p.password or p.fragment or '\\' in url or any(c.isspace() or ord(c) < 32 for c in url):
        raise ValueError('固件下载地址不安全，要求 HTTPS')
    return url

class Redirects(urllib.request.HTTPRedirectHandler):
    def __init__(self, test_origin=None): self.test_origin = test_origin
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        validate_url(newurl, self.test_origin)
        return super().redirect_request(req, fp, code, msg, headers, newurl)

def contract(req):
    size = req.get('size')
    if not isinstance(size, int) or isinstance(size, bool) or not 0 < size <= MAX_SIZE:
        raise ValueError('官方固件大小缺失或超过 12 GiB 限制')
    hashes = {}
    for key, length in [('sha256', 64), ('md5', 32)]:
        val = req.get(key)
        if val:
            if not re.fullmatch('[0-9a-fA-F]{%s}' % length, val): raise ValueError('官方固件摘要格式无效')
            hashes[key] = val.lower()
    if not hashes: raise ValueError('官方未提供固件摘要，不能校验下载')
    return size, hashes

def secure_file(path):
    p = pathlib.Path(path)
    if p.is_symlink() or not p.is_file(): raise ValueError('固件缓存不是普通文件')
    return p

def metadata(path):
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)) or len(names) > 4096: raise ValueError('固件 ZIP 目录无效')
        for name in names:
            if name.startswith(('/', '\\')) or '\\' in name or '..' in pathlib.PurePosixPath(name).parts: raise ValueError('固件 ZIP 包含不安全路径')
        def read(name):
            entry = archive.getinfo(name)
            if entry.file_size > 65536: raise ValueError('固件元数据过大')
            return archive.read(name).decode('utf-8')
        raw = read('META-INF/com/android/metadata')
        info = dict(line.split('=', 1) for line in raw.splitlines() if '=' in line)
        if info.get('ota-type') != 'AB' or not info.get('pre-device') or not info.get('post-build') or not info.get('post-build-incremental') or not info.get('post-aios-version'):
            raise ValueError('缺少有效的 Android A/B 固件目标信息')
        payload = archive.getinfo('payload.bin')
        if payload.compress_type != zipfile.ZIP_STORED or payload.file_size <= 0: raise ValueError('OTA payload 格式无效')
        props = dict(line.split('=', 1) for line in read('payload_properties.txt').splitlines() if '=' in line)
        if props.get('FILE_SIZE') != str(payload.file_size): raise ValueError('OTA payload 大小不匹配')
        if len(base64.b64decode(props.get('FILE_HASH', ''), validate=True)) != 32: raise ValueError('OTA payload 摘要无效')
        if props.get('POWERWASH', '0') != '0': info['ota-wipe'] = 'yes'
        if 'META-INF/com/android/otacert' not in names: raise ValueError('OTA 缺少签名证书，不能交给手机安装')
        return info

def verify(req, path):
    size, expected = contract(req)
    path = secure_file(path)
    if path.stat().st_size != size: raise ValueError('固件大小与官方记录不匹配')
    digests = {key: hashlib.new(key) for key in set(expected) | {'sha256'}}
    done, last = 0, 0
    with path.open('rb') as stream:
        start = os.fstat(stream.fileno())
        while True:
            check_cancel(req)
            data = stream.read(CHUNK)
            if not data: break
            done += len(data)
            if done > size: raise ValueError('校验时固件大小发生变化')
            for digest in digests.values(): digest.update(data)
            if time.monotonic() - last > .2:
                emit('progress', phase='verify', done=done, total=size); last = time.monotonic()
        end = os.fstat(stream.fileno())
    if (start.st_size, start.st_mtime_ns, start.st_ino) != (end.st_size, end.st_mtime_ns, end.st_ino) or done != size:
        raise ValueError('校验时固件文件发生变化')
    actual = {key: digest.hexdigest() for key, digest in digests.items()}
    for key, value in expected.items():
        if actual[key] != value: raise ValueError('固件 %s 校验失败，请重新下载' % key.upper())
    info = metadata(path)
    if req.get('version') and info['post-aios-version'] != req['version']: raise ValueError('固件目标版本与官方候选不匹配')
    emit('progress', phase='verify', done=size, total=size)
    return dict(path=str(path), sha256=actual['sha256'], metadata=info, size=size, verified_stat=[end.st_size, end.st_mtime_ns, end.st_ino])

def download(req, test_origin=None):
    size, hashes = contract(req)
    url = validate_url(req['url'], test_origin)
    cache = pathlib.Path(req['cache'])
    cache.mkdir(parents=True, exist_ok=True, mode=0o700)
    if cache.is_symlink(): raise ValueError('固件缓存目录不能为符号链接')
    key = hashes.get('sha256') or hashes['md5']
    dest, part = cache / (key + '.zip'), cache / (key + '.part')
    if dest.exists():
        try: return verify(req, dest)
        except Cancelled: raise
        except Exception:
            # Only this content-addressed, application-owned cache is removed.
            dest.unlink()
            emit('progress', phase='download', done=0, total=size)
    if part.is_symlink(): raise ValueError('固件临时文件不能为符号链接')
    offset = part.stat().st_size if part.exists() else 0
    if offset > size: part.unlink(); offset = 0
    stream_digests = None
    if offset < size:
        check_cancel(req)
        # Reserve enough space for the complete remaining bytes; keep a small OS margin.
        if shutil.disk_usage(cache).free < size - offset + 64 * 1024**2: raise ValueError('电脑剩余空间不足以保存完整固件')
        headers = {'User-Agent': 'StepX-Desktop-Phone-OTA/1', 'Accept-Encoding': 'identity'}
        if offset: headers['Range'] = 'bytes=%d-' % offset
        opener = urllib.request.build_opener(Redirects(test_origin))
        with opener.open(urllib.request.Request(url, headers=headers), timeout=15) as response:
            if response.status == 206:
                match = re.fullmatch(r'bytes (\d+)-(\d+)/(\d+)', response.headers.get('Content-Range', ''))
                if not match or int(match[1]) != offset or int(match[3]) != size or int(match[2]) != size - 1: raise ValueError('服务器断点续传范围不匹配')
            elif response.status == 200:
                offset = 0
            else: raise ValueError('服务器没有返回可下载固件')
            length = response.headers.get('Content-Length')
            if length and int(length) != size - offset: raise ValueError('服务器固件长度与官方记录不匹配')
            done, last = offset, 0
            if offset == 0:
                stream_digests = {k: hashlib.new(k) for k in set(hashes) | {'sha256'}}
            with part.open('ab' if offset else 'wb') as stream:
                os.chmod(part, 0o600)
                while True:
                    check_cancel(req)
                    data = response.read(min(CHUNK, size - done + 1))
                    if not data: break
                    if done + len(data) > size: raise ValueError('下载超过官方固件大小')
                    stream.write(data); done += len(data)
                    if stream_digests is not None:
                        for d in stream_digests.values(): d.update(data)
                    if time.monotonic() - last > .2:
                        emit('progress', phase='download', done=done, total=size); last = time.monotonic()
                stream.flush(); os.fsync(stream.fileno())
            if done != size: raise ValueError('固件下载中断，可继续下载')
    try:
        if stream_digests is not None:
            check_cancel(req)
            part_file = secure_file(part)
            if part_file.stat().st_size != size: raise ValueError('固件大小与官方记录不匹配')
            actual = {k: d.hexdigest() for k, d in stream_digests.items()}
            for k, v in hashes.items():
                if actual[k] != v: raise ValueError('固件 %s 校验失败，请重新下载' % k.upper())
            info = metadata(part_file)
            if req.get('version') and info['post-aios-version'] != req['version']: raise ValueError('固件目标版本与官方候选不匹配')
            emit('progress', phase='verify', done=size, total=size)
            result = dict(path=str(part_file), sha256=actual['sha256'], metadata=info, size=size)
        else:
            result = verify(req, part)
    except Cancelled: raise
    except Exception:
        part.unlink(missing_ok=True)
        raise
    os.replace(part, dest)
    st = os.stat(dest)
    result['path'] = str(dest)
    result['verified_stat'] = [st.st_size, st.st_mtime_ns, st.st_ino]
    return result

def stage(req):
    size, expected = contract(req)
    path = secure_file(req['path'])
    st = path.stat()
    vstat = req.get('verified_stat')
    sha = expected.get('sha256')
    if (
        isinstance(vstat, (list, tuple))
        and len(vstat) == 3
        and [st.st_size, st.st_mtime_ns, st.st_ino] == list(vstat)
        and st.st_size == size
        and sha
    ):
        check_cancel(req)
        info = metadata(path)
        if req.get('version') and info['post-aios-version'] != req['version']:
            raise ValueError('固件目标版本与官方候选不匹配')
        emit('progress', phase='verify', done=size, total=size)
        verified = dict(
            path=str(path),
            sha256=sha,
            metadata=info,
            size=size,
            verified_stat=[st.st_size, st.st_mtime_ns, st.st_ino],
        )
    else:
        verified = verify(req, req['path'])
    adb = [req['adb'], '-s', req['transport']]
    remote = req['remote']
    if not re.fullmatch(r'/sdcard/Download/StepX-OTA/[0-9a-f]{32}/update\.zip', remote): raise ValueError('手机固件目标路径无效')
    def run(args, timeout=30, cancellable=True):
        with tempfile.TemporaryFile() as output:
            child = subprocess.Popen(adb + ['shell', ' '.join(shlex.quote(str(v)) for v in args)], stdout=output, stderr=subprocess.DEVNULL)
            began = time.monotonic()
            try:
                while child.poll() is None:
                    if cancellable: check_cancel(req)
                    if time.monotonic() - began > timeout: raise ValueError('手机命令超时，请检查 USB 连接')
                    time.sleep(.1)
                if child.returncode: raise ValueError('手机传输命令失败，请检查连接和空间')
                output.seek(0)
                out = output.read(16384).decode('utf-8', errors='replace').strip()
                if 'Permission denied' in out or 'Permission Denial' in out: raise ValueError('手机拒绝固件文件访问')
                return out
            finally:
                if child.poll() is None: child.kill(); child.wait()
    if run(['getprop', 'ro.serialno']) != req['serial']: raise ValueError('手机身份已改变，已停止传输')
    folder = remote.rsplit('/', 1)[0]
    available = run(['df', '-k', '/sdcard']).splitlines()[-1].split()
    if len(available) < 4 or not available[3].isdigit() or int(available[3])*1024 < req['size'] + 128*1024**2: raise ValueError('手机剩余空间不足，无法暂存完整固件')
    run(['mkdir', '-p', folder])
    temp = remote + '.part'
    proc = None
    try:
        check_cancel(req)
        proc = subprocess.Popen(adb + ['push', req['path'], temp], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        last = 0
        while proc.poll() is None:
            check_cancel(req)
            if time.monotonic() - last > 1:
                amount = run(['sh', '-c', 'stat -c %s ' + temp + ' 2>/dev/null || echo 0'], timeout=10)
                emit('progress', phase='transfer', done=min(int(amount), req['size']) if amount.isdigit() else 0, total=req['size']); last=time.monotonic()
            time.sleep(.15)
        if proc.returncode: raise ValueError('固件传输失败，请检查 USB 连接及手机剩余空间')
        check_cancel(req)
        emit('progress', phase='phone_verify', done=0, total=req['size'])
        digest = run(['sha256sum', temp], timeout=240).split()[0]
        if digest != verified['sha256']: raise ValueError('手机固件校验失败，未提交安装')
        check_cancel(req)
        run(['mv', temp, remote])
        emit('progress', phase='phone_verify', done=req['size'], total=req['size'])
        return dict(verified, remote=remote)
    finally:
        if proc is not None and proc.poll() is None:
            proc.kill(); proc.wait()
        try:
            run(['rm', '-f', temp], timeout=5, cancellable=False)
            run(['rmdir', folder], timeout=5, cancellable=False)
        except Exception: pass

def main():
    try:
        req = json.load(sys.stdin)
        if req['op'] == 'download': result = download(req)
        elif req['op'] == 'verify': result = verify(req, req['path'])
        elif req['op'] == 'stage': result = stage(req)
        else: raise ValueError('未知 OTA 操作')
        emit('complete', **result)
    except Cancelled: emit('cancelled')
    except urllib.error.HTTPError as error: emit('failed', message='官方下载服务返回 HTTP %s' % error.code)
    except ValueError as error: emit('failed', message=str(error)[:180])
    except Exception: emit('failed', message='固件读取或网络连接失败；下载进度已保留，可重试')
if __name__ == '__main__': main()
