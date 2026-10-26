"""GitHub portable updater. Stdlib only; never runs on the UI thread.
Download/extract into private state; install using an isolated Python runtime after
old process exits. Keep a complete rollback and require a new-process receipt.
"""
import hashlib, json, os, re, shutil, stat, subprocess, sys, tarfile, time, urllib.error, urllib.parse, urllib.request, uuid, zipfile
from pathlib import Path, PurePosixPath

REPO = 'https://github.com/xydzf520/stepx'
API = 'https://api.github.com/repos/xydzf520/stepx/releases/latest'
MAX_PACKAGE = 1024 * 1024 * 1024
MAX_EXPANDED = 4 * MAX_PACKAGE

def emit(kind, **data):
    print(json.dumps(dict(kind=kind, **data), ensure_ascii=False), flush=True)

def atomic_json(path, data):
    path = Path(path); tmp = path.with_suffix('.tmp')
    tmp.write_text(json.dumps(data, ensure_ascii=False), encoding='utf-8')
    os.replace(tmp, path)

def trusted_url(url):
    p = urllib.parse.urlsplit(url)
    if p.scheme != 'https' or p.username or p.password or p.port not in (None,443) or p.fragment:
        raise ValueError('更新链接不是可信的 HTTPS 地址')
    if p.hostname not in ('github.com','api.github.com','release-assets.githubusercontent.com','objects.githubusercontent.com'):
        raise ValueError('更新链接不属于 GitHub')
    return url

class Redirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        trusted_url(newurl)
        return super().redirect_request(req,fp,code,msg,headers,newurl)

def open_url(url):
    trusted_url(url)
    return urllib.request.build_opener(Redirect()).open(urllib.request.Request(url,headers={
        'User-Agent':'StepX-Companion-Updater', 'Accept':'application/vnd.github+json' if url == API else 'application/octet-stream',
        'X-GitHub-Api-Version':'2022-11-28'}), timeout=20)

def check(platform):
    with open_url(API) as response:
        raw = response.read(2*1024*1024+1)
    if len(raw)>2*1024*1024: raise ValueError('版本信息超过大小限制')
    return parse_release(json.loads(raw),platform)

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

def download(release,job):
    job=Path(job); job.mkdir(parents=True,exist_ok=True)
    cancelled(job)
    expected=release['sha256']
    if not expected:
        with open_url(release['checksum_url']) as r: checksum=r.read(4097).decode('ascii')
        parts=checksum.strip().split()
        if len(parts)!=2 or parts[1].lstrip('*')!=release['name'] or not re.fullmatch('[a-fA-F0-9]{64}',parts[0]):raise ValueError('发布校验文件无效')
        expected=parts[0].lower()
    archive=job/'package.part'; start=time.monotonic(); last=0; received=0;h=hashlib.sha256()
    emit('progress',phase='download',received=0,total=release['size'],speed=0)
    try:
        with open_url(release['url']) as response,open(archive,'wb') as out:
            while True:
                cancelled(job); chunk=response.read(256*1024)
                if not chunk:break
                received+=len(chunk)
                if received>release['size']:raise ValueError('下载大小与发布信息不匹配')
                out.write(chunk);h.update(chunk); now=time.monotonic()
                if now-last>=0.15:
                    emit('progress',phase='download',received=received,total=release['size'],speed=int(received/max(now-start,0.1)));last=now
            out.flush();os.fsync(out.fileno())
        if received!=release['size']:raise ValueError('下载未完成，请重试')
        emit('progress',phase='verify',received=received,total=release['size'],speed=0)
        if h.hexdigest()!=expected:raise ValueError('SHA-256 校验失败，未安装此文件')
        cancelled(job)
        dest=job/'unpacked';dest.mkdir()
        emit('progress',phase='extract',received=received,total=release['size'],speed=0)
        extract(archive,dest,job)
        root=dest/release['name'].removesuffix('.tar.gz').removesuffix('.zip')
        payload,exe=verify_payload(root,release)
        # Verified bytes are retained to re-check before preparing installation.
        atomic_json(job/'unpacked.json',dict(sha256=expected,files=tree_fingerprint(dest)))
        atomic_json(job/'ready.json',dict(release=release,sha256=expected,payload=str(payload),exe=str(exe)))
        cancelled(job)
        emit('ready',job=str(job))
    except BaseException:
        archive.unlink(missing_ok=True)
        (job/'unpacked.json').unlink(missing_ok=True)
        shutil.rmtree(job/'unpacked',ignore_errors=True)
        raise

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

def prepare(job,executable,pid):
    job=Path(job); ready=json.loads((job/'ready.json').read_text(encoding='utf-8'));r=ready['release']
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
    stage=target.with_name(stem+'.update-'+uuid.uuid4().hex[:8]+suffix)
    backup=target.with_name(stem+'.backup-'+uuid.uuid4().hex[:8]+suffix)
    # File writes in the parent are necessary for atomic directory rename.
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
        if r['platform'].startswith('macos'):
            subprocess.run(['codesign','--verify','--deep','--strict',str(stage)],check=True,timeout=45,capture_output=True)
        # Copy Python out of the install tree: Windows cannot rename an in-use runtime.
        _,_,resources=package_layout(root,r['platform'])
        runtime=job/'installer-python';shutil.copytree(resources/'runtime/python',runtime)
        python=runtime/('python.exe' if r['platform'].startswith('windows') else 'bin/python3')
        config=dict(target=str(target),stage=str(stage),backup=str(backup),relative_exe=str(rel),pid=pid,version=r['version'],nonce=uuid.uuid4().hex,job=str(job))
        atomic_json(job/'install.json',config)
        helper=job/'installer.py';shutil.copy2(Path(__file__),helper)
        log=open(job/'installer.log','ab')
        kwargs={'creationflags':0x08000000|0x00000008} if os.name=='nt' else {'start_new_session':True}
        subprocess.Popen([str(python),'-B','-X','utf8',str(helper),'install',str(job/'install.json')],stdin=subprocess.DEVNULL,stdout=log,stderr=log,cwd=job,**kwargs)
        deadline=time.monotonic()+15
        while not (job/'installer-ready').exists():
            if time.monotonic()>deadline:raise TimeoutError('更新安装器未能启动，客户端保持运行')
            time.sleep(.1)
        emit('prepared',job=str(job))
    except BaseException:
        shutil.rmtree(stage,ignore_errors=True);raise

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
                    atomic_json(job/'status.json',dict(state='installed',version=config['version'],backup=str(backup)));return True
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
        if installed and target.exists():replace_directory(target,stage)
        if moved:replace_directory(backup,target)
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
