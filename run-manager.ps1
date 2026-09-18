param(
    [string]$DataDir = (Join-Path $PSScriptRoot '.manager-data'),
    [switch]$SmokeTest
)
$ErrorActionPreference = 'Stop'
# Keep this script ASCII for Windows PowerShell 5.1 on any system locale.
if ($env:OS -ne 'Windows_NT') { throw 'Local management mode requires Windows.' }
Push-Location $PSScriptRoot
try {
    $gameManagerDataDir = [System.IO.Path]::GetFullPath($DataDir)
    & cargo build --locked -p manager-gui -p gsm-ctrlc-helper --all-features --target-dir (Join-Path $PSScriptRoot 'target')
    if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
    $gameManagerArgs = @('--data-dir', $gameManagerDataDir, '--backend', 'local')
    if ($SmokeTest) { $gameManagerArgs += '--smoke-test' }
    & (Join-Path $PSScriptRoot 'target\debug\manager-gui.exe') @gameManagerArgs
    if ($LASTEXITCODE -ne 0) { throw "Local management mode exited with code: $LASTEXITCODE" }
} finally {
    Pop-Location
}
