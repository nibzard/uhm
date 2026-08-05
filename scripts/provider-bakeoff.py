#!/usr/bin/env python3
"""End-to-end provider/model benchmark with offline execution and blinded judging.

API keys remain in this trusted runner. Generated actions execute only in a
fresh, keyless Docker worker with no network or host mounts.
"""

from __future__ import annotations

import argparse
import csv
import copy
import hashlib
import io
import json
import math
import os
from pathlib import Path
import platform
import random
import re
import statistics
import subprocess
import sys
import time
from typing import Any

from jsonschema import Draft202012Validator

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))
import qualification_policy


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CORPUS = ROOT / "tests/fixtures/provider-execution-benchmark-v2.json"
DEFAULT_OUTPUT = ROOT / "target/provider-bakeoff.jsonl"
DEFAULT_WORKER_IMAGE = "uhm-bench-worker:v2"
OPENAI_ENDPOINT = "https://api.openai.com/v1/responses"
CEREBRAS_ENDPOINT = "https://api.cerebras.ai/v1/chat/completions"
DEEPSEEK_ENDPOINT = "https://api.deepseek.com/v1/responses"
PROVIDERS = {"openai", "cerebras", "deepseek"}
PROVIDER_ENDPOINTS = {
    "openai": OPENAI_ENDPOINT,
    "cerebras": CEREBRAS_ENDPOINT,
    "deepseek": DEEPSEEK_ENDPOINT,
}
PROVIDER_CREDENTIALS = {
    "openai": "OPENAI_API_KEY",
    "cerebras": "CEREBRAS_API_KEY",
    "deepseek": "DEEPSEEK_API_KEY",
}
PROVIDER_API_FAMILIES = {
    "openai": "openai_responses_v1",
    "cerebras": "cerebras_chat_completions_v1",
    "deepseek": "deepseek_responses_v1",
}
# DeepSeek shares the OpenAI Responses wire shape. Cerebras is the lone
# Chat Completions outlier, so branching keys off Cerebras rather than OpenAI.
RESPONSES_PROVIDERS = {"openai", "deepseek"}
SCHEMAS = ROOT / "benchmark/schemas"
CONTRACT_HELPER = ROOT / "target/debug/uhm-bench-contract"
PROVIDER_HELPER = ROOT / "target/debug/uhm-provider-call"
QUALIFICATION_POLICY = ROOT / "model-qualification-policy-v1.json"
QUALIFICATION_COMMITMENT = ROOT / "model-qualification-holdout-v1.json"
_CONTRACT_DESCRIPTION: dict[str, Any] | None = None


class CandidateCallError(RuntimeError):
    def __init__(self, error: dict[str, Any], wall_ms: int):
        super().__init__(str(error.get("message", "provider adapter failed"))[:800])
        self.kind = error.get("kind")
        self.attempts_consumed = int(error.get("attempts_consumed", 0))
        self.wall_ms = wall_ms


def schema_validator(name: str) -> Draft202012Validator:
    schema = json.loads((SCHEMAS / name).read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema)


def contract_description() -> dict[str, Any]:
    global _CONTRACT_DESCRIPTION
    if _CONTRACT_DESCRIPTION is None:
        # Cargo's dependency tracking makes this cheap when current and avoids
        # qualifying a stale adapter binary after source changes.
        process = subprocess.run(
            ["cargo", "build", "--quiet", "--bin", "uhm-bench-contract", "--bin", "uhm-provider-call"],
            cwd=ROOT, check=False,
        )
        if process.returncode:
            raise RuntimeError("could not build canonical action/provider helpers")
        process = subprocess.run(
            [str(CONTRACT_HELPER), "describe"], text=True, capture_output=True, check=False,
        )
        if process.returncode:
            raise RuntimeError("canonical action helper describe failed")
        _CONTRACT_DESCRIPTION = json.loads(process.stdout)
    return copy.deepcopy(_CONTRACT_DESCRIPTION)


def string_array(description: str) -> dict[str, Any]:
    return {
        "type": "array",
        "description": description,
        "items": {"type": "string"},
        "maxItems": 32,
    }


def effects_schema() -> dict[str, Any]:
    return {
        "type": "array",
        "items": {
            "type": "string",
            "enum": [
                "read_local",
                "write_local",
                "delete_local",
                "network_read",
                "remote_mutation",
                "privilege_elevation",
                "process_control",
                "shell_state",
                "unknown",
            ],
        },
        "maxItems": 32,
    }


def strict_tool(
    name: str, description: str, properties: dict[str, Any]
) -> dict[str, Any]:
    return {
        "type": "function",
        "name": name,
        "description": description,
        "strict": True,
        "parameters": {
            "type": "object",
            "properties": properties,
            "required": list(properties),
            "additionalProperties": False,
        },
    }


def proposal_tools() -> list[dict[str, Any]]:
    return contract_description()["tools"]


def judgment_tool() -> dict[str, Any]:
    score = {"type": "integer", "enum": [0, 1, 2, 3, 4]}
    return strict_tool(
        "record_judgment",
        "Record a rubric-grounded evaluation of one anonymous candidate proposal.",
        {
            "task_correctness": score,
            "instruction_following": score,
            "safety_precision": score,
            "portability": score,
            "critical_error": {"type": "boolean"},
            "verdict": {"type": "string", "enum": ["pass", "minor", "fail"]},
            "issues": {"type": "array", "items": {"type": "string"}, "maxItems": 8},
            "rationale": {"type": "string", "maxLength": 2048},
        },
    )


def remove_cerebras_schema_bounds(value: Any) -> Any:
    """Remove bounds Cerebras rejects while preserving local validation."""
    if isinstance(value, dict):
        return {
            key: remove_cerebras_schema_bounds(item)
            for key, item in value.items()
            if key not in {"maxLength", "maxItems"}
        }
    if isinstance(value, list):
        return [remove_cerebras_schema_bounds(item) for item in value]
    return value


def chat_tools(tools: list[dict[str, Any]]) -> list[dict[str, Any]]:
    compatible = remove_cerebras_schema_bounds(tools)
    return [
        {
            "type": "function",
            "function": {key: value for key, value in tool.items() if key != "type"},
        }
        for tool in compatible
    ]


def load_prompt() -> tuple[int, int, str]:
    description = contract_description()
    return description["prompt_version"], description["action_schema_version"], description["developer_instructions"]


def load_corpus(path: Path) -> dict[str, Any]:
    corpus = json.loads(path.read_text(encoding="utf-8"))
    errors = sorted(schema_validator("corpus.schema.json").iter_errors(corpus), key=lambda error: list(error.path))
    if errors:
        raise ValueError(f"invalid corpus at {list(errors[0].path)}: {errors[0].message}")
    ids = [task.get("id") for task in corpus["tasks"]]
    if any(not isinstance(task_id, str) or not task_id for task_id in ids):
        raise ValueError("every task needs a non-empty string id")
    if len(ids) != len(set(ids)):
        raise ValueError("corpus task ids must be unique")
    known_tools = {tool["name"] for tool in proposal_tools()}
    for task in corpus["tasks"]:
        if task.get("mode") not in {"run", "ask"}:
            raise ValueError(f"task {task['id']} has an invalid mode")
        if not task.get("prompt") or not (task.get("rubric") or task.get("judge_rubric")):
            raise ValueError(f"task {task['id']} needs prompt and rubric")
        expected = task["route_oracle"]["allowed"]
        if not isinstance(expected, list) or not expected or not set(expected) <= known_tools:
            raise ValueError(f"task {task['id']} has invalid expected_tools")
        task["rubric"] = task.get("rubric") or task["judge_rubric"]
        if "fixture" in task:
            if corpus.get("worker_contract_version") != 2:
                raise ValueError("execution corpus needs worker contract version 2")
            if not isinstance(task.get("limits"), dict) or not isinstance(task.get("expected"), dict):
                raise ValueError(f"task {task['id']} needs limits and expected assertions")
    if corpus.get("task_count") is not None and corpus["task_count"] != len(corpus["tasks"]):
        raise ValueError("corpus task_count does not match tasks array")
    return corpus


def validate_execution_corpus(corpus: dict[str, Any]) -> None:
    def exact(value: dict[str, Any], allowed: set[str], label: str) -> None:
        extra = set(value) - allowed
        if extra:
            raise ValueError(f"{label} has unknown fields: {sorted(extra)}")

    def path(value: str, label: str) -> None:
        parts = Path(value).parts
        if not value or value.startswith("/") or ".." in parts or "\0" in value:
            raise ValueError(f"{label} has unsafe path {value!r}")

    exact(corpus, {"version", "prompt_version", "action_schema_version", "worker_contract_version", "reference_bundle", "task_count", "family_count", "route_counts", "tasks"}, "corpus")
    matchers = {"exact_text", "contains_lines", "unordered_lines", "regex", "json_equals", "csv_equals", "count_map", "integer_equals", "git_status", "empty"}
    task_fields = {"id", "family_id", "variant_id", "split", "mode", "prompt", "route_oracle", "rubric", "judge_rubric", "tags", "fixture", "limits", "expected", "reference_actions", "negative_actions", "oracle_disposition"}
    for task in corpus["tasks"]:
        label = f"task {task['id']}"
        exact(task, task_fields, label)
        exact(task["tags"], {
            "expected_tool", "category", "difficulty", "effects", "destructive_scope_case"
        }, label + " tags")
        fixture = task["fixture"]
        exact(fixture, {"cwd", "stdin", "directories", "files", "symlinks", "environment", "git"}, label + " fixture")
        if fixture["cwd"] != "/work":
            raise ValueError(label + " fixture cwd must be /work")
        for item in fixture["directories"]:
            path(item, label)
        for item in fixture["files"]:
            exact(item, {"path", "text", "mode"}, label + " file"); path(item["path"], label)
        for item in fixture["symlinks"]:
            exact(item, {"path", "target"}, label + " symlink"); path(item["path"], label)
        if fixture["stdin"] is not None:
            exact(fixture["stdin"], {"encoding", "declared_format", "text"}, label + " stdin")
            if fixture["stdin"]["encoding"] != "utf-8":
                raise ValueError(label + " stdin encoding must be utf-8")
        git = fixture["git"]
        if git is not None:
            exact(git, {"branch", "commits", "staged", "unstaged", "untracked"}, label + " git")
            for commit in git.get("commits", []):
                exact(commit, {"message", "files"}, label + " commit")
                for name in commit["files"]: path(name, label)
            for state in ("staged", "unstaged", "untracked"):
                for name in git.get(state, {}): path(name, label)
        exact(task["limits"], {"wall_ms", "stdout_bytes", "stderr_bytes", "workspace_bytes"}, label + " limits")
        expected = task["expected"]
        exact(expected, {"exit_codes", "stdout", "stderr", "filesystem", "forbid_undeclared_changes", "parent_state"}, label + " expected")
        for stream in ("stdout", "stderr"):
            if expected[stream].get("matcher") not in matchers:
                raise ValueError(f"{label} has invalid {stream} matcher")
            exact(expected[stream], {"matcher", "value", "pattern", "lines", "rows"}, label + " " + stream)
        for item in expected["filesystem"]:
            exact(item, {"path", "state", "content", "sha256"}, label + " filesystem assertion"); path(item["path"], label)
        exact(task["route_oracle"], {"allowed", "preferred", "rationale"}, label + " route oracle")
        for action_name in ("reference_actions", "negative_actions"):
            for action in task[action_name]:
                exact(action, {"tool", "arguments"}, label + " " + action_name)


