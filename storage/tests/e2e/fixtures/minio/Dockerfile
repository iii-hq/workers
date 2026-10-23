# Build the same MinIO/mc releases used by this fixture without relying on
# the retired public quay.io/minio images. No private registry credentials.
FROM golang:1.23.12-bookworm AS build
ENV CGO_ENABLED=0
WORKDIR /src/minio
RUN git init . \
    && git remote add origin https://github.com/minio/minio.git \
    && git fetch --depth=1 origin 16f8cf1c52f0a77eeb8f7565aaf7f7df12454583 \
    && git checkout --detach FETCH_HEAD \
    && test "$(git rev-parse HEAD)" = 16f8cf1c52f0a77eeb8f7565aaf7f7df12454583 \
    && go build -p 2 -trimpath -ldflags='-s -w' -o /out/minio .
WORKDIR /src/mc
RUN git init . \
    && git remote add origin https://github.com/minio/mc.git \
    && git fetch --depth=1 origin 1681e4497c09d7438a34e846f76dbde972ab7daf \
    && git checkout --detach FETCH_HEAD \
    && test "$(git rev-parse HEAD)" = 1681e4497c09d7438a34e846f76dbde972ab7daf \
    && go build -p 2 -trimpath -ldflags='-s -w' -o /out/mc .

FROM debian:bookworm-slim
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /out/minio /out/mc /usr/local/bin/
COPY --from=build /src/minio/LICENSE /usr/share/licenses/minio/LICENSE
COPY --from=build /src/mc/LICENSE /usr/share/licenses/mc/LICENSE
EXPOSE 9000 9001
ENTRYPOINT ["minio"]
