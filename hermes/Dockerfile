FROM python:3.11-slim

# Hermes installer prerequisites (Python 3.11 is the base; the installer pulls
# its own uv venv, a Node 22 bridge, ripgrep, and ffmpeg).
RUN apt-get update && apt-get install -y --no-install-recommends \
      git curl xz-utils build-essential ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Install the Hermes agent CLI. The installer sets up `hermes` on PATH.
RUN curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash

# Install the iii CLI so the Hermes agent can drive the engine live through its
# shell — `iii trigger engine::functions::list`, `<fn> --help`, etc. This is
# what the injected iii runtime context calls; without it the agent has the
# instructions but not the tool.
RUN curl -fsSL https://install.iii.dev/iii/main/install.sh | sh

ENV PATH="/root/.local/bin:${PATH}"

WORKDIR /app

COPY pyproject.toml ./
COPY src/ src/

RUN pip install --no-cache-dir .

ENV III_URL=ws://localhost:49134

CMD ["hermes-worker"]
