#!/usr/bin/env python3
"""Write the benchmark image's content-addressed runtime manifest as JSON."""

from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess

names = ["bash", "find", "grep", "sort", "head", "git", "python3", "jq", "rg",
         "file", "tar", "gzip", "xz", "zip", "unzip", "ps", "ip", "sha256sum",
         "awk", "cp", "mkdir", "mv", "rm", "touch", "uniq", "wc"]
tools = {}
for name in names:
    resolved = shutil.which(name)
    version = None
    if resolved:
        for flag in ("--version", "-V"):
            process = subprocess.run([resolved, flag], text=True, capture_output=True, check=False)
            line = (process.stdout or process.stderr).splitlines()
            if line:
                version = line[0][:300]
                break
    tools[name] = {"path": resolved, "version": version}

def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

python_path = shutil.which("python3")
python_version = subprocess.run([python_path, "--version"], text=True, capture_output=True, check=True).stdout.strip()
description = subprocess.run(["/opt/uhm-bench/uhm-bench-contract", "describe"], text=True, capture_output=True, check=True).stdout.encode()
schema_hash = hashlib.sha256()
for path in sorted(Path("/opt/uhm-bench/schemas").glob("*.json")):
    schema_hash.update(path.name.encode() + b"\0" + path.read_bytes())
manifest = {
    "manifest_version": 1,
    "worker_contract_version": 2,
    "architecture": platform.machine(),
    "base_image_digest": "sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818",
    "python": {"path": python_path, "version": python_version.removeprefix("Python "), "isolated_no_site": True},
    "shell": shutil.which("bash"),
    "tools": tools,
    "hashes": {
        "fixture_bundle_and_oracle": os.environ["UHM_BENCH_CORPUS_SHA256"],
        "production_execution_sources": os.environ["UHM_BENCH_RUST_SOURCE_SHA256"],
        "worker_source": sha256("/opt/uhm-bench/worker.py"),
        "dockerfile": sha256("/opt/uhm-bench/Dockerfile"),
        "schemas": schema_hash.hexdigest(),
        "canonical_action_description": hashlib.sha256(description).hexdigest(),
        "benchmark_helper_binary": sha256("/opt/uhm-bench/uhm-bench-contract"),
        "benchmark_execution_binary": sha256("/opt/uhm-bench/uhm-bench-exec"),
        "tool_manifest_source": sha256("/opt/uhm-bench/tool_manifest.py"),
    },
    "built_at_utc": datetime.now(timezone.utc).isoformat(),
}
projection = {key: value for key, value in manifest.items() if key not in {"built_at_utc", "identity_sha256"}}
manifest["identity_sha256"] = hashlib.sha256(json.dumps(projection, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
print(json.dumps(manifest, sort_keys=True, separators=(",", ":")))
