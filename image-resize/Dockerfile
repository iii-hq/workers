FROM --platform=$BUILDPLATFORM rust:1.88-slim AS builder

ARG TARGETARCH
ARG TARGETOS

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    gcc-x86-64-linux-gnu \
    gcc-aarch64-linux-gnu \
    libc6-dev-amd64-cross \
    libc6-dev-arm64-cross \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/

RUN case "${TARGETARCH}" in \
      amd64) \
        TARGET_TRIPLE="x86_64-unknown-linux-gnu"; \
        export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc; \
        ;; \
      arm64) \
        TARGET_TRIPLE="aarch64-unknown-linux-gnu"; \
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc; \
        ;; \
      *) \
        echo "Unsupported TARGETARCH=${TARGETARCH}" >&2; \
        exit 1; \
        ;; \
    esac && \
    cargo build --release --target "${TARGET_TRIPLE}" && \
    cp "target/${TARGET_TRIPLE}/release/image-resize" /worker

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /worker /worker

ENV III_ENGINE_URL=ws://host.containers.internal:49134

ENTRYPOINT ["/worker"]
CMD ["--url", "ws://host.containers.internal:49134"]
