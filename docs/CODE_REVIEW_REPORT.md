# OpenClaw RS Gateway — 코드 리뷰 개선점 리포트

> 대상: 전체 워크스페이스 (15개 소스 파일, 5개 crate)
> 리뷰 일자: 2026-02-23
> 총 소스 라인: ~3,500줄 (테스트 포함)

---

## 0. 총평

아키텍처 방향성(hexagonal, 어댑터-코어 분리)이 명확하고 도메인 모델이 깔끔하며,
입력 검증(`normalize_scope_key`, `normalize_session_id`)과 CLI 보안(`sanitize_for_log`, `kill_on_drop`, 셸 미사용) 수준이 높다.
초기 프로젝트 치고 테스트 커버리지가 양호한 편이나, 아래 개선점을 해결하면 운영 안정성과 유지보수성이 크게 향상될 것이다.

### 발견 분류 요약

| 심각도 | 건수 | 주요 영역 |
|--------|------|----------|
| CRITICAL | 2 | 보안, 에러 체인 소실 |
| HIGH | 6 | 코드 중복, 전역 상태, 테스트 커버리지, 설정 검증 |
| MEDIUM | 7 | 타입 시스템, 아키텍처, 메트릭스 |
| LOW | 4 | 코드 정리, 컨벤션 |

---

## 1. CRITICAL

### 1.1 에러 체인 소실 — Telegram 어댑터

**위치**: `crates/adapters-telegram/src/lib.rs`

| 라인 | 코드 |
|------|------|
| 74 | `.map_err(\|_\| anyhow!("telegram getUpdates request failed"))` |
| 79 | `.map_err(\|_\| anyhow!("telegram getUpdates response read failed"))` |
| 85 | `.map_err(\|_\| anyhow!("telegram getUpdates payload decode failed"))` |
| 147 | `.map_err(\|_\| anyhow!("telegram sendMessage request failed"))` |
| 152 | `.map_err(\|_\| anyhow!("telegram sendMessage response read failed"))` |
| 158 | `.map_err(\|_\| anyhow!("telegram sendMessage payload decode failed"))` |

**문제**: 원본 `reqwest::Error` / `serde_json::Error`가 완전히 버려진다. 프로덕션에서 네트워크 장애, TLS 핸드셰이크 실패, JSON 파싱 에러 등의 근본 원인을 추적할 수 없다.

**개선**:
```rust
// Before
.map_err(|_| anyhow!("telegram getUpdates request failed"))

// After
.context("telegram getUpdates request failed")?
```

`anyhow::Context` 트레잇의 `.context()`를 사용하면 원본 에러가 체인에 보존된다.

---

### 1.2 CLI 런타임의 사용자 입력 직접 전달

**위치**: `crates/app/src/runtime.rs:141`

```rust
args.push(request.event.text.clone());
```

**분석**:
- `Command::new().args()` 사용으로 **셸 인젝션은 차단**됨 (셸을 거치지 않음)
- 그러나 사용자 메시지가 CLI 인자로 **그대로** 전달됨
- `--` 같은 값이 플래그로 해석될 가능성이 있음 (예: 사용자가 `--help` 입력)
- provider CLI가 내부적으로 인자를 셸에 전달하는 경우 2차 인젝션 위험

**개선**: 사용자 텍스트 앞에 `--` 인자 구분자를 삽입하여 positional argument로 강제:

```rust
args.push("--".to_string());
args.push(request.event.text.clone());
```

또는 stdin pipe로 전달 방식 전환을 검토.

---

## 2. HIGH

### 2.1 Operator 권한 검사 5회 반복

**위치**: `crates/core/src/pipeline.rs`

| 커맨드 | 라인 범위 |
|--------|----------|
| `ProviderSet` | 336–346 |
| `ModeSet` | 369–379 |
| `SessionReset` | 402–412 |
| `Pause` | 426–436 |
| `Resume` | 449–459 |

**문제**: 아래 패턴이 **동일한 코드**로 5번 반복된다:

```rust
if !self.policy.is_operator(event.channel, &event.user_id) {
    self.dispatch(OutboundAction {
        channel: event.channel,
        chat_id: event.chat_id.clone(),
        text: "unauthorized: operator only command".to_string(),
    }, None).await?;
    return Ok(());
}
```

**개선**:

