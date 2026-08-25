FROM ghcr.io/astral-sh/uv:0.8.14 AS uv

FROM python:3.12-slim AS runtime

COPY --from=uv /uv /uvx /bin/

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    YELLO_WEB_HOST=0.0.0.0 \
    YELLO_WEB_PORT=8080 \
    YELLO_DEVICE_HOST=0.0.0.0 \
    YELLO_DEVICE_WS_PORT=8765 \
    YELLO_DATA_DIR=/data

WORKDIR /app
COPY server/pyproject.toml server/uv.lock ./
RUN uv sync --frozen --no-dev --no-install-project

COPY server/ ./
RUN mkdir -p /data && chown -R 10001:10001 /app /data

USER 10001:10001
VOLUME ["/data"]
EXPOSE 8080 8765

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD ["python", "-c", "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8080/api/status', timeout=3).read()"]

CMD ["/app/.venv/bin/python", "server.py"]
