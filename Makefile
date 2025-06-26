# Makefile for olang
#
# This Makefile provides a centralized build mechanism for the olang programming language
# and its toolchain (otc).

# Variables
# Install directory for the binaries. Default is ~/.local/bin
# You can override it from the command line, e.g., make install INSTALL_DIR=/usr/local/bin
INSTALL_DIR ?= $(HOME)/.local/bin
OLANG_BIN_NAME = olang
OTC_BIN_NAME = otc

# Phony targets (targets that are not files)
.PHONY: all build build-dev build-prod release run install uninstall clean help

# Default target
all: build-dev

# Build targets
build: build-prod

build-dev:
	@echo "Building for development..."
	@cargo build --workspace

build-prod:
	@echo "Building for production..."
	@cargo build --release --workspace

release: build-prod

# Run target
run:
	@echo "Running otc repl for development..."
	@cargo run -p otc -- repl

# Install target
install: build-prod
	@echo "Installing binaries to $(INSTALL_DIR)..."
	@mkdir -p $(INSTALL_DIR)
	@cp target/release/$(OLANG_BIN_NAME) $(INSTALL_DIR)
	@cp target/release/$(OTC_BIN_NAME) $(INSTALL_DIR)
	@echo "Installation complete."
	@echo "Make sure '$(INSTALL_DIR)' is in your PATH."

# Uninstall target
uninstall:
	@echo "Uninstalling binaries from $(INSTALL_DIR)..."
	@rm -f $(INSTALL_DIR)/$(OLANG_BIN_NAME)
	@rm -f $(INSTALL_DIR)/$(OTC_BIN_NAME)
	@echo "Uninstallation complete."

# Clean target
clean:
	@echo "Cleaning up build artifacts..."
	@cargo clean
	@echo "Clean complete."

# Help target
help:
	@echo "Usage: make [target]"
	@echo ""
	@echo "Targets:"
	@echo "  all          Build for development (default)."
	@echo "  build        Build for production (alias for build-prod)."
	@echo "  build-dev    Build for development."
	@echo "  build-prod   Build for production (optimized)."
	@echo "  release      Alias for build-prod."
	@echo "  run          Run otc repl for development."
	@echo "  install      Build for production and install binaries to $(INSTALL_DIR)."
	@echo "  uninstall    Uninstall binaries from $(INSTALL_DIR)."
	@echo "  clean        Clean up build artifacts."
	@echo "  help         Show this help message." 