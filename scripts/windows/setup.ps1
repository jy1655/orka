<#
.SYNOPSIS
    Sets up the Orka runtime environment on Windows.
.DESCRIPTION
    Installs prerequisites (VC++ Runtime, PowerShell 7, Node.js, Codex CLI)
    and prepares the deployment directory. Designed for cross-compiled binary deployment
    (no Rust/build tools needed on the target machine).
.PARAMETER InstallDir
    Deployment directory. Default: C:\Users\<you>\orka
.PARAMETER SkipCLIs
    Skip AI CLI tool installation.
.PARAMETER InstallService
    Register Orka as a Windows service after prerequisites are prepared. Requires
    an elevated PowerShell session.
.PARAMETER InstallNssm
    Let install-service.ps1 download and install NSSM if it is missing.
.PARAMETER ServiceName
    Windows service name used with -InstallService. Default: OrkGateway.
.PARAMETER ProfileRoot
    User profile root to expose to the service when a provider CLI needs it.
.PARAMETER ServiceAccount
    Windows service account passed to install-service.ps1.
.PARAMETER ServicePassword
    Password for ServiceAccount when using a normal user account.
.PARAMETER RunAsLocalSystem
    Legacy service-account compatibility switch.
.PARAMETER ImportEnvFile
    Import .env values into NSSM service configuration. Avoid for live use.
.PARAMETER DelayedAutoStart
    Use delayed automatic service start when -InstallService is supplied.
#>
param(
    [string]$InstallDir,
    [switch]$SkipCLIs,
    [switch]$InstallService,
    [switch]$InstallNssm,
    [string]$ServiceName = 'OrkGateway',
    [string]$ProfileRoot,
    [string]$ServiceAccount,
    [string]$ServicePassword,
    [switch]$RunAsLocalSystem,
    [switch]$ImportEnvFile,
    [switch]$DelayedAutoStart
)

$ErrorActionPreference = 'Stop'

if (-not $InstallDir) {
    $InstallDir = Join-Path $env:USERPROFILE 'orka'
}

function Test-Command($cmd) {
    return [bool](Get-Command $cmd -ErrorAction SilentlyContinue)
}

function Refresh-Path {
    $env:Path = [System.Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
                [System.Environment]::GetEnvironmentVariable('Path', 'User')
}

function Get-ServiceAclPrincipal {
    param(
        [string]$Account,
        [switch]$RunAsLocalSystem,
        [string]$Name
    )

    if ($RunAsLocalSystem -or $Account -ieq 'LocalSystem') {
        return 'NT AUTHORITY\SYSTEM'
    }
    if ($Account) {
        return $Account
    }

    return "NT SERVICE\$Name"
}

function Invoke-Icacls {
    param(
        [string]$Path,
        [string[]]$Arguments
    )

    $icaclsArgs = @($Path) + $Arguments
    & icacls @icaclsArgs
    if ($LASTEXITCODE -ne 0) {
        throw "icacls failed for path: $Path"
    }
}

function Protect-EnvFileAcl {
    param(
        [string]$Path,
        [string]$Principal
    )

    if (-not $Path -or -not (Test-Path $Path)) {
        return
    }

    Write-Host "[OK] Locking .env ACL for service principal: $Principal" -ForegroundColor Green
    Invoke-Icacls -Path $Path -Arguments @('/reset')
    Invoke-Icacls -Path $Path -Arguments @('/inheritance:r')
    Invoke-Icacls -Path $Path -Arguments @('/remove:g', '*S-1-5-32-545', '*S-1-5-11', '*S-1-1-0')
    Invoke-Icacls -Path $Path -Arguments @('/grant:r', "${Principal}:R")
}

function Find-WindowsScript {
    param([string]$Name)

    $candidates = @(
        (Join-Path $InstallDir $Name),
        (Join-Path $PSScriptRoot $Name)
    )

    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path $candidate)) {
            return $candidate
        }
    }

    return $null
}

function Assert-FileSha256 {
    param(
        [string]$Path,
        [string]$ExpectedSha256,
        [string]$Label
    )

    $actual = (Get-FileHash -Path $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $ExpectedSha256.ToLowerInvariant()) {
        Write-Error "$Label SHA256 mismatch. Expected $ExpectedSha256 but got $actual"
        exit 1
    }
}

function Assert-TrustedSignature {
    param(
        [string]$Path,
        [string]$ExpectedSubjectContains,
        [string]$Label
    )

    $signature = Get-AuthenticodeSignature -FilePath $Path
    if ($signature.Status -ne 'Valid') {
        Write-Error "$Label signature is not valid: $($signature.Status)"
        exit 1
    }
    if ($ExpectedSubjectContains -and $signature.SignerCertificate.Subject -notlike "*$ExpectedSubjectContains*") {
        Write-Error "$Label signer mismatch: $($signature.SignerCertificate.Subject)"
        exit 1
    }
}

