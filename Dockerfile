# syntax=docker/dockerfile:1.7

# Pawan — multi-stage build. Produces the two workspace binaries:
#   pawan      (agent CLI)
#   pawan-api  (HTTP server; binds 0.0.0.0, default port 3300, PAWAN_API_PORT override)
#
# ares-server 0.7.5 (default-features = false, openai+ollama) carries no
# sqlx::query! macros, so no DATABASE_URL or .sqlx cache is needed to build.

FROM rust:1.98-bookworm AS builder

# prost-build (gRPC) shells out to protoc to compile .proto files.
RUN apt-get update && \
    apt-get install -y --no-install-recommends protobuf-compiler && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Dependency manifest first for layer caching; then the workspace sources.
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked --bin pawan --bin pawan-api && \
    cp /app/target/release/pawan /tmp/pawan && \
    cp /app/target/release/pawan-api /tmp/pawan-api

FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --create-home --uid 1000 --shell /usr/sbin/nologin pawan

WORKDIR /app

COPY --from=builder /tmp/pawan /usr/local/bin/pawan
COPY --from=builder /tmp/pawan-api /usr/local/bin/pawan-api

RUN chown -R pawan:pawan /app

USER pawan

ENV RUST_LOG=info \
    PAWAN_API_PORT=3300

EXPOSE 3300

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS "http://127.0.0.1:${PAWAN_API_PORT}/api/health" || exit 1

# Default to the HTTP server; override with ["pawan", ...] to run the CLI.
CMD ["pawan-api"]
