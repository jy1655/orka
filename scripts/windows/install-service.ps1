#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Installs Orka as a Windows service using NSSM.
.DESCRIPTION
    Registers orka-app.exe as a Windows service via NSSM (Non-Sucking Service Manager).
    NSSM must be installed and available on PATH, or this script can install it
    when -InstallNssm is supplied. If the service already exists, its NSSM
    settings are updated in place.
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
.PARAMETER InstallNssm
    Download and install NSSM if it is not already available on PATH.
.PARAMETER NssmInstallDir
    Directory used when -InstallNssm is supplied. Default: C:\ProgramData\nssm.
.PARAMETER DelayedAutoStart
    Use Windows delayed automatic start so networking services can settle after boot.
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
    [switch]$InstallNssm,
    [string]$NssmInstallDir = (Join-Path $env:ProgramData 'nssm'),
    [switch]$DelayedAutoStart,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'

function Add-ToMachinePath {
    param([string]$Directory)

    $machinePath = [System.Environment]::GetEnvironmentVariable('Path', 'Machine')
    $entries = @()
    if ($machinePath) {
        $entries = $machinePath -split ';' | Where-Object { $_ }
    }

    $alreadyPresent = $entries | Where-Object {
        $_.TrimEnd('\') -ieq $Directory.TrimEnd('\')
    }

    if (-not $alreadyPresent) {
        $newPath = (@($entries) + $Directory) -join ';'
        [System.Environment]::SetEnvironmentVariable('Path', $newPath, 'Machine')
    }

    if (($env:Path -split ';') -notcontains $Directory) {
        $env:Path = "$Directory;$env:Path"
    }
}

function Install-Nssm {
    param([string]$DestinationDir)

    $version = '2.24'
    $zipUrl = "https://nssm.cc/release/nssm-$version.zip"
    $zipPath = Join-Path $env:TEMP "nssm-$version.zip"
    $extractDir = Join-Path $env:TEMP "nssm-install-$([Guid]::NewGuid().ToString('N'))"

    Write-Host "NSSM not found. Installing NSSM $version..." -ForegroundColor Yellow
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $zipUrl -OutFile $zipPath -UseBasicParsing

    if (Test-Path $extractDir) {
        Remove-Item $extractDir -Recurse -Force
    }
    Expand-Archive -Path $zipPath -DestinationPath $extractDir -Force

    $sourceExe = Get-ChildItem $extractDir -Recurse -Filter 'nssm.exe' |
        Where-Object { $_.FullName -match '\\win64\\' } |
        Select-Object -First 1

    if (-not $sourceExe) {
        Write-Error "Downloaded NSSM archive did not contain win64\nssm.exe"
        exit 1
    }

    if (-not (Test-Path $DestinationDir)) {
        New-Item -ItemType Directory -Path $DestinationDir -Force | Out-Null
    }

    $installedExe = Join-Path $DestinationDir 'nssm.exe'
    Copy-Item $sourceExe.FullName $installedExe -Force
    Add-ToMachinePath $DestinationDir
    Write-Host "NSSM installed: $installedExe" -ForegroundColor Green
    return $installedExe
}

function Test-NssmExecutable {
    param([string]$Path)

    if (-not $Path -or -not (Test-Path $Path)) {
        return $false
    }

    try {
        $startInfo = New-Object System.Diagnostics.ProcessStartInfo
        $startInfo.FileName = $Path
        $startInfo.Arguments = 'status __nssm_probe__'
        $startInfo.UseShellExecute = $false
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true

        $process = New-Object System.Diagnostics.Process
        $process.StartInfo = $startInfo
        [void]$process.Start()
        if (-not $process.WaitForExit(5000)) {
            $process.Kill()
            return $false
        }

        return $true
    } catch {
        return $false
    }
}

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

$NssmCommand = Get-Command nssm -ErrorAction SilentlyContinue
$NssmExe = $null
if ($NssmCommand -and (Test-NssmExecutable $NssmCommand.Source)) {
    $NssmExe = $NssmCommand.Source
}

if (-not $NssmExe -and $InstallNssm) {
    $NssmExe = Install-Nssm -DestinationDir $NssmInstallDir
}

if (-not (Test-NssmExecutable $NssmExe)) {
    Write-Error @"
NSSM not found. Install it first:
  .\install-service.ps1 -InstallNssm -BinaryPath C:\Users\you\orka\orka-app.exe
  choco install nssm
  -- or --
  Download from https://nssm.cc and add to PATH.
"@
    exit 1
}

if ($Uninstall) {
    Write-Host "Stopping service '$ServiceName'..." -ForegroundColor Yellow
    & $NssmExe stop $ServiceName 2>$null
    Write-Host "Removing service '$ServiceName'..." -ForegroundColor Yellow
    & $NssmExe remove $ServiceName confirm
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
if ($DelayedAutoStart) {
    Write-Host "  Startup: delayed automatic"
} else {
    Write-Host "  Startup: automatic"
}

# Install service
$ExistingService = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
if ($ExistingService) {
    Write-Host "  Existing service found; updating NSSM configuration"
    & $NssmExe set $ServiceName ImagePath $NssmExe
    & $NssmExe set $ServiceName Application $BinaryPath
} else {
    & $NssmExe install $ServiceName $BinaryPath
}
& $NssmExe set $ServiceName AppDirectory $WorkDir
& $NssmExe set $ServiceName DisplayName "Orka AI Gateway"
& $NssmExe set $ServiceName Description "Discord/Telegram AI CLI gateway"
if ($DelayedAutoStart) {
    & $NssmExe set $ServiceName Start SERVICE_DELAYED_AUTO_START
} else {
    & $NssmExe set $ServiceName Start SERVICE_AUTO_START
}
& $NssmExe set $ServiceName ObjectName LocalSystem
& $NssmExe set $ServiceName AppExit Default Restart
& $NssmExe set $ServiceName AppRestartDelay $RestartDelayMs

# Stdout/stderr logging
$LogDir = Join-Path $WorkDir 'logs'
if (-not (Test-Path $LogDir)) {
    New-Item -ItemType Directory -Path $LogDir -Force | Out-Null
}
& $NssmExe set $ServiceName AppStdout (Join-Path $LogDir 'orka-stdout.log')
& $NssmExe set $ServiceName AppStderr (Join-Path $LogDir 'orka-stderr.log')
& $NssmExe set $ServiceName AppStdoutCreationDisposition 4  # append
& $NssmExe set $ServiceName AppStderrCreationDisposition 4  # append
& $NssmExe set $ServiceName AppRotateFiles 1
& $NssmExe set $ServiceName AppRotateBytes 10485760  # 10 MB

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
    & $NssmExe set $ServiceName AppEnvironmentExtra @envVars
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
