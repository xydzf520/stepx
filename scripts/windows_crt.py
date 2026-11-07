"""App-local Windows CRT deployment, without PATH/System32 dependency fallback.

Only unmodified x64 release CRT files from the active Visual Studio installation
are distributable inputs. PE inspection is pure Python so the closure tests also
run on Linux and macOS; Windows itself still runs the packaged executable.
"""
from __future__ import annotations

import ctypes
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
from pathlib import Path, PurePosixPath

CRT_DLLS = frozenset({
    'concrt140.dll', 'msvcp140.dll', 'msvcp140_1.dll', 'msvcp140_2.dll',
    'msvcp140_atomic_wait.dll', 'msvcp140_codecvt_ids.dll',
    'vcruntime140.dll', 'vcruntime140_1.dll', 'vcruntime140_threads.dll',
})
CRT_MANIFEST = 'windows-crt.json'
# OS components of supported Windows 10/11. Do not accept arbitrary files merely
# because the build runner happens to have them in System32 (notably VC redist).
SYSTEM_DLLS = frozenset(('advapi32 avrt bcrypt bcryptprimitives cabinet cfgmgr32 '
    'combase comctl32 comdlg32 crypt32 cryptbase cryptsp d2d1 d3d11 d3d12 '
    'd3dcompiler_47 dcomp dnsapi dwmapi dwrite dxgi dxva2 gdi32 hid imm32 '
    'iphlpapi kernel32 kernelbase mpr msimg32 msvcrt mswsock ncrypt netapi32 '
    'normaliz nsi ntdll ole32 oleacc oleaut32 opengl32 powrprof propsys psapi '
    'rpcrt4 secur32 setupapi shell32 shlwapi user32 userenv ucrtbase uiautomationcore '
    'urlmon usp10 uxtheme version winhttp wininet winmm wintrust ws2_32 wtsapi32').split())


def is_system_dll(name: str) -> bool:
    name = name.lower()
    return (name == 'winspool.drv' or name.startswith(('api-ms-win-', 'ext-ms-win-'))
            or (name.endswith('.dll') and name[:-4] in SYSTEM_DLLS))


