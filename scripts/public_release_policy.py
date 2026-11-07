"""Shared publication boundary. Findings never contain matched secret values.

Source export is reviewed code + explicit documentation/media. Runtime resources
are a second, smaller list. Neither mode reads the user's state directory.
"""
from __future__ import annotations
import hashlib
import json
import re
from pathlib import Path, PurePosixPath

MANIFEST = 'scripts/release-resources.json'
MEDIA = {'.png', '.jpg', '.jpeg', '.gif', '.webp', '.ico', '.mp4', '.h264'}
FORBIDDEN_PARTS = {'.git', '.runtime', '.tools', '__pycache__', 'diagnostics',
    'review-assets', 'evidence', 'state', 'backups', 'cache', 'caches', 'logs',
    'downloads', 'node_modules', 'target'}
FORBIDDEN_SUFFIXES = {'.db', '.sqlite', '.sqlite3', '.log', '.p12', '.pfx',
    '.jks', '.keystore', '.key', '.pem', '.pyc', '.idsig'}
PRIVATE_FILENAMES = {'model.json', 'credentials.json', 'secrets.json', 'auth.json',
    'token.json', 'adbkey', 'adbkey.pub', 'id_rsa', 'id_ed25519'}
SECRET_PATTERNS = (
    ('private key', re.compile(r'-----BEGIN (?:[A-Z0-9 ]*)PRIVATE' + r' KEY-----')),
    ('GitHub token', re.compile(r'\b(?:gh[pousr]_|github_pat_)[A-Za-z0-9_]{20,}\b')),
    ('API token', re.compile(r'\bsk' + r'-(?:proj-)?[A-Za-z0-9_-]{20,}\b')),
    ('credential URL', re.compile(r'https?://[^\s/\"\'<>:@]+:[^\s/\"\'<>@]+@')),
    ('device serial', re.compile(r'ZYPZ261A[0-9A-Z]{8,}')),
    ('developer home', re.compile(r'/(?:home|Users)/(?!stepx-user(?:/|\b)|example(?:/|\b)|macuser(?:/|\b)|user(?:/|\b)|test(?:/|\b)|testuser(?:/|\b)|alice(?:/|\b)|bob(?:/|\b)|john(?:/|\b)|admin(?:/|\b)|blue(?:/|\b)|aloke(?:/|\b)|demo(?:/|\b))[A-Za-z0-9_.-]+/')),
)


def manifest(root: Path) -> dict:
    value = json.loads((root / MANIFEST).read_text(encoding='utf-8'))
    if value.get('schema') != 1:
        raise ValueError('Unsupported release resource manifest')
    return value


def path_problems(rel: str) -> list[str]:
    p = PurePosixPath(rel)
    if p.is_absolute() or '..' in p.parts or '\\' in rel:
        return [f'{rel}: unsafe relative path']
    if any(part.lower() in FORBIDDEN_PARTS for part in p.parts):
        return [f'{rel}: runtime/private-data directory is not a release input']
    name = p.name.lower()
    if p.suffix.lower() in FORBIDDEN_SUFFIXES or name in PRIVATE_FILENAMES or name.startswith('.env'):
        return [f'{rel}: private-data or credential filename is not a release input']
    if name.endswith(('-wal', '-shm', '-journal')):
        return [f'{rel}: database sidecar is not a release input']
    return []


def content_problems(rel: str, data: bytes, policy: dict, *, media_review: bool = True) -> list[str]:
    problems = path_problems(rel)
    suffix = PurePosixPath(rel).suffix.lower()
    if suffix in MEDIA:
        if media_review and hashlib.sha256(data).hexdigest() not in policy['reviewed_media_sha256']:
            problems.append(f'{rel}: new or changed media needs visual privacy review and a recorded SHA-256')
        return problems
    try:
        text = data.decode('utf-8')
    except UnicodeDecodeError:
        # Compiled application assets are separately allowlisted and built from
        # reviewed sources. Raw DBs and private keys are rejected by filename.
        return problems
    for name, pattern in SECRET_PATTERNS:
        for match in pattern.finditer(text):
            if (name == 'credential URL'
                    and rel.startswith('runtime/python/')
                    and rel.endswith('/site-packages/pip/_vendor/urllib3/util/url.py')
                    and match[0] == 'https://' + 'username:password@'):
                # Literal URL formatting example in the hash-pinned CPython
                # distribution, not account credentials.
                continue
            if any(f['path'] == rel and f['pattern'] == name and f['sha256'] == hashlib.sha256(match[0].encode()).hexdigest() for f in policy.get('reviewed_text_fixtures', [])):
                continue
            if name == 'credential URL' and match[0] in ('https://' + 'user:pass@', 'https://' + 'u:p@'):
                # Only the literal redaction-test credentials at an RFC example host.
                if re.match(r'example\.(?:org|com|net)(?:[\s/\"\\]|$)', text[match.end():]):
                    continue
            problems.append(f'{rel}:{text.count(chr(10), 0, match.start()) + 1}: {name}')
    return problems


