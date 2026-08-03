#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
tag="${UHM_BENCH_IMAGE:-uhm-bench-worker:v2}"
corpus="${UHM_BENCH_CORPUS:-$root/tests/fixtures/provider-execution-benchmark-v2.json}"
corpus_sha256="$(sha256sum "$corpus" | cut -d ' ' -f 1)"
rust_source_sha256="$(python3 - "$root" <<'PY'
import hashlib, pathlib, sys
root=pathlib.Path(sys.argv[1]); digest=hashlib.sha256()
for path in sorted(list((root/'src').rglob('*.rs'))+list((root/'assets'/'shell').glob('*'))):
    digest.update(path.relative_to(root).as_posix().encode()+b'\0'+path.read_bytes())
print(digest.hexdigest())
PY
)"
docker build --build-arg "UHM_BENCH_CORPUS_SHA256=$corpus_sha256" --build-arg "UHM_BENCH_RUST_SOURCE_SHA256=$rust_source_sha256" --file "$root/benchmark/docker/Dockerfile" --tag "$tag" "$root"
docker image inspect --format '{{.Id}}' "$tag"
identity="$(docker run --rm --network none --read-only --entrypoint python3 "$tag" -c 'import json; print(json.load(open("/opt/uhm-bench/tool-manifest.json"))["identity_sha256"])')"
content_tag="uhm-bench-worker:sha256-${identity:0:16}"
docker tag "$tag" "$content_tag"
printf '%s\n' "$content_tag"
