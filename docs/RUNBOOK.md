# OpenClaw Gateway Runbook

## 1. 목적

이 문서는 `Orka Gateway` 운영 시 필요한 표준 절차를 정리한다.

- 기동
- 실시간 검증
- 장애 대응
- 안전 종료

기준 상태: 2026-02-23 코드베이스.

## 2. 사전 점검

1. 환경파일 준비

```bash
cp .env.example .env
```

2. 필수 값 확인

- `DISCORD_BOT_TOKEN` 또는 `TELEGRAM_BOT_TOKEN` 중 최소 1개
- `RUNTIME_ENGINE=echo|cli`
- `DEFAULT_PROVIDER=claude|codex|opencode`
- `DEFAULT_RUNTIME_MODE=session|event`
- `STORE_FULL_PAYLOADS=false` 권장
- `RUNTIME_ENGINE=cli`에서는 `RATE_LIMIT_MAX_REQUESTS=5` 이상 권장
- 외부 health/metrics 노출 시 `HEALTH_BEARER_TOKEN` 설정
- 운영 권한
  - 권장: `OPEN_ACCESS=false`, `ALLOWLIST=discord:<user_id>,telegram:<user_id>`
  - 공개 채널 AI 호출은 `CHANNEL_ALLOWLIST=discord:<channel_id>,telegram:<chat_id>`로 제한
  - `PUBLIC_CHAT=false` 유지
  - DM을 쓸 사용자는 반드시 `ALLOWLIST`에 포함

주의:
- 잘못된 설정값(예: `DEFAULT_PROVIDER=claud`, `DEFAULT_RUNTIME_MODE=sesion`, `PROVIDER_TIMEOUT_MS=abc`)은
  기본값으로 폴백되며 부팅 로그에 경고가 출력된다.

3. CLI 경로 확인(`RUNTIME_ENGINE=cli`인 경우)

```bash
command -v claude
command -v codex
command -v opencode
```

Codex shim 오류(`Operation not permitted`)가 있으면 `CODEX_BIN`을 네이티브 경로로 지정한다.

CLI 호출 안전성:
- 사용자 입력 프롬프트는 provider CLI 인자에서 `--` 뒤에 붙도록 강제되어
  입력 문자열이 옵션 플래그로 해석되는 위험을 줄인다.

## 3. 기동 절차

### macOS/Linux 개발 실행

```bash
cargo run -p orka-app -- doctor
cargo run -p orka-app
```

### Windows 라이브 서비스 운영

Windows 미니 PC에서 라이브 운영 중이면 `cargo run`/`pkill`/`nohup` 대신 NSSM 서비스 명령을 사용한다.

```powershell
nssm status OrkGateway
nssm stop OrkGateway
nssm start OrkGateway
nssm restart OrkGateway
Get-Content C:\Users\you\orka\logs\orka-stdout.log -Tail 100
Get-Content C:\Users\you\orka\logs\orka-stderr.log -Tail 100
```

참고:
- `cargo run -p orka-app -- onboard` 는 `.env`를 생성/갱신하는 보조 명령이다.
- `cargo run -p orka-app -- status --deep` 는 현재 설정/바이너리/DB 상태를 한 번에 보여준다.
- `orka-app`은 현재 디렉토리 또는 상위 디렉토리의 `.env`를 자동으로 로드한다.
- Windows 미니 PC에서 PIN/password 입력 전 자동 복구가 필요하면 Startup 폴더가 아니라
  `scripts/windows/install-service.ps1`로 NSSM 서비스를 등록한다.
- Windows 서비스 등록 시 `.env`를 NSSM 환경으로 import하지 않는 구성이 기본이다. 평문 `.env` 없이 서비스 계정 환경 변수나 credential loader를 사용한다.
- Windows PowerShell 스크립트의 `.env` fallback 파서는 단순 `KEY=VALUE`만 지원한다. 따옴표, 줄바꿈, 복잡한 이스케이프가 필요한 값은 서비스 계정 환경 변수로 설정한다.

정상 기동 로그 기준:

- `booting orka-gateway`
- `health server listening on ...`
- `gateway ready`
- Discord 사용 시: `discord adapter ready as ...`
- Telegram 사용 시: `telegram adapter started (polling mode)`

## 4. 헬스 확인

```bash
curl -sS http://127.0.0.1:8787/healthz
curl -sS http://127.0.0.1:8787/readyz
curl -sS http://127.0.0.1:8787/metrics
```

핵심 지표:

- `orka_inbound_total`
- `orka_outbound_total`
- `orka_error_total`
- `orka_provider_requests_total{provider,mode,status}`

## 5. 채널 기능 검증

1. Discord 또는 Telegram에서 테스트 메시지 전송
- 예: `ping`

2. 운영 명령 확인
- `/help`
- `/status`
- `/new`
- `/provider list`
- `/envvars`

3. operator 명령 확인(운영자 계정으로 실행)
- `/provider set codex` (또는 `claude`, `opencode`)
- `/mode set session` (또는 `event`)
- `/session reset`

4. 응답 확인
- inbound/outbound 모두 생성되어야 정상
- Telegram command menu와 Discord slash/menu에서 명령이 보이는지 확인

## 6. DB 검증 명령

```bash
sqlite3 data/orka-gateway.db \
  "SELECT channel,direction,count(*) FROM event_log GROUP BY channel,direction ORDER BY channel,direction;"
```

