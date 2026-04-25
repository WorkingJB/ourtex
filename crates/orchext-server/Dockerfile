# syntax=docker/dockerfile:1.7
#
# Multi-stage build for ourtex-server.
# - Stage 1 (builder) compiles the workspace against a full Rust toolchain.
# - Stage 2 (runtime) is a debian-slim image with only the binary and
#   the CA bundle for outbound HTTPS (OAuth IdPs, etc.).
#
# The image is built from the repo root so `docker build -f
# crates/ourtex-server/Dockerfile .` sees the entire Cargo workspace.

ARG RUST_VERSION=1.82

# --------------------------- builder ---------------------------
FROM rust:${RUST_VERSION}-slim-bookworm AS builder

RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

# `--locked` would reject any Cargo.lock drift; we don't enforce that
# at image-build time so the same Dockerfile can build from a branch
# that has lock updates pending. CI should run `cargo check --locked`
# separately.
RUN cargo build --release -p ourtex-server

# --------------------------- runtime ---------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --system --gid 1000 ourtex \
 && useradd --system --uid 1000 --gid ourtex --no-create-home ourtex

COPY --from=builder /src/target/release/ourtex-server /usr/local/bin/ourtex-server

USER ourtex
EXPOSE 8080

# DATABASE_URL is required. OURTEX_BIND defaults to 0.0.0.0:8080 inside
# the process itself; see src/config.rs.
ENV OURTEX_BIND=0.0.0.0:8080

# Healthcheck is defined in docker-compose.yml (HTTP GET /healthz) so
# we don't need to bake curl or an extra binary into the image.

ENTRYPOINT ["/usr/local/bin/ourtex-server"]
