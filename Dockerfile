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

# Create resolv.conf in builder stage
FROM alpine:3.18 AS resolver
RUN echo "nameserver 8.8.8.8" > /resolv.conf && \
    echo "options ndots:0" >> /resolv.conf

FROM debian:bookworm-slim AS tzdata
RUN apt-get update && apt-get install -y --no-install-recommends \
    tzdata && \
    rm -rf /var/lib/apt/lists/*

# Final stage
FROM scratch
WORKDIR /app

# Copy the binary
COPY --from=builder /usr/src/app/target/x86_64-unknown-linux-musl/release/pingora_proxy /app/pingora_proxy

# Copy essential libraries and certificates
COPY --from=builder /lib/x86_64-linux-gnu/libc.so.6 /lib/x86_64-linux-gnu/libc.so.6
COPY --from=builder /lib/x86_64-linux-gnu/libdl.so.2 /lib/x86_64-linux-gnu/libdl.so.2
COPY --from=builder /lib/x86_64-linux-gnu/libpthread.so.0 /lib/x86_64-linux-gnu/libpthread.so.0
COPY --from=builder /lib/x86_64-linux-gnu/libm.so.6 /lib/x86_64-linux-gnu/libm.so.6
COPY --from=builder /lib/x86_64-linux-gnu/libgcc_s.so.1 /lib/x86_64-linux-gnu/libgcc_s.so.1
COPY --from=builder /lib64/ld-linux-x86-64.so.2 /lib64/ld-linux-x86-64.so.2

# Copy CA certificates for SSL/TLS
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt

# Copy resolv.conf (created in builder stage)
COPY --from=resolver /resolv.conf /etc/resolv.conf

# Copy timezone data
COPY --from=tzdata /usr/share/zoneinfo /usr/share/zoneinfo

# Set environment variables
ENV TZ=Asia/Jakarta
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt

CMD ["/app/pingora_proxy"]