```rust
async fn require_operator(&self, event: &InboundEvent) -> Result<bool> {
    if self.policy.is_operator(event.channel, &event.user_id) {
        return Ok(true);
    }
    self.dispatch(OutboundAction {
        channel: event.channel,
        chat_id: event.chat_id.clone(),
        text: "unauthorized: operator only command".to_string(),
    }, None).await?;
    Ok(false)
}
```

그러면 각 커맨드 핸들러는:

```rust
Command::Pause => {
    if !self.require_operator(event).await? { return Ok(()) }
    // ...
}
```

---

### 2.2 `normalize_inbound_text` / `normalize_outbound_text` 어댑터 간 중복

**위치**:
- `crates/adapters-discord/src/lib.rs:125-143`
- `crates/adapters-telegram/src/lib.rs:241-255`

**문제**: 동일한 trim + truncate 로직이 양쪽 어댑터에 복사되어 있다. 상수값만 다름 (Discord: 1900자, Telegram: 3500자).

**개선**: `core` 크레이트에 공용 함수를 만들고 max_chars를 파라미터로 받기:

```rust
// core/src/model.rs (또는 별도 text 모듈)
pub fn normalize_text(raw: &str, max_chars: usize) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() { return None; }
    Some(trimmed.chars().take(max_chars).collect())
}
```

---

### 2.3 테스트 MockStore / MemoryStore 2회 중복

**위치**:
- `crates/core/src/pipeline.rs` 테스트 모듈 (576–689행, ~113줄)
- `crates/app/src/control.rs` 테스트 모듈 (338–447행, ~109줄)

**문제**: `EventStore` 트레잇의 테스트용 인메모리 구현이 거의 동일한 코드로 두 번 작성되어 있다.

**개선**:
- `crates/core`에 `#[cfg(test)]` 전용 `pub mod test_fixtures` 모듈을 만들어 `MemoryStore`를 한 번만 정의
- 또는 별도 테스트 유틸 크레이트 생성

---

### 2.4 전역 싱글턴 메트릭스 상태

**위치**: `crates/core/src/pipeline.rs:41-56`

```rust
fn gateway_metrics_store() -> &'static Mutex<GatewayMetrics> {
    static STORE: OnceLock<Mutex<GatewayMetrics>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(GatewayMetrics::default()))
}
```

**문제**:
1. `std::sync::Mutex` 전역 싱글턴 — 파이프라인 인스턴스와 무관한 프로세스 전역 상태
2. 테스트 격리 불가 — 병렬 테스트가 동일한 메트릭스 카운터를 공유
3. 다중 파이프라인 시나리오 지원 불가
4. 리셋 함수 없음

**개선**: 메트릭스를 `GatewayPipeline` 인스턴스 멤버로 이동:

```rust
pub struct GatewayPipeline {
    // ...
    metrics: Arc<Mutex<GatewayMetrics>>,
}
```

외부 노출은 `pipeline.metrics_snapshot()` 메서드로 제공.

---

### 2.5 주요 모듈 테스트 부재

| 모듈 | 테스트 유무 | 위험도 |
|------|-----------|--------|
| `core/src/model.rs` | ✅ 있음 | - |
| `core/src/pipeline.rs` | ✅ 있음 (7개) | - |
| `core/src/session.rs` | ❌ 없음 | session_key 생성 로직 |
| `core/src/policy.rs` | ❌ 없음 | 권한 정책 — 보안 핵심 |
| `app/src/config.rs` | ❌ 없음 | 환경변수 파싱 — 잘못된 설정 = 운영 장애 |
| `app/src/health.rs` | ❌ 없음 | 메트릭스 출력 형식 |
| `app/src/outbound.rs` | ❌ 없음 | 채널 라우팅 |
| `app/src/main.rs` | ❌ 없음 | 셧다운/드레인 로직 |
| `app/src/runtime.rs` | ✅ 있음 (5개) | - |
| `app/src/control.rs` | ✅ 있음 (5개) | - |
| `adapters-discord/src/lib.rs` | ✅ 있음 (3개) | - |
| `adapters-telegram/src/lib.rs` | ✅ 있음 (3개) | - |
| `storage-sqlite/src/lib.rs` | ✅ 있음 (2개) | - |

**특히 `policy.rs`는 보안 핵심 모듈인데 테스트가 없다.** `is_operator()`의 open_access, scoped allowlist, unscoped allowlist 등 경계 케이스 검증 필요.

---

