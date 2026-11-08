"""Canonical bytes for reviewed, local Python source sent to a Unix device.

Only checkout line endings are normalized. Device readback, recovery evidence,
APK/DEX files and other binaries must continue to use exact-byte hashes.
"""
from __future__ import annotations

import hashlib
from pathlib import Path


def canonical_python_source(data: bytes, *, label: str = 'Python source') -> bytes:
    """Accept UTF-8 LF/CRLF source, rejecting ambiguous bare CR characters."""
    try:
        data.decode('utf-8', errors='strict')
    except UnicodeDecodeError as error:
        raise ValueError(f'{label}: expected UTF-8 Python source') from error
    result = data.replace(b'\r\n', b'\n')
    if b'\r' in result:
        raise ValueError(f'{label}: bare CR is not a supported source line ending')
    return result


def read_python_source(path: str | Path) -> bytes:
    path = Path(path)
    if path.suffix.lower() != '.py':
        raise ValueError('Source normalization is restricted to Python files')
    return canonical_python_source(path.read_bytes(), label=path.name)


def python_source_sha256(path: str | Path) -> str:
    return hashlib.sha256(read_python_source(path)).hexdigest()
