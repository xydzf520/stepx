#!/usr/bin/env python3
"""Privacy boundaries: fail before publishing, preserve developer/user data."""
import hashlib
import importlib.util
import io
import json
import os
import stat
import struct
import subprocess
from pathlib import Path
import sys
import tarfile
import tempfile
import types
import unittest
import urllib.error
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import public_release_policy as policy
import release_download as downloads


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


stage = load('release_stage', 'stage-release-resources.py').stage
export = load('release_export', 'sync-public-repo.py')
package = load('release_package', 'package-release.py')
runtime_pkg = load('release_runtime', 'package-runtime.py')


class PackagedGoogleImportsTests(unittest.TestCase):
    def test_google_entry_points_import_from_staged_resources_without_checkout(self):
        # Use the real release allowlist and an isolated interpreter. Importing
        # from the checkout can hide a missing transitive runtime dependency.
        with tempfile.TemporaryDirectory(prefix='StepX packaged imports ') as td:
            root = Path(td) / 'resources'
            stage(SCRIPTS.parent, root)
            code = """import importlib, importlib.util, subprocess, sys
from pathlib import Path
root = Path(sys.argv[1])
sys.path[:0] = [str(root/'scripts'), str(root/'handoff/google-play-stepx/tools')]
def forbidden(*args, **kwargs):
    raise AssertionError('Import verification must not invoke external tools')
subprocess.run = subprocess.Popen = subprocess.check_output = forbidden
for name in ['common', 'manage', 'health', 'install_recovery', 'workflow_errors']:
    module = importlib.import_module(name)
    assert Path(module.__file__).resolve().is_relative_to(root.resolve())
for name in ['companion-permissions', 'companion-power', 'companion-system']:
    spec = importlib.util.spec_from_file_location(name, root/'scripts'/(name+'.py'))
    spec.loader.exec_module(importlib.util.module_from_spec(spec))
print('Isolated packaged Google imports: PASS')
"""
            env = {key: value for key, value in os.environ.items()
                   if not key.startswith('STEPX_') and key not in ('PYTHONPATH', 'PYTHONHOME')}
            env['STEPX_STATE_DIR'] = str(Path(td) / 'isolated-user-data')
            result = subprocess.run([sys.executable, '-I', '-B', '-c', code, str(root)],
                                    cwd=td, env=env, capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_verify_packaged_runtime_asserts_offline_google_play_apks_and_digests(self):
        verifier_src = (SCRIPTS / 'verify-packaged-runtime.py').read_text(encoding='utf-8')
        self.assertIn("google_dir=root/'runtime/google-play'", verifier_src)
        self.assertIn("manage._bundled_google_apks_ready(", verifier_src)
        self.assertIn("hashlib.sha256(bundled_apk.read_bytes()).hexdigest()==entry['sha256']", verifier_src)


class ToolDownloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.target = Path(self.temp.name) / 'tool.tar.gz'
        self.url = 'https://github.com/example/tool/releases/download/v1/tool.tar.gz'
        self.payload = b'verified-upstream-tool'
        self.digest = hashlib.sha256(self.payload).hexdigest()
        self.sleep = mock.patch.object(downloads.time, 'sleep').start()
        self.addCleanup(mock.patch.stopall)

    def test_transient_failure_retries_fresh_request_and_keeps_digest_verification(self):
        failure = urllib.error.HTTPError(self.url, 504, 'Gateway timeout', {}, None)
        with mock.patch.object(downloads.urllib.request, 'urlopen', side_effect=[failure, io.BytesIO(self.payload)]) as opener:
            package.download_verified_tool(self.url, self.target, self.digest)
        self.assertEqual(self.target.read_bytes(), self.payload)
        self.assertEqual(opener.call_args_list[0].args[0].full_url, self.url)
        self.assertTrue(opener.call_args_list[1].args[0].full_url.startswith(self.url + '?stepx_retry='))
        self.assertEqual(opener.call_args_list[1].kwargs['timeout'], 30)
        self.assertEqual(list(self.target.parent.glob('*.part')), [])

    def test_wrong_hash_never_replaces_existing_archive_or_retries(self):
        self.target.write_bytes(b'previous-verified-archive')
        with mock.patch.object(downloads.urllib.request, 'urlopen', return_value=io.BytesIO(b'wrong-file')) as opener:
            with self.assertRaisesRegex(RuntimeError, 'SHA256 mismatch'):
                package.download_verified_tool(self.url, self.target, self.digest)
        self.assertEqual(opener.call_count, 1)
        self.assertEqual(self.target.read_bytes(), b'previous-verified-archive')
        self.assertEqual(list(self.target.parent.glob('*.part')), [])

    def test_upstream_gateway_failure_uses_same_official_release_asset(self):
        api = 'https://api.github.com/repos/example/tool/releases/assets/123'
        failure = urllib.error.HTTPError(self.url, 504, 'Gateway timeout', {}, None)
        with mock.patch.object(downloads.urllib.request, 'urlopen', side_effect=[failure, io.BytesIO(self.payload)]) as opener:
            package.download_verified_tool(self.url, self.target, self.digest, asset_api_url=api)
        request = opener.call_args_list[1].args[0]
        self.assertTrue(request.full_url.startswith(api + '?stepx_retry='))
        self.assertEqual(request.get_header('Accept'), 'application/octet-stream')
        self.assertEqual(self.target.read_bytes(), self.payload)

    def test_network_failure_has_bounded_retries_and_leaves_no_archive(self):
        with mock.patch.object(downloads.urllib.request, 'urlopen', side_effect=urllib.error.URLError('offline')) as opener:
            with self.assertRaises(urllib.error.URLError):
                package.download_verified_tool(self.url, self.target, self.digest)
        self.assertEqual(opener.call_count, 3)
        self.assertFalse(self.target.exists())
        self.assertEqual(list(self.target.parent.glob('*.part')), [])

    def test_permanent_http_failure_is_not_retried(self):
        failure = urllib.error.HTTPError(self.url, 404, 'Not found', {}, None)
        with mock.patch.object(downloads.urllib.request, 'urlopen', side_effect=failure) as opener:
            with self.assertRaises(urllib.error.HTTPError):
                package.download_verified_tool(self.url, self.target, self.digest)
        self.assertEqual(opener.call_count, 1)
        self.assertFalse(self.target.exists())

    def test_github_network_failure_falls_back_to_mirrors_and_verifies_digest(self):
        api = 'https://api.github.com/repos/example/tool/releases/assets/123'
        effects = [
            urllib.error.URLError('github timeout'),
            TimeoutError('api timeout'),
            urllib.error.URLError('first mirror reset'),
            io.BytesIO(self.payload),
        ]
        with mock.patch.object(downloads.urllib.request, 'urlopen', side_effect=effects) as opener:
            package.download_verified_tool(self.url, self.target, self.digest, asset_api_url=api)
        urls = [call.args[0].full_url for call in opener.call_args_list]
        self.assertEqual(urls[0], self.url)
        self.assertTrue(urls[1].startswith(api + '?stepx_retry='))
        self.assertEqual(urls[2], 'https://ghfast.top/' + self.url)
        self.assertEqual(urls[3], 'https://gh-proxy.com/' + self.url)
        self.assertEqual(self.target.read_bytes(), self.payload)

        self.target.write_bytes(b'previous-verified-archive')
        bad_effects = [
            TimeoutError('github timeout'),
            io.BytesIO(b'corrupt-mirror-payload'),
            io.BytesIO(b'second-corrupt-mirror-payload'),
        ]
        with mock.patch.object(downloads.urllib.request, 'urlopen', side_effect=bad_effects) as opener:
            with self.assertRaisesRegex(RuntimeError, 'SHA256 mismatch'):
                package.download_verified_tool(self.url, self.target, self.digest)
        self.assertEqual(opener.call_count, 3)
        self.assertEqual(self.target.read_bytes(), b'previous-verified-archive')
        self.assertEqual(list(self.target.parent.glob('*.part')), [])

    def test_large_slow_download_exceeding_120s_succeeds_with_steady_liveness(self):
        chunks = [b'x' * (64 * 1024) for _ in range(10)] + [b'']
        expected = hashlib.sha256(b''.join(chunks)).hexdigest()
        clock = [0.0]

        class SlowSteadyStream:
            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

            def read1(self, _size):
                chunk = chunks.pop(0)
                # 64 KB every 6s => 128 KB per 12s (>= 128 KB / 15s), spanning > 120s total
                clock[0] += 30.0 if not chunk else 6.0
                return chunk

        # Override total elapsed to > 240s via 42 chunks of 64 KB at 6s each (= 252s total)
        chunks[:] = [b'x' * (64 * 1024) for _ in range(42)] + [b'']
        expected = hashlib.sha256(b''.join(chunks)).hexdigest()
        with mock.patch.object(downloads.time, 'monotonic', side_effect=lambda: clock[0]), \
             mock.patch.object(downloads.urllib.request, 'urlopen', return_value=SlowSteadyStream()):
            package.download_verified_tool(self.url, self.target, expected)
        self.assertGreater(clock[0], 240.0)
        self.assertEqual(hashlib.sha256(self.target.read_bytes()).hexdigest(), expected)

    def test_stalled_reader_without_window_progress_times_out_and_cleans_partial(self):
        clock = [0.0]

        class StalledStream:
            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

            def read1(self, _size):
                clock[0] += 65.0
                return b'tiny'

        with mock.patch.object(downloads.time, 'monotonic', side_effect=lambda: clock[0]), \
             mock.patch.object(downloads.urllib.request, 'urlopen', side_effect=lambda *a, **kw: StalledStream()) as opener:
            with self.assertRaises(TimeoutError):
                package.download_verified_tool(self.url, self.target, self.digest)
        self.assertEqual(opener.call_count, 3)
        self.assertFalse(self.target.exists())
        self.assertEqual(list(self.target.parent.glob('*.part')), [])

    def test_github_token_attached_as_unredirected_header_only_for_api_github_com(self):
        api = 'https://api.github.com/repos/example/tool/releases/assets/123'
        effects = [
            urllib.error.URLError('github timeout'),
            urllib.error.URLError('api timeout'),
            io.BytesIO(self.payload),
        ]
        with mock.patch.dict(os.environ, {'GH_TOKEN': 'test-token-value'}, clear=False), \
             mock.patch.object(downloads.urllib.request, 'urlopen', side_effect=effects) as opener:
            package.download_verified_tool(self.url, self.target, self.digest, asset_api_url=api)
        req_github = opener.call_args_list[0].args[0]
        req_api = opener.call_args_list[1].args[0]
        req_mirror = opener.call_args_list[2].args[0]
        self.assertNotIn('Authorization', req_github.headers)
        self.assertNotIn('Authorization', req_github.unredirected_hdrs)
        self.assertNotIn('Authorization', req_api.headers)
        self.assertEqual(req_api.unredirected_hdrs.get('Authorization'), 'Bearer test-token-value')
        self.assertNotIn('Authorization', req_mirror.headers)
        self.assertNotIn('Authorization', req_mirror.unredirected_hdrs)


class PublicationBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'source'
        self.root.mkdir()
        self.policy = {'schema': 1, 'runtime_files': ['scripts/main.py'],
                       'reviewed_media_sha256': {}, 'public_scripts': [], 'public_docs': []}
        self.write('scripts/main.py', 'print("safe")\n')
        self.write('desktop/Cargo.toml', '[package]\nversion="1.2.3"\n')
        self.write('handoff/google-play-stepx/target.example.json', '{"serial":"","sdk":36}')
        self.save_policy()

    def write(self, rel, data):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data.encode() if isinstance(data, str) else data)

    def save_policy(self):
        self.write(policy.MANIFEST, json.dumps(self.policy))

    def test_private_paths_and_traversal_rejected(self):
        for rel in ['../escape.py', '/abs.py', 'docs/review-assets/capture.png',
                    'handoff/evidence/result.json', 'scripts/model.json', 'keys/release.p12',
                    'data/app.db-wal', 'scripts/.env', 'scripts/cache/data.json']:
            with self.subTest(rel=rel):
                self.assertTrue(policy.path_problems(rel))

    def test_secrets_are_reported_without_values(self):
        secret = 'gh' + 'p_' + 'Q' * 30
        findings = policy.content_problems('scripts/main.py', secret.encode(), self.policy)
        self.assertTrue(findings)
        self.assertNotIn(secret, str(findings))

    def test_media_changes_require_new_review(self):
        sample = b'approved-media-content'
        self.policy['reviewed_media_sha256'][hashlib.sha256(sample).hexdigest()] = {}
        self.assertEqual([], policy.content_problems('desktop/assets/guide.png', sample, self.policy))
        self.assertTrue(policy.content_problems('desktop/assets/guide.png', sample + b'changed', self.policy))

    def test_generic_target_never_copies_real_target(self):
        self.write('handoff/google-play-stepx/target.json', '{"serial":"PRIVATE-DEVICE"}')
        output = policy.runtime_plan(self.root)['handoff/google-play-stepx/target.json']
        self.assertEqual('', json.loads(output)['serial'])
        self.assertIn('PRIVATE-DEVICE', (self.root / 'handoff/google-play-stepx/target.json').read_text())

    def test_validation_precedes_writes(self):
        self.write('scripts/main.py', 'sk' + '-' + 'Z' * 30)
        destination = Path(self.temp.name) / 'output'
        with self.assertRaises(ValueError):
            stage(self.root, destination)
        self.assertFalse(destination.exists())

    def test_stage_keeps_user_data_and_only_installs_current_release(self):
        self.policy['runtime_files'] += ['desktop/assets/release-notes/1.2.2.json',
                                        'desktop/assets/release-notes/1.2.3.json']
        self.write('desktop/assets/release-notes/1.2.2.json', '{}')
        self.write('desktop/assets/release-notes/1.2.3.json', '{}')
        self.save_policy()
        destination = Path(self.temp.name) / 'output'
        destination.mkdir()
        (destination / 'user.db').write_bytes(b'user-state')
        stage(self.root, destination)
        self.assertEqual(b'user-state', (destination / 'user.db').read_bytes())
        self.assertFalse((destination / 'desktop/assets/release-notes/1.2.2.json').exists())
        self.assertTrue((destination / 'desktop/assets/release-notes/1.2.3.json').exists())

    def test_crlf_checkout_stages_identical_python_bytes_and_hashes_without_rewriting_other_inputs(self):
        source = '# coding: utf-8\nprint("你好")\n'.encode('utf-8')
        windows_source = source.replace(b'\n', b'\r\n')
        # Exercise the actual staging entry point, not only the normalizer.
        # Even an explicitly allowlisted vendor .py must remain byte-for-byte.
        untouched = {
            'desktop/assets/runtime.bin': b'\x00\xffbinary\r\n\rpayload',
            'runtime/python/Lib/vendor.py': b'# vendor\r\nVALUE = 42\r\n',
            'tools/vendor.py': b'# external tool\r\nVALUE = 7\r\n',
        }
        self.policy['runtime_files'] += [policy.MANIFEST, *untouched]
        self.save_policy()
        self.write('scripts/main.py', windows_source)
        for rel, data in untouched.items():
            self.write(rel, data)
        destination = Path(self.temp.name) / 'windows-stage'
        stage(self.root, destination)
        self.assertEqual(source, (destination / 'scripts/main.py').read_bytes())
        self.assertEqual(windows_source, (self.root / 'scripts/main.py').read_bytes(),
                         'packaging must not mutate the developer checkout')
        digests = json.loads((destination / 'release-resources.sha256.json').read_bytes())
        self.assertEqual(hashlib.sha256(source).hexdigest(), digests['scripts/main.py'])
        for rel, data in untouched.items():
            self.assertEqual(data, (destination / rel).read_bytes())
            self.assertEqual(hashlib.sha256(data).hexdigest(), digests[rel])
        for rel, digest in digests.items():
            self.assertEqual(hashlib.sha256((destination / rel).read_bytes()).hexdigest(), digest)
        self.assertEqual([], policy.audit_bundle(destination))

        self.write('scripts/main.py', source)
        reference = Path(self.temp.name) / 'lf-stage'
        stage(self.root, reference)
        self.assertEqual(digests, json.loads((reference / 'release-resources.sha256.json').read_bytes()),
                         'the same source checkout must produce the same resource digests on every OS')

    def test_noncanonical_packaged_python_is_rejected_even_with_a_matching_resource_digest(self):
        self.policy['runtime_files'].append(policy.MANIFEST)
        self.save_policy()
        destination = Path(self.temp.name) / 'output'
        stage(self.root, destination)
        for broken in [b'print("safe")\r\n', b'print("safe")\r', b'# invalid UTF-8 \xff\n']:
            with self.subTest(data=broken):
                (destination / 'scripts/main.py').write_bytes(broken)
                hashes = json.loads((destination / 'release-resources.sha256.json').read_bytes())
                hashes['scripts/main.py'] = hashlib.sha256(broken).hexdigest()
                (destination / 'release-resources.sha256.json').write_text(json.dumps(hashes))
                issues = policy.audit_bundle(destination)
                self.assertTrue(any('scripts/main.py: first-party Python source' in issue for issue in issues))

    def test_invalid_first_party_python_fails_before_staging_any_files(self):
        for index, broken in enumerate([b'print("safe")\r', b'# invalid UTF-8 \xff\n']):
            with self.subTest(data=broken):
                self.write('scripts/main.py', broken)
                destination = Path(self.temp.name) / f'invalid-stage-{index}'
                with self.assertRaises(ValueError):
                    stage(self.root, destination)
                self.assertFalse(destination.exists())

    def test_source_symlink_rejected(self):
        (self.root / 'scripts/main.py').unlink()
        external = Path(self.temp.name) / 'external.py'
        external.write_text('private')
        (self.root / 'scripts/main.py').symlink_to(external)
        with self.assertRaises(ValueError):
            policy.runtime_plan(self.root)

    def test_system_alias_above_source_root_is_allowed(self):
        parent = Path(self.temp.name)
        physical = parent / 'physical'
        nested = physical / 'nested'
        nested.mkdir(parents=True)
        self.root.rename(nested / 'source')
        alias = parent / 'system-alias'
        alias.symlink_to(physical, target_is_directory=True)
        source = alias / 'nested' / 'source'
        self.assertEqual(policy.safe_read(source, 'scripts/main.py'), b'print("safe")\n')
        symlink = source / 'scripts/alias.py'
        symlink.symlink_to(source / 'scripts/main.py')
        with self.assertRaisesRegex(ValueError, 'symlink'):
            policy.safe_read(source, 'scripts/alias.py')

    def test_unlisted_installed_file_is_rejected(self):
        self.policy['runtime_files'].append(policy.MANIFEST)
        self.save_policy()
        destination = Path(self.temp.name) / 'output'
        stage(self.root, destination)
        (destination / 'scripts/unexpected.py').write_text('print("not reviewed")')
        self.assertTrue(any('unexpected.py' in issue for issue in policy.audit_bundle(destination)))

    def test_fixture_replacement_preserves_identity_relationships(self):
        original = 'ZYPZ' + '261A' + '000000001'
        replaced = export.fixture_placeholders('desktop/src/test.rs', original + ' ' + original)
        first, second = replaced.split()
        self.assertEqual(first, second)
        self.assertNotIn(original, replaced)

    def test_cross_platform_home_input_and_expected_output_remain_consistent(self):
        for family in ['home', 'Users']:
            bare = '/' + family + '/private-developer'
            text = '"' + bare + '" "' + bare + '/Library/data"'
            expected = '"/' + family + '/stepx-user" "/' + family + '/stepx-user/Library/data"'
            self.assertEqual(expected, export.fixture_placeholders('test.rs', text))
        fixture = '"/Users/macuser" "/Users/macuser/Library/data"'
        self.assertEqual(fixture, export.fixture_placeholders('test.rs', fixture))

    def test_public_phone_fixture_preserves_normalization_and_is_idempotent(self):
        original = '133' + '2298' + '8887'
        formats = [original, '+86' + original, '0086' + original,
                   '86' + original, '+86 ' + original[:3] + '-' + original[3:7] + '-' + original[7:]]
        clean = export.fixture_placeholders('desktop/src/normalization.rs', ' '.join('"' + v + '"' for v in formats))
        self.assertNotIn(original, clean)
        local = clean.split('"')[1]
        self.assertTrue(local.startswith('1380000'))
        self.assertIn('+86' + local, clean)
        self.assertIn('0086' + local, clean)
        self.assertIn('+86 ' + local[:3] + '-' + local[3:7] + '-' + local[7:], clean)
        self.assertEqual(clean, export.fixture_placeholders('desktop/src/normalization.rs', clean))

    def test_public_recipient_fixtures_stay_distinct(self):
        originals = ['139' + '1024' + '6688', '186' + '2100' + '5520']
        clean = [export.fixture_placeholders('desktop/src/test.rs', v) for v in originals]
        self.assertEqual(len(set(clean)), len(originals))
        self.assertFalse(set(clean) & set(originals))

    def test_existing_public_inputs_receive_full_policy_before_writes(self):
        public = Path(self.temp.name) / 'public'
        public.mkdir()
        subprocess.run(['git', 'init', '-q', str(public)], check=True)
        (public / 'old.db').write_bytes(b'private runtime database')
        (public / 'old.png').write_bytes(b'unreviewed screenshot')
        with mock.patch.object(export, 'ROOT', self.root), mock.patch.object(export, 'PUBLIC_ROOT', public):
            findings = export.scan_public_repo({})
        self.assertTrue(any('old.db' in f for f in findings))
        self.assertTrue(any('old.png' in f for f in findings))

    def test_tracked_ignored_private_files_are_not_silently_skipped(self):
        public = Path(self.temp.name) / 'public'
        public.mkdir()
        subprocess.run(['git', 'init', '-q', str(public)], check=True)
        (public / 'state').mkdir()
        (public / 'state/device.json').write_text('{}')
        subprocess.run(['git', '-C', str(public), 'add', 'state/device.json'], check=True)
        (public / '.gitignore').write_text('state/\n')
        with mock.patch.object(export, 'ROOT', self.root), mock.patch.object(export, 'PUBLIC_ROOT', public):
            findings = export.scan_public_repo({})
        self.assertTrue(any('state/device.json' in f for f in findings))

    def test_retired_media_removed_only_from_public_copy_after_preflight(self):
        rel = 'docs/videos/obsolete.mp4'
        self.policy['public_removed_files'] = [rel]
        self.save_policy()
        self.write(rel, b'local recording retained')
        public = Path(self.temp.name) / 'public'
        (public / rel).parent.mkdir(parents=True)
        (public / rel).write_bytes(b'old public output')
        with mock.patch.object(export, 'ROOT', self.root), mock.patch.object(export, 'PUBLIC_ROOT', public):
            export.execute_sync({}, check_only=True)
            self.assertTrue((public / rel).exists())
            export.execute_sync({}, check_only=False)
        self.assertFalse((public / rel).exists())
        self.assertEqual(b'local recording retained', (self.root / rel).read_bytes())

    def test_retired_media_cannot_escape_public_videos(self):
        for rel in ['../private', 'state/user.db', 'desktop/src/main.rs']:
            self.policy['public_removed_files'] = [rel]
            self.save_policy()
            with mock.patch.object(export, 'ROOT', self.root), self.assertRaises(ValueError):
                export.publication_removals({})

    def test_runtime_verification_cannot_write_bytecode_to_stage_or_inherited_cache(self):
        # Exercise the actual child-process launcher, not just its argv. The
        # verifier imports a new module then starts another Python process;
        # both must preserve the candidate package byte-for-byte.
        fake_root = Path(self.temp.name) / 'packaging'
        scripts = fake_root / 'scripts'
        scripts.mkdir(parents=True)
        candidate = Path(self.temp.name) / 'candidate'
        candidate.mkdir()
        (candidate / 'probe.py').write_text('VALUE = 42\n')
        (candidate / 'child.py').write_text('VALUE = 7\n')
        (scripts / 'verify-packaged-runtime.py').write_text(
            'import sys, subprocess\n'
            'sys.path.insert(0, sys.argv[1])\n'
            'import probe\n'
            'assert probe.VALUE == 42 and sys.dont_write_bytecode\n'
            'subprocess.run([sys.executable, "-c", '
            '"import child; assert child.VALUE == 7"], cwd=sys.argv[1], check=True)\n')
        before = {p.relative_to(candidate): p.read_bytes() for p in candidate.rglob('*') if p.is_file()}
        inherited = Path(self.temp.name) / 'external-cache'
        with mock.patch.object(package, 'ROOT', fake_root), mock.patch.dict(
                os.environ, {'PYTHONDONTWRITEBYTECODE': '', 'PYTHONPYCACHEPREFIX': str(inherited)}):
            package.verify_runtime(Path(sys.executable), candidate)
        after = {p.relative_to(candidate): p.read_bytes() for p in candidate.rglob('*') if p.is_file()}
        self.assertEqual(before, after)
        self.assertFalse(inherited.exists())

    def test_packaged_documents_keep_local_files_and_pin_omitted_resources(self):
        source = ('[安装](GETTING_STARTED.md#usb)\n'
                  '[视频](videos/产品介绍.mp4 "演示")\n'
                  '![截图](images/home.png)\n'
                  '[指南](<../desktop/neo-agent-adapter/README.md#使用>)\n'
                  '[索引][guide]\n[guide]: STORAGE_EXPANSION.md#内容\n'
                  '[远程](https://example.com/docs) [本页](#目的)\n'
                  '```md\n[示例](examples/local.md)\n```\n')
        result = package.package_document('docs/RELEASE_NOTES.md', source.encode(), '1.2.3',
                                          {'docs/GETTING_STARTED.md'}).decode()
        self.assertIn('[安装](GETTING_STARTED.md#usb)', result)
        self.assertIn('releases/download/v1.2.3/%E4%BA%A7%E5%93%81%E4%BB%8B%E7%BB%8D.mp4 "演示"', result)
        self.assertIn('![截图](https://raw.githubusercontent.com/xydzf520/stepx/v1.2.3/docs/images/home.png)', result)
        self.assertIn('<https://github.com/xydzf520/stepx/blob/v1.2.3/desktop/neo-agent-adapter/README.md#使用>', result)
        self.assertIn('[guide]: https://github.com/xydzf520/stepx/blob/v1.2.3/docs/STORAGE_EXPANSION.md#内容', result)
        self.assertIn('[远程](https://example.com/docs) [本页](#目的)', result)
        self.assertIn('```md\n[示例](examples/local.md)\n```', result)

    def test_packaged_document_cannot_link_outside_repository(self):
        with self.assertRaisesRegex(ValueError, 'leaves the source tree'):
            package.package_document('README.md', b'[private](../private.txt)', '1.2.3', set())

    def test_maintenance_package_keeps_video_links_on_the_recorded_release(self):
        source = ('[产品](docs/videos/StepX-product-0.2.180.mp4)\n'
                  '[更新](docs/videos/StepX-0.2.60-to-0.2.180.mp4)\n')
        result = package.package_document('README.md', source.encode(), '0.2.181', set()).decode()
        self.assertIn('releases/download/v0.2.180/StepX-product-0.2.180.mp4', result)
        self.assertIn('releases/download/v0.2.180/StepX-0.2.60-to-0.2.180.mp4', result)
        self.assertNotIn('v0.2.181', result)

    def test_maintenance_release_stages_apk_without_fabricating_new_videos(self):
        self.write('desktop/assets/stepx-companion.apk', b'signed-apk-fixture')
        dist = Path(self.temp.name)/'dist'
        names = package.stage_release_media(self.root, dist, '0.2.181')
        self.assertEqual(names, ['StepX-companion.apk'])
        self.assertEqual((dist/'StepX-companion.apk').read_bytes(), b'signed-apk-fixture')
        self.assertEqual((dist/'media-sha256.txt').read_text().splitlines(),
                         [hashlib.sha256(b'signed-apk-fixture').hexdigest()+'  StepX-companion.apk'])

    def test_release_video_pair_is_atomic_and_each_file_must_be_nonempty(self):
        self.write('desktop/assets/stepx-companion.apk', b'signed-apk-fixture')
        product = 'docs/videos/StepX-product-0.2.181.mp4'
        update = 'docs/videos/StepX-0.2.60-to-0.2.181.mp4'
        self.write(product, b'product-video-fixture')
        dist = Path(self.temp.name)/'dist'
        with self.assertRaisesRegex(ValueError, 'both product and update'):
            package.stage_release_media(self.root, dist, '0.2.181')
        self.assertFalse(dist.exists())
        self.write(update, b'')
        with self.assertRaisesRegex(ValueError, 'empty'):
            package.stage_release_media(self.root, dist, '0.2.181')
        self.assertFalse(dist.exists())
        self.write(update, b'update-video-fixture')
        names = package.stage_release_media(self.root, dist, '0.2.181')
        self.assertEqual(len(names), 3)
        self.assertEqual(len((dist/'media-sha256.txt').read_text().splitlines()), 3)

    def test_macos_packaging_strips_extended_attributes_before_deep_codesign(self):
        source = (SCRIPTS / 'package-release.py').read_text(encoding='utf-8')
        xattr_idx = source.find("['xattr','-cr',str(base/'StepX Companion.app')]")
        sign_idx = source.find("['codesign','--force','--deep','--sign','-',str(base/'StepX Companion.app')]")
        verify_idx = source.find("['codesign','--verify','--deep','--strict',str(base/'StepX Companion.app')]")
        self.assertNotEqual(xattr_idx, -1, 'macOS packaging must strip extended attributes with xattr -cr')
        self.assertNotEqual(sign_idx, -1, 'macOS packaging must deep ad-hoc codesign the app bundle')
        self.assertNotEqual(verify_idx, -1, 'macOS packaging must strictly verify the deep signature')
        self.assertLess(xattr_idx, sign_idx, 'xattr -cr must run before codesign --force --deep')
        self.assertLess(sign_idx, verify_idx, 'codesign --force --deep must run before codesign --verify')


