# Run in PowerShell on Windows. Keep the executable, dictionary and licenses together.
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    if (-not $SkipBuild) {
        cargo build --release -p peek-gpui
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    }
    $executable = Join-Path $root 'target/release/peek-gpui.exe'
    if (-not (Test-Path $executable)) { throw 'Release executable is missing' }
    $destination = Join-Path $root 'dist/Crant-Peek-Windows'
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    Copy-Item $executable (Join-Path $destination 'CrantPeek.exe') -Force
    Copy-Item (Join-Path $root 'LICENSE') $destination -Force
    $dictionary = Join-Path $root 'local-assets/ecdict.pkd'
    if (Test-Path $dictionary) {
        Copy-Item $dictionary $destination -Force
        Copy-Item (Join-Path $root 'assets/ECDICT-LICENSE') $destination -Force
    } else {
        Write-Warning 'Dictionary is not built. Put ecdict.pkd beside CrantPeek.exe to enable offline lookup.'
    }
    Compress-Archive -Path "$destination/*" -DestinationPath (Join-Path $root 'dist/Crant-Peek-Windows.zip') -Force
    Write-Host 'Built dist/Crant-Peek-Windows.zip (portable test package; not signed).'
} finally {
    Pop-Location
}
