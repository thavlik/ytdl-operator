FROM rust:1.67.1-slim-bullseye AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        libssl-dev \
        pkg-config \
        wget \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /ytdl-operator
COPY common common
COPY types types
COPY Cargo.lock .
WORKDIR /ytdl-operator/operator
COPY operator/Cargo.toml .
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs \
    && cargo build \
    && rm -rf src \
    && rm target/debug/ytdl-operator*
COPY operator/src src
RUN cargo build
FROM debian:bullseye-slim
WORKDIR /
COPY --from=builder /ytdl-operator/operator/target/debug/ytdl-operator .
ENV RUST_BACKTRACE=1
CMD ["/ytdl-operator"]