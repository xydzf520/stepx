"""Verify large system-service installers before any phone process is launched."""
import ast
import base64
import hashlib
from pathlib import Path
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[2] / 'handoff/google-play-stepx/tools/root_worker.py'


class WorkerStagingTest(unittest.TestCase):
    def stage(self, gateway):
        module = ast.parse(SOURCE.read_text())
        function = next(node for node in module.body if isinstance(node, ast.FunctionDef) and node.name == 'stage_source')
        scope = {'gw': gateway, 'base64': base64, 'hashlib': hashlib}
        exec(compile(ast.Module(body=[function], type_ignores=[]), str(SOURCE), 'exec'), scope)
        return scope['stage_source']

    def test_large_unicode_worker_roundtrip_and_no_overwrite(self):
        commands = []
        def gateway(command):
            self.assertLess(len(command), 12000)
            commands.append(command)
            exec(command, {})
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'worker.py'
            body = '长文本测试 ABC\n' * 20000
            stage = self.stage(gateway)
            stage(str(path), body)
            self.assertEqual(path.read_text(), body)
            self.assertGreater(len(commands), 20)
            with self.assertRaises(FileExistsError):
                stage(str(path), 'replacement')
            self.assertEqual(path.read_text(), body)

    def test_failed_write_removes_partial_non_executable_worker(self):
        count = 0
        def gateway(command):
            nonlocal count
            count += 1
            if count == 3:
                raise RuntimeError('disconnected')
            exec(command, {})
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'worker.py'
            with self.assertRaisesRegex(RuntimeError, 'disconnected'):
                self.stage(gateway)(str(path), 'x' * 50000)
            self.assertFalse(path.exists())

    def test_oversize_rejected_without_phone_calls(self):
        def gateway(command):
            self.fail('must not stage invalid payload')
        with self.assertRaises(RuntimeError):
            self.stage(gateway)('/tmp/unused', 'x' * (512 * 1024 + 1))


if __name__ == '__main__':
    unittest.main()