def pe_imports(path: Path) -> list[str]:
    """Read normal and delay imports; reject malformed/non-x64 PE images."""
    data = path.read_bytes()

    def unpack(fmt, offset):
        try:
            return struct.unpack_from(fmt, data, offset)
        except (struct.error, OverflowError) as exc:
            raise ValueError(f'{path.name}: truncated PE image') from exc

    if len(data) < 64 or data[:2] != b'MZ':
        raise ValueError(f'{path.name}: not a PE image')
    pe, = unpack('<I', 60)
    if data[pe:pe + 4] != b'PE\0\0':
        raise ValueError(f'{path.name}: invalid PE signature')
    machine, count = unpack('<HH', pe + 4)
    optional_size, = unpack('<H', pe + 20)
    opt = pe + 24
    magic, = unpack('<H', opt)
    if machine != 0x8664 or magic != 0x20b or optional_size < 112:
        raise ValueError(f'{path.name}: expected x64 PE32+ image')
    image_base, = unpack('<Q', opt + 24)
    header_size, = unpack('<I', opt + 60)
    directory_count, = unpack('<I', opt + 108)
    sections = []
    for index in range(count):
        virtual_size, address, raw_size, raw_at = unpack('<IIII', opt + optional_size + index * 40 + 8)
        sections.append((address, max(virtual_size, raw_size), raw_at, raw_size))

    def offset(rva, length=1):
        if rva < header_size and rva + length <= min(header_size, len(data)):
            return rva
        for address, size, raw_at, raw_size in sections:
            delta = rva - address
            if 0 <= delta < size and delta + length <= raw_size and raw_at + delta + length <= len(data):
                return raw_at + delta
        raise ValueError(f'{path.name}: PE RVA outside file')

    result = set()
    for directory, stride in [(1, 20), (13, 32)]:
        if directory >= directory_count:
            continue
        if 112 + (directory + 1) * 8 > optional_size:
            raise ValueError(f'{path.name}: invalid PE directory count')
        rva, size = unpack('<II', opt + 112 + directory * 8)
        if not rva and not size:
            continue
        if not rva or size < stride or size > 1_000_000:
            raise ValueError(f'{path.name}: invalid PE import table')
        terminated = False
        for index in range(size // stride):
            values = unpack('<' + 'I' * (stride // 4), offset(rva + index * stride, stride))
            if not any(values):
                terminated = True
                break
            name_rva = values[3] if directory == 1 else values[1]
            if directory == 13 and not values[0] & 1:
                name_rva -= image_base
            start = offset(name_rva)
            end = data.find(b'\0', start, min(start + 260, len(data)))
            if end < 0:
                raise ValueError(f'{path.name}: unterminated DLL name')
            try:
                name = data[start:end].decode('ascii').lower()
            except UnicodeDecodeError as exc:
                raise ValueError(f'{path.name}: invalid DLL name') from exc
            if not re.fullmatch(r'[a-z0-9_.-]+\.(?:dll|drv)', name):
                raise ValueError(f'{path.name}: unsafe DLL name')
            result.add(name)
        if not terminated:
            raise ValueError(f'{path.name}: unterminated import table')
    return sorted(result)


def local_dlls(directory: Path) -> dict[str, Path]:
    result = {}
    for path in directory.iterdir():
        if path.suffix.lower() != '.dll':
            continue
        key = path.name.lower()
        if key in result or path.is_symlink() or not path.is_file():
            raise ValueError('Ambiguous or linked app-local DLL')
        result[key] = path
    return result


def verify_closure(executable: Path) -> list[str]:
    """Every non-OS import must resolve beside the executable, recursively."""
    local = local_dlls(executable.parent)
    visited = set()
    pending = [executable]
    while pending:
        path = pending.pop()
        key = path.name.lower()
        if key in visited:
            continue
        visited.add(key)
        for name in pe_imports(path):
            if is_system_dll(name):
                continue
            if name not in local:
                raise ValueError(f'{path.name}: missing app-local dependency {name}')
            pending.append(local[name])
    return sorted(visited - {executable.name.lower()})


def select_redist(installation: Path, toolset: str) -> Path:
    """Select release x64 CRT from the same installation/toolset family."""
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:\.\d+)?', toolset):
        raise ValueError('Invalid active MSVC toolset version')
    family = tuple(map(int, toolset.split('.')[:2]))
    root = installation / 'VC/Redist/MSVC'
    choices = []
    for version in root.iterdir():
        if not re.fullmatch(r'\d+(?:\.\d+){2,3}', version.name):
            continue
        numbers = tuple(map(int, version.name.split('.')))
        if numbers[:2] != family:
            continue
        for path in (version / 'x64').glob('Microsoft.VC*.CRT'):
            path.resolve(strict=True).relative_to(root.resolve(strict=True))
            if path.is_dir() and not path.is_symlink():
                choices.append((numbers, path))
    if not choices:
        raise RuntimeError(f'Matching x64 Visual C++ release CRT not installed for toolset {toolset}')
    return max(choices, key=lambda item: item[0])[1]


def discover_redist() -> tuple[Path, str, str]:
    if os.name != 'nt':
        raise RuntimeError('Visual C++ CRT packaging requires a native Windows runner')
    base = os.environ.get('ProgramFiles(x86)') or os.environ.get('PROGRAMFILES(X86)')
    if not base:
        raise RuntimeError('Windows ProgramFiles(x86) is unavailable')
    vswhere = Path(base) / 'Microsoft Visual Studio/Installer/vswhere.exe'
    # An absolute installer-owned tool, never an arbitrary executable on PATH.
    output = subprocess.check_output([str(vswhere), '-latest', '-products', '*',
        '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
        '-property', 'installationPath', '-utf8'], timeout=30).decode('utf-8-sig').strip()
    installation = Path(output)
    if not output or not installation.is_absolute() or not installation.is_dir():
        raise RuntimeError('Cannot locate active Visual Studio C++ installation')
    active = os.environ.get('VCToolsInstallDir')
    if active:
        tool_path = Path(active).resolve(strict=True)
        rel = tool_path.relative_to((installation / 'VC/Tools/MSVC').resolve(strict=True))
        if len(rel.parts) != 1:
            raise RuntimeError('Active MSVC toolset is outside the selected installation')
        toolset = rel.name
    else:
        toolset = (installation / 'VC/Auxiliary/Build/Microsoft.VCToolsVersion.default.txt').read_text().strip()
    redist = select_redist(installation, toolset)
    return redist, toolset, redist.relative_to(installation).as_posix()


def windows_file_version(path: Path) -> tuple[int, int, int, int]:
    from ctypes import wintypes
    api = ctypes.WinDLL('version', use_last_error=True)
    api.GetFileVersionInfoSizeW.argtypes = [wintypes.LPCWSTR, ctypes.POINTER(wintypes.DWORD)]
    api.GetFileVersionInfoW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p]
    api.VerQueryValueW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR, ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(wintypes.UINT)]
    ignored = wintypes.DWORD()
    size = api.GetFileVersionInfoSizeW(str(path), ctypes.byref(ignored))
    if not size:
        raise RuntimeError(f'{path.name}: missing Microsoft DLL version resource')
    buf = ctypes.create_string_buffer(size)
    value = ctypes.c_void_p()
    length = wintypes.UINT()
    if not api.GetFileVersionInfoW(str(path), 0, size, buf) or not api.VerQueryValueW(buf, '\\', ctypes.byref(value), ctypes.byref(length)):
        raise RuntimeError(f'{path.name}: cannot read Microsoft DLL version')
    if length.value < 52:
        raise RuntimeError(f'{path.name}: invalid Microsoft DLL version')
    fields = ctypes.cast(value, ctypes.POINTER(wintypes.DWORD))
    if fields[0] != 0xfeef04bd:
        raise RuntimeError(f'{path.name}: invalid Microsoft DLL version signature')
    return fields[2] >> 16, fields[2] & 65535, fields[3] >> 16, fields[3] & 65535


