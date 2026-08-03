# syntax=docker/dockerfile:1

FROM node:22-bookworm AS web
ARG BUILD_SHA=unknown
ENV VITE_BUILD_SHA=$BUILD_SHA
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.88-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY fixtures/ fixtures/
COPY --from=web /src/web/dist web/dist
RUN cargo build --locked --release --package rustytune-server --bin rustytune

# A scratch stage lets `docker build --output` export only the executable.
FROM scratch AS artifact
COPY --from=builder /src/target/release/rustytune /rustytune