```bash
sqlite3 data/orka-gateway.db \
  "SELECT id,channel,direction,chat_id,user_id,payload_text,provider_kind,runtime_mode,provider_status,provider_latency_ms,created_at FROM event_log ORDER BY id DESC LIMIT 20;"
```

```bash
sqlite3 data/orka-gateway.db \
  "SELECT id,channel,chat_id,status,last_seen_at FROM sessions ORDER BY last_seen_at DESC LIMIT 20;"
```

## 7. 장애 대응

### 7.1 CLI spawn 실패

증상:
- `failed to spawn`

조치:
1. `*_BIN` 경로 확인
2. `command -v <binary>` 확인
3. Codex shim 실패 시 `CODEX_BIN` 네이티브 경로 지정
4. 재기동

### 7.2 CLI timeout

증상:
- `timed out after ...`

조치:
1. provider 상태/로그인 확인
2. `PROVIDER_TIMEOUT_MS` 상향
3. 임시로 `/mode set event` 전환

### 7.3 Session 오류 반복

증상:
- session 모드에서 반복 실패

조치:
1. 일반 사용자: `/new`
2. 운영자: `/session reset`
3. 필요 시 `/mode set event`
4. 필요 시 `/provider set ...` 전환

### 7.4 Discord 메시지 미수신

점검:
1. 봇이 서버에 초대되어 있는지
2. Discord Developer Portal에서 `MESSAGE CONTENT` intent 활성화 여부
3. 토큰 유효성

### 7.5 Telegram 메시지 미수신

점검:
1. 토큰 유효성
2. 네트워크/API 접근 상태
3. getUpdates를 다른 프로세스가 소모하고 있지 않은지
4. `getMyCommands`에 명령 목록이 등록되어 있는지

## 8. 안전 종료

실행 중 터미널에서 `Ctrl+C`.

Windows NSSM 서비스는 `nssm stop OrkGateway`를 사용한다. 설치 스크립트는 console stop timeout을 15초로 맞춰 기본 `SHUTDOWN_DRAIN_TIMEOUT_MS=10000`보다 길게 둔다.

정상 종료 로그 기준:
- `shutdown signal received; ... draining inflight queue`
- `inflight drain complete`
- `gateway stopped`

## Bot Token Rotation (C-1)

운영 bot token은 90일 이내 주기로 교체하고, 교체 직후 `.env` ACL과 Windows 서비스를 같이 확인한다.

1. Discord token 교체
- Discord Developer Portal → 대상 Application → Bot → Reset Token
- 새 token을 install directory의 `.env`에 `DISCORD_BOT_TOKEN=...`으로 반영

2. Telegram token 교체
- BotFather → `/revoke`
- 대상 bot을 선택한 뒤 새 token을 발급
- 새 token을 install directory의 `.env`에 `TELEGRAM_BOT_TOKEN=...`으로 반영

3. 수동 교체 후 `.env` ACL 재적용

기본 서비스명은 `OrkGateway`이다. 다른 이름(예: `OrkaGateway`)으로 등록한 환경은 service principal과 restart 명령의 이름을 맞춘다.

```powershell
$envPath = 'C:\Users\you\orka\.env'
icacls $envPath /reset
icacls $envPath /inheritance:r
icacls $envPath /remove:g *S-1-5-32-545 *S-1-5-11 *S-1-1-0
icacls $envPath /grant:r 'NT SERVICE\OrkGateway:R'
icacls $envPath
```

`LocalSystem` 서비스로 등록한 예외 환경은 마지막 grant principal을 `NT AUTHORITY\SYSTEM:R`로 바꾼다.

4. Windows 서비스 재시작

```powershell
Restart-Service OrkGateway
# or
nssm restart OrkGateway
```

재시작 후 Discord/Telegram 중 사용 채널에서 `ping` 1회와 `/help` 응답을 확인한다.

## 9. 운영 수칙

- 운영 환경에서 `OPEN_ACCESS=true` 사용 금지
- 운영 환경에서 `PUBLIC_CHAT=true` 사용 금지
- 운영 환경에서 `RATE_LIMIT_MAX_REQUESTS=0` 사용 금지
- `HEALTH_BIND`를 loopback 밖으로 열면 `HEALTH_BEARER_TOKEN`과 네트워크 allowlist를 함께 사용
- DM 사용자는 `ALLOWLIST` 기준으로만 운영
- 공개 채널 AI 호출은 `CHANNEL_ALLOWLIST` 기준으로만 운영
- provider 전환/모드 전환 시각과 사유 기록
- 장애 테스트는 소규모 채널에서 먼저 수행
- 변경 후 최소 1회 smoke test(Discord/Telegram 중 사용 채널) 수행

## 10. CI 게이트

`main` 대상으로 들어오는 `push`와 `pull_request`에는 GitHub Actions CI가 실행된다.

필수 게이트:

- `cargo build --workspace --locked`
- `cargo test --workspace --locked`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo fmt --all -- --check`
- `cargo audit`

`cargo audit`는 경고를 실패로 처리한다. `.cargo/audit.toml`에는 `serenity`의 upstream `tokio-tungstenite`/`rustls-webpki` 고정 의존성에서 들어오는 advisories만 명시적으로 ignore한다. ignore 항목은 GitHub 이슈에서 추적하고, 그 외 advisory는 CI 차단 대상이다.
