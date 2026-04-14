# Failure & Edge Case Plan (Pre-Production)

## 1. 목적

실제 봇 토큰으로 운영 테스트를 진행하기 전,
장애/엣지 케이스에 대한 감지 신호와 대응 절차를 고정한다.

범위:
- Discord/Telegram adapter
- Core pipeline
- CLI runtime(`claude`/`codex`/`opencode`)
- SQLite storage

## 2. 운영 목표

- 이벤트 유실 방지(중복은 idempotency로 흡수)
- 장애 시 빠른 실패 + 안전한 사용자 응답
- 자동 복구 우선, 실패 시 수동 조치 전환

## 3. 장애 매트릭스

| ID | 시나리오 | 감지 신호 | 자동 대응 | 운영자 조치 |
|---|---|---|---|---|
| R-01 | CLI 바이너리 누락 | `failed to spawn` | 요청 실패 + 안전 오류 메시지 | `*_BIN` 경로 점검 후 재기동 |
| R-02 | CLI 응답 타임아웃 | `timed out after` | 즉시 실패 | `PROVIDER_TIMEOUT_MS`/provider 상태 점검 |
| R-03 | 세션 ID 오염/만료 | session 실패 | `SESSION_FAIL_FALLBACK_EVENT=true`면 event 1회 폴백 | 반복 시 `/session reset` 또는 provider 전환 |
| R-04 | CLI 비정상 종료 | non-zero exit | 실패 처리 + 오류 로그 | provider 로그인/권한/옵션 점검 |
| R-05 | 과도한 출력 | `output too large` | 즉시 실패 | `MAX_OUTPUT_BYTES`/프롬프트 정책 조정 |
| C-01 | Discord 토큰 불량 | 인증/연결 실패 로그 | adapter task 종료 | 토큰/권한 점검 |
| C-02 | Discord MESSAGE CONTENT 미설정 | 텍스트 미수신 | 없음 | Developer Portal intent 활성화 |
| C-03 | Telegram 네트워크 오류 | polling error | 백오프 재시도 | 네트워크/방화벽 점검 |
| C-04 | Telegram API 오류 | API status 에러 | 재시도 루프 | rate-limit 대응 |
| S-01 | SQLite 권한/잠금 오류 | store/migrate error | 요청 실패/부팅 실패 | DB 파일 권한/WAL 상태 점검 |
| P-01 | 비인가 운영 명령 | `unauthorized` 응답 | 실행 차단 | `ALLOWLIST`/`OPEN_ACCESS` 재검토 |
| P-02 | 비인가 DM 접근 | DM 거절 응답 | 런타임 미실행 | `ALLOWLIST`/DM 운영 정책 재검토 |

## 4. 현재 반영된 보호장치

- provider timeout/출력 상한 강제
- session 실패 시 event 폴백 옵션
- runtime 최종 실패 시 안전 오류 메시지 반환
- Discord/Telegram bot message 무시 + 길이 제한
- 운영 명령 권한 통제(allowlist/open_access)
- idempotency 기반 duplicate event 무시
- graceful shutdown inflight drain

## 5. 사전 리허설 체크리스트

### 5.1 Runtime/Process

- `RUNTIME_ENGINE=cli`에서 `claude`, `codex`, `opencode` 최소 1회 성공
- 낮은 `PROVIDER_TIMEOUT_MS`로 timeout 처리 확인
- 잘못된 `*_BIN`으로 spawn 실패 확인
- 낮은 `MAX_OUTPUT_BYTES`로 출력 초과 확인

### 5.2 Session/Mode

- `DEFAULT_RUNTIME_MODE=session` + `SESSION_FAIL_FALLBACK_EVENT=true`
- 잘못된 session 유도 후 event 폴백 확인
- 폴백 후 provider session 삭제 확인

### 5.3 Channel

- Discord: 텍스트 수신/응답, bot message 무시
- Telegram: polling 수신/응답, 네트워크 단절 후 백오프
- `/help` 명령과 command menu/slash command 노출 확인

### 5.4 Security/Control

- 비인가 사용자 `/provider set`, `/mode set`, `/session reset` 차단
- 비allowlist DM 차단 + allowlist DM 성공 확인
- operator 사용자 명령 성공 및 상태 반영 확인
- `OPEN_ACCESS=true`는 로컬 테스트에서만 사용

## 6. Go / No-Go

Go:
- Blocker(`R-01`, `R-02`, `R-03`, `C-01`, `S-01`) 전부 통과
- 같은 시나리오 3회 반복 시 결과 일관

No-Go:
- Blocker 1개 이상 실패
- 복구 절차가 문서대로 재현되지 않음

## 7. 자동 드릴

```bash
./scripts/pretest_blocker_drill.sh
./scripts/pretest_manual_blocker_drill.sh
```

자동 검증 커버리지:
- `R-01` CLI 바이너리 누락
- `R-02` provider timeout
- `R-03` session 실패 -> event 폴백
- `R-05` output size limit
- 최종 실패 시 안전 오류 메시지

수동 검증 필요:
- `C-01` Discord 토큰/권한 실제 인증
- `S-01` 운영 DB 권한/잠금 장애

## 8. 수동 Blocker 드릴 예시

### 8.1 C-01 Discord 토큰 불량

```bash
export DISCORD_BOT_TOKEN="invalid-token"
export TELEGRAM_BOT_TOKEN=""
export RUNTIME_ENGINE=echo
cargo run -p orka-app
```

기대 결과:
- Discord adapter에서 인증/연결 실패 로그
- 프로세스는 health endpoint 유지

### 8.2 S-01 SQLite 권한 오류

```bash
mkdir -p /tmp/orka-readonly
chmod 500 /tmp/orka-readonly
export DATABASE_URL="sqlite:///tmp/orka-readonly/orka.db"
cargo run -p orka-app
```

기대 결과:
- DB connect/migrate 단계에서 권한 오류로 실패

복구:

```bash
chmod 700 /tmp/orka-readonly
unset DATABASE_URL
```
