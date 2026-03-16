<#
.SYNOPSIS
    Registers Orka to start automatically at Windows logon.
.DESCRIPTION
    Creates a .cmd file in the user's Startup folder that launches orka-app.exe
    via start-orka.ps1. This survives reboots and does not require admin privileges.
.PARAMETER InstallDir
    Directory containing orka-app.exe and .env. Default: same directory as this script.
.PARAMETER Unregister
    Remove the startup entry.
#>
param(
    [string]$InstallDir,
    [switch]$Unregister
)

$ErrorActionPreference = 'Stop'

$startupDir = [System.Environment]::GetFolderPath('Startup')
$startupFile = Join-Path $startupDir 'orka-gateway.cmd'

if ($Unregister) {
    if (Test-Path $startupFile) {
        Remove-Item $startupFile -Force
        Write-Host "Startup entry removed." -ForegroundColor Green
    } else {
        Write-Host "No startup entry found." -ForegroundColor Yellow
    }
    exit 0
}

if (-not $InstallDir) {
    $InstallDir = $PSScriptRoot
    if (-not $InstallDir) { $InstallDir = Split-Path -Parent $MyInvocation.MyCommand.Path }
}

$startScript = Join-Path $InstallDir 'start-orka.ps1'
if (-not (Test-Path $startScript)) {
    Write-Error "start-orka.ps1 not found in $InstallDir"
    exit 1
}

$content = @(
    '@echo off',
    "cd /d ""$InstallDir""",
    "pwsh.exe -ExecutionPolicy Bypass -WindowStyle Hidden -File ""$startScript"""
)
Set-Content $startupFile $content

Write-Host "Startup entry created:" -ForegroundColor Green
Write-Host "  $startupFile" -ForegroundColor Cyan
Write-Host ""
Write-Host "Orka will start automatically at next logon." -ForegroundColor White
Write-Host "To remove: pwsh -File $($MyInvocation.MyCommand.Path) -Unregister" -ForegroundColor Yellow
