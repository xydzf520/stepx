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
        for url in ['http://github.com/x','https://evil.test/x','https://github.com@evil.test/x','https://github.com:90/x','https://github.com/x#bad']:
            with self.assertRaises(ValueError):u.trusted_url(url)
        u.trusted_url('https://release-assets.githubusercontent.com/x?sig=example')
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
if __name__=='__main__':unittest.main()