Write-Host "=== Orka Windows Setup ===" -ForegroundColor Cyan
Write-Host "  Install dir: $InstallDir"
Write-Host ""

# --- 1. Visual C++ Redistributable (required by Codex CLI native binary) ---
if (Test-Path "$env:SystemRoot\System32\vcruntime140.dll") {
    Write-Host "[OK] Visual C++ Runtime" -ForegroundColor Green
} else {
    Write-Host "[INSTALLING] Visual C++ Redistributable..." -ForegroundColor Yellow
    $vcUrl = 'https://aka.ms/vs/17/release/vc_redist.x64.exe'
    $vcPath = Join-Path $env:TEMP 'vc_redist.x64.exe'
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $vcUrl -OutFile $vcPath
    Assert-TrustedSignature -Path $vcPath -ExpectedSubjectContains 'Microsoft' -Label 'Visual C++ Redistributable'
    Start-Process $vcPath -ArgumentList '/install', '/quiet', '/norestart' -Wait
    if (Test-Path "$env:SystemRoot\System32\vcruntime140.dll") {
        Write-Host "[OK] Visual C++ Runtime installed" -ForegroundColor Green
    } else {
        Write-Host "[ERROR] VC++ Runtime installation failed" -ForegroundColor Red
        exit 1
    }
}

# --- 2. PowerShell 7 (required by Codex CLI for shell command execution) ---
Refresh-Path
if (Test-Command 'pwsh') {
    $pwshVersion = (pwsh --version)
    Write-Host "[OK] $pwshVersion" -ForegroundColor Green
} else {
    Write-Host "[INSTALLING] PowerShell 7..." -ForegroundColor Yellow
    $pwshUrl = 'https://github.com/PowerShell/PowerShell/releases/download/v7.5.1/PowerShell-7.5.1-win-x64.msi'
    $pwshSha256 = 'b110eccaf55bb53ae5e6b6de478587ed8203570b0bda9bd374a0998e24d4033a'
    $pwshPath = Join-Path $env:TEMP 'pwsh-install.msi'
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $pwshUrl -OutFile $pwshPath
    Assert-FileSha256 -Path $pwshPath -ExpectedSha256 $pwshSha256 -Label 'PowerShell 7 MSI'
    Assert-TrustedSignature -Path $pwshPath -ExpectedSubjectContains 'Microsoft' -Label 'PowerShell 7 MSI'
    Start-Process msiexec.exe -ArgumentList '/i', $pwshPath, '/quiet', '/norestart' -Wait
    Refresh-Path
    if (Test-Command 'pwsh') {
        Write-Host "[OK] $(pwsh --version) installed" -ForegroundColor Green
    } else {
        Write-Host "[ERROR] PowerShell 7 installation failed" -ForegroundColor Red
        exit 1
    }
}

# --- 3. Node.js (required for AI CLI tools) ---
Refresh-Path
if (Test-Command 'node') {
    $nodeVersion = (node --version)
    Write-Host "[OK] Node.js $nodeVersion" -ForegroundColor Green
} else {
    Write-Host "[INSTALLING] Node.js LTS..." -ForegroundColor Yellow
    $nodeUrl = 'https://nodejs.org/dist/v22.14.0/node-v22.14.0-x64.msi'
    $nodeSha256 = '2c0cc97ec64c1e4111362e1e32e0547fd870e4d9c79ec844c117da583f21b386'
    $nodePath = Join-Path $env:TEMP 'node-install.msi'
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $nodeUrl -OutFile $nodePath
    Assert-FileSha256 -Path $nodePath -ExpectedSha256 $nodeSha256 -Label 'Node.js MSI'
    Assert-TrustedSignature -Path $nodePath -ExpectedSubjectContains 'OpenJS Foundation' -Label 'Node.js MSI'
    Start-Process msiexec.exe -ArgumentList '/i', $nodePath, '/quiet', '/norestart' -Wait
    Refresh-Path
    if (Test-Command 'node') {
        Write-Host "[OK] Node.js $(node --version) installed" -ForegroundColor Green
    } else {
        Write-Host "[WARN] Node.js installed but not in PATH. Restart shell or reboot." -ForegroundColor Yellow
    }
}

# --- 4. Install directory ---
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}
$dataDir = Join-Path $InstallDir 'data'
if (-not (Test-Path $dataDir)) {
    New-Item -ItemType Directory -Path $dataDir -Force | Out-Null
}
Write-Host "[OK] Install directory: $InstallDir" -ForegroundColor Green

