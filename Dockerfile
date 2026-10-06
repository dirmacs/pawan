# syntax=docker/dockerfile:1.7
#
# pawan — workspace build producing two binaries:
#   pawan-api  (HTTP server; binds 0.0.0.0, default port 3300, PAWAN_API_PORT override)
#   pawan      (agent CLI)
#
# Multi-stage: a cargo builder with the native deps the build needs
# (protobuf-compiler for prost/tonic, pkg-config, a C toolchain), then a slim
# Debian runtime carrying only the linked binaries + TLS certs + libssl3.
#
# pawan-api serves /api/health for healthchecks. No DATABASE_URL or .sqlx
# cache is needed to build (no sqlx::query! compile-time macros in the tree).
#
# Build:  docker build -t pawan .
# Run:    docker run --rm -p 3300:3300 pawan            # serves pawan-api
#         docker run --rm pawan pawan --help            # runs the CLI

FROM rust:1.99-bookworm AS builder

WORKDIR /app

# Native build deps: protobuf-compiler (prost/tonic codegen), pkg-config, and a
# C toolchain for crates with build scripts.
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      protobuf-compiler pkg-config clang && \
    rm -rf /var/lib/apt/lists/*

# Manifests first so source edits do not bust the dependency build.
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked --bin pawan-api --bin pawan && \
    cp /app/target/release/pawan-api /app/target/release/pawan /tmp/

FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl libssl3 && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --create-home --uid 1000 --shell /usr/sbin/nologin pawan

WORKDIR /app

COPY --from=builder /tmp/pawan-api /usr/local/bin/pawan-api
COPY --from=builder /tmp/pawan /usr/local/bin/pawan

RUN chown -R pawan:pawan /app

USER pawan

ENV RUST_LOG=info
EXPOSE 3300

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://127.0.0.1:3300/api/health || exit 1

CMD ["pawan-api"]
