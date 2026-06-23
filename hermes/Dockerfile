FROM python:3.11-slim

# Hermes installer prerequisites (Python 3.11 is the base; the installer pulls
# its own uv venv, a Node 22 bridge, ripgrep, and ffmpeg).
RUN apt-get update && apt-get install -y --no-install-recommends \
      git curl xz-utils build-essential ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Install the Hermes agent CLI. The installer sets up `hermes` on PATH.
RUN curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash
ENV PATH="/root/.local/bin:${PATH}"

WORKDIR /app

COPY pyproject.toml ./
COPY src/ src/

RUN pip install --no-cache-dir .

ENV III_URL=ws://localhost:49134

CMD ["hermes-worker"]
