#!/usr/bin/env python3
"""Fail on silent skips in a captured Cargo test log.

Portable fixtures mean tool absence is the main legitimate reason a test can
skip. Those skips are recorded in the log; this gate makes them visible and
turns them into failures unless ``LOI_ALLOW_SKIPS=1`` is set deliberately.
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path

SKIP_LINE = re.compile(r"^\s*(?:skipping|SKIP)\b", re.IGNORECASE)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: check_test_output.py <test-log>", file=sys.stderr)
        return 2

    lines = Path(sys.argv[1]).read_text(errors="replace").splitlines()
    skipped = [line.strip() for line in lines if SKIP_LINE.match(line)]
    if not skipped:
        print("no skipped tests detected")
        return 0

    if os.environ.get("LOI_ALLOW_SKIPS") == "1":
        print(f"{len(skipped)} skipped test(s) explicitly allowed:")
        for line in skipped:
            print(f"  {line}")
        return 0

    print(f"{len(skipped)} skipped test(s) detected; set LOI_ALLOW_SKIPS=1 to allow:", file=sys.stderr)
    for line in skipped:
        print(f"  {line}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
