.PHONY: build release test clean dashboard

# Development build
build:
	cargo build

# Release build (optimized)
release:
	cargo build --release
	@echo "\n✅ Binary: target/release/rustclaw"
	@ls -lh target/release/rustclaw

# Run tests
test:
	cargo test

# Run clippy linter
lint:
	cargo clippy -- -W clippy::all

# Start dashboard (dev mode)
dashboard:
	cargo run -- dashboard --host 127.0.0.1 --port 8080

# Start gateway API
gateway:
	cargo run -- gateway --host 0.0.0.0 --port 3000

# Interactive chat
chat:
	cargo run

# Clean build artifacts
clean:
	cargo clean

# Build for all macOS targets (run on macOS only)
release-macos:
	rustup target add x86_64-apple-darwin aarch64-apple-darwin
	cargo build --release --target x86_64-apple-darwin
	cargo build --release --target aarch64-apple-darwin
	@echo "\n✅ Intel: target/x86_64-apple-darwin/release/rustclaw"
	@echo "✅ ARM64: target/aarch64-apple-darwin/release/rustclaw"
