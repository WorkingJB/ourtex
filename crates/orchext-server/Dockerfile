# syntax=docker/dockerfile:1.7
#
# Multi-stage build for orchext-server.
#
# Stages:
#   1. chef     — base image with `cargo-chef` installed (Rust + tool).
#   2. planner  — analyses the workspace and produces `recipe.json`,
#                 a deterministic snapshot of every dependency.
#   3. builder  — restores deps from the recipe (this layer is cached
#                 across code-only changes), then compiles the binary.
#   4. runtime  — debian-slim with the binary, ca-certificates, and
#                 a tiny `wget` for HEALTHCHECK. Runs as a non-root user.
#
# The image is built from the repo root so `docker build -f
# crates/orchext-server/Dockerfile .` sees the entire Cargo workspace.

ARG RUST_VERSION=1.95

# --------------------------- chef base ---------------------------
FROM rust:${RUST_VERSION}-slim-bookworm AS chef

# `cargo-chef` is the canonical pattern for caching Cargo workspace
# deps in Docker — without it, every code change invalidates the
# build cache and we recompile every dep on every push.
# https://github.com/LukeMathWalker/cargo-chef
RUN cargo install cargo-chef --locked

WORKDIR /src

# --------------------------- planner ---------------------------
FROM chef AS planner

COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --------------------------- builder ---------------------------
FROM chef AS builder

# Build only the deps from the recipe. As long as Cargo.toml/Cargo.lock
# don't change, this layer is cached even when src/ changes — drops
# rebuild time from ~6 minutes to ~30 seconds.
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p orchext-server

# Now bring in the actual source and build the binary. Only this
# (relatively cheap) step re-runs on most pushes.
COPY . .
RUN cargo build --release -p orchext-server

# --------------------------- runtime ---------------------------
FROM debian:bookworm-slim AS runtime

# `ca-certificates` is needed for outbound HTTPS (OAuth IdPs, etc.).
# `wget` is added solely so the Dockerfile HEALTHCHECK below works
# without any further dependencies. It's ~500KB in slim and avoids
# pulling in curl's larger dep tree.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates wget \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --system --gid 1000 orchext \
 && useradd --system --uid 1000 --gid orchext --no-create-home orchext

COPY --from=builder /src/target/release/orchext-server /usr/local/bin/orchext-server

USER orchext
EXPOSE 8080

# DATABASE_URL is required. ORCHEXT_BIND defaults to 0.0.0.0:8080 inside
# the process itself; see src/config.rs.
ENV ORCHEXT_BIND=0.0.0.0:8080

# Liveness probe for any orchestrator that honours Dockerfile
# HEALTHCHECK (docker-compose, Nomad, etc.). Fly.io ignores this and
# uses `[[http_service.checks]]` from fly.toml instead — see
# docs/DEPLOYMENT.md §4.2.
HEALTHCHECK --interval=15s --timeout=5s --start-period=10s --retries=3 \
  CMD wget --quiet --tries=1 --spider http://localhost:8080/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/orchext-server"]
