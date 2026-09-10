FROM rust:1-slim AS build

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential perl pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY server-rs/Cargo.toml server-rs/Cargo.lock ./
COPY server-rs/src ./src
RUN cargo build --release

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
COPY --from=build /src/target/release/yello-server ./
COPY server-rs/static ./static

USER 10001:10001
VOLUME ["/data"]
EXPOSE 8080 8765

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD ["curl", "-fsS", "http://127.0.0.1:8080/api/status"]

CMD ["/app/yello-server"]
