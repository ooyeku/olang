#!/bin/bash

# Olang Setup Script
# Cross-platform installation script for Olang and OTC

set -e  # Exit on any error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored output
print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to detect OS
detect_os() {
    case "$(uname -s)" in
        Linux*)     echo "linux";;
        Darwin*)    echo "macos";;
        CYGWIN*|MINGW*|MSYS*) echo "windows";;
        *)          echo "unknown";;
    esac
}

# Function to get home directory
get_home_dir() {
    if [ "$OS" = "windows" ]; then
        echo "$USERPROFILE"
    else
        echo "$HOME"
    fi
}

# Function to check if command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Function to get cargo bin directory
get_cargo_bin() {
    if [ -n "$CARGO_HOME" ]; then
        echo "$CARGO_HOME/bin"
    else
        echo "$(get_home_dir)/.cargo/bin"
    fi
}

# Main setup function
main() {
    print_status "Starting Olang setup..."
    
    # Detect OS
    OS=$(detect_os)
    print_status "Detected OS: $OS"
    
    # Get home directory
    HOME_DIR=$(get_home_dir)
    print_status "Home directory: $HOME_DIR"
    
    # Check if Rust and Cargo are installed
    if ! command_exists cargo; then
        print_error "Cargo is not installed. Please install Rust first: https://rustup.rs/"
        exit 1
    fi
    
    print_success "Cargo found: $(cargo --version)"
    
    # Get cargo bin directory
    CARGO_BIN=$(get_cargo_bin)
    print_status "Cargo bin directory: $CARGO_BIN"
    
    # The ~/.olang layout is owned by the runtime (src/home.rs) and by
    # `otc update`; setup creates nothing there. Subsystems create their
    # own directories on first use, and `otc doctor` audits the result.

    # Install olang and otc using cargo install
    print_status "Installing olang..."
    if cargo install --path . --bin olang; then
        print_success "olang installed successfully"
    else
        print_error "Failed to install olang"
        exit 1
    fi
    
    print_status "Installing otc..."
    if cargo install --path ./otc --bin otc; then
        print_success "otc installed successfully"
    else
        print_error "Failed to install otc"
        exit 1
    fi
    
    # Create unified executables
    print_status "Creating unified executables..."
    
    # Create olang executable
    OLANG_EXEC="$OLANG_DIR/olang"
    if [ "$OS" = "windows" ]; then
        OLANG_EXEC="$OLANG_DIR/olang.exe"
    fi
    
    cat > "$OLANG_EXEC" << 'EOF'
#!/bin/bash
# Unified olang executable
CARGO_BIN="$(dirname "$0")/../.cargo/bin"
if [ -n "$CARGO_HOME" ]; then
    CARGO_BIN="$CARGO_HOME/bin"
fi

if [ -f "$CARGO_BIN/olang" ]; then
    exec "$CARGO_BIN/olang" "$@"
elif [ -f "$CARGO_BIN/olang.exe" ]; then
    exec "$CARGO_BIN/olang.exe" "$@"
else
    echo "Error: olang executable not found in $CARGO_BIN"
    exit 1
fi
EOF
    
    # Create otc executable
    OTC_EXEC="$OLANG_DIR/otc"
    if [ "$OS" = "windows" ]; then
        OTC_EXEC="$OLANG_DIR/otc.exe"
    fi
    
    cat > "$OTC_EXEC" << 'EOF'
#!/bin/bash
# Unified otc executable
CARGO_BIN="$(dirname "$0")/../.cargo/bin"
if [ -n "$CARGO_HOME" ]; then
    CARGO_BIN="$CARGO_HOME/bin"
fi

if [ -f "$CARGO_BIN/otc" ]; then
    exec "$CARGO_BIN/otc" "$@"
elif [ -f "$CARGO_BIN/otc.exe" ]; then
    exec "$CARGO_BIN/otc.exe" "$@"
else
    echo "Error: otc executable not found in $CARGO_BIN"
    exit 1
fi
EOF
    
    # Make executables executable (Unix-like systems only)
    if [ "$OS" != "windows" ]; then
        chmod +x "$OLANG_EXEC"
        chmod +x "$OTC_EXEC"
    fi
    
    print_success "Created unified executables:"
    print_success "  - $OLANG_EXEC"
    print_success "  - $OTC_EXEC"
    
    # Add to PATH if not already there
    print_status "Checking PATH configuration..."
    
    # Detect shell
    SHELL_NAME=$(basename "$SHELL")
    RC_FILE=""
    
    case "$SHELL_NAME" in
        bash)
            RC_FILE="$HOME_DIR/.bashrc"
            ;;
        zsh)
            RC_FILE="$HOME_DIR/.zshrc"
            ;;
        fish)
            RC_FILE="$HOME_DIR/.config/fish/config.fish"
            ;;
        *)
            RC_FILE="$HOME_DIR/.profile"
            ;;
    esac
    
    if [ -n "$RC_FILE" ] && [ -f "$RC_FILE" ]; then
        if ! grep -q "$OLANG_DIR" "$RC_FILE"; then
            print_status "Adding $OLANG_DIR to PATH in $RC_FILE"
            echo "" >> "$RC_FILE"
            echo "# Olang PATH configuration" >> "$RC_FILE"
            echo "export PATH=\"$OLANG_DIR:\$PATH\"" >> "$RC_FILE"
            print_success "Added PATH configuration to $RC_FILE"
            print_warning "Please restart your terminal or run 'source $RC_FILE' to use olang and otc commands"
        else
            print_success "PATH already configured in $RC_FILE"
        fi
    else
        print_warning "Could not detect shell configuration file. Please manually add $OLANG_DIR to your PATH"
    fi
    
    # Test the installation
    print_status "Testing installation..."
    
    # Test olang
    if "$OLANG_EXEC" --version >/dev/null 2>&1; then
        print_success "olang test passed"
    else
        print_warning "olang test failed - you may need to restart your terminal"
    fi
    
    # Test otc
    if "$OTC_EXEC" --version >/dev/null 2>&1; then
        print_success "otc test passed"
    else
        print_warning "otc test failed - you may need to restart your terminal"
    fi
    
    print_success "Setup completed successfully!"
    print_status "Directory structure created:"
    print_status "  $OLANG_DIR/"
    print_status "  ├── olang (unified executable)"
    print_status "  ├── otc (unified executable)"
    print_status "  ├── .otc/ (otc configuration directory)"
    print_status "  ├── projects/ (default projects directory)"
    print_status "  ├── templates/ (project templates)"
    print_status "  ├── cache/ (build and dependency cache)"
    print_status "  ├── logs/ (application logs)"
    print_status "  ├── config/ (user configuration files)"
    print_status "  ├── packages/ (local package storage)"
    print_status "  ├── examples/ (example projects and snippets)"
    print_status "  └── docs/ (local documentation)"
    
    print_status "Next steps:"
    print_status "1. Restart your terminal or run 'source $RC_FILE'"
    print_status "2. Try running 'olang --version' and 'otc --version'"
    print_status "3. Start the REPL with 'olang'"
}

# Run main function
main "$@" 