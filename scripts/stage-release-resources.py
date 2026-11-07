#!/usr/bin/env python3
"""Copy reviewed application resources without touching user state or old files."""
import argparse
import hashlib
import json
import os
from pathlib import Path
from public_release_policy import runtime_plan


def stage(source: Path, destination: Path, check: bool = False) -> int:
    source = source.resolve()
    plan = runtime_plan(source)  # all validation happens before the first write
    if check:
        print(f'Release resources validated: {len(plan)} files')
        return len(plan)
    destination = destination.resolve()
    for rel in plan:
        path = destination / rel
        current = path
        while current != destination.parent:
            if current.is_symlink():
                raise ValueError(f'{rel}: destination contains a symlink')
            current = current.parent
    for rel, data in plan.items():
        path = destination / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        temp = path.with_name(path.name + '.release-new')
        temp.write_bytes(data)
        src = source / rel
        os.chmod(temp, src.stat().st_mode & 0o777 if src.exists() else 0o644)
        temp.replace(path)
    digest = {rel: hashlib.sha256(data).hexdigest() for rel, data in sorted(plan.items())}
    (destination / 'release-resources.sha256.json').write_text(json.dumps(digest, indent=2) + '\n')
    print(f'Staged {len(plan)} reviewed resources; no user data was copied or removed')
    return len(plan)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    stage(args.source, args.destination, args.check)
