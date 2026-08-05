#!/usr/bin/env bash
# Plan 18 §5 measurement harness.
#
# Runs live `uhm --dry-run --fresh` proposals for a fixed intent set and
# classifies each, so the plan's completion gate can be checked against data
# rather than inspection. It spends real provider calls and sends the intent
# text to the configured provider; run it only when that is intended.
#
#   scripts/measure-plan-18.sh [SAMPLES]
#
# SAMPLES defaults to 12 (the plan asks for 10+ per condition). Output is a
# Markdown table on stdout and a full per-sample JSON dump at
# scripts/measure-plan-18-results.json. The tool-surface store is reset to a
# top-level-only baseline before every sample so each sample faces the same
# starting surface and the probe's contribution is isolated from amortization.
set -euo pipefail

SAMPLES="${1:-12}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if ! command -v python3 >/dev/null; then
  echo "python3 is required" >&2
  exit 2
fi

cd "$ROOT"
cargo build --release --quiet
BIN="$ROOT/target/release/uhm"

python3 - "$BIN" "$SAMPLES" "$ROOT" <<'PY'
import json, os, pty, select, sys, time

BIN, SAMPLES, ROOT = sys.argv[1], int(sys.argv[2]), sys.argv[3]
SAMPLES = max(SAMPLES, 1)

# Isolated HOME/XDG so the persisted tool-surface store is owned by this run.
WORK = os.environ.get("UHM_MEASURE_WORK") or "/tmp/uhm-plan18-measure"
os.makedirs(WORK, exist_ok=True)
os.makedirs(f"{WORK}/config/uhm", exist_ok=True)
with open(f"{WORK}/config/uhm/config.yaml", "w") as fh:
    # Keep telemetry local so live measurement never reaches the Worker.
    fh.write("history:\n  enabled: true\ntelemetry:\n  enabled: false\n")

ENV = os.environ.copy()
ENV.update(
    HOME=WORK,
    XDG_CONFIG_HOME=f"{WORK}/config",
    XDG_DATA_HOME=f"{WORK}/data",
    XDG_CACHE_HOME=f"{WORK}/cache",
    TERM="dumb",
    # steel lives outside the isolated HOME; keep it on PATH so it resolves.
    PATH=os.environ.get("PATH", ""),
)
STORE = f"{WORK}/data/uhm/tool-surface.json"

# The fixed intent set. The first two name steel — the one uncataloged tool
# installed here whose top-level help advertises a subcommand group but omits
# the verb the target needs (browser hides start/navigate; sessions hides list).
# The third names no tool and is the regression fixture.
INTENTS = [
    (
        "steel-browser",
        "open a steel browser session and navigate to hacker news",
        "browser",  # advertised at top level; navigate/start are not
    ),
    (
        "steel-sessions",
        "show my active steel sessions",
        "sessions",  # advertised at top level; list is not
    ),
    (
        "no-tool",
        "count the number of lines in /etc/hostname",
        None,
    ),
]


def run_under_pty(args, consent, timeout):
    """Run BIN under a pty. When consent is true, answer every [y/N] prompt
    with y (so all named binaries are allowed and their help is retained).
    Returns (stdout+stderr bytes, exit code)."""
    pid, fd = pty.fork()
    if pid == 0:  # child
        try:
            os.execvpe(BIN, args, ENV)
        except Exception:
            os._exit(127)
    buf = bytearray()
    answered = 0
    deadline = time.time() + timeout
    status = 1
    while True:
        remaining = deadline - time.time()
        if remaining <= 0:
            try:
                os.kill(pid, 9)
            except ProcessLookupError:
                pass
            break
        r, _, _ = select.select([fd], [], [], min(1.0, remaining))
        if fd in r:
            try:
                chunk = os.read(fd, 8192)
            except OSError:
                break
            if not chunk:
                break
            buf.extend(chunk)
            # Answer every consent prompt that has appeared so far.
            if consent:
                while answered < buf.count(b"[y/N]"):
                    try:
                        os.write(fd, b"y\n")
                    except OSError:
                        break
                    answered += 1
        try:
            wpid, wstatus = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            break
        if wpid == pid:
            status = os.waitstatus_to_exitcode(wstatus)
            # Drain anything remaining.
            while True:
                r, _, _ = select.select([fd], [], [], 0.3)
                if fd not in r:
                    break
                try:
                    chunk = os.read(fd, 8192)
                except OSError:
                    break
                if not chunk:
                    break
                buf.extend(chunk)
            break
    try:
        os.close(fd)
    except OSError:
        pass
    return bytes(buf), status


def seed_consent():
    """Run the browser intent once under a pty answering every consent prompt,
    so the store gains an allowed steel record with top-level help. Returns the
    reset-template store object (subcommands cleared, unrelated binaries that
    happened to resolve removed, so each sample starts from steel-only depth)."""
    # Consent is gated on `tty_available() && !args.json`, so the seed run must
    # NOT pass --json (that would auto-decline every named tool). --plain keeps
    # the captured stream free of escape codes. Sampling runs below use --json;
    # by then steel is already allowed in the store and needs no prompt.
    run_under_pty(
        [BIN, "--dry-run", "--plain", INTENTS[0][1]],
        consent=True,
        timeout=150,
    )
    with open(STORE) as fh:
        store = json.load(fh)
    keep = {}
    for key, record in store.get("tools", {}).items():
        # Only an allowed record whose retained help names steel is a useful
        # baseline; a declined or help-less bystander binary would only noise
        # the surface the model sees.
        help_text = record.get("help") or ""
        if record.get("allowed") and "steel" in help_text.lower():
            record["subcommands"] = []  # baseline: top-level help only
            keep[key] = record
    store["tools"] = keep
    return store


