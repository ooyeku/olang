# Olang Setup Script for Windows PowerShell
# Cross-platform installation script for Olang and OTC

param(
    [switch]$Force,
    [switch]$Verbose
)

# Set error action preference
$ErrorActionPreference = "Stop"

# Function to write colored output
function Write-Status {
    param([string]$Message)
    Write-Host "[INFO] $Message" -ForegroundColor Blue
}

function Write-Success {
    param([string]$Message)
    Write-Host "[SUCCESS] $Message" -ForegroundColor Green
}

function Write-Warning {
    param([string]$Message)
    Write-Host "[WARNING] $Message" -ForegroundColor Yellow
}

function Write-Error {
    param([string]$Message)
    Write-Host "[ERROR] $Message" -ForegroundColor Red
}

# Function to check if command exists
function Test-Command {
    param([string]$Command)
    try {
        Get-Command $Command -ErrorAction Stop | Out-Null
        return $true
    }
    catch {
        return $false
    }
}

Write-Status "Starting Olang setup..."

# Check if Rust and Cargo are installed
if (-not (Test-Command "cargo")) {
    Write-Error "Cargo is not installed. Please install Rust first: https://rustup.rs/"
    exit 1
}

$cargoVersion = cargo --version
Write-Success "Cargo found: $cargoVersion"

# Get cargo bin directory
if ($env:CARGO_HOME) {
    $cargoBin = Join-Path $env:CARGO_HOME "bin"
} else {
    $cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
}
Write-Status "Cargo bin directory: $cargoBin"

# Create .olang directory structure
$olangDir = Join-Path $env:USERPROFILE ".olang"
$otcDir = Join-Path $olangDir ".otc"
$projectsDir = Join-Path $olangDir "projects"
$templatesDir = Join-Path $olangDir "templates"
$cacheDir = Join-Path $olangDir "cache"
$logsDir = Join-Path $olangDir "logs"
$configDir = Join-Path $olangDir "config"
$packagesDir = Join-Path $olangDir "packages"
$examplesDir = Join-Path $olangDir "examples"
$docsDir = Join-Path $olangDir "docs"

Write-Status "Creating directory structure..."
$directories = @($olangDir, $otcDir, $projectsDir, $templatesDir, $cacheDir, $logsDir, $configDir, $packagesDir, $examplesDir, $docsDir)
foreach ($dir in $directories) {
    if (-not (Test-Path $dir)) {
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
    }
}
Write-Success "Created comprehensive .olang directory structure"

# Copy example files to user's .olang\examples directory
Write-Status "Copying example files..."
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$sourceExamples = Join-Path $scriptDir "examples"

if (Test-Path $sourceExamples) {
    # Copy all .ol files
    Get-ChildItem -Path $sourceExamples -Filter "*.ol" | ForEach-Object {
        Copy-Item $_.FullName -Destination $examplesDir -Force
    }
    
    # Copy utils directory if it exists
    $utilsDir = Join-Path $sourceExamples "utils"
    if (Test-Path $utilsDir) {
        Copy-Item -Path $utilsDir -Destination $examplesDir -Recurse -Force
    }
    Write-Success "Copied example files to $examplesDir"
} else {
    Write-Warning "Source examples directory not found, skipping example copy"
}

# Install olang and otc using cargo install
Write-Status "Installing olang..."
try {
    cargo install --path . --bin olang
    Write-Success "olang installed successfully"
} catch {
    Write-Error "Failed to install olang: $_"
    exit 1
}

Write-Status "Installing otc..."
try {
    cargo install --path .\otc --bin otc
    Write-Success "otc installed successfully"
} catch {
    Write-Error "Failed to install otc: $_"
    exit 1
}

# Create unified executables
Write-Status "Creating unified executables..."

# Create olang executable
$olangExec = Join-Path $olangDir "olang.ps1"
$olangScript = @"
# Unified olang executable
`$cargoBin = if (`$env:CARGO_HOME) { Join-Path `$env:CARGO_HOME "bin" } else { Join-Path `$env:USERPROFILE ".cargo\bin" }
`$olangPath = Join-Path `$cargoBin "olang.exe"

if (Test-Path `$olangPath) {
    & `$olangPath @args
} else {
    Write-Error "olang executable not found in `$cargoBin"
    exit 1
}
"@

$olangScript | Out-File -FilePath $olangExec -Encoding UTF8

# Create otc executable
$otcExec = Join-Path $olangDir "otc.ps1"
$otcScript = @"
# Unified otc executable
`$cargoBin = if (`$env:CARGO_HOME) { Join-Path `$env:CARGO_HOME "bin" } else { Join-Path `$env:USERPROFILE ".cargo\bin" }
`$otcPath = Join-Path `$cargoBin "otc.exe"

if (Test-Path `$otcPath) {
    & `$otcPath @args
} else {
    Write-Error "otc executable not found in `$cargoBin"
    exit 1
}
"@

$otcScript | Out-File -FilePath $otcExec -Encoding UTF8

Write-Success "Created unified executables:"
Write-Success "  - $olangExec"
Write-Success "  - $otcExec"

# Add to PATH if not already there
Write-Status "Checking PATH configuration..."

$currentPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($currentPath -notlike "*$olangDir*") {
    Write-Status "Adding $olangDir to PATH"
    $newPath = "$currentPath;$olangDir"
    [Environment]::SetEnvironmentVariable("PATH", $newPath, "User")
    Write-Success "Added PATH configuration"
    Write-Warning "Please restart your terminal to use olang and otc commands"
} else {
    Write-Success "PATH already configured"
}

# Test the installation
Write-Status "Testing installation..."

# Test olang
try {
    & $olangExec --version | Out-Null
    Write-Success "olang test passed"
} catch {
    Write-Warning "olang test failed - you may need to restart your terminal"
}

# Test otc
try {
    & $otcExec --version | Out-Null
    Write-Success "otc test passed"
} catch {
    Write-Warning "otc test failed - you may need to restart your terminal"
}

Write-Success "Setup completed successfully!"
Write-Status "Directory structure created:"
Write-Status "  $olangDir\"
Write-Status "  ├── olang.ps1 (unified executable)"
Write-Status "  ├── otc.ps1 (unified executable)"
Write-Status "  ├── .otc\ (otc configuration directory)"
Write-Status "  ├── projects\ (default projects directory)"
Write-Status "  ├── templates\ (project templates)"
Write-Status "  ├── cache\ (build and dependency cache)"
Write-Status "  ├── logs\ (application logs)"
Write-Status "  ├── config\ (user configuration files)"
Write-Status "  ├── packages\ (local package storage)"
Write-Status "  ├── examples\ (example projects and snippets)"
Write-Status "  └── docs\ (local documentation)"

Write-Status "Next steps:"
Write-Status "1. Restart your terminal"
Write-Status "2. Try running 'olang --version' and 'otc --version'"
Write-Status "3. Start the REPL with 'olang'"

if (-not $Force) {
    Read-Host "Press Enter to continue"
} 