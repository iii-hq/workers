FROM python:3.12-slim

WORKDIR /app

COPY pyproject.toml ./
COPY src/ src/

RUN pip install --no-cache-dir .

ENV III_URL=ws://localhost:49134

CMD ["todo-worker"]
