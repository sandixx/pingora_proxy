# Build stage
FROM rust:1.87 AS builder
WORKDIR /usr/src/app

# Install musl + OpenSSL + CMake + C++ compiler
RUN rustup target add x86_64-unknown-linux-musl && \
    apt-get update && apt-get install -y --no-install-recommends \
    musl-tools musl-dev cmake pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# Copy source & build
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl

FROM debian:bookworm-slim AS tzdata
RUN apt-get update && apt-get install -y --no-install-recommends \
    tzdata && \
    rm -rf /var/lib/apt/lists/*

# Final stage
FROM scratch
WORKDIR /app
COPY --from=builder /usr/src/app/target/x86_64-unknown-linux-musl/release/pingora_proxy /app/pingora_proxy
# Copy timezone database
COPY --from=tzdata /usr/share/zoneinfo /usr/share/zoneinfo

# Set timezone
ENV TZ=Asia/Jakarta

CMD ["/app/pingora_proxy"]
