# Implementation Roadmap (Updated)

## Status Summary (2026-02-22)

- Phase 1 Foundation: done
- Phase 2 Core: done
- Phase 3 Discord: done
- Phase 4 Telegram: done
- Phase 5 Hardening: in progress
- Phase 6 Multi CLI Runtime: done
- Phase 7 Runtime Control: done (채널 명령 기반)
- Phase 8 Production Readiness: in progress

## Phase Details

### Phase 1: Foundation (Done)
- Rust workspace 구성
- 공통 설정/로그 초기화
- health endpoint(`/healthz`, `/readyz`)
- SQLite 마이그레이션 기반

### Phase 2: Core (Done)
- 공통 이벤트/메시지 모델
- 세션 키 규칙
- idempotency 처리
- 정책 엔진(allowlist, pause/resume)

### Phase 3: Discord (Done)
- Gateway 연결/수신 정규화
- text outbound
- bot message 필터링/입력 길이 제한

### Phase 4: Telegram (Done)
- polling 수신 정규화
- text outbound
- polling 오류 백오프
- bot message 필터링/입력 길이 제한

### Phase 5: Hardening (In Progress)
- graceful shutdown inflight drain
- runtime metadata logging(event_log 확장)
- metrics endpoint 추가
- 남은 항목
  - 장애 시나리오 자동화 확대
  - 운영 runbook 정교화

### Phase 6: Multi CLI Runtime (Done)
- provider 추상화(`claude`/`codex`/`opencode`)
- timeout/출력 제한/에러 매핑
- session/event 모드 라우팅
- provider별 JSON/text 파서

### Phase 7: Runtime Control (Done)
- scope별 provider/mode/session 저장
- 채널 운영 명령
  - `/provider list`, `/provider set`
  - `/mode set`, `/session reset`
  - `/pause`, `/resume`, `/status`
- 참고: HTTP Control API는 현재 런타임 배선에서 제외

### Phase 8: Production Readiness (In Progress)
- 실제 토큰 기반 Discord/Telegram smoke 통과
- `RUNTIME_ENGINE=cli` + `codex` session 성공 확인
- 운영 runbook 추가 (`docs/RUNBOOK.md`)
- 배포/롤백 체크리스트 추가 (`docs/DEPLOYMENT_CHECKLIST.md`)
- 남은 항목
  - 장애 주입 테스트 자동화 범위 확장
  - 운영 로그/알람 임계치 표준화