def reset_store(template):
    os.makedirs(os.path.dirname(STORE), exist_ok=True)
    with open(STORE, "w") as fh:
        json.dump(template, fh)


def one_sample(intent):
    """Non-interactive single proposal via plain subprocess (no pty). Steel is
    already consented in the store, so no prompt can block; a clarification
    takes the non-tty path and returns immediately. Probe firing is detected
    from the store (the probe persists the subcommand help it read), since
    --json suppresses the stderr narration line."""
    import subprocess
    proc = subprocess.run(
        [BIN, "--dry-run", "--fresh", "--json", intent],
        env=ENV,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=150,
    )
    text = proc.stdout.decode("utf-8", "replace")
    err = proc.stderr.decode("utf-8", "replace")
    command = None
    message = None
    outcome = None
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("{") and '"namespace"' in line:
            try:
                obj = json.loads(line)
                outcome = obj.get("outcome")
                command = obj.get("command")
                message = obj.get("message")
            except json.JSONDecodeError:
                pass
    # The probe persists whatever subcommand help it read; the baseline cleared
    # subcommands, so any entry now present means a probe ran this sample.
    probe_target = None
    try:
        with open(STORE) as fh:
            store = json.load(fh)
        for record in store.get("tools", {}).values():
            if record.get("allowed") and (record.get("help") or "").lower().count("steel"):
                subs = [s["subcommand"] for s in record.get("subcommands", [])]
                if subs:
                    probe_target = subs[-1]
                    break
    except (OSError, ValueError):
        pass
    return {
        "exit": proc.returncode,
        "outcome": outcome,
        "command": command,
        "message": message,
        "probe_fired": probe_target is not None,
        "probe_target": probe_target,
        "raw_stdout": text,
        "raw_stderr": err,
    }


def classify(cond, key, sample):
    cmd = (sample.get("command") or "").lower()
    outcome = sample.get("outcome")
    if outcome == "clarification_required":
        return "clarification"
    if outcome != "dry_run" and outcome is not None:
        return f"outcome:{outcome}"
    if outcome is None and cmd == "":
        # No structured result: most likely a provider/transport error.
        return "provider_error"
    if cond == "no-tool":
        if sample["probe_fired"]:
            return "spurious_probe"
        if "wc" in cmd or "hostname" in cmd:
            return "correct_no_tool"
        return "other"
    # steel conditions: the target verb lives under the subcommand group.
    if key == "browser":
        reached = "navigate" in cmd
        used_depth = "steel browser" in cmd
        chained_opener = any(tok in cmd for tok in ("xdg-open", "open ", "&& open"))
        if reached and used_depth and not chained_opener:
            return "correct_complete"
        if "steel" in cmd and chained_opener:
            return "chained_unrelated"
        if "steel" in cmd and not reached:
            return "dropped_target"
        return "other"
    if key == "sessions":
        # Ground truth (confirmed against the installed binary): both
        # `steel sessions list` (the cloud group) and `steel browser sessions`
        # (`steel browser --help` advertises it as "List active browser
        # sessions") are real session-listing commands. Either is a correct
        # complete composition for "active sessions".
        if ("sessions" in cmd and "list" in cmd) or "browser sessions" in cmd:
            return "correct_complete"
        if "sessions" in cmd:
            return "dropped_target"
        return "other"
    return "other"


template = seed_consent()

results = {}
for cond, intent, key in INTENTS:
    samples = []
    for _ in range(SAMPLES):
        reset_store(template)
        sample = one_sample(intent)
        sample["classification"] = classify(cond, key, sample)
        samples.append(sample)
        sys.stderr.write(f"  {cond}: {sample['classification']} (probe={sample['probe_fired']})\n")
        sys.stderr.flush()
    tally = {}
    for s in samples:
        tally[s["classification"]] = tally.get(s["classification"], 0) + 1
    probe_count = sum(1 for s in samples if s["probe_fired"])
    results[cond] = {
        "intent": intent,
        "subcommand": key,
        "samples": SAMPLES,
        "probe_fired": probe_count,
        "tally": tally,
        "raw": samples,
    }

with open(f"{ROOT}/scripts/measure-plan-18-results.json", "w") as fh:
    json.dump(results, fh, indent=2)

# Markdown summary table on stdout.
print("| Condition | Intent | Samples | Probe fired | Tally |")
print("| --- | --- | ---: | ---: | --- |")
for cond, intent, _key in INTENTS:
    r = results[cond]
    tally = ", ".join(f"{k}={v}" for k, v in sorted(r["tally"].items()))
    print(
        f"| {cond} | `{intent}` | {r['samples']} | {r['probe_fired']} | {tally} |"
    )
PY