def _macho64_with_build_version(major, minor, patch=0, *, use_version_min=False):
    raw_ver = (major << 16) | (minor << 8) | patch
    if use_version_min:
        cmd = struct.pack('<IIII', 0x24, 16, raw_ver, raw_ver)
    else:
        cmd = struct.pack('<IIIIII', 0x32, 24, 1, raw_ver, raw_ver, 0)
    header = struct.pack('<IIIIIIII', 0xfeedfacf, 0x0100000c, 0, 2, 1, len(cmd), 0, 0)
    return header + cmd


class ReleasePackagingHardeningTests(unittest.TestCase):
    def test_root_facloud_licenses_and_manifest_entries_exist_in_main_repo(self):
        repo_root = SCRIPTS.parent
        mf = policy.manifest(repo_root)
        expected_licenses = [
            'licenses/FaCloud-LICENSE-AGPL',
            'licenses/FaCloud-LICENSE-MIT',
            'licenses/FaCloud-LICENSES.md',
            'licenses/FaCloud-NOTICE',
        ]
        for rel in expected_licenses:
            self.assertIn(rel, mf['package_documents'])
            self.assertIn(rel, mf['public_files'])
            self.assertTrue((repo_root / rel).is_file(), f'Missing root {rel}')
        self.assertIn('scripts/test-metal-glyph-fade.py', mf['public_scripts'])
        for rel in mf['package_documents']:
            data = policy.safe_read(repo_root, rel)
            self.assertGreater(len(data), 0, rel)

    def test_macho_binaries_distinguishes_fat_macho_from_java_class_and_sorts_deepest_first(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            shallow = root / 'tools/bin/adb'
            deep = root / 'runtime/python/lib/python3.13/lib-dynload/_ssl.so'
            fat = root / 'runtime/java/lib/server/libjvm.dylib'
            java_cls = root / 'runtime/classes/ApkIdentity.class'
            java_preview_cls = root / 'runtime/classes/Preview.class'
            bad_fat = root / 'runtime/classes/FakeFat.bin'
            for path in (shallow, deep, fat, java_cls, java_preview_cls, bad_fat):
                path.parent.mkdir(parents=True, exist_ok=True)
            shallow.write_bytes(b'\xcf\xfa\xed\xfe\x07\x00\x00\x01' + b'\x00' * 24)
            deep.write_bytes(b'\xfe\xed\xfa\xcf\x01\x00\x00\x0c' + b'\x00' * 24)
            fat.write_bytes(b'\xca\xfe\xba\xbe\x00\x00\x00\x02' + b'\x00' * 40)
            # Java 11 class file: magic 0xCAFEBABE, minor=0, major=55 (0x0037 > 20)
            java_cls.write_bytes(b'\xca\xfe\xba\xbe\x00\x00\x00\x37' + b'\x00' * 16)
            # Java preview class file: magic 0xCAFEBABE, minor=65535, major=65
            java_preview_cls.write_bytes(b'\xca\xfe\xba\xbe\xff\xff\x00\x41' + b'\x00' * 16)
            # Zero-arch header is not a valid Fat Mach-O
            bad_fat.write_bytes(b'\xca\xfe\xba\xbe\x00\x00\x00\x00' + b'\x00' * 16)
            link = root / 'tools/bin/adb-link'
            link.symlink_to(shallow)

            found = package.macho_binaries(root)
            self.assertEqual(found, [deep, fat, shallow])

    def test_verify_macos_minos_accepts_11_0_and_rejects_higher_deployment_targets(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            ok_bin = root / 'stepx-11'
            ok_bin.write_bytes(_macho64_with_build_version(11, 0))
            self.assertEqual(package.verify_macos_minos(ok_bin), [(11, 0, 0)])

            legacy_bin = root / 'stepx-10-15'
            legacy_bin.write_bytes(_macho64_with_build_version(10, 15, 7, use_version_min=True))
            self.assertEqual(package.verify_macos_minos(legacy_bin), [(10, 15, 7)])

            bad_bin = root / 'stepx-15'
            bad_bin.write_bytes(_macho64_with_build_version(15, 0))
            with self.assertRaisesRegex(RuntimeError, 'macos minos 15.0 exceeds 11.0|macOS minos 15.0 exceeds 11.0'):
                package.verify_macos_minos(bad_bin)

            # Universal binary with one 11.0 slice and one 14.0 slice must fail closed.
            s1 = _macho64_with_build_version(11, 0)
            s2 = _macho64_with_build_version(14, 0)
            off1 = 8 + 2 * 20
            off2 = off1 + len(s1)
            fat_hdr = struct.pack('>II', 0xcafebabe, 2)
            arch1 = struct.pack('>iiIII', 0x01000007, 3, off1, len(s1), 0)
            arch2 = struct.pack('>iiIII', 0x0100000c, 0, off2, len(s2), 0)
            fat_bin = root / 'stepx-universal'
            fat_bin.write_bytes(fat_hdr + arch1 + arch2 + s1 + s2)
            with self.assertRaisesRegex(RuntimeError, '14.0 exceeds 11.0'):
                package.verify_macos_minos(fat_bin)

    def test_macos_info_plist_declares_gpu_switching_and_all_bonjour_services(self):
        info = package.macos_info_plist('0.2.183')
        self.assertEqual(info['CFBundleInfoDictionaryVersion'], '6.0')
        self.assertTrue(info['NSSupportsAutomaticGraphicsSwitching'])
        self.assertEqual(info['LSMinimumSystemVersion'], '11.0')
        self.assertEqual(info['CFBundleIdentifier'], 'io.github.xydzf520.stepx')
        self.assertEqual(
            set(info['NSBonjourServices']),
            {'_adb-tls-pairing._tcp', '_adb-tls-connect._tcp', '_adb._tcp',
             '_airplay._tcp', '_googlecast._tcp', '_raop._tcp'},
        )

    def test_sign_macos_bundle_signs_readonly_nested_machos_before_deep_sign(self):
        with tempfile.TemporaryDirectory() as td:
            base = Path(td)
            app = base / 'StepX Companion.app'
            binary = app / 'Contents/MacOS/stepx-companion'
            resources = app / 'Contents/Resources'
            tools = resources / 'tools/bin'
            py_so = resources / 'runtime/python/lib/python3.13/lib-dynload/_ctypes.so'
            java_bin = resources / 'runtime/java/bin/java'
            scrcpy_bin = tools / 'scrcpy'
            java_cls = resources / 'runtime/android-build-tools/ApkIdentity.class'
            for path in (binary, py_so, java_bin, scrcpy_bin, java_cls):
                path.parent.mkdir(parents=True, exist_ok=True)
            macho_bytes = _macho64_with_build_version(11, 0)
            for path in (binary, py_so, java_bin, scrcpy_bin):
                path.write_bytes(macho_bytes)
                path.chmod(0o555)
            java_cls.write_bytes(b'\xca\xfe\xba\xbe\x00\x00\x00\x37' + b'\x00' * 16)

            calls = []
            with mock.patch.object(package.subprocess, 'run', side_effect=lambda cmd, **kw: calls.append(cmd)):
                signed = package.sign_macos_bundle(base, binary, resources, tools, {'PYTHONDONTWRITEBYTECODE': '1'})

            self.addCleanup(lambda: [p.chmod(0o755) for p in (binary, py_so, java_bin, scrcpy_bin) if p.exists()])
            self.assertEqual(signed, [py_so, java_bin, scrcpy_bin])
            for p in (binary, py_so, java_bin, scrcpy_bin):
                self.assertTrue(p.stat().st_mode & stat.S_IWUSR)
            self.assertEqual(calls[0], ['xattr', '-cr', str(app)])
            self.assertEqual(calls[1:4], [['codesign', '--force', '--sign', '-', str(p)] for p in [py_so, java_bin, scrcpy_bin]])
            self.assertEqual(calls[4], ['codesign', '--force', '--deep', '--sign', '-', str(app)])
            self.assertEqual(calls[5], [str(binary), '--check-runtime'])
            self.assertEqual(calls[6], [str(scrcpy_bin), '--version'])
            self.assertEqual(calls[7:10], [['codesign', '--verify', '--strict', str(p)] for p in [py_so, java_bin, scrcpy_bin]])
            self.assertEqual(calls[10], ['codesign', '--verify', '--deep', '--strict', str(app)])

    def test_clean_tar_info_strips_host_uid_gid_and_usernames(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            src = root / 'pkg'
            src.mkdir()
            (src / 'stepx-companion').write_bytes(b'binary')
            archive = root / 'out.tar.gz'
            with tarfile.open(archive, 'w:gz') as tf:
                tf.add(src, arcname='StepX-Companion', filter=package.clean_tar_info)
            with tarfile.open(archive, 'r:gz') as tf:
                members = tf.getmembers()
            self.assertTrue(members)
            for member in members:
                self.assertEqual(member.uid, 0)
                self.assertEqual(member.gid, 0)
                self.assertEqual(member.uname, '')
                self.assertEqual(member.gname, '')

    def test_windows_quiet_popen_skips_create_no_window_for_detached_and_new_console(self):
        recorded = []

        class FakePopen:
            def __init__(self, *args, **kwargs):
                recorded.append(kwargs)

        fake_subproc = types.SimpleNamespace(
            Popen=FakePopen,
            CREATE_NO_WINDOW=0x08000000,
            DETACHED_PROCESS=0x00000008,
            CREATE_NEW_CONSOLE=0x00000010,
            CREATE_NEW_PROCESS_GROUP=0x00000200,
        )
        fake_sys = types.SimpleNamespace(dont_write_bytecode=False)
        ns = {}
        with mock.patch.dict(sys.modules, {'subprocess': fake_subproc, 'sys': fake_sys}):
            exec(runtime_pkg.SITE_CUSTOMIZE, ns)
        self.assertTrue(fake_sys.dont_write_bytecode)
        fake_subproc.Popen(['cmd'])
        fake_subproc.Popen(['cmd'], creationflags=fake_subproc.CREATE_NEW_PROCESS_GROUP)
        fake_subproc.Popen(['cmd'], creationflags=fake_subproc.DETACHED_PROCESS)
        fake_subproc.Popen(['cmd'], creationflags=fake_subproc.CREATE_NEW_CONSOLE)
        self.assertEqual(recorded[0]['creationflags'], fake_subproc.CREATE_NO_WINDOW)
        self.assertEqual(recorded[1]['creationflags'], fake_subproc.CREATE_NEW_PROCESS_GROUP | fake_subproc.CREATE_NO_WINDOW)
        self.assertEqual(recorded[2]['creationflags'], fake_subproc.DETACHED_PROCESS)
        self.assertEqual(recorded[3]['creationflags'], fake_subproc.CREATE_NEW_CONSOLE)

    def test_windows_mt_numeric_sorting_and_npmmirror_fallback(self):
        paths = [
            Path('/kits/10.0.19041.0/x64/mt.exe'),
            Path('/kits/10.0.9999.0/x64/mt.exe'),
            Path('/kits/10.0.26100.0/x64/mt.exe'),
        ]
        self.assertEqual(runtime_pkg.sort_mt_candidates(paths)[0], Path('/kits/10.0.26100.0/x64/mt.exe'))

        with tempfile.TemporaryDirectory() as td:
            archive = Path(td) / 'python.zip'
            seen = []

            def fake_download(url, dest, sha):
                seen.append((url, sha))
                if 'python.org' in url:
                    raise TimeoutError('python.org timeout')
                Path(dest).write_bytes(b'verified-embed-zip')

            with mock.patch.object(runtime_pkg, 'download_verified_tool', side_effect=fake_download):
                runtime_pkg.download_windows_python(archive)
            self.assertEqual(seen, [
                (runtime_pkg.PYTHON_URL, runtime_pkg.WIN_PY_SHA256),
                (runtime_pkg.PYTHON_FALLBACK_URL, runtime_pkg.WIN_PY_SHA256),
            ])
            self.assertEqual(archive.read_bytes(), b'verified-embed-zip')

    def test_strip_pe_authenticode_removes_certificates_in_reverse_order(self):
        removed = []
        closed = []

        class FakeKernel32:
            class CreateFileW:
                argtypes = None
                restype = None
                def __call__(self, *args):
                    return 42

            class CloseHandle:
                argtypes = None
                restype = None
                def __call__(self, handle):
                    closed.append(handle)
                    return 1

        FakeKernel32.CreateFileW = FakeKernel32.CreateFileW()
        FakeKernel32.CloseHandle = FakeKernel32.CloseHandle()

        class FakeImagehlp:
            class ImageEnumerateCertificates:
                argtypes = None
                restype = None
                def __call__(self, handle, type_filter, count_ptr, indices, index_count):
                    count_ptr._obj.value = 2
                    return 1

            class ImageRemoveCertificate:
                argtypes = None
                restype = None
                def __call__(self, handle, index):
                    removed.append((handle, index))
                    return 1

        FakeImagehlp.ImageEnumerateCertificates = FakeImagehlp.ImageEnumerateCertificates()
        FakeImagehlp.ImageRemoveCertificate = FakeImagehlp.ImageRemoveCertificate()

        import ctypes
        import ctypes.wintypes
        fake_wintypes = types.SimpleNamespace(
            LPCWSTR=ctypes.c_wchar_p,
            DWORD=ctypes.c_uint32,
            WORD=ctypes.c_uint16,
            HANDLE=ctypes.c_void_p,
            BOOL=ctypes.c_int,
        )
        with mock.patch.object(ctypes, 'WinDLL', side_effect=lambda name, **kw: FakeKernel32 if name == 'kernel32' else FakeImagehlp, create=True), \
             mock.patch.object(ctypes, 'get_last_error', return_value=0, create=True), \
             mock.patch.dict(sys.modules, {'ctypes.wintypes': fake_wintypes}):
            count = runtime_pkg.strip_pe_authenticode(Path('java.exe'))
        self.assertEqual(count, 2)
        self.assertEqual(removed, [(42, 1), (42, 0)])
        self.assertEqual(closed, [42])


if __name__ == '__main__':
    unittest.main()
