param([string]$DataDir = (Join-Path $PSScriptRoot 'data'))
$ErrorActionPreference = 'Stop'
$gameManagerDataDir = [System.IO.Path]::GetFullPath($DataDir)
& (Join-Path $PSScriptRoot 'manager-gui.exe') --data-dir $gameManagerDataDir --backend local
if ($LASTEXITCODE -ne 0) { throw "GameServerManager exited with code $LASTEXITCODE" }
