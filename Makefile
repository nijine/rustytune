# rust-embed bakes web/dist into the server binary, so the web build must
# run before any cargo build that should serve a real page.

.PHONY: all web build run bench dev test fmt clean

all: build

web:
	cd web && npm install && npm run build

build: web
	cargo build --release

run: web
	cargo run -p rustytune-server

# Hardware-free test bench: fake Speeduino on a pty + the server against it.
bench:
	tools/bench.sh

# Frontend dev loop: Vite on :5173 with HMR, proxying /api to the server.
# Run `cargo run -p rustytune-server -- --no-open` in another terminal.
dev:
	cd web && npm run dev

test:
	cargo fmt --all --check
	cargo clippy --all-targets -- -D warnings
	cargo test

fmt:
	cargo fmt --all

clean:
	cargo clean
	rm -rf web/node_modules web/dist/*
