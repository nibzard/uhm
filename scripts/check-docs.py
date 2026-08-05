#!/usr/bin/env python3
"""Fail when current user-facing documentation drifts from release metadata or CLI help."""

from __future__ import annotations

import json
import pathlib
import re
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def fail(message: str) -> None:
    print(f"docs check: {message}", file=sys.stderr)
    raise SystemExit(1)


def cargo_version() -> str:
    match = re.search(r'^version = "([^"]+)"$', read("Cargo.toml"), re.MULTILINE)
    if not match:
        fail("Cargo.toml package version is missing")
    return match.group(1)


def check_versions(version: str) -> None:
    release = ".".join(version.split(".")[:2])
    required = {
        "README.md": [f"v{version} release", f"--tag v{version}"],
        "docs/README.md": [f"--tag v{version}"],
        "docs/_coverpage.md": [f"v{version} ·"],
        "docs/_sidebar.md": [f"releases/v{version}.md"],
        "docs/install.md": [f"release page](https://github.com/nibzard/uhm/releases/tag/v{version})"],
        "PRIVACY.md": [f'"release": "{release}"'],
        "docs/privacy.md": [f'"release": "{release}"'],
    }
    for path, needles in required.items():
        body = read(path)
        for needle in needles:
            if needle not in body:
                fail(f"{path} does not contain current release marker {needle!r}")
    if not (ROOT / f"docs/releases/v{version}.md").is_file():
        fail(f"docs/releases/v{version}.md is missing")


def cli_help() -> str:
    result = subprocess.run(
        ["cargo", "run", "--quiet", "--locked", "--bin", "uhm", "--", "--help"],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        fail(f"could not render CLI help: {result.stderr.strip()}")
    return result.stdout


def check_cli_reference(help_text: str) -> None:
    reference = read("docs/cli-reference.md")
    long_flags = sorted(set(re.findall(r"--[a-z][a-z-]*", help_text)))
    missing = [flag for flag in long_flags if flag not in reference]
    if missing:
        fail(f"docs/cli-reference.md omits help flags: {', '.join(missing)}")
    for usage in ("uhm doctor [all] [network|environment]", "--provider <openai|cerebras|deepseek>"):
        if usage not in help_text:
            fail(f"CLI help is missing expected v0.3 surface {usage!r}")
    if "uhm doctor [all] [network|environment]" not in read("README.md"):
        fail("README.md doctor synopsis differs from CLI help")


def check_provider_language() -> None:
    current_docs = [
        "README.md",
        "PRIVACY.md",
        "docs/README.md",
        "docs/behavior-contract.md",
        "docs/concepts.md",
        "docs/local-history.md",
        "docs/privacy.md",
        "docs/recovery.md",
    ]
    forbidden = (
        "terminal work goes to OpenAI",
        "Before any OpenAI request",
        "OpenAI receives the prompt",
        "never attached to an OpenAI request",
        "asks OpenAI for one reviewed",
        "serialized into an OpenAI request",
    )
    for path in current_docs:
        body = read(path)
        for phrase in forbidden:
            if phrase in body:
                fail(f"{path} contains stale provider-specific phrase {phrase!r}")

    privacy_requirements = (
        "## Provider requests",
        "selected, disclosed provider endpoint",
        "Cerebras",
    )
    for path in ("PRIVACY.md", "docs/privacy.md"):
        body = read(path)
        for phrase in privacy_requirements:
            if phrase not in body:
                fail(f"{path} is missing shared privacy claim {phrase!r}")


def check_manifest_status() -> None:
    manifest = json.loads(read("model-qualification-manifest.json"))
    docs = read("docs/model-selection.md") + read("docs/configuration.md")
    if not manifest.get("entries") and "no provider/model pair is currently qualified" not in docs.lower():
        fail("empty qualification manifest is not described as unavailable")


def check_local_links() -> None:
    paths = [ROOT / "README.md", ROOT / "PRIVACY.md", *(ROOT / "docs").rglob("*.md")]
    for path in paths:
        body = path.read_text(encoding="utf-8")
        for target in re.findall(r"\[[^\]]*\]\(([^)]+)\)", body):
            target = target.strip().strip("<>").split("#", 1)[0]
            if not target or target.startswith(("https://", "http://", "mailto:", "/")):
                continue
            resolved = (path.parent / target).resolve()
            if not resolved.exists():
                fail(f"{path.relative_to(ROOT)} links to missing local path {target!r}")


def check_diataxis_navigation() -> None:
    sidebar = read("docs/_sidebar.md")
    for section in ("Tutorials", "How-to guides", "Reference", "Explanation"):
        if f"**{section}**" not in sidebar:
            fail(f"docs/_sidebar.md is missing the {section!r} Diátaxis section")

    allowed = {"tutorial", "how-to", "reference", "explanation", "navigation", "project"}
    for target in re.findall(r"\[[^\]]*\]\(([^)]+\.md(?:#[^)]*)?)\)", sidebar):
        target = target.split("#", 1)[0]
        path = ROOT / "docs" / target
        match = re.search(r"<!--\s*diataxis:\s*([a-z-]+)\s*-->", path.read_text(encoding="utf-8"))
        if not match:
            fail(f"{path.relative_to(ROOT)} is in the sidebar without Diátaxis metadata")
        if match.group(1) not in allowed:
            fail(f"{path.relative_to(ROOT)} has unknown Diátaxis type {match.group(1)!r}")

    expected = {
        "Start here": {"tutorial", "how-to"},
        "Tutorials": {"tutorial"},
        "How-to guides": {"how-to"},
        "Reference": {"reference"},
        "Explanation": {"explanation"},
        "Project": {"navigation", "project"},
    }
    section = None
    for line in sidebar.splitlines():
        heading = re.match(r"^- \*\*(.+)\*\*$", line)
        if heading and heading.group(1) in expected:
            section = heading.group(1)
            continue
        link = re.search(r"\[[^\]]*\]\(([^)]+\.md)(?:#[^)]*)?\)", line)
        if not link or section is None:
            continue
        path = ROOT / "docs" / link.group(1)
        kind = re.search(
            r"<!--\s*diataxis:\s*([a-z-]+)\s*-->", path.read_text(encoding="utf-8")
        ).group(1)
        if kind not in expected[section]:
            fail(f"{path.relative_to(ROOT)} is {kind!r} but appears under {section!r}")


def main() -> None:
    version = cargo_version()
    check_versions(version)
    check_cli_reference(cli_help())
    check_provider_language()
    check_manifest_status()
    check_local_links()
    check_diataxis_navigation()
    print(f"docs check: v{version} documentation is synchronized")


if __name__ == "__main__":
    main()
