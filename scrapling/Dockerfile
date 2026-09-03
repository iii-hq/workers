# syntax=docker/dockerfile:1.7

FROM ghcr.io/astral-sh/uv:0.12.5@sha256:e85be844203885286c60ffad8a858d48afb6c5a5c237ca0e67f12e74b8f174b1 AS uv
FROM mcr.microsoft.com/playwright/python:v1.60.0-noble@sha256:8ff591d613b01c884cc488339ed4318b4513eaf0c57a164a878ba49e70e3f384 AS runtime

ARG SOURCE_DATE_EPOCH=0

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    UV_CACHE_DIR=/tmp/uv-cache \
    UV_FROZEN=1 \
    UV_LINK_MODE=copy \
    UV_PYTHON=/usr/bin/python3.12 \
    UV_PYTHON_DOWNLOADS=never

COPY --from=uv /uv /uvx /usr/local/bin/

WORKDIR /app
COPY pyproject.toml uv.lock ./
COPY src/ src/
RUN UV_PROJECT_ENVIRONMENT=/app/.venv uv sync --frozen --no-dev --no-editable \
    && find /app/.venv -path '*.dist-info/uv_cache.json' -delete \
    && find /app/.venv -path '*.dist-info/RECORD' -exec sed -i '/uv_cache\.json,/d' {} + \
    && rm -rf /tmp/uv-cache \
    && find /app -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} +

ENV PATH="/app/.venv/bin:${PATH}"

CMD ["python", "-m", "src.main"]
