$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Run this script on Windows with Visual Studio C++ Build Tools.' }
$gameManagerRoot = Split-Path $PSScriptRoot -Parent
Push-Location $gameManagerRoot
try {
    & cargo build --locked --release -p manager-gui -p gsm-ctrlc-helper --all-features --target x86_64-pc-windows-msvc --target-dir (Join-Path $gameManagerRoot 'target')
    if ($LASTEXITCODE -ne 0) { throw 'Windows build failed.' }
    $gameManagerManifest = Get-Content (Join-Path $gameManagerRoot 'Cargo.toml') -Raw
    $gameManagerVersion = [regex]::Match($gameManagerManifest, '(?m)^version = "([^"]+)"').Groups[1].Value
    if (!$gameManagerVersion) { throw 'Cannot read the workspace version.' }
    $gameManagerDist = Join-Path $gameManagerRoot 'dist\GameServerManager'
    New-Item -ItemType Directory -Force $gameManagerDist | Out-Null
    foreach ($gameManagerBinary in @('manager-gui.exe','gsm-ctrlc-helper.exe')) {
        Copy-Item (Join-Path 'target\x86_64-pc-windows-msvc\release' $gameManagerBinary) $gameManagerDist -Force
    }
    Copy-Item 'tools\start-packaged.ps1' (Join-Path $gameManagerDist 'start-manager.ps1') -Force
    foreach ($gameManagerFile in @('README.md','README.en.md','CHANGELOG.md','LICENSE','THIRD_PARTY_NOTICES.md','Cargo.lock')) {
        Copy-Item $gameManagerFile $gameManagerDist -Force
    }
    # Keep documentation links valid and copy only reviewed public documents.
    $gameManagerDocs = @('local-manager.md','native-ui.md','server-settings.md','compatibility.md','development.md')
    New-Item -ItemType Directory -Force (Join-Path $gameManagerDist 'docs') | Out-Null
    foreach ($gameManagerDoc in $gameManagerDocs) {
        Copy-Item (Join-Path 'docs' $gameManagerDoc) (Join-Path $gameManagerDist 'docs') -Force
    }
    New-Item -ItemType Directory -Force (Join-Path $gameManagerDist 'licenses') | Out-Null
    Copy-Item 'licenses\Slint-Royalty-free-2.0.md' (Join-Path $gameManagerDist 'licenses') -Force
    foreach ($gameManagerGame in @('valheim','windrose','conan','arksa')) {
        $gameManagerLicenseDir = Join-Path $gameManagerDist "crates\games\$gameManagerGame"
        New-Item -ItemType Directory -Force $gameManagerLicenseDir | Out-Null
        Copy-Item "crates\games\$gameManagerGame\LICENSE" $gameManagerLicenseDir -Force
    }
    $gameManagerIconDir = Join-Path $gameManagerDist 'apps\manager-gui\assets\game-icons'
    New-Item -ItemType Directory -Force $gameManagerIconDir | Out-Null
    Copy-Item 'apps\manager-gui\assets\game-icons\*' $gameManagerIconDir -Force
    New-Item -ItemType Directory -Force (Join-Path $gameManagerDist 'assets') | Out-Null
    Copy-Item 'assets\logo-dark-trimmed.png' (Join-Path $gameManagerDist 'assets') -Force
    # Build the ZIP from exact files, never from directories containing old user data.
    $gameManagerRelativeFiles = @('manager-gui.exe','gsm-ctrlc-helper.exe','start-manager.ps1','README.md','README.en.md','CHANGELOG.md','LICENSE','THIRD_PARTY_NOTICES.md','Cargo.lock','licenses/Slint-Royalty-free-2.0.md','assets/logo-dark-trimmed.png')
    $gameManagerRelativeFiles += $gameManagerDocs | ForEach-Object { "docs/$_" }
    $gameManagerRelativeFiles += @('valheim','windrose','conan','arksa') | ForEach-Object { "crates/games/$_/LICENSE" }
    $gameManagerRelativeFiles += @('SOURCES.md','arksa.jpg','valheim.jpg','windrose.jpg','satisfactory.jpg','conan.jpg') | ForEach-Object { "apps/manager-gui/assets/game-icons/$_" }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $gameManagerZipPath = Join-Path $gameManagerRoot "dist\GameServerManager-v$gameManagerVersion-windows-x64.zip"
    if (Test-Path $gameManagerZipPath) { Remove-Item $gameManagerZipPath }
    $gameManagerZip = [System.IO.Compression.ZipFile]::Open($gameManagerZipPath, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($gameManagerRelativeFile in $gameManagerRelativeFiles) {
            [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($gameManagerZip, (Join-Path $gameManagerDist $gameManagerRelativeFile), $gameManagerRelativeFile) | Out-Null
        }
    } finally {
        $gameManagerZip.Dispose()
    }
    Write-Host "Package: $gameManagerDist"
    Write-Host 'Start with .\start-manager.ps1'
} finally {
    Pop-Location
}
