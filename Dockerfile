FROM rust:1.87 AS builder

RUN apt-get update && apt-get install -y \
    cmake \
    pkg-config \
    libssl-dev \
    ca-certificates \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release && rm -rf src target/release/deps/pingora_proxy*
COPY src/ ./src/
RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

ENV TZ=Asia/Jakarta

RUN useradd -m -u 1001 appuser
WORKDIR /app
COPY --from=builder /usr/src/app/target/release/pingora_proxy /app/pingora_proxy
RUN chown -R appuser:appuser /app
USER appuser

ARG PROXY_PORT
ENV APP_PORT=${PROXY_PORT}
EXPOSE ${APP_PORT}

CMD ["./pingora_proxy"]