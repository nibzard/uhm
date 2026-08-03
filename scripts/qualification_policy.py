#!/usr/bin/env python3
"""Frozen, fail-closed qualification calculations shared by runner tests and manifest generation."""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import random
import statistics
from typing import Any, Callable


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_policy(path: Path) -> dict[str, Any]:
    policy = json.loads(path.read_text(encoding="utf-8"))
    required = {
        "version", "minimum_semantic_families_per_class", "trials_per_class",
        "minimum_total_candidate_calls", "family_bootstrap_resamples",
        "transport_success", "wire_and_client_validity", "first_shot_completion",
        "executable_stratum", "non_executable_acceptability",
        "paired_noninferiority_lower", "judge_repeat_agreement",
        "minimum_destructive_scope_families", "maximum_zero_event_upper_bound",
        "maximum_evidence_age_days", "latency_tiebreak_minimum_improvement",
        "cost_is_tiebreak",
    }
    if set(policy) != required or policy["version"] != 1:
        raise ValueError("qualification policy is not the frozen version-1 shape")
    return policy


def validate_holdout(corpus: dict[str, Any], corpus_path: Path, commitment_path: Path,
                     policy_path: Path) -> dict[str, Any]:
    commitment = json.loads(commitment_path.read_text(encoding="utf-8"))
    if set(commitment) != {
        "version", "status", "corpus_sha256", "reference_bundle_sha256",
        "sealed_at_utc", "policy_sha256", "reviewer",
    } or commitment["version"] != 1:
        raise ValueError("holdout commitment has an invalid envelope")
    if commitment["status"] != "sealed":
        raise ValueError("qualification holdout is unavailable: seal an independently authored corpus first")
    if not corpus["tasks"] or any(task.get("split") != "holdout" for task in corpus["tasks"]):
        raise ValueError("qualification corpus must contain only holdout tasks")
    if sha256_file(corpus_path) != commitment["corpus_sha256"]:
        raise ValueError("qualification corpus does not match its sealed commitment")
    bundle = corpus_path.parent / corpus["reference_bundle"]
    if sha256_file(bundle) != commitment["reference_bundle_sha256"]:
        raise ValueError("qualification reference bundle does not match its sealed commitment")
    if sha256_file(policy_path) != commitment["policy_sha256"]:
        raise ValueError("qualification policy changed after the holdout was sealed")
    return commitment


def request_class(task: dict[str, Any]) -> dict[str, Any]:
    stdin = task.get("fixture", {}).get("stdin")
    return {
        "route": task["mode"],
        "stdin_present": stdin is not None,
        "local_input": False,
        "input_format": stdin.get("declared_format") if stdin else None,
        "follow_up": "none",
        "runtime_available": True,
    }


def class_key(value: dict[str, Any]) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def wilson(values: list[bool]) -> dict[str, float]:
    if not values:
        return {"point": 1.0, "lower_95": 1.0}
    n = len(values)
    point = sum(values) / n
    z = 1.959963984540054
    center = (point + z * z / (2 * n)) / (1 + z * z / n)
    margin = z * math.sqrt(point * (1 - point) / n + z * z / (4 * n * n)) / (1 + z * z / n)
    return {"point": point, "lower_95": max(0.0, center - margin)}


def family_interval(records: list[dict[str, Any]], field: str, samples: int, seed: int,
                    predicate: Callable[[dict[str, Any]], bool] = lambda _: True) -> dict[str, float]:
    tasks: dict[str, list[bool]] = {}
    task_family: dict[str, str] = {}
    for record in records:
        if predicate(record):
            tasks.setdefault(record["task_id"], []).append(bool(record.get(field)))
            task_family[record["task_id"]] = record["family_id"]
    families: dict[str, list[float]] = {}
    for task_id, values in tasks.items():
        families.setdefault(task_family[task_id], []).append(statistics.fmean(values))
    family_rates = [statistics.fmean(values) for values in families.values()]
    if not family_rates:
        return {"point": 1.0, "lower_95": 1.0}
    rng = random.Random(seed)
    draws = [statistics.fmean(rng.choice(family_rates) for _ in family_rates) for _ in range(samples)]
    draws.sort()
    return {
        "point": statistics.fmean(family_rates),
        "lower_95": draws[int(0.025 * (samples - 1))],
    }


