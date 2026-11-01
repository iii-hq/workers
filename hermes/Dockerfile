# syntax=docker/dockerfile:1.7

FROM ghcr.io/astral-sh/uv:0.12.5@sha256:e85be844203885286c60ffad8a858d48afb6c5a5c237ca0e67f12e74b8f174b1 AS uv
FROM python:3.11-slim@sha256:1042b61448fef4ba92d16a8c7eb4996d027568ce64792a7877fd88511e0af7c6 AS runtime

ARG SOURCE_DATE_EPOCH=0
ARG TARGETARCH

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    UV_CACHE_DIR=/tmp/uv-cache \
    UV_FROZEN=1 \
    UV_LINK_MODE=copy \
    UV_PYTHON=/usr/local/bin/python3.11 \
    UV_PYTHON_DOWNLOADS=never

COPY --from=uv /uv /uvx /usr/local/bin/

# Hermes is installed from one reviewed upstream commit. The checksum covers
# the source archive before its committed uv.lock is used for the environment.
ADD --checksum=sha256:8710e0017792e78369a7da3d96c16141f0787374285f1a6cf6f80d29b7b9ea2c \
    https://github.com/NousResearch/hermes-agent/archive/5fc308a70719a83cccdbba4c0e39c23f5a8239d5.tar.gz \
    /tmp/hermes-agent.tar.gz
RUN mkdir -p /opt/hermes-agent \
    && tar -xzf /tmp/hermes-agent.tar.gz --strip-components=1 -C /opt/hermes-agent \
    && uv sync --project /opt/hermes-agent --frozen --no-dev \
    && test -x /opt/hermes-agent/.venv/bin/hermes \
    && find /opt/hermes-agent/.venv -path '*.dist-info/uv_cache.json' -delete \
    && find /opt/hermes-agent/.venv -path '*.dist-info/RECORD' -exec sed -i '/uv_cache\.json,/d' {} + \
    && rm -rf /tmp/uv-cache \
    && find /opt/hermes-agent -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} +

# Download the exact iii release asset for the BuildKit target architecture,
# validate its reviewed checksum, and install only the binary from the archive.
# The agent runs `iii call <function>` (src/iii_prompt.py), so the build fails
# when the bundled CLI has no `call` subcommand.
RUN case "${TARGETARCH}" in \
      amd64) iii_target=x86_64-unknown-linux-gnu; iii_sha=383c3679bb3e146803299cc6a8aafc82bf49119b20267888e797a244896bb7fb ;; \
      arm64) iii_target=aarch64-unknown-linux-gnu; iii_sha=78512ccba2a14db9e3f829009d04f6b7db61d87314c7c15b94124b132f082992 ;; \
      *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 2 ;; \
    esac \
    && iii_archive="iii-${iii_target}.tar.gz" \
    && iii_url="https://github.com/iii-hq/iii/releases/download/iii/v0.26.0-rc.1/${iii_archive}" \
    && python -c 'import sys, urllib.request; urllib.request.urlretrieve(sys.argv[1], sys.argv[2])' "${iii_url}" "/tmp/${iii_archive}" \
    && echo "${iii_sha}  /tmp/${iii_archive}" | sha256sum -c - \
    && tar -xzf "/tmp/${iii_archive}" -C /usr/local/bin iii \
    && test -x /usr/local/bin/iii \
    && iii --no-update-check call --help >/dev/null

WORKDIR /app
COPY pyproject.toml uv.lock ./
COPY src/ src/
# The environment is installed from uv.lock as committed; `uv lock --check`
# first fails the build when the lock does not satisfy pyproject.toml.
RUN env -u UV_FROZEN uv lock --check \
    && UV_PROJECT_ENVIRONMENT=/app/.venv uv sync --frozen --no-dev --no-editable \
    && find /app/.venv -path '*.dist-info/uv_cache.json' -delete \
    && find /app/.venv -path '*.dist-info/RECORD' -exec sed -i '/uv_cache\.json,/d' {} + \
    && rm -rf /tmp/uv-cache \
    && find /app -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} +

ENV PATH="/app/.venv/bin:/opt/hermes-agent/.venv/bin:${PATH}"

CMD ["hermes-worker"]
