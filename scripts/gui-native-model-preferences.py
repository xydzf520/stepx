#!/usr/bin/env python3
"""Research-only, non-native MMKV metadata reader. No device execution here.

Format references, checked against the pinned APK's MMKV ABI/symbols:
https://github.com/Tencent/MMKV/blob/v2.2.4/Core/MMKVMetaInfo.hpp
https://github.com/Tencent/MMKV/blob/v2.2.4/Core/MiniPBCoder.cpp#L467
https://github.com/Tencent/MMKV/blob/v2.2.4/Core/CodedInputData.cpp
https://github.com/Tencent/MMKV/blob/v2.2.4/Core/MMKV_IO.cpp#L1486

Only a bounded unencrypted, non-expiring, CRC-consistent snapshot is accepted.
No native MMKV, mmap, recovery, migration, write, guessed key scan, all-keys
export, account read, model selection or cloud invocation is performed.
build_worker() returns code for the existing independently guarded Android
worker; this module never starts that transport. Import has no side effects.
"""
import hashlib
import inspect
import json
import os
import re
import stat
import struct
import zlib

APK = '/system/priv-app/ability/ability.apk'
APK_SHA = 'c9f56f3078df96d8d726422b1c622a1c19fe8c7dec8cadd8537d9a6ecf513ac4'
ALLOWED_APK_SHAS = (
    APK_SHA,
    'a7dbeaa721c13836bbb3990026af6c3d66bf164afc4c73f70bd453aa119fbca8',
)
SOURCE = '/data/user/0/com.stepos.ability/files/mmkv/model_config_prefs'
DEFAULT_ID = 'step3p7-mtp3-0520-mm-reasoning-effort-fp8-aos-prod'
BUILTINS = {
    'uiagentmeituan-12071133': 'StepPlanTreeModelCell',
    'doubao-1.5-ui-tars': 'DoubaoTarsModelCell',
    'doubao-seed-1-8-251228': 'DoubaoSeedModelCell',
    'google/gemini-2.5-pro': 'GeminiModelCell',
    DEFAULT_ID: 'StepGelabModelCell',
    'autoglm-phone': 'GlmAtuoModelCell',
    'glm-4.6v': 'GLMModelCell',
    'MiMo-VL-7B-RL-2508-V1-1209': 'MiMoModelCell',
    'local-vlm': 'LocalModelCell',
    'q3-4b-tob-sample-ge-cap-aw-aoss2v4-ls1v15-260530-aos-prod': 'LocalCloudModelCell',
}
MAX_DATA = 1048576
MAX_CRC = 65536


def require(condition, code):
    if not condition:
        raise ValueError(code)


def varint32(data, offset, end):
    value = 0
    for index in range(5):
        require(offset < end, 'truncated_varint')
        byte = data[offset]
        offset += 1
        require(index < 4 or byte <= 15, 'oversized_varint')
        value |= (byte & 127) << (7 * index)
        if not byte & 128:
            return value, offset
    raise ValueError('oversized_varint')


def block(data, offset, end, limit):
    size, offset = varint32(data, offset, end)
    require(size <= limit and size <= end - offset, 'invalid_block_length')
    return data[offset:offset + size], offset + size


def selected_strings(data, crc):
    """Return ONLY the two whitelisted keys; never decode unrelated values."""
    require(type(data) is bytes and type(crc) is bytes, 'invalid_input_type')
    require(4 <= len(data) <= MAX_DATA and 112 <= len(crc) <= MAX_CRC, 'invalid_file_length')
    actual = struct.unpack_from('<I', data)[0]
    digest, version = struct.unpack_from('<II', crc)
    require(version in (1, 2, 3, 4), 'unsupported_meta_version')
    require(not any(crc[12:28]), 'unsupported_crypto_metadata')
    require(struct.unpack_from('<Q', crc, 104)[0] == 0, 'unsupported_meta_flags')
    require(actual <= len(data) - 4, 'invalid_actual_length')
    if version >= 3:
        require(struct.unpack_from('<I', crc, 28)[0] == actual, 'inconsistent_actual_length')
    payload = data[4:4 + actual]
    require(zlib.crc32(payload) & 0xffffffff == digest, 'crc_mismatch')
    if not payload:
        return {}, version
    # The initial varint is an ignored/random size holder, not a trustworthy
    # total length. OEM MiniPBCoder explicitly reads and discards it.
    _, offset = varint32(payload, 0, len(payload))
    selected = {}
    count = 0
    while offset < len(payload):
        count += 1
        require(count <= 4096, 'too_many_entries')
        key, offset = block(payload, offset, len(payload), 4096)
        require(bool(key), 'unsupported_empty_key')
        value, offset = block(payload, offset, len(payload), MAX_DATA)
        if key not in (b'current_model_id', b'model_map'):
            continue
        name = key.decode('ascii')
        if not value:  # Append-only deletion tombstone; last value wins.
            selected.pop(name, None)
            continue
        limit = 160 if name == 'current_model_id' else 131072
        encoded, end = block(value, 0, len(value), limit)
        require(end == len(value), 'unexpected_string_trailer')
        try:
            selected[name] = encoded.decode('utf-8', 'strict')
        except UnicodeDecodeError:
            raise ValueError('invalid_selected_utf8') from None
    return selected, version


