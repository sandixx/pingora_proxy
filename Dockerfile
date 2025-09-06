FROM rust:1.87-slim AS builder

RUN apt-get update && apt-get install -y \
    cmake \
    pkg-config \
    libssl-dev \
    ca-certificates \
    build-essential \
    musl-tools \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# Set Rust to build with MUSL (static binary)
RUN rustup target add x86_64-unknown-linux-musl

# Pre-cache dependencies
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release --target x86_64-unknown-linux-musl
RUN rm -rf src target/x86_64-unknown-linux-musl/release/deps/pingora_proxy*

# Copy actual source and build
COPY src/ ./src/
RUN cargo build --release --target x86_64-unknown-linux-musl

# Strip binary to reduce size
RUN strip target/x86_64-unknown-linux-musl/release/pingora_proxy

# ---- Runtime stage ----
FROM gcr.io/distroless/static-debian12:nonroot

# Copy zoneinfo data from builder stage
COPY --from=builder /usr/share/zoneinfo /usr/share/zoneinfo

WORKDIR /app
COPY --from=builder /usr/src/app/target/x86_64-unknown-linux-musl/release/pingora_proxy /app/pingora_proxy

# Use non-root user from distroless
USER nonroot:nonroot

ARG PROXY_PORT
ENV APP_PORT=${PROXY_PORT}
EXPOSE ${APP_PORT}

CMD ["/app/pingora_proxy"]
