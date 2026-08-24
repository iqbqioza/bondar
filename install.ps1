#Requires -Version 5.1
<#
    bondar installer for Windows

    Downloads the latest bondar release binary (bondar-windows-x86_64.exe) and
    installs it into %LOCALAPPDATA%\Programs\bondar, adding that directory to
    the user PATH when needed. No administrator rights are required.

    Usage:
      powershell -ExecutionPolicy Bypass -File install.ps1

    If bondar.exe already exists at the install target, you are asked to
    confirm the overwrite (answer 'y' to proceed).

    Internal/testing overrides (optional environment variables):
      BONDAR_VERSION       - install a specific release tag instead of the latest
      BONDAR_API_BASE      - GitHub API endpoint for resolving the latest release
      BONDAR_DOWNLOAD_BASE - base URL the release assets are downloaded from
#>

$ErrorActionPreference = 'Stop'
# PowerShell 5.1 defaults to TLS 1.0/1.1; GitHub requires TLS 1.2+
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$Repo             = 'iqbqioza/bondar'
$ApiBase          = "https://api.github.com/repos/$Repo/releases/latest"
$DownloadBase     = "https://github.com/$Repo/releases/download"
$Asset            = 'bondar-windows-x86_64.exe'
$TagOverride      = $env:BONDAR_VERSION
$ApiBaseOverride  = $env:BONDAR_API_BASE
$DownloadOverride = $env:BONDAR_DOWNLOAD_BASE

# --- architecture detection (informational only; x86_64 is the published build) ----
$ProcArch = $env:PROCESSOR_ARCHITECTURE
if ($ProcArch -and $ProcArch -notin @('AMD64', 'x86_64')) {
    Write-Host "Warning: detected $ProcArch; installing the x86_64 build (Windows on ARM runs it via emulation)." -ForegroundColor Yellow
}

# --- resolve the latest release tag -----------------------------------------

$tag = $TagOverride
if (-not $tag) {
    Write-Host 'Resolving the latest bondar release...'
    $apiUrl = if ($ApiBaseOverride) { $ApiBaseOverride } else { $ApiBase }
    $json = Invoke-RestMethod -Uri $apiUrl -Headers @{ 'User-Agent' = 'bondar-installer' }
    $tag = [string]$json.tag_name
    if (-not $tag) { throw 'Could not parse the latest release tag from the GitHub API.' }
}
Write-Host "Latest release: $tag"

# --- install directory (per-user, no admin) ----------------------------------

$localAppData = if ($env:LOCALAPPDATA) {
    $env:LOCALAPPDATA
}
elseif ($env:USERPROFILE) {
    Join-Path $env:USERPROFILE 'AppData\Local'
}
else {
    [Environment]::GetFolderPath('LocalApplicationData')
}
$InstallDir = Join-Path $localAppData 'Programs\bondar'
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$Target = Join-Path $InstallDir 'bondar.exe'

# --- overwrite confirmation ---------------------------------------------------

if (Test-Path -LiteralPath $Target) {
    if ((Get-Item -LiteralPath $Target).PSIsContainer) {
        throw "$Target is a directory; refusing to replace it."
    }
    $answer = Read-Host "$Target already exists. Overwrite? [y/N]"
    if ($answer -notmatch '^(y|yes)$') {
        Write-Host "Aborted: $Target was not overwritten." -ForegroundColor Yellow
        exit 1
    }
}

# --- download, verify and install ----------------------------------------------

$base = if ($DownloadOverride) { $DownloadOverride } else { $DownloadBase }
$url  = "$base/$tag/$Asset"
Write-Host "Downloading $url ..."

$tmpDir = Join-Path ([IO.Path]::GetTempPath()) ('bondar-install-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmpDir | Out-Null
try {
    Invoke-WebRequest -Uri $url -OutFile (Join-Path $tmpDir 'bondar.exe') -UseBasicParsing

    # Checksum verification (best effort; SHA256SUMS ships with the release)
    try {
        Invoke-WebRequest -Uri "$base/$tag/SHA256SUMS" -OutFile (Join-Path $tmpDir 'SHA256SUMS') -UseBasicParsing
        $expected = (Get-Content -LiteralPath (Join-Path $tmpDir 'SHA256SUMS') | Select-Object -First 1) -split '\s+' | Select-Object -First 1
        $actual = (Get-FileHash -LiteralPath (Join-Path $tmpDir 'bondar.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($expected -and $actual -ne $expected.ToLowerInvariant()) {
            throw "Checksum verification failed for $Asset."
        }
        Write-Host 'Checksum verified.'
    }
    catch {
        if ($_.Exception.Message -match '^Checksum') { throw }
        Write-Host 'Warning: SHA256SUMS not found; skipping checksum verification.' -ForegroundColor Yellow
    }

    Copy-Item -LiteralPath (Join-Path $tmpDir 'bondar.exe') -Destination $Target -Force

    # PowerShell on Unix does not preserve the executable bit through
    # Copy-Item; restore it when running on a non-Windows platform.
    if (-not $IsWindows -and (Get-Command chmod -ErrorAction SilentlyContinue)) {
        & chmod +x $Target
    }
}
finally {
    Remove-Item -LiteralPath $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "Installed bondar $tag to $Target"

# --- add the install directory to the user PATH (no admin required) ------------

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -split ';' -notcontains $InstallDir) {
    $newPath = if ($userPath) { $userPath.TrimEnd(';') + ';' + $InstallDir } else { $InstallDir }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host "Added $InstallDir to your user PATH. Open a new terminal before using bondar."
}
else {
    Write-Host "$InstallDir is already on your PATH."
}

# --- smoke test -----------------------------------------------------------------

& $Target --version
if ($LASTEXITCODE -eq 0) {
    Write-Host 'Done. Run ''bondar --help'' to get started.'
}
else {
    throw 'The installed binary could not be executed.'
}