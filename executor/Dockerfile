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
WORKDIR /
RUN cargo new ytdl-executor
WORKDIR /ytdl-executor
COPY Cargo.toml .
#COPY Cargo.lock .
RUN cargo build \
    && rm src/*.rs \
    && rm ./target/debug/ytdl-executor*
COPY src ./src
RUN cargo build
FROM debian:bullseye-slim
WORKDIR /
COPY --from=builder /ytdl-executor/target/debug/ytdl-executor .
CMD ["/ytdl-executor"]