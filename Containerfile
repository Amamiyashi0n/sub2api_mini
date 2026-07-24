FROM docker.io/library/rust:1.97.1-alpine3.22 AS builder

RUN apk add --no-cache build-base
WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY build.rs ./
COPY migrations ./migrations
COPY src ./src
COPY web ./web

RUN --mount=type=cache,id=sub2api-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=sub2api-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=sub2api-target,target=/build/target \
    cargo build --release --locked \
    && install -D -m 0755 target/release/sub2api_mini /out/sub2api_mini

FROM docker.io/library/alpine:3.22.5

RUN apk add --no-cache ca-certificates \
    && addgroup -S -g 1000 amamiya \
    && adduser -S -D -H -u 1000 -G amamiya -s /sbin/nologin amamiya \
    && install -d -o amamiya -g amamiya /data/sub2api_mini

COPY --from=builder --chown=1000:1000 /out/sub2api_mini /usr/local/bin/sub2api_mini

ENV HOME=/tmp \
    SUB2API_MINI_ENV_FILE=/data/sub2api_mini/.env

WORKDIR /data/sub2api_mini
USER 1000:1000
EXPOSE 8080 1455

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -q -O /dev/null http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/sub2api_mini"]
