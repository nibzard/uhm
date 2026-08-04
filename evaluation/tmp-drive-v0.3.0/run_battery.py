#!/usr/bin/env python3
"""Run the uhm evaluation battery in the isolated tmp-drive sandbox.

For each task in battery.json: invoke uhm (--plain --json) in the corpus workdir
under an isolated HOME/XDG env, capture exit code / stdout / stderr / latency,
parse the --json envelope off stderr, evaluate the per-task check, and append a
record to results.jsonl. Two bonus pty tasks are driven via `script(1)`.
"""
import os, sys, json, time, hashlib, subprocess, shlex

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.environ.get("UHM_EVAL_ROOT", "/dev/shm/uhm-eval")
WORK = f"{ROOT}/work"
OUT = f"{ROOT}/out"
UHM = os.environ.get("UHM_EVAL_BINARY", "/home/agent/.local/bin/uhm")
PROVIDER = os.environ.get("UHM_EVAL_PROVIDER", "openai")
MODEL = os.environ.get("UHM_EVAL_MODEL", "gpt-5.6-terra")
TIMEOUT = 150
PTY_TIMEOUT = 90
os.makedirs(OUT, exist_ok=True)

expected = json.load(open(f"{ROOT}/expected.json"))
battery = json.load(open(os.environ.get("UHM_EVAL_BATTERY", f"{HERE}/battery.json")))

# --- isolated env ---
ENV = os.environ.copy()
ENV.update(
    HOME=f"{ROOT}/home",
    XDG_CONFIG_HOME=f"{ROOT}/config",
    XDG_DATA_HOME=f"{ROOT}/data",
    XDG_CACHE_HOME=f"{ROOT}/cache",
    UHM_TELEMETRY="off",
    TERM="dumb",
    NO_COLOR="1",
)
assert ENV.get("OPENAI_API_KEY"), "OPENAI_API_KEY must be present in the environment"