### 2.6 환경변수 설정 실패 시 경고 없는 기본값 적용

**위치**: `crates/app/src/config.rs:118-143`

```rust
fn parse_provider_env(name: &str, default: ProviderKind) -> ProviderKind {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<ProviderKind>().ok())
        .unwrap_or(default)
}
```

**문제**: `DEFAULT_PROVIDER=claud` (오타)가 조용히 `Claude`로 폴백된다. 운영자가 오타를 인지할 수 없다.

**개선**: 값이 존재하지만 파싱에 실패하면 `warn!` 로그 출력:

```rust
fn parse_provider_env(name: &str, default: ProviderKind) -> ProviderKind {
    match env::var(name) {
        Ok(value) => match value.parse::<ProviderKind>() {
            Ok(provider) => provider,
            Err(_) => {
                warn!(env = name, value = %value, "unrecognized value; using default");
                default
            }
        }
        Err(_) => default,
    }
}
```

---

## 3. MEDIUM

### 3.1 `handle_event`의 복잡한 재시도 로직 — 메서드 분리 필요

**위치**: `crates/core/src/pipeline.rs:123-287` (165줄)

**문제**: `handle_event`가 아래 책임을 모두 갖고 있다:
1. 중복 이벤트 검사
2. scope_key 정규화
3. 커맨드 파싱/분기
4. 일시정지 상태 검사
5. 런타임 preference 조회
6. 세션 ID 조회
7. 런타임 호출 + 실패 시 fallback 재시도
8. 세션 ID 저장
9. 아웃바운드 디스패치

**개선**: 최소한 재시도 로직을 별도 메서드로 추출:

```rust
async fn invoke_with_fallback(
    &self,
    request: RuntimeInvokeRequest,
    scope_key: &str,
    preference: RuntimePreference,
) -> Result<(RuntimeInvokeResponse, RuntimeMode)>
```

---

### 3.2 `handle_command`의 반복적 구조 (183줄)

**위치**: `crates/core/src/pipeline.rs:289-476`

**문제**: 7개 커맨드 핸들러가 순차적 match arm으로 나열되어 있어 단일 메서드가 183줄이다.

**개선**: 각 커맨드를 개별 메서드로 분리하고 match에서 위임:

```rust
async fn handle_command(&self, command: Command, event: &InboundEvent, scope_key: &str) -> Result<()> {
    match command {
        Command::Status => self.cmd_status(event, scope_key).await,
        Command::ProviderList => self.cmd_provider_list(event, scope_key).await,
        // ...
    }
}
```

---

### 3.3 도메인 ID에 Newtype 패턴 미적용

**위치**: `crates/core/src/model.rs`

| 필드 | 현재 타입 | 개선 제안 |
|------|----------|----------|
| `chat_id` | `String` | `ChatId(String)` |
| `user_id` | `String` | `UserId(String)` |
| `idempotency_key` | `String` | `IdempotencyKey(String)` |
| `scope_key` | `String` | `ScopeKey(String)` |

**이유**: 현재 `chat_id`와 `user_id`를 컴파일 타임에 혼동할 수 있다. Newtype 래퍼로 컴파일 타임 안전성 확보.

---

### 3.4 `Display` 트레잇 미구현

**위치**: `Channel`, `ProviderKind`, `RuntimeMode` 모두

**문제**:
- `FromStr`은 구현되어 있으나 `Display`는 없음 (비대칭)
- `format!("{}", provider)` 불가 → 매번 `.as_str()` 호출 필요
- `anyhow!("unsupported provider: {other}")` 에서 이미 문자열로 변환하는 boilerplate 존재

**개선**: `Display` 구현 추가 (또는 `strum` 크레이트 활용):

```rust
impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
```

---

### 3.5 Dead Code — `control.rs` 미연결

**위치**: `crates/app/src/control.rs` (586줄)

**문제**:
- `main.rs`에서 `mod control;`이 **선언되지 않았다**
- 파일은 존재하지만 컴파일 대상에 포함되지 않음
- 컴파일 에러가 있어도 감지 불가
- 약 586줄의 관리되지 않는 코드

**개선 방안**:
1. **활성화 예정이면**: `mod control;`을 추가하되 feature flag 뒤에 배치: `#[cfg(feature = "control-api")] mod control;`
2. **폐기 예정이면**: 파일 삭제 또는 `_archived/` 디렉토리로 이동

