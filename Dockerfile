# ==============================================================================
# SoliDB Docker Image
# Multi-stage build for minimal image size
# ==============================================================================

# ------------------------------------------------------------------------------
# Stage 1: Build
# ------------------------------------------------------------------------------
FROM rust:1.83-bookworm AS builder

# Install build dependencies for RocksDB and other native libs
RUN apt-get update && apt-get install -y \
    build-essential \
    clang \
    libclang-dev \
    pkg-config \
    libssl-dev \
    libzstd-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./

# Create dummy main.rs to build dependencies
RUN mkdir -p src/bin && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > src/bin/solidb-dump.rs && \
    echo "fn main() {}" > src/bin/solidb-restore.rs

# Build dependencies only (cached layer)
RUN cargo build --release && rm -rf src

# Copy actual source code
COPY src ./src

# Touch main.rs to ensure rebuild
RUN touch src/main.rs

# Build the actual binary
RUN cargo build --release --bin solidb --bin solidb-dump --bin solidb-restore

# Strip debug symbols for smaller binary
RUN strip /app/target/release/solidb \
    /app/target/release/solidb-dump \
    /app/target/release/solidb-restore

# ------------------------------------------------------------------------------
# Stage 2: Runtime
# ------------------------------------------------------------------------------
FROM debian:bookworm-slim

# Labels
LABEL org.opencontainers.image.title="SoliDB"
LABEL org.opencontainers.image.description="A lightweight, high-performance multi-document database"
LABEL org.opencontainers.image.url="https://github.com/solisoft/solidb"
LABEL org.opencontainers.image.source="https://github.com/solisoft/solidb"
LABEL org.opencontainers.image.vendor="Solisoft"
LABEL org.opencontainers.image.licenses="MIT"

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    libzstd1 \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user for security
RUN groupadd -r solidb && useradd -r -g solidb solidb

# Create data directory
RUN mkdir -p /data && chown solidb:solidb /data

# Copy binaries from builder
COPY --from=builder /app/target/release/solidb /usr/local/bin/
COPY --from=builder /app/target/release/solidb-dump /usr/local/bin/
COPY --from=builder /app/target/release/solidb-restore /usr/local/bin/

# Set ownership
RUN chown solidb:solidb /usr/local/bin/solidb*

# Switch to non-root user
USER solidb

# Environment variables (can be overridden)
ENV SOLIDB_PORT=6745
ENV SOLIDB_DATA_DIR=/data
ENV SOLIDB_LOG_LEVEL=info
ENV RUST_LOG=solidb=info,tower_http=info

# Expose default port
EXPOSE 6745

# Data volume
VOLUME ["/data"]

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:${SOLIDB_PORT}/_api/health || exit 1

# Default command
CMD ["sh", "-c", "solidb --port ${SOLIDB_PORT} --data-dir ${SOLIDB_DATA_DIR}"]
