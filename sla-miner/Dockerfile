# Build from the repo root (workspace):  docker build -f sla-miner/Dockerfile -t sla-miner .
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY sla-miner ./sla-miner
RUN cargo build --release -p sla-miner --bin sla-miner && strip target/release/sla-miner

# distroless cc: glibc + CA certificates, no shell; runs as nonroot
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/sla-miner /usr/local/bin/sla-miner
ENV PORT=8080 RUST_LOG=sla_miner=info
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/sla-miner"]