---

### 3.6 Prometheus 메트릭스 수동 포맷팅

**위치**: `crates/app/src/health.rs:62-94`

**문제**: Prometheus exposition format을 `push_str` + `format!`으로 수동 생성. HELP 행 누락, 히스토그램/게이지 미지원, OpenMetrics 호환성 미검증.

**개선**: `prometheus` 또는 `metrics` + `metrics-exporter-prometheus` 크레이트 사용.

---

### 3.7 `DiscordOutbound`에 불필요한 `token` 필드

**위치**: `crates/adapters-discord/src/lib.rs:95-104`

```rust
pub struct DiscordOutbound {
    token: String,     // ← 빈 토큰 검사(line 113)에만 사용
    http: Arc<Http>,   // ← Http 내부에 이미 토큰 보관
}
```

**문제**: `token` 필드가 빈 토큰 검사에만 사용된다. `Http`가 이미 내부적으로 토큰을 저장하므로 중복 보관이다.

**개선**: 토큰 유효성 검사를 생성자에서 수행하고, 런타임에는 `http` 필드만 사용:

```rust
impl DiscordOutbound {
    pub fn new(token: String) -> Result<Self> {
        if token.trim().is_empty() {
            bail!("discord outbound requires DISCORD_BOT_TOKEN");
        }
        let http = Arc::new(Http::new(&token));
        Ok(Self { http })
    }
}
```

---

## 4. LOW

### 4.1 하드코딩된 운영 파라미터

| 위치 | 값 | 설명 |
|------|---|------|
| `adapters-discord/src/lib.rs:17-18` | `MAX_INBOUND_CHARS=4000`, `MAX_OUTBOUND_CHARS=1900` | 채널별 문자 제한 |
| `adapters-telegram/src/lib.rs:15-18` | `POLL_TIMEOUT_SECS=30`, `MAX_BACKOFF_SECS=30` | 폴링 타이밍 |
| `app/src/main.rs:68` | `mpsc::channel(1024)` | 인바운드 버퍼 크기 |
| `storage-sqlite/src/lib.rs:36` | `max_connections(5)` | SQLite 풀 크기 |

이들은 운영 환경에 따라 조정이 필요할 수 있다. 환경변수로 노출하거나, 최소한 `AppConfig`에서 관리하도록 이동 권장.

---

### 4.2 `storage-sqlite/src/lib.rs:31` — Async 컨텍스트 내 블로킹 `std::fs` 호출

```rust
fs::create_dir_all(parent)?;
```

**문제**: `connect()`가 async 함수이므로 `tokio::fs::create_dir_all`을 사용해야 한다.
**실질 영향**: 서버 시작 시 1회만 호출되므로 영향은 미미하나 원칙적으로 `tokio::fs` 사용이 적합.

---

### 4.3 CI/CD 파이프라인 부재

- `.github/workflows/`, `.gitlab-ci.yml` 등 CI 설정 파일 없음
- `cargo clippy`, `cargo fmt --check`, `cargo test` 자동 실행 미보장
- 코드 품질 게이트 없이 코드가 병합될 위험

**권장**: 최소한 아래를 포함하는 CI 설정:

```yaml
# .github/workflows/ci.yml (예시)
- cargo fmt --all --check
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test --all
```

---

### 4.4 `.gitignore`에 `data/` 디렉토리 누락

**현재**:
```
/target
/.env
/.env.local
/.DS_Store
```

**문제**: SQLite 데이터베이스가 `data/` 디렉토리에 생성되는데 `.gitignore`에 포함되지 않았다. 실수로 DB 파일이 커밋될 수 있다.

**개선**: `/data/` 추가.

---

## 5. 아키텍처 검증 결과

### 의존성 방향 (검증 통과 ✅)

```
app → core, storage-sqlite, adapters-discord, adapters-telegram
adapters-discord → core (only)
adapters-telegram → core (only)
storage-sqlite → core (only)
core → (external crates only, no internal deps)
```

- ✅ Adapter는 core 타입에만 의존
- ✅ Core는 adapter 구현을 참조하지 않음
- ✅ Storage는 저장/조회만 담당
- ✅ App이 의존성 주입과 수명주기 제어

### 트레잇 추상화 (양호 ✅)

