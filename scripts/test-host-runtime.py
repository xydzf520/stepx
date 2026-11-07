"""Portable runtime regression tests. No phone, account or network service needed."""
import contextlib
import http.server
import io
import json
import os
from pathlib import Path
import runpy
import sys
import tempfile
import threading
import types
import unittest
from unittest.mock import patch

from host_runtime import adb_executable, build_tools, java_environment, migrate_legacy_state, sdk_tool, user_state_root


class HostRuntimeTests(unittest.TestCase):
    def test_adb_executable_precedence(self):
        exe = 'adb.exe' if os.name == 'nt' else 'adb'
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            packaged_adb = root / 'tools/bin' / exe
            local_adb = root / '.tools/bin' / exe
            local_adb.parent.mkdir(parents=True)
            local_adb.touch()
            with patch('host_runtime.ROOT', root), patch.dict(os.environ, {}, clear=True), \
                 patch('pathlib.Path.home', side_effect=AssertionError('Bundled ADB needs no user profile')):
                self.assertEqual(adb_executable(), str(local_adb))
                packaged_adb.parent.mkdir(parents=True)
                packaged_adb.touch()
                self.assertEqual(adb_executable(), str(packaged_adb))
            with patch('host_runtime.ROOT', root), patch.dict(os.environ, {'STEPX_ADB': '/custom/adb'}, clear=True):
                self.assertEqual(adb_executable(), '/custom/adb')

    def test_adb_sdk_and_path_fallback_without_user_profile(self):
        exe = 'adb.exe' if os.name == 'nt' else 'adb'
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            sdk_adb = root / 'sdk/platform-tools' / exe
            sdk_adb.parent.mkdir(parents=True)
            sdk_adb.touch()
            with patch('host_runtime.ROOT', root), patch.dict(os.environ, {'ANDROID_SDK_ROOT': str(root / 'sdk')}, clear=True), \
                 patch('pathlib.Path.home', side_effect=AssertionError('Explicit SDK needs no user profile')):
                self.assertEqual(adb_executable(), str(sdk_adb))
            with patch('host_runtime.ROOT', root), patch.dict(os.environ, {}, clear=True), \
                 patch('pathlib.Path.home', side_effect=RuntimeError('Could not determine home directory.')), \
                 patch('shutil.which', return_value=str(root / 'path' / exe)):
                self.assertEqual(adb_executable(), str(root / 'path' / exe))
            with patch('host_runtime.ROOT', root), patch.dict(os.environ, {}, clear=True), \
                 patch('pathlib.Path.home', side_effect=RuntimeError('Could not determine home directory.')), \
                 patch('shutil.which', return_value=None):
                self.assertEqual(adb_executable(), 'adb')

    def test_bundled_java_without_user_profile(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            bundled = root / 'runtime/java'
            (bundled / 'bin').mkdir(parents=True)
            (bundled / 'bin' / ('java.exe' if os.name == 'nt' else 'java')).touch()
            with patch('host_runtime.ROOT', root), patch.dict(os.environ, {}, clear=True), \
                 patch('pathlib.Path.home', side_effect=RuntimeError('Could not determine home directory.')):
                environment = java_environment()
                self.assertEqual(environment['JAVA_HOME'], str(bundled))
                self.assertEqual(environment['PATH'].split(os.pathsep)[0], str(bundled / 'bin'))

    def test_sdk_tool_fallback_without_user_profile(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with patch('host_runtime.ROOT', root), patch.dict(os.environ, {}, clear=True), \
                 patch('pathlib.Path.home', side_effect=RuntimeError('Could not determine home directory.')), \
                 patch('shutil.which', return_value=str(root / 'aapt2')):
                self.assertEqual(list(build_tools()), [root / 'runtime/android-build-tools', root / '.tools/android-build-tools'])
                self.assertEqual(sdk_tool('aapt2'), str(root / 'aapt2'))

    def test_bundled_java_takes_precedence_over_system_java_home(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);bundled=root/'runtime/java';external=root/'other-java'
            for home in [bundled,external]:
                (home/'bin').mkdir(parents=True)
                (home/'bin'/('java.exe' if os.name=='nt' else 'java')).touch()
            with patch('host_runtime.ROOT', root), patch('pathlib.Path.home', return_value=root/'user'), \
                 patch.dict(os.environ, {'JAVA_HOME':str(external)}, clear=True):
                self.assertEqual(java_environment()['JAVA_HOME'],str(bundled))

    def test_native_user_storage(self):
        with patch.dict(os.environ, {}, clear=True), patch('pathlib.Path.home', return_value=Path('/user')):
            with patch.object(sys, 'platform', 'darwin'):
                if os.name != 'nt':
                    self.assertEqual(user_state_root(), Path('/user/Library/Application Support/StepX Companion'))
        with patch.dict(os.environ, {'STEPX_STATE_DIR': str(Path.cwd() / 'custom state')}):
            self.assertEqual(user_state_root(), Path.cwd() / 'custom state')

    def test_recovery_migration_preserves_old_and_new_receipts(self):
        with tempfile.TemporaryDirectory() as temp:
            old, new = Path(temp) / 'old', Path(temp) / 'data/state'
            old.mkdir(); (old / 'baseline.json').write_text('{"original":true}')
            migrate_legacy_state(old, new)
            self.assertEqual((new / 'baseline.json').read_bytes(), (old / 'baseline.json').read_bytes())
            (new / 'baseline.json').write_text('{"current":true}')
            migrate_legacy_state(old, new)
            self.assertEqual(json.loads((new / 'baseline.json').read_text()), {'current': True})
            self.assertEqual(json.loads((old / 'baseline.json').read_text()), {'original': True})

    @unittest.skipIf(os.name == 'nt', 'Creating symlinks requires Windows developer mode')
    def test_recovery_migration_rejects_links_without_partial_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            old, new = Path(temp) / 'old', Path(temp) / 'data/state'
            old.mkdir(); (old / 'linked').symlink_to('/tmp')
            with self.assertRaises(RuntimeError):
                migrate_legacy_state(old, new)
            self.assertFalse(new.exists())
            self.assertTrue((old / 'linked').is_symlink())

    def test_adb_loopback_ignores_external_proxy(self):
        received = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                received.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
                body = b'{"stdout":"local bridge reached"}'
                self.send_response(200); self.end_headers(); self.wfile.write(body)

            def log_message(self, *args):
                pass

        server = http.server.HTTPServer(('127.0.0.1', 0), Handler)
        worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
        common = types.SimpleNamespace(ADB=['adb', '-s', 'test-device'], guard=lambda: None)
        gateway = Path(__file__).resolve().parents[1] / 'handoff/google-play-stepx/tools/gateway.py'
        try:
            with patch.dict(sys.modules, {'common': common}), patch.dict(os.environ, {
                'http_proxy': 'http://127.0.0.1:1', 'HTTP_PROXY': 'http://127.0.0.1:1',
                'no_proxy': '', 'NO_PROXY': '',
            }), patch('subprocess.check_output', side_effect=['test-android-id', str(server.server_port)]), \
                 patch('subprocess.run'), patch('sys.stdin', io.StringIO('{"command":["probe"]}')), \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                runpy.run_path(str(gateway), run_name='__main__')
                self.assertEqual(json.loads(output.getvalue()), {'stdout': 'local bridge reached'})
            self.assertEqual(received, [{'command': ['probe']}])
        finally:
            server.shutdown(); server.server_close(); worker.join(timeout=3)


if __name__ == '__main__':
    unittest.main()
