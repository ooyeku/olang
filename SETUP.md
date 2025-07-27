# Olang Setup Scripts

This directory contains cross-platform setup scripts for installing Olang and OTC (Olang Tool Chain) on your system.

## Quick Start

### Unix-like Systems (Linux, macOS, WSL)
```bash
./setup
```

### Windows
```cmd
setup.bat
```

### Windows PowerShell
```powershell
.\setup.ps1
```

## What the Setup Scripts Do

The setup scripts perform the following operations:

1. **Check Prerequisites**: Verify that Rust and Cargo are installed
2. **Install Binaries**: Use `cargo install` to install `olang` and `otc` binaries
3. **Create Directory Structure**: Set up comprehensive `~/.olang/` directory structure
4. **Copy Examples**: Copy all example `.ol` files to `~/.olang/examples/`
5. **Create Unified Executables**: Create platform-specific wrapper scripts in `~/.olang/`
6. **Configure PATH**: Add `~/.olang/` to your system PATH
7. **Test Installation**: Verify that both `olang` and `otc` work correctly

## Directory Structure Created

After running the setup script, you'll have:

```
~/.olang/
├── olang          # Unified olang executable (Unix)
├── olang.bat      # Unified olang executable (Windows CMD)
├── olang.ps1      # Unified olang executable (PowerShell)
├── otc            # Unified otc executable (Unix)
├── otc.bat        # Unified otc executable (Windows CMD)
├── otc.ps1        # Unified otc executable (PowerShell)
├── .otc/          # OTC configuration directory
├── projects/      # Default projects directory
├── templates/     # Project templates
├── cache/         # Build and dependency cache
├── logs/          # Application logs
├── config/        # User configuration files
├── packages/      # Local package storage
├── examples/      # Example projects and snippets
└── docs/          # Local documentation
```

### Directory Purposes

#### **Executables**
- **`olang`/`olang.bat`/`olang.ps1`**: Unified executables that point to the cargo-installed binaries
- **`otc`/`otc.bat`/`otc.ps1`**: Unified executables for the Olang Tool Chain

#### **`.otc/`** - OTC Configuration
- Tool chain configuration files
- Project templates and scaffolding
- Build system configurations

#### **`projects/`** - Default Projects Directory
- Default location for new Olang projects
- Can be used as a workspace for multiple projects
- Organized by project name or category

#### **`templates/`** - Project Templates
- Pre-built project templates for common use cases
- Web applications, CLI tools, libraries, etc.
- Customizable starting points for new projects

#### **`cache/`** - Build and Dependency Cache
- Compiled bytecode cache for faster startup
- Dependency resolution cache
- OVM optimization cache
- Temporary build artifacts

#### **`logs/`** - Application Logs
- Runtime logs from olang and otc
- Error logs and debugging information
- Performance metrics and profiling data
- REPL session logs

#### **`config/`** - User Configuration Files
- User preferences and settings
- Editor configurations (VS Code, Vim, etc.)
- Linting and formatting rules
- Custom keybindings and shortcuts

#### **`packages/`** - Local Package Storage
- Locally installed packages and modules
- Package registry cache
- Version management for local packages
- Package development workspace

#### **`examples/`** - Example Projects and Snippets
- Code examples and tutorials (copied from repository during installation)
- Learning materials and exercises
- Reference implementations
- Quick-start code snippets
- Includes all `.ol` files from the repository's examples directory
- Contains `utils/` subdirectory with utility modules

#### **`docs/`** - Local Documentation
- Generated documentation for local projects
- API reference documentation
- User guides and tutorials
- Development notes and research

## Platform-Specific Scripts

### `setup` (Unified Script)
- **Purpose**: Automatically detects your platform and runs the appropriate setup script
- **Usage**: `./setup`
- **Supported**: Linux, macOS, Windows (WSL/Git Bash)

### `setup.sh` (Unix Shell Script)
- **Purpose**: Setup script for Unix-like systems
- **Usage**: `./setup.sh`
- **Supported**: Linux, macOS, BSD, WSL
- **Features**: 
  - Colored output
  - Shell detection (bash, zsh, fish)
  - Automatic PATH configuration
  - Error handling

