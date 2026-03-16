#Requires -Version 5.1
<#
.SYNOPSIS
    Starts Orka gateway with environment variables loaded from .env file.
.DESCRIPTION
    Loads .env file and runs orka-app.exe. Works with both PowerShell 5.1 and 7+.
    Logs startup output to logs/orka-startup.log via Start-Transcript.
.PARAMETER EnvFile
    Path to the .env file. Defaults to .env in the same directory as this script.
.PARAMETER Binary
    Path to orka-app.exe. Defaults to orka-app.exe in the same directory as this script.
#>
param(
    [string]$EnvFile,
    [string]$Binary
)

$ErrorActionPreference = 'Stop'

# Resolve project root from the script location.
$ProjectRoot = $PSScriptRoot
$LogDir = Join-Path $ProjectRoot 'logs'
$TranscriptPath = Join-Path $LogDir 'orka-startup.log'

if (-not $EnvFile) {
    $EnvFile = Join-Path $ProjectRoot '.env'
}
if (-not $Binary) {
    $Binary = Join-Path $ProjectRoot 'orka-app.exe'
}

# Validate paths
if (-not (Test-Path $EnvFile)) {
    Write-Error "Environment file not found: $EnvFile"
    exit 1
}
if (-not (Test-Path $Binary)) {
    Write-Error "Binary not found: $Binary"
    exit 1
}

# Ensure relative file access resolves from the app directory.
$BinaryDirectory = Split-Path -Parent $Binary
Set-Location $BinaryDirectory

if (-not (Test-Path $LogDir)) {
    New-Item -ItemType Directory -Path $LogDir -Force | Out-Null
}

try {
    Start-Transcript -Path $TranscriptPath -Append | Out-Null
} catch {
    Write-Warning "Unable to start transcript logging: $($_.Exception.Message)"
}

# Load .env into current process environment
Get-Content $EnvFile | ForEach-Object {
    $line = $_.Trim()
    if ($line -and -not $line.StartsWith('#') -and $line -match '^([^=]+)=(.*)$') {
        $key = $matches[1].Trim()
        $value = $matches[2].Trim()
        [Environment]::SetEnvironmentVariable($key, $value, 'Process')
        Write-Verbose "Set $key"
    }
}

Write-Host "Starting Orka gateway..." -ForegroundColor Green
Write-Host "  Binary : $Binary"
Write-Host "  EnvFile: $EnvFile"
Write-Host "  WorkDir: $BinaryDirectory"
Write-Host ""

try {
    # Run the binary; Ctrl+C will propagate graceful shutdown
    & $Binary
}
finally {
    try {
        Stop-Transcript | Out-Null
    } catch {
    }
}