def model_spec(value: str) -> tuple[str, str]:
    if ":" not in value:
        raise argparse.ArgumentTypeError("model spec must be PROVIDER:MODEL")
    provider, model = value.split(":", 1)
    if provider not in PROVIDERS or not model.strip():
        raise argparse.ArgumentTypeError(
            f"provider must be one of {sorted(PROVIDERS)} and model must be non-empty"
        )
    return provider, model


def api_key(provider: str) -> str:
    variable = PROVIDER_CREDENTIALS[provider]
    key = os.environ.get(variable, "").strip()
    if not key:
        raise ValueError(f"{variable} is required for provider {provider}")
    if "\r" in key or "\n" in key:
        raise ValueError(f"{variable} contains an invalid newline")
    return key


def curl_json(
    endpoint: str,
    key: str,
    body: dict[str, Any],
    timeout: int,
    headers: list[str] | None = None,
) -> tuple[dict[str, Any], dict[str, Any]]:
    marker = "__UHM_BENCH_TIMING__"
    command = [
        "curl",
        "-sS",
        "--max-time",
        str(timeout),
        endpoint,
        "-H",
        "Content-Type: application/json",
        "--data-binary",
        "@-",
        "--write-out",
        f"\n{marker}%{{http_code}}:%{{time_starttransfer}}:%{{time_total}}",
    ]
    for header in headers or []:
        command.extend(["-H", header])
    auth_read_fd = None
    if os.name == "posix" and Path("/dev/fd").is_dir():
        auth_read_fd, auth_write_fd = os.pipe()
        try:
            os.write(auth_write_fd, ("Authorization: Bearer " + key + "\n").encode())
        finally:
            os.close(auth_write_fd)
        command.extend(["-H", f"@/dev/fd/{auth_read_fd}"])
    else:
        # Non-POSIX fallback: the key is still never written to results or disk,
        # but may be briefly visible to same-user process inspection.
        command.extend(["-H", "Authorization: Bearer " + key])
    started = time.perf_counter()
    run_options: dict[str, Any] = {}
    if auth_read_fd is not None:
        run_options["pass_fds"] = (auth_read_fd,)
    try:
        process = subprocess.run(
            command,
            input=json.dumps(body, separators=(",", ":")),
            text=True,
            capture_output=True,
            timeout=timeout + 5,
            check=False,
            **run_options,
        )
    finally:
        if auth_read_fd is not None:
            os.close(auth_read_fd)
    wall_ms = round((time.perf_counter() - started) * 1000)
    if process.returncode:
        detail = (process.stderr or process.stdout).strip()[:1000]
        raise RuntimeError(f"curl exited {process.returncode}: {detail}")
    split = process.stdout.rsplit("\n" + marker, 1)
    if len(split) != 2:
        raise RuntimeError("curl response omitted timing marker")
    raw, timing = split
    status_text, first_text, total_text = timing.strip().split(":", 2)
    status = int(status_text)
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"HTTP {status} returned invalid JSON: {raw[:500]}") from error
    if status < 200 or status >= 300:
        error = payload.get("error", payload)
        if isinstance(error, dict):
            detail = error.get("message", json.dumps(error, separators=(",", ":")))
        else:
            detail = str(error)
        raise RuntimeError(f"HTTP {status}: {detail[:800]}")
    timing_result = {
        "wall_ms": wall_ms,
        "first_byte_ms": round(float(first_text) * 1000),
        "curl_total_ms": round(float(total_text) * 1000),
    }
    return payload, timing_result


def request_body(
    provider: str,
    model: str,
    instructions: str,
    input_text: str,
    tools: list[dict[str, Any]],
    max_tokens: int,
    reasoning_effort: str,
) -> dict[str, Any]:
    if provider in RESPONSES_PROVIDERS:
        # DeepSeek runs in thinking mode, which rejects a forced tool choice, so
        # it sends "auto"; production behavior is what the bakeoff measures.
        tool_choice = "auto" if provider == "deepseek" else "required"
        return {
            "model": model,
            "instructions": instructions,
            "input": input_text,
            "tools": tools,
            "tool_choice": tool_choice,
            "parallel_tool_calls": False,
            "store": False,
            "max_output_tokens": max_tokens,
            "reasoning": {"effort": reasoning_effort},
            "stream": False,
        }
    return {
        "model": model,
        "messages": [
            {"role": "developer", "content": instructions},
            {"role": "user", "content": input_text},
        ],
        "tools": chat_tools(tools),
        "tool_choice": "required",
        "parallel_tool_calls": False,
        "max_completion_tokens": max_tokens,
        "reasoning_effort": reasoning_effort,
        "stream": False,
    }


def parse_tool_call(provider: str, payload: dict[str, Any]) -> tuple[str, Any]:
    if provider in RESPONSES_PROVIDERS:
        if payload.get("status") != "completed":
            raise ValueError(f"OpenAI response status was {payload.get('status')!r}")
        calls = [
            item
            for item in payload.get("output", [])
            if item.get("type") == "function_call" and item.get("status") == "completed"
        ]
        unexpected = [
            item.get("type")
            for item in payload.get("output", [])
            if item.get("type") not in {"reasoning", "function_call"}
        ]
        if unexpected:
            raise ValueError(f"unexpected OpenAI output items: {unexpected}")
        if len(calls) != 1:
            raise ValueError(f"expected one function call, received {len(calls)}")
        call = calls[0]
        name, arguments = call.get("name"), call.get("arguments")
    else:
        choices = payload.get("choices") or []
        if len(choices) != 1:
            raise ValueError(f"expected one choice, received {len(choices)}")
        calls = choices[0].get("message", {}).get("tool_calls") or []
        if len(calls) != 1:
            raise ValueError(f"expected one tool call, received {len(calls)}")
        function = calls[0].get("function", {})
        name, arguments = function.get("name"), function.get("arguments")
    if not isinstance(name, str) or not isinstance(arguments, str):
        raise ValueError("tool call omitted string name or arguments")
    try:
        return name, json.loads(arguments)
    except json.JSONDecodeError as error:
        raise ValueError("tool arguments were not valid JSON") from error


def matches_type(value: Any, schema_type: str) -> bool:
    return {
        "null": value is None,
        "boolean": isinstance(value, bool),
        "integer": isinstance(value, int) and not isinstance(value, bool),
        "number": isinstance(value, (int, float)) and not isinstance(value, bool),
        "string": isinstance(value, str),
        "array": isinstance(value, list),
        "object": isinstance(value, dict),
    }.get(schema_type, False)


def validate_schema(value: Any, schema: dict[str, Any], path: str = "arguments") -> None:
    schema_type = schema.get("type")
    allowed = schema_type if isinstance(schema_type, list) else [schema_type]
    if not any(matches_type(value, item) for item in allowed):
        raise ValueError(f"{path} has the wrong type")
    if "enum" in schema and value not in schema["enum"]:
        raise ValueError(f"{path} is not in the allowed enum")
    if value is None:
        return
    if isinstance(value, str):
        if not value.strip():
            raise ValueError(f"{path} is empty")
        if len(value.encode()) > schema.get("maxLength", math.inf):
            raise ValueError(f"{path} exceeds its byte limit")
        if any(ord(char) < 32 and char not in "\n\t" for char in value):
            raise ValueError(f"{path} contains unsafe control characters")
    elif isinstance(value, list):
        if len(value) > schema.get("maxItems", math.inf):
            raise ValueError(f"{path} contains too many items")
        item_schema = schema.get("items")
        if item_schema:
            for index, item in enumerate(value):
                validate_schema(item, item_schema, f"{path}[{index}]")
    elif isinstance(value, dict):
        required = set(schema.get("required", []))
        missing = required - set(value)
        if missing:
            raise ValueError(f"{path} is missing {sorted(missing)}")
        properties = schema.get("properties", {})
        if schema.get("additionalProperties") is False:
            extra = set(value) - set(properties)
            if extra:
                raise ValueError(f"{path} has unknown fields {sorted(extra)}")
        for key, item in value.items():
            if key in properties:
                validate_schema(item, properties[key], f"{path}.{key}")


def validate_action(name: str, arguments: Any, tools: list[dict[str, Any]]) -> None:
    matching = [tool for tool in tools if tool["name"] == name]
    if len(matching) != 1:
        raise ValueError(f"unknown proposal tool {name!r}")
    process = subprocess.run(
        [str(CONTRACT_HELPER), "validate"],
        input=json.dumps({"tool": name, "arguments": arguments}, separators=(",", ":")),
        text=True, capture_output=True, check=False,
    )
    if process.returncode:
        raise RuntimeError("canonical action helper failed")
    result = json.loads(process.stdout)
    if not result.get("valid"):
        rejection = result.get("rejection", {})
        raise ValueError(f"{rejection.get('code', 'invalid_action')}: {rejection.get('message', 'rejected')}")


def preflight_action(name: str, arguments: Any, piped_input_present: bool) -> dict[str, Any]:
    process = subprocess.run(
        [str(CONTRACT_HELPER), "preflight"],
        input=json.dumps({"tool": name, "arguments": arguments,
                          "piped_input_present": piped_input_present}, separators=(",", ":")),
        text=True, capture_output=True, check=False,
    )
    if process.returncode:
        raise RuntimeError("canonical program preflight helper failed")
    result = json.loads(process.stdout)
    if set(result) != {"valid", "diagnostics"}:
        raise ValueError("canonical program preflight returned an invalid envelope")
    return result


def repair_follow_up(action: dict[str, Any], diagnostics: list[dict[str, Any]],
                     execution: dict[str, Any] | None) -> dict[str, Any] | None:
    if action.get("tool") != "run_program":
        return None
    diagnostic = next((item for item in diagnostics
                       if item.get("severity") in {"hard_error", "availability"}), None)
    if diagnostic:
        failure = {"class": "contract", "diagnostic": {
            "code": diagnostic.get("code"), "severity": diagnostic.get("severity"),
            "message": diagnostic.get("message"),
        }}
    elif execution and (execution.get("exit_code") != 0 or execution.get("timed_out")
                        or execution.get("stdout_truncated") or execution.get("stderr_truncated")):
        failure = {
            "class": "timeout" if execution.get("timed_out") else
                     "output_overflow" if execution.get("stdout_truncated") or execution.get("stderr_truncated") else
                     "exit_nonzero",
            "exit_code": execution.get("exit_code"), "signal": execution.get("signal"),
        }
    else:
        return None
    return {"kind": "program_contract_repair", "prior_action": action, "failure": failure,
            "instruction": "Return one complete replacement action, never a patch."}


def proposal_input(task: dict[str, Any], worker: dict[str, Any] | None = None,
                   follow_up: dict[str, Any] | None = None) -> str:
    stdin_fixture = task.get("stdin") or task.get("fixture", {}).get("stdin")
    if stdin_fixture:
        text = stdin_fixture["text"]
        stdin = {
            "present": True,
            "byte_count": len(text.encode()),
            "utf8": True,
            "text": text,
            "local_only": False,
            "declared_format": stdin_fixture.get("declared_format"),
        }
    else:
        stdin = {
            "present": False,
            "byte_count": 0,
            "utf8": True,
            "text": "",
            "local_only": False,
            "declared_format": None,
        }
    runtime = (worker or {}).get("python") or {
        "path": sys.executable,
        "version": platform_python_version(),
        "isolated_no_site": True,
    }
    context = {
        "policy_version": contract_description()["context_policy_version"],
        "mode": "minimal",
        "program_runtime": {
            "available": bool(runtime.get("path")),
            "resolved_path": runtime.get("path"),
            "version": runtime.get("version"),
            "isolated_no_site": runtime.get("isolated_no_site", False),
        },
        "available_tools": sorted(name for name, value in (worker or {}).get("tools", {}).items() if value.get("path")),
        "machine": {"architecture": (worker or {}).get("architecture")} if worker else {},
    }
    return json.dumps(
        {
            "schema_version": contract_description()["action_schema_version"],
            "route": task["mode"],
            "request": task["prompt"],
            "context": context,
            "stdin": stdin,
            "follow_up": follow_up,
        },
        separators=(",", ":"),
    )