### `setup.bat` (Windows Batch Script)
- **Purpose**: Setup script for Windows Command Prompt
- **Usage**: `setup.bat`
- **Supported**: Windows
- **Features**:
  - Automatic PATH configuration using `setx`
  - Error handling
  - Installation testing

### `setup.ps1` (PowerShell Script)
- **Purpose**: Setup script for Windows PowerShell
- **Usage**: `.\setup.ps1`
- **Supported**: Windows PowerShell 5.1+
- **Features**:
  - Better error handling than batch script
  - Colored output
  - Parameter support (`-Force`, `-Verbose`)
  - Modern PowerShell features

## Prerequisites

Before running any setup script, ensure you have:

1. **Rust and Cargo**: Install from [https://rustup.rs/](https://rustup.rs/)
2. **Git**: For cloning the repository
3. **Build Tools**: 
   - **Linux**: `build-essential` or equivalent
   - **macOS**: Xcode Command Line Tools
   - **Windows**: Visual Studio Build Tools or Visual Studio

## Installation Steps

1. **Clone the Repository**:
   ```bash
   git clone https://github.com/ooyeku/olang.git
   cd olang
   ```

2. **Run Setup Script**:
   ```bash
   # Unix-like systems
   ./setup
   
   # Windows Command Prompt
   setup.bat
   
   # Windows PowerShell
   .\setup.ps1
   ```

3. **Restart Your Terminal**: To ensure PATH changes take effect

4. **Verify Installation**:
   ```bash
   olang --version
   otc --version
   ```

5. **Explore Examples**:
   ```bash
   # List available examples
   ls ~/.olang/examples/
   
   # Run an example
   olang ~/.olang/examples/string_interpolation.ol
   
   # Explore utility modules
   ls ~/.olang/examples/utils/
   ```

## Troubleshooting

### Common Issues

**"Cargo is not installed"**
- Install Rust from [https://rustup.rs/](https://rustup.rs/)

**"Permission denied" on Unix systems**
- Make scripts executable: `chmod +x setup setup.sh`

**"olang/otc command not found" after setup**
- Restart your terminal or run `source ~/.bashrc` (or equivalent)
- Check that `~/.olang` is in your PATH: `echo $PATH | grep olang`

**Build errors during installation**
- Ensure you have the latest Rust toolchain: `rustup update`
- Install required system dependencies for your platform

### Platform-Specific Issues

**Windows**:
- Run Command Prompt or PowerShell as Administrator if PATH modification fails
- Ensure Visual Studio Build Tools are installed
- Use PowerShell for better error messages

**macOS**:
- Install Xcode Command Line Tools: `xcode-select --install`
- If using Homebrew, ensure it's up to date

**Linux**:
- Install build essentials: `sudo apt-get install build-essential` (Ubuntu/Debian)
- For other distributions, install equivalent packages

## Manual Installation

If the setup scripts don't work for your environment, you can install manually:

1. **Install olang**:
   ```bash
   cargo install --path . --bin olang
   ```

2. **Install otc**:
   ```bash
   cargo install --path ./otc --bin otc
   ```

3. **Create directories**:
   ```bash
   mkdir -p ~/.olang/.otc
   ```

4. **Add to PATH** (manually add `~/.olang` to your shell's PATH)

## Uninstallation

To uninstall Olang and OTC:

1. **Remove binaries**:
   ```bash
   cargo uninstall olang
   cargo uninstall otc
   ```

2. **Remove directories**:
   ```bash
   rm -rf ~/.olang
   ```

3. **Remove from PATH**: Edit your shell configuration file and remove the `~/.olang` entry

## Support

If you encounter issues with the setup scripts:

1. Check the troubleshooting section above
2. Ensure you have the latest version of the repository
3. Try running the platform-specific script directly
4. Check the script output for detailed error messages

## Contributing

To improve the setup scripts:

1. Test on your target platform
2. Ensure backward compatibility
3. Add appropriate error handling
4. Update this documentation 