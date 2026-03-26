FROM rust:1.83-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

RUN cargo build --release -p image-resize
RUN ./target/release/image-resize --manifest > /build/worker.yaml

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/image-resize /worker
COPY --from=builder /build/worker.yaml /iii/worker.yaml

ENV III_ENGINE_URL=ws://host.docker.internal:49134

ENTRYPOINT ["/worker"]
CMD ["--url", "ws://host.docker.internal:49134"]
