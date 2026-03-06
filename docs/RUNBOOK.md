# OpenClaw Gateway Runbook

## 1. 목적

이 문서는 `openclaw-rs-gateway` 운영 시 필요한 표준 절차를 정리한다.

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
- 운영 권한
  - 권장: `OPEN_ACCESS=false`, `ALLOWLIST=discord:<user_id>,telegram:<user_id>`

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

```bash
set -a; source .env; set +a
cargo run -p openclaw-app
```

정상 기동 로그 기준:

- `booting openclaw-rs-gateway`
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

- `openclaw_inbound_total`
- `openclaw_outbound_total`
- `openclaw_error_total`
- `openclaw_provider_requests_total{provider,mode,status}`

## 5. 채널 기능 검증

1. Discord 또는 Telegram에서 테스트 메시지 전송
- 예: `ping`

2. 운영 명령 확인
- `/status`
- `/provider list`

3. operator 명령 확인(운영자 계정으로 실행)
- `/provider set codex` (또는 `claude`, `opencode`)
- `/mode set session` (또는 `event`)
- `/session reset`

4. 응답 확인
- inbound/outbound 모두 생성되어야 정상

## 6. DB 검증 명령

```bash
sqlite3 data/openclaw-rs-gateway.db \
  "SELECT channel,direction,count(*) FROM event_log GROUP BY channel,direction ORDER BY channel,direction;"
```

```bash
sqlite3 data/openclaw-rs-gateway.db \
  "SELECT id,channel,direction,chat_id,user_id,payload_text,provider_kind,runtime_mode,provider_status,provider_latency_ms,created_at FROM event_log ORDER BY id DESC LIMIT 20;"
```

```bash
sqlite3 data/openclaw-rs-gateway.db \
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
1. `/session reset`
2. 필요 시 `/mode set event`
3. 필요 시 `/provider set ...` 전환

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

## 8. 안전 종료

실행 중 터미널에서 `Ctrl+C`.

정상 종료 로그 기준:
- `shutdown signal received; ... draining inflight queue`
- `inflight drain complete`
- `gateway stopped`

## 9. 운영 수칙

- 운영 환경에서 `OPEN_ACCESS=true` 사용 금지
- provider 전환/모드 전환 시각과 사유 기록
- 장애 테스트는 소규모 채널에서 먼저 수행
- 변경 후 최소 1회 smoke test(Discord/Telegram 중 사용 채널) 수행
