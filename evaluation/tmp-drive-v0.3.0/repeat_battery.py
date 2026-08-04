#!/usr/bin/env python3
"""Run the contract-aware live battery repeatedly in fresh tmpfs sandboxes."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile


HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent


def command_output(argv: list[str], cwd: Path | None = None) -> str:
    process = subprocess.run(argv, cwd=cwd, text=True, capture_output=True, check=False)
    return (process.stdout or process.stderr).strip()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/uhm")
    parser.add_argument("--provider", default="openai")
    parser.add_argument("--model", default="gpt-5.6-terra")
    parser.add_argument("--reasoning-effort", default="low")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f"binary does not exist: {binary}")

    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    output = (args.output or REPO / "target" / "tmp-drive-eval" / stamp).resolve()
    output.mkdir(parents=True, exist_ok=False)
    metadata = {
        "schema_version": 1,
        "started_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "repeats": args.repeats,
        "provider": args.provider,
        "requested_model": args.model,
        "reasoning_effort": args.reasoning_effort,
        "temperature": None,
        "streaming": True,
        "context": "standard",
        "fresh": True,
        "telemetry": False,
        "binary": str(binary),
        "binary_sha256": sha256(binary),
        "uhm_version": command_output([str(binary), "--version"]),
        "git_commit": command_output(["git", "rev-parse", "HEAD"], REPO),
        "git_dirty": bool(command_output(["git", "status", "--porcelain"], REPO)),
        "platform": platform.platform(),
        "python": platform.python_version(),
    }
    (output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")

    all_results: list[dict] = []
    for attempt in range(1, args.repeats + 1):
        destination = output / f"run-{attempt}"
        with tempfile.TemporaryDirectory(prefix="uhm-eval-", dir="/dev/shm") as temporary:
            sandbox = Path(temporary)
            env = os.environ.copy()
            env.update(
                UHM_EVAL_ROOT=str(sandbox),
                UHM_EVAL_BINARY=str(binary),
                UHM_EVAL_PROVIDER=args.provider,
                UHM_EVAL_MODEL=args.model,
                UHM_EVAL_BATTERY=str(HERE / "battery-contract.json"),
                UHM_REASONING_EFFORT=args.reasoning_effort,
            )
            setup = subprocess.run(["python3", str(HERE / "setup_corpus.py")], env=env, check=False)
            if setup.returncode:
                return setup.returncode
            config_dir = sandbox / "config" / "uhm"
            config_dir.mkdir(parents=True, exist_ok=True)
            (config_dir / "config.yaml").write_text(
                f"provider: {args.provider}\n"
                f"model: {args.model}\n"
                f"reasoning_effort: {args.reasoning_effort}\n"
            )
            run = subprocess.run(
                ["python3", str(HERE / "run_battery.py")],
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )
            destination.mkdir()
            (destination / "run.log").write_text(run.stdout + run.stderr)
            for name in ["expected.json", "results.jsonl", "results_all.json"]:
                shutil.copy2(sandbox / name, destination / name)
            shutil.copytree(sandbox / "out", destination / "out")
            records = json.loads((sandbox / "results_all.json").read_text())
            for record in records:
                record["attempt"] = attempt
                all_results.append(record)
            if run.returncode:
                return run.returncode

    passed = sum(record["verdict"] == "PASS" for record in all_results)
    summary = {
        "attempted": len(all_results),
        "passed": passed,
        "pass_rate": passed / len(all_results),
        "consistent_tasks": sum(
            all(
                record["verdict"] == "PASS"
                for record in all_results
                if record["id"] == task_id
            )
            for task_id in sorted({record["id"] for record in all_results})
        ),
        "task_count": len({record["id"] for record in all_results}),
    }
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (output / "results.jsonl").write_text(
        "".join(json.dumps(record, sort_keys=True) + "\n" for record in all_results)
    )
    print(json.dumps({"output": str(output), **summary}, indent=2))
    return 0 if passed == len(all_results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
