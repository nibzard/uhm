#!/usr/bin/env python3
"""Active Docker boundary canaries; opt in with UHM_BENCH_DOCKER_TESTS=1."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("provider_bakeoff", ROOT / "scripts/provider-bakeoff.py")
BENCH = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(BENCH)


@unittest.skipUnless(os.environ.get("UHM_BENCH_DOCKER_TESTS") == "1", "Docker integration is opt-in")
class ContainmentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.image = os.environ.get("UHM_BENCH_IMAGE", BENCH.DEFAULT_WORKER_IMAGE)
        BENCH.worker_image_id(cls.image)

    def execute(self, command, wall_ms=3000, stdout_bytes=4096):
        action = {"tool": "run_shell", "arguments": {"command": command, "summary": "Run containment canary", "assumptions": [], "effects": [], "requirements": ["bash"], "stdin_mode": "none"}}
        envelope = {"contract_version": 2,
                    "fixture": {"cwd": "/work", "stdin": None, "directories": [], "files": [], "symlinks": [], "environment": {}, "git": None},
                    "limits": {"wall_ms": wall_ms, "stdout_bytes": stdout_bytes, "stderr_bytes": 4096, "workspace_bytes": 1048576},
                    "action": action}
        process = subprocess.run(BENCH.worker_command(self.image), input=json.dumps(envelope), text=True, capture_output=True, timeout=wall_ms / 1000 + 10)
        result = json.loads(process.stdout); BENCH.schema_validator("worker-result.schema.json").validate(result)
        return result

    def test_network_is_unreachable(self):
        result = self.execute("bash -c 'exec 3<>/dev/tcp/1.1.1.1/80'", 2000)
        self.assertNotEqual(result["exit_code"], 0)

    def test_secrets_host_and_answers_are_absent(self):
        result = self.execute("test -z \"${OPENAI_API_KEY+x}${CEREBRAS_API_KEY+x}${UHM_BENCH_HOST_SENTINEL+x}\" && test ! -e /var/run/docker.sock && test ! -e /opt/uhm-bench/corpus.json && test ! -e /workspace && test ! -r /root/.ssh/id_rsa")
        self.assertEqual(result["exit_code"], 0)

    def test_root_is_read_only_and_privileges_are_bounded(self):
        result = self.execute("! touch /rootfs-canary && ! unshare -Ur true")
        self.assertEqual(result["exit_code"], 0)

    def test_wall_and_output_limits_are_enforced(self):
        timed = self.execute("while :; do :; done", 200)
        self.assertTrue(timed["timed_out"])
        noisy = self.execute("head -c 4096 /dev/zero | tr '\\0' x", 1000, 128)
        self.assertTrue(noisy["stdout_truncated"])
        signaled = self.execute("kill -TERM $$", 1000)
        self.assertEqual(signaled["signal"], 15)

    def test_memory_pid_and_workspace_limits_are_enforced(self):
        controls = self.execute("test \"$(cat /sys/fs/cgroup/memory.max)\" = 536870912 && test \"$(cat /sys/fs/cgroup/pids.max)\" = 128")
        self.assertEqual(controls["exit_code"], 0)
        workspace = self.execute("dd if=/dev/zero of=too-large.bin bs=1M count=160 status=none", 5000)
        self.assertNotEqual(workspace["exit_code"], 0)
        pids = self.execute("i=0; while [ $i -lt 200 ]; do sleep 5 & i=$((i+1)); done; wait", 1000)
        self.assertTrue(pids["status"] == "error" or pids["timed_out"] or pids["exit_code"] != 0 or "Resource temporarily unavailable" in pids["stderr"])


if __name__ == "__main__": unittest.main()
