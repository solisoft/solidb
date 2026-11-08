# ==============================================================================
# SoliDB Docker Image
# Uses pre-built binaries from the release
# ==============================================================================

# The base must not be older than the host the release binary is built on:
# `build-binaries` builds solidb-linux-amd64 on ubuntu-latest (glibc 2.39), and
# on bookworm (glibc 2.36) it exited at once with "GLIBC_2.38 not found".
# Every image from v2.5.0 to v2.7.2 was published that way; the docker job now
# runs the image before pushing it.
FROM debian:trixie-slim

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
# The server binds loopback unless told otherwise (`bind_host`), which inside
# a container leaves a published port answering nothing. The container's own
# network is the boundary here, as for the k8s manifests (k8s/configmap.yaml).
ENV SOLIDB_HOST=0.0.0.0
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
# `exec` makes solidb PID 1, so `docker stop`'s SIGTERM reaches it and it
# shuts down cleanly; under a plain `sh -c` the shell received the signal,
# ignored it, and the container was killed after the grace period.
CMD ["sh", "-c", "exec solidb --port ${SOLIDB_PORT} --data-dir ${SOLIDB_DATA_DIR}"]
