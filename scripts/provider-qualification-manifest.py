#!/usr/bin/env python3
"""Generate a strict runtime qualification manifest from one finalized reviewed holdout artifact."""

from __future__ import annotations

import argparse
import calendar
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from jsonschema import Draft202012Validator

import qualification_policy

ROOT = Path(__file__).resolve().parent.parent
SCHEMA = ROOT / "benchmark/schemas/run-event.schema.json"
CORPUS_SCHEMA = ROOT / "benchmark/schemas/corpus.schema.json"
POLICY = ROOT / "model-qualification-policy-v1.json"
COMMITMENT = ROOT / "model-qualification-holdout-v1.json"
HELPER = ROOT / "target/debug/uhm-bench-contract"
PROVIDER_HELPER = ROOT / "target/debug/uhm-provider-call"


def named_bundle_hash(directory: Path, pattern: str) -> str:
    digest = hashlib.sha256()
    for path in sorted(directory.glob(pattern)):
        digest.update(path.name.encode() + b"\0" + path.read_bytes())
    return digest.hexdigest()


def source_bundle_hash() -> str:
    digest = hashlib.sha256()
    paths = list((ROOT / "src").rglob("*.rs")) + list((ROOT / "assets/shell").glob("*"))
    for path in sorted(paths):
        digest.update(path.relative_to(ROOT).as_posix().encode() + b"\0" + path.read_bytes())
    return digest.hexdigest()


def helper(operation: str, input_text: str | None = None) -> dict:
    build = subprocess.run(["cargo", "build", "--quiet", "--bin", "uhm-bench-contract"],
                           cwd=ROOT, check=False)
    if build.returncode:
        raise ValueError("could not build the production qualification helper")
    process = subprocess.run([str(HELPER), operation], input=input_text, text=True,
                             capture_output=True, cwd=ROOT, check=False)
    if process.returncode:
        raise ValueError(f"qualification helper failed: {process.stderr[:400]}")
    return json.loads(process.stdout)


def load_artifact(path: Path) -> tuple[list[dict], dict, dict, dict]:
    validator = Draft202012Validator(json.loads(SCHEMA.read_text(encoding="utf-8")))
    events = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        try:
            event = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"invalid artifact JSON at line {number}: {error}") from error
        validator.validate(event)
        events.append(event)
    if not events or events[-1]["type"] != "run_completed":
        raise ValueError("qualification artifact is not finalized")
    if [event["sequence"] for event in events] != list(range(len(events))):
        raise ValueError("qualification artifact has a non-contiguous event sequence")
    fingerprints = {event["run_fingerprint"] for event in events}
    if len(fingerprints) != 1:
        raise ValueError("qualification artifact mixes run fingerprints")
    started = [event["payload"] for event in events if event["type"] == "run_started"]
    summaries = [event["payload"] for event in events if event["type"] == "summary_computed"]
    completed = [event["payload"] for event in events if event["type"] == "run_completed"]
    if len(started) != 1 or len(summaries) != 1 or len(completed) != 1:
        raise ValueError("qualification artifact must contain one start, summary, and completion")
    return events, started[0], summaries[0], completed[0]


