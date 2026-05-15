# Deployment & Rollback Checklist

## 1. 목적

운영 배포 시 실수 확률을 줄이고, 문제 발생 시 빠르게 롤백하기 위한 표준 체크리스트.

## 2. 배포 전 체크 (Pre-Deploy)

1. 코드/문서 기준 확인
- 최신 운영 문서 확인: `docs/RUNBOOK.md`
- 장애 계획 확인: `docs/FAILURE_EDGECASE_PLAN.md`

2. 환경값 확인
- `.env`에 운영 토큰 설정
  - `DISCORD_BOT_TOKEN`
  - `TELEGRAM_BOT_TOKEN` (사용 시)
- C-1 bot token 보안
  - .env ACL locked to service SID only (verify with icacls .env)
  - Bot tokens rotated within last 90 days; rotation procedure in RUNBOOK Bot Token Rotation section
- 운영 권한 정책
  - `OPEN_ACCESS=false`
  - `ALLOWLIST` 설정 완료
  - `PUBLIC_CHAT=false`
  - 필요한 공개 채널만 `CHANNEL_ALLOWLIST`에 등록
  - DM 사용 계정이 `ALLOWLIST`에 포함됐는지 확인
- 비용/남용 방지
  - `RUNTIME_ENGINE=cli`이면 `RATE_LIMIT_MAX_REQUESTS`가 0이 아닌지 확인
- 저장 정책
  - `STORE_FULL_PAYLOADS=false` 권장
- Health/Metrics 정책
  - `HEALTH_BIND`가 loopback 밖이면 `HEALTH_BEARER_TOKEN`과 네트워크 allowlist 설정
- 런타임 정책
  - `RUNTIME_ENGINE=cli` 또는 `echo`
  - `DEFAULT_PROVIDER`
  - `DEFAULT_RUNTIME_MODE`
- Windows service account hardening
  - [ ] Service account is not LocalSystem in production (verify with sc.exe qc OrkGateway | findstr SERVICE_START_NAME)
  - [ ] For AI CLI access (codex/claude/opencode): service runs as the user that holds the CLI auth tokens

3. 바이너리/CLI 확인 (`RUNTIME_ENGINE=cli`)

```bash
command -v claude
command -v codex
command -v opencode
```

Codex shim 오류가 있으면 `CODEX_BIN` 네이티브 경로를 사용한다.
Windows 무인 운영이면 `register-startup.ps1`가 아니라 `install-service.ps1` 서비스 등록을 사용한다. Startup 폴더 등록은 사용자 로그인 전 복구를 보장하지 않는다.
Windows 라이브 서비스에서는 `.env`를 NSSM 환경으로 import하지 말고, 서비스 계정 환경 변수나 credential loader를 사용한다.
PowerShell fallback `.env` 파서는 단순 `KEY=VALUE`만 지원하므로 따옴표/줄바꿈/복잡한 이스케이프가 필요한 값은 `.env`에 두지 않는다.

4. 로컬 검증

```bash
cargo test --workspace
```

5. DB 백업

```bash
mkdir -p backups
cp data/orka-gateway.db backups/orka-$(date +%Y%m%d-%H%M%S).db
```

## 3. 배포 절차 (Deploy)

1. 기존 프로세스 종료

macOS/Linux 개발 실행:

```bash
pkill -f orka-app || true
```

Windows 라이브 서비스:

```powershell
nssm stop OrkGateway
```

2. 앱 기동

macOS/Linux 개발 실행:

```bash
cargo run -p orka-app -- doctor
cargo run -p orka-app
```

Windows 라이브 서비스:

```powershell
C:\Users\you\orka\orka-app.exe doctor
nssm start OrkGateway
nssm status OrkGateway
Get-Content C:\Users\you\orka\logs\orka-stdout.log -Tail 100
```

3. 기동 로그 확인
- `booting orka-gateway`
- `gateway ready`
- 필요 채널 adapter ready 로그

### 3.1 실행 명령 템플릿 (문서 기준)

운영에서는 아래 템플릿을 기준으로 실행 명령만 관리한다.

Foreground:

```bash
cargo run -p orka-app
```

Background (로그 파일):

```bash
nohup cargo run -p orka-app > /tmp/orka-app.log 2>&1 &
```

중지:

```bash
pkill -f orka-app || true
```

## 4. 배포 직후 검증 (Post-Deploy Smoke)

1. HTTP 확인

```bash
curl -sS http://127.0.0.1:8787/healthz
curl -sS http://127.0.0.1:8787/readyz
curl -sS http://127.0.0.1:8787/metrics
```

2. 채널 메시지 확인
- Discord/Telegram 중 사용 채널에서 `ping` 1회
- `/help`로 명령 목록 표시 확인
- 응답 메시지 수신 확인
- DM을 사용할 경우 allowlist 계정은 성공, 비allowlist 계정은 거절 응답 확인

3. DB 이벤트 확인

```bash
sqlite3 data/orka-gateway.db \
  "SELECT id,channel,direction,payload_text,provider_kind,runtime_mode,provider_status,created_at FROM event_log ORDER BY id DESC LIMIT 10;"
```

4. 메트릭 확인
- `orka_error_total` 급증 여부
- `orka_provider_requests_total{provider,mode,status}` 증가 여부

## 5. 롤백 트리거

아래 중 1개 이상이면 즉시 롤백 검토:

- 기동 실패(ready 상태 진입 실패)
- 채널 메시지 수신/응답 불가
- provider 호출 연속 실패
- `orka_error_total` 급증
- 운영자 명령(`/status`, `/provider`, `/mode`) 비정상

## 6. 롤백 절차 (Rollback)

1. 프로세스 중지

```bash
pkill -f orka-app || true
```

2. 이전 안정 설정으로 복원
- 직전 안정 `.env` 복원
- 필요 시 `CODEX_BIN` 경로 복원

3. DB 복원이 필요한 경우(마이그레이션/데이터 이상)

```bash
cp backups/<stable-backup>.db data/orka-gateway.db
```

4. 안정 버전으로 재기동

```bash
cargo run -p orka-app
```

5. 재검증
- `healthz/readyz/metrics`
- 채널 `ping` smoke
- 필요 시 `cargo run -p orka-app -- status --deep`

## 7. 롤백 후 기록

반드시 기록:

- 배포/롤백 시각
- 증상 및 영향 범위
- 임시 조치
- 재발 방지 액션

## 8. 권장 운영 원칙

- 운영 시간대 변경은 소규모 단계 배포 우선
- 배포 직후 10~15분 집중 모니터링
- 권한 변경(`ALLOWLIST`, `OPEN_ACCESS`, `CHANNEL_ALLOWLIST`, `PUBLIC_CHAT`)은 별도 변경으로 분리