| 트레잇 | 위치 | 구현체 수 | 평가 |
|--------|------|----------|------|
| `EventStore` | `core/ports.rs` | 1 운영 + 2 테스트 | ✅ 적절 |
| `OutboundSender` | `core/ports.rs` | 3 (discord, telegram, multiplex) + 1 테스트 | ✅ 적절 |
| `AgentRuntime` | `core/ports.rs` | 3 (echo, cli) + 3 테스트 | ✅ 적절 |

---

## 6. 우선순위별 실행 로드맵

### Phase 1 — 즉시 (1-2일)

| # | 항목 | 난이도 | 영향 |
|---|------|-------|------|
| 1 | 1.1 에러 체인 복원 (`.context()` 변환) | 🟢 Easy | 운영 디버깅 능력 |
| 2 | 1.2 CLI 인자에 `--` 구분자 추가 | 🟢 Easy | 보안 |
| 3 | 2.6 설정 파싱 실패 시 warn 로그 | 🟢 Easy | 운영 안정성 |
| 4 | 4.4 `.gitignore`에 `/data/` 추가 | 🟢 Easy | 실수 방지 |

### Phase 2 — 단기 (3-5일)

| # | 항목 | 난이도 | 영향 |
|---|------|-------|------|
| 5 | 2.1 operator 권한 검사 추출 | 🟡 Medium | 코드 품질 |
| 6 | 2.2 텍스트 정규화 공용화 | 🟡 Medium | 코드 중복 제거 |
| 7 | 2.5 `policy.rs`, `config.rs` 테스트 추가 | 🟡 Medium | 테스트 커버리지 |
| 8 | 3.5 `control.rs` 정리 (feature flag 또는 삭제) | 🟢 Easy | 데드 코드 제거 |
| 9 | 4.3 CI 파이프라인 구성 | 🟡 Medium | 품질 게이트 |

### Phase 3 — 중기 (1-2주)

| # | 항목 | 난이도 | 영향 |
|---|------|-------|------|
| 10 | 2.3 테스트 MockStore 공용화 | 🟡 Medium | 테스트 유지보수성 |
| 11 | 2.4 메트릭스를 파이프라인 인스턴스 스코프로 이동 | 🔴 Hard | 테스트 격리, 확장성 |
| 12 | 3.1-3.2 handle_event/handle_command 리팩토링 | 🟡 Medium | 가독성 |
| 13 | 3.4 `Display` 트레잇 추가 | 🟢 Easy | 코드 일관성 |
| 14 | 3.6 Prometheus 크레이트 도입 | 🟡 Medium | 메트릭스 정합성 |

### Phase 4 — 장기 (검토 후 결정)

| # | 항목 | 난이도 | 영향 |
|---|------|-------|------|
| 15 | 3.3 Newtype 패턴 도입 | 🔴 Hard | 타입 안전성 (파급 범위 큼) |
| 16 | 4.1 하드코딩 상수 설정화 | 🟡 Medium | 운영 유연성 |
| 17 | 통합 테스트 추가 (end-to-end) | 🔴 Hard | 전체 파이프라인 검증 |

---

## 7. 긍정적 관찰 사항 (잘 된 점)

1. **입력 검증 견고함**: `normalize_scope_key`, `normalize_session_id`가 길이·문자·포맷을 모두 검증. path traversal 차단 테스트(`../../etc/passwd`) 포함.
2. **CLI 보안**: 셸을 거치지 않는 `Command::new().args()`, `kill_on_drop(true)`, 출력 크기 제한, 타임아웃, 로그 민감 정보 마스킹(`sanitize_for_log`).
3. **Graceful Shutdown**: `ctrl_c` 시그널 핸들링, 어댑터 abort, inflight 큐 드레인(타임아웃 포함).
4. **Idempotency**: DB 기반 중복 이벤트 방지 + `INSERT OR IGNORE`.
5. **Session Fallback**: 세션 모드 실패 시 이벤트 모드로 자동 전환하는 resilience 패턴.
6. **테스트 품질**: 파이프라인 테스트가 다양한 시나리오(중복 이벤트, 세션 재사용, 권한 검사, fallback, 런타임 실패)를 커버.
7. **아키텍처 준수**: 선언된 경계 규칙(adapter → core only)이 실제 의존성에서 정확히 지켜지고 있음.
8. **Mutex poisoning 처리**: `pipeline.rs:53` — `poisoned.into_inner()`로 poisoned mutex를 안전하게 복구.

---

*끝.*
