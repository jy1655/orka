# Architecture

기준 상태: 2026-02-23

## Workspace Layout

```text
openclaw-rs-gateway/
  Cargo.toml
  crates/
    app/
    core/
    storage-sqlite/
    adapters-discord/
    adapters-telegram/
  migrations/
  docs/
```

## Crate Responsibilities

- `crates/core`
  - 공통 도메인 타입
  - 런타임/명령 파싱
  - 메시지 파이프라인 및 정책
  - 공통 텍스트 정규화 유틸(`text`)
- `crates/storage-sqlite`
  - 세션/이벤트 저장
  - idempotency 처리
  - runtime preference/session 상태 저장
  - 마이그레이션 실행
- `crates/adapters-discord`
  - Discord Gateway 수신 이벤트 정규화
  - Discord REST outbound 전송
- `crates/adapters-telegram`
  - Telegram polling 수신 이벤트 정규화
  - Telegram REST outbound 전송
- `crates/app`
  - 설정 로드(`.env`)
  - 어댑터/스토리지/코어 배선
  - 부트스트랩/메인루프/종료 드레인 오케스트레이션
  - health/ready/metrics HTTP 서버
  - graceful shutdown + inflight drain

## Runtime Interfaces

- Inbound: Discord, Telegram
- Outbound: Discord, Telegram
- Runtime Engine:
  - `echo` (기본)
  - `cli` (`claude`/`codex`/`opencode`)

Control API 모듈 소스(`crates/app/src/control.rs`)는 저장소에 남아 있지만,
현재 `main.rs`에서 배선되지 않아 런타임 엔드포인트로 노출되지 않습니다.

## Recent Refactoring Notes

- `GatewayPipeline` 메트릭은 전역 싱글턴이 아니라 파이프라인 인스턴스 내부 상태로 관리된다.
  - `health` 엔드포인트는 해당 파이프라인의 snapshot을 렌더링한다.
- `handle_command`는 커맨드별 메서드(`cmd_status`, `cmd_provider_set` 등)로 분해되어
  변경 범위와 테스트 안정성이 개선되었다.
- Discord/Telegram 어댑터의 텍스트 정규화는 `core::text` 유틸을 공통 사용한다.

## Data Flow

1. Adapter가 채널 이벤트 수신
2. `InboundEvent`로 정규화
3. `GatewayPipeline`에서 idempotency/정책/명령 처리
4. runtime invoke (`echo` 또는 provider CLI)
5. `OutboundAction` 생성 후 채널 전송
6. inbound/outbound 및 runtime 메타를 SQLite에 기록

## Boundary Rules

- Adapter는 `core` 타입에만 의존
- `core`는 adapter 구현을 참조하지 않음
- `storage`는 저장/조회만 담당
- `app`이 의존성 주입과 수명주기 제어 담당
