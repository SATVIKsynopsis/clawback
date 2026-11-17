# Build from the repo root:  docker build -f app/Dockerfile -t clawback .
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY sla-miner ./sla-miner
COPY app ./app
RUN cargo build --release -p clawback && strip target/release/clawback && mkdir -p /out/data/claims

FROM gcr.io/distroless/cc-debian12:nonroot
WORKDIR /app
COPY --from=build /src/target/release/clawback /usr/local/bin/clawback
# writable data dir for the nonroot user (named volumes inherit this ownership)
COPY --from=build --chown=65532:65532 /out/data /app/data
COPY app/clawback.toml /app/clawback.toml
ENV RUST_LOG=clawback=info,purchase=info
ENTRYPOINT ["/usr/local/bin/clawback"]
CMD ["run", "--config", "/app/clawback.toml"]
