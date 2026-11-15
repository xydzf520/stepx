"""Local-only tests of task-host result, identity and emitted cleanup guards."""
import ast
import base64
import gzip
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('native_task_host', ROOT / 'scripts/gui-native-task-host.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def record():
    identifier = 'a' * 32
    value = dict(schema=runner.SCHEMA, id=identifier, serial=runner.recovery.SERIAL,
                 fingerprint=runner.recovery.FINGERPRINT, ability_sha256=runner.recovery.APK_SHA256,
                 main=runner.MAIN, socket=runner.SOCKET, signer='b' * 64, dex_sha256='c' * 64,
                 companion_uid=10215, boot_id='a' * 8 + '-bbbb-cccc-dddd-' + 'e' * 12,
                 backup='/tmp/owned-backup', system_server_pid=2859, ability_pid=7204,
                 code_cache_realpath=runner.REMOTE_DIR,
                 dex_name='stepx-native-gui-task-' + identifier + '.dex',
                 result_name='stepx-native-gui-task-' + identifier + '.result.jsonl')
    value['process'] = dict(pid=24567, start_ticks='91234', boot_id=value['boot_id'],
                            argv=[runner.MAIN], uids=[1000] * 4, gids=[1000] * 4,
                            context='u:r:system_app:s0')
    value['files'] = {key: dict(dev=1, ino=i, uid=1000, gid=1000, mode=mode, nlink=1)
                      for key, i, mode in (('dex', 42, 0o400), ('result', 43, 0o600))}
    return value


def final_result(value):
    return dict(event='finished', pid=value['process']['pid'], host_uid=1000,
                target_package='com.stepos.ability', socket=runner.SOCKET,
                companion_uid=value['companion_uid'], ok=True, cleanup_verified=True,
                cleanup_scope='host_process_resources', global_rollback_verified=False)


class TaskHostGuardTests(unittest.TestCase):
    def test_unreviewed_dex_never_reaches_phone(self):
        with patch.object(runner, 'DEX_SHA256', 'UNREVIEWED'), \
                patch.object(runner, 'permissions_and_adb') as phone:
            with self.assertRaisesRegex(RuntimeError, 'launch is disabled'):
                runner.start(Path('/does/not/exist'), Path('companion.apk'))
            phone.assert_not_called()

    def test_record_rejects_path_injection_and_foreign_identity(self):
        for key, changed in (('dex_name', '../../other'), ('result_name', '/data/system/a'),
                             ('serial', 'other'), ('signer', 'bad'), ('companion_uid', 1000),
                             ('system_server_pid', 0), ('main', 'other.Main'),
                             ('code_cache_realpath', '/data/data/another/package')):
            value = record(); value[key] = changed
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                runner.validate_record(value)

    def test_process_reuse_uid_domain_and_argv_are_rejected(self):
        value = record()
        runner.validate_process(value['process'], value)
        for key, changed in (('uids', [1000, 0, 1000, 1000]), ('context', 'u:r:shell:s0'),
                             ('argv', ['other.Main']), ('boot_id', 'another'), ('start_ticks', 'bad')):
            observed = {**value['process'], key: changed}
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                runner.validate_process(observed, value)
        self.assertFalse(runner.same_process(value['process'], {**value['process'], 'start_ticks': '91235'}))

    def test_canonical_path_can_be_unknown_only_before_worker_launch(self):
        value = record()
        value.pop('process')
        value.update(status='preparing', code_cache_realpath=None)
        runner.validate_record(value)
        worker = runner.worker_prefix(value)
        self.assertIn("out['code_cache_realpath']=canonical", worker)
        value['status'] = 'launch_unverified'
        with self.assertRaisesRegex(RuntimeError, 'directory alias'):
            runner.validate_record(value)

    def test_final_json_filters_sensitive_or_unreviewed_fields(self):
        value = record(); response = final_result(value)
        response.update(token='secret', stdout='private screen', events=[{'raw': 'private'}],
                        storage_root='/private/path', test_action_events=2, test_terminal='finish')
        raw = b'non-JSON diagnostic text\n' + json.dumps(response).encode() + b'\n'
        result = runner.filtered_result(raw, value)
        self.assertNotIn('secret', json.dumps(result))
        self.assertNotIn('private', json.dumps(result))
        self.assertEqual(result['test_action_events'], 2)
        self.assertFalse(result['global_rollback_verified'])

    def test_final_json_requires_exact_owner_and_no_global_rollback_claim(self):
        value = record()
        for key, changed in (('pid', 99), ('host_uid', 0), ('companion_uid', 10001),
                             ('socket', 'other'), ('event', 'ready'), ('global_rollback_verified', True),
                             ('cleanup_verified', 'true'), ('error_type', 'secret: account text')):
            answer = {**final_result(value), key: changed}
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                runner.filtered_result(json.dumps(answer).encode(), value)

    def test_resource_diagnostics_drop_unreviewed_nested_content(self):
        value = record(); response = final_result(value)
        response['mdi_registry_before_aux_cleanup'] = dict(
            ok=True, task_count=1, watcher_count=1, other_count=0,
            tasks=[dict(category='private account')], token='secret')
        response['auxiliary_cleanup'] = dict(
            complete=True, watcher_reset_invoked=True, watcher_display_gone=True,
            watcher_display_id=90000002, raw='private screen')
        result = runner.filtered_result(json.dumps(response).encode(), value)
        self.assertEqual(result['mdi_registry_before_aux_cleanup']['watcher_count'], 1)
        self.assertTrue(result['auxiliary_cleanup']['watcher_display_gone'])
        self.assertNotIn('private', json.dumps(result))
        self.assertNotIn('secret', json.dumps(result))
        response['auxiliary_cleanup']['watcher_error'] = 'private error text'
        with self.assertRaisesRegex(RuntimeError, 'invalid_diagnostic_error'):
            runner.filtered_result(json.dumps(response).encode(), value)

    def test_model_observer_retains_phase_without_payload_or_error_message(self):
        value = record(); response = final_result(value)
        response['model_diagnostics'] = dict(
            installed=True, service_chat_count=1, service_response_count=0,
            events=[dict(stage='SERVICE_CHAT_ASYNC_ERROR', error_class='org.json.JSONException',
                         error_frames=['org.json.JSONTokener.nextValue:1', 'secret text'],
                         message='private response', payload='secret')])
        result = runner.filtered_result(json.dumps(response).encode(), value)
        diagnostic = result['model_diagnostics']
        self.assertEqual(diagnostic['service_chat_count'], 1)
        self.assertEqual(diagnostic['events'][0]['error_frames'], ['org.json.JSONTokener.nextValue:1'])
        self.assertNotIn('private', json.dumps(result))
        self.assertNotIn('secret', json.dumps(result))
        response['model_diagnostics']['events'][0]['stage'] = 'PRIVATE_PAYLOAD'
        with self.assertRaisesRegex(RuntimeError, 'invalid_model_observer_stage'):
            runner.filtered_result(json.dumps(response).encode(), value)

    def test_network_diagnostics_keep_codes_but_never_header_values_or_reason(self):
        value = record(); response = final_result(value)
        response['event_diagnostics'] = dict(installed=True, received_count=1, forwarded_count=1,
            events=[dict(stage='AGENT_CALL_RECEIVED', message_type=4, raw_body_length=40,
                         error_schema_candidate_code=403, reason='private server details',
                         numeric_header_codes={'code':403,'authorization':'secret'},
                         wire_fields=[{'field':1,'wire':0,'value':'private'}])])
        result = runner.filtered_result(json.dumps(response).encode(), value)
        event = result['event_diagnostics']['events'][0]
        self.assertEqual(event['error_schema_candidate_code'], 403)
        self.assertEqual(event['numeric_header_codes'], {'code':403})
        self.assertEqual(event['wire_fields'], [{'field':1,'wire':0}])
        self.assertNotIn('private', json.dumps(result))
        self.assertNotIn('secret', json.dumps(result))
        response['event_diagnostics']['events'][0]['numeric_header_codes']['code'] = 'private'
        with self.assertRaisesRegex(RuntimeError, 'invalid_header_code'):
            runner.filtered_result(json.dumps(response).encode(), value)

    def test_failed_initialization_and_safe_frames_are_collectable(self):
        value = record()
        for scope, cleanup in (('no_initialization', None), ('temporary_storage_only', True)):
            response = {**final_result(value), 'ok': False, 'cleanup_scope': scope,
                        'cleanup_verified': cleanup, 'phase': 'cloud_client_initialization',
                        'error_type': 'IllegalStateException',
                        'error_frames': ['java.lang.reflect.Method.invoke(Native Method)',
                                         'com.example.Client.run(Client.kt:123)',
                                         'Secret account or message content']}
            result = runner.filtered_result(json.dumps(response).encode(), value)
            self.assertEqual(result['cleanup_scope'], scope)
            self.assertEqual(len(result['error_frames']), 2)
            self.assertEqual(result['error_frames_filtered'], 1)
            self.assertNotIn('Secret', json.dumps(result))

    def test_error_classification_accepts_only_fixed_categories_and_boolean_keywords(self):
        value = record(); response = final_result(value)
        response['event_diagnostics'] = dict(events=[dict(stage='AGENT_CALL_RECEIVED',
            reason_classification_candidate='rpc_unimplemented',
            reason_keyword_candidates={'unimplemented':True,'raw':'private server details'},
            reason_resource_candidates={'agent':True,'raw':'private'},
            reason_missing_phrase_candidates={'agent_not_found':True,'private':True},
            reason_known_config_candidates={'mentions_default_model_id':True,'model_id':'private'})])
        result = runner.filtered_result(json.dumps(response).encode(), value)
        event = result['event_diagnostics']['events'][0]
        self.assertEqual(event['reason_keyword_candidates'], {'unimplemented':True})
        self.assertEqual(event['reason_resource_candidates'], {'agent':True})
        self.assertEqual(event['reason_missing_phrase_candidates'], {'agent_not_found':True})
        self.assertEqual(event['reason_known_config_candidates'], {'mentions_default_model_id':True})
        self.assertNotIn('private', json.dumps(result))
        response['event_diagnostics']['events'][0]['reason_classification_candidate'] = 'private server details'
        with self.assertRaisesRegex(RuntimeError, 'invalid_reason_classification'):
            runner.filtered_result(json.dumps(response).encode(), value)
        response['event_diagnostics']['events'][0]['reason_classification_candidate'] = 'unclassified'
        response['event_diagnostics']['events'][0]['reason_keyword_candidates']['unimplemented'] = 'private'
        with self.assertRaisesRegex(RuntimeError, 'invalid_reason_keyword'):
            runner.filtered_result(json.dumps(response).encode(), value)

    def test_saved_model_metadata_never_claims_cloud_permission_or_exposes_preferences(self):
        value = record(); response = final_result(value)
        item = dict(status='saved_selection', permission_verified=False, source_modified=False,
            current_model_id='known-model', model_cell_type='StepGelabModelCell',
            model_map='private', account_token='secret', stable_copy_verified=True)
        response['model_preferences'] = item
        result = runner.filtered_result(json.dumps(response).encode(), value)
        self.assertEqual(result['model_preferences']['current_model_id'], 'known-model')
        self.assertNotIn('private', json.dumps(result))
        self.assertNotIn('secret', json.dumps(result))
        for key, changed in [('permission_verified', True), ('source_modified', True),
                             ('current_model_id', 'https://private/token'),
                             ('model_cell_type', 'unknown')]:
            response['model_preferences'] = {**item, key:changed}
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                runner.filtered_result(json.dumps(response).encode(), value)

    def test_agentbus_supervisor_supplies_non_file_publish(self):
        permissions = runner.module('task_host_template_test', 'companion-permissions.py')
        template = ast.parse(permissions.AGENTBUS_WORKER_TEMPLATE)
        first_import = next(node for node in template.body if isinstance(node, ast.Import))
        self.assertTrue({'gzip', 'zlib'}.issubset({item.name for item in first_import.names}))
        publish = next(node for node in template.body
                       if isinstance(node, ast.FunctionDef) and node.name == 'publish')
        namespace = dict(json=json, shared=bytearray(1024))
        exec(compile(ast.Module(body=[publish], type_ignores=[]), '<supervisor-publish>', 'exec'), namespace)
        namespace['publish']({'spawned_pid': 42})
        length = int.from_bytes(namespace['shared'][:4], 'little')
        self.assertEqual(json.loads(namespace['shared'][4:4 + length]), {'spawned_pid': 42})

    def test_workers_compile_after_real_transport_code_result_substitution(self):
        permissions = runner.module('task_host_transport_test', 'companion-permissions.py')
        template = permissions.agentbus_worker_source(
            (permissions.WORKFLOW / 'tools/root_worker.py').read_text())
        assignment = next(node for node in ast.parse(template).body
                          if isinstance(node, ast.Assign)
                          and any(isinstance(target, ast.Name) and target.id == 'src' for target in node.targets))
        value = record(); payload = b'dex\n035\x00test'
        value['dex_sha256'] = hashlib.sha256(payload).hexdigest()
        workers = (runner.build_start_worker(value, payload, {'CLASSPATH': '/reviewed.dex'}),
                   runner.build_collect_worker(value, True))
        for code in workers:
            with self.subTest(operation='start' if 'host_detached' in code else 'collect'):
                self.assertNotIn('RESULT', code)
                self.assertNotIn('CODE', code)
                # Evaluate only the real transport's literal template expression;
                # no root_worker top-level code or device command is executed.
                transported = eval(compile(ast.Expression(assignment.value), '<transport-src>', 'eval'),
                                   dict(code=code, result='/tmp/stepx-worker-test.json', os=os))
                compile(transported, '<actual-transport-worker>', 'exec')

    def test_generated_workers_compile_and_have_no_host_kill_or_wait(self):
        value = record(); payload = b'dex\n035\x00test'
        value['dex_sha256'] = hashlib.sha256(payload).hexdigest()
        start = runner.build_start_worker(value, payload, {'CLASSPATH': runner.REMOTE_DIR + '/' + value['dex_name']})
        collect = runner.build_collect_worker(value, True)
        for code in (start, collect):
            tree = ast.parse(code)
            for call in (node for node in ast.walk(tree) if isinstance(node, ast.Call)):
                self.assertNotIn(getattr(call.func, 'attr', ''), ('kill', 'terminate', 'communicate', 'wait'))
            compile(code, '<worker>', 'exec')

    def test_actual_compressed_dex_transport_stays_below_single_argument_limit(self):
        permissions = runner.module('task_host_transport_size_test', 'companion-permissions.py')
        source = permissions.agentbus_worker_source((permissions.WORKFLOW / 'tools/root_worker.py').read_text())
        tree = ast.parse(source)
        src = next(node for node in tree.body if isinstance(node, ast.Assign)
                   and any(isinstance(target, ast.Name) and target.id == 'src' for target in node.targets))
        stage_def = next(node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == 'stage_source')
        value = record(); payload = runner.DEX.read_bytes() if runner.DEX.is_file() else (b'dex\n035\x00' + b'x' * 16384)
        value['dex_sha256'] = hashlib.sha256(payload).hexdigest()
        code = runner.build_start_worker(value, payload, {'CLASSPATH': runner.REMOTE_DIR + '/' + value['dex_name']})
        payload_assignment = next(node for node in ast.parse(code).body if isinstance(node, ast.Assign)
                                  and any(isinstance(target, ast.Name) and target.id == 'PAYLOAD' for target in node.targets))
        packed = ast.literal_eval(payload_assignment.value)
        self.assertEqual(gzip.decompress(base64.b64decode(packed)), payload)
        self.assertIn("hashlib.sha256(data).hexdigest()==RECORD['dex_sha256']", code)
        transported = eval(compile(ast.Expression(src.value), '<real-src>', 'eval'),
                           dict(code=code, result='/tmp/stepx-worker-test.json', os=os))
        compile(transported, '<real-worker>', 'exec')
        gw_calls = []
        with tempfile.TemporaryDirectory() as tmp:
            wrapper = str(Path(tmp) / 'stepx-gms-worker-ffffffffff.py')
            def fake_gw(cmd):
                gw_calls.append(cmd)
                exec(cmd, {})
            ns = {'gw': fake_gw, 'base64': base64, 'hashlib': hashlib}
            exec(compile(ast.Module(body=[stage_def], type_ignores=[]), '<stage_source>', 'exec'), ns)
            ns['stage_source'](wrapper, transported)
            self.assertEqual(Path(wrapper).read_text(encoding='utf-8'), transported)
        self.assertTrue(gw_calls)
        # Linux limits a single exec argument to 128 KiB on this host. Keep
        # 16 KiB spare for every chunked gateway call.
        for argument in gw_calls:
            self.assertLess(len(argument.encode()), 112 * 1024)

    def execute_collect(self, mutation=None, live=None, externally_exited=True, result_changes=None):
        """Execute the actual emitted unlink/read block against disposable files."""
        value = record()
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            files = {'dex': directory / value['dex_name'], 'result': directory / value['result_name']}
            files['dex'].write_bytes(b'owned dex')
            files['result'].write_text(json.dumps({**final_result(value), **(result_changes or {})}) + '\n')
            for key, path in files.items():
                path.chmod(0o400 if key == 'dex' else 0o600)
                current = path.stat()
                value['files'][key] = dict(dev=current.st_dev, ino=current.st_ino,
                                           uid=1000, gid=1000, mode=stat.S_IMODE(current.st_mode), nlink=1)
            if mutation:
                mutation(value, files)
            code = runner.build_collect_worker(value, externally_exited)
            # Keep the real generated metadata, owner match, filtered-result and
            # collection code. Stub only device process observation/publication.
            tree = ast.parse(runner.WORKER_COMMON)
            helpers = [node for node in tree.body if isinstance(node, ast.FunctionDef)
                       and node.name in ('checked', 'metadata', 'match_file')]
            namespace = dict(os=os, stat=stat, json=json, re=re, RECORD=value,
                             MAX_OUTPUT_BYTES=runner.MAX_OUTPUT_BYTES, out={},
                             EXTERNALLY_EXITED=externally_exited, observed=lambda pid: live,
                             publish=lambda value: None)
            exec(compile(ast.Module(body=helpers, type_ignores=[]), '<helpers>', 'exec'), namespace)
            # Test users need not have phone UID 1000. Normalize only metadata's
            # owner from these locally-created files; inode/type/mode remain real.
            original_metadata = namespace['metadata']
            def local_metadata(current):
                observed = original_metadata(current)
                observed.update(uid=1000, gid=1000)
                return observed
            namespace['metadata'] = local_metadata
            exec(runner.RESULT_FILTER, namespace)
            namespace['directory'] = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
            exec(code[code.index("out['operation']='task_host_collect'"):], namespace)
            return namespace['out'], {key: path.exists() for key, path in files.items()}

    def test_collection_removes_only_verified_owned_files_after_exit(self):
        output, exists = self.execute_collect()
        self.assertEqual(output['status'], 'collected')
        self.assertEqual(exists, {'dex': False, 'result': False})

    def test_live_host_is_pending_without_reading_or_removing_files(self):
        output, exists = self.execute_collect(live=record()['process'], externally_exited=False)
        self.assertEqual(output['status'], 'pending')
        self.assertTrue(all(exists.values()))
        self.assertNotIn('final', output)

    def test_inode_replacement_pid_reuse_hidepid_and_invalid_output_keep_files(self):
        cases = (
            dict(mutation=lambda value, files: value['files']['result'].update(ino=value['files']['result']['ino'] + 1)),
            dict(live={**record()['process'], 'start_ticks': 'new-process'}),
            dict(externally_exited=False),
            dict(result_changes={'pid': 99}),
        )
        for case in cases:
            with self.subTest(case=list(case)):
                output, exists = self.execute_collect(**case)
                self.assertIn('operation_error', output)
                self.assertTrue(all(exists.values()))
                self.assertNotIn('owned_files_removed', output)

    def test_idle_display_baseline_rejects_missing_membership_and_active_display(self):
        idle = ('  OmniApps:\n    com.stepos.ability\n'
                '  OmniImeEnabledDisplays: size=0\n  KeepInputFocusDisplays: size=0\n  Display 0 other\n')
        class Adb:
            def __init__(self, text): self.text = text
            def shell(self, *args): return self.text
        self.assertEqual(runner.readiness.display_baseline(Adb(idle))['displays'], ['0'])
        for output in (idle.replace('com.stepos.ability', 'other'), idle.replace('size=0', 'size=1', 1),
                       idle + '  Display 2 other\n'):
            with self.subTest(output=output), self.assertRaises(RuntimeError):
                runner.readiness.display_baseline(Adb(output))

    def test_collection_retry_finalizes_without_worker_or_file_cleanup(self):
        value = record()
        value['display_baseline'] = {'idle': True}
        value['collect_proof'] = dict(operation='task_host_collect', status='collected',
                                      host_exited=True, owned_files_removed=True,
                                      removed={'dex': True, 'result': True}, worker_exit_code=0,
                                      final=final_result(value))
        class Adb:
            def shell(self, *args):
                return value['boot_id'] if args[-1] == '/proc/sys/kernel/random/boot_id' else ''
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'record.json'
            with patch.object(runner, 'load_record', return_value=(path, value)), \
                    patch.object(runner, 'permissions_and_adb', return_value=(object(), Adb())), \
                    patch.object(runner, 'strict_baseline', return_value={'immutable_files_verified': True}), \
                    patch.object(runner, 'external_process', return_value=None), \
                    patch.object(runner.readiness, 'display_baseline', side_effect=[RuntimeError('display active'), {'idle': True}]), \
                    patch.object(runner, 'preserve_worker') as worker:
                with self.assertRaisesRegex(RuntimeError, 'display active'):
                    runner.collect(path)
                self.assertEqual(value['status'], 'collection_verification_pending')
                self.assertTrue(value['collect_proof']['owned_files_removed'])
                result = runner.collect(path, finalize_only=True)
                self.assertTrue(result['verification_only'])
                self.assertFalse(result['global_rollback_verified'])
                worker.assert_not_called()

    def test_finalization_rejects_partial_file_cleanup_proof(self):
        value = record()
        value['collect_proof'] = dict(operation='task_host_collect', status='collected',
                                      host_exited=True, owned_files_removed=True,
                                      removed={'dex': True}, worker_exit_code=0, final=final_result(value))
        with self.assertRaisesRegex(RuntimeError, 'complete owned-file'):
            runner.collected_evidence(value)


if __name__ == '__main__':
    unittest.main()
