@echo off
setlocal enabledelayedexpansion

REM Olang Setup Script for Windows
REM Cross-platform installation script for Olang and OTC

echo [INFO] Starting Olang setup...

REM Check if Rust and Cargo are installed
where cargo >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERROR] Cargo is not installed. Please install Rust first: https://rustup.rs/
    exit /b 1
)

for /f "tokens=*" %%i in ('cargo --version') do set CARGO_VERSION=%%i
echo [SUCCESS] Cargo found: %CARGO_VERSION%

REM Get cargo bin directory
if defined CARGO_HOME (
    set CARGO_BIN=%CARGO_HOME%\bin
) else (
    set CARGO_BIN=%USERPROFILE%\.cargo\bin
)
echo [INFO] Cargo bin directory: %CARGO_BIN%

REM Create .olang directory structure
set OLANG_DIR=%USERPROFILE%\.olang
set OTC_DIR=%OLANG_DIR%\.otc
set PROJECTS_DIR=%OLANG_DIR%\projects
set TEMPLATES_DIR=%OLANG_DIR%\templates
set CACHE_DIR=%OLANG_DIR%\cache
set LOGS_DIR=%OLANG_DIR%\logs
set CONFIG_DIR=%OLANG_DIR%\config
set PACKAGES_DIR=%OLANG_DIR%\packages
set EXAMPLES_DIR=%OLANG_DIR%\examples
set DOCS_DIR=%OLANG_DIR%\docs

echo [INFO] Creating directory structure...
if not exist "%OLANG_DIR%" mkdir "%OLANG_DIR%"
if not exist "%OTC_DIR%" mkdir "%OTC_DIR%"
if not exist "%PROJECTS_DIR%" mkdir "%PROJECTS_DIR%"
if not exist "%TEMPLATES_DIR%" mkdir "%TEMPLATES_DIR%"
if not exist "%CACHE_DIR%" mkdir "%CACHE_DIR%"
if not exist "%LOGS_DIR%" mkdir "%LOGS_DIR%"
if not exist "%CONFIG_DIR%" mkdir "%CONFIG_DIR%"
if not exist "%PACKAGES_DIR%" mkdir "%PACKAGES_DIR%"
if not exist "%EXAMPLES_DIR%" mkdir "%EXAMPLES_DIR%"
if not exist "%DOCS_DIR%" mkdir "%DOCS_DIR%"
echo [SUCCESS] Created comprehensive .olang directory structure

REM Copy example files to user's .olang\examples directory
echo [INFO] Copying example files...
set SCRIPT_DIR=%~dp0
set SOURCE_EXAMPLES=%SCRIPT_DIR%examples

if exist "%SOURCE_EXAMPLES%" (
    REM Copy all .ol files
    for %%f in ("%SOURCE_EXAMPLES%\*.ol") do (
        copy "%%f" "%EXAMPLES_DIR%\" >nul 2>&1
    )
    
    REM Copy utils directory if it exists
    if exist "%SOURCE_EXAMPLES%\utils" (
        xcopy "%SOURCE_EXAMPLES%\utils" "%EXAMPLES_DIR%\utils\" /E /I /Y >nul 2>&1
    )
    echo [SUCCESS] Copied example files to %EXAMPLES_DIR%
) else (
    echo [WARNING] Source examples directory not found, skipping example copy
)

REM Install olang and otc using cargo install
echo [INFO] Installing olang...
cargo install --path . --bin olang
if %errorlevel% neq 0 (
    echo [ERROR] Failed to install olang
    exit /b 1
)
echo [SUCCESS] olang installed successfully

echo [INFO] Installing otc...
cargo install --path .\otc --bin otc
if %errorlevel% neq 0 (
    echo [ERROR] Failed to install otc
    exit /b 1
)
echo [SUCCESS] otc installed successfully

REM Create unified executables
echo [INFO] Creating unified executables...

REM Create olang executable
set OLANG_EXEC=%OLANG_DIR%\olang.bat
(
echo @echo off
echo REM Unified olang executable
echo if defined CARGO_HOME ^(
echo     set CARGO_BIN=%%CARGO_HOME%%\bin
echo ^) else ^(
echo     set CARGO_BIN=%%USERPROFILE%%\.cargo\bin
echo ^)
echo.
echo if exist "%%CARGO_BIN%%\olang.exe" ^(
echo     "%%CARGO_BIN%%\olang.exe" %%*
echo ^) else ^(
echo     echo Error: olang executable not found in %%CARGO_BIN%%
echo     exit /b 1
echo ^)
) > "%OLANG_EXEC%"

REM Create otc executable
set OTC_EXEC=%OLANG_DIR%\otc.bat
(
echo @echo off
echo REM Unified otc executable
echo if defined CARGO_HOME ^(
echo     set CARGO_BIN=%%CARGO_HOME%%\bin
echo ^) else ^(
echo     set CARGO_BIN=%%USERPROFILE%%\.cargo\bin
echo ^)
echo.
echo if exist "%%CARGO_BIN%%\otc.exe" ^(
echo     "%%CARGO_BIN%%\otc.exe" %%*
echo ^) else ^(
echo     echo Error: otc executable not found in %%CARGO_BIN%%
echo     exit /b 1
echo ^)
) > "%OTC_EXEC%"

echo [SUCCESS] Created unified executables:
echo [SUCCESS]   - %OLANG_EXEC%
echo [SUCCESS]   - %OTC_EXEC%

REM Add to PATH if not already there
echo [INFO] Checking PATH configuration...

REM Check if .olang is already in PATH
echo %PATH% | findstr /i "%OLANG_DIR%" >nul
if %errorlevel% neq 0 (
    echo [INFO] Adding %OLANG_DIR% to PATH
    setx PATH "%PATH%;%OLANG_DIR%"
    echo [SUCCESS] Added PATH configuration
    echo [WARNING] Please restart your terminal to use olang and otc commands
) else (
    echo [SUCCESS] PATH already configured
)

REM Test the installation
echo [INFO] Testing installation...

REM Test olang
"%OLANG_EXEC%" --version >nul 2>&1
if %errorlevel% equ 0 (
    echo [SUCCESS] olang test passed
) else (
    echo [WARNING] olang test failed - you may need to restart your terminal
)

REM Test otc
"%OTC_EXEC%" --version >nul 2>&1
if %errorlevel% equ 0 (
    echo [SUCCESS] otc test passed
) else (
    echo [WARNING] otc test failed - you may need to restart your terminal
)

echo [SUCCESS] Setup completed successfully!
echo [INFO] Directory structure created:
echo [INFO]   %OLANG_DIR%\
echo [INFO]   ├── olang.bat (unified executable)
echo [INFO]   ├── otc.bat (unified executable)
echo [INFO]   ├── .otc\ (otc configuration directory)
echo [INFO]   ├── projects\ (default projects directory)
echo [INFO]   ├── templates\ (project templates)
echo [INFO]   ├── cache\ (build and dependency cache)
echo [INFO]   ├── logs\ (application logs)
echo [INFO]   ├── config\ (user configuration files)
echo [INFO]   ├── packages\ (local package storage)
echo [INFO]   ├── examples\ (example projects and snippets)
echo [INFO]   └── docs\ (local documentation)

echo [INFO] Next steps:
echo [INFO] 1. Restart your terminal
echo [INFO] 2. Try running 'olang --version' and 'otc --version'
echo [INFO] 3. Start the REPL with 'olang'

pause 