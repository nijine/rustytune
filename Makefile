# rust-embed bakes web/dist into the server binary, so the web build must
# run before any cargo build that should serve a real page.

.PHONY: all web build run bench dev test fmt clean release oled oled-check appliance-check

all: build

web:
	cd web && npm install && VITE_BUILD_SHA=$$(git rev-parse --short HEAD 2>/dev/null || echo unknown) npm run build

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

# Local single-binary release tarball for this machine's OS/arch.
release: build
	mkdir -p dist
	tar czf dist/rustytune-$$(uname -s | tr 'A-Z' 'a-z')-$$(uname -m).tar.gz \
		-C target/release rustytune
	@echo "dist/rustytune-$$(uname -s | tr 'A-Z' 'a-z')-$$(uname -m).tar.gz"

test:
	cargo fmt --all --check
	cargo clippy --all-targets -- -D warnings
	cargo test

fmt:
	cargo fmt --all

# Native Raspberry Pi OLED configurator. It remains a separate C build and is
# intentionally not a Cargo workspace member.
oled:
	$(MAKE) -C appliance/oled-configurator

oled-check:
	$(MAKE) -C appliance/oled-configurator check

appliance-check: oled-check
	cargo test -p rustytune-server

clean:
	cargo clean
	rm -rf web/node_modules web/dist/*
	$(MAKE) -C appliance/oled-configurator clean