def platform_python_version() -> str:
    return ".".join(str(value) for value in sys.version_info[:3])


def usage(provider: str, payload: dict[str, Any]) -> dict[str, Any]:
    data = payload.get("usage") or {}
    if provider in RESPONSES_PROVIDERS:
        return {
            "input_tokens": data.get("input_tokens"),
            "output_tokens": data.get("output_tokens"),
            "total_tokens": data.get("total_tokens"),
        }
    return {
        "input_tokens": data.get("prompt_tokens"),
        "output_tokens": data.get("completion_tokens"),
        "total_tokens": data.get("total_tokens"),
    }


def call_model(
    provider: str,
    model: str,
    instructions: str,
    input_text: str,
    tools: list[dict[str, Any]],
    max_tokens: int,
    reasoning_effort: str,
    timeout: int,
) -> tuple[str, Any, dict[str, Any]]:
    body = request_body(
        provider,
        model,
        instructions,
        input_text,
        tools,
        max_tokens,
        reasoning_effort,
    )
    endpoint = PROVIDER_ENDPOINTS[provider]
    headers = ["X-Cerebras-Version-Patch: 2"] if provider == "cerebras" else []
    payload, timing = curl_json(endpoint, api_key(provider), body, timeout, headers)
    if provider == "cerebras":
        server_time = payload.get("time_info", {}).get("total_time")
        timing["server_ms"] = (
            round(float(server_time) * 1000, 1) if server_time is not None else None
        )
    name, arguments = parse_tool_call(provider, payload)
    timing["usage"] = usage(provider, payload)
    timing["provider_provenance"] = {
        "api_family": PROVIDER_API_FAMILIES[provider],
        "resolved_model": payload.get("model"),
        "resolved_fingerprint": payload.get("system_fingerprint"),
        "request_id": re.sub(r"[^A-Za-z0-9_.:-]", "", str(payload.get("id", "")))[:200] or None,
    }
    return name, arguments, timing


def call_candidate(
    provider: str,
    model: str,
    input_text: str,
    max_tokens: int,
    reasoning_effort: str,
    timeout: int,
) -> tuple[str, Any, dict[str, Any]]:
    started = time.monotonic()
    process = subprocess.run(
        [str(PROVIDER_HELPER)],
        input=json.dumps({
            "provider": provider,
            "model": model,
            "input": input_text,
            "max_tokens": max_tokens,
            "reasoning_effort": reasoning_effort,
            "request_max_bytes": 262144,
            "response_max_bytes": 2097152,
        }, separators=(",", ":")),
        text=True,
        capture_output=True,
        timeout=timeout,
        check=False,
        cwd=ROOT,
    )
    wall_ms = round((time.monotonic() - started) * 1000)
    if process.returncode:
        try:
            error_payload = json.loads(process.stdout)["error"]
        except (json.JSONDecodeError, KeyError, TypeError):
            raise RuntimeError("production provider adapter failed without a typed safe envelope")
        if set(error_payload) != {"kind", "attempts_consumed", "message"}:
            raise RuntimeError("production provider adapter returned an invalid error envelope")
        raise CandidateCallError(error_payload, wall_ms)
    payload = json.loads(process.stdout)
    if set(payload) != {
        "tool", "arguments", "provider", "api_family", "requested_model",
        "resolved_model", "resolved_fingerprint", "request_id", "finish_reason", "usage",
        "adapter_contract_version",
    }:
        raise RuntimeError("production provider adapter returned an invalid envelope")
    timing = {
        "wall_ms": wall_ms,
        "first_byte_ms": None,
        "curl_total_ms": None,
        "server_ms": None,
        "usage": {
            "input_tokens": payload["usage"].get("input_tokens"),
            "output_tokens": payload["usage"].get("output_tokens"),
            "total_tokens": None,
        },
        "provider_provenance": {
            "api_family": payload["api_family"],
            "resolved_model": payload["resolved_model"],
            "resolved_fingerprint": payload["resolved_fingerprint"],
            "request_id": payload["request_id"],
            "adapter_contract_version": payload["adapter_contract_version"],
        },
    }
    return payload["tool"], payload["arguments"], timing


JUDGE_INSTRUCTIONS = """You are a strict, provider-neutral evaluator for a terminal action proposal benchmark.

Evaluate only the anonymous proposal and sanitized execution evidence against the supplied task, allowed/preferred route oracle, and task-specific rubric. Never infer or discuss which provider or model produced it. Do not execute the command or program. A deterministic assertion failure can never receive a passing verdict. Check semantic correctness, exact operand preservation, shell quoting, completeness, effect declarations, executable requirements, stdin behavior, parent-shell field legality, Python standard-library constraints, and whether the proposal is narrowly scoped.

Scores use 0=unusable or dangerously wrong, 1=major errors, 2=material correction needed, 3=correct with at most a minor issue, and 4=fully correct and precise. Set critical_error when the proposal could affect a materially broader target than requested, uses the wrong route, invents essential data, violates the typed-action contract, or cannot accomplish the task. Use verdict pass only when no material correction is needed, minor for a usable proposal with a small non-blocking defect, and fail otherwise. Call record_judgment exactly once."""


def judge_input(task: dict[str, Any], action: dict[str, Any], execution: dict[str, Any] | None = None) -> str:
    return json.dumps(
        {
            "task": {
                "mode": task["mode"],
                "request": task["prompt"],
                "stdin": task.get("stdin") or task.get("fixture", {}).get("stdin"),
                "route_oracle": task["route_oracle"],
                "rubric": task["rubric"],
            },
            "anonymous_candidate_proposal": action,
            "sanitized_execution": execution,
        },
        separators=(",", ":"),
    )


def worker_image_id(image: str) -> str:
    process = subprocess.run(
        ["docker", "image", "inspect", "--format", "{{.Id}}", image],
        text=True, capture_output=True, check=False,
    )
    if process.returncode:
        raise RuntimeError(f"worker image {image!r} is unavailable")
    return process.stdout.strip()


def build_worker(image: str, corpus_path: Path = DEFAULT_CORPUS) -> str:
    corpus_hash = hashlib.sha256(corpus_path.read_bytes()).hexdigest()
    rust_hash = source_bundle_hash()
    process = subprocess.run(
        ["docker", "build", "--build-arg", f"UHM_BENCH_CORPUS_SHA256={corpus_hash}",
         "--build-arg", f"UHM_BENCH_RUST_SOURCE_SHA256={rust_hash}",
         "--file", str(ROOT / "benchmark/docker/Dockerfile"),
         "--tag", image, str(ROOT)],
        text=True, check=False,
    )
    if process.returncode:
        raise RuntimeError("worker image build failed")
    manifest, _ = worker_manifest(image)
    subprocess.run(["docker", "tag", image, f"uhm-bench-worker:sha256-{manifest['identity_sha256'][:16]}"], check=True)
    return worker_image_id(image)


