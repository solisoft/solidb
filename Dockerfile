# ==============================================================================
# SoliDB Docker Image
# Uses pre-built binaries from the release
# ==============================================================================

FROM debian:bookworm-slim

# Labels
LABEL org.opencontainers.image.title="SoliDB"
LABEL org.opencontainers.image.description="A lightweight, high-performance multi-document database"
LABEL org.opencontainers.image.url="https://github.com/solisoft/solidb"
LABEL org.opencontainers.image.source="https://github.com/solisoft/solidb"
LABEL org.opencontainers.image.vendor="Solisoft"
LABEL org.opencontainers.image.licenses="MIT"

# Install runtime dependencies
# libssl3 is no longer needed: TLS is rustls, statically linked into the binary.
# ca-certificates still is — rustls reads the system trust store.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libzstd1 \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user for security
RUN groupadd -r solidb && useradd -r -g solidb solidb

# Create data directory
RUN mkdir -p /data && chown solidb:solidb /data

# Copy pre-built binaries
COPY binaries/solidb /usr/local/bin/
COPY binaries/solidb-dump /usr/local/bin/
COPY binaries/solidb-restore /usr/local/bin/

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