# --- baseline hashes for files_unchanged ---
def sha(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()

BASELINE = {}
for rel in ("data.json", "sales.csv", "NOTES.md", "README.md", "events.log", "src/parser.rs", "big.bin"):
    p = os.path.join(WORK, rel)
    if os.path.exists(p):
        BASELINE[rel] = sha(p)


def build_args(t):
    mode = t["mode"]
    if mode == "subcommand":
        return list(t["cmd"])
    args = []
    if mode in ("run", "ask", "explain"):
        args.append(mode)
    args += ["--plain", "--json", "--fresh", "--provider", PROVIDER, "--model", MODEL]
    if t.get("extra"):
        args += t["extra"]
    if t.get("dash_sep"):
        args.append("--")
    args.append(t["intent"])
    return args


def parse_envelope(stderr):
    env = {}
    for line in reversed(stderr.splitlines()):
        s = line.strip()
        if s.startswith("{"):
            try:
                obj = json.loads(s)
            except Exception:
                continue
            if obj.get("namespace") in ("uhm", "uhm.child"):
                return obj
    return env


# ---------- check evaluator ----------
def ev(check, ctx):
    k = check["kind"]
    rc, so, se = ctx["rc"], ctx["stdout"], ctx["stderr"]
    rc_ok = (rc == 0)

    if k == "rc_zero":
        return (rc == 0, f"rc={rc}")
    if k == "rc_eq":
        v = check["value"]
        return (rc == v, f"rc={rc} want={v}")
    if k == "rc_in":
        return (rc in check["values"], f"rc={rc} want one of {check['values']}")
    if k == "rc_not_in":
        return (rc not in check["values"], f"rc={rc} want not in {check['values']}")
    if k == "stdout_nonempty":
        return (rc_ok and bool(so.strip()), f"rc={rc} stdout_len={len(so.strip())}")
    if k == "num_in_stdout":
        val = expected[check["ref"]] if "ref" in check else check["value"]
        sval = str(val)
        return (rc_ok and sval in so, f"rc={rc} looking for '{sval}' in stdout")
    if k == "contains":
        n = check["needle"]
        return (rc_ok and n in so, f"rc={rc} needle='{n}' present={n in so}")
    if k == "file_exists":
        p = os.path.join(WORK, check["path"])
        ex = os.path.exists(p)
        return (rc_ok and ex, f"rc={rc} exists({check['path']})={ex}")
    if k == "file_absent":
        p = os.path.join(WORK, check["path"])
        ex = os.path.exists(p)
        return ((not ex), f"exists({check['path']})={ex}")
    if k == "file_contains":
        p = os.path.join(WORK, check["path"])
        try:
            txt = open(p).read()
        except Exception as e:
            return (False, f"read({check['path']}) failed: {e}")
        n = check["needle"]
        return (rc_ok and n in txt, f"rc={rc} '{n}' in {check['path']}={n in txt}")
    if k == "file_contains_all":
        p = os.path.join(WORK, check["path"])
        try:
            txt = open(p).read()
        except Exception as e:
            return (False, f"read({check['path']}) failed: {e}")
        missing = [n for n in check["needles"] if n not in txt]
        return (rc_ok and not missing, f"rc={rc} missing_in_{check['path']}={missing}")
    if k == "file_json_valid":
        p = os.path.join(WORK, check["path"])
        try:
            json.load(open(p))
            ok = True
            detail = f"{check['path']} parses as JSON"
        except Exception as e:
            ok, detail = False, f"{check['path']} not valid JSON: {e}"
        return (rc_ok and ok, f"rc={rc} {detail}")
    if k == "dir_exists":
        p = os.path.join(WORK, check["path"])
        ex = os.path.isdir(p)
        return (rc_ok and ex, f"rc={rc} dir({check['path']})={ex}")
    if k == "files_unchanged":
        bad = []
        for rel in check["paths"]:
            p = os.path.join(WORK, rel)
            if rel not in BASELINE:
                bad.append(f"{rel}(no-baseline)")
            elif not os.path.exists(p) or sha(p) != BASELINE[rel]:
                bad.append(rel)
        return (not bad, f"changed={bad}")
    if k == "graceful_fail":
        crashed = rc in (134, 139) or (rc is not None and rc < 0)
        nonzero = rc not in (0, None)
        has_msg = bool((so + se).strip())
        return (nonzero and not crashed and has_msg, f"rc={rc} crashed={crashed} has_msg={has_msg}")
    if k == "compound":
        details = []
        for sub in check["all"]:
            passed, d = ev(sub, ctx)
            details.append(f"[{'OK' if passed else 'X'}]{d}")
            if not passed:
                return (False, "; ".join(details))
        return (True, "; ".join(details))
    if k == "any":
        details = []
        for sub in check["of"]:
            passed, d = ev(sub, ctx)
            details.append(f"[{'OK' if passed else 'X'}]{d}")
            if passed:
                return (True, "any: " + "; ".join(details))
        return (False, "any (none): " + "; ".join(details))
    return (False, f"unknown check kind: {k}")


def run_one(t):
    args = build_args(t)
    rec = {"id": t["id"], "group": t.get("group", ""), "mode": t["mode"],
           "intent": t.get("intent", " ".join(t.get("cmd", []))), "args": args,
           "route_ns": None, "route_outcome": None, "route_executed": None,
           "route_effect": None, "exit_code": None, "latency_ms": None,
           "timed_out": False, "verdict": None, "detail": None,
           "stdout_excerpt": None, "stderr_excerpt": None}

    # produce piped stdin if requested
    stdin_bytes = None
    if t.get("stdin_cmd"):
        stdin_bytes = subprocess.run(["sh", "-c", t["stdin_cmd"]], cwd=WORK, capture_output=True).stdout

    start = time.perf_counter()
    try:
        if t.get("pty"):
            shellcmd = " ".join(shlex.quote(a) for a in [UHM] + args)
            feed = b"c\n" if t["pty"] == "review_cancel" else None
            proc = subprocess.run(["script", "-q", "-e", "-c", shellcmd, "/dev/null"],
                                  input=feed, cwd=WORK, env=ENV,
                                  capture_output=True, timeout=PTY_TIMEOUT)
            so, se, rc = proc.stdout.decode("utf-8", "replace"), proc.stderr.decode("utf-8", "replace"), proc.returncode
            rec["pty"] = t["pty"]
        else:
            proc = subprocess.run([UHM] + args, input=stdin_bytes, cwd=WORK, env=ENV,
                                  capture_output=True, timeout=TIMEOUT)
            so, se, rc = proc.stdout.decode("utf-8", "replace"), proc.stderr.decode("utf-8", "replace"), proc.returncode
    except subprocess.TimeoutExpired:
        rec["latency_ms"] = int((time.perf_counter() - start) * 1000)
        rec["timed_out"] = True
        rec["exit_code"] = None
        rec["verdict"] = "FAIL"
        rec["detail"] = f"timeout after {(PTY_TIMEOUT if t.get('pty') else TIMEOUT)}s"
        rec["stdout_excerpt"] = ""
        rec["stderr_excerpt"] = ""
        return rec

    rec["latency_ms"] = int((time.perf_counter() - start) * 1000)
    rec["exit_code"] = rc

    # persist full artifacts
    with open(f"{OUT}/{t['id']}.stdout", "w") as f:
        f.write(so)
    with open(f"{OUT}/{t['id']}.stderr", "w") as f:
        f.write(se)

    env_obj = parse_envelope(se) or parse_envelope(so)
    rec["route_ns"] = env_obj.get("namespace")
    rec["route_outcome"] = env_obj.get("outcome")
    rec["route_executed"] = env_obj.get("executed")
    rec["route_effect"] = env_obj.get("message")

    ctx = {"rc": rc, "stdout": so, "stderr": se}
    passed, detail = ev(t["check"], ctx)
    rec["verdict"] = "PASS" if passed else "FAIL"
    rec["detail"] = detail
    rec["stdout_excerpt"] = so.strip()[:300]
    rec["stderr_excerpt"] = (env_obj and json.dumps(env_obj)) or se.strip()[-300:]
    return rec


def main():
    results = []
    print(f"running {len(battery)} tasks (cwd={WORK}, provider={PROVIDER}, model={MODEL}, telemetry=off)")
    for t in battery:
        rec = run_one(t)
        results.append(rec)
        with open(f"{ROOT}/results.jsonl", "a") as f:  # incremental append
            f.write(json.dumps(rec) + "\n")
        flag = "✓" if rec["verdict"] == "PASS" else "✗"
        extra = " [pty]" if rec.get("pty") else ""
        tout = " TIMEOUT" if rec["timed_out"] else ""
        print(f"  {flag} {t['id']:>3} {t.get('group',''):<16} rc={str(rec['exit_code']):>4} "
              f"{rec['latency_ms']:>6}ms{extra}{tout}  {rec['detail'][:90]}")
    # summary
    n = len(results)
    p = sum(1 for r in results if r["verdict"] == "PASS")
    print(f"\nDONE: {p}/{n} PASS")
    json.dump(results, open(f"{ROOT}/results_all.json", "w"), indent=2)


if __name__ == "__main__":
    # fresh results file each run
    try:
        os.remove(f"{ROOT}/results.jsonl")
    except FileNotFoundError:
        pass
    main()
