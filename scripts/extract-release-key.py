#!/usr/bin/env python3
# Resolve the compiled-in RELEASE_PUBLIC_KEY from src/update.rs.
#
# release.yml calls this to cross-check the produced SHA256SUMS.minisig against
# the exact public key every installed binary pins, before publishing. The
# RELEASE_PUBLIC_KEY const may name another const (the placeholder indirection),
# so the chain is resolved recursively until it bottoms out at a string literal.
# Prints the key on stdout; exits non-zero if the chain is malformed.
import re
import sys
from pathlib import Path

SOURCE = Path(__file__).resolve().parents[1] / "src" / "update.rs"
ROOT_NAME = "RELEASE_PUBLIC_KEY"
_PATTERN = r"^const {name}: &str = (.+?);\s*$"


def resolve(name, src):
    match = re.search(_PATTERN.format(name=re.escape(name)), src, re.MULTILINE)
    if not match:
        sys.exit(f"could not find const {name} in src/update.rs")
    rhs = match.group(1).strip()
    if rhs.startswith('"') and rhs.endswith('"'):
        return rhs[1:-1]
    return resolve(rhs, src)


def main():
    print(resolve(ROOT_NAME, SOURCE.read_text()))


if __name__ == "__main__":
    main()
