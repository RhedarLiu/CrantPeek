# Run in PowerShell on Windows. Keep the executable, dictionary and licenses together.
param([switch]$SkipBuild, [switch]$Installer, [string]$SignThumbprint = $env:WINDOWS_SIGN_THUMBPRINT)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
function Sign-Artifact([string]$path) {
    if (-not $SignThumbprint) { return }
    $signTool = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if (-not $signTool) {
        $signTool = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin/*/x64/signtool.exe" -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | Select-Object -First 1
    }
    if (-not $signTool) { throw 'signtool is required when WINDOWS_SIGN_THUMBPRINT is set' }
    $toolPath = if ($signTool.Source) { $signTool.Source } else { $signTool.FullName }
    & $toolPath sign /sha1 $SignThumbprint /fd SHA256 /tr 'http://timestamp.digicert.com' /td SHA256 $path
    if ($LASTEXITCODE -ne 0) { throw 'Code signing failed' }
    & $toolPath verify /pa $path
    if ($LASTEXITCODE -ne 0) { throw 'Signature verification failed' }
}
try {
    if (-not $SkipBuild) {
        $previousRustFlags = $env:RUSTFLAGS
        try {
            $env:RUSTFLAGS = "$previousRustFlags -C target-feature=+crt-static"
            cargo build --release -p peek-gpui
        } finally { $env:RUSTFLAGS = $previousRustFlags }
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    }
    $executable = Join-Path $root 'target/release/peek-gpui.exe'
    if (-not (Test-Path $executable)) { throw 'Release executable is missing' }
    $destination = Join-Path $root 'dist/Crant-Peek-Windows'
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    Copy-Item $executable (Join-Path $destination 'CrantPeek.exe') -Force
    Sign-Artifact (Join-Path $destination 'CrantPeek.exe')
    Copy-Item (Join-Path $root 'LICENSE') $destination -Force
    $dictionary = Join-Path $root 'local-assets/ecdict.pkd'
    if (Test-Path $dictionary) {
        Copy-Item $dictionary $destination -Force
        Copy-Item (Join-Path $root 'assets/ECDICT-LICENSE') $destination -Force
    } else {
        Write-Warning 'Dictionary is not built. Put ecdict.pkd beside CrantPeek.exe to enable offline lookup.'
    }
    Compress-Archive -Path "$destination/*" -DestinationPath (Join-Path $root 'dist/Crant-Peek-Windows.zip') -Force
    if ($Installer) {
        $compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
        $compilerPath = if ($compiler) { $compiler.Source } else { "${env:ProgramFiles(x86)}/Inno Setup 6/ISCC.exe" }
        if (-not (Test-Path $compilerPath)) { throw 'Install Inno Setup 6 to build the installer' }
        $version = [regex]::Match((Get-Content (Join-Path $root 'Cargo.toml') -Raw), 'version\s*=\s*"([^"]+)"').Groups[1].Value
        & $compilerPath "/DAppVersion=$version" (Join-Path $root 'tools/windows-installer.iss')
        if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
        Sign-Artifact (Join-Path $root 'dist/Crant-Peek-Windows-Setup.exe')
    }
    Write-Host 'Built Windows package(s).'
    if (-not $SignThumbprint) { Write-Warning 'Unsigned test build; no release certificate configured.' }
} finally {
    Pop-Location
}
