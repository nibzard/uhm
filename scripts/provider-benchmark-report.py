#!/usr/bin/env python3
"""Generate redacted JSON and HTML from a finalized benchmark event artifact."""

import argparse
import hashlib
import html
import json
import os
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parent.parent


def write_atomic(path: Path, text: str) -> None:
    temporary = Path(str(path) + ".tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        output.write(text); output.flush(); os.fsync(output.fileno())
    os.replace(temporary, path)
    directory = os.open(path.parent, os.O_RDONLY)
    try: os.fsync(directory)
    finally: os.close(directory)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact", type=Path)
    parser.add_argument("--prefix", type=Path)
    args = parser.parse_args()
    schema = json.loads((ROOT / "benchmark/schemas/run-event.schema.json").read_text())
    validator = Draft202012Validator(schema)
    events = []
    for line_number, line in enumerate(args.artifact.read_text(encoding="utf-8").splitlines(), 1):
        try: event = json.loads(line)
        except json.JSONDecodeError as error: raise SystemExit(f"truncated event at line {line_number}: {error}")
        validator.validate(event); events.append(event)
    if not events or events[-1]["type"] != "run_completed":
        raise SystemExit("artifact is not finalized")
    fingerprints = {event["run_fingerprint"] for event in events}
    if len(fingerprints) != 1 or [event["sequence"] for event in events] != list(range(len(events))):
        raise SystemExit("event fingerprint or sequence is inconsistent")
    summaries = [event["payload"] for event in events if event["type"] == "summary_computed"]
    if len(summaries) != 1: raise SystemExit("artifact must contain one summary")
    summary = summaries[0]
    redacted = {
        "artifact_sha256": hashlib.sha256(args.artifact.read_bytes()).hexdigest(),
        "run_fingerprint": next(iter(fingerprints)), "task_count": summary["task_count"],
        "family_count": summary["family_count"], "models": summary["models"],
        "comparisons": summary["comparisons"], "selection": summary["selection"],
        "independent_audit": summary["independent_audit"]["status"],
        "qualification": summary.get("qualification"),
        "qualification_commitment_status": (summary.get("qualification_commitment") or {}).get("status"),
        "limitations": ["Raw proposals and fixture evidence remain private.", "Product usage weighting is unavailable unless a versioned weight file is supplied."],
    }
    prefix = args.prefix or args.artifact
    json_path = Path(str(prefix) + ".summary.json")
    html_path = Path(str(prefix) + ".html")
    rows = "".join(
        "<tr>" + "".join(f"<td>{html.escape(str(model.get(key)))}</td>" for key in ("provider", "model", "attempts", "client_valid", "route_allowed", "completed_outcomes", "family_macro_completion")) + "</tr>"
        for model in redacted["models"]
    )
    page = f"""<!doctype html><meta charset=utf-8><title>UHM benchmark report</title>
<h1>UHM benchmark report</h1><p>Artifact SHA-256: <code>{redacted['artifact_sha256']}</code></p>
<p>{redacted['task_count']} tasks across {redacted['family_count']} semantic families. Audit: {html.escape(redacted['independent_audit'])}.</p>
<table><thead><tr><th>Provider</th><th>Model</th><th>Attempts</th><th>Client valid</th><th>Allowed route</th><th>Completed</th><th>Family macro %</th></tr></thead><tbody>{rows}</tbody></table>
<h2>Selection</h2><pre>{html.escape(json.dumps(redacted['selection'], indent=2))}</pre>
<h2>Frozen qualification</h2><pre>{html.escape(json.dumps(redacted['qualification'], indent=2))}</pre>
<h2>Limitations</h2><ul>{''.join(f'<li>{html.escape(item)}</li>' for item in redacted['limitations'])}</ul>
"""
    write_atomic(json_path, json.dumps(redacted, indent=2) + "\n")
    write_atomic(html_path, page)
    print(json_path); print(html_path)
    return 0


if __name__ == "__main__": raise SystemExit(main())
