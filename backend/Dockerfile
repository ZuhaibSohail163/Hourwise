# ---- build ----
FROM rust:1-bookworm AS builder
WORKDIR /app

# Cache dependencies: build a stub binary with only the manifest first.
COPY Cargo.toml Cargo.lock* build.rs ./
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src

COPY src ./src
COPY migrations ./migrations
RUN touch src/main.rs && cargo build --release

# ---- runtime ----
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 hourwise \
    && mkdir -p /data/uploads /data/db && chown -R hourwise /data

COPY --from=builder /app/target/release/hourwise-backend /usr/local/bin/hourwise-backend

USER hourwise
ENV BIND_ADDR=0.0.0.0:8080 \
    UPLOAD_DIR=/data/uploads \
    RUST_LOG=hourwise_backend=info,tower_http=info
EXPOSE 8080
CMD ["hourwise-backend"]
