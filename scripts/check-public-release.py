#!/usr/bin/env python3
"""Fail before publishing an installed bundle containing private local data."""
import argparse
from pathlib import Path
from public_release_policy import audit_bundle, audit_source

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path, nargs='?', default=Path(__file__).resolve().parents[1])
    parser.add_argument('--source', action='store_true', help='Check public Git inputs rather than an installed bundle')
    args = parser.parse_args()
    issues = (audit_source if args.source else audit_bundle)(args.root.resolve())
    if issues:
        print('Release privacy check failed (values intentionally omitted):')
        print('\n'.join(issues))
        raise SystemExit(1)
    print('Release privacy check passed; reviewed resources and no private-data paths')
