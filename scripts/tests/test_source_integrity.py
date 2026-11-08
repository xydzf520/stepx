"""Cross-platform source bytes and exact device integrity; no phone operations."""
import ast
import base64
import copy
import hashlib
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from source_integrity import canonical_python_source, read_python_source, python_source_sha256


def tree(relative):
    return ast.parse((ROOT / relative).read_text(encoding='utf-8'))


def assignment(nodes, name):
    return next(node for node in nodes if isinstance(node, ast.Assign)
                and any(isinstance(target, ast.Name) and target.id == name for target in node.targets))


def execute(nodes, scope):
    exec(compile(ast.fix_missing_locations(ast.Module(body=nodes, type_ignores=[])), '<source-integrity-test>', 'exec'), scope)


class CanonicalSourceTests(unittest.TestCase):
    def test_crlf_lf_and_utf8_produce_identical_reviewed_bytes(self):
        lf = "#!/usr/bin/python3\nprint('演示资料')\n".encode('utf-8')
        for data in (lf, lf.replace(b'\n', b'\r\n'), lf.replace(b'\n', b'\r\n', 1)):
            self.assertEqual(canonical_python_source(data), lf)
        self.assertNotEqual(hashlib.sha256(lf).digest(), hashlib.sha256(lf + b'# changed\n').digest())

    def test_invalid_encoding_and_bare_cr_fail_closed(self):
        for data in (b'print(1)\rprint(2)\n', b'print(1)\r\n# bad\r', b'\xff\n'):
            with self.subTest(data=data), self.assertRaises(ValueError):
                canonical_python_source(data)

    def test_binary_and_recovery_digest_stays_exact(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'backup.bin'
            data = b'\xff\x00\r\n\rRAW'; path.write_bytes(data)
            with self.assertRaises(ValueError):
                read_python_source(path)
            # Execute the real Google generic SHA helper, without importing its
            # config or starting ADB. Binary/backups retain every byte.
            fn = next(n for n in tree('handoff/google-play-stepx/tools/common.py').body
                      if isinstance(n, ast.FunctionDef) and n.name == 'sha256')
            scope = {'hashlib': hashlib}; execute([fn], scope)
            self.assertEqual(scope['sha256'](path), hashlib.sha256(data).hexdigest())
            self.assertNotEqual(scope['sha256'](path), hashlib.sha256(data.replace(b'\r\n', b'\n')).hexdigest())

    def test_permission_guard_accepts_local_crlf_but_not_changed_device_bytes(self):
        fn = copy.deepcopy(next(n for n in tree('scripts/companion-permissions.py').body
                                if isinstance(n, ast.FunctionDef) and n.name == 'run_bounded_android_worker'))
        # Run the actual read-only launcher guards and stop before the first
        # subprocess/permission operation. Device proof is raw-byte SHA only.
        end = next(i for i, n in enumerate(fn.body)
                   if isinstance(n, ast.Assign) and any(isinstance(t, ast.Name) and t.id == 'google_before' for t in n.targets))
        fn.body = fn.body[:end] + [ast.Return(value=ast.Constant(True))]
        with tempfile.TemporaryDirectory() as directory:
            local = Path(directory); (local / 'module').mkdir()
            lf = b"#!/usr/bin/python3\nprint('reviewed')\n"
            (local / 'module/launcher.py').write_bytes(lf.replace(b'\n', b'\r\n'))
            for device_bytes, accepted in ((lf, True), (lf.replace(b'\n', b'\r\n'), False), (lf + b'# tampered\n', False)):
                with self.subTest(device_bytes=device_bytes):
                    proof = Mock(return_value={'launcher': hashlib.sha256(device_bytes).hexdigest(), 'root_available': True})
                    scope = {'STATE': local, 'WORKFLOW': local, 'CONFIG': {}, 'common': SimpleNamespace(),
                             'shell': Mock(return_value=''), 'root_json': proof,
                             'python_source_sha256': python_source_sha256}
                    execute([fn], scope)
                    if accepted:
                        self.assertTrue(scope[fn.name]('pass', 123))
                    else:
                        with self.assertRaisesRegex(RuntimeError, 'launcher differs'):
                            scope[fn.name]('pass', 123)
                    self.assertIn("open('/usr/local/bin/ubuntu_zygote','rb')", proof.call_args.args[0])
                    self.assertNotIn('replace(', proof.call_args.args[0])

    def test_companion_boot_expected_hash_and_transferred_sources_are_lf(self):
        module = tree('scripts/companion-boot.py')
        fn = next(n for n in module.body if isinstance(n, ast.FunctionDef) and n.name == 'install')
        with tempfile.TemporaryDirectory() as directory:
            local = Path(directory); source_dir = local / 'scripts/companion_boot'; source_dir.mkdir(parents=True)
            (local / 'module').mkdir()
            lf = b"#!/usr/bin/python3\nprint('reviewed')\n"
            for name in ('chain.py', 'boot.py', 'launcher.py'):
                (source_dir / name).write_bytes(lf.replace(b'\n', b'\r\n'))
            (local / 'module/boot.py').write_bytes(lf.replace(b'\n', b'\r\n'))
            scope = {'ROOT': local, 'p': SimpleNamespace(WORKFLOW=local),
                     'read_python_source': read_python_source, 'python_source_sha256': python_source_sha256}
            execute([assignment(module.body, 'ORIGINAL_SHA'), assignment(fn.body, 'files')], scope)
            self.assertEqual(scope['ORIGINAL_SHA'], hashlib.sha256(lf).hexdigest())
            self.assertEqual(scope['files'], {name: lf for name in ('chain.py', 'boot.py', 'launcher.py')})

    def test_google_stages_the_same_canonical_bytes_it_hashes(self):
        module = tree('handoff/google-play-stepx/tools/install_adapter.py')
        with tempfile.TemporaryDirectory() as directory:
            local = Path(directory); (local / 'module').mkdir()
            lf = b"#!/usr/bin/python3\nprint('reviewed')\n"
            for name in ('boot.py', 'inject.py', 'launcher.py'):
                (local / 'module' / name).write_bytes(lf.replace(b'\n', b'\r\n'))
            stage = local / 'staged'
            scope = {'ROOT': local, 'read_python_source': read_python_source, 'hashlib': hashlib,
                     'base64': base64, 'stage': str(stage)}
            execute([assignment(module.body, name) for name in ('files', 'manifest', 'remote')], scope)
            exec(scope['remote'], {})
            for name, expected in scope['manifest'].items():
                self.assertEqual((stage / name).read_bytes(), lf)
                self.assertEqual(hashlib.sha256((stage / name).read_bytes()).hexdigest(), expected)
                (stage / name).write_bytes(lf.replace(b'\n', b'\r\n'))
                self.assertNotEqual(hashlib.sha256((stage / name).read_bytes()).hexdigest(), expected)

    def test_google_repair_and_metadata_expect_the_canonical_module(self):
        module = tree('handoff/google-play-stepx/tools/manage.py')
        with tempfile.TemporaryDirectory() as directory:
            local = Path(directory); (local / 'module').mkdir()
            lf = b'print(1)\n'
            for name in ('boot.py', 'inject.py', 'launcher.py'):
                (local / 'module' / name).write_bytes(lf.replace(b'\n', b'\r\n'))
            wrapper = next(n for n in module.body if isinstance(n, ast.FunctionDef) and n.name == 'repair_runtime')
            self.assertIn('operation_lock()', ast.unparse(wrapper))
            self.assertIn('_repair_runtime()', ast.unparse(wrapper))
            for function in ('_repair_runtime', 'recover_metadata'):
                fn = next(n for n in module.body if isinstance(n, ast.FunctionDef) and n.name == function)
                scope = {'ROOT': local, 'python_source_sha256': python_source_sha256}
                execute([assignment(fn.body, 'expected')], scope)
                self.assertTrue(scope['expected'])
                self.assertEqual(set(scope['expected'].values()), {hashlib.sha256(lf).hexdigest()})

    def test_audited_loader_and_worker_pins_are_unchanged(self):
        module = tree('scripts/companion-permissions.py')
        scope = {}; execute([assignment(module.body, name) for name in ('LOADER_SHA256', 'WORKER_SHA256')], scope)
        for relative, pin in [('handoff/google-play-stepx/module/inject.py', 'LOADER_SHA256'),
                              ('handoff/google-play-stepx/tools/root_worker.py', 'WORKER_SHA256')]:
            source = (ROOT / relative).read_bytes()
            for data in (source, source.replace(b'\n', b'\r\n')):
                self.assertEqual(hashlib.sha256(canonical_python_source(data)).hexdigest(), scope[pin])
            self.assertNotEqual(hashlib.sha256(canonical_python_source(source + b'# changed\n')).hexdigest(), scope[pin])


if __name__ == '__main__':
    unittest.main()