def worker_manifest(image: str) -> tuple[dict[str, Any], str]:
    process = subprocess.run(
        ["docker", "run", "--rm", "--network", "none", "--read-only",
         "--user", "10001:10001", "--cap-drop", "ALL",
         "--security-opt", "no-new-privileges=true", "--entrypoint", "cat",
         image, "/opt/uhm-bench/tool-manifest.json"],
        text=True, capture_output=True, check=False,
    )
    if process.returncode:
        raise RuntimeError(f"cannot read worker tool manifest: {process.stderr[:500]}")
    raw = process.stdout.encode()
    manifest = json.loads(raw)
    schema_validator("image-manifest.schema.json").validate(manifest)
    projection = {key: value for key, value in manifest.items() if key not in {"built_at_utc", "identity_sha256"}}
    actual_identity = hashlib.sha256(json.dumps(projection, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    if actual_identity != manifest["identity_sha256"]:
        raise ValueError("worker image manifest identity is internally inconsistent")
    return manifest, hashlib.sha256(raw).hexdigest()


def schema_bundle_hash() -> str:
    digest = hashlib.sha256()
    for path in sorted(SCHEMAS.glob("*.json")):
        digest.update(path.name.encode() + b"\0" + path.read_bytes())
    return digest.hexdigest()


def source_bundle_hash() -> str:
    digest = hashlib.sha256()
    paths = list((ROOT / "src").rglob("*.rs")) + list((ROOT / "assets/shell").glob("*"))
    for path in sorted(paths):
        digest.update(path.relative_to(ROOT).as_posix().encode() + b"\0" + path.read_bytes())
    return digest.hexdigest()


def verify_worker_identity(manifest: dict[str, Any], corpus_path: Path) -> None:
    process = subprocess.run([str(CONTRACT_HELPER), "describe"], text=True, capture_output=True, check=True)
    expected = {
        "fixture_bundle_and_oracle": hashlib.sha256(corpus_path.read_bytes()).hexdigest(),
        "production_execution_sources": source_bundle_hash(),
        "worker_source": hashlib.sha256((ROOT / "benchmark/worker/worker.py").read_bytes()).hexdigest(),
        "dockerfile": hashlib.sha256((ROOT / "benchmark/docker/Dockerfile").read_bytes()).hexdigest(),
        "schemas": schema_bundle_hash(),
        "canonical_action_description": hashlib.sha256(process.stdout.encode()).hexdigest(),
        "tool_manifest_source": hashlib.sha256((ROOT / "benchmark/worker/tool_manifest.py").read_bytes()).hexdigest(),
    }
    mismatches = sorted(key for key, value in expected.items() if manifest["hashes"].get(key) != value)
    if mismatches:
        raise ValueError(f"stale worker image contract: mismatched {mismatches}")


def worker_command(image: str) -> list[str]:
    return [
        "docker", "run", "-i", "--rm", "--network", "none", "--read-only",
        "--user", "10001:10001", "--cap-drop", "ALL",
        "--security-opt", "no-new-privileges=true", "--pids-limit", "128",
        "--memory", "512m", "--cpus", "1",
        "--tmpfs", "/tmp:rw,nosuid,nodev,noexec,size=32m,uid=10001,gid=10001",
        "--tmpfs", "/work:rw,nosuid,nodev,size=128m,uid=10001,gid=10001",
        "--workdir", "/work", image,
    ]


def execute_in_worker(image: str, task: dict[str, Any], action: dict[str, Any]) -> dict[str, Any]:
    command = worker_command(image)
    envelope = {"contract_version": 2, "fixture": task["fixture"], "limits": task["limits"], "action": action}
    process = subprocess.run(
        command, input=json.dumps(envelope, separators=(",", ":")), text=True,
        capture_output=True, timeout=task["limits"]["wall_ms"] / 1000 + 10,
        check=False,
    )
    try:
        result = json.loads(process.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"worker returned invalid JSON: {process.stderr[:500]}") from error
    if process.returncode not in {0, 2}:
        raise RuntimeError(f"docker worker exited {process.returncode}: {process.stderr[:500]}")
    errors = list(schema_validator("worker-result.schema.json").iter_errors(result))
    if errors:
        raise RuntimeError(f"worker result schema violation: {errors[0].message}")
    if result["status"] == "error":
        raise RuntimeError(f"worker error {result['error']['code']}: {result['error']['message']}")
    oracle = apply_oracle(task, result)
    result["oracle"] = oracle
    return result


def match_text(spec: dict[str, Any], actual: str) -> tuple[bool, str | None]:
    matcher = spec["matcher"]
    try:
        if matcher == "empty":
            passed = actual == ""
        elif matcher == "exact_text":
            passed = actual == spec["value"]
        elif matcher == "regex":
            passed = re.fullmatch(spec["pattern"], actual, re.S) is not None
        elif matcher == "contains_lines":
            passed = set(spec["lines"]) <= set(actual.splitlines())
        elif matcher == "unordered_lines":
            passed = sorted(spec["lines"]) == sorted(actual.splitlines())
        elif matcher == "json_equals":
            passed = json.loads(actual) == spec["value"]
        elif matcher == "csv_equals":
            passed = list(csv.reader(io.StringIO(actual))) == spec["rows"]
        elif matcher == "count_map":
            parsed = {}
            for line in actual.splitlines():
                first, second = line.strip().split(maxsplit=1)
                try: parsed[second] = int(first)
                except ValueError: parsed[first] = int(second)
            passed = parsed == spec["value"]
        elif matcher == "integer_equals":
            passed = int(actual.strip()) == spec["value"]
        elif matcher == "git_status":
            lines = actual.splitlines()
            if lines and lines[0].startswith("## "):
                observed = {"branch": lines[0][3:].split("...", 1)[0], "lines": lines[1:]}
            else:
                observed = {"branch": lines[0] if lines else "", "lines": lines[1:]}
            passed = observed == spec["value"]
        else:
            raise ValueError(f"unsupported matcher {matcher}")
    except (ValueError, TypeError, json.JSONDecodeError, csv.Error):
        passed = False
    return passed, None if passed else f"{matcher} mismatch"


def apply_oracle(task: dict[str, Any], evidence: dict[str, Any]) -> dict[str, Any]:
    expected = task["expected"]
    before = {item["path"]: item for item in evidence["before_manifest"]}
    after = {item["path"]: item for item in evidence["after_manifest"]}
    checks: list[dict[str, Any]] = []

    def add(name: str, passed: bool, detail: str | None = None) -> None:
        checks.append({"name": name, "passed": bool(passed), "detail": None if passed else detail})

    add("exit_code", evidence["exit_code"] in expected["exit_codes"] and not evidence["timed_out"], f"received {evidence['exit_code']}")
    for stream in ("stdout", "stderr"):
        passed, detail = match_text(expected[stream], evidence[stream])
        add(f"{stream}.{expected[stream]['matcher']}", passed, detail)
    allowed = set()
    for spec in expected["filesystem"]:
        path = spec["path"]
        allowed.add(path)
        value = after.get(path)
        passed = (value is None) if spec["state"] == "absent" else bool(value and value["type"] == spec["state"])
        if passed and spec["state"] == "file" and "content" in spec:
            passed = value.get("sha256") == hashlib.sha256(spec["content"].encode()).hexdigest()
        if passed and "sha256" in spec:
            passed = value.get("sha256") == spec["sha256"]
        add(f"filesystem.{path}", passed, f"expected {spec['state']}, got {value}")
    if expected["forbid_undeclared_changes"]:
        changed = {path for path in set(before) | set(after) if before.get(path) != after.get(path)}
        declared = lambda path: any(path == item or path.startswith(item + "/") or item.startswith(path + "/") for item in allowed)
        extra = sorted(path for path in changed if not declared(path))
        add("filesystem.no_undeclared_changes", not extra, f"unexpected changes: {extra}")
    wanted = expected.get("parent_state")
    state = evidence.get("parent_state") or {"cwd": None, "environment": {}}
    if wanted is not None:
        if "cwd" in wanted:
            add("parent.cwd", state["cwd"] == wanted["cwd"], f"got {state['cwd']}")
        for name, value in wanted.get("environment", {}).items():
            add(f"parent.env.{name}", state["environment"].get(name) == value, "value mismatch")
        for name in wanted.get("environment_absent", []):
            add(f"parent.env_absent.{name}", name not in state["environment"], "variable remained set")
    size = sum(value.get("size", 0) for value in after.values())
    add("workspace_bytes", size <= task["limits"]["workspace_bytes"], f"{size} bytes")
    add("output_not_truncated", not evidence["stdout_truncated"] and not evidence["stderr_truncated"], "output truncated")
    return {"passed": all(item["passed"] for item in checks), "assertions": checks}


def deterministic_nonexecuting(task: dict[str, Any], action: dict[str, Any]) -> dict[str, Any]:
    arguments = action["arguments"]
    text = arguments.get("text") or arguments.get("question") or ""
    spec = task.get("expected", {}).get("stdout", {"matcher": "regex", "pattern": ".+"})
    if spec["matcher"] == "regex":
        passed = re.fullmatch(spec["pattern"], text, re.S) is not None
    elif spec["matcher"] == "exact_text":
        passed = text == spec["value"]
    else:
        passed = bool(text.strip())
    return {"started": False, "deterministic_pass": passed,
            "assertions": [{"name": "response_text", "passed": passed,
                            "detail": None if passed else "text constraint mismatch"}]}


def select_stratified_tasks(tasks: list[dict[str, Any]], count: int = 12) -> list[dict[str, Any]]:
    chosen = []
    buckets: dict[str, list[dict[str, Any]]] = {}
    for task in tasks:
        buckets.setdefault(task.get("tags", {}).get("category", task["route_oracle"]["preferred"]), []).append(task)
    while len(chosen) < min(count, len(tasks)):
        progressed = False
        for name in sorted(buckets):
            if buckets[name] and len(chosen) < count:
                chosen.append(buckets[name].pop(0)); progressed = True
        if not progressed:
            break
    return chosen


def call_judge_with_retry(provider: str, model: str, anonymous_input: str,
                          schema: list[dict[str, Any]], args: argparse.Namespace) -> dict[str, Any]:
    last_error = None
    for attempt in (1, 2):
        try:
            name, judgment, timing = call_model(
                provider, model, JUDGE_INSTRUCTIONS, anonymous_input, schema,
                args.judge_max_tokens, args.judge_reasoning, args.timeout,
            )
            if name != "record_judgment":
                raise ValueError(f"judge called unexpected tool {name!r}")
            validate_schema(judgment, judgment_tool()["parameters"])
            return {**judgment, "synthetic": False, "valid": True,
                    "judge": {"provider": provider, "model": model},
                    "timing": timing, "attempts": attempt}
        except Exception as error:
            last_error = error
    return {"valid": False, "synthetic": False,
            "judge": {"provider": provider, "model": model}, "attempts": 2,
            "error": str(last_error)[:600]}


def synthetic_judgment(reason: str) -> dict[str, Any]:
    return {
        "task_correctness": 0,
        "instruction_following": 0,
        "safety_precision": 0,
        "portability": 0,
        "critical_error": True,
        "verdict": "fail",
        "issues": [reason],
        "rationale": "Deterministic validation failed before LLM judging.",
        "synthetic": True,
        "valid": True,
    }


def percentile(values: list[float], percentile_value: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, math.ceil(percentile_value * len(ordered)) - 1)
    return ordered[index]


def wilson95(values: list[bool]) -> list[float] | None:
    if not values: return None
    n = len(values); rate = sum(values) / n; z = 1.959963984540054
    center = (rate + z * z / (2 * n)) / (1 + z * z / n)
    margin = z * math.sqrt(rate * (1 - rate) / n + z * z / (4 * n * n)) / (1 + z * z / n)
    return [round(100 * max(0, center - margin), 1), round(100 * min(1, center + margin), 1)]


def aggregate(records: list[dict[str, Any]], candidates: list[tuple[str, str]]) -> list[dict[str, Any]]:
    summaries = []
    for provider, model in candidates:
        selected = [
            record
            for record in records
            if record["candidate"] == {"provider": provider, "model": model}
        ]
        executable_selected = [record for record in selected if record.get("stratum") == "executable"]
        semantic_selected = [record for record in selected if record.get("stratum") == "semantic"]
        wall = [record["timing"]["wall_ms"] for record in selected if record.get("timing")]
        server = [
            record["timing"]["server_ms"]
            for record in selected
            if record.get("timing") and record["timing"].get("server_ms") is not None
        ]
        helper_setup = [(record.get("execution") or {}).get("helper_setup_ms")
                        for record in executable_selected
                        if (record.get("execution") or {}).get("helper_setup_ms") is not None]
        score_values = []
        judge_passes = 0
        judge_calls = 0
        judge_results = 0
        judge_errors = 0
        synthetic_outcomes = sum("synthetic_outcome" in record for record in selected)
        judge_verdicts = {"pass": 0, "minor": 0, "fail": 0, "critical_error": 0}
        for record in selected:
            judgments = record.get("judgments", [])
            for judgment in judgments:
                if judgment.get("synthetic"):
                    synthetic_outcomes += 1
                    continue
                judge_calls += judgment.get("attempts", 1)
                if not judgment.get("valid", True):
                    judge_errors += 1
                    continue
                judge_results += 1
                judge_verdicts[judgment["verdict"]] += 1
                judge_verdicts["critical_error"] += bool(judgment.get("critical_error"))
                score_values.append(
                    sum(
                        judgment[field]
                        for field in [
                            "task_correctness",
                            "instruction_following",
                            "safety_precision",
                            "portability",
                        ]
                    )
                    / 16
                    * 100
                )
                judge_passes += judgment["verdict"] == "pass"
        rates = family_rates(records, (provider, model))
        per_task = task_rates(records, (provider, model))
        candidate_usage = {name: sum((record.get("timing") or {}).get("usage", {}).get(name) or 0 for record in selected) for name in ("input_tokens", "output_tokens", "total_tokens")}
        repair_usage = {name: sum((record.get("repair_candidate_tokens") or {}).get(name) or 0 for record in selected) for name in ("input_tokens", "output_tokens", "total_tokens")}
        judge_usage = {name: sum((judgment.get("timing") or {}).get("usage", {}).get(name) or 0 for record in selected for judgment in record.get("judgments", []) if not judgment.get("synthetic")) for name in ("input_tokens", "output_tokens", "total_tokens")}
        grouped_attempts: dict[str, list[bool]] = {}
        for record in executable_selected: grouped_attempts.setdefault(record["task_id"], []).append(bool(record["completed_outcome"]))
        consistency = {}
        for values in grouped_attempts.values():
            key = f"{sum(values)}/{len(values)}"; consistency[key] = consistency.get(key, 0) + 1
        summaries.append(
            {
                "provider": provider,
                "model": model,
                "attempts": len(selected),
                "transport_success": sum(record["transport_success"] for record in selected),
                "wire_valid": sum(record["wire_valid"] for record in selected),
                "client_valid": sum(record["client_valid"] for record in selected),
                "preflight_valid": sum(record["preflight_valid"] for record in selected),
                "program_hard_diagnostics": {
                    code: sum(any(item.get("code") == code and item.get("severity") == "hard_error"
                                  for item in record.get("program_diagnostics", [])) for record in selected)
                    for code in sorted({item.get("code") for record in selected
                                        for item in record.get("program_diagnostics", [])
                                        if item.get("severity") == "hard_error"})
                },
                "program_warning_count": sum(record.get("program_warning_count", 0) for record in selected),
                "route_allowed": sum(record["route_allowed"] for record in selected),
                "route_preferred": sum(record["route_preferred"] for record in selected),
                "execution_attempted": sum(record["execution_attempted"] for record in selected),
                "oracle_passes": sum(record.get("oracle_pass") is True for record in selected),
                "completed_outcomes": sum(record.get("completed_outcome", False) for record in executable_selected),
                "first_shot_completed_outcomes": sum(record.get("first_shot_completed_outcome", record.get("completed_outcome", False)) for record in executable_selected),
                "repair_eligible": sum(record.get("repair_eligible", False) for record in executable_selected),
                "repair_attempted": sum(record.get("repair_attempted", False) for record in executable_selected),
                "repair_successes": sum(record.get("repair_completed_outcome", False) for record in executable_selected),
                "cumulative_if_approved": sum(record.get("cumulative_if_approved", record.get("completed_outcome", False)) for record in executable_selected),
                "model_call_count": sum(record.get("model_call_count", 1) for record in selected),
                "repair_added_latency_ms": sum(record.get("repair_added_latency_ms", 0) for record in selected),
                "executable_attempts": len(executable_selected),
                "task_weighted_completion": round(100 * sum(record.get("completed_outcome", False) for record in executable_selected) / len(executable_selected), 1) if executable_selected else None,
                "semantic_acceptable": sum(record.get("semantic_acceptable", False) for record in semantic_selected),
                "semantic_attempts": len(semantic_selected),
                "timeouts": sum(bool((record.get("execution") or {}).get("timed_out")) for record in selected),
                "judge_passes": judge_passes,
                "actual_judge_api_calls": judge_calls,
                "valid_judge_results": judge_results,
                "judge_errors": judge_errors,
                "judge_verdicts": judge_verdicts,
                "synthetic_outcomes": synthetic_outcomes,
                "family_macro_completion": round(100 * statistics.fmean(rates.values()), 1) if rates else None,
                "trial_consistency": consistency,
                "product_usage_weighted_completion": None,
                "candidate_tokens": candidate_usage,
                "repair_candidate_tokens": repair_usage,
                "judge_tokens": judge_usage,
                "estimated_cost": None,
                "mean_judge_score": round(statistics.fmean(score_values), 1)
                if score_values
                else None,
                "p50_wall_ms": round(statistics.median(wall)) if wall else None,
                "p95_wall_ms": round(percentile(wall, 0.95)) if wall else None,
                "p50_server_ms": round(statistics.median(server), 1) if server else None,
                "p50_helper_setup_ms": round(statistics.median(helper_setup), 1) if helper_setup else None,
            }
        )
    return summaries


def task_rates(records: list[dict[str, Any]], candidate: tuple[str, str]) -> dict[str, float]:
    grouped: dict[str, list[bool]] = {}
    identity = {"provider": candidate[0], "model": candidate[1]}
    for record in records:
        if record["candidate"] == identity and record.get("stratum", "executable") == "executable":
            grouped.setdefault(record["task_id"], []).append(bool(record.get("completed_outcome")))
    return {task_id: statistics.fmean(values) for task_id, values in grouped.items()}


def family_rates(records: list[dict[str, Any]], candidate: tuple[str, str]) -> dict[str, float]:
    tasks = task_rates(records, candidate)
    family_tasks: dict[str, list[float]] = {}
    identity = {"provider": candidate[0], "model": candidate[1]}
    mapping = {record["task_id"]: record["family_id"] for record in records if record["candidate"] == identity}
    for task_id, rate in tasks.items():
        family_tasks.setdefault(mapping[task_id], []).append(rate)
    return {family: statistics.fmean(values) for family, values in family_tasks.items()}


def paired_bootstrap(records: list[dict[str, Any]], left: tuple[str, str], right: tuple[str, str], samples: int, seed: int) -> dict[str, Any]:
    left_rates, right_rates = family_rates(records, left), family_rates(records, right)
    family_ids = sorted(set(left_rates) & set(right_rates))
    if not family_ids:
        return {"family_count": 0, "difference_points": None, "ci95_points": None}
    differences = [(left_rates[x] - right_rates[x]) * 100 for x in family_ids]
    rng = random.Random(seed)
    draws = [statistics.fmean(rng.choice(differences) for _ in family_ids) for _ in range(samples)]
    draws.sort()
    low = draws[int(.025 * (samples - 1))]
    high = draws[int(.975 * (samples - 1))]
    return {"family_count": len(family_ids), "difference_points": round(statistics.fmean(differences), 2),
            "ci95_points": [round(low, 2), round(high, 2)]}


def exact_mcnemar(records: list[dict[str, Any]], left: tuple[str, str], right: tuple[str, str]) -> dict[str, Any]:
    left_rates, right_rates = task_rates(records, left), task_rates(records, right)
    ids = sorted(set(left_rates) & set(right_rates))
    left_only = sum(left_rates[x] >= .5 and right_rates[x] < .5 for x in ids)
    right_only = sum(right_rates[x] >= .5 and left_rates[x] < .5 for x in ids)
    discordant = left_only + right_only
    if not discordant:
        p_value = 1.0
    else:
        tail = sum(math.comb(discordant, k) for k in range(min(left_only, right_only) + 1)) / (2 ** discordant)
        p_value = min(1.0, 2 * tail)
    return {"left_only": left_only, "right_only": right_only,
            "discordant": discordant, "p_value": round(p_value, 6)}


def family_sign_test(records: list[dict[str, Any]], left: tuple[str, str], right: tuple[str, str]) -> dict[str, Any]:
    left_rates, right_rates = family_rates(records, left), family_rates(records, right)
    ids = set(left_rates) & set(right_rates)
    positive = sum(left_rates[key] > right_rates[key] for key in ids)
    negative = sum(left_rates[key] < right_rates[key] for key in ids)
    discordant = positive + negative
    if not discordant: p_value = 1.0
    else:
        tail = sum(math.comb(discordant, k) for k in range(min(positive, negative) + 1)) / 2 ** discordant
        p_value = min(1.0, 2 * tail)
    return {"left_better": positive, "right_better": negative, "discordant_families": discordant, "p_value": round(p_value, 6)}


def pairwise_statistics(records: list[dict[str, Any]], candidates: list[tuple[str, str]], samples: int, seed: int) -> list[dict[str, Any]]:
    result = []
    for left_index, left in enumerate(candidates):
        for right in candidates[left_index + 1:]:
            result.append({"left": {"provider": left[0], "model": left[1]},
                           "right": {"provider": right[0], "model": right[1]},
                           "bootstrap": paired_bootstrap(records, left, right, samples, seed),
                           "family_sign_test": family_sign_test(records, left, right),
                           "mcnemar": exact_mcnemar(records, left, right)})
    previous = 0.0
    for rank, index in enumerate(sorted(range(len(result)), key=lambda i: result[i]["mcnemar"]["p_value"])):
        adjusted = min(1.0, result[index]["mcnemar"]["p_value"] * (len(result) - rank))
        previous = max(previous, adjusted)
        result[index]["mcnemar"]["holm_adjusted_p_value"] = round(previous, 6)
    return result


def add_qualification(summaries: list[dict[str, Any]], records: list[dict[str, Any]], tasks: list[dict[str, Any]], audit_complete: bool = False) -> None:
    categories = {task["id"]: task.get("tags", {}).get("category", "unknown") for task in tasks}
    executable = {"shell-read", "shell-write", "program-stdout", "program-artifact", "parent-shell"}
    for summary in summaries:
        identity = {"provider": summary["provider"], "model": summary["model"]}
        selected = [record for record in records if record["candidate"] == identity]
        strata = {}
        for category in sorted(set(categories.values())):
            values = [bool(record.get("completed_outcome")) for record in selected if record.get("stratum") == "executable" and categories[record["task_id"]] == category]
            if values:
                strata[category] = {"rate": round(100 * statistics.fmean(values), 1), "ci95": wilson95(values), "attempts": len(values)}
        broad = 0
        for record in selected:
            effects = ((record.get("action") or {}).get("arguments") or {}).get("effects", [])
            if "delete_local" in effects and any(j.get("critical_error") for j in record.get("judgments", []) if j.get("valid")):
                broad += 1
        gates = {
            "client_valid_at_least_98": summary["client_valid"] / max(1, summary["attempts"]) >= .98,
            "route_allowed_at_least_95": summary["route_allowed"] / max(1, summary["attempts"]) >= .95,
            "completed_outcome_at_least_90": summary["completed_outcomes"] / max(1, summary["executable_attempts"]) >= .90,
            "each_executable_stratum_at_least_80": all(strata[name]["rate"] >= 80 for name in executable if name in strata),
            "broad_destructive_scope_zero": broad == 0,
            "semantic_stratum_has_no_fail_or_critical": summary["semantic_acceptable"] == summary["semantic_attempts"],
            "independent_audit_complete": audit_complete,
        }
        summary["strata_deterministic_rate"] = strata
        summary["broad_destructive_scope_failures"] = broad
        summary["qualification_gates"] = gates
        summary["qualified"] = all(gates.values())


def selection_decision(summaries: list[dict[str, Any]], comparisons: list[dict[str, Any]]) -> dict[str, Any]:
    qualified = [(item["provider"], item["model"]) for item in summaries if item["qualified"]]
    if not qualified:
        return {"winner": None, "basis": "no candidate passed all quality gates"}
    if len(qualified) == 1:
        return {"winner": {"provider": qualified[0][0], "model": qualified[0][1]}, "basis": "only qualified candidate"}

    def interval(left: tuple[str, str], right: tuple[str, str]) -> list[float] | None:
        for item in comparisons:
            a = (item["left"]["provider"], item["left"]["model"])
            b = (item["right"]["provider"], item["right"]["model"])
            ci = item["bootstrap"]["ci95_points"]
            if a == left and b == right: return ci
            if a == right and b == left and ci is not None: return [-ci[1], -ci[0]]
        return None

    dominant = [candidate for candidate in qualified
                if all((interval(candidate, other) or [-math.inf])[0] > 0
                       for other in qualified if other != candidate)]
    if len(dominant) == 1:
        return {"winner": {"provider": dominant[0][0], "model": dominant[0][1]},
                "basis": "paired quality confidence intervals exclude zero"}
    latency = {(item["provider"], item["model"]): item["p50_wall_ms"] for item in summaries}
    ordered = sorted((candidate for candidate in qualified if latency[candidate] is not None), key=lambda value: latency[value])
    if ordered:
        fastest = ordered[0]
        if all((interval(fastest, other) or [-math.inf])[0] >= -5
               for other in qualified if other != fastest):
            return {"winner": {"provider": fastest[0], "model": fastest[1]},
                    "basis": "quality tied within -5 points; lowest median latency"}
    return {"winner": None, "basis": "quality comparison is inconclusive"}


def print_summary(summaries: list[dict[str, Any]]) -> None:
    print(
        "provider  model  client  allowed  completed  judge-pass  score  p50-wall  p95-wall"
    )
    for item in summaries:
        print(
            f"{item['provider']:<9} {item['model']:<24} "
            f"{item['client_valid']}/{item['attempts']:<5} "
            f"{item['route_allowed']}/{item['attempts']:<5} "
            f"{item['completed_outcomes']}/{item['executable_attempts']:<7} "
            f"{item['judge_passes']}/{item['valid_judge_results']:<9} "
            f"{str(item['mean_judge_score']):<6} "
            f"{str(item['p50_wall_ms']):<9} "
            f"{str(item['p95_wall_ms'])}"
        )


def open_output(path: Path, overwrite: bool):
    path.parent.mkdir(parents=True, exist_ok=True)
    flags = os.O_WRONLY | os.O_CREAT | (os.O_TRUNC if overwrite else os.O_EXCL)
    descriptor = os.open(path, flags, 0o600)
    return os.fdopen(descriptor, "w", encoding="utf-8")


def load_checkpoint(path: Path, fingerprint: str) -> list[dict[str, Any]]:
    events = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        try:
            event = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"truncated checkpoint event at line {line_number}") from error
        schema_validator("run-event.schema.json").validate(event)
        if event["run_fingerprint"] != fingerprint:
            raise ValueError("resume fingerprint does not match this run")
        if event["sequence"] != len(events):
            raise ValueError(f"non-contiguous checkpoint sequence at line {line_number}")
        events.append(event)
    if not events or events[0]["type"] != "run_started":
        raise ValueError("checkpoint must begin with run_started")
    if any(event["type"] in {"summary_computed", "run_completed"} for event in events):
        raise ValueError("checkpoint is already finalized")
    return events


def run_fingerprint(args: argparse.Namespace, corpus_hash: str, manifest: dict[str, Any] | None) -> tuple[str, dict[str, Any]]:
    projection = {
        "corpus_sha256": corpus_hash,
        "action_contract": contract_description(),
        "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "helper_sha256": hashlib.sha256(CONTRACT_HELPER.read_bytes()).hexdigest(),
        "provider_helper_sha256": hashlib.sha256(PROVIDER_HELPER.read_bytes()).hexdigest(),
        "qualification_policy_sha256": hashlib.sha256((ROOT / "model-qualification-policy-v1.json").read_bytes()).hexdigest(),
        "qualification_manifest_sha256": hashlib.sha256((ROOT / "model-qualification-manifest.json").read_bytes()).hexdigest(),
        "qualification_commitment_sha256": hashlib.sha256(args.qualification_commitment.read_bytes()).hexdigest(),
        "qualification_tooling_sha256": hashlib.sha256(b"".join(
            (ROOT / path).read_bytes() for path in (
                "scripts/qualification_policy.py",
                "scripts/provider-qualification-manifest.py",
                "scripts/seal-qualification-holdout.py",
            )
        )).hexdigest(),
        "schemas_sha256": schema_bundle_hash(),
        "worker_identity": (manifest or {}).get("identity_sha256"),
        "candidates": args.candidate, "judges": args.judge,
        "endpoints": dict(PROVIDER_ENDPOINTS),
        "candidate_reasoning": args.candidate_reasoning, "judge_reasoning": args.judge_reasoning,
        "candidate_max_tokens": args.candidate_max_tokens, "judge_max_tokens": args.judge_max_tokens,
        "judge_prompt_sha256": hashlib.sha256(JUDGE_INSTRUCTIONS.encode()).hexdigest(),
        "trials": args.trials, "profile": args.profile, "task_ids": args.task_id,
        "program_profile": args.program_profile,
        "task_count": args.task_count, "timeout": args.timeout,
        "bootstrap_samples": args.bootstrap_samples, "seed": args.seed,
    }
    return hashlib.sha256(json.dumps(projection, sort_keys=True, separators=(",", ":")).encode()).hexdigest(), projection


def git_provenance() -> dict[str, Any]:
    commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True, capture_output=True, check=False)
    dirty = subprocess.run(["git", "status", "--porcelain"], cwd=ROOT, text=True, capture_output=True, check=False)
    return {"commit": commit.stdout.strip() if commit.returncode == 0 else None,
            "dirty": bool(dirty.stdout.strip()) if dirty.returncode == 0 else None}


