# Dependency caching uses cargo-chef (the pre-built lukemathwalker/cargo-chef
# image): a planner stage distills the manifests + sources into recipe.json,
# and the builder "cooks" the whole dependency tree from that recipe in its
# own layer. The layer is reused unless the dependency graph itself changes,
# so ordinary code pushes skip recompiling every dependency (including the
# vendored OpenSSL build) and only rebuild the yello-server crate.
# The chef image tracks the rust 1-slim-bookworm alias, keeping the build on
# bookworm: the runtime below is debian:bookworm-slim (glibc 2.36), and the
# floating rust:1-slim now uses a newer Debian whose glibc it cannot load.
FROM lukemathwalker/cargo-chef:latest-rust-1-slim-bookworm AS chef

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential perl pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

FROM chef AS planner
COPY server-rs/Cargo.toml server-rs/Cargo.lock ./
COPY server-rs/src ./src
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
# Both cargo caches (registry downloads and the target dir) are BuildKit
# cache mounts, which the workflow's `cache-to: type=gha, mode=max`
# persists between runs: on a recipe change the cook below reuses compiled
# artifacts instead of starting over.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo chef cook --release --recipe-path recipe.json
COPY server-rs/Cargo.toml server-rs/Cargo.lock ./
COPY server-rs/src ./src
# The binary is copied out of the cache-mounted target dir: mounts are not
# part of the image layer, so the runtime stage copies from /out instead.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release && mkdir -p /out && cp target/release/yello-server /out/yello-server

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -r -g 10001 yello && useradd -r -g yello -u 10001 yello \
    && mkdir -p /data && chown yello:yello /data

ENV YELLO_WEB_HOST=0.0.0.0 \
    YELLO_WEB_PORT=8080 \
    YELLO_DEVICE_HOST=0.0.0.0 \
    YELLO_DEVICE_WS_PORT=8765 \
    YELLO_STUDIO_PATH=/data/studio.json \
    YELLO_SECRETS_PATH=/data/secrets.json \
    YELLO_STATIC_DIR=/app/static

WORKDIR /app
COPY --from=builder /out/yello-server ./
COPY server-rs/static ./static

USER 10001:10001
VOLUME ["/data"]
EXPOSE 8080 8765

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD ["curl", "-fsS", "http://127.0.0.1:8080/api/status"]

CMD ["/app/yello-server"]