def recompute_qualification(events: list[dict], started: dict, summary: dict,
                            completed: dict, fingerprint: str) -> dict:
    projection = started["fingerprint_projection"]
    calculated_fingerprint = hashlib.sha256(json.dumps(
        projection, sort_keys=True, separators=(",", ":")
    ).encode()).hexdigest()
    if calculated_fingerprint != fingerprint:
        raise ValueError("artifact run fingerprint does not match its frozen inputs")
    if projection.get("profile") != "qualification" \
            or projection.get("program_profile") != "first-shot" \
            or projection.get("task_ids") or projection.get("task_count") is not None:
        raise ValueError("artifact was not an unfiltered first-shot qualification run")

    corpus_path = Path(started["corpus"])
    if not corpus_path.is_absolute():
        corpus_path = ROOT / corpus_path
    corpus = json.loads(corpus_path.read_text(encoding="utf-8"))
    Draft202012Validator(json.loads(CORPUS_SCHEMA.read_text(encoding="utf-8"))).validate(corpus)
    qualification_policy.validate_holdout(corpus, corpus_path, COMMITMENT, POLICY)
    if qualification_policy.sha256_file(corpus_path) != projection.get("corpus_sha256"):
        raise ValueError("artifact corpus differs from its fingerprint projection")

    records = [event["payload"]["record"] for event in events
               if event["type"] == "judgment_completed"]
    candidate_values = projection.get("candidates", [])
    if not isinstance(candidate_values, list) or len(candidate_values) < 2 \
            or any(not isinstance(value, list) or len(value) != 2
                   or not all(isinstance(item, str) and item for item in value)
                   for value in candidate_values):
        raise ValueError("artifact has an invalid candidate projection")
    candidates = [tuple(value) for value in candidate_values]
    trials = qualification_policy.load_policy(POLICY)["trials_per_class"]
    expected_records = len(corpus["tasks"]) * len(candidates) * trials
    keys = {(record["task_id"], record["trial"], record["candidate"]["provider"],
             record["candidate"]["model"]) for record in records}
    if started.get("task_count") != len(corpus["tasks"]) \
            or summary.get("task_count") != len(corpus["tasks"]) \
            or completed.get("record_count") != expected_records \
            or len(records) != expected_records or len(keys) != expected_records:
        raise ValueError("artifact does not contain exactly one judged record per qualification job")
    calibration = [event["payload"] for event in events
                   if event["type"] == "calibration_completed"]
    audit = (summary.get("independent_audit") or {}).get("metadata") or {}
    recomputed = qualification_policy.evaluate(
        records, corpus["tasks"], candidates, calibration, audit,
        qualification_policy.load_policy(POLICY), int(projection["seed"]),
    )
    if recomputed != summary.get("qualification"):
        raise ValueError("qualification summary does not match deterministic recomputation")
    return recomputed


