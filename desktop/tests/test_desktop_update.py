import hashlib, importlib.util, io, json, os, stat, subprocess, tarfile, tempfile, unittest, zipfile
from pathlib import Path
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('updater',Path(__file__).parents[1]/'src/desktop_update.py')
u=importlib.util.module_from_spec(spec);spec.loader.exec_module(u)
class UpdateTests(unittest.TestCase):
    def release(self,platform='linux-x86_64'):
        v='0.2.19';n=f'StepX-Companion-{v}-{platform}'+('.tar.gz' if platform.startswith('linux') else '.zip')
        return {'tag_name':'v'+v,'html_url':u.REPO+'/releases/tag/v'+v,'draft':False,'prerelease':False,'body':'notes',
                'assets':[{'name':n,'size':100,'digest':'sha256:'+'a'*64,'browser_download_url':u.REPO+'/releases/download/v'+v+'/'+n}]}
    def test_platforms_and_digest(self):
        for p in ['linux-x86_64','macos-arm64','macos-x86_64','windows-x86_64']:
            r=u.parse_release(self.release(p),p);self.assertEqual(r['platform'],p);self.assertEqual(r['sha256'],'a'*64)
        with self.assertRaises(ValueError):u.parse_release(self.release(),'linux-arm64')
    def test_missing_and_wrong_release_assets(self):
        for mutate in [lambda d:d.update(draft=True),lambda d:d.update(prerelease=True),lambda d:d.update(tag_name='../x'),
                       lambda d:d['assets'][0].update(digest=None),lambda d:d['assets'][0].update(size=u.MAX_PACKAGE+1),
                       lambda d:d['assets'][0].update(browser_download_url='https://evil.test/app.zip'),lambda d:d.update(assets=[])]:
            d=self.release();mutate(d)
            with self.assertRaises(ValueError):u.parse_release(d,'linux-x86_64')
    def test_urls(self):
        for url in [
            'http://github.com/x',
            'https://evil.test/x',
            'https://github.com@evil.test/x',
            'https://github.com:90/x',
            'https://github.com/x#bad',
            'https://ghfast.top/https://evil.test/x',
            'https://gh-proxy.com/https://github.com/other/repo/releases/download/v1.0.0/a.zip',
            'https://ghfast.top/https://github.com/xydzf520/stepx/../../evil/repo/a.zip',
            'https://ghfast.top/http://github.com/xydzf520/stepx/releases/download/v0.2.19/a.zip',
        ]:
            with self.assertRaises(ValueError, msg=url):u.trusted_url(url)
        u.trusted_url('https://release-assets.githubusercontent.com/x?sig=example')
        u.trusted_url('https://ghfast.top/' + u.REPO + '/releases/download/v0.2.19/StepX-Companion-0.2.19-linux-x86_64.tar.gz')
        u.trusted_url('https://gh-proxy.com/' + u.REPO + '/releases/download/v0.2.19/StepX-Companion-0.2.19-linux-x86_64.tar.gz')
        u.trusted_url('https://ghfast.top/https://github.com/starred-ai/stepx/releases/download/v0.2.19/StepX-Companion-0.2.19-linux-x86_64.tar.gz')
        u.trusted_url('https://gh-proxy.com/https://github.com/starred-ai/stepx/releases/download/v0.2.19/StepX-Companion-0.2.19-linux-x86_64.tar.gz')
    def archive(self,path,name,link=None):
        with tarfile.open(path,'w:gz') as t:
            m=tarfile.TarInfo(name)
            if link:m.type=tarfile.SYMTYPE;m.linkname=link;t.addfile(m)
            else:m.size=4;t.addfile(m,io.BytesIO(b'data'))
    def test_archive_escape_devices_links(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);out=root/'out';out.mkdir()
            for name,link in [('../escape',None),('/escape',None),('C:\\escape',None),('link','../../escape')]:
                self.archive(root/'x.tar.gz',name,link)
                with self.assertRaises(ValueError):u.extract(root/'x.tar.gz',out,root)
            self.assertFalse((root/'escape').exists())
    def test_relative_library_links_and_modes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);out=root/'out';out.mkdir()
            with tarfile.open(root/'x.tar.gz','w:gz') as t:
                m=tarfile.TarInfo('bin/python3.13');m.size=4;m.mode=0o755;t.addfile(m,io.BytesIO(b'data'))
                m=tarfile.TarInfo('bin/python3');m.type=tarfile.SYMTYPE;m.linkname='python3.13';t.addfile(m)
            u.extract(root/'x.tar.gz',out,root)
            self.assertEqual((out/'bin/python3').read_bytes(),b'data')
            if os.name != 'nt':
                self.assertTrue((out/'bin/python3').stat().st_mode&stat.S_IXUSR)
    def test_windows_zip_and_macos_layout(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);out=root/'out';out.mkdir()
            with zipfile.ZipFile(root/'x.zip','w') as z:z.writestr('app/stepx-companion.exe',b'data')
            u.extract(root/'x.zip',out,root);self.assertEqual((out/'app/stepx-companion.exe').read_bytes(),b'data')
            p,e,r=u.package_layout(root,'macos-arm64');self.assertEqual(e,root/'StepX Companion.app/Contents/MacOS/stepx-companion')
    def test_cancel_and_hash_failure_do_not_leave_ready(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);r=u.parse_release(self.release(),'linux-x86_64');r['size']=4
            with patch.object(u,'open_url',return_value=io.BytesIO(b'data')):
                with self.assertRaisesRegex(ValueError,'SHA-256'):u.download(r,root)
            self.assertFalse((root/'ready.json').exists());self.assertFalse((root/'package.part').exists())
            (root/'cancel').touch()
            with self.assertRaises(InterruptedError):u.download(r,root)
    def test_truncated_download(self):
        with tempfile.TemporaryDirectory() as tmp,patch.object(u,'open_url',return_value=io.BytesIO(b'data')):
            with self.assertRaisesRegex(ValueError,'下载未完成'):u.download(u.parse_release(self.release(),'linux-x86_64'),Path(tmp))
    def test_install_wait_does_not_touch_running_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);a=root/'app';a.mkdir();(a/'marker').write_text('old')
            c=dict(target=str(a),stage=str(root/'stage'),backup=str(root/'backup'),job=tmp,pid=os.getpid(),version='0.2.19')
            self.assertFalse(u.install(c,wait_seconds=0));self.assertEqual((a/'marker').read_text(),'old')
    def test_locked_installation_reopens_unchanged_app(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);app=root/'app';app.mkdir();(app/'stepx-companion').write_text('old')
            c=dict(target=str(app),stage=str(root/'stage'),backup=str(root/'backup'),job=tmp,pid=99999999,version='0.2.19',relative_exe='stepx-companion',nonce='test')
            with patch.object(u,'replace_directory',side_effect=PermissionError('locked')),patch.object(u,'launch') as launch:
                self.assertFalse(u.install(c));launch.assert_called_once()
            self.assertEqual((app/'stepx-companion').read_text(),'old')
            self.assertEqual(json.loads((root/'status.json').read_text())['state'],'failed')
    @unittest.skipIf(os.name=='nt','POSIX fixture launch; Windows paths covered separately')
    def test_actual_process_install_and_startup_rollback(self):
        for success in [True,False]:
            with self.subTest(success=success),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);a=root/'app';stage=root/'stage';a.mkdir();stage.mkdir()
                old=a/'stepx-companion';old.write_text('#!/usr/bin/env python3\n');old.chmod(0o755)
                new=stage/'stepx-companion'
                new.write_text('#!/usr/bin/env python3\nimport sys,json,time\nfrom pathlib import Path\np=Path(sys.argv[2]);d=json.loads(p.read_text());\n'+
                    ("p.with_name('ack.json').write_text(json.dumps({'nonce':d['nonce'],'version':d['version']}))\ntime.sleep(3)\n" if success else 'sys.exit(1)\n'))
                new.chmod(0o755)
                c=dict(target=str(a),stage=str(stage),backup=str(root/'backup'),job=tmp,pid=99999999,version='0.2.19',relative_exe='stepx-companion',nonce='test')
                u.atomic_json(root/'install.json',c)
                # Production must leave the accepted app (or restored app) open.
                # This fixture owns both child processes and reaps them before
                # removing their working directory, including assertion failures.
                children=[];launch=u.launch
                def record_launch(*args,**kwargs):
                    child=launch(*args,**kwargs);children.append(child);return child
                try:
                    with patch.object(u,'launch',side_effect=record_launch):
                        result=u.install(c,ack_seconds=2)
                    self.assertEqual(result,success)
                    status=json.loads((root/'status.json').read_text())
                    self.assertEqual(status['state'],'installed' if success else 'rolled_back')
                    if success:self.assertTrue((root/'backup/stepx-companion').exists())
                    else:self.assertEqual((a/'stepx-companion').read_text(),'#!/usr/bin/env python3\n')
                finally:
                    for child in children:
                        if child.poll() is None:child.terminate()
                        try:child.wait(timeout=2)
                        except subprocess.TimeoutExpired:child.kill();child.wait(timeout=2)

    def _valid_package_bytes(self, version='0.2.19', platform='linux-x86_64'):
        buf = io.BytesIO()
        stem = f'StepX-Companion-{version}-{platform}'
        with tarfile.open(fileobj=buf, mode='w:gz') as t:
            for rel, content in [
                (f'{stem}/stepx-companion', b'#!/bin/sh\n'),
                (f'{stem}/scripts/ok.py', b'print(1)\n'),
                (f'{stem}/desktop/assets/stepx-companion.apk', b'PK\x03\x04apk'),
                (f'{stem}/runtime/python/bin/python3', b'#!/bin/sh\n'),
                (f'{stem}/tools/bin/adb', b'adb'),
            ]:
                m = tarfile.TarInfo(rel)
                m.size = len(content)
                m.mode = 0o755
                t.addfile(m, io.BytesIO(content))
        return buf.getvalue()

    def test_mirror_fallback_and_mandatory_sha256_verification(self):
        pkg = self._valid_package_bytes()
        sha = hashlib.sha256(pkg).hexdigest()
        rel_json = self.release('linux-x86_64')
        rel_json['assets'][0]['size'] = len(pkg)
        rel_json['assets'][0]['digest'] = None
        asset_name = rel_json['assets'][0]['name']
        dl_url = rel_json['assets'][0]['browser_download_url']
        rel_json['assets'].append({
            'name': asset_name + '.sha256',
            'size': 90,
            'browser_download_url': dl_url + '.sha256',
        })
        seen_urls = []

        def fake_open(url, headers=None, timeout=15):
            seen_urls.append(url)
            if url == u.API:
                return io.BytesIO(json.dumps(rel_json).encode('utf-8'))
            if url == dl_url + '.sha256':
                return io.BytesIO(f'{sha} *{asset_name}\n'.encode('ascii'))
            if url == dl_url:
                raise urllib.error.URLError('primary github release asset blocked')
            if url == 'https://ghfast.top/' + dl_url:
                return io.BytesIO(pkg)
            raise AssertionError(f'unexpected url {url}')

        import urllib.error
        # Metadata & trust digest must NEVER fall back to untrusted third-party proxies
        blocked_api_urls = []
        def blocked_api_open(url, headers=None, timeout=15):
            blocked_api_urls.append(url)
            raise urllib.error.URLError('primary github api blocked')

        with patch.object(u, 'open_url', side_effect=blocked_api_open):
            with self.assertRaises(Exception):
                u.check('linux-x86_64')
        self.assertNotIn('https://ghfast.top/' + u.API, blocked_api_urls)

        with tempfile.TemporaryDirectory() as tmp:
            staging = Path(tmp)
            with patch.object(u, 'open_url', side_effect=fake_open):
                parsed = u.check('linux-x86_64')
                u.download(parsed, staging / 'job-mirror')
            self.assertTrue((staging / 'job-mirror' / 'ready.json').is_file())
            self.assertIn(u.API, seen_urls)
            self.assertIn(dl_url + '.sha256', seen_urls)
            self.assertNotIn('https://ghfast.top/' + u.API, seen_urls)
            self.assertIn('https://ghfast.top/' + dl_url, seen_urls)

        # Mirror returning tampered payload must fail SHA-256 and delete .part
        tampered = pkg[:-1] + bytes([pkg[-1] ^ 0xFF])
        def tampered_open(url, headers=None, timeout=15):
            if url.endswith('.sha256'):
                return io.BytesIO(f'{sha}  {asset_name}\n'.encode('ascii'))
            return io.BytesIO(tampered)

        with tempfile.TemporaryDirectory() as tmp:
            staging = Path(tmp)
            parsed = u.parse_release(rel_json, 'linux-x86_64')
            with patch.object(u, 'open_url', side_effect=tampered_open):
                with self.assertRaisesRegex(ValueError, 'SHA-256'):
                    u.download(parsed, staging / 'job-bad')
            part = u.part_cache_path(parsed, staging / 'job-bad')
            self.assertFalse(part.exists())
            self.assertFalse((staging / 'job-bad' / 'ready.json').exists())

    def test_range_resume_http_416_and_stall_watchdog(self):
        import urllib.error
        pkg = self._valid_package_bytes()
        sha = hashlib.sha256(pkg).hexdigest()
        rel_json = self.release('linux-x86_64')
        rel_json['assets'][0]['size'] = len(pkg)
        rel_json['assets'][0]['digest'] = 'sha256:' + sha
        parsed = u.parse_release(rel_json, 'linux-x86_64')
        split = len(pkg) // 2

        class FakeResp(io.BytesIO):
            def __init__(self, data, status=200, headers=None):
                super().__init__(data)
                self.status = status
                self.headers = headers or {}
            def __enter__(self):
                return self
            def __exit__(self, *args):
                self.close()

        with tempfile.TemporaryDirectory() as tmp:
            staging = Path(tmp)
            part = u.part_cache_path(parsed, staging / 'job-1')

            # 1) First attempt stalls / drops halfway -> preserves deterministic .part
            with patch.object(u, 'open_url', side_effect=lambda url, headers=None, timeout=15: FakeResp(pkg[:split] if not headers else b'', status=200)):
                with self.assertRaisesRegex(ValueError, '下载未完成'):
                    u.download(parsed, staging / 'job-1')
            self.assertTrue(part.is_file())
            self.assertEqual(part.stat().st_size, split)

            # 2) Second job resumes with Range: bytes=<split>- and 206 Partial Content
            recorded_headers = []
            def resume_open(url, headers=None, timeout=15):
                recorded_headers.append(headers)
                self.assertEqual(headers, {'Range': f'bytes={split}-'})
                return FakeResp(
                    pkg[split:],
                    status=206,
                    headers={'Content-Range': f'bytes {split}-{len(pkg)-1}/{len(pkg)}'},
                )
            with patch.object(u, 'open_url', side_effect=resume_open):
                u.download(parsed, staging / 'job-2')
            self.assertEqual(len(recorded_headers), 1)
            self.assertTrue((staging / 'job-2' / 'ready.json').is_file())

            # 3) HTTP 416 deletes stale .part
            part.write_bytes(b'stale-partial-bytes')
            def fail_416(url, headers=None, timeout=15):
                fp = io.BytesIO(b'')
                err = urllib.error.HTTPError(url, 416, 'Range Not Satisfiable', {}, fp)
                fp.close()
                raise err
            with patch.object(u, 'open_url', side_effect=fail_416):
                with self.assertRaises(urllib.error.HTTPError):
                    u.download(parsed, staging / 'job-3')
            self.assertFalse(part.exists())

            # 4) Stall watchdog (<64KB / window) preserves .part and aborts stalled stream
            class SlowStream:
                status = 200
                headers = {}
                def __init__(self):
                    self.calls = 0
                def read(self, n):
                    self.calls += 1
                    if self.calls == 1:
                        return pkg[:1024]
                    return b'x'
                def __enter__(self): return self
                def __exit__(self, *a): pass

            ticks = iter([0.0, 0.1, 16.0, 16.1, 32.0, 32.1, 48.0, 48.1, 64.0])
            with patch.object(u, 'open_url', side_effect=lambda *a, **kw: SlowStream()), \
                 patch.object(u.time, 'monotonic', side_effect=lambda: next(ticks, 100.0)):
                with self.assertRaises(TimeoutError):
                    u.download(parsed, staging / 'job-4')
            self.assertTrue(part.is_file())

    def test_windows_prepare_creationflags_symlinks_and_closed_log_fd(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'space' / 'sub'
            install_dir = root / 'StepX'
            (install_dir / 'desktop/assets').mkdir(parents=True)
            (install_dir / 'desktop/assets/stepx-companion.apk').write_bytes(b'apk')
            (install_dir / 'scripts').mkdir(parents=True)
            exe = install_dir / 'stepx-companion.exe'
            exe.write_bytes(b'old-exe')

            job = root / 'desktop-updates/job-win'
            job.mkdir(parents=True)
            v = '0.2.19'
            stem = f'StepX-Companion-{v}-windows-x86_64'
            archive = job / 'package.part'
            with zipfile.ZipFile(archive, 'w') as z:
                z.writestr(f'{stem}/stepx-companion.exe', b'new-exe')
                z.writestr(f'{stem}/scripts/ok.py', b'1')
                z.writestr(f'{stem}/desktop/assets/stepx-companion.apk', b'apk')
                z.writestr(f'{stem}/runtime/python/python.exe', b'py')
                z.writestr(f'{stem}/tools/bin/adb.exe', b'adb')
            sha = u.digest_file(archive)
            rel_json = self.release('windows-x86_64')
            rel_json['assets'][0]['size'] = archive.stat().st_size
            rel_json['assets'][0]['digest'] = 'sha256:' + sha
            parsed = u.parse_release(rel_json, 'windows-x86_64')
            u.atomic_json(job / 'ready.json', dict(release=parsed, sha256=sha, payload='', exe=''))

            captured = {}
            real_copytree = u.shutil.copytree

            def spy_copytree(src, dst, *args, **kwargs):
                if str(dst).endswith('installer-python'):
                    captured['runtime_symlinks'] = kwargs.get('symlinks')
                return real_copytree(src, dst, *args, **kwargs)

            def fake_popen(cmd, **kwargs):
                captured['creationflags'] = kwargs.get('creationflags')
                captured['stdout_closed_inside'] = kwargs['stdout'].closed
                captured['stdout_obj'] = kwargs['stdout']
                (job / 'installer-ready').touch()
                class Dummy: pass
                return Dummy()

            import pathlib
            path_cls = pathlib.WindowsPath if os.name == 'nt' else pathlib.PosixPath
            with patch.object(u.os, 'name', 'nt'), \
                 patch.object(u, 'Path', path_cls), \
                 patch.object(u.shutil, 'copytree', side_effect=spy_copytree), \
                 patch.object(u.subprocess, 'Popen', side_effect=fake_popen):
                u.prepare(job, exe, 12345)

            self.assertIs(captured.get('runtime_symlinks'), True)
            self.assertEqual(captured.get('creationflags'), 0x08000000 | 0x00000200)
            self.assertEqual(captured.get('creationflags') & 0x00000008, 0)
            self.assertFalse(captured['stdout_closed_inside'])
            self.assertTrue(captured['stdout_obj'].closed)
if __name__=='__main__':unittest.main()