def describe(selected, version):
    out = {'version': 1, 'source': 'original_gui_model_preferences',
           'parser': 'bounded_mmkv_bytes_v1', 'native_mmkv_loaded': False,
           'permission_verified': False, 'meta_version': version}
    current = selected.get('current_model_id')
    out['saved_selection_present'] = bool(current)
    if not current:
        return dict(out, status='no_saved_selection')
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/+:-]{0,159}', current) or '://' in current or '..' in current:
        return dict(out, status='invalid_selection')
    cell = BUILTINS.get(current)
    custom = cell is None
    if custom and selected.get('model_map'):
        try:
            mapping = json.loads(selected['model_map'])
        except (ValueError, RecursionError):
            raise ValueError('invalid_model_map') from None
        require(type(mapping) is dict and len(mapping) <= 512, 'invalid_model_map')
        value = mapping.get(current)
        if type(value) is str and value in BUILTINS.values():
            cell = value
    if cell is None:
        return dict(out, status='unregistered_selection')
    return dict(out, status='saved_selection', current_model_id=current,
                model_cell_type=cell, custom_model=custom,
                matches_static_default=current == DEFAULT_ID)


def parse_preferences(data, crc):
    values, version = selected_strings(data, crc)
    return describe(values, version)


def read_owned(path, maximum):
    before = os.lstat(path)
    require(stat.S_ISREG(before.st_mode) and before.st_uid == 1000 and before.st_nlink == 1,
            'invalid_source_identity')
    require(0 <= before.st_size <= maximum, 'invalid_source_length')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        opened = os.fstat(fd)
        identity = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
        require(identity(before) == identity(opened), 'source_changed')
        chunks, size = [], 0
        while True:
            chunk = os.read(fd, min(65536, maximum + 1 - size))
            if not chunk:
                break
            chunks.append(chunk)
            size += len(chunk)
            require(size <= maximum, 'source_changed')
        require(size == before.st_size and identity(before) == identity(os.fstat(fd)), 'source_changed')
        return b''.join(chunks)
    finally:
        os.close(fd)


def worker_snapshot():
    # The existing outer transport guards firmware/lifetime; pin the code ABI
    # again here. These assertions run only when root explicitly executes it.
    require(os.geteuid() == 0, 'unexpected_worker_uid')
    with open(APK, 'rb') as stream:
        digest = hashlib.sha256()
        for chunk in iter(lambda: stream.read(65536), b''):
            digest.update(chunk)
    require(digest.hexdigest() in ALLOWED_APK_SHAS, 'unsupported_original_apk')
    source = os.path.realpath(SOURCE)
    require(source == os.path.join(os.path.realpath('/data/user/0/com.stepos.ability'),
                                  'files', 'mmkv', 'model_config_prefs'), 'redirected_source')
    exists = []
    for path in (SOURCE, SOURCE + '.crc'):
        try:
            os.lstat(path)
            exists.append(True)
        except FileNotFoundError:
            exists.append(False)
    if not any(exists):
        return {'version': 1, 'source': 'original_gui_model_preferences', 'status': 'absent',
                'source_exists': False, 'source_modified': False, 'native_mmkv_loaded': False}
    require(all(exists), 'incomplete_source_pair')
    data = read_owned(SOURCE, MAX_DATA)
    crc = read_owned(SOURCE + '.crc', MAX_CRC)
    result = parse_preferences(data, crc)
    require(data == read_owned(SOURCE, MAX_DATA) and crc == read_owned(SOURCE + '.crc', MAX_CRC),
            'source_changed')
    return dict(result, source_exists=True, source_modified=False, stable_copy_verified=True)


def build_worker():
    """Generate self-contained, read-only code; never execute a device command."""
    code = 'import hashlib,json,os,re,stat,struct,zlib\n'
    for name in ('APK', 'APK_SHA', 'ALLOWED_APK_SHAS', 'SOURCE', 'DEFAULT_ID', 'BUILTINS', 'MAX_DATA', 'MAX_CRC'):
        code += name + '=' + repr(globals()[name]) + '\n'
    for function in (require, varint32, block, selected_strings, describe, parse_preferences,
                     read_owned, worker_snapshot):
        code += inspect.getsource(function) + '\n'
    code += '''try:
 result=worker_snapshot()
except Exception as failure:
 # Never print path-bearing OSError, JSON fragments, or arbitrary messages.
 allowed={'invalid_input_type','invalid_file_length','unsupported_meta_version','unsupported_crypto_metadata',
 'unsupported_meta_flags','invalid_actual_length','inconsistent_actual_length','crc_mismatch','truncated_varint',
 'oversized_varint','too_many_entries','invalid_block_length','unsupported_empty_key','unexpected_string_trailer',
 'invalid_selected_utf8','invalid_model_map','invalid_source_identity','invalid_source_length','source_changed',
 'unexpected_worker_uid','unsupported_original_apk','redirected_source','incomplete_source_pair'}
 reason=str(failure) if type(failure) is ValueError and str(failure) in allowed else 'read_failed'
 result={'version':1,'source':'original_gui_model_preferences','status':'rejected','reason':reason,'native_mmkv_loaded':False}
print(json.dumps(result,separators=(',',':'),sort_keys=True))
'''
    compile(code, 'gui-model-preferences-readonly-worker', 'exec')
    return code


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--emit-worker', action='store_true', help='print code only; does not run it')
    args = parser.parse_args()
    if not args.emit_worker:
        parser.error('Only --emit-worker is supported; device execution belongs to the audited caller')
    print(build_worker())
