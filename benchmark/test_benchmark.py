#!/usr/bin/env python3
"""Fast contract tests; set UHM_BENCH_DOCKER_TESTS=1 for worker integration."""

import importlib.util
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest

from jsonschema import Draft202012Validator, ValidationError

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("provider_bakeoff", ROOT / "scripts/provider-bakeoff.py")
BENCH = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(BENCH)


class BenchmarkTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.corpus = BENCH.load_corpus(BENCH.DEFAULT_CORPUS)

    def test_fixed_distribution_and_actions(self):
        counts = {}
        tools = BENCH.proposal_tools()
        for task in self.corpus["tasks"]:
            category = task["tags"]["category"]
            counts[category] = counts.get(category, 0) + 1
            for action in task["reference_actions"] + task["negative_actions"]:
                BENCH.validate_action(action["tool"], action["arguments"], tools)
            self.assertIn(task["route_oracle"]["preferred"], task["route_oracle"]["allowed"])
            self.assertTrue(task["family_id"])
            self.assertTrue(task["variant_id"])
        self.assertEqual(counts, {"shell-read": 28, "shell-write": 20, "program-stdout": 28,
                                  "program-artifact": 20, "parent-shell": 8,
                                  "clarification": 8, "answer": 8})

    def test_nonexecuting_oracles(self):
        for task in self.corpus["tasks"]:
            action = task["reference_actions"][0]
            if action["tool"] in {"return_answer", "request_clarification"}:
                self.assertTrue(BENCH.deterministic_nonexecuting(task, action)["deterministic_pass"], task["id"])
                self.assertFalse(BENCH.deterministic_nonexecuting(task, task["negative_actions"][0])["deterministic_pass"], task["id"])

    def test_execution_contract_rejects_unknown_fields_and_unsafe_paths(self):
        scope_labeled = copy.deepcopy(self.corpus)
        scope_labeled["tasks"][0]["tags"]["destructive_scope_case"] = True
        BENCH.validate_execution_corpus(scope_labeled)
        unknown = copy.deepcopy(self.corpus)
        unknown["tasks"][0]["grader_script"] = "true"
        with self.assertRaises(ValueError):
            BENCH.validate_execution_corpus(unknown)
        unsafe = copy.deepcopy(self.corpus)
        unsafe["tasks"][0]["fixture"]["files"][0]["path"] = "../escape"
        with self.assertRaises(ValueError):
            BENCH.validate_execution_corpus(unsafe)

    def test_paired_statistics_cluster_by_task(self):
        records = []
        for trial in range(3):
            for task, left, right in [("one", True, False), ("two", True, True), ("three", False, False)]:
                records.extend([
                    {"task_id": task, "family_id": task, "trial": trial, "candidate": {"provider": "openai", "model": "a"}, "completed_outcome": left},
                    {"task_id": task, "family_id": task, "trial": trial, "candidate": {"provider": "cerebras", "model": "b"}, "completed_outcome": right},
                ])
        result = BENCH.paired_bootstrap(records, ("openai", "a"), ("cerebras", "b"), 1000, 7)
        self.assertEqual(result["family_count"], 3)
        self.assertEqual(result["difference_points"], 33.33)
        self.assertEqual(BENCH.exact_mcnemar(records, ("openai", "a"), ("cerebras", "b"))["left_only"], 1)

    def test_family_bootstrap_is_invariant_to_variant_cloning(self):
        records = []
        for task_id, family, left, right in [("a-1", "a", True, False), ("b-1", "b", False, True)]:
            for candidate, passed in [("left", left), ("right", right)]:
                records.append({"task_id": task_id, "family_id": family, "trial": 1,
                                "candidate": {"provider": "openai", "model": candidate},
                                "completed_outcome": passed})
        baseline = BENCH.paired_bootstrap(records, ("openai", "left"), ("openai", "right"), 1000, 9)
        cloned = copy.deepcopy(records)
        for index in range(20):
            for record in records[:2]:
                item = copy.deepcopy(record); item["task_id"] = f"a-clone-{index}-{item['candidate']['model']}"; cloned.append(item)
        repeated = BENCH.paired_bootstrap(cloned, ("openai", "left"), ("openai", "right"), 1000, 9)
        self.assertEqual(baseline, repeated)

    def test_all_schemas_are_valid_and_strict(self):
        for path in (ROOT / "benchmark/schemas").glob("*.json"):
            schema = json.loads(path.read_text())
            Draft202012Validator.check_schema(schema)
        malformed = {"contract_version": 2, "status": "error", "error": {"code": "bad", "message": "x"}, "extra": True}
        with self.assertRaises(ValidationError):
            BENCH.schema_validator("worker-result.schema.json").validate(malformed)

    def test_audit_dispositions_are_exactly_twenty_and_strict(self):
        validator = BENCH.schema_validator("audit-dispositions.schema.json")
        audit = {
            "version": 1,
            "reviewer": "independent reviewer",
            "rubric_version": 1,
            "items": [{
                "audit_id": f"{index:016x}",
                "disposition": "agree",
                "rationale": "Deterministic evidence and rubric agree.",
            } for index in range(20)],
        }
        validator.validate(audit)
        too_short = copy.deepcopy(audit)
        too_short["items"].pop()
        with self.assertRaises(ValidationError):
            validator.validate(too_short)
        unknown = copy.deepcopy(audit)
        unknown["items"][0]["candidate"] = "hidden identity"
        with self.assertRaises(ValidationError):
            validator.validate(unknown)

    def test_stale_image_manifest_is_rejected(self):
        manifest = {"hashes": {"fixture_bundle_and_oracle": "0" * 64}}
        with self.assertRaises(ValueError):
            BENCH.verify_worker_identity(manifest, BENCH.DEFAULT_CORPUS)

    def test_qualification_refuses_unsealed_or_development_corpus(self):
        with self.assertRaisesRegex(ValueError, "unavailable"):
            BENCH.qualification_policy.validate_holdout(
                self.corpus, BENCH.DEFAULT_CORPUS,
                ROOT / "model-qualification-holdout-v1.json",
                ROOT / "model-qualification-policy-v1.json",
            )

    def qualification_fixture(self, cerebras_latency=100):
        tasks = []
        records = []
        candidates = [("openai", "gpt-5.6-terra"), ("cerebras", "gpt-oss-120b")]
        for index in range(100):
            task_id = f"scope-{index}"
            task = {
                "id": task_id, "family_id": f"family-{index}", "mode": "run",
                "fixture": {"stdin": None},
                "route_oracle": {"allowed": ["run_shell"], "preferred": "run_shell"},
                "tags": {"category": "shell-write", "effects": ["write_local"],
                         "destructive_scope_case": index < 60},
            }
            tasks.append(task)
            for trial in range(1, 4):
                for provider, model in candidates:
                    latency = cerebras_latency if provider == "cerebras" else 100
                    records.append({
                        "task_id": task_id, "family_id": task["family_id"], "trial": trial,
                        "candidate": {"provider": provider, "model": model},
                        "stratum": "executable", "transport_success": True,
                        "wire_valid": True, "client_valid": True,
                        "first_shot_completed_outcome": True,
                        "action": {"tool": "run_shell", "arguments": {"effects": ["write_local"]}},
                        "execution": {"oracle": {"assertions": [{
                            "name": "filesystem.no_undeclared_changes", "passed": True}]}},
                        "timing": {"wall_ms": latency, "usage": {"input_tokens": 1, "output_tokens": 1},
                                   "provider_provenance": {
                                       "resolved_fingerprint": f"{provider}-revision",
                                       "resolved_model": model,
                                       "api_family": {"openai": "openai_responses_v1", "cerebras": "cerebras_chat_completions_v1", "deepseek": "deepseek_responses_v1"}[provider]}},
                    })
        calibration = []
        for provider, model in candidates:
            calibration.extend({"candidate": {"provider": provider, "model": model}, "agreement": True}
                               for _ in range(12))
        audit = {"items": [{"disposition": "agree"} for _ in range(20)]}
        policy = copy.deepcopy(BENCH.qualification_policy.load_policy(
            ROOT / "model-qualification-policy-v1.json"))
        policy["family_bootstrap_resamples"] = 100
        return records, tasks, candidates, calibration, audit, policy

    def test_exact_policy_profiles_and_latency_tiebreak(self):
        values = self.qualification_fixture(cerebras_latency=81)
        result = BENCH.qualification_policy.evaluate(*values, seed=7)
        self.assertTrue(all(profile["qualified"] for profile in result["profiles"]))
        selection = next(iter(result["selections"].values()))
        self.assertEqual(selection["winner"], {"provider": "openai", "model": "gpt-5.6-terra"})
        values = self.qualification_fixture(cerebras_latency=79)
        result = BENCH.qualification_policy.evaluate(*values, seed=7)
        selection = next(iter(result["selections"].values()))
        self.assertEqual(selection["winner"], {"provider": "cerebras", "model": "gpt-oss-120b"})

    def test_exact_policy_fails_closed_at_independent_gates(self):
        records, tasks, candidates, calibration, audit, policy = self.qualification_fixture()
        for record in records:
            if record["candidate"]["provider"] == "openai" and record["task_id"] in {"scope-0", "scope-1"}:
                record["transport_success"] = False
        audit["items"][0]["disposition"] = "critical_error"
        for task in tasks:
            if task["family_id"] == "family-59":
                task["tags"]["destructive_scope_case"] = False
        result = BENCH.qualification_policy.evaluate(
            records, tasks, candidates, calibration, audit, policy, seed=7)
        openai = next(profile for profile in result["profiles"]
                      if profile["candidate"]["provider"] == "openai")
        self.assertFalse(openai["checks"]["transport_success"])
        self.assertFalse(openai["checks"]["independent_audit"])
        self.assertFalse(openai["checks"]["destructive_scope"])
        self.assertFalse(openai["qualified"])

    def test_report_is_derived_from_finalized_events(self):
        fingerprint = "a" * 64
        events = [
            {"event_version": 1, "type": "run_started", "run_fingerprint": fingerprint, "sequence": 0,
             "payload": {"started_utc": "2026-01-01T00:00:00Z", "fingerprint_projection": {}, "corpus": "fixture", "task_count": 0, "worker_manifest": None, "git": {}, "host": {}, "docker_version": None}},
            {"event_version": 1, "type": "summary_computed", "run_fingerprint": fingerprint, "sequence": 1,
             "payload": {"models": [], "comparisons": [], "selection": {"winner": None}, "judge_calibration": [], "calibration_records": [], "independent_audit": {"status": "complete"}, "task_count": 0, "family_count": 0, "product_usage_weighted_completion": None}},
            {"event_version": 1, "type": "run_completed", "run_fingerprint": fingerprint, "sequence": 2,
             "payload": {"completed_utc": "2026-01-01T00:00:01Z", "record_count": 0}},
        ]
        with tempfile.TemporaryDirectory() as directory:
            artifact = Path(directory) / "run.jsonl"
            artifact.write_text("".join(json.dumps(event) + "\n" for event in events))
            process = BENCH.subprocess.run([BENCH.sys.executable, str(ROOT / "scripts/provider-benchmark-report.py"), str(artifact)], capture_output=True, text=True)
            self.assertEqual(process.returncode, 0, process.stderr)
            summary = json.loads(Path(str(artifact) + ".summary.json").read_text())
            self.assertEqual(summary["artifact_sha256"], BENCH.hashlib.sha256(artifact.read_bytes()).hexdigest())
            self.assertTrue(Path(str(artifact) + ".html").is_file())

    def test_resume_checkpoint_requires_exact_fingerprint_and_sequence(self):
        fingerprint = "b" * 64
        started = {
            "event_version": 1, "type": "run_started", "run_fingerprint": fingerprint,
            "sequence": 0,
            "payload": {"started_utc": "2026-01-01T00:00:00Z", "fingerprint_projection": {},
                        "corpus": "fixture", "task_count": 0, "worker_manifest": None,
                        "git": {}, "host": {}, "docker_version": None},
        }
        with tempfile.TemporaryDirectory() as directory:
            checkpoint = Path(directory) / "run.partial"
            checkpoint.write_text(json.dumps(started) + "\n")
            self.assertEqual(BENCH.load_checkpoint(checkpoint, fingerprint), [started])
            with self.assertRaisesRegex(ValueError, "fingerprint"):
                BENCH.load_checkpoint(checkpoint, "c" * 64)
            skipped = copy.deepcopy(started); skipped["sequence"] = 1
            checkpoint.write_text(json.dumps(skipped) + "\n")
            with self.assertRaisesRegex(ValueError, "sequence"):
                BENCH.load_checkpoint(checkpoint, fingerprint)
            checkpoint.write_text(json.dumps(started) + "\n{")
            with self.assertRaisesRegex(ValueError, "truncated"):
                BENCH.load_checkpoint(checkpoint, fingerprint)

    def test_canonical_contract_conformance_vectors(self):
        fixture = json.loads((ROOT / "tests/fixtures/action-validation-cases-v2.json").read_text())
        for case in fixture["cases"]:
            process = BENCH.subprocess.run([str(BENCH.CONTRACT_HELPER), "validate"], input=json.dumps(case["envelope"]), text=True, capture_output=True)
            result = json.loads(process.stdout)
            self.assertEqual(result["valid"], case["valid"], case["id"])
            if case["valid"]:
                self.assertEqual(result["action"], case["normalized"], case["id"])
            else:
                self.assertEqual(result["rejection"]["code"], case["rejection_code"], case["id"])

    def test_reference_bundle_is_schema_v4_and_preflight_clean(self):
        bundle = json.loads((ROOT / "tests/fixtures/provider-execution-reference-actions-v4.json").read_text())
        self.assertEqual(bundle["program_contract"], "uhm_helper_v1")
        self.assertEqual({item["id"] for item in bundle["tasks"]},
                         {task["id"] for task in self.corpus["tasks"]})
        by_id = {item["id"]: item for item in bundle["tasks"]}
        for task in self.corpus["tasks"]:
            self.assertEqual(by_id[task["id"]]["reference_actions"], task["reference_actions"])
            for action in task["reference_actions"]:
                result = BENCH.preflight_action(
                    action["tool"], action["arguments"], task["fixture"]["stdin"] is not None
                )
                self.assertTrue(result["valid"], (task["id"], result["diagnostics"]))

    def test_production_and_benchmark_preflight_share_diagnostic_vectors(self):
        fixture = json.loads((ROOT / "tests/fixtures/program-preflight-cases-v1.json").read_text())
        for case in fixture["cases"]:
            result = BENCH.preflight_action("run_program", case["arguments"], case["piped_input_present"])
            self.assertEqual(sorted(item["code"] for item in result["diagnostics"]),
                             sorted(case["codes"]), case["id"])
            self.assertEqual(result["valid"], not any(
                item["severity"] == "hard_error" for item in result["diagnostics"]
            ), case["id"])
            messages = json.dumps(result["diagnostics"])
            for file in case["arguments"]["files"]:
                self.assertNotIn(file["path"], messages, case["id"])

    def test_semantic_count_oracle_accepts_conventional_padding(self):
        passed, _ = BENCH.match_text({"matcher": "count_map", "value": {"apple": 2, "pear": 1}}, "      2 apple\n      1 pear\n")
        self.assertTrue(passed)

    def test_repair_uses_only_production_visible_failure_evidence(self):
        action = {"tool": "run_program", "arguments": {
            "runtime": "python3", "contract": "uhm_helper_v1", "source": "input()",
            "summary": "Read", "assumptions": [], "stdin_mode": "none", "files": [],
            "effects": ["read_local"],
        }}
        follow_up = BENCH.repair_follow_up(action, [{
            "code": "builtin_input_is_unsupported", "severity": "hard_error",
            "message": "Built-in input() is unsupported because process stdin is closed.",
        }], None)
        encoded = json.dumps(follow_up)
        for sentinel in ("HIDDEN-EXPECTED-SENTINEL", "ORACLE-DIFF-SENTINEL",
                         "JUDGE-RATIONALE-SENTINEL", ".uhm-stage-secret"):
            self.assertNotIn(sentinel, encoded)
        self.assertIsNone(BENCH.repair_follow_up(action, [], {
            "exit_code": 0, "timed_out": False, "stdout_truncated": False,
            "stderr_truncated": False, "oracle": {"passed": False,
            "reason": "ORACLE-DIFF-SENTINEL"},
        }))

    @unittest.skipUnless(os.environ.get("UHM_BENCH_DOCKER_TESTS") == "1", "Docker integration is opt-in")
    def test_model_context_comes_from_selected_worker(self):
        manifest, _ = BENCH.worker_manifest(BENCH.DEFAULT_WORKER_IMAGE)
        task = self.corpus["tasks"][0]
        payload = json.loads(BENCH.proposal_input(task, manifest))
        self.assertEqual(payload["context"]["program_runtime"]["version"], manifest["python"]["version"])
        self.assertEqual(payload["context"]["machine"]["architecture"], manifest["architecture"])

    @unittest.skipUnless(os.environ.get("UHM_BENCH_DOCKER_TESTS") == "1", "Docker integration is opt-in")
    def test_malformed_worker_response_is_rejected(self):
        process = BENCH.subprocess.run(
            ["docker", "run", "--rm", "--network", "none", "--read-only", "--entrypoint", "printf",
             BENCH.DEFAULT_WORKER_IMAGE, '{"status":"success","unknown":true}'],
            text=True, capture_output=True, check=False,
        )
        value = json.loads(process.stdout)
        with self.assertRaises(ValidationError):
            BENCH.schema_validator("worker-result.schema.json").validate(value)

    def test_worker_command_has_no_secret_or_host_channels(self):
        command = BENCH.worker_command("uhm-bench-worker:v1")
        joined = " ".join(command)
        self.assertIn("--network none", joined)
        self.assertIn("--read-only", joined)
        self.assertIn("--cap-drop ALL", joined)
        self.assertIn("no-new-privileges=true", joined)
        self.assertNotIn("--env", command)
        self.assertNotIn("-e", command)
        self.assertNotIn("--volume", command)
        self.assertNotIn("-v", command)
        self.assertNotIn("docker.sock", joined)

    @unittest.skipUnless(os.environ.get("UHM_BENCH_DOCKER_TESTS") == "1", "Docker integration is opt-in")
    def test_all_reference_actions_in_locked_worker(self):
        image = os.environ.get("UHM_BENCH_IMAGE", BENCH.DEFAULT_WORKER_IMAGE)
        BENCH.worker_image_id(image)
        for task in self.corpus["tasks"]:
            for action in task["reference_actions"]:
                if action["tool"] in {"run_shell", "run_program", "require_parent_shell"}:
                    result = BENCH.execute_in_worker(image, task, action)
                    self.assertTrue(result["oracle"]["passed"], task["id"])
            if task["negative_actions"][0]["tool"] in {"run_shell", "run_program", "require_parent_shell"}:
                bad = BENCH.execute_in_worker(image, task, task["negative_actions"][0])
                self.assertFalse(bad["oracle"]["passed"], task["id"])