def safe_read(root: Path, rel: str) -> bytes:
    issues = path_problems(rel)
    if issues:
        raise ValueError('\n'.join(issues))
    path = root / rel
    if any(part.is_symlink() for part in [path, *path.parents] if part != root.parent):
        raise ValueError(f'{rel}: symlink is not a source publication input')
    path.resolve().relative_to(root.resolve())
    return path.read_bytes()


def generic_target(root: Path) -> bytes:
    # Do not read target.json: on developer machines it can name a real phone.
    value = json.loads(safe_read(root, 'handoff/google-play-stepx/target.example.json'))
    if value.get('serial'):
        raise ValueError('target.example.json must not select a physical phone')
    return (json.dumps(value, ensure_ascii=False, indent=2) + '\n').encode()


def runtime_plan(root: Path) -> dict[str, bytes]:
    policy = manifest(root)
    # Ship only the release story matching the built app. Older histories remain
    # in source control; duplicating all videos in every installer is unnecessary.
    import tomllib
    cargo = root / 'desktop/Cargo.toml'
    version = tomllib.loads(cargo.read_text())['package']['version'] if cargo.exists() else None
    result = {}
    for rel in policy['runtime_files']:
        if rel.startswith('desktop/assets/release-notes/') and version:
            tail = rel.removeprefix('desktop/assets/release-notes/')
            if tail != version + '.json' and not tail.startswith(version + '/'):
                continue
        result[rel] = safe_read(root, rel)
    result['handoff/google-play-stepx/target.json'] = generic_target(root)
    result['handoff/google-play-stepx/target.example.json'] = generic_target(root)
    issues = [issue for rel, data in result.items() for issue in content_problems(rel, data, policy)]
    if issues:
        raise ValueError('\n'.join(issues))
    return result


def audit_bundle(root: Path) -> list[str]:
    """Check an already staged package, including unexpected loose files.

    Tool/JRE/Python trees have vendor-specific layouts. They are still checked
    for private-data filenames, external symlinks, DBs and secret text; product
    resource directories are additionally an exact allowlist.
    """
    policy = manifest(root)
    allowed = set(policy['runtime_files']) | set(policy.get('package_documents', [])) | {
        'handoff/google-play-stepx/target.json', 'handoff/google-play-stepx/target.example.json',
        'release-resources.sha256.json', 'stepx-companion', 'stepx-companion.exe', 'StepX.icns'}
    issues = []
    for path in root.rglob('*'):
        rel = path.relative_to(root).as_posix()
        if path.is_symlink():
            try: path.resolve(strict=True).relative_to(root.resolve())
            except (ValueError, OSError): issues.append(f'{rel}: broken or external package symlink')
            continue
        if not path.is_file():
            continue
        parts = PurePosixPath(rel).parts
        is_product = parts[0] not in {'runtime', 'tools'}
        if is_product and rel not in allowed:
            issues.append(f'{rel}: file is outside the installed-resource allowlist')
        # Public trust certificates in a bundled runtime are not signing secrets.
        if parts[0] in {'runtime', 'tools'}:
            if path.name in {'cacerts', 'cert.pem', 'cacert.pem'}:
                continue
            # Vendor tool roots include executables and licence text, not source
            # export media. Limit textual scanning to reasonable-sized files.
            vendor_cache_module = (rel.startswith('runtime/python/') and
                '/site-packages/pip/_vendor/cachecontrol/caches/' in rel and path.suffix == '.py')
            checked_rel = rel.replace('/cachecontrol/caches/', '/cachecontrol/cache_modules/') if vendor_cache_module else rel
            issues.extend(path_problems(checked_rel))
            if path.stat().st_size <= 2_000_000 and path.suffix.lower() in {'.json', '.txt', '.py', '.sh', '.ps1'}:
                issues.extend(content_problems(checked_rel, path.read_bytes(), policy, media_review=False))
        else:
            issues.extend(content_problems(rel, path.read_bytes(), policy, media_review=is_product))
    return sorted(set(issues))


def audit_source(root: Path) -> list[str]:
    """Audit Git publication inputs, including tracked files ignored today.

    Build output and local ignored files are not publication inputs. We never
    modify or rewrite Git history here; historical remediation is a separate
    explicit operation.
    """
    import subprocess
    policy = manifest(root)
    result = subprocess.run(['git', '-C', str(root), 'ls-files', '-z', '--cached',
                             '--others', '--exclude-standard'], check=True,
                            stdout=subprocess.PIPE)
    issues = []
    for rel in sorted(set(result.stdout.decode('utf-8').split('\0')) - {''}):
        path = root / rel
        if not path.exists():
            continue  # a staged deletion is not published content
        if path.is_symlink():
            issues.append(f'{rel}: source symlinks require explicit review')
            continue
        if not path.is_file():
            continue
        issues.extend(content_problems(rel, path.read_bytes(), policy))
    return sorted(set(issues))
