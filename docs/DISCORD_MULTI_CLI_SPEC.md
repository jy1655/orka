# Channel + Multi CLI Runtime Spec (Current v0.3)

## 1. 목적

이 문서는 현재 `orka-gateway` 구현 기준의 runtime 동작 규칙을 정리한다.

- 입력 채널: Discord, Telegram
- 실행 backend: Claude Code CLI, Codex CLI, OpenCode CLI
- 동작 모드: `session` / `event`

## 2. 현재 범위

### In Scope

- Discord inbound/outbound
- Telegram inbound/outbound (polling)
- provider 선택(`claude`/`codex`/`opencode`)
- mode 선택(`session`/`event`)
- scope별 runtime preference/session 저장
- 채널 명령 기반 운영 제어

### Out of Scope

- HTTP Control API 런타임 노출
- 웹 UI
- 클러스터/분산 락

## 3. 용어

- `scope_key`: `channel:chat_id` 형태의 conversation 식별자
- `provider`: CLI 실행 엔진 (`claude`, `codex`, `opencode`)
- `mode`
  - `session`: provider session 재사용
  - `event`: 요청 단위 stateless 실행

## 4. 아키텍처 흐름

```text
Discord/Telegram Adapter
  -> InboundEvent
  -> GatewayPipeline
    -> policy + command handling
    -> runtime invoke
  -> OutboundAction
  -> Discord/Telegram Outbound
```

## 5. Provider 실행 규칙

공통 규칙:
- 쉘 문자열 실행 금지 (`Command::new + args`)
- timeout 강제 (`PROVIDER_TIMEOUT_MS`)
- 출력 크기 제한 (`MAX_OUTPUT_BYTES`)
- stdout/stderr 분리 수집
- stderr 민감정보 마스킹 로그

기본 호출 규칙:

- `claude`
  - event: `claude -p --output-format json "<prompt>"`
  - session: `claude -p --output-format json -r <session_id> "<prompt>"`
- `codex`
  - event: `codex exec --json --skip-git-repo-check "<prompt>"`
  - session: `codex exec --json --skip-git-repo-check resume <session_id> "<prompt>"`
- `opencode`
  - event: `opencode run --format json "<prompt>"`
  - session: `opencode run --format json --session <session_id> "<prompt>"`

참고:
- `CODEX_BIN`이 shim 경로일 때 macOS 권한 이슈가 있으면 네이티브 바이너리 경로를 직접 지정한다.

## 6. Mode/Session 동작

- `event`
  - 매 요청 독립 실행
  - provider session 저장/갱신 없음
- `session`
  - 저장된 provider session이 있으면 resume
  - 없으면 새 session으로 실행 후 session id 저장

폴백 정책:
- `SESSION_FAIL_FALLBACK_EVENT=true`이고,
  session 실행 실패 + 기존 session id가 있는 경우
  - 해당 provider session 삭제
  - event 모드로 1회 재시도

## 7. 운영 제어 명령

- 공통
  - `/help`
  - `/status`
  - `/new`
  - `/provider list`
- operator only
  - `/provider set <claude|codex|opencode>`
  - `/mode set <session|event>`
  - `/session reset`
  - `/pause`
  - `/resume`
  - `/envvars`
  - `/audit [count]`

명령 노출:
- Discord: `/ask` + global slash command로 `help`, `status`, `new`, `provider_*`, `mode_*`, `session_reset` 등을 노출
- Telegram: `setMyCommands`로 `help`, `status`, `new`, `provider_*`, `mode_*`, `session_reset` 등을 노출
- 텍스트 입력은 기존 공백 형식(`/provider set codex`)과 menu 친화적 별칭(`/provider_codex`)을 모두 지원

권한 정책:
- `OPEN_ACCESS=true`면 모든 사용자가 operator
- 아니면 `ALLOWLIST`와 매칭되는 사용자만 operator
  - `user_id`
  - `channel:user_id`
  - `role:*`
- DM(Discord DM / Telegram private chat)은 allowlist 사용자만 허용
- 비allowlist DM은 런타임 실행 없이 거절 응답 반환

## 8. 데이터 모델

- `provider_preferences`
  - `scope_key` PK
  - `provider_kind`
  - `mode`
  - `updated_at`

- `provider_sessions`
  - PK(`scope_key`, `provider_kind`)
  - `provider_session_id`
  - `last_used_at`
  - `metadata_json`

- `event_log` runtime 확장 컬럼
  - `provider_kind`
  - `runtime_mode`
  - `provider_latency_ms`
  - `provider_status`

## 9. 관측성

HTTP:
- `/healthz`
- `/readyz`
- `/metrics`

메트릭:
- `orka_inbound_total`
- `orka_outbound_total`
- `orka_error_total`
- `orka_provider_requests_total{provider,mode,status}`

## 10. 보안 기준

- Bot 토큰은 env 입력만 허용
- 운영 권한 명령은 allowlist/open_access 정책 적용
- `scope_key`, `session_id` 정규화/검증
- provider stderr 로그 민감 키워드 마스킹

## 11. 검증 상태 (2026-02-22)

- Discord ping inbound/outbound 확인
- Telegram ping inbound/outbound 확인
- `RUNTIME_ENGINE=cli`, `DEFAULT_PROVIDER=codex`, `mode=session` 실호출 성공
- provider 메트릭에서 `codex session success` 증가 확인

## 12. 추후 과제

- 운영 환경 표준 runbook 강화
- 장애 주입 자동화 범위 확대
- 필요 시 HTTP Control API 재배선 여부 재평가
