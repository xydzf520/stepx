"""Execute the real staging protocol in a temporary directory, never sudo/ADB."""
import ast
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import shlex
import tempfile
import unittest

TOOLS = Path(__file__).resolve().parents[2] / 'handoff/google-play-stepx/tools'
spec = importlib.util.spec_from_file_location('bounded_root_bridge', TOOLS / 'root_bridge.py')
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)


@unittest.skipUnless(os.name == 'posix', 'executes Linux-side NOFOLLOW staging protocol')
class RootBridgeStagingTests(unittest.TestCase):
    def transport(self, directory, *, tamper=False, fail_chunk=False, fifo=False):
        calls = []
        def send(command):
            calls.append(command)
            self.assertEqual(command[:2], ['python3', '-c'])
            source = command[2].replace('/tmp/stepx-root-', str(Path(directory) / 'stepx-root-'))
            if source.startswith('import subprocess;'):
                call = next(n for n in ast.walk(ast.parse(source)) if isinstance(n, ast.Call)
                            and isinstance(n.func, ast.Attribute) and n.func.attr == 'run')
                argv = ast.literal_eval(call.args[0])
                self.assertLessEqual(sum(len(a.encode()) + 1 for a in argv), 1900)
                hook = next(a.split('::=', 1)[1] for a in argv if 'Pre-Invoke::=' in a)
                self.assertTrue(hook.endswith('; exit 73'))
                arguments = shlex.split(hook.removesuffix('; exit 73'))
                self.assertEqual(arguments[:3], ['/usr/bin/python3', '-I', '-c'])
                code = arguments[3]
                if tamper:
                    for file in Path(directory).glob('stepx-root-*.py'):
                        file.write_bytes(b'raise RuntimeError("tampered code executed")')
                if fifo:
                    for file in Path(directory).glob('stepx-root-*.py'):
                        file.unlink();os.mkfifo(file)
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    exec(compile(code, '<test-root>', 'exec'), {})
                return {'exitCode': 0, 'stdout': output.getvalue()}
            if fail_chunk and 'base64.b64decode' in source:
                raise RuntimeError('transport interrupted')
            exec(compile(source, '<test-gateway>', 'exec'), {})
            return {'exitCode': 0}
        return send, calls

    def test_large_unicode_operation_uses_bounded_ipc_and_removes_source(self):
        with tempfile.TemporaryDirectory() as directory:
            send, calls = self.transport(directory)
            result = bridge.execute('value=' + repr('测试载荷' * 1800) + '\nprint(len(value))', send)
            self.assertEqual(result['stdout'].strip(), '7200')
            self.assertGreater(len(calls), 8)
            self.assertFalse(list(Path(directory).iterdir()))

    def test_short_probe_stays_one_request(self):
        with tempfile.TemporaryDirectory() as directory:
            send, calls = self.transport(directory)
            self.assertEqual(bridge.execute('print("probe")', send)['stdout'].strip(), 'probe')
            self.assertEqual(len(calls), 1)

    def test_heavily_quoted_short_source_falls_back_to_staging(self):
        with tempfile.TemporaryDirectory() as directory:
            send, calls = self.transport(directory)
            source = "#" + "'" * 550 + '\nprint("ok")'
            self.assertEqual(bridge.execute(source, send)['stdout'].strip(), 'ok')
            self.assertGreater(len(calls), 1)
            self.assertFalse(list(Path(directory).iterdir()))

    def test_fifo_substitution_fails_without_blocking(self):
        with tempfile.TemporaryDirectory() as directory:
            send, _ = self.transport(directory, fifo=True)
            with self.assertRaisesRegex(RuntimeError, 'Invalid staged file'):
                bridge.execute('#' * 7000 + '\nprint("ok")', send)
            self.assertFalse(list(Path(directory).iterdir()))

    def test_tampered_staged_bytes_never_execute(self):
        with tempfile.TemporaryDirectory() as directory:
            send, _ = self.transport(directory, tamper=True)
            with self.assertRaisesRegex(RuntimeError, 'digest mismatch'):
                bridge.execute('#' * 7000 + '\nprint("ok")', send)
            self.assertFalse(list(Path(directory).iterdir()))

    def test_failed_transfer_cleans_up_without_privileged_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            send, calls = self.transport(directory, fail_chunk=True)
            with self.assertRaisesRegex(RuntimeError, 'interrupted'):
                bridge.execute('#' * 7000 + '\nprint("ok")', send)
            self.assertFalse(any(c[2].startswith('import subprocess;') for c in calls))
            self.assertFalse(list(Path(directory).iterdir()))

    def test_invalid_source_fails_before_any_transport(self):
        for source in ('', 'bad (', '#' * 1048577):
            with self.subTest(source_length=len(source)):
                with self.assertRaises((RuntimeError, SyntaxError)):
                    bridge.execute(source, lambda _: self.fail('must not contact phone'))


if __name__ == '__main__':
    unittest.main()
