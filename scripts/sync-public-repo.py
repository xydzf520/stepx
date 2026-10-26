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
import os
import pathlib
import re
import shutil
import stat
import sys
from typing import Dict, List, Set, Tuple

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
        if part in {"__pycache__", ".git", ".DS_Store", "target", ".runtime"}:
            return True
    if path.suffix == ".pyc":
        return True
    return False


def collect_sync_plan() -> Dict[str, Tuple[bytes, int]]:
    """Return mapping of relative path -> (transformed_bytes, file_mode)."""
    plan: Dict[str, Tuple[bytes, int]] = {}

    for rel_dir in ALLOWLIST_DIRS:
        src_dir = ROOT / rel_dir
        if not src_dir.is_dir():
            raise FileNotFoundError(f"Missing allowlisted directory: {src_dir}")
        for path in sorted(src_dir.rglob("*")):
            if not path.is_file() or should_skip_path(path):
                continue
            rel = path.relative_to(ROOT).as_posix()
            plan[rel] = (path.read_bytes(), stat.S_IMODE(path.stat().st_mode))

    for rel_file in ALLOWLIST_FILES:
        src_file = ROOT / rel_file
        if not src_file.is_file():
            raise FileNotFoundError(f"Missing allowlisted file: {src_file}")
        mode = stat.S_IMODE(src_file.stat().st_mode)
        if rel_file == "desktop/Cargo.toml":
            rewritten = rewrite_cargo_toml(src_file.read_text(encoding="utf-8"))
            if 'license = "AGPL-3.0-only"' not in rewritten:
                raise RuntimeError("Cargo.toml license rewrite failed")
            if "../vendor/facloud_ui" not in rewritten or "../vendor/markdown_parser" not in rewritten:
                raise RuntimeError("Cargo.toml vendor path rewrite failed")
            plan[rel_file] = (rewritten.encode("utf-8"), mode)
        else:
            plan[rel_file] = (src_file.read_bytes(), mode)

    # Collect scripts/
    scripts_dir = ROOT / "scripts"
    for path in sorted(scripts_dir.rglob("*")):
        if not path.is_file() or should_skip_path(path):
            continue
        rel_in_scripts = path.relative_to(scripts_dir).as_posix()
        if rel_in_scripts in EXCLUDED_SCRIPTS:
            continue
        rel = path.relative_to(ROOT).as_posix()
        mode = stat.S_IMODE(path.stat().st_mode)
        if path.suffix in {".py", ".sh", ".ps1"}:
            text = path.read_text(encoding="utf-8")
            text = desensitize_script(rel, text)
            plan[rel] = (text.encode("utf-8"), mode)
        else:
            plan[rel] = (path.read_bytes(), mode)

    # Collect docs/ (excluding internal design/ and device-specific diagnosis notes)
    docs_dir = ROOT / "docs"
    for path in sorted(docs_dir.rglob("*")):
        if not path.is_file() or should_skip_path(path):
            continue
        rel_in_docs = path.relative_to(docs_dir).as_posix()
        if rel_in_docs.startswith("design/") or rel_in_docs in EXCLUDED_DOCS:
            continue
        rel = path.relative_to(ROOT).as_posix()
        mode = stat.S_IMODE(path.stat().st_mode)
        plan[rel] = (path.read_bytes(), mode)

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

    # 2. Scan existing files in releases/stepx-public
    if PUBLIC_ROOT.is_dir():
        for path in sorted(PUBLIC_ROOT.rglob("*")):
            if not path.is_file() or should_skip_path(path):
                continue
            if path.suffix.lower() in BINARY_EXTENSIONS:
                continue
            rel = path.relative_to(PUBLIC_ROOT).as_posix()
            findings.extend(scan_bytes_for_secrets(rel, path.read_bytes()))

    return findings


def execute_sync(plan: Dict[str, Tuple[bytes, int]], check_only: bool) -> Tuple[int, int, int]:
    added = 0
    modified = 0
    unchanged = 0

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

    added, modified, unchanged = execute_sync(plan, check_only=args.check)
    findings = scan_public_repo(plan)

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

    print("[SECURITY] Secret scan passed (0 matches for ghp_, sk-, PRIVATE KEY).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