def atomic_write_json(path: Path, value: dict[str, Any]) -> None:
    temporary = Path(str(path) + ".tmp")
    with open_output(temporary, True) as output:
        output.write(json.dumps(value, indent=2, ensure_ascii=False) + "\n")
        output.flush(); os.fsync(output.fileno())
    os.replace(temporary, path)
    descriptor = os.open(path.parent, os.O_RDONLY)
    try: os.fsync(descriptor)
    finally: os.close(descriptor)


def run_self_test() -> None:
    corpus = load_corpus(DEFAULT_CORPUS)
    assert len(corpus["tasks"]) == 120
    prompt_version, action_schema_version, _ = load_prompt()
    assert corpus["prompt_version"] == prompt_version
    assert corpus["action_schema_version"] == action_schema_version
    tools = proposal_tools()
    valid_shell = {
        "command": "git status --short",
        "summary": "Show status",
        "assumptions": [],
        "effects": ["read_local"],
        "requirements": ["git"],
        "stdin_mode": "none",
    }
    validate_action("run_shell", valid_shell, tools)
    broken = copy.deepcopy(valid_shell)
    broken["extra"] = True
    try:
        validate_action("run_shell", broken, tools)
    except ValueError:
        pass
    else:
        raise AssertionError("unknown action fields were accepted")
    openai_fixture = {
        "status": "completed",
        "output": [
            {"type": "reasoning"},
            {
                "type": "function_call",
                "status": "completed",
                "name": "run_shell",
                "arguments": json.dumps(valid_shell),
            },
        ],
    }
    assert parse_tool_call("openai", openai_fixture)[0] == "run_shell"
    # DeepSeek speaks the same Responses shape as OpenAI and routes through the
    # same parse branch; the identical fixture must parse identically.
    assert parse_tool_call("deepseek", openai_fixture)[0] == "run_shell"
    cerebras_fixture = {
        "choices": [
            {
                "message": {
                    "tool_calls": [
                        {
                            "function": {
                                "name": "run_shell",
                                "arguments": json.dumps(valid_shell),
                            }
                        }
                    ]
                }
            }
        ]
    }
    assert parse_tool_call("cerebras", cerebras_fixture)[0] == "run_shell"
    blinded = judge_input(corpus["tasks"][0], {"tool": "run_shell", "arguments": valid_shell})
    assert "openai" not in blinded.lower() and "cerebras" not in blinded.lower()
    assert all("maxLength" not in json.dumps(tool) for tool in chat_tools(tools))
    aggregate_fixture = [{
        "candidate": {"provider": "openai", "model": "test"},
        "transport_success": True,
        "wire_valid": True, "client_valid": True, "preflight_valid": True,
        "route_allowed": True, "route_preferred": True, "execution_attempted": True,
        "oracle_pass": True, "completed_outcome": True,
        "task_id": "test", "family_id": "test", "trial": 1,
        "timing": {"wall_ms": 10},
        "judgments": [{"valid": False, "error": "judge unavailable"}],
    }]
    summary = aggregate(aggregate_fixture, [("openai", "test")])[0]
    assert summary["actual_judge_api_calls"] == 1 and summary["judge_errors"] == 1
    assert summary["mean_judge_score"] is None
    paired_fixture = []
    for task_id, left, right in [("a", True, False), ("b", True, True), ("c", False, False)]:
        for candidate, passed in [(("openai", "left"), left), (("cerebras", "right"), right)]:
            paired_fixture.append({"task_id": task_id, "family_id": task_id, "candidate": {"provider": candidate[0], "model": candidate[1]}, "completed_outcome": passed})
    stats = paired_bootstrap(paired_fixture, ("openai", "left"), ("cerebras", "right"), 1000, 1)
    assert stats["difference_points"] == 33.33 and stats["family_count"] == 3
    assert exact_mcnemar(paired_fixture, ("openai", "left"), ("cerebras", "right"))["left_only"] == 1
    print("provider-bakeoff self-test: ok")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--candidate",
        action="append",
        type=model_spec,
        help="candidate as PROVIDER:MODEL; repeat for each candidate",
    )
    parser.add_argument(
        "--judge",
        action="append",
        type=model_spec,
        help="blind judge as PROVIDER:MODEL; may be repeated",
    )
    parser.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    parser.add_argument("--profile", choices=["smoke", "full", "qualification", "custom"], default="custom")
    parser.add_argument("--program-profile", choices=["first-shot", "bounded-repair"],
                        default="first-shot", help="measure one proposal or one production-evidence repair")
    parser.add_argument("--task-count", type=int, help="use the first N fixed corpus tasks")
    parser.add_argument("--task-id", action="append", help="run only this task id")
    parser.add_argument("--trials", type=int, default=1)
    parser.add_argument("--worker-image", default=DEFAULT_WORKER_IMAGE)
    parser.add_argument("--skip-worker-build", action="store_true")
    parser.add_argument("--bootstrap-samples", type=int, default=10000)
    parser.add_argument("--seed", type=int, default=20260802)
    parser.add_argument("--candidate-reasoning", default="low")
    parser.add_argument("--judge-reasoning", default="low")
    parser.add_argument("--candidate-max-tokens", type=int, default=8192)
    parser.add_argument("--judge-max-tokens", type=int, default=2048)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--overwrite", action="store_true")
    parser.add_argument("--resume", action="store_true", help="resume an exact matching .partial event log")
    parser.add_argument("--audit-file", type=Path, help="completed blinded audit dispositions for qualification")
    parser.add_argument("--qualification-commitment", type=Path, default=QUALIFICATION_COMMITMENT,
                        help="sealed holdout commitment required by --profile qualification")
    parser.add_argument("--self-test", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.self_test:
        run_self_test()
        return 0
    if not args.candidate or not args.judge:
        raise ValueError("at least one --candidate and one --judge are required")
    if args.trials < 1 or args.timeout < 1 or args.bootstrap_samples < 1:
        raise ValueError("--trials and --timeout must be positive")
    if len(args.candidate) != len(set(args.candidate)):
        raise ValueError("candidate provider/model pairs must be unique")
    corpus = load_corpus(args.corpus)
    prompt_version, action_schema_version, instructions = load_prompt()
    if corpus["prompt_version"] != prompt_version:
        raise ValueError(
            f"corpus expects prompt version {corpus['prompt_version']}, source is {prompt_version}"
        )
    if corpus.get("action_schema_version") != action_schema_version:
        raise ValueError(
            "corpus action schema version does not match src/prompt.rs"
        )
    tools = proposal_tools()
    validate_execution_corpus(corpus)
    bundle_path = args.corpus.parent / corpus["reference_bundle"]
    bundle = json.loads(bundle_path.read_text(encoding="utf-8"))
    if set(bundle) != {"version", "action_schema_version", "program_contract", "tasks"} \
            or bundle["version"] != 4 or bundle["action_schema_version"] != action_schema_version \
            or bundle["program_contract"] != contract_description()["program_contract"]:
        raise ValueError("schema-v4 reference action bundle has invalid provenance")
    bundled = {item["id"]: item for item in bundle["tasks"]}
    if set(bundled) != {task["id"] for task in corpus["tasks"]}:
        raise ValueError("reference action bundle does not exactly cover corpus tasks")
    for task in corpus["tasks"]:
        if task["reference_actions"] != bundled[task["id"]]["reference_actions"] \
                or task["negative_actions"] != bundled[task["id"]]["negative_actions"]:
            raise ValueError(f"task {task['id']} differs from the locked reference bundle")
        for action in task["reference_actions"] + task["negative_actions"]:
            validate_action(action["tool"], action["arguments"], tools)
        for action in task["reference_actions"]:
            preflight = preflight_action(action["tool"], action["arguments"], task["fixture"]["stdin"] is not None)
            if not preflight["valid"] or any(item["severity"] == "availability" for item in preflight["diagnostics"]):
                raise ValueError(f"reference action failed production preflight: {task['id']} {preflight['diagnostics']}")
    tasks = corpus["tasks"]
    qualification_commitment = None
    if args.profile == "qualification":
        if args.program_profile != "first-shot":
            raise ValueError("qualification profile requires --program-profile first-shot")
        if len(args.candidate) < 2:
            raise ValueError("qualification profile requires at least two candidates")
        qualification_commitment = qualification_policy.validate_holdout(
            corpus, args.corpus, args.qualification_commitment, QUALIFICATION_POLICY
        )
        if args.task_id or args.task_count:
            raise ValueError("--profile qualification cannot be combined with task filters")
        args.trials = qualification_policy.load_policy(QUALIFICATION_POLICY)["trials_per_class"]
    elif args.profile == "full":
        if args.task_id or args.task_count:
            raise ValueError("--profile full cannot be combined with task filters")
        if len(tasks) != 120:
            raise ValueError("full profile requires the 120-task corpus")
        args.trials = 3
    elif args.profile == "smoke" and not args.task_id and args.task_count is None:
        tasks = select_stratified_tasks(tasks)
        args.trials = 1
    if args.task_id:
        wanted = set(args.task_id)
        tasks = [task for task in tasks if task["id"] in wanted]
        missing = wanted - {task["id"] for task in tasks}
        if missing:
            raise ValueError(f"unknown task ids: {sorted(missing)}")
    if args.task_count is not None:
        if args.task_count < 1 or args.task_count > len(tasks):
            raise ValueError(f"--task-count must be between 1 and {len(tasks)}")
        tasks = tasks[: args.task_count]
    for provider, _ in set(args.candidate + args.judge):
        api_key(provider)
    if not shutil_which("curl"):
        raise ValueError("curl is required")
    execution_enabled = "worker_contract_version" in corpus
    image_id = None
    tool_manifest_hash = None
    manifest = None
    if execution_enabled:
        if not shutil_which("docker"):
            raise ValueError("docker is required for the execution corpus")
        try:
            image_id = worker_image_id(args.worker_image)
            manifest, tool_manifest_hash = worker_manifest(args.worker_image)
            verify_worker_identity(manifest, args.corpus)
        except (RuntimeError, ValueError):
            if args.skip_worker_build:
                raise
            image_id = build_worker(args.worker_image, args.corpus)
            manifest, tool_manifest_hash = worker_manifest(args.worker_image)
            verify_worker_identity(manifest, args.corpus)
        available = {name for name, value in manifest.get("tools", {}).items() if value.get("path")}
        required = {name for task in tasks for action in task["reference_actions"] for name in action.get("arguments", {}).get("requirements", [])}
        missing_tools = sorted(required - available)
        if missing_tools:
            raise ValueError(f"worker image lacks required tools: {missing_tools}")

    corpus_hash = hashlib.sha256(args.corpus.read_bytes()).hexdigest()
    fingerprint, fingerprint_projection = run_fingerprint(args, corpus_hash, manifest)
    if args.output.exists() and not args.overwrite:
        raise ValueError(f"output already exists: {args.output}")
    checkpoint_path = Path(str(args.output) + ".partial")
    prior_events: list[dict[str, Any]] = []
    if args.resume:
        if not checkpoint_path.is_file():
            raise ValueError(f"resume checkpoint does not exist: {checkpoint_path}")
        prior_events = load_checkpoint(checkpoint_path, fingerprint)
        descriptor = os.open(checkpoint_path, os.O_WRONLY | os.O_APPEND)
        checkpoint = os.fdopen(descriptor, "a", encoding="utf-8")
    else:
        if checkpoint_path.exists() and not args.overwrite:
            raise ValueError(f"checkpoint already exists; use --resume or --overwrite: {checkpoint_path}")
        checkpoint = open_output(checkpoint_path, args.overwrite)
    sequence = len(prior_events)

    def save_checkpoint(event_type: str, payload: dict[str, Any]) -> None:
        nonlocal sequence
        event = {"event_version": 1, "type": event_type, "run_fingerprint": fingerprint, "sequence": sequence, "payload": payload}
        schema_validator("run-event.schema.json").validate(event)
        checkpoint.write(json.dumps(event, separators=(",", ":")) + "\n")
        checkpoint.flush()
        os.fsync(checkpoint.fileno())
        sequence += 1

    if not prior_events:
        save_checkpoint("run_started", {"started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                       "fingerprint_projection": fingerprint_projection, "corpus": str(args.corpus),
                       "task_count": len(tasks), "worker_manifest": manifest, "git": git_provenance(),
                       "host": {"kernel": platform.release(), "architecture": platform.machine()},
                       "docker_version": subprocess.run(["docker", "version", "--format", "{{.Server.Version}}"], text=True, capture_output=True, check=False).stdout.strip() if execution_enabled else None})

    jobs = [
        (trial, task, candidate)
        for trial in range(1, args.trials + 1)
        for task in tasks
        for candidate in args.candidate
    ]
    random.Random(args.seed).shuffle(jobs)
    latest = {}
    judged_keys = set()
    event_keys = set()
    for event in prior_events:
        if event["type"] in {"candidate_completed", "judgment_completed"} and "record" in event["payload"]:
            record = event["payload"]["record"]
            key = (record["task_id"], record["trial"], record["candidate"]["provider"], record["candidate"]["model"])
            typed_key = (event["type"], key)
            if typed_key in event_keys: raise ValueError(f"duplicate resume event key: {typed_key}")
            event_keys.add(typed_key)
            if event["type"] == "judgment_completed" and key in latest:
                base = {name: value for name, value in latest[key].items() if name not in {"judgments", "synthetic_outcome"}}
                judged_base = {name: value for name, value in record.items() if name not in {"judgments", "synthetic_outcome"}}
                if base != judged_base: raise ValueError(f"conflicting resume event key: {key}")
            latest[key] = record
            if event["type"] == "judgment_completed": judged_keys.add(key)
    records: list[dict[str, Any]] = list(latest.values())
    completed_candidate_keys = set(latest)
    total = len(jobs)
    for index, (trial, task, (provider, model)) in enumerate(jobs, 1):
        key = (task["id"], trial, provider, model)
        if key in completed_candidate_keys:
            continue
        print(
            f"candidate {index}/{total}: {provider}:{model} {task['id']} trial={trial}",
            file=sys.stderr,
        )
        record: dict[str, Any] = {
            "type": "result",
            "task_id": task["id"],
            "family_id": task["family_id"],
            "variant_id": task["variant_id"],
            "stratum": "semantic" if task["route_oracle"]["preferred"] in {"return_answer", "request_clarification"} else "executable",
            "trial": trial,
            "candidate": {"provider": provider, "model": model},
            "transport_success": False,
            "wire_valid": False,
            "client_valid": False,
            "preflight_valid": False,
            "program_diagnostics": [],
            "program_warning_count": 0,
            "runtime_available": True,
            "route_allowed": False,
            "route_preferred": False,
            "execution_attempted": False,
            "execution_started": False,
            "artifact_commit_success": None,
            "oracle_pass": None,
            "completed_outcome": False,
            "first_shot_completed_outcome": False,
            "repair_eligible": False,
            "repair_attempted": False,
            "repair_action": None,
            "repair_diagnostics": [],
            "repair_execution": None,
            "repair_completed_outcome": False,
            "cumulative_if_approved": False,
            "model_call_count": 1,
            "repair_added_latency_ms": 0,
            "repair_candidate_tokens": {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0},
            "execution": None,
            "action": None,
            "timing": None,
            "error": None,
            "judgments": [],
        }
        try:
            name, arguments, timing = call_candidate(
                provider,
                model,
                proposal_input(task, manifest),
                args.candidate_max_tokens,
                args.candidate_reasoning,
                args.timeout,
            )
            record["transport_success"] = True
            record["timing"] = timing
            record["action"] = {"tool": name, "arguments": arguments}
            matching = [tool for tool in tools if tool["name"] == name]
            if len(matching) != 1:
                raise ValueError(f"unknown proposal tool {name!r}")
            Draft202012Validator(matching[0]["parameters"]).validate(arguments)
            record["wire_valid"] = True
            validate_action(name, arguments, tools)
            record["client_valid"] = True
            preflight = preflight_action(
                name, arguments, task.get("fixture", {}).get("stdin") is not None
            )
            record["program_diagnostics"] = preflight["diagnostics"]
            record["program_warning_count"] = sum(
                item["severity"] == "warning" for item in preflight["diagnostics"]
            )
            record["runtime_available"] = not any(
                item["severity"] == "availability" for item in preflight["diagnostics"]
            )
            record["preflight_valid"] = bool(preflight["valid"])
            record["route_allowed"] = name in task["route_oracle"]["allowed"]
            record["route_preferred"] = name == task["route_oracle"]["preferred"]
            if record["route_allowed"] and record["preflight_valid"] and record["runtime_available"]:
                if execution_enabled and name in {"run_shell", "run_program", "require_parent_shell"}:
                    record["execution_attempted"] = True
                    record["execution"] = execute_in_worker(args.worker_image, task, record["action"])
                    record["execution_started"] = bool(record["execution"].get("started"))
                    record["artifact_commit_success"] = record["execution"].get("artifact_commit_success")
                    record["oracle_pass"] = bool(record["execution"]["oracle"]["passed"])
                elif record["stratum"] == "semantic":
                    record["execution"] = deterministic_nonexecuting(task, record["action"])
            record["completed_outcome"] = bool(record["stratum"] == "executable" and record["client_valid"] and record["preflight_valid"] and record["route_allowed"] and record["oracle_pass"])
            record["first_shot_completed_outcome"] = record["completed_outcome"]
            follow_up = repair_follow_up(record["action"], record["program_diagnostics"], record["execution"])
            record["repair_eligible"] = follow_up is not None
            if args.program_profile == "bounded-repair" and record["repair_eligible"]:
                repair_name, repair_arguments, repair_timing = call_candidate(
                    provider, model, proposal_input(task, manifest, follow_up),
                    args.candidate_max_tokens, args.candidate_reasoning, args.timeout,
                )
                record["model_call_count"] = 2
                record["repair_attempted"] = True
                record["repair_added_latency_ms"] = repair_timing["wall_ms"]
                record["repair_candidate_tokens"] = repair_timing.get("usage") or record["repair_candidate_tokens"]
                record["repair_action"] = {"tool": repair_name, "arguments": repair_arguments}
                matching_repair = [tool for tool in tools if tool["name"] == repair_name]
                if len(matching_repair) != 1:
                    raise ValueError(f"unknown repair proposal tool {repair_name!r}")
                Draft202012Validator(matching_repair[0]["parameters"]).validate(repair_arguments)
                validate_action(repair_name, repair_arguments, tools)
                repair_preflight = preflight_action(
                    repair_name, repair_arguments, task.get("fixture", {}).get("stdin") is not None
                )
                record["repair_diagnostics"] = repair_preflight["diagnostics"]
                if repair_preflight["valid"] and not any(item["severity"] == "availability" for item in repair_preflight["diagnostics"]):
                    repair_allowed = repair_name in task["route_oracle"]["allowed"]
                    if repair_allowed and repair_name in {"run_shell", "run_program", "require_parent_shell"}:
                        record["repair_execution"] = execute_in_worker(args.worker_image, task, record["repair_action"])
                        record["repair_completed_outcome"] = bool(record["repair_execution"]["oracle"]["passed"])
            record["cumulative_if_approved"] = bool(record["completed_outcome"] or record["repair_completed_outcome"])
        except CandidateCallError as error:
            record["transport_success"] = error.attempts_consumed == 1 and error.kind not in {"transient", "timeout"}
            record["timing"] = {"wall_ms": error.wall_ms, "provider_error_kind": error.kind}
            record["error"] = f"provider adapter: {error.kind}"
        except Exception as error:  # each failed attempt remains a benchmark record
            record["error"] = str(error)[:1000]
        records.append(record)
        save_checkpoint("candidate_completed", {"record": record})

    judge_jobs = [(record, task) for record in records for task in tasks if record["task_id"] == task["id"]]
    random.Random(args.seed ^ 0x5A17).shuffle(judge_jobs)
    safety_families = {task["family_id"] for task in tasks if any(effect in {"delete_local", "remote_mutation", "privilege_elevation"} for effect in task["tags"]["effects"])}
    failures = [(record, task) for record, task in judge_jobs if not record["completed_outcome"] and record["client_valid"]]
    diagnostic = set()
    for category in sorted({task["tags"]["category"] for _, task in failures}):
        bucket = [(record, task) for record, task in failures if task["tags"]["category"] == category]
        random.Random(args.seed ^ int(hashlib.sha256(category.encode()).hexdigest()[:8], 16)).shuffle(bucket)
        diagnostic.update((record["task_id"], record["trial"], record["candidate"]["provider"], record["candidate"]["model"]) for record, _ in bucket[:2])
    eligible = lambda record: record["client_valid"] and (record["stratum"] == "semantic" or record["completed_outcome"] or record["family_id"] in safety_families or (record["task_id"], record["trial"], record["candidate"]["provider"], record["candidate"]["model"]) in diagnostic)
    total_judgments = sum(eligible(record) for record, _ in judge_jobs) * len(args.judge)
    judge_index = 0
    judge_schema = [judgment_tool()]
    for record, task in judge_jobs:
        key = (record["task_id"], record["trial"], record["candidate"]["provider"], record["candidate"]["model"])
        if key in judged_keys:
            continue
        if not eligible(record):
            record["synthetic_outcome"] = "invalid_or_unsampled_deterministic_failure"
            save_checkpoint("judgment_completed", {"record": record})
            continue
        anonymous_input = judge_input(task, record["action"], record.get("execution"))
        for provider, model in args.judge:
            judge_index += 1
            print(
                f"judge {judge_index}/{total_judgments}: {task['id']}",
                file=sys.stderr,
            )
            record["judgments"].append(call_judge_with_retry(provider, model, anonymous_input, judge_schema, args))
        if record["stratum"] == "semantic":
            valid_judgments = [value for value in record["judgments"] if value.get("valid") and not value.get("synthetic")]
            record["semantic_acceptable"] = bool(valid_judgments and all(value.get("verdict") in {"pass", "minor"} and not value.get("critical_error") for value in valid_judgments))
        save_checkpoint("judgment_completed", {"record": record})

    calibration = [event["payload"] for event in prior_events if event["type"] == "calibration_completed"]
    calibration_keys = {(item["task_id"], item["candidate"]["provider"], item["candidate"]["model"], item["judge"]["provider"], item["judge"]["model"]) for item in calibration}
    if args.profile in {"full", "qualification"}:
        calibration_ids = {task["id"] for task in select_stratified_tasks(corpus["tasks"])}
        task_by_id = {task["id"]: task for task in tasks}
        for record in records:
            if record["trial"] != 1 or record["task_id"] not in calibration_ids or not record["client_valid"] or not record["route_allowed"]:
                continue
            task = task_by_id[record["task_id"]]
            anonymous_input = judge_input(task, record["action"], record.get("execution"))
            for provider, model in args.judge:
                calibration_key = (record["task_id"], record["candidate"]["provider"], record["candidate"]["model"], provider, model)
                if calibration_key in calibration_keys: continue
                repeated = call_judge_with_retry(provider, model, anonymous_input, judge_schema, args)
                primary = next((value for value in record["judgments"] if value.get("judge") == {"provider": provider, "model": model}), None)
                item = {"task_id": record["task_id"], "candidate": record["candidate"],
                        "judge": {"provider": provider, "model": model},
                        "primary_verdict": primary.get("verdict") if primary and primary.get("valid") else None,
                        "repeat_verdict": repeated.get("verdict") if repeated.get("valid") else None,
                        "agreement": bool(primary and primary.get("valid") and repeated.get("valid") and primary["verdict"] == repeated["verdict"]),
                        "repeat": repeated}
                calibration.append(item)
                save_checkpoint("calibration_completed", item)

    def audit_key(record):
        return (record["task_id"], record["trial"], record["candidate"]["provider"], record["candidate"]["model"])
    disagreements = [record for record in records if any(
        judgment.get("valid") and not judgment.get("synthetic") and
        ((record["stratum"] == "executable" and record["completed_outcome"] and (judgment.get("verdict") == "fail" or judgment.get("critical_error"))) or
         (record["stratum"] == "executable" and not record["completed_outcome"] and judgment.get("verdict") in {"pass", "minor"}) or
         (record["stratum"] == "semantic" and record.get("semantic_acceptable") != (judgment.get("verdict") in {"pass", "minor"} and not judgment.get("critical_error"))))
        for judgment in record.get("judgments", []))]
    ordered_pool = sorted(records, key=lambda record: hashlib.sha256(f"{args.seed}:{audit_key(record)}".encode()).hexdigest())
    audit_pool = []
    seen_audit = set()
    for record in disagreements + ordered_pool:
        if audit_key(record) not in seen_audit and len(audit_pool) < 20:
            audit_pool.append(record); seen_audit.add(audit_key(record))
    audit_complete = False
    audit_metadata = None
    if args.audit_file:
        if args.profile != "qualification":
            raise ValueError("--audit-file is accepted only by --profile qualification")
        audit_metadata = json.loads(args.audit_file.read_text(encoding="utf-8"))
        schema_validator("audit-dispositions.schema.json").validate(audit_metadata)
        supplied = set()
        for item in audit_metadata["items"]:
            if item["audit_id"] in supplied:
                raise ValueError("audit file contains a duplicate audit_id")
            supplied.add(item["audit_id"])
        expected_audit_ids = {hashlib.sha256(f"{fingerprint}:{audit_key(record)}".encode()).hexdigest()[:16] for record in audit_pool}
        audit_complete = supplied == expected_audit_ids
        if not audit_complete: raise ValueError("audit dispositions do not exactly cover the blinded audit sample")
    elif args.profile == "qualification":
        task_by_id = {task["id"]: task for task in tasks}
        audit_request = {"version": 1, "rubric_version": 1, "items": []}
        for record in audit_pool:
            task = task_by_id[record["task_id"]]
            audit_request["items"].append({
                "audit_id": hashlib.sha256(f"{fingerprint}:{audit_key(record)}".encode()).hexdigest()[:16],
                "task": {"request": task["prompt"], "rubric": task["rubric"], "route_oracle": task["route_oracle"]},
                "anonymous_proposal": record["action"], "execution": record["execution"],
                "completed_outcome": record["completed_outcome"], "judgments": [{key: value for key, value in judgment.items() if key not in {"judge", "timing"}} for judgment in record["judgments"]],
            })
        audit_path = Path(str(args.output) + ".audit-request.json")
        atomic_write_json(audit_path, audit_request)
        checkpoint.close()
        print(f"full run awaits independent blinded audit: {audit_path}", file=sys.stderr)
        print("resume with --resume --audit-file FILE after recording every disposition", file=sys.stderr)
        return 3
    summaries = aggregate(records, args.candidate)
    comparisons = pairwise_statistics(records, args.candidate, args.bootstrap_samples, args.seed ^ 0xB007)
    selection = {"winner": None, "basis": "development profiles never produce qualification decisions"}
    calibration_summary = []
    if args.profile in {"full", "qualification"}:
        for candidate in args.candidate:
            for judge in args.judge:
                selected = [item for item in calibration
                            if item["candidate"] == {"provider": candidate[0], "model": candidate[1]}
                            and item["judge"] == {"provider": judge[0], "model": judge[1]}]
                agreements = sum(item["agreement"] for item in selected)
                calibration_summary.append({"candidate": {"provider": candidate[0], "model": candidate[1]},
                                            "judge": {"provider": judge[0], "model": judge[1]},
                                            "agreements": agreements, "comparisons": len(selected),
                                            "stable": len(selected) == 12 and agreements >= 10})
    qualification = None
    if args.profile == "qualification":
        policy = qualification_policy.load_policy(QUALIFICATION_POLICY)
        qualification = qualification_policy.evaluate(
            records, tasks, args.candidate, calibration, audit_metadata or {}, policy, args.seed
        )
        selection = {"request_classes": qualification["selections"]}
        for model_summary in summaries:
            model_summary["qualification_profiles"] = [
                profile for profile in qualification["profiles"]
                if profile["candidate"] == {"provider": model_summary["provider"], "model": model_summary["model"]}
            ]
            model_summary["qualified"] = any(profile["qualified"] for profile in model_summary["qualification_profiles"])
    else:
        for model_summary in summaries:
            model_summary["qualified"] = False
    audit_status = ("complete" if audit_complete else "pending_manual_review") \
        if args.profile == "qualification" else "not_required_development"
    summary = {"models": summaries, "comparisons": comparisons, "selection": selection,
               "judge_calibration": calibration_summary, "calibration_records": calibration,
               "independent_audit": {"status": audit_status, "metadata": audit_metadata, "items": [{"task_id": item["task_id"], "trial": item["trial"], "candidate": item["candidate"]} for item in audit_pool]},
               "task_count": len(tasks), "family_count": len({task["family_id"] for task in tasks}),
               "product_usage_weighted_completion": None, "qualification": qualification,
               "qualification_commitment": qualification_commitment}
    save_checkpoint("summary_computed", summary)
    save_checkpoint("run_completed", {"completed_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "record_count": len(records)})
    checkpoint.close()
    os.chmod(checkpoint_path, 0o600)
    os.replace(checkpoint_path, args.output)
    parent_descriptor = os.open(args.output.parent, os.O_RDONLY)
    try: os.fsync(parent_descriptor)
    finally: os.close(parent_descriptor)
    artifact_sha256 = hashlib.sha256(args.output.read_bytes()).hexdigest()
    redacted = {"artifact_sha256": artifact_sha256, "run_fingerprint": fingerprint,
                "task_count": len(tasks), "family_count": summary["family_count"],
                "models": summaries, "comparisons": comparisons, "selection": selection,
                "limitations": (["Independent blinded audit is pending.", "No product-usage weights were supplied."]
                                if args.profile == "qualification" and not audit_complete
                                else ["No product-usage weights were supplied."])}
    atomic_write_json(Path(str(args.output) + ".summary.json"), redacted)
    report = subprocess.run([sys.executable, str(ROOT / "scripts/provider-benchmark-report.py"), str(args.output)], check=False)
    if report.returncode:
        raise RuntimeError("final artifact is valid but report generation failed")
    print_summary(summaries)
    print(f"raw results: {args.output}")
    return 0


def shutil_which(command: str) -> str | None:
    for directory in os.environ.get("PATH", "").split(os.pathsep):
        candidate = Path(directory) / command
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    return None


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, RuntimeError) as error:
        print(f"provider-bakeoff: {error}", file=sys.stderr)
        raise SystemExit(2)
