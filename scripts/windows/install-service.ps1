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
.PARAMETER Uninstall
    Remove the service instead of installing.
#>
param(
    [string]$ServiceName = 'OrkGateway',
    [string]$BinaryPath,
    [string]$WorkDir,
    [string]$EnvFile,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'

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

if (-not (Test-Path $BinaryPath)) {
    Write-Error "Binary not found: $BinaryPath"
    exit 1
}

Write-Host "Installing service '$ServiceName'..." -ForegroundColor Green
Write-Host "  Binary : $BinaryPath"
Write-Host "  WorkDir: $WorkDir"
Write-Host "  EnvFile: $EnvFile"

# Install service
nssm install $ServiceName $BinaryPath
nssm set $ServiceName AppDirectory $WorkDir
nssm set $ServiceName DisplayName "Orka AI Gateway"
nssm set $ServiceName Description "Discord/Telegram AI CLI gateway"
nssm set $ServiceName Start SERVICE_AUTO_START
nssm set $ServiceName ObjectName LocalSystem

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
if (Test-Path $EnvFile) {
    $envVars = @()
    Get-Content $EnvFile | ForEach-Object {
        $line = $_.Trim()
        if ($line -and -not $line.StartsWith('#') -and $line -match '^([^=]+)=(.*)$') {
            $envVars += "$($matches[1].Trim())=$($matches[2].Trim())"
        }
    }
    if ($envVars.Count -gt 0) {
        $envString = $envVars -join [char]0
        nssm set $ServiceName AppEnvironmentExtra $envString
        Write-Host "  Loaded $($envVars.Count) environment variables from .env" -ForegroundColor Cyan
    }
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