def atomic_write(path: Path, value: dict, overwrite: bool) -> None:
    if path.exists() and not overwrite:
        raise ValueError(f"output already exists: {path}; pass --overwrite to replace it")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(str(path) + ".tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        output.write(json.dumps(value, indent=2, sort_keys=True) + "\n")
        output.flush(); os.fsync(output.fileno())
    os.replace(temporary, path)
    directory = os.open(path.parent, os.O_RDONLY)
    try: os.fsync(directory)
    finally: os.close(directory)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--overwrite", action="store_true")
    args = parser.parse_args()

    events, started, summary, completed = load_artifact(args.artifact)
    fingerprint = events[0]["run_fingerprint"]
    qualification = summary.get("qualification")
    audit = summary.get("independent_audit") or {}
    commitment = json.loads(COMMITMENT.read_text(encoding="utf-8"))
    if not qualification or audit.get("status") != "complete" or not audit.get("metadata"):
        raise ValueError("artifact lacks completed structured qualification and independent audit")
    if summary.get("qualification_commitment") != commitment or commitment.get("status") != "sealed":
        raise ValueError("artifact does not match the current sealed holdout commitment")
    build = subprocess.run(
        ["cargo", "build", "--quiet", "--bin", "uhm-bench-contract",
         "--bin", "uhm-provider-call"], cwd=ROOT, check=False,
    )
    if build.returncode:
        raise ValueError("could not build current qualification helpers")
    projection = started["fingerprint_projection"]
    expected_sha = {
        "runner_sha256": qualification_policy.sha256_file(ROOT / "scripts/provider-bakeoff.py"),
        "helper_sha256": qualification_policy.sha256_file(HELPER),
        "provider_helper_sha256": qualification_policy.sha256_file(PROVIDER_HELPER),
        "qualification_policy_sha256": qualification_policy.sha256_file(POLICY),
        "qualification_manifest_sha256": qualification_policy.sha256_file(
            ROOT / "model-qualification-manifest.json"),
        "qualification_commitment_sha256": qualification_policy.sha256_file(COMMITMENT),
        "qualification_tooling_sha256": hashlib.sha256(b"".join(
            (ROOT / path).read_bytes() for path in (
                "scripts/qualification_policy.py",
                "scripts/provider-qualification-manifest.py",
                "scripts/seal-qualification-holdout.py",
            )
        )).hexdigest(),
        "schemas_sha256": named_bundle_hash(ROOT / "benchmark/schemas", "*.json"),
    }
    for key, expected in expected_sha.items():
        if projection.get(key) != expected:
            raise ValueError(f"artifact compatibility input changed: {key}")
    worker_manifest = started.get("worker_manifest") or {}
    worker_projection = {key: value for key, value in worker_manifest.items()
                         if key not in {"built_at_utc", "identity_sha256"}}
    worker_identity = hashlib.sha256(json.dumps(
        worker_projection, sort_keys=True, separators=(",", ":")
    ).encode()).hexdigest()
    if worker_identity != worker_manifest.get("identity_sha256") \
            or worker_identity != projection.get("worker_identity"):
        raise ValueError("artifact worker identity is inconsistent")
    worker_hashes = worker_manifest.get("hashes") or {}
    expected_worker_hashes = {
        "fixture_bundle_and_oracle": commitment["corpus_sha256"],
        "production_execution_sources": source_bundle_hash(),
        "worker_source": qualification_policy.sha256_file(ROOT / "benchmark/worker/worker.py"),
        "dockerfile": qualification_policy.sha256_file(ROOT / "benchmark/docker/Dockerfile"),
        "schemas": named_bundle_hash(ROOT / "benchmark/schemas", "*.json"),
        "tool_manifest_source": qualification_policy.sha256_file(
            ROOT / "benchmark/worker/tool_manifest.py"),
    }
    mismatches = sorted(key for key, value in expected_worker_hashes.items()
                        if worker_hashes.get(key) != value)
    if mismatches:
        raise ValueError(f"artifact worker source changed: {mismatches}")
    qualification = recompute_qualification(events, started, summary, completed, fingerprint)
    context = helper("qualification-context")
    if context.get("corpus_hash") != commitment["corpus_sha256"]:
        raise ValueError("compiled runtime does not contain the artifact's holdout commitment")

    report_process = subprocess.run(
        [sys.executable, str(ROOT / "scripts/provider-benchmark-report.py"), str(args.artifact)],
        cwd=ROOT, check=False,
    )
    if report_process.returncode:
        raise ValueError("could not regenerate the report from the finalized artifact")
    report_path = Path(str(args.artifact) + ".summary.json")
    artifact_hash = qualification_policy.sha256_file(args.artifact)
    report_hash = qualification_policy.sha256_file(report_path)
    try:
        evaluated_at = int(calendar.timegm(time.strptime(completed["completed_utc"], "%Y-%m-%dT%H:%M:%SZ")))
    except ValueError as error:
        raise ValueError("artifact completion timestamp is invalid") from error
    reviewer = audit["metadata"]["reviewer"]
    entries = []
    for profile in qualification["profiles"]:
        if not profile.get("qualified"):
            continue
        candidate = profile["candidate"]
        provider = candidate["provider"]
        evidence = dict(profile["evidence"])
        evidence["artifact_hash"] = artifact_hash
        evidence["report_hash"] = report_hash
        evidence["reviewer_disposition"] = f"qualified:{reviewer}"
        # Runtime intentionally accepts only the categorical disposition; the
        # reviewer identity stays in the private artifact/report provenance.
        runtime_evidence = dict(evidence)
        runtime_evidence["reviewer_disposition"] = "qualified"
        entries.append({
            "selected": bool(profile.get("selected")),
            "provider": provider,
            "api_family": profile["api_family"],
            "endpoint": context["endpoints"][provider],
            "model": candidate["model"],
            "resolved_model": profile["resolved_model"],
            "resolved_fingerprint": profile["resolved_fingerprint"],
            "prompt_version": context["prompt_version"],
            "action_schema_version": context["action_schema_version"],
            "program_contract": context["program_contract"],
            "context_policy_version": context["context_policy_version"],
            "adapter_contract_version": context["adapter_contract_version"],
            "selection_policy_version": context["selection_policy_version"],
            "corpus_hash": context["corpus_hash"],
            "worker_hash": context["worker_hash"],
            "runner_hash": context["runner_hash"],
            "policy_hash": context["policy_hash"],
            "request_class": profile["request_class"],
            "permitted_action_types": profile["permitted_action_types"],
            "evidence": runtime_evidence,
            "evaluated_at_unix": evaluated_at,
            "reviewed": True,
            "qualified": True,
        })
    if not entries:
        raise ValueError("no request class passed every frozen qualification gate")
    manifest = {
        "version": context["evidence_manifest_version"],
        "policy_version": context["qualification_policy_version"],
        "policy_hash": context["policy_hash"],
        "entries": sorted(entries, key=lambda item: (
            json.dumps(item["request_class"], sort_keys=True), not item["selected"],
            item["provider"], item["model"])),
    }
    encoded = json.dumps(manifest, sort_keys=True, separators=(",", ":"))
    validation = helper("validate-qualification-manifest", encoded)
    if validation.get("valid") is not True:
        raise ValueError(f"runtime rejected generated manifest: {validation.get('message')}")
    atomic_write(args.output, manifest, args.overwrite)
    print(args.output)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError) as error:
        print(f"provider-qualification-manifest: {error}", file=sys.stderr)
        raise SystemExit(2)
