# syntax=docker/dockerfile:1
# Production image for the Hourwise API (Rust, release build).
#
#   docker build -t hourwise-backend ./backend
#
# Runtime configuration comes only from environment variables (DATABASE_URL, JWT_SECRET, ...);
# no secrets are baked into the image. Migrations are embedded and applied on startup.

# ---- build ----
FROM rust:1-bookworm AS builder
WORKDIR /app

# Cache dependencies: build a stub binary with only the manifests first, so code changes don't
# recompile every crate.
COPY Cargo.toml Cargo.lock build.rs ./
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY src ./src
COPY migrations ./migrations
RUN touch src/main.rs src/lib.rs && cargo build --release --locked

# ---- runtime ----
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin hourwise \
    && mkdir -p /data/uploads \
    && chown -R hourwise /data

COPY --from=builder /app/target/release/hourwise-backend /usr/local/bin/hourwise-backend

USER hourwise
ENV BIND_ADDR=0.0.0.0:8080 \
    UPLOAD_DIR=/data/uploads \
    RUST_LOG=hourwise_backend=info,tower_http=info
EXPOSE 8080

# Liveness probe without curl: the binary calls its own /api/health endpoint.
HEALTHCHECK --interval=15s --timeout=5s --start-period=60s --retries=3 \
    CMD ["hourwise-backend", "healthcheck"]

# SIGTERM (the default stop signal) triggers a graceful shutdown: in-flight requests finish,
# then the connection pool is closed.
CMD ["hourwise-backend"]