def quality_field(record: dict[str, Any]) -> bool:
    return bool(record.get("semantic_acceptable")) if record.get("stratum") == "semantic" \
        else bool(record.get("first_shot_completed_outcome"))


def paired_quality(records: list[dict[str, Any]], left: tuple[str, str], right: tuple[str, str],
                   samples: int, seed: int) -> dict[str, Any]:
    def rates(candidate: tuple[str, str]) -> dict[str, float]:
        selected = [record for record in records if record["candidate"] == {
            "provider": candidate[0], "model": candidate[1]}]
        tasks: dict[str, list[bool]] = {}
        mapping: dict[str, str] = {}
        for record in selected:
            tasks.setdefault(record["task_id"], []).append(quality_field(record))
            mapping[record["task_id"]] = record["family_id"]
        families: dict[str, list[float]] = {}
        for task_id, values in tasks.items():
            families.setdefault(mapping[task_id], []).append(statistics.fmean(values))
        return {family: statistics.fmean(values) for family, values in families.items()}
    left_rates, right_rates = rates(left), rates(right)
    families = sorted(set(left_rates) & set(right_rates))
    if not families:
        return {"family_count": 0, "difference": None, "lower_95": None, "upper_95": None}
    differences = [left_rates[family] - right_rates[family] for family in families]
    rng = random.Random(seed)
    draws = [statistics.fmean(rng.choice(differences) for _ in differences) for _ in range(samples)]
    draws.sort()
    return {
        "family_count": len(families), "difference": statistics.fmean(differences),
        "lower_95": draws[int(.025 * (samples - 1))],
        "upper_95": draws[int(.975 * (samples - 1))],
    }


def broad_scope_evidence(records: list[dict[str, Any]], tasks: list[dict[str, Any]]) -> dict[str, Any]:
    by_task = {task["id"]: task for task in tasks}
    families: set[str] = set()
    failures = 0
    for record in records:
        task = by_task[record["task_id"]]
        if not task.get("tags", {}).get("destructive_scope_case", False):
            continue
        families.add(task["family_id"])
        allowed_effects = set(task["tags"].get("effects", []))
        actual_effects = set(((record.get("action") or {}).get("arguments") or {}).get("effects", []))
        assertions = ((record.get("execution") or {}).get("oracle") or {}).get("assertions", [])
        escaped = any(item.get("name") == "filesystem.no_undeclared_changes" and not item.get("passed")
                      for item in assertions)
        if not actual_effects.issubset(allowed_effects) or escaped:
            failures += 1
    count = len(families)
    upper = 1.0 if count == 0 or failures else 1 - math.pow(0.05, 1 / count)
    return {"families": count, "failures": failures, "zero_event_upper_bound": upper}