def stage_crt(executable: Path, redist: Path, toolset: str, source: str, *, version_reader=windows_file_version) -> dict:
    """Copy only the actual release CRT dependency closure, then verify it."""
    if (PurePosixPath(source).is_absolute() or '..' in PurePosixPath(source).parts
            or '\\' in source or not source.startswith('VC/Redist/MSVC/')):
        raise ValueError('CRT provenance must be relative to the Visual Studio installation')
    available = local_dlls(redist)
    local = local_dlls(executable.parent)
    pending = [executable]
    visited = set()
    records = {}
    minimum = tuple(map(int, toolset.split('.')))
    while pending:
        path = pending.pop()
        if path.name.lower() in visited:
            continue
        visited.add(path.name.lower())
        for name in pe_imports(path):
            if is_system_dll(name):
                continue
            if name in CRT_DLLS:
                origin = available.get(name)
                if origin is None:
                    raise RuntimeError(f'Matching Visual Studio release CRT missing {name}')
                pe_imports(origin)  # x64 and structural validation before copy
                version = version_reader(origin)
                if version[:len(minimum)] < minimum:
                    raise RuntimeError(f'{name}: CRT version is older than active MSVC toolset')
                destination = executable.parent / name
                if name not in records:
                    shutil.copy2(origin, destination)
                    records[name] = {'sha256': hashlib.sha256(destination.read_bytes()).hexdigest(),
                                     'version': '.'.join(map(str, version))}
                local[name] = destination
            if name not in local:
                raise RuntimeError(f'{path.name}: unknown non-system dependency {name}; cannot source from PATH/System32')
            pending.append(local[name])
    dependencies = verify_closure(executable)
    report = {'schema': 1, 'architecture': 'x64', 'toolset': toolset,
              'source': source, 'files': records, 'dependencies': dependencies,
              'license': 'https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution'}
    (executable.parent / CRT_MANIFEST).write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    return report


def verify_manifest(executable: Path) -> dict:
    """Verify shipped DLL hashes and closure without consulting machine DLLs."""
    report = json.loads((executable.parent / CRT_MANIFEST).read_text(encoding='utf-8'))
    if report.get('schema') != 1 or report.get('architecture') != 'x64':
        raise ValueError('Unsupported app-local CRT manifest')
    records = report.get('files')
    if not isinstance(records, dict) or not set(records) <= CRT_DLLS:
        raise ValueError('Unexpected app-local CRT file list')
    local = local_dlls(executable.parent)
    if set(local) & CRT_DLLS != set(records):
        raise ValueError('App-local CRT files do not match manifest')
    for name, record in records.items():
        if hashlib.sha256(local[name].read_bytes()).hexdigest() != record.get('sha256'):
            raise ValueError(f'{name}: app-local CRT checksum mismatch')
    dependencies = verify_closure(executable)
    if dependencies != report.get('dependencies') or set(dependencies) & CRT_DLLS != set(records):
        raise ValueError('App-local CRT dependency closure does not match manifest')
    return report


def relocate_crt(executable: Path, destination: Path) -> None:
    """Copy the verified DLL set with the EXE for a clean relocated runtime test."""
    report = verify_manifest(executable)
    local = local_dlls(executable.parent)
    for name in report['files']:
        shutil.copy2(local[name], destination / name)
    shutil.copy2(executable.parent / CRT_MANIFEST, destination / CRT_MANIFEST)


def package_windows_crt(executable: Path) -> dict:
    redist, toolset, source = discover_redist()
    return stage_crt(executable, redist, toolset, source)