class ResumeTests(unittest.TestCase):
    """Plan 19 W10: a checkpoint written through the documented audit pause
    resumes, and only judgment-derived fields may differ between events."""

    FINGERPRINT = "d" * 64

    def checkpoint_event(self, sequence, event_type, payload):
        event = {"event_version": 1, "type": event_type, "run_fingerprint": self.FINGERPRINT,
                 "sequence": sequence, "payload": payload}
        BENCH.schema_validator("run-event.schema.json").validate(event)
        return event

    def candidate_record(self, task_id="task-a", tool="return_answer", stratum="semantic"):
        return {
            "task_id": task_id, "family_id": "family-a", "trial": 1, "variant_id": "0",
            "candidate": {"provider": "openai", "model": "gpt-5.6-terra"},
            "type": "result", "stratum": stratum, "transport_success": True,
            "wire_valid": True, "client_valid": True, "preflight_valid": True,
            "execution_attempted": False, "execution_started": False,
            "completed_outcome": False, "oracle_pass": None, "error": None,
            "execution": None, "route_allowed": True, "route_preferred": True,
            "action": {"tool": tool, "arguments": {}},
            "judgments": [], "timing": {"wall_ms": 1},
        }

    def started_payload(self, task_count=1):
        return {"started_utc": "2026-01-01T00:00:00Z", "fingerprint_projection": {},
                "corpus": "fixture", "task_count": task_count, "worker_manifest": None,
                "git": {}, "host": {}, "docker_version": None}

    def write_checkpoint(self, events):
        directory = tempfile.mkdtemp()
        path = Path(directory) / "run.partial"
        with path.open("w", encoding="utf-8") as handle:
            for event in events:
                handle.write(json.dumps(event) + "\n")
        return path

    def judged(self, record, verdict="pass", semantic=True):
        judged = copy.deepcopy(record)
        judged["judgments"] = [
            {"valid": True, "synthetic": False, "verdict": verdict, "critical_error": False}
        ]
        if semantic and record["stratum"] == "semantic":
            judged["semantic_acceptable"] = verdict in {"pass", "minor"}
        return judged

    def test_audit_pause_resume_accepts_semantic_judgments(self):
        # The qualification pause sits between the judgment event and the
        # resume for both semantic routes.
        for tool in ("return_answer", "request_clarification"):
            record = self.candidate_record(tool=tool)
            events = [
                self.checkpoint_event(0, "run_started", self.started_payload()),
                self.checkpoint_event(1, "candidate_completed", {"record": record}),
                self.checkpoint_event(2, "judgment_completed", {"record": self.judged(record)}),
            ]
            path = self.write_checkpoint(events)
            prior = BENCH.load_checkpoint(path, self.FINGERPRINT)
            records, judged, completed = BENCH.rebuild_resume_state(prior)
            self.assertEqual(len(records), 1)
            self.assertTrue(judged, f"{tool} must resume as judged")
            self.assertTrue(completed)
            self.assertTrue(records[0]["semantic_acceptable"])

    def test_resume_rejects_changed_candidate_evidence(self):
        conflicts = {
            "action bytes": lambda r: r["action"].update({"arguments": {"changed": True}}),
            "execution evidence": lambda r: r.update({"execution": {"oracle": {}}}),
            "route evidence": lambda r: r.update({"route_allowed": False}),
        }
        for label, mutate in conflicts.items():
            record = self.candidate_record()
            judged = self.judged(record)
            mutate(judged)
            with self.assertRaisesRegex(ValueError, "conflicting resume event key", msg=label):
                BENCH.rebuild_resume_state([
                    self.checkpoint_event(1, "candidate_completed", {"record": record}),
                    self.checkpoint_event(2, "judgment_completed", {"record": judged}),
                ])
        # Changing the key itself strands the judgment without its candidate.
        for label, mutate in {
            "candidate identity": lambda r: r["candidate"].update({"model": "other-model"}),
            "task": lambda r: r.update({"task_id": "task-z"}),
            "trial": lambda r: r.update({"trial": 2}),
        }.items():
            record = self.candidate_record()
            judged = self.judged(record)
            mutate(judged)
            with self.assertRaisesRegex(ValueError, "without its candidate event", msg=label):
                BENCH.rebuild_resume_state([
                    self.checkpoint_event(1, "candidate_completed", {"record": record}),
                    self.checkpoint_event(2, "judgment_completed", {"record": judged}),
                ])

    def test_resume_rejects_duplicate_events(self):
        record = self.candidate_record()
        with self.assertRaisesRegex(ValueError, "duplicate resume event key"):
            BENCH.rebuild_resume_state([
                self.checkpoint_event(1, "candidate_completed", {"record": record}),
                self.checkpoint_event(2, "candidate_completed", {"record": copy.deepcopy(record)}),
            ])

    def test_resume_rejects_an_inconsistent_derived_verdict(self):
        record = self.candidate_record()
        judged = self.judged(record, verdict="fail")
        judged["semantic_acceptable"] = True  # tampered: judgments say fail
        with self.assertRaisesRegex(ValueError, "inconsistent with its judgments"):
            BENCH.rebuild_resume_state([
                self.checkpoint_event(1, "candidate_completed", {"record": record}),
                self.checkpoint_event(2, "judgment_completed", {"record": judged}),
            ])

    def test_audit_pause_resume_accepts_ineligible_semantic_records(self):
        # A failed candidate call leaves a client-invalid semantic record; the
        # judge phase marks it synthetic without ever deriving a verdict.
        # Such checkpoints are legitimate and must resume.
        record = self.candidate_record()
        record["client_valid"] = False
        judged = copy.deepcopy(record)
        judged["synthetic_outcome"] = "invalid_or_unsampled_deterministic_failure"
        events = [
            self.checkpoint_event(0, "run_started", self.started_payload()),
            self.checkpoint_event(1, "candidate_completed", {"record": record}),
            self.checkpoint_event(2, "judgment_completed", {"record": judged}),
        ]
        path = self.write_checkpoint(events)
        prior = BENCH.load_checkpoint(path, self.FINGERPRINT)
        records, judged_keys, completed = BENCH.rebuild_resume_state(prior)
        self.assertEqual(len(records), 1)
        self.assertTrue(judged_keys and completed)
        # A client-invalid semantic record can never acquire judgments.
        tampered = copy.deepcopy(judged)
        tampered["judgments"] = [
            {"valid": True, "synthetic": False, "verdict": "pass", "critical_error": False}
        ]
        with self.assertRaisesRegex(ValueError, "ineligible judgments"):
            BENCH.rebuild_resume_state([
                self.checkpoint_event(1, "candidate_completed", {"record": record}),
                self.checkpoint_event(2, "judgment_completed", {"record": tampered}),
            ])
        # Supplying a consistent pass verdict still cannot make a
        # client-invalid candidate eligible after the audit pause.
        tampered["semantic_acceptable"] = True
        with self.assertRaisesRegex(ValueError, "ineligible judgments"):
            BENCH.rebuild_resume_state([
                self.checkpoint_event(1, "candidate_completed", {"record": record}),
                self.checkpoint_event(2, "judgment_completed", {"record": tampered}),
            ])

        # Conversely, the synthetic no-judgment form belongs only to the
        # client-invalid path.
        valid = self.candidate_record()
        synthetic = copy.deepcopy(valid)
        synthetic["synthetic_outcome"] = "invalid_or_unsampled_deterministic_failure"
        with self.assertRaisesRegex(ValueError, "ineligible synthetic outcome"):
            BENCH.rebuild_resume_state([
                self.checkpoint_event(1, "candidate_completed", {"record": valid}),
                self.checkpoint_event(2, "judgment_completed", {"record": synthetic}),
            ])

    def test_resume_reuses_completed_work_without_new_provider_calls(self):
        # A mid-judgment checkpoint: both candidates are complete, only the
        # first is judged. The production filters the runner loops consult
        # must exclude the completed job and the judged record, so a resume
        # neither re-calls the provider for the completed candidate nor
        # re-judges the judged record.
        first = self.candidate_record(task_id="task-a")
        second = self.candidate_record(task_id="task-b", stratum="executable", tool="run_shell")
        first_key = ("task-a", 1, "openai", "gpt-5.6-terra")
        second_key = ("task-b", 1, "openai", "gpt-5.6-terra")
        events = [
            self.checkpoint_event(0, "run_started", self.started_payload(task_count=2)),
            self.checkpoint_event(1, "candidate_completed", {"record": first}),
            self.checkpoint_event(2, "candidate_completed", {"record": second}),
            self.checkpoint_event(3, "judgment_completed", {"record": self.judged(first)}),
        ]
        path = self.write_checkpoint(events)
        prior = BENCH.load_checkpoint(path, self.FINGERPRINT)
        records, judged, completed = BENCH.rebuild_resume_state(prior)
        self.assertEqual(completed, {first_key, second_key})
        self.assertEqual(judged, {first_key})
        self.assertEqual(len(records), 2)
        task_a = {"id": "task-a"}
        task_b = {"id": "task-b"}
        jobs = [
            (1, task_a, ("openai", "gpt-5.6-terra")),
            (1, task_b, ("openai", "gpt-5.6-terra")),
        ]
        self.assertEqual(BENCH.pending_candidate_jobs(jobs, completed), [])
        self.assertEqual(
            BENCH.pending_candidate_jobs(jobs, {first_key}),
            [(1, task_b, ("openai", "gpt-5.6-terra"))],
        )
        self.assertEqual(
            BENCH.pending_judge_jobs([(records[0], task_a), (records[1], task_b)], judged),
            [(records[1], task_b)],
        )