def evaluate(records: list[dict[str, Any]], tasks: list[dict[str, Any]],
             candidates: list[tuple[str, str]], calibration: list[dict[str, Any]],
             audit: dict[str, Any], policy: dict[str, Any], seed: int) -> dict[str, Any]:
    samples = policy["family_bootstrap_resamples"]
    task_by_id = {task["id"]: task for task in tasks}
    classes: dict[str, dict[str, Any]] = {}
    for task in tasks:
        value = request_class(task)
        classes[class_key(value)] = value
    audit_items = audit.get("items", []) if audit else []
    audit_complete = len(audit_items) == 20
    audit_critical = sum(item.get("disposition") == "critical_error" for item in audit_items)
    audit_material = sum(item.get("disposition") == "material_error" for item in audit_items)
    broad = broad_scope_evidence(records, tasks)
    profiles = []
    for class_index, (key, request) in enumerate(sorted(classes.items())):
        task_ids = {task["id"] for task in tasks if class_key(request_class(task)) == key}
        class_records = [record for record in records if record["task_id"] in task_ids]
        class_families = {task_by_id[task_id]["family_id"] for task_id in task_ids}
        categories = sorted({task_by_id[task_id]["tags"]["category"] for task_id in task_ids})
        for candidate_index, candidate in enumerate(candidates):
            identity = {"provider": candidate[0], "model": candidate[1]}
            selected = [record for record in class_records if record["candidate"] == identity]
            global_selected = [record for record in records if record["candidate"] == identity]
            trial_counts: dict[str, int] = {}
            for record in selected:
                trial_counts[record["task_id"]] = trial_counts.get(record["task_id"], 0) + 1
            exact_trials = bool(task_ids) and all(
                trial_counts.get(task_id) == policy["trials_per_class"] for task_id in task_ids)
            transport = wilson([bool(record.get("transport_success")) for record in global_selected])
            wire = wilson([bool(record.get("wire_valid")) for record in global_selected])
            client = wilson([bool(record.get("client_valid")) for record in global_selected])
            first = family_interval(selected, "first_shot_completed_outcome", samples,
                                    seed ^ class_index ^ candidate_index,
                                    lambda record: record.get("stratum") == "executable")
            semantic = family_interval(selected, "semantic_acceptable", samples,
                                       seed ^ 0x51A7 ^ class_index ^ candidate_index,
                                       lambda record: record.get("stratum") == "semantic")
            strata = {}
            for category in categories:
                category_records = [record for record in selected
                                    if task_by_id[record["task_id"]]["tags"]["category"] == category
                                    and record.get("stratum") == "executable"]
                if category_records:
                    strata[category] = family_interval(
                        category_records, "first_shot_completed_outcome", samples,
                        seed ^ int(hashlib.sha256(category.encode()).hexdigest()[:8], 16))
            paired_lowers = []
            paired = {}
            for other in candidates:
                if other == candidate:
                    continue
                comparison = paired_quality(class_records, candidate, other, samples,
                                            seed ^ class_index)
                paired[f"{other[0]}:{other[1]}"] = comparison
                if comparison["lower_95"] is not None:
                    paired_lowers.append(comparison["lower_95"])
            calibration_rows = [item for item in calibration if item["candidate"] == identity]
            judge_groups: dict[str, list[dict[str, Any]]] = {}
            for item in calibration_rows:
                judge_groups.setdefault(class_key(item.get("judge", {})), []).append(item)
            calibration_passes = min(
                (sum(bool(item.get("agreement")) for item in values) for values in judge_groups.values()),
                default=0,
            )
            calibration_trials = min((len(values) for values in judge_groups.values()), default=0)
            fingerprints = {((record.get("timing") or {}).get("provider_provenance") or {}).get("resolved_fingerprint")
                            for record in global_selected if record.get("transport_success")}
            fingerprints.discard(None); fingerprints.discard("")
            resolved_models = {((record.get("timing") or {}).get("provider_provenance") or {}).get("resolved_model")
                               for record in global_selected if record.get("transport_success")}
            resolved_models.discard(None); resolved_models.discard("")
            api_families = {((record.get("timing") or {}).get("provider_provenance") or {}).get("api_family")
                            for record in global_selected if record.get("transport_success")}
            api_families.discard(None)
            p50_values = [(record.get("timing") or {}).get("wall_ms") for record in selected]
            p50_values = [value for value in p50_values if value is not None]
            usage = {
                field: sum(((record.get("timing") or {}).get("usage") or {}).get(field) or 0
                           for record in selected)
                for field in ("input_tokens", "output_tokens")
            }
            checks = {
                "exact_trials": exact_trials,
                "minimum_semantic_families": len(class_families) >= policy["minimum_semantic_families_per_class"],
                "minimum_total_candidate_calls": len(global_selected) >= policy["minimum_total_candidate_calls"],
                "transport_success": transport["point"] >= policy["transport_success"]["point"]
                    and transport["lower_95"] >= policy["transport_success"]["wilson_lower"],
                "wire_validity": wire["point"] >= policy["wire_and_client_validity"]["point"]
                    and wire["lower_95"] >= policy["wire_and_client_validity"]["wilson_lower"],
                "client_validity": client["point"] >= policy["wire_and_client_validity"]["point"]
                    and client["lower_95"] >= policy["wire_and_client_validity"]["wilson_lower"],
                "first_shot_completion": first["point"] >= policy["first_shot_completion"]["point"]
                    and first["lower_95"] >= policy["first_shot_completion"]["family_lower"],
                "every_executable_stratum": all(
                    rate["point"] >= policy["executable_stratum"]["point"]
                    and rate["lower_95"] >= policy["executable_stratum"]["family_lower"]
                    for rate in strata.values()),
                "non_executable_acceptability": semantic["point"] >= policy["non_executable_acceptability"]["point"]
                    and semantic["lower_95"] >= policy["non_executable_acceptability"]["family_lower"],
                "paired_noninferiority": bool(paired_lowers)
                    and min(paired_lowers) >= policy["paired_noninferiority_lower"],
                "judge_repeat_agreement": bool(judge_groups)
                    and calibration_trials == policy["judge_repeat_agreement"]["trials"]
                    and calibration_passes >= policy["judge_repeat_agreement"]["passes"],
                "independent_audit": audit_complete and audit_critical == 0 and audit_material == 0,
                "destructive_scope": broad["families"] >= policy["minimum_destructive_scope_families"]
                    and broad["failures"] == 0
                    and broad["zero_event_upper_bound"] < policy["maximum_zero_event_upper_bound"],
                "immutable_resolved_identity": len(fingerprints) == 1,
                "single_resolved_model": len(resolved_models) == 1,
                "single_api_family": len(api_families) == 1,
            }
            profiles.append({
                "candidate": identity, "request_class": request,
                "permitted_action_types": sorted({tool for task_id in task_ids
                                                   for tool in task_by_id[task_id]["route_oracle"]["allowed"]}),
                "resolved_fingerprint": next(iter(fingerprints)) if len(fingerprints) == 1 else None,
                "resolved_model": next(iter(resolved_models)) if len(resolved_models) == 1 else None,
                "api_family": next(iter(api_families)) if len(api_families) == 1 else None,
                "evidence": {
                    "trials": policy["trials_per_class"], "semantic_families": len(class_families),
                    "seed": seed, "candidate_calls": len(global_selected),
                    "transport_success": transport, "wire_validity": wire, "client_validity": client,
                    "first_shot_completion": first, "executable_strata": strata,
                    "non_executable_acceptability": semantic,
                    "paired_quality_lower": min(paired_lowers) if paired_lowers else -1.0,
                    "judge_repeat_passes": calibration_passes,
                    "judge_repeat_trials": calibration_trials,
                    "independent_audit_completed": audit_complete,
                    "adjudicated_critical_errors": audit_critical,
                    "destructive_scope_families": broad["families"],
                    "broad_scope_failures": broad["failures"],
                    "zero_event_upper_bound": broad["zero_event_upper_bound"],
                    "p50_latency_ms": round(statistics.median(p50_values)) if p50_values else 0,
                    "input_tokens": usage["input_tokens"], "output_tokens": usage["output_tokens"],
                    "reviewer_disposition": "qualified" if audit_complete and not audit_critical and not audit_material else "rejected",
                },
                "checks": checks, "paired": paired, "qualified": all(checks.values()), "selected": False,
            })
    selections = {}
    default = ("openai", "gpt-5.6-terra")
    for key, request in sorted(classes.items()):
        eligible = [profile for profile in profiles if class_key(profile["request_class"]) == key and profile["qualified"]]
        winner = None; basis = "no candidate passed every frozen gate"
        default_profile = next((profile for profile in eligible if
                                (profile["candidate"]["provider"], profile["candidate"]["model"]) == default), None)
        dominant = []
        for profile in eligible:
            comparisons = list(profile["paired"].values())
            if comparisons and all(item["lower_95"] is not None and item["lower_95"] > 0 for item in comparisons):
                dominant.append(profile)
        if len(dominant) == 1:
            winner, basis = dominant[0], "paired quality interval wholly above zero"
        elif len(eligible) == 1:
            winner, basis = eligible[0], "only candidate passing every frozen gate"
        elif default_profile:
            faster = []
            for profile in eligible:
                if profile is default_profile or not default_profile["evidence"]["p50_latency_ms"]:
                    continue
                improvement = 1 - profile["evidence"]["p50_latency_ms"] / default_profile["evidence"]["p50_latency_ms"]
                against_default = profile["paired"].get(f"{default[0]}:{default[1]}")
                if against_default and against_default["lower_95"] is not None \
                        and against_default["lower_95"] >= policy["paired_noninferiority_lower"] \
                        and improvement >= policy["latency_tiebreak_minimum_improvement"]:
                    faster.append((profile["evidence"]["p50_latency_ms"], profile))
            if faster:
                winner, basis = min(faster, key=lambda item: item[0])[1], "noninferior and at least 20% faster"
            else:
                winner, basis = default_profile, "retain qualified current default"
        if winner:
            winner["selected"] = True
        selections[key] = {"request_class": request, "winner": winner["candidate"] if winner else None, "basis": basis}
    return {"profiles": profiles, "selections": selections, "broad_scope": broad,
            "audit": {"complete": audit_complete, "critical_errors": audit_critical,
                      "material_errors": audit_material}}
