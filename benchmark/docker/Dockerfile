FROM rust:1.82-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY assets ./assets
COPY model-qualification-policy-v1.json model-qualification-manifest.json model-qualification-holdout-v1.json ./
COPY scripts/provider-bakeoff.py ./scripts/provider-bakeoff.py
COPY scripts/qualification_policy.py scripts/provider-qualification-manifest.py scripts/seal-qualification-holdout.py ./scripts/
COPY tests/fixtures/provider-execution-benchmark-v2.json ./tests/fixtures/provider-execution-benchmark-v2.json
COPY benchmark/worker/worker.py ./benchmark/worker/worker.py
COPY benchmark/docker/Dockerfile ./benchmark/docker/Dockerfile
RUN cargo build --locked --release --bin uhm-bench-contract --bin uhm-bench-exec

FROM debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818

RUN apt-get update && apt-get install -y --no-install-recommends \
    bash coreutils findutils grep sed gawk diffutils git python3 jq ripgrep \
    file tar gzip xz-utils zip unzip procps iproute2 util-linux ca-certificates \
    && rm -rf /var/lib/apt/lists/*

ARG UHM_BENCH_CORPUS_SHA256
ARG UHM_BENCH_RUST_SOURCE_SHA256
ENV UHM_BENCH_CORPUS_SHA256=${UHM_BENCH_CORPUS_SHA256}
ENV UHM_BENCH_RUST_SOURCE_SHA256=${UHM_BENCH_RUST_SOURCE_SHA256}

RUN groupadd --gid 10001 bench && useradd --uid 10001 --gid 10001 --no-create-home bench \
    && mkdir -p /opt/uhm-bench /work /tmp/home \
    && chown -R 10001:10001 /work /tmp/home

COPY benchmark/worker/worker.py /opt/uhm-bench/worker.py
COPY benchmark/worker/tool_manifest.py /opt/uhm-bench/tool_manifest.py
COPY benchmark/docker/Dockerfile /opt/uhm-bench/Dockerfile
COPY benchmark/schemas /opt/uhm-bench/schemas
COPY --from=build /src/target/release/uhm-bench-contract /opt/uhm-bench/uhm-bench-contract
COPY --from=build /src/target/release/uhm-bench-exec /opt/uhm-bench/uhm-bench-exec
RUN chmod 0555 /opt/uhm-bench/worker.py /opt/uhm-bench/tool_manifest.py \
    /opt/uhm-bench/uhm-bench-contract /opt/uhm-bench/uhm-bench-exec \
    && python3 /opt/uhm-bench/tool_manifest.py > /opt/uhm-bench/tool-manifest.json

USER 10001:10001
WORKDIR /work
ENTRYPOINT ["python3", "/opt/uhm-bench/worker.py"]
