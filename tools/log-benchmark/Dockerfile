# Build an isolated benchmark without installing or starting the appliance.
FROM rust:1.88-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY tools/log-benchmark/ tools/log-benchmark/
RUN cargo build --locked --release --manifest-path tools/log-benchmark/Cargo.toml

FROM scratch AS artifact
COPY --from=builder /src/tools/log-benchmark/target/release/rustytune-log-benchmark /rustytune-log-benchmark
