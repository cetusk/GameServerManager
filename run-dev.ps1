param(
    [string]$DataDir = (Join-Path $PSScriptRoot '.dev-data\default'),
    [switch]$SmokeTest
)
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    $gameManagerDataDir = [System.IO.Path]::GetFullPath($DataDir)
    $gameManagerArgs = @('run', '--locked', '-p', 'manager-gui', '--', '--data-dir', $gameManagerDataDir, '--backend', 'mock')
    if ($SmokeTest) { $gameManagerArgs += '--smoke-test' }
    & cargo @gameManagerArgs
    if ($LASTEXITCODE -ne 0) { throw "Development preview exited with code $LASTEXITCODE" }
} finally {
    Pop-Location
}
