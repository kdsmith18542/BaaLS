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
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* && \
    groupadd -r baals --gid=1000 && \
    useradd -r -g baals --uid=1000 --home=/data --no-create-home baals
COPY --from=builder /app/target/release/baalsd /usr/local/bin/baalsd
RUN mkdir -p /data && chown -R baals:baals /data && chmod 0700 /data
VOLUME /data
EXPOSE 8080
USER baals
ENTRYPOINT ["/usr/local/bin/baalsd"]
CMD ["node", "start", "--data-dir", "/data"]
