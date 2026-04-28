# Windows Deployment Guide

Orka runs on Windows without code modifications. All dependencies (tokio, sqlx/SQLite, serenity, reqwest) fully support Windows. Discord transport now uses the platform native TLS backend, and migration SQL is loaded from the runtime `migrations/` directory, so that folder must be deployed alongside the binary.

## Tested Environment

| Item | Detail |
|------|--------|
| OS | Windows 11 (10.0.26100) |
| CPU | Intel N100 (x64) |
| Node.js | v22.14.0 |
| PowerShell | 7.5.x |
| Codex CLI | 0.113.0+ |
| Cross-compile target | `x86_64-pc-windows-gnu` (from macOS arm64) |

## Prerequisites (Windows PC)

크로스 컴파일된 바이너리를 배포하는 경우, Windows PC에 필요한 것:

| Requirement | Why | Install |
|-------------|-----|---------|
| Visual C++ Redistributable | Codex CLI 네이티브 바이너리(`codex.exe`)에 필요. 없으면 exit code 53 | [vc_redist.x64.exe](https://aka.ms/vs/17/release/vc_redist.x64.exe) |
| PowerShell 7 | Codex CLI가 셸 명령 실행 시 `pwsh.exe`를 사용. WindowsApps 스텁만으로는 동작 안 함 | [PowerShell MSI](https://github.com/PowerShell/PowerShell/releases) |
| Node.js | npm으로 AI CLI 도구 설치 | [nodejs.org](https://nodejs.org) MSI |

> **`setup.ps1`을 실행하면 위 3가지를 자동으로 설치합니다.**

### 빌드 머신 (macOS/Linux)

| Requirement | Install |
|-------------|---------|
| Rust + `x86_64-pc-windows-gnu` target | `rustup target add x86_64-pc-windows-gnu` |
| mingw-w64 | `brew install mingw-w64` (macOS) |

## Quick Start (Cross-compile + Deploy)

### 1. macOS에서 빌드

```bash
brew install mingw-w64
rustup target add x86_64-pc-windows-gnu
cargo build --release -p orka-app --target x86_64-pc-windows-gnu
```

### 2. Windows PC에 배포

```bash
# 스크립트와 바이너리 전송
scp target/x86_64-pc-windows-gnu/release/orka-app.exe user@windows-pc:"C:/Users/you/orka/"
scp -r migrations user@windows-pc:"C:/Users/you/orka/"
scp scripts/windows/setup.ps1 user@windows-pc:"C:/Users/you/orka/"
scp scripts/windows/start-orka.ps1 user@windows-pc:"C:/Users/you/orka/"
scp scripts/windows/register-startup.ps1 user@windows-pc:"C:/Users/you/orka/"
scp .env.example user@windows-pc:"C:/Users/you/orka/.env.example"
```

### 3. Windows PC에서 설정

```powershell
# 전제조건 자동 설치 (VC++ Runtime, PowerShell 7, Node.js, Codex CLI)
powershell -ExecutionPolicy Bypass -File C:\Users\you\orka\setup.ps1

# .env 편집
copy C:\Users\you\orka\.env.example C:\Users\you\orka\.env
notepad C:\Users\you\orka\.env

# 진단
C:\Users\you\orka\orka-app.exe doctor
C:\Users\you\orka\orka-app.exe status --deep

# 실행
pwsh -ExecutionPolicy Bypass -File C:\Users\you\orka\start-orka.ps1

# 자동 시작 등록
pwsh -ExecutionPolicy Bypass -File C:\Users\you\orka\register-startup.ps1
```

## Configuration (.env)

Copy `.env.example` to `.env` and set values. Windows-specific notes:

- **Paths**: Use `/` or `\\` as separator (e.g., `C:/Users/you/orka/data/orka.db`)
- **DATABASE_URL**: `sqlite://data/orka-gateway.db` (relative) works fine, or use an absolute path
- **ALLOWLIST**: keep Discord/Telegram user IDs in `ALLOWLIST` if those users should be allowed to use DM conversations

### Codex CLI 권한 설정

Codex CLI의 `-s` (sandbox)와 `-a` (approval) 플래그는 **루트 레벨 옵션**이며, `exec` 서브커맨드의 옵션이 아닙니다. 따라서 `.env`의 `CODEX_EVENT_ARGS`에 넣으면 에러가 발생합니다.

해결: `setup.ps1`이 자동으로 **래퍼 스크립트** (`codex-wrapper.cmd`)를 생성합니다:

```cmd
@echo off
setlocal
cd /d C:\Users\YOU\orka
"C:\Users\YOU\AppData\Roaming\npm\codex.cmd" %*
```

`.env` 설정:
```ini
# 래퍼를 통해 Codex 실행 (작업 디렉토리 고정)
CODEX_BIN=C:/Users/YOU/orka/codex-wrapper.cmd
CODEX_EVENT_ARGS=exec --json --skip-git-repo-check
CODEX_SESSION_ARGS=exec --json --skip-git-repo-check
```

> 기본값에서는 Codex의 sandbox/approval 정책을 약화하지 않습니다. 운영 환경에서는 우회 플래그를 문서화된 기본값으로 배포하지 마세요.

## Auto-start / Supervision

### 방법 1: NSSM Windows 서비스 (권장, 상시 운용)

로그인 없이도 부팅 시 자동 실행이 필요하거나, 미니 PC를 무인으로 계속 돌릴 경우 이 방법을 사용하세요.
`install-service.ps1`는 `orka-app.exe`가 비정상 종료되면 NSSM이 다시 시작하도록 기본 재시작 정책을 설정합니다.

```powershell
choco install nssm
.\scripts\windows\install-service.ps1
nssm start OrkGateway
```

### 방법 2: register-startup.ps1 (로그인 세션용, 관리자 불필요)

```powershell
# 등록
pwsh -ExecutionPolicy Bypass -File C:\Users\you\orka\register-startup.ps1

# 해제
pwsh -ExecutionPolicy Bypass -File C:\Users\you\orka\register-startup.ps1 -Unregister
```

사용자 로그인 시 자동으로 `start-orka.ps1`이 한 번 실행됩니다.
이 방식은 편의용이며, 프로세스가 종료된 뒤 자동 복구를 보장하지 않습니다.

## Remote Deployment via SSH

SSH로 원격 배포 시 추가 주의사항:

```bash
# 스크립트 전송 후 setup.ps1로 한 번에 설치
ssh user@windows-pc "powershell -ExecutionPolicy Bypass -File C:\Users\you\orka\setup.ps1"
```

### SSH 세션 주의사항

| 이슈 | 원인 | 해결 |
|------|------|------|
| PATH에 새 도구가 안 보임 | MSI 설치 후 현재 세션에 반영 안 됨 | `$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User')` |
| PowerShell 스크립트 실행 거부 | ExecutionPolicy 기본값이 Restricted | `-ExecutionPolicy Bypass` 플래그 사용 |
| SSH 세션 종료 시 프로세스 죽음 | SSH 프로세스 트리가 함께 종료 | 상시 운용이면 NSSM 서비스 사용. 로그인 세션 기반이면 `register-startup.ps1` 등록 후 로그인으로 시작 |
| Tailscale IP로 SSH 불가 | 재부팅 후 Tailscale 서비스 시작 지연 | 부팅 후 1-2분 대기, 또는 로컬 IP 사용 |

## Verification

```powershell
# Health check
curl http://localhost:8787/healthz   # "ok"
curl http://localhost:8787/readyz    # "ready"
```

## Troubleshooting

### Codex CLI exits with code 53
`vcruntime140.dll` 누락. Visual C++ Redistributable 설치:
```powershell
Invoke-WebRequest -Uri 'https://aka.ms/vs/17/release/vc_redist.x64.exe' -OutFile vc_redist.x64.exe
Start-Process vc_redist.x64.exe -ArgumentList '/install','/quiet','/norestart' -Wait
```

### Codex 셸 명령이 출력 없이 실패
PowerShell 7이 설치되지 않음. Codex는 `pwsh.exe`를 사용하는데, WindowsApps의 스텁은 동작하지 않음:
```powershell
# PowerShell 7 설치
Invoke-WebRequest -Uri 'https://github.com/PowerShell/PowerShell/releases/download/v7.5.1/PowerShell-7.5.1-win-x64.msi' -OutFile pwsh.msi
Start-Process msiexec.exe -ArgumentList '/i','pwsh.msi','/quiet','/norestart' -Wait
```

### Codex exec에서 `-a` unexpected argument 에러
`-s`와 `-a`는 `codex` 루트 옵션이지 `exec` 서브커맨드 옵션이 아님. `codex-wrapper.cmd`를 사용하세요 (위 "Codex CLI 권한 설정" 참고).

### Port already in use (error 10048)
이전 프로세스가 완전히 종료되지 않음:
```powershell
taskkill /IM orka-app.exe /F
# 3초 대기 후 재시작
```

### Firewall
Windows Defender Firewall에서 `orka-app.exe`의 아웃바운드 연결 허용:
- Settings > Privacy & Security > Windows Security > Firewall > Allow an app

### Sleep Mode
상시 가동 시 절전 모드 비활성화:
- Settings > System > Power > Screen and sleep > Never

### CLI not found / PATH issues
`.env`에 절대 경로를 지정:
```ini
CODEX_BIN=C:/Users/YOU/orka/codex-wrapper.cmd
CLAUDE_BIN=C:/Users/YOU/AppData/Roaming/npm/claude.cmd
```
