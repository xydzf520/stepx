#!/usr/bin/env python3
"""Synchronize allowlisted StepX Companion sources into releases/stepx-public.

Supports:
  - Default mode: sync allowlisted directories/files, apply public Cargo.toml
    and script desensitization rewrites, and scan releases/stepx-public for
    secret patterns.
  - --check mode: dry-run verification without modifying files on disk, plus
    full secret pattern scan across staged content and releases/stepx-public.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import stat
import sys
from typing import Dict, List, Set, Tuple

from public_release_policy import manifest, safe_read, content_problems, generic_target, path_problems

ROOT = pathlib.Path(__file__).resolve().parent.parent
PUBLIC_ROOT = ROOT / "releases" / "stepx-public"

ALLOWLIST_DIRS: Tuple[str, ...] = (
    "desktop/src",
    "desktop/tests",
    "desktop/assets",
    "desktop/android-bridge",
    "desktop/amoo-background",
    "desktop/companion-app/src",
    "desktop/companion-app/res",
    "desktop/companion-app/tests",
    "desktop/companion-app/third_party",
    "desktop/companion-app/libs",
    "desktop/companion-permissions",
    "desktop/neo-agent-adapter",
    "handoff/google-play-stepx/tools",
    "handoff/google-play-stepx/module",
)

ALLOWLIST_FILES: Tuple[str, ...] = (
    "desktop/Cargo.toml",
    "desktop/Cargo.lock",
    "desktop/build.rs",
    "desktop/README.md",
    "desktop/DESIGN.md",
    "desktop/companion-app/AndroidManifest.xml",
    "desktop/companion-app/README.md",
    "README.md",
    "LICENSE",
    "NOTICE",
    "THIRD_PARTY_NOTICES.md",
    "CHANGELOG.md",
)

EXCLUDED_SCRIPTS: Set[str] = {
    "google-enable.sh",
    "install-gapps.sh",
    "lan-control.sh",
}

PRESERVED_PUBLIC_SCRIPTS: Set[str] = {
    "ci-platform-smoke.py",
    "package-release.py",
}

EXCLUDED_DOCS: Set[str] = {
    "PROJECT_ANALYSIS.md",
    "stepx-analysis.md",
    "stepclaw-gms-assessment-20261021.md",
    "stepclaw-gms-install-20261022.md",
    "stepclaw-root-audit-20261021.md",
    "google-play-compat-20261023.md",
    "google-play-certification-plan-20261023.md",
    "google-domain-verifier-followup.md",
    "play-protect-certification-model.md",
    "amoo-native-realtime-sync.md",
    "amoo-screen-off-lease.md",
    "amoo-tool-registration-20261023.md",
    "cloudwake-cloud-execution-assessment-20261022.md",
    "native-mcp-integration-20261023.md",
    "storage-compression-assessment-20261022.md",
}

PRESERVED_PUBLIC_DOCS: Set[str] = {
    "BUILDING.md",
}

BINARY_EXTENSIONS: Set[str] = {
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".webp",
    ".ico",
    ".apk",
    ".dex",
    ".jar",
    ".ttf",
    ".otf",
    ".woff",
    ".woff2",
    ".pyc",
    ".so",
    ".dll",
    ".dylib",
    ".exe",
    ".zip",
    ".gz",
}

# Construct secret regexes dynamically so this script never matches itself.
SECRET_PATTERNS: Tuple[Tuple[str, re.Pattern[str]], ...] = (
    ("ghp_", re.compile(r"\b" + "ghp" + r"_[A-Za-z0-9]{20,}\b")),
    ("sk-", re.compile(r"\b" + "sk" + r"-(?:proj-)?[A-Za-z0-9]{20,}\b")),
    (
        "PRIVATE KEY",
        re.compile(r"-----BEGIN [A-Z0-9 ]*" + "PRIVATE " + r"KEY-----"),
    ),
)


def rewrite_cargo_toml(content: str) -> str:
    """Apply public license (AGPL-3.0-only) and ../vendor/ path rewrites."""
    updated = re.sub(
        r'^license\s*=\s*"[^"]+"',
        'license = "AGPL-3.0-only"',
        content,
        count=1,
        flags=re.MULTILINE,
    )
    updated = updated.replace(
        'path = "../../facloud/desktop/crates/facloud_ui"',
        'path = "../vendor/facloud_ui"',
    )
    updated = updated.replace(
        'path = "../../facloud/desktop/crates/markdown_parser"',
        'path = "../vendor/markdown_parser"',
    )
    return updated


def desensitize_script(rel_path: str, content: str) -> str:
    """Strip host-specific developer SDK/JDK paths and device serials."""
    text = content
    text = re.sub(
        r'JDK="\$\{STEPX_JDK:-/home/[^"\n]+/\.local/opt/android-studio/jbr\}"',
        'JDK="${STEPX_JDK:-${JAVA_HOME:?Set JAVA_HOME to JDK 17}}"',
        text,
    )
    text = re.sub(
        r'TASK_JDK="\$\{STEPX_JDK:-/home/[^"\n]+/\.local/opt/android-studio/jbr\}"',
        'TASK_JDK="${STEPX_JDK:-${JAVA_HOME:?Set JAVA_HOME to JDK 17}}"',
        text,
    )
    text = re.sub(
        r'SDK="\$\{ANDROID_SDK_ROOT:-/home/[^"\n]+/Android/Sdk\}"',
        'SDK="${ANDROID_SDK_ROOT:-${ANDROID_HOME:?Set ANDROID_HOME to Android SDK}}"',
        text,
    )
    text = re.sub(
        r'TASK_SDK="\$\{ANDROID_SDK_ROOT:-/home/[^"\n]+/Android/Sdk\}"',
        'TASK_SDK="${ANDROID_SDK_ROOT:-${ANDROID_HOME:?Set ANDROID_HOME to Android SDK}}"',
        text,
    )
    text = re.sub(
        r'SDK_JAR="\$\{STEPX_ANDROID_JAR:-/home/[^"\n]+/Android/Sdk/platforms/android-36/android\.jar\}"',
        'SDK_JAR="${STEPX_ANDROID_JAR:-${ANDROID_SDK_ROOT:-${ANDROID_HOME:?Set ANDROID_HOME}}/platforms/android-36/android.jar}"',
        text,
    )
    text = re.sub(
        r'R8_JAR="\$\{STEPX_R8_JAR:-/home/[^"\n]+/Android/Sdk/cmdline-tools/latest/lib/r8\.jar\}"',
        'R8_JAR="${STEPX_R8_JAR:-${ANDROID_SDK_ROOT:-${ANDROID_HOME:?Set ANDROID_HOME}}/cmdline-tools/latest/lib/r8.jar}"',
        text,
    )
    if rel_path == "scripts/install-desktop.sh":
        text = text.replace(
            'cp -af "$ROOT/.tools/bin" "$ROOT/.tools/root" "$STEPX_INSTALL_DIR/tools/"',
            'cp -a "$ROOT/.tools/bin" "$ROOT/.tools/root" "$STEPX_INSTALL_DIR/tools/"',
        )
    elif rel_path == "scripts/play_protect_certification.py":
        text = re.sub(
            r'JDK_CANDIDATES = \("/home/[^"]+/\.local/opt/android-studio/jbr",\s*"/home/[^"]+/\.local/jdk-17"\)',
            'JDK_CANDIDATES = tuple(p for p in (os.environ.get("JAVA_HOME"), os.environ.get("STEPX_JDK")) if p)',
            text,
        )
    elif rel_path == "scripts/configure-phone-gui-model.py":
        if "import os, shutil" not in text:
            text = text.replace(
                "import json, subprocess, sys\n",
                "import json, os, shutil, subprocess, sys\n",
            )
        text = re.sub(
            r"ADB=\['/home/[^']+/Android/Sdk/platform-tools/adb','-s','ZYPZ261A[0-9A-Z]+'\]",
            "ADB=[os.environ.get('STEPX_ADB', shutil.which('adb') or 'adb'),'-s',os.environ['STEPX_SERIAL']]",
            text,
        )
    elif rel_path == "scripts/verify-gui-agent-lifecycle.py":
        if "import os, shutil" not in text:
            text = text.replace(
                "import base64, json, subprocess, time, uuid\n",
                "import base64, json, os, shutil, subprocess, time, uuid\n",
            )
        text = re.sub(
            r"\['/home/[^']+/Android/Sdk/platform-tools/adb','-s','ZYPZ261A[0-9A-Z]+'",
            "[os.environ.get('STEPX_ADB', shutil.which('adb') or 'adb'),'-s',os.environ['STEPX_SERIAL']",
            text,
        )
    elif rel_path == "scripts/verify-gui-controls.py":
        if "import os, shutil" not in text:
            text = text.replace(
                "import base64, json, subprocess, time\n",
                "import base64, json, os, shutil, subprocess, time\n",
            )
        text = re.sub(
            r"ADB='/home/[^']+/Android/Sdk/platform-tools/adb';SERIAL='ZYPZ261A[0-9A-Z]+'",
            "ADB=os.environ.get('STEPX_ADB', shutil.which('adb') or 'adb');SERIAL=os.environ['STEPX_SERIAL']",
            text,
        )
    return text


def should_skip_path(path: pathlib.Path) -> bool:
    for part in path.parts:
        if part in {"__pycache__", ".git", ".DS_Store", "target", ".runtime", ".build", "build", ".gradle"}:
            return True
    if path.suffix == ".pyc":
        return True
    return False


def fixture_placeholders(rel: str, text: str) -> str:
    """Deterministic public fixtures, preserving equality and test relationships.

    Device execution targets are fixed in source to explicit env vars. Remaining
    serials are parser/demo fixtures and never identify a public user's phone.
    Original developer evidence is untouched.
    """
    text = re.sub(r'ZYPZ261A[0-9A-Z]{8,}', lambda m: 'STEPXTEST' + hashlib.sha256(m[0].encode()).hexdigest()[:12].upper(), text)
    # Keep synthetic cross-platform fixtures and preserve the OS path family.
    # Match both the bare home input and a joined expected path consistently.
    fixture_users = {'stepx-user', 'example', 'macuser', 'user', 'test', 'testuser',
        'alice', 'bob', 'john', 'admin', 'blue', 'aloke', 'demo'}
    def public_home(match):
        family, user = match.group(1), match.group(2)
        return match[0] if user in fixture_users else f'/{family}/stepx-user'
    text = re.sub(r'''/(home|Users)/([A-Za-z0-9_.-]+)(?=/|["'\s]|$)''', public_home, text)
    # Known developer LAN endpoint fixtures only; retain private-address semantics.
    text = text.replace('10.0.1.177', '10.0.0.42').replace('10.0.1.200', '10.0.0.43')
    # Parser and UI scenario fixtures must not publish conversation names taken
    # from a developer's phone. Keep exact-match relationships across files.
    conversation_fixtures = {
        'd72ffe602380b4dd66888d02362a9f35d99993c209a606d72bfb55a7d8eeed52': '演示交流小组',
        'fa0f35b8a6923ddf5816bded2768a19df37ebab73243016e400c281701b0bf3f': '示例团队 • 项目讨论',
        'a03e6539e2aea5f48e276231ccdef3e0b9d602c1a854f594389063760d1c3eb6': '示例团队 · 项目讨论',
    }
    text = re.sub(r'(?:Builder\s+One[\s•·]*STEPX|STEPX)[\u3400-\u9fff]{2,16}',
                  lambda m: conversation_fixtures.get(hashlib.sha256(m[0].encode()).hexdigest(), m[0]), text)
    if rel == 'CHANGELOG.md':
        # Historical implementation notes quoted an actual @ mention. A neutral
        # example explains the behavior without publishing the conversation.
        text = re.sub(r'如 `[^`\n]*@[^`\n]*`', '如 `演示成员: @我 请确认明天的评审时间`', text)
    # Source fixtures exercise number normalization and recipient matching. Use
    # deterministic synthetic numbers while preserving country-code/separator
    # formatting, so both inputs and expected outputs still agree. Do not touch
    # author contact information explicitly retained in the client About panel.
    if pathlib.PurePosixPath(rel).suffix in {'.rs', '.java', '.kt', '.py'}:
        def public_phone(match):
            local = match.group('local')
            digits = re.sub(r'\D', '', local)
            if digits.startswith('1380000'):
                return match.group(0)
            replacement = '1380000' + str(int(hashlib.sha256(digits.encode()).hexdigest()[:8], 16) % 10000).zfill(4)
            cursor = iter(replacement)
            return match.group('prefix') + ''.join(next(cursor) if char.isdigit() else char for char in local)
        text = re.sub(r'(?<![0-9A-Za-z])(?P<prefix>(?:(?:\+?86|0086)[ \t]?)?)(?P<local>1[3-9](?:[ -]?\d){9})(?![0-9A-Za-z])', public_phone, text)
    return text


def collect_sync_plan() -> Dict[str, Tuple[bytes, int]]:
    """Build and validate the complete transformed plan before writing anything."""
    policy = manifest(ROOT)
    plan = {}
    paths = set(ALLOWLIST_FILES) | set(policy['public_scripts']) | set(policy['public_docs']) | set(policy.get('public_files', []))
    paths.update({'.gitignore', 'handoff/google-play-stepx/target.example.json'})
    for rel_dir in ALLOWLIST_DIRS:
        src_dir = ROOT / rel_dir
        if not src_dir.is_dir():
            raise FileNotFoundError(f'Missing allowlisted directory: {rel_dir}')
        for path in sorted(src_dir.rglob('*')):
            if path.is_file() and not should_skip_path(path):
                paths.add(path.relative_to(ROOT).as_posix())
    allowed_code_suffixes = {'.rs', '.toml', '.lock', '.java', '.kt', '.xml', '.json', '.srt',
        '.md', '.txt', '.py', '.sh', '.ps1', '.js', '.cjs', '.mjs', '.svg', '.rc', '.yml', '.yaml',
        '.dex', '.jar', '.apk', '.aar', '.so', '.ttf', '.otf', '.pro', '.gradle'}
    allowed_code_suffixes.update({'.png', '.jpg', '.jpeg', '.gif', '.webp', '.ico', '.mp4', '.h264'})
    for rel in sorted(paths):
        path = ROOT / rel
        if path.suffix and path.suffix.lower() not in allowed_code_suffixes and rel != '.gitignore':
            raise ValueError(f'{rel}: unknown file type in source allowlist')
        data = safe_read(ROOT, rel)
        try:
            text = data.decode('utf-8')
        except UnicodeDecodeError:
            pass
        else:
            if rel == 'desktop/Cargo.toml':
                text = rewrite_cargo_toml(text)
            if rel.startswith('scripts/'):
                text = desensitize_script(rel, text)
            # The policy/manifest contain literal scanner expressions and reviewed
            # media hashes; these are not fixture text and must remain byte exact.
            if rel not in {'scripts/public_release_policy.py', 'scripts/release-resources.json', 'scripts/sync-public-repo.py'}:
                text = fixture_placeholders(rel, text)
            data = text.encode('utf-8')
        issues = content_problems(rel, data, policy)
        if issues:
            raise ValueError('\n'.join(issues))
        plan[rel] = (data, stat.S_IMODE(path.stat().st_mode))
    plan['handoff/google-play-stepx/target.json'] = (generic_target(ROOT), 0o644)
    # Existing public documentation is retained. Remove developer machine
    # identifiers from its current tree without touching the original checkout,
    # author/contact details or Git history. Vendor parser fixtures are audited
    # separately so different users/paths continue to exercise distinct cases.
    for path in sorted(PUBLIC_ROOT.rglob('*')):
        if not path.is_file() or should_skip_path(path):
            continue
        rel = path.relative_to(PUBLIC_ROOT).as_posix()
        if rel in plan or rel in policy.get('public_removed_files', []) or rel.startswith(('vendor/', '.github/')):
            continue
        if path.suffix.lower() not in {'.md', '.txt', '.rs', '.py', '.sh', '.ps1', '.json', '.toml', '.js', '.cjs'}:
            continue
        data = safe_read(PUBLIC_ROOT, rel)
        try:
            text = data.decode('utf-8')
        except UnicodeDecodeError:
            continue
        updated = fixture_placeholders(rel, desensitize_script(rel, text)).encode('utf-8')
        if updated != data:
            issues = content_problems(rel, updated, policy)
            if issues:
                raise ValueError('\n'.join(issues))
            plan[rel] = (updated, stat.S_IMODE(path.stat().st_mode))
    return plan


def scan_bytes_for_secrets(label: str, data: bytes) -> List[str]:
    findings: List[str] = []
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return findings
    for pat_name, regex in SECRET_PATTERNS:
        for match in regex.finditer(text):
            line_no = text[: match.start()].count("\n") + 1
            findings.append(f"{label}:{line_no}: matched secret pattern '{pat_name}'")
    return findings


def scan_public_repo(plan: Dict[str, Tuple[bytes, int]]) -> List[str]:
    findings: List[str] = []

    # 1. Scan all staged payloads in plan
    for rel, (data, _) in sorted(plan.items()):
        ext = pathlib.Path(rel).suffix.lower()
        if ext in BINARY_EXTENSIONS:
            continue
        findings.extend(scan_bytes_for_secrets(f"[staged] {rel}", data))

    # 2. Audit every existing publication input not replaced by the plan, using
    # the same full path/content/media policy. The public checkout can retain
    # older media or ignored-but-tracked runtime data; a token-only scan misses
    # those. Git's file list avoids inspecting private untracked build outputs.
    if PUBLIC_ROOT.is_dir():
        import subprocess
        policy = manifest(ROOT)
        result = subprocess.run(['git', '-C', str(PUBLIC_ROOT), 'ls-files', '-z',
                                 '--cached', '--others', '--exclude-standard'],
                                check=True, stdout=subprocess.PIPE)
        removed = publication_removals(plan)
        for rel in sorted(set(result.stdout.decode('utf-8').split('\0')) - {''}):
            if rel in plan or rel in removed:
                continue
            path = PUBLIC_ROOT / rel
            if not path.exists() and not path.is_symlink():
                continue
            if path.is_symlink():
                findings.append(f'{rel}: source symlinks require explicit review')
                continue
            if path.is_file():
                findings.extend(content_problems(rel, path.read_bytes(), policy))

    return findings


def publication_removals(plan: Dict[str, Tuple[bytes, int]]) -> Set[str]:
    """Explicitly retired video outputs, only in the public mirror.

    Local historical recordings are never removed. This also prevents stale,
    superseded media from being silently republished by the existing Git tree.
    """
    removed = set(manifest(ROOT).get('public_removed_files', []))
    for rel in removed:
        if path_problems(rel) or not rel.startswith('docs/videos/') or rel in plan:
            raise ValueError(f'{rel}: invalid retired-public-media entry')
        dest = PUBLIC_ROOT / rel
        if dest.is_symlink() or (dest.exists() and not dest.is_file()):
            raise ValueError(f'{rel}: retired public media must be a regular file')
    return removed


def execute_sync(plan: Dict[str, Tuple[bytes, int]], check_only: bool) -> Tuple[int, int, int]:
    added = 0
    modified = 0
    unchanged = 0
    removed = publication_removals(plan)

    for rel in plan:
        dest = PUBLIC_ROOT / rel
        if any(p.is_symlink() for p in [dest, *dest.parents] if p != PUBLIC_ROOT.parent):
            raise ValueError(f'{rel}: publication destination contains a symlink')

    for rel, (data, mode) in sorted(plan.items()):
        dest = PUBLIC_ROOT / rel
        if not dest.exists():
            added += 1
            if not check_only:
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(data)
                os.chmod(dest, mode)
        else:
            existing = dest.read_bytes()
            if existing != data:
                modified += 1
                if not check_only:
                    dest.write_bytes(data)
                    os.chmod(dest, mode)
            else:
                unchanged += 1

    if not check_only and PUBLIC_ROOT.is_dir():
        for rel in sorted(removed):
            (PUBLIC_ROOT / rel).unlink(missing_ok=True)
        for pycache in list((PUBLIC_ROOT / "scripts").rglob("__pycache__")):
            if pycache.is_dir():
                shutil.rmtree(pycache, ignore_errors=True)

    return added, modified, unchanged


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Sync desensitized StepX Companion sources to releases/stepx-public."
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Dry-run verification and secret scan without modifying releases/stepx-public.",
    )
    args = parser.parse_args()

    if not PUBLIC_ROOT.is_dir():
        print(f"ERROR: Public repository directory not found: {PUBLIC_ROOT}", file=sys.stderr)
        return 1

    try:
        plan = collect_sync_plan()
    except Exception as exc:
        print(f"ERROR: Failed to build sync plan: {exc}", file=sys.stderr)
        return 1

    findings = scan_public_repo(plan)
    if findings:
        print('ERROR: Privacy preflight failed before any file was written:', file=sys.stderr)
        print('\n'.join(findings), file=sys.stderr)
        return 2
    added, modified, unchanged = execute_sync(plan, check_only=args.check)

    mode_str = "CHECK (dry-run)" if args.check else "SYNC"
    print(
        f"[{mode_str}] Staged {len(plan)} allowlisted files -> "
        f"added={added}, modified={modified}, unchanged={unchanged}"
    )
    print(
        "[REWRITE] Verified desktop/Cargo.toml license='AGPL-3.0-only' "
        "and '../vendor/' dependency paths."
    )

    if findings:
        print("ERROR: Secret pattern scan failed:", file=sys.stderr)
        for item in findings:
            print(f"  - {item}", file=sys.stderr)
        return 2

    print("[SECURITY] Preflight passed: private paths, credentials, device fixtures and reviewed media. Existing Git history is unchanged.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
