# Exercise real Inno Setup installation in a disposable Windows Actions account.
param(
    [Parameter(Mandatory)][string]$Installer,
    [Parameter(Mandatory)][string]$PortableArchive,
    [Parameter(Mandatory)][string]$PreviousInstaller,
    [Parameter(Mandatory)][string]$PreviousVersion
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows -or $env:GITHUB_ACTIONS -ne 'true') {
    throw 'Run this test only in a disposable Windows GitHub Actions account.'
}
$repository = Split-Path -Parent $PSScriptRoot
$installerPath = (Resolve-Path $Installer).Path
$archivePath = (Resolve-Path $PortableArchive).Path
$previousPath = (Resolve-Path $PreviousInstaller).Path
$version = [regex]::Match((Get-Content (Join-Path $repository 'Cargo.toml') -Raw), 'version\s*=\s*"([^"]+)"').Groups[1].Value
if ([version]$PreviousVersion -ge [version]$version) { throw 'Upgrade baseline must be older than the package.' }
$logs = Join-Path $repository 'dist/installer-test-logs'
New-Item -ItemType Directory -Force -Path $logs | Out-Null
$scratch = Join-Path $env:RUNNER_TEMP ('CrantPeek-install-' + [guid]::NewGuid().ToString('N'))
$application = Join-Path $scratch 'Crant Peek'
$expected = Join-Path $scratch 'portable'
$group = 'Crant Peek Installer Test ' + [guid]::NewGuid().ToString('N')
$shortcutDirectory = Join-Path ([Environment]::GetFolderPath('Programs')) $group
$registry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{9FA46BCA-804A-4B40-80D7-DFF74C6F3C93}_is1'
# This is directories::ProjectDirs::from("dev", "Crant", "CrantPeek").config_dir().
$configuration = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'Crant\CrantPeek\config'
if ((Test-Path $registry) -or (Test-Path $configuration)) {
    throw 'Existing Crant Peek installation or profile found; refusing to alter it.'
}
$results = [System.Collections.Generic.List[object]]::new()
$profileHashes = @{}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Record-Pass([string]$Name) {
    $results.Add([ordered]@{ name = $Name; result = 'passed' })
    Write-Host "[installer-test] $Name passed"
}
function Assert-Profile {
    foreach ($name in $profileHashes.Keys) {
        $path = Join-Path $configuration $name
        Assert-True (Test-Path $path) "User profile file was deleted: $name"
        Assert-True ((Get-FileHash $path -Algorithm SHA256).Hash -eq $profileHashes[$name]) "User profile file was changed: $name"
    }
}
function Install-Package([string]$Package, [string]$Stage) {
    $arguments = @('/SP-', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART',
        "/DIR=`"$application`"", "/GROUP=`"$group`"", "/LOG=`"$(Join-Path $logs "$Stage.log")`"")
    $process = Start-Process -FilePath $Package -ArgumentList $arguments -Wait -PassThru
    Assert-True ($process.ExitCode -eq 0) "$Stage installer exit code: $($process.ExitCode)"
    Assert-Profile
}
function Assert-Registration([string]$ExpectedVersion) {
    Assert-True (Test-Path $registry) 'Per-user uninstall registration is missing.'
    $registration = Get-ItemProperty $registry
    Assert-True ($registration.DisplayVersion -eq $ExpectedVersion) 'Registered version does not match the installed package.'
    Assert-True ([IO.Path]::GetFullPath($registration.InstallLocation).TrimEnd('\') -ieq [IO.Path]::GetFullPath($application).TrimEnd('\')) 'Registered installation directory differs.'
    $shortcut = Join-Path $shortcutDirectory 'Crant Peek.lnk'
    Assert-True (Test-Path $shortcut) 'Start Menu shortcut is missing.'
    $shell = New-Object -ComObject WScript.Shell
    try {
        $link = $shell.CreateShortcut($shortcut)
        Assert-True ($link.TargetPath -ieq (Join-Path $application 'CrantPeek.exe')) 'Shortcut does not point to the installed executable.'
    } finally {
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell)
    }
}
function Assert-Payload {
    foreach ($file in Get-ChildItem $expected -File -Recurse) {
        $relative = [IO.Path]::GetRelativePath($expected, $file.FullName)
        $installed = Join-Path $application $relative
        Assert-True (Test-Path $installed) "Installed file missing: $relative"
        Assert-True ((Get-FileHash $installed -Algorithm SHA256).Hash -eq (Get-FileHash $file.FullName -Algorithm SHA256).Hash) "Installed file differs from the portable package: $relative"
    }
    foreach ($name in @('NotoSerif-OFL.txt', 'JetBrainsMono-OFL.txt', 'README.txt')) {
        $installed = Join-Path $application "licenses/$name"
        Assert-True (Test-Path $installed) "Font notice missing: $name"
        Assert-True ((Get-FileHash $installed -Algorithm SHA256).Hash -eq (Get-FileHash (Join-Path $repository "assets/licenses/$name") -Algorithm SHA256).Hash) "Font notice differs: $name"
    }
    Assert-True (Test-Path (Join-Path $application 'ecdict.pkd')) 'Offline dictionary missing.'
    Assert-True (Test-Path (Join-Path $application 'unins000.exe')) 'Uninstaller missing.'
}
function Uninstall-Package([string]$Stage) {
    $uninstaller = Join-Path $application 'unins000.exe'
    Assert-True (Test-Path $uninstaller) 'No uninstaller found.'
    $process = Start-Process -FilePath $uninstaller -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$(Join-Path $logs "$Stage.log")`"") -Wait -PassThru
    Assert-True ($process.ExitCode -eq 0) "$Stage uninstaller exit code: $($process.ExitCode)"
    for ($i = 0; $i -lt 20 -and ((Test-Path (Join-Path $application 'CrantPeek.exe')) -or (Test-Path $registry)); $i++) {
        Start-Sleep -Milliseconds 250
    }
    foreach ($file in Get-ChildItem $expected -File -Recurse) {
        $relative = [IO.Path]::GetRelativePath($expected, $file.FullName)
        Assert-True (-not (Test-Path (Join-Path $application $relative))) "Uninstall left a payload file: $relative"
    }
    Assert-True (-not (Test-Path $registry)) 'Uninstall registration was not removed.'
    Assert-True (-not (Test-Path (Join-Path $shortcutDirectory 'Crant Peek.lnk'))) 'Start Menu shortcut was not removed.'
    Assert-Profile
}

try {
    New-Item -ItemType Directory -Force -Path $scratch, $configuration | Out-Null
    Expand-Archive -Path $archivePath -DestinationPath $expected
    # Synthetic fixtures only: never inspect or upload a person's credentials.
    $fixtures = @{
        'config.json' = '{"channels":[],"installer_test":"preserve-settings"}'
        'credentials.json' = '{"installer-test":"synthetic-value"}'
    }
    foreach ($name in $fixtures.Keys) {
        $path = Join-Path $configuration $name
        Set-Content -LiteralPath $path -Value $fixtures[$name] -Encoding utf8NoBOM -NoNewline
        $profileHashes[$name] = (Get-FileHash $path -Algorithm SHA256).Hash
    }
    Install-Package $installerPath 'fresh-install'
    Assert-Registration $version
    Assert-Payload
    Record-Pass 'fresh-install-payload-registration-shortcut'

    # Run the installed program's existing no-window diagnostic, including its
    # native dependencies. GUI interactions are covered by the user's testing.
    $oldReport = $env:PEEK_MONITOR_REPORT
    $oldLog = $env:PEEK_SELECTION_LOG
    try {
        $env:PEEK_MONITOR_REPORT = '1'
        $env:PEEK_SELECTION_LOG = Join-Path $logs 'installed-startup.txt'
        $process = Start-Process -FilePath (Join-Path $application 'CrantPeek.exe') -WorkingDirectory $application -Wait -PassThru
        Assert-True ($process.ExitCode -eq 0) 'Installed executable failed to start.'
        Assert-True ((Get-Content $env:PEEK_SELECTION_LOG -Raw).Contains('cursor monitor bounds:')) 'Installed executable did not complete its diagnostic.'
    } finally {
        $env:PEEK_MONITOR_REPORT = $oldReport
        $env:PEEK_SELECTION_LOG = $oldLog
    }
    Assert-Profile
    Record-Pass 'installed-executable-startup'

    Uninstall-Package 'fresh-uninstall'
    Record-Pass 'fresh-uninstall-preserves-profile'

    Install-Package $previousPath 'baseline-install'
    Assert-Registration $PreviousVersion
    Record-Pass 'baseline-install'

    Install-Package $installerPath 'upgrade-install'
    Assert-Registration $version
    Assert-Payload
    Record-Pass 'upgrade-payload-and-profile-preservation'

    Uninstall-Package 'upgrade-uninstall'
    Record-Pass 'upgrade-uninstall-preserves-profile'
    Write-Host "[installer-test] all checks passed: $PreviousVersion -> $version"
} finally {
    [ordered]@{ version = $version; previousVersion = $PreviousVersion; checks = $results } |
        ConvertTo-Json -Depth 5 | Set-Content (Join-Path $logs 'results.json') -Encoding utf8NoBOM
    # All touched profile files were created by this test after the fresh-account
    # guard. Keep installer logs, but remove these disposable fixtures.
    if (Test-Path (Join-Path $application 'unins000.exe')) {
        try { Uninstall-Package 'cleanup-uninstall' } catch { Write-Warning 'Cleanup failed; see the retained installer logs.' }
    }
    if (Test-Path $configuration) { Remove-Item $configuration -Recurse -Force }
    if (Test-Path $shortcutDirectory) { Remove-Item $shortcutDirectory -Recurse -Force }
    if (Test-Path $scratch) { Remove-Item $scratch -Recurse -Force }
}
