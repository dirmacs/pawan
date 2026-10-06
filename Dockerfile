# syntax=docker/dockerfile:1
#
# pawan — workspace build producing the `pawan-api` service binary and the
# `pawan` CLI. Multi-stage: a cargo builder with the native deps the build
# needs (protobuf-compiler for prost, pkg-config, a C toolchain), then a slim
# Debian runtime that carries only the linked binaries + TLS certs.
#
# Build:  DOCKER_BUILDKIT=1 docker build -t pawan .
# Run:    docker run --rm pawan            # pawan-api (the service)
#         docker run --rm --entrypoint pawan pawan --help   # the CLI

FROM rust:1.95-bookworm AS builder

# Native build deps. protobuf-compiler is required by the prost build in the
# workspace; pkg-config + a C toolchain cover the openssl/native crates.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      protobuf-compiler \
      pkg-config \
      libssl-dev \
      ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy the whole workspace. .dockerignore keeps target/ and other build junk
# out of the context so the source layer cache stays meaningful.
COPY . .

# Build both binaries in release. --locked keeps the build honest against the
# committed Cargo.lock (the same guarantee CI's --locked gives).
#
# The two BuildKit cache mounts persist the cargo registry and the target dir
# across builds, so re-building after a source-only change does not recompile
# all ~950 dependencies. Requires BuildKit (DOCKER_BUILDKIT=1), the default in
# current Docker. The cp step runs outside the cached target mount's shadow.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked -p pawan-api -p pawan \
 && cp target/release/pawan target/release/pawan-api /usr/local/bin/

# --- runtime ---------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      libssl3 \
 && rm -rf /var/lib/apt/lists/*

# Run unprivileged.
RUN useradd --system --uid 10001 --create-home pawan
USER pawan

COPY --from=builder /usr/local/bin/pawan /usr/local/bin/pawan
COPY --from=builder /usr/local/bin/pawan-api /usr/local/bin/pawan-api

# pawan-api is configured by environment (no CLI flags):
#   PAWAN_API_PORT  listen port (default 3300)
#   PAWAN_AGENT_ID  reported in /api/health (default pawan@<hostname>)
ENV PAWAN_API_PORT=3300

EXPOSE 3300

# /api/health returns {status:"ok", version, uptime_secs, agent_id} — a genuine
# liveness signal from the running service, not a capability probe.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:3300/api/health || exit 1

# The service binary is the default; the CLI is available via --entrypoint.
ENTRYPOINT ["pawan-api"]
