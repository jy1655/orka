# Technical Plan (Current v0.2)

## 1. Goal

`orka-gateway`는 Discord/Telegram 메시지를 공통 파이프라인으로 처리하고,
runtime backend를 `echo` 또는 로컬 CLI(`claude`/`codex`/`opencode`)로 선택 실행한다.

핵심 목표:
- Discord / Telegram 양방향 메시지 파이프라인
- scope 단위 runtime 선택(`provider`, `mode`)
- session/event 모드 지원
- 예측 가능한 동작(중복 방지, 실패 복구, 종료 시 drain)
- 운영 기본기(health, metrics, structured logging)

## 2. Scope

### In Scope (현재 구현)

- Discord inbound/outbound (text)
- Telegram inbound/outbound (text, polling)
- 공통 `InboundEvent -> GatewayPipeline -> OutboundAction`
- Runtime 엔진
  - `echo`
  - `cli` (`claude`/`codex`/`opencode`)
- SQLite 기반 상태 저장
  - sessions
  - event_log
  - command_state
  - provider_preferences
  - provider_sessions
- idempotency key 기반 중복 이벤트 방지
- 채널 명령 기반 운영 제어
  - `/help`
  - `/status`
  - `/new`
  - `/provider list`
  - `/provider set ...`
  - `/mode set ...`
  - `/session reset`
  - `/pause`, `/resume`
  - `/envvars`
  - `/audit`
- `/healthz`, `/readyz`, `/metrics`

### Out of Scope (현재)

- HTTP Control API 런타임 노출
- 웹 UI
- 분산 실행/클러스터링
- 미디어/파일/음성 고급 처리

## 3. Architecture Principles

- Single runtime: `tokio`
- Clear boundaries
  - `core`: 채널 비의존 도메인/정책/파이프라인
  - `adapters`: 채널별 I/O
  - `storage`: 내구성/상태
  - `app`: 부팅/배선/수명주기
- Failure containment
  - adapter 장애와 runtime 실패를 경계 내에서 처리
- Deterministic processing
  - idempotency 기반 duplicate 방지
- Security baseline
  - 토큰은 env 입력
  - operator 명령 권한 체크(`ALLOWLIST`, `OPEN_ACCESS`)
  - DM은 allowlist 사용자만 허용

## 4. Non-Functional Requirements

- Reliability
  - Telegram polling 백오프
  - graceful shutdown inflight drain (`SHUTDOWN_DRAIN_TIMEOUT_MS`)
- Runtime safety
  - provider timeout (`PROVIDER_TIMEOUT_MS`)
  - output size limit (`MAX_OUTPUT_BYTES`)
  - session 실패 시 event 폴백 옵션 (`SESSION_FAIL_FALLBACK_EVENT`)
- Observability
  - JSON tracing logs
  - metrics
    - inbound/outbound/error counter
    - `orka_provider_requests_total{provider,mode,status}`

## 5. Data Model (SQLite)

- `sessions`
  - `id`, `channel`, `chat_id`, `status`, `last_seen_at`
- `event_log`
  - `idempotency_key`, `channel`, `direction`, `chat_id`, `user_id`, `payload_text`, `created_at`
  - `provider_kind`, `runtime_mode`, `provider_latency_ms`, `provider_status`
- `command_state`
  - `scope_key`, `paused`, `updated_at`
- `provider_preferences`
  - `scope_key`, `provider_kind`, `mode`, `updated_at`
- `provider_sessions`
  - `scope_key`, `provider_kind`, `provider_session_id`, `last_used_at`, `metadata_json`

## 6. Runtime Flow

1. Adapter가 채널 메시지 수신
2. `InboundEvent` 생성
3. pipeline에서 duplicate/command/policy 처리
4. provider+mode 결정
5. runtime invoke
6. outbound 전송 및 로그 저장

## 7. Current Risks

- provider CLI 버전별 출력 포맷 변화
- 운영 환경에서 외부 CLI 인증 상태 의존
- Discord DM 사용성 제약(친구 목록 노출 불가, 서버 컨텍스트 의존)
- 채널별 command menu/slash 전파 지연

## 8. References

- `docs/ARCHITECTURE.md`
- `docs/IMPLEMENTATION_ROADMAP.md`
- `docs/FAILURE_EDGECASE_PLAN.md`
- `docs/DISCORD_MULTI_CLI_SPEC.md`
