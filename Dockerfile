# Stage 1: Build
FROM rust:1.70-slim AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
COPY tests/ ./tests/
COPY sdk/ ./sdk/
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/baalsd /usr/local/bin/baalsd
RUN mkdir -p /data
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/baalsd"]
CMD ["node", "start", "--data-dir", "/data"]