class QualificationInteroperabilityTests(unittest.TestCase):
    """Plan 19 W10: Python-generated profiles use the runtime's canonical
    action kinds, checked against the actual Rust validator and membership
    predicate rather than a Python approximation."""

    def helper_json(self, operation, payload):
        process = BENCH.subprocess.run(
            [str(BENCH.CONTRACT_HELPER), operation],
            input=json.dumps(payload), text=True, capture_output=True,
        )
        self.assertEqual(process.returncode, 0, process.stderr)
        return json.loads(process.stdout)

    def test_profiles_map_to_runtime_action_kinds(self):
        records, tasks, candidates, calibration, audit, policy = (
            BenchmarkTests().qualification_fixture()
        )
        summary = BENCH.qualification_policy.evaluate(
            records, tasks, candidates, calibration, audit, policy, 7
        )
        mapping = BENCH.contract_description()["tool_to_action_kind"]
        self.assertEqual(mapping["run_shell"], "shell")
        for profile in summary["profiles"]:
            self.assertIn("run_shell", profile["permitted_action_types"])
        import importlib.util
        spec = importlib.util.spec_from_file_location(
            "generator", ROOT / "scripts/provider-qualification-manifest.py")
        generator = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(generator)
        kinds = generator.manifest_action_kinds(["run_shell", "return_answer"])
        self.assertEqual(kinds, ["answer", "shell"])
        with self.assertRaisesRegex(ValueError, "unknown wire tool"):
            generator.manifest_action_kinds(["run_shell", "not-a-tool"])

    def test_rust_membership_predicate_matches_mapped_profiles(self):
        permitted = ["answer", "shell"]
        cases = [("shell", True), ("program", False), ("answer", True),
                 ("clarification", False), ("parent_shell", False)]
        for kind, expected in cases:
            result = self.helper_json("action-permitted", {
                "permitted": permitted, "action_kind": kind,
            })
            self.assertTrue(result["valid"], result)
            self.assertEqual(result["permitted"], expected, kind)
        unknown = self.helper_json("action-permitted", {
            "permitted": permitted, "action_kind": "run_shell",
        })
        self.assertFalse(unknown["valid"])
        # No profile (fixed selection) permits everything.
        open_profile = self.helper_json("action-permitted", {
            "permitted": None, "action_kind": "shell",
        })
        self.assertTrue(open_profile["valid"] and open_profile["permitted"])

    def test_empty_manifest_stays_fail_closed(self):
        manifest = json.loads((ROOT / "model-qualification-manifest.json").read_text())
        self.assertEqual(manifest["entries"], [])



if __name__ == "__main__":
    unittest.main()
