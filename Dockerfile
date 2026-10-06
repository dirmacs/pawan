# syntax=docker/dockerfile:1
#
# pawan — HTTP API (pawan-api) + CLI (the `pawan` binary) workspace.
#
# The runtime image carries the pawan-api service binary. pawan-api binds
# 0.0.0.0:$PAWAN_API_PORT (default 3300) and is the service-shaped member;
# the CLI binary (`pawan`) is an operator/CI tool and is also installed for
# in-container use.
#
# Multi-stage build:
#   1. builder: compiles the release binaries. Needs a C toolchain because the
#      dependency tree pulls ring, rusqlite (bundled SQLite), tree-sitter,
#      and rs-utcp (gRPC protos via protoc), all of which compile C/protos.
#   2. runtime: slim debian; the binaries statically link the bundled C libs,
#      so no runtime C deps are needed beyond ca-certificates (TLS to upstreams).
#
# Build:  docker build -t pawan .
# Run:    docker run --rm -p 3300:3300 pawan            # serves the API
#         docker run --rm pawan pawan --help            # run the CLI instead
#
# Runtime configuration:
#   PAWAN_API_PORT  listen port (default 3300)
#
# Note: pawan's CI floats @stable (with a 1.94 release pin in release.yml) and
# the workspace declares no rust-version floor; the builder uses the current
# stable line (1.99) and Cargo.lock IS committed, so the build is --locked
# reproducible.

# ---- builder ---------------------------------------------------------------
FROM rust:1.99-bookworm AS builder

WORKDIR /build

# Cargo.lock is committed in this repo, so the build resolves reproducibly.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build the API service and the CLI. The CLI package is named `pawan` (binary
# also `pawan`). ring/rusqlite/tree-sitter need cc + pkg-config; rs-utcp needs
# protoc for gRPC proto compilation — install protobuf-compiler explicitly.
RUN apt-get update \
 && apt-get install -y --no-install-recommends protobuf-compiler pkg-config \
 && rm -rf /var/lib/apt/lists/*
RUN cargo build --release --locked -p pawan-api -p pawan

# ---- runtime ---------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends tini ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Run as an unprivileged user.
RUN useradd --system --uid 10021 --create-home pawan

COPY --from=builder /build/target/release/pawan-api /usr/local/bin/pawan-api
COPY --from=builder /build/target/release/pawan /usr/local/bin/pawan

USER pawan

ENV PAWAN_API_PORT=3300

EXPOSE 3300

ENTRYPOINT ["/usr/bin/tini", "--"]
# Default to the service; override with e.g. `pawan ...` to run the CLI.
CMD ["pawan-api"]
