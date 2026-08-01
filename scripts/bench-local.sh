#!/usr/bin/env bash
set -euo pipefail

if ! command -v python3 >/dev/null; then
  echo "python3 is required for timing" >&2
  exit 2
fi

cargo build --release --quiet
bench_root="$(mktemp -d)"
trap 'rm -r "$bench_root"' EXIT
mkdir -p "$bench_root/config/uhm"
printf '%s\n' 'history:' '  enabled: false' 'aliases:' '  local-noop: true' > "$bench_root/config/uhm/config.yaml"

python3 - "$bench_root" <<'PY'
import os, statistics, subprocess, sys, time
root = sys.argv[1]
env = os.environ.copy()
env.update(HOME=root, XDG_CONFIG_HOME=root + "/config", XDG_DATA_HOME=root + "/data", XDG_CACHE_HOME=root + "/cache", TERM="dumb")
samples = []
for _ in range(100):
    start = time.perf_counter_ns()
    subprocess.run(["target/release/uhm", "--plain", "local-noop"], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    samples.append((time.perf_counter_ns() - start) / 1_000_000)
samples.sort()
print(f"local alias: p50={statistics.median(samples):.2f}ms p95={samples[94]:.2f}ms n={len(samples)}")
if samples[94] >= 25:
    raise SystemExit("p95 exceeds the 25ms release budget")
PY