# --- 5. AI CLI tools ---
if (-not $SkipCLIs) {
    Refresh-Path
    $npmCmd = "$(& { where.exe npm 2>$null | Select-Object -First 1 })"
    if (-not $npmCmd) {
        $npmCmd = "C:\Program Files\nodejs\npm.cmd"
    }
    if (Test-Path $npmCmd) {
        Write-Host ""
        Write-Host "Installing AI CLI tools..." -ForegroundColor Cyan

        Write-Host "  Installing @openai/codex..."
        & $npmCmd install -g @openai/codex 2>&1 | Out-Null

        $npmPrefix = (& $npmCmd prefix -g).Trim()
        $codexCmd = Join-Path $npmPrefix 'codex.cmd'

        if (Test-Path $codexCmd) {
            Write-Host "[OK] Codex CLI installed" -ForegroundColor Green

            # Create a lightweight codex wrapper that runs from the install
            # directory without weakening Codex's default sandbox/approval policy.
            $wrapperPath = Join-Path $InstallDir 'codex-wrapper.cmd'
            $wrapperContent = @(
                '@echo off',
                'setlocal',
                "cd /d ""$InstallDir""",
                """$codexCmd"" %*"
            )
            Set-Content $wrapperPath $wrapperContent
            Write-Host "[OK] Codex wrapper created: $wrapperPath" -ForegroundColor Green
            Write-Host "     Set CODEX_BIN=$($wrapperPath -replace '\\','/')" -ForegroundColor Yellow
        } else {
            Write-Host "[WARN] Codex CLI install may have failed" -ForegroundColor Yellow
        }
    } else {
        Write-Host "[WARN] npm not found. Restart shell after Node.js install." -ForegroundColor Yellow
    }
}

# --- 6. .env file ---
$envFile = Join-Path $InstallDir '.env'
$envExample = Join-Path $InstallDir '.env.example'
if (Test-Path $envFile) {
    Write-Host "[OK] .env file exists" -ForegroundColor Green
} elseif (Test-Path $envExample) {
    Copy-Item $envExample $envFile
    Write-Host "[CREATED] .env copied from .env.example" -ForegroundColor Yellow
    $serviceAclPrincipal = Get-ServiceAclPrincipal -Account $ServiceAccount -RunAsLocalSystem:$RunAsLocalSystem -Name $ServiceName
    Protect-EnvFileAcl -Path $envFile -Principal $serviceAclPrincipal
} else {
    Write-Host "[WARN] No .env file. Copy .env.example and edit it." -ForegroundColor Yellow
}

# --- 7. Optional unattended Windows service registration ---
if ($InstallService) {
    $binaryPath = Join-Path $InstallDir 'orka-app.exe'
    $serviceScript = Find-WindowsScript 'install-service.ps1'

    if (-not (Test-Path $binaryPath)) {
        Write-Error "Cannot install service because binary was not found: $binaryPath"
        exit 1
    }

    if (-not $serviceScript) {
        Write-Error "Cannot install service because install-service.ps1 was not found in $InstallDir or $PSScriptRoot"
        exit 1
    }

    $serviceArgs = @{
        ServiceName = $ServiceName
        BinaryPath = $binaryPath
        WorkDir = $InstallDir
        EnvFile = $envFile
    }
    if ($ProfileRoot) {
        $serviceArgs['ProfileRoot'] = $ProfileRoot
    }
    if ($ServiceAccount) {
        $serviceArgs['ServiceAccount'] = $ServiceAccount
    }
    if ($ServicePassword) {
        $serviceArgs['ServicePassword'] = $ServicePassword
    }
    if ($RunAsLocalSystem) {
        $serviceArgs['RunAsLocalSystem'] = $true
    }
    if ($ImportEnvFile) {
        $serviceArgs['ImportEnvFile'] = $true
    }
    if ($InstallNssm) {
        $serviceArgs['InstallNssm'] = $true
    }
    if ($DelayedAutoStart) {
        $serviceArgs['DelayedAutoStart'] = $true
    }

    Write-Host ""
    Write-Host "Installing unattended Windows service..." -ForegroundColor Cyan
    & $serviceScript @serviceArgs
}

Write-Host ""
Write-Host "=== Setup complete ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "Next steps:" -ForegroundColor White
if ($InstallService) {
    Write-Host "  1. Start now: nssm start $ServiceName"
    Write-Host "  2. Verify: nssm status $ServiceName"
    Write-Host "  3. Check logs: $InstallDir\logs\orka-stdout.log and orka-stderr.log"
} else {
    Write-Host "  1. Copy orka-app.exe to $InstallDir"
    Write-Host "  2. Copy .env.example to $InstallDir\.env and edit with your tokens"
    Write-Host "  3. Set CODEX_BIN to the codex-wrapper.cmd path in .env"
    Write-Host "  4. Unattended boot: run this script again with -InstallService -InstallNssm -DelayedAutoStart"
    Write-Host "  5. Login-only start: pwsh -File $InstallDir\register-startup.ps1"
}
