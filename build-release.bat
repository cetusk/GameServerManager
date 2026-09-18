@echo off
setlocal EnableExtensions DisableDelayedExpansion
rem Run from the repository directory, including when launched elsewhere.
pushd "%~dp0"
if errorlevel 1 (
    echo ERROR: Cannot open the repository directory.
    exit /b 1
)

where cargo.exe >nul 2>&1
if errorlevel 1 (
    echo ERROR: cargo.exe was not found. Install Rust and reopen the terminal.
    popd
    exit /b 1
)

echo Building GameServerManager and its shutdown helper for Windows x64...
cargo.exe build --release --locked -p manager-gui -p gsm-ctrlc-helper --all-features --target x86_64-pc-windows-msvc --target-dir target
set "gsmBuildExit=%ERRORLEVEL%"
if not "%gsmBuildExit%"=="0" (
    echo ERROR: Release build failed. See the compiler output above.
    popd
    exit /b %gsmBuildExit%
)

echo.
echo Release build completed:
echo   "%~dp0target\x86_64-pc-windows-msvc\release\manager-gui.exe"
echo   "%~dp0target\x86_64-pc-windows-msvc\release\gsm-ctrlc-helper.exe"
echo Keep both executable files in the same folder.
popd
exit /b 0
