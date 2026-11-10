#!/usr/bin/env python3
"""Mirror the repository examples into the website.

The site and playground link to real example files, so ``website/examples/``
must stay byte-identical to ``examples/``. Run this after editing an example:

    python3 tools/sync_examples.py

Use ``--check`` in CI or before committing to fail when the mirror is stale.
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "examples"
MIRROR = ROOT / "website" / "examples"


def sources() -> dict[str, Path]:
    return {path.name: path for path in SOURCE.glob("*.loi") if path.is_file()}


def stale() -> list[str]:
    problems: list[str] = []
    expected = sources()
    if not MIRROR.is_dir():
        return ["website/examples is missing"]
    for name, path in expected.items():
        target = MIRROR / name
        if not target.is_file():
            problems.append(f"missing: website/examples/{name}")
        elif target.read_bytes() != path.read_bytes():
            problems.append(f"differs: website/examples/{name}")
    for target in MIRROR.glob("*.loi"):
        if target.name not in expected:
            problems.append(f"extra: website/examples/{target.name}")
    return problems


def sync() -> None:
    MIRROR.mkdir(parents=True, exist_ok=True)
    expected = sources()
    for target in MIRROR.glob("*.loi"):
        if target.name not in expected:
            target.unlink()
    for name, path in expected.items():
        shutil.copyfile(path, MIRROR / name)
    print(f"synced {len(expected)} example(s) into {MIRROR.relative_to(ROOT)}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="report differences instead of copying (exit 1 when stale)",
    )
    args = parser.parse_args()
    if args.check:
        problems = stale()
        if problems:
            for problem in problems:
                print(problem, file=sys.stderr)
            print("website/examples is stale; run tools/sync_examples.py", file=sys.stderr)
            return 1
        print("website/examples is in sync")
        return 0
    sync()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
