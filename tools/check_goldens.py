#!/usr/bin/env python3
"""Protect checked-in golden files from accidental updates.

``LOI_BLESS=1`` is an intentional maintenance mode for regenerating expected
diagnostics. CI must never run in that mode, and the golden files must be clean
in the working tree so a blessed change cannot pass unnoticed.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PATTERNS = ["tests/errors/*.expected", "tests/**/*.golden"]


def main() -> int:
    if os.environ.get("LOI_BLESS") == "1":
        print("LOI_BLESS=1 is not allowed in CI; commit regenerated goldens instead", file=sys.stderr)
        return 1

    status = subprocess.run(
        ["git", "status", "--porcelain", "--", *PATTERNS],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if status.returncode != 0:
        print(status.stderr.strip() or "git status failed", file=sys.stderr)
        return status.returncode

    dirty = [line for line in status.stdout.splitlines() if line.strip()]
    if dirty:
        print("golden files are modified; regenerate and commit them explicitly:", file=sys.stderr)
        for line in dirty:
            print(f"  {line}", file=sys.stderr)
        return 1

    print("golden files are clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
