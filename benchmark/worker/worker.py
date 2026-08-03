#!/usr/bin/env python3
"""Keyless worker for one validated action; grading remains on the host."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import signal
import subprocess
import sys
import tempfile
import time

WORK = Path(os.environ.get("UHM_BENCH_WORK", "/work"))
BASE_ENV = {
    "PATH": "/usr/local/bin:/usr/bin:/bin",
    "LANG": "C.UTF-8",
    "LC_ALL": "C.UTF-8",
    "TZ": "UTC",
    "HOME": "/tmp/home",
    "TMPDIR": "/tmp",
    "PYTHONDONTWRITEBYTECODE": "1",
}
EXECUTOR = "/opt/uhm-bench/uhm-bench-exec"
EXECUTION_RESULT = Path("/tmp/uhm-bench-execution-result.json")


def safe_path(value: str) -> Path:
    path = PurePosixPath(value)
    if path.is_absolute() or not value or ".." in path.parts or "\0" in value:
        raise ValueError(f"unsafe fixture path {value!r}")
    return WORK.joinpath(*path.parts)


def write_file(spec: dict) -> None:
    path = safe_path(spec["path"])
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(spec["text"], encoding="utf-8")
    path.chmod(int(spec.get("mode", "0644"), 8))


def git_run(*args: str, env: dict | None = None) -> None:
    subprocess.run(
        ["git", *args], cwd=WORK, env=env or BASE_ENV, check=True,
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )


def create_fixture(fixture: dict) -> dict[str, str]:
    for child in WORK.iterdir():
        if child.is_dir() and not child.is_symlink(): shutil.rmtree(child)
        else: child.unlink()
    for value in fixture.get("directories", []): safe_path(value).mkdir(parents=True, exist_ok=True)
    for spec in fixture.get("files", []): write_file(spec)
    for spec in fixture.get("symlinks", []):
        path=safe_path(spec["path"]); path.parent.mkdir(parents=True,exist_ok=True); path.symlink_to(spec["target"])
    git = fixture.get("git")
    if git:
        git_run("init", "-q", "-b", git.get("branch", "main"))
        env={**BASE_ENV,"GIT_AUTHOR_NAME":"UHM Bench","GIT_AUTHOR_EMAIL":"bench@example.invalid","GIT_COMMITTER_NAME":"UHM Bench","GIT_COMMITTER_EMAIL":"bench@example.invalid","GIT_AUTHOR_DATE":"2000-01-01T00:00:00Z","GIT_COMMITTER_DATE":"2000-01-01T00:00:00Z"}
        for commit in git.get("commits", []):
            for path,text in commit.get("files",{}).items(): write_file({"path":path,"text":text})
            git_run("add","--all",env=env); git_run("commit","-q","-m",commit["message"],env=env)
        for path,text in git.get("staged",{}).items(): write_file({"path":path,"text":text}); git_run("add","--",path)
        for path,text in git.get("unstaged",{}).items(): write_file({"path":path,"text":text})
        for path,text in git.get("untracked",{}).items(): write_file({"path":path,"text":text})
    return {**BASE_ENV, **fixture.get("environment", {})}


def manifest() -> dict[str, dict]:
    result={}
    for path in sorted(WORK.rglob("*")):
        if len(result) >= 4096:
            raise ValueError("workspace contains more than 4096 entries")
        relative=path.relative_to(WORK).as_posix()
        if relative==".git" or relative.startswith(".git/"): continue
        stat=path.lstat()
        if path.is_symlink(): result[relative]={"type":"symlink","target":os.readlink(path)}
        elif path.is_dir(): result[relative]={"type":"directory","mode":oct(stat.st_mode & 0o777)}
        elif path.is_file():
            data=path.read_bytes(); result[relative]={"type":"file","size":len(data),"sha256":hashlib.sha256(data).hexdigest(),"mode":oct(stat.st_mode & 0o777)}
    return result


def run_process(argv: list[str], cwd: Path, env: dict[str,str], stdin_bytes: bytes, limits: dict) -> dict:
    start=time.perf_counter(); timed_out=False
    with tempfile.TemporaryFile(dir="/tmp") as stdout_file, tempfile.TemporaryFile(dir="/tmp") as stderr_file:
        proc=subprocess.Popen(argv,cwd=cwd,env=env,stdin=subprocess.PIPE,stdout=stdout_file,stderr=stderr_file,start_new_session=True)
        try: proc.communicate(stdin_bytes,timeout=limits["wall_ms"]/1000)
        except subprocess.TimeoutExpired:
            timed_out=True
            os.killpg(proc.pid,signal.SIGTERM)
            try: proc.wait(timeout=.5)
            except subprocess.TimeoutExpired: os.killpg(proc.pid,signal.SIGKILL); proc.wait()
        stdout_file.seek(0); stderr_file.seek(0)
        stdout=stdout_file.read(limits["stdout_bytes"]+1); stderr=stderr_file.read(limits["stderr_bytes"]+1)
    return {"started":True,"exit_code":proc.returncode,"signal":-proc.returncode if proc.returncode is not None and proc.returncode<0 else None,"timed_out":timed_out,"duration_ms":round((time.perf_counter()-start)*1000),"stdout":stdout[:limits["stdout_bytes"]].decode("utf-8","replace"),"stderr":stderr[:limits["stderr_bytes"]].decode("utf-8","replace"),"stdout_truncated":len(stdout)>limits["stdout_bytes"],"stderr_truncated":len(stderr)>limits["stderr_bytes"]}


def execute_production(action: dict, fixture: dict, limits: dict, env: dict) -> tuple[dict, dict | None]:
    EXECUTION_RESULT.unlink(missing_ok=True)
    envelope = {"action": action, "stdin": (fixture.get("stdin") or {}).get("text"), "limits": limits}
    supervisor_limits = {**limits, "wall_ms": limits["wall_ms"] + 2000}
    captured = run_process([EXECUTOR], WORK, env, json.dumps(envelope, separators=(",", ":")).encode(), supervisor_limits)
    if not EXECUTION_RESULT.is_file():
        raise ValueError(f"production executor failed with {captured['exit_code']}: {captured['stderr'][:300]}")
    metadata = json.loads(EXECUTION_RESULT.read_text(encoding="utf-8"))
    for name in ("started", "exit_code", "signal", "timed_out", "duration_ms"):
        captured[name] = metadata[name]
    captured["artifact_commit_success"] = metadata.get("artifact_commit_success")
    captured["helper_setup_ms"] = metadata.get("helper_setup_ms")
    if metadata.get("output_overflow"):
        captured["stdout_truncated"] = True
    return captured, metadata.get("parent_state")


def main() -> int:
    WORK.mkdir(parents=True, exist_ok=True)
    Path(BASE_ENV["HOME"]).mkdir(parents=True, exist_ok=True)
    envelope=json.load(sys.stdin)
    if set(envelope)!={"contract_version","fixture","limits","action"} or envelope["contract_version"]!=2: raise ValueError("invalid action envelope")
    fixture=envelope["fixture"]; limits=envelope["limits"]
    env=create_fixture(fixture); before=manifest(); action=envelope["action"]; tool=action["tool"]; args=action["arguments"]
    if tool not in {"run_shell", "run_program", "require_parent_shell"}: raise ValueError("worker only executes shell, program, and parent-shell actions")
    result,parent_state=execute_production(action,fixture,limits,env)
    after=manifest(); result["contract_version"]=2; result["status"]="success"
    result["before_manifest"]=[{"path":path,**value} for path,value in before.items()]
    result["after_manifest"]=[{"path":path,**value} for path,value in after.items()]
    result["parent_state"]=parent_state
    print(json.dumps(result,separators=(",",":"),ensure_ascii=False)); return 0


if __name__=="__main__":
    try: raise SystemExit(main())
    except Exception as error:
        print(json.dumps({"contract_version":2,"status":"error","error":{"code":"worker_error","message":str(error)[:1000]}},separators=(",",":")))
        raise SystemExit(2)
