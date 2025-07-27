# Simple Makefile for Olang

.PHONY: build install remove clean test

# Default target
all: build

# Build the project
build:
	cargo build --release

# Install using the setup script
install:
	@echo "Installing Olang and OTC using setup script..."
	./setup

# Remove installed binaries and directories
remove:
	@echo "Removing Olang and OTC..."
	cargo uninstall olang || true
	cargo uninstall otc || true
	rm -rf ~/.olang
	@echo "Removal complete - all .olang directories and files removed"

# Clean build artifacts
clean:
	cargo clean

# Run tests
test:
	cargo test 