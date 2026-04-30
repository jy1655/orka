#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Installs Orka as a Windows service using NSSM.
.DESCRIPTION
    Registers orka-app.exe as a Windows service via NSSM (Non-Sucking Service Manager).
    NSSM must be installed and available on PATH (e.g., choco install nssm).
.PARAMETER ServiceName
    Name for the Windows service. Default: OrkGateway
.PARAMETER BinaryPath
    Full path to orka-app.exe.
.PARAMETER WorkDir
    Working directory for the service. Defaults to the binary's parent directory.
.PARAMETER EnvFile
    Path to .env file. Variables are loaded into the service environment.
.PARAMETER ProfileRoot
    User profile root to expose to the service when running under LocalSystem
    (for example C:\Users\you). When omitted, the script infers it from the
    deployment path if possible.
.PARAMETER RestartDelayMs
    Delay before NSSM restarts the app after an unexpected exit. Default: 5000.
.PARAMETER Uninstall
    Remove the service instead of installing.
#>
param(
    [string]$ServiceName = 'OrkGateway',
    [string]$BinaryPath,
    [string]$WorkDir,
    [string]$EnvFile,
    [string]$ProfileRoot,
    [int]$RestartDelayMs = 5000,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'

function Get-InferredProfileRoot {
    param([string]$Path)

    if (-not $Path) {
        return $null
    }

    try {
        $fullPath = [System.IO.Path]::GetFullPath($Path)
    } catch {
        return $null
    }

    if ($fullPath -match '^[A-Za-z]:\\Users\\[^\\]+') {
        return $matches[0]
    }

    return $null
}

# Check NSSM availability
if (-not (Get-Command nssm -ErrorAction SilentlyContinue)) {
    Write-Error @"
NSSM not found. Install it first:
  choco install nssm
  -- or --
  Download from https://nssm.cc and add to PATH.
"@
    exit 1
}

if ($Uninstall) {
    Write-Host "Stopping service '$ServiceName'..." -ForegroundColor Yellow
    nssm stop $ServiceName 2>$null
    Write-Host "Removing service '$ServiceName'..." -ForegroundColor Yellow
    nssm remove $ServiceName confirm
    Write-Host "Service removed." -ForegroundColor Green
    exit 0
}

# Resolve defaults
$ProjectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)

if (-not $BinaryPath) {
    $BinaryPath = Join-Path $ProjectRoot 'target\release\orka-app.exe'
}
if (-not $WorkDir) {
    $WorkDir = Split-Path -Parent $BinaryPath
}
if (-not $EnvFile) {
    $EnvFile = Join-Path $ProjectRoot '.env'
}
if (-not $ProfileRoot) {
    $ProfileRoot = Get-InferredProfileRoot $WorkDir
    if (-not $ProfileRoot) {
        $ProfileRoot = Get-InferredProfileRoot $BinaryPath
    }
}

if (-not (Test-Path $BinaryPath)) {
    Write-Error "Binary not found: $BinaryPath"
    exit 1
}

Write-Host "Installing service '$ServiceName'..." -ForegroundColor Green
Write-Host "  Binary : $BinaryPath"
Write-Host "  WorkDir: $WorkDir"
Write-Host "  EnvFile: $EnvFile"
if ($ProfileRoot) {
    Write-Host "  Profile: $ProfileRoot"
}
Write-Host "  Restart: on exit (${RestartDelayMs}ms delay)"

# Install service
nssm install $ServiceName $BinaryPath
nssm set $ServiceName AppDirectory $WorkDir
nssm set $ServiceName DisplayName "Orka AI Gateway"
nssm set $ServiceName Description "Discord/Telegram AI CLI gateway"
nssm set $ServiceName Start SERVICE_AUTO_START
nssm set $ServiceName ObjectName LocalSystem
nssm set $ServiceName AppExit Default Restart
nssm set $ServiceName AppRestartDelay $RestartDelayMs

# Stdout/stderr logging
$LogDir = Join-Path $WorkDir 'logs'
if (-not (Test-Path $LogDir)) {
    New-Item -ItemType Directory -Path $LogDir -Force | Out-Null
}
nssm set $ServiceName AppStdout (Join-Path $LogDir 'orka-stdout.log')
nssm set $ServiceName AppStderr (Join-Path $LogDir 'orka-stderr.log')
nssm set $ServiceName AppStdoutCreationDisposition 4  # append
nssm set $ServiceName AppStderrCreationDisposition 4  # append
nssm set $ServiceName AppRotateFiles 1
nssm set $ServiceName AppRotateBytes 10485760  # 10 MB

# Load .env into service environment
$envMap = [ordered]@{}
if (Test-Path $EnvFile) {
    Get-Content $EnvFile | ForEach-Object {
        $line = $_.Trim()
        if ($line -and -not $line.StartsWith('#') -and $line -match '^([^=]+)=(.*)$') {
            $envMap[$matches[1].Trim()] = $matches[2].Trim()
        }
    }
}

if ($ProfileRoot -and (Test-Path $ProfileRoot)) {
    # LocalSystem does not have the interactive user's Codex/npm profile by
    # default. Point the service at the deployment user's profile so CLI auth
    # and config resolve the same way they do in an interactive shell.
    $envMap['USERPROFILE'] = $ProfileRoot
    $envMap['HOME'] = $ProfileRoot

    $appData = Join-Path $ProfileRoot 'AppData\Roaming'
    if (Test-Path $appData) {
        $envMap['APPDATA'] = $appData
    }

    $localAppData = Join-Path $ProfileRoot 'AppData\Local'
    if (Test-Path $localAppData) {
        $envMap['LOCALAPPDATA'] = $localAppData
    }
}

if ($envMap.Count -gt 0) {
    $envVars = @($envMap.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" })
    $envString = $envVars -join [char]0
    nssm set $ServiceName AppEnvironmentExtra $envString
    Write-Host "  Loaded $($envVars.Count) environment variables into the service" -ForegroundColor Cyan
}

Write-Host ""
Write-Host "Service installed. Commands:" -ForegroundColor Green
Write-Host "  nssm start $ServiceName    # Start"
Write-Host "  nssm stop $ServiceName     # Stop"
Write-Host "  nssm status $ServiceName   # Status"
Write-Host "  nssm edit $ServiceName     # Edit (GUI)"
Write-Host ""
Write-Host "To start now:" -ForegroundColor Yellow
Write-Host "  nssm start $ServiceName"
