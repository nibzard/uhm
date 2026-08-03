#!/usr/bin/env python3
"""Validate and commit to an independently authored holdout before any candidate results are revealed."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

from jsonschema import Draft202012Validator

import qualification_policy

ROOT = Path(__file__).resolve().parent.parent
POLICY = ROOT / "model-qualification-policy-v1.json"
CORPUS_SCHEMA = ROOT / "benchmark/schemas/corpus.schema.json"
CONTRACT_HELPER = ROOT / "target/debug/uhm-bench-contract"


def validate_reference_bundle(corpus: dict, bundle_path: Path) -> None:
    """Require exact corpus coverage and production-valid reference actions."""
    bundle = json.loads(bundle_path.read_text(encoding="utf-8"))
    if set(bundle) != {"version", "action_schema_version", "program_contract", "tasks"} \
            or bundle["version"] != 4 \
            or bundle["action_schema_version"] != corpus["action_schema_version"] \
            or bundle["program_contract"] != "uhm_helper_v1" \
            or not isinstance(bundle["tasks"], list):
        raise ValueError("schema-v4 reference action bundle has invalid provenance")
    if any(not isinstance(item, dict) or set(item) != {
            "id", "reference_actions", "negative_actions"} for item in bundle["tasks"]):
        raise ValueError("reference action bundle has an invalid task envelope")
    bundled = {item["id"]: item for item in bundle["tasks"]}
    if len(bundled) != len(bundle["tasks"]) or set(bundled) != {
            task["id"] for task in corpus["tasks"]}:
        raise ValueError("reference action bundle must exactly cover holdout task IDs")

    build = subprocess.run(
        ["cargo", "build", "--quiet", "--bin", "uhm-bench-contract"],
        cwd=ROOT, check=False,
    )
    if build.returncode:
        raise ValueError("could not build the production action contract helper")
    for task in corpus["tasks"]:
        item = bundled[task["id"]]
        if item["reference_actions"] != task["reference_actions"] \
                or item["negative_actions"] != task["negative_actions"]:
            raise ValueError(f"task {task['id']} differs from the locked reference bundle")
        for action in item["reference_actions"] + item["negative_actions"]:
            process = subprocess.run(
                [str(CONTRACT_HELPER), "validate"],
                input=json.dumps(action, separators=(",", ":")), text=True,
                capture_output=True, cwd=ROOT, check=False,
            )
            if process.returncode:
                raise ValueError(f"production action validation failed for {task['id']}")
            result = json.loads(process.stdout)
            if not result.get("valid"):
                raise ValueError(f"invalid reference action for {task['id']}: {result.get('rejection')}")
        for action in item["reference_actions"]:
            request = dict(action)
            request["piped_input_present"] = task["fixture"]["stdin"] is not None
            process = subprocess.run(
                [str(CONTRACT_HELPER), "preflight"],
                input=json.dumps(request, separators=(",", ":")), text=True,
                capture_output=True, cwd=ROOT, check=False,
            )
            if process.returncode:
                raise ValueError(f"production preflight failed for {task['id']}")
            result = json.loads(process.stdout)
            if not result.get("valid") or any(
                    item.get("severity") == "availability"
                    for item in result.get("diagnostics", [])):
                raise ValueError(f"reference action failed production preflight: {task['id']}")


def write_atomic(path: Path, value: dict, overwrite: bool) -> None:
    if path.exists() and not overwrite:
        raise ValueError(f"output exists: {path}; pass --overwrite after review")
    temporary = Path(str(path) + ".tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        output.write(json.dumps(value, indent=2, sort_keys=True) + "\n")
        output.flush(); os.fsync(output.fileno())
    os.replace(temporary, path)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--reviewer", required=True,
                        help="independent reviewer identity recorded in the commitment")
    parser.add_argument("--sealed-at-utc", required=True,
                        help="reviewed UTC timestamp, e.g. 2026-08-04T12:00:00Z")
    parser.add_argument("--output", type=Path, default=ROOT / "model-qualification-holdout-v1.json")
    parser.add_argument("--overwrite", action="store_true")
    args = parser.parse_args()
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    Draft202012Validator(json.loads(CORPUS_SCHEMA.read_text(encoding="utf-8"))).validate(corpus)
    if not args.reviewer.strip() or not args.sealed_at_utc.endswith("Z"):
        raise ValueError("reviewer and an explicit UTC seal timestamp are required")
    if any(task.get("split") != "holdout" for task in corpus["tasks"]):
        raise ValueError("every qualification task must be marked holdout")
    if len({task["id"] for task in corpus["tasks"]}) != len(corpus["tasks"]):
        raise ValueError("holdout task IDs must be unique")
    if corpus["task_count"] != len(corpus["tasks"]):
        raise ValueError("holdout task_count does not match its tasks")
    if corpus["family_count"] != len({task["family_id"] for task in corpus["tasks"]}):
        raise ValueError("holdout family_count does not match its tasks")
    route_counts = {route: sum(task["mode"] == route for task in corpus["tasks"])
                    for route in corpus["route_counts"]}
    if route_counts != corpus["route_counts"] or sum(route_counts.values()) != len(corpus["tasks"]):
        raise ValueError("holdout route_counts do not match its tasks")
    bundle = args.corpus.parent / corpus["reference_bundle"]
    if not bundle.is_file():
        raise ValueError("holdout reference bundle is missing")
    validate_reference_bundle(corpus, bundle)
    policy = qualification_policy.load_policy(POLICY)
    if len(corpus["tasks"]) * policy["trials_per_class"] < policy["minimum_total_candidate_calls"]:
        raise ValueError("holdout is underpowered for the minimum candidate-call gate")
    classes = {}
    for task in corpus["tasks"]:
        key = qualification_policy.class_key(qualification_policy.request_class(task))
        classes.setdefault(key, set()).add(task["family_id"])
    if not any(len(families) >= policy["minimum_semantic_families_per_class"]
               for families in classes.values()):
        raise ValueError("no request class has the minimum independent semantic families")
    destructive_families = {task["family_id"] for task in corpus["tasks"]
                            if task.get("tags", {}).get("destructive_scope_case", False)}
    if len(destructive_families) < policy["minimum_destructive_scope_families"]:
        raise ValueError("holdout lacks the minimum independent destructive-scope families")
    commitment = {
        "version": 1, "status": "sealed",
        "corpus_sha256": qualification_policy.sha256_file(args.corpus),
        "reference_bundle_sha256": qualification_policy.sha256_file(bundle),
        "sealed_at_utc": args.sealed_at_utc,
        "policy_sha256": qualification_policy.sha256_file(POLICY),
        "reviewer": args.reviewer.strip(),
    }
    write_atomic(args.output, commitment, args.overwrite)
    print(args.output)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError) as error:
        print(f"seal-qualification-holdout: {error}", file=sys.stderr)
        raise SystemExit(2)
