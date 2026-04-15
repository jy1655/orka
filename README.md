# Orka

**Discord나 Telegram 채팅으로 AI CLI 도구를 원격 실행하는 게이트웨이**

채팅 메시지를 받아서 로컬 머신에 설치된 AI CLI(`claude`, `codex`, `opencode`)를 실행하고, 결과를 다시 채팅으로 돌려줍니다. 서버 한 대에 띄워두면 어디서든 Discord/Telegram으로 AI 에이전트에 접근할 수 있습니다.

---

## 이런 분에게 유용합니다

- Discord에서 Codex CLI를 원격으로 돌리고 싶은 분
- Claude CLI를 팀원들과 공유 채널에서 사용하고 싶은 분
- 여러 AI provider를 채팅 명령 하나로 전환하며 쓰고 싶은 분
- Telegram으로 서버의 AI 에이전트에 접근하고 싶은 분

---

## 어떻게 동작하나요?

```
사용자 (Discord/Telegram)
    |
    |  메시지 전송
    v
+-----------------------+
|   Orka Gateway    |
|                       |
|  1. 메시지 수신       |
|  2. 권한 확인         |
|  3. AI CLI 실행       |  <-- claude / codex / opencode
|  4. 결과 응답         |
+-----------------------+
    |
    |  응답 전송
    v
사용자 (Discord/Telegram)
```

**핵심 흐름:**

1. 사용자가 Discord에서 `/ask prompt:이 코드 설명해줘` 입력
2. 게이트웨이가 로컬의 `claude` CLI를 실행하고 프롬프트 전달
3. CLI 출력을 파싱해서 Discord 채팅으로 응답

**지원하는 AI Provider:**

| Provider | CLI 도구 | 설명 |
|---|---|---|
| `claude` | [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) | Anthropic의 Claude |
| `codex` | [Codex CLI](https://github.com/openai/codex) | OpenAI의 Codex |
| `opencode` | [OpenCode CLI](https://github.com/nichochar/opencode) | 오픈소스 AI CLI |

채팅에서 `/provider set codex` 명령으로 provider를 즉시 전환할 수 있습니다.

---

## 빠른 시작 (5분)

### 사전 준비

- **Rust** 1.75+ (설치: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- **Discord 봇 토큰** 또는 **Telegram 봇 토큰** (아래 설정 가이드 참고)
- **AI CLI 도구** 중 하나 이상 설치 (`claude`, `codex`, `opencode`)

### Step 1: 프로젝트 클론 및 온보딩

```bash
git clone <repository-url>
cd cli-rs-gateway-DT
cargo run -p orka-app -- onboard
```

`onboard`는 `.env.example`을 기준으로 `.env`를 만들고, PATH에서 발견한 `claude`/`codex`/`opencode` 바이너리를 자동으로 채워 넣습니다.

이미 `.env`가 있으면 덮어쓰지 않습니다. 다시 생성하려면:

```bash
cargo run -p orka-app -- onboard --force
```

### Step 2: `.env` 파일 편집

에디터로 `.env`를 열고 최소 설정을 입력합니다:

```env
# Discord 봇 토큰 (Discord를 사용하는 경우)
DISCORD_BOT_TOKEN=your_discord_bot_token_here

# 실제 AI CLI를 실행하도록 설정
RUNTIME_ENGINE=cli

# 사용할 AI provider (claude, codex, opencode 중 택 1)
DEFAULT_PROVIDER=claude

# 로컬 테스트: 모든 사용자에게 모든 명령 허용
OPEN_ACCESS=true
```

> **`RUNTIME_ENGINE=echo`** 로 설정하면 AI를 실행하지 않고 입력을 그대로 돌려줍니다. 봇 연동만 먼저 테스트할 때 유용합니다.

### Step 3: 진단 실행

```bash
cargo run -p orka-app -- doctor
```

문제가 없으면 `0 error(s)`로 끝납니다. 더 자세한 현재 상태는 아래 명령으로 볼 수 있습니다:

```bash
cargo run -p orka-app -- status --deep
```

### Step 4: 빌드 및 실행

```bash
cargo run -p orka-app
```

`orka-app`은 현재 디렉토리 또는 상위 디렉토리의 `.env`를 자동으로 로드합니다. 별도로 `source .env` 할 필요가 없습니다.

다음 로그가 보이면 성공입니다:

```
{"level":"INFO","message":"booting orka-gateway"}
{"level":"INFO","message":"health server listening on 127.0.0.1:8787"}
{"level":"INFO","message":"discord adapter started (gateway mode)"}
{"level":"INFO","message":"gateway ready"}
```

### Step 5: 테스트

Discord 또는 Telegram에서 `/help`를 실행해 명령 목록이 보이는지 확인한 뒤, 메시지 1건 또는 Discord의 `/ask prompt:안녕하세요`를 실행합니다.

### 로컬 운영 CLI

```bash
# .env 생성/갱신 보조
cargo run -p orka-app -- onboard

# 실행 전 점검
cargo run -p orka-app -- doctor

# 현재 설정/바이너리/DB 상태 상세 보기
cargo run -p orka-app -- status --deep
```

---

## Discord 봇 설정 가이드

### 1단계: Discord Application 생성

1. [Discord Developer Portal](https://discord.com/developers/applications)에 접속
2. 우측 상단 **New Application** 클릭
3. 이름 입력 (예: `My AI Gateway`) 후 **Create**

### 2단계: 봇 토큰 발급

1. 왼쪽 메뉴에서 **Bot** 클릭
2. **Reset Token** 버튼 클릭 후 토큰을 복사
3. `.env` 파일의 `DISCORD_BOT_TOKEN=` 뒤에 붙여넣기

> **주의:** 토큰은 비밀번호와 같습니다. 절대 git에 커밋하거나 공개하지 마세요.

### 3단계: 봇 권한 설정 (Intents)

Bot 페이지 하단의 **Privileged Gateway Intents**에서 다음을 활성화합니다:

- **MESSAGE CONTENT INTENT** (필수 — 메시지 내용을 읽기 위해)
- **SERVER MEMBERS INTENT** (역할 기반 권한을 사용하는 경우)

### 4단계: 봇을 서버에 초대

1. 왼쪽 메뉴에서 **OAuth2** > **URL Generator** 클릭
2. **SCOPES**에서 체크:
   - `bot`
   - `applications.commands`
3. **BOT PERMISSIONS**에서 체크:
   - `Send Messages`
   - `Read Message History`
   - `Use Slash Commands`
   - `Create Public Threads` (긴 응답의 스레드 분할을 원하는 경우)
   - `Embed Links` (Embed 형식 응답을 원하는 경우)
4. 하단에 생성된 URL을 브라우저에 붙여넣기
5. 봇을 초대할 서버를 선택하고 **승인**

---

## Telegram 봇 설정 가이드

1. Telegram에서 [@BotFather](https://t.me/BotFather)에게 `/newbot` 명령 전송
2. 봇 이름과 username 입력
3. 발급된 토큰을 `.env`의 `TELEGRAM_BOT_TOKEN=`에 입력
4. 게이트웨이를 실행하면 자동으로 Telegram polling이 시작됩니다

> Discord와 Telegram을 동시에 사용할 수 있습니다. 두 토큰을 모두 설정하면 됩니다.

---

## 사용법

### Discord에서 AI와 대화하기

**방법 1 — 슬래시 커맨드 (권장):**

```
/ask prompt:이 프로젝트의 구조를 설명해줘
```

- 입력 즉시 "Processing..." 표시
- 처리 중 5초마다 경과 시간 업데이트 (예: "Processing... (10s)")
- 완료되면 AI 응답으로 교체

**방법 2 — 일반 메시지:**

봇이 있는 채널에 그냥 메시지를 보내면 자동으로 응답합니다.

```
사용자:  src/main.rs 파일을 읽고 주요 함수를 설명해줘
봇:      main() 함수는 다음과 같은 역할을 합니다...
```

### Telegram에서 AI와 대화하기

- 일반 메시지를 보내면 바로 응답합니다.
- 명령 목록은 Telegram command menu와 `/help`에서 확인할 수 있습니다.
- operator 전용 명령은 allowlist 사용자에게만 적용됩니다.

### 대화 모드 이해하기

| 모드 | 설명 | 언제 쓰나요? |
|---|---|---|
| `session` | 이전 대화를 기억합니다 | 연속 대화, 코드 리뷰 등 맥락이 필요할 때 |
| `event` | 매 메시지를 독립적으로 처리합니다 | 단발 질문, 서로 관련 없는 요청 |

기본값은 `session`이며, 채팅에서 변경할 수 있습니다:

```
/mode set event     # 단발 모드로 변경
/mode set session   # 대화 유지 모드로 복귀
```

### 긴 응답 처리

AI 응답이 Discord 메시지 길이 제한(2000자)을 초과하면:

1. 첫 번째 부분을 원래 메시지에 표시
2. 나머지는 자동으로 **스레드를 생성**해서 이어서 전송
3. 스레드 생성이 불가능한 경우 **후속 메시지**로 분할 전송

---

## 채널 명령어

채팅창에서 텍스트로 입력할 수 있는 명령입니다. `Discord`와 `Telegram`에서는 `/help`로 바로 확인할 수 있고, command menu/slash command에도 노출됩니다.

### 누구나 사용 가능

```
/help              사용 가능한 명령을 보여줍니다
/status           현재 상태를 보여줍니다
                  (scope, provider, mode, session 상태)

/provider list    사용 가능한 provider 목록을 보여줍니다
                  (claude, codex, opencode + 현재 설정)
/provider_list    위와 동일 (menu/slash command 별칭)
```

**`/status` 응답 예시:**
```
status: active · scope=discord:<chat_id>:<user_id> · provider=claude · mode=session · session=active
```

### 사용자 명령

```
/new                    현재 사용자 scope의 AI 세션만 새로 시작합니다
                        (대화 맥락이 꼬였을 때 사용)
```

### Operator 전용 명령

다음 명령은 **operator 권한**이 필요합니다. (권한 설정은 아래 RBAC 섹션 참고)

```
/provider set claude    AI provider를 claude로 변경
/provider set codex     AI provider를 codex로 변경
/provider set opencode  AI provider를 opencode로 변경

/provider_claude        위와 동일 (menu/slash command 별칭)
/provider_codex         위와 동일 (menu/slash command 별칭)
/provider_opencode      위와 동일 (menu/slash command 별칭)

/mode set session       대화 유지 모드로 변경
/mode set event         단발 처리 모드로 변경

/mode_session           위와 동일 (menu/slash command 별칭)
/mode_event             위와 동일 (menu/slash command 별칭)

/session reset          현재 사용자 scope의 모든 세션을 초기화합니다
                        (대화 맥락이 꼬였을 때 사용)
/session_reset          위와 동일 (menu/slash command 별칭)

/pause                  현재 사용자 scope의 AI 응답을 일시 중지합니다
/resume                 일시 중지를 해제합니다

/audit                  최근 10건의 이벤트 로그를 조회합니다
/audit 30               최근 30건의 이벤트 로그를 조회합니다 (최대 50)

/envvars                현재 프로세스의 런타임 설정 요약을 조회합니다
                        (토큰 값은 숨기고 configured/missing만 표시)
```

---

## 권한 관리 (RBAC)

게이트웨이는 **누가 operator 명령을 실행할 수 있는지**를 제어합니다. 채널 내 일반 AI 대화는 누구나 가능하지만, provider 변경/일시정지 등의 운영 명령은 operator만 가능합니다. 또한 **DM(Discord DM / Telegram 개인 대화)은 allowlist된 operator만 사용할 수 있습니다.**

### 기본 설정

```env
# 방법 1: 모든 사용자에게 operator 권한 부여 (테스트/개인 사용)
OPEN_ACCESS=true

# 방법 2: 특정 사용자만 operator로 지정 (운영 환경 권장)
OPEN_ACCESS=false
ALLOWLIST=discord:<your_discord_user_id>
```

### ALLOWLIST 형식 상세

쉼표(`,`)로 구분하여 여러 항목을 지정할 수 있습니다:

**사용자 ID 직접 지정:**
```env
# 모든 채널에서 operator
ALLOWLIST=<your_user_id>

# 특정 채널에서만 operator
ALLOWLIST=discord:<your_discord_user_id>
ALLOWLIST=telegram:<your_telegram_user_id>
```

**Discord 역할 기반:**
```env
# 특정 Discord 역할을 가진 모든 사용자에게 operator 부여
ALLOWLIST=discord:role:<your_discord_role_id>

# 모든 채널에서 해당 역할이면 operator
ALLOWLIST=role:admin
```

**여러 항목 혼합:**
```env
ALLOWLIST=discord:<user_a>,discord:role:<ops_role_id>,telegram:<user_b>
```

DM 정책:
- allowlist에 없는 사용자가 DM으로 메시지를 보내면 작업을 실행하지 않고 거절 응답을 보냅니다.
- 서버/그룹 채널 메시지는 기존처럼 일반 사용자도 사용할 수 있습니다.

### Discord 사용자 ID / 역할 ID 확인 방법

1. Discord > 사용자 설정 > 앱 설정 > **고급** > **개발자 모드** 활성화
2. **사용자 ID**: 사용자를 우클릭 > "사용자 ID 복사"
3. **역할 ID**: 서버 설정 > 역할 > 원하는 역할 우클릭 > "역할 ID 복사"

---

## 실전 설정 예제

### 예제 1: 개인용 Codex 원격 실행

집 서버에 Codex를 설치해두고 Discord에서 원격으로 사용:

```env
DISCORD_BOT_TOKEN=your_discord_bot_token
RUNTIME_ENGINE=cli
DEFAULT_PROVIDER=codex
DEFAULT_RUNTIME_MODE=session
SESSION_FAIL_FALLBACK_EVENT=true

OPEN_ACCESS=false
ALLOWLIST=discord:<your_discord_user_id>    # 본인 Discord ID
```

```
Discord 채팅:
  나:    /ask prompt:현재 디렉토리의 파일 목록 보여줘
  봇:    Processing... (5s)
  봇:    src/
         ├── main.rs
         ├── lib.rs
         └── config.rs
         ...

  나:    /ask prompt:main.rs에서 에러 처리 부분을 개선해줘
  봇:    (이전 대화 컨텍스트를 기억한 채 응답)
```

### 예제 2: 팀 공유 채널에서 여러 AI 사용

팀원들이 Discord 서버에서 Claude/Codex를 공유:

```env
DISCORD_BOT_TOKEN=your_discord_bot_token
RUNTIME_ENGINE=cli
DEFAULT_PROVIDER=claude
DEFAULT_RUNTIME_MODE=event       # 팀원들이 섞여서 쓰므로 단발 모드

OPEN_ACCESS=false
ALLOWLIST=discord:role:<your_discord_role_id>    # "AI-Operator" 역할을 가진 사람만 설정 변경 가능

MAX_CONCURRENT_TASKS=4           # 동시 4건까지 처리
RATE_LIMIT_MAX_REQUESTS=5        # 채팅방당 분당 5건 제한
RATE_LIMIT_WINDOW_SECS=60
```

```
Discord 채팅:
  팀원A: /ask prompt:이 PR의 코드를 리뷰해줘
  봇:    [claude 응답]

  팀원B: /ask prompt:Python으로 정렬 알고리즘 작성해줘
  봇:    [claude 응답]

  운영자: /provider set codex
  봇:    provider set: codex

  팀원C: /ask prompt:이 버그 원인을 찾아줘
  봇:    [codex 응답]
```

### 예제 3: Discord + Telegram 동시 운영

```env
DISCORD_BOT_TOKEN=your_discord_bot_token
TELEGRAM_BOT_TOKEN=your_telegram_bot_token

RUNTIME_ENGINE=cli
DEFAULT_PROVIDER=claude
OPEN_ACCESS=false
ALLOWLIST=discord:<discord_user_id>,telegram:<telegram_user_id>
```

각 채널은 독립적인 scope로 운영되고, 공유 채널 안에서도 사용자별 scope가 분리됩니다. Discord에서 `/provider set codex`를 실행해도 Telegram 채팅에는 영향 없고, 같은 채널의 다른 사용자 세션도 바뀌지 않습니다.

Discord/Telegram의 DM scope는 allowlist 사용자에게만 열립니다.

---

## 환경변수 전체 레퍼런스

### 채널 토큰

| 변수 | 기본값 | 설명 |
|---|---|---|
| `DISCORD_BOT_TOKEN` | (빈값=비활성) | Discord 봇 토큰. 빈 값이면 Discord 어댑터가 비활성화됩니다. |
| `TELEGRAM_BOT_TOKEN` | (빈값=비활성) | Telegram 봇 토큰. 빈 값이면 Telegram 어댑터가 비활성화됩니다. |

### 스토리지

| 변수 | 기본값 | 설명 |
|---|---|---|
| `DATABASE_URL` | `sqlite://data/orka-gateway.db` | SQLite 데이터베이스 파일 경로. 이벤트 로그, 세션, 설정이 저장됩니다. 경로의 디렉토리는 자동 생성됩니다. |
| `HEALTH_BIND` | `127.0.0.1:8787` | Health/Metrics HTTP 서버가 바인딩할 주소. Prometheus 스크래핑이나 로드밸런서 헬스체크에 사용합니다. |
| `STORE_FULL_PAYLOADS` | `false` | `false`이면 SQLite `event_log.payload_text`에 원문 대신 redacted placeholder만 저장합니다. 운영 환경에서는 기본값 유지가 권장됩니다. |

### 접근 정책

| 변수 | 기본값 | 설명 |
|---|---|---|
| `OPEN_ACCESS` | `false` | `true`이면 모든 사용자가 operator 명령과 DM 대화를 사용할 수 있습니다. 개인 사용이나 테스트에 적합합니다. |
| `ALLOWLIST` | (빈값) | operator로 허용할 사용자/역할 목록. 쉼표로 구분합니다. 형식은 위의 RBAC 섹션을 참고하세요. `OPEN_ACCESS=false`일 때는 allowlist 사용자만 DM 대화를 사용할 수 있습니다. |

### 런타임 엔진

| 변수 | 기본값 | 설명 |
|---|---|---|
| `RUNTIME_ENGINE` | `echo` | `echo`: 입력을 그대로 돌려주는 테스트 모드. `cli`: 실제 AI CLI를 실행합니다. |
| `DEFAULT_PROVIDER` | `claude` | 기본 AI provider. `claude`, `codex`, `opencode` 중 선택. 채팅에서 `/provider set`으로 변경 가능합니다. |
| `DEFAULT_RUNTIME_MODE` | `session` | `session`: 대화 맥락 유지 (같은 세션 ID로 CLI 재호출). `event`: 매 메시지를 독립 처리. 채팅에서 `/mode set`으로 변경 가능합니다. |
| `SESSION_FAIL_FALLBACK_EVENT` | `false` | `true`이면 session 모드 실행이 실패했을 때 자동으로 event 모드로 재시도합니다. 세션이 깨졌을 때 유용합니다. |
| `PROVIDER_TIMEOUT_MS` | `90000` | CLI 프로세스의 최대 실행 시간 (밀리초). 이 시간을 초과하면 프로세스를 강제 종료합니다. |
| `MAX_OUTPUT_BYTES` | `262144` | CLI 출력의 최대 크기 (바이트). 256KB가 기본값입니다. 초과하면 에러를 반환합니다. |

### 동시성 및 Rate Limit

| 변수 | 기본값 | 설명 |
|---|---|---|
| `MAX_CONCURRENT_TASKS` | `8` | 동시에 처리할 수 있는 최대 태스크 수. 서버 리소스에 맞게 조절하세요. |
| `RATE_LIMIT_WINDOW_SECS` | `60` | Rate limit 윈도우 크기 (초). |
| `RATE_LIMIT_MAX_REQUESTS` | `0` | 윈도우 내 scope(채널+채팅방+사용자)당 최대 요청 수. `0`이면 제한 없음. 예: `10`이면 60초간 같은 사용자 scope에서 최대 10건. |

### Discord UX 옵션

| 변수 | 기본값 | 설명 |
|---|---|---|
| `DISCORD_USE_EMBEDS` | `false` | `true`이면 AI 응답을 Discord Embed 형식으로 전송합니다. 코드 블록이 더 깔끔하게 보이지만, 일부 상황에서는 일반 텍스트가 나을 수 있습니다. |

### 종료 설정

| 변수 | 기본값 | 설명 |
|---|---|---|
| `SHUTDOWN_DRAIN_TIMEOUT_MS` | `10000` | 종료 시 진행 중인 작업을 기다리는 최대 시간 (밀리초). 절반은 진행중 태스크 완료 대기, 나머지 절반은 대기열 처리에 사용됩니다. |

### CLI Provider 바이너리 설정

각 provider의 CLI 바이너리 경로와 실행 인자를 커스터마이즈할 수 있습니다.

| 변수 | 기본값 | 설명 |
|---|---|---|
| `CLAUDE_BIN` | `claude` | Claude CLI 실행 파일 경로 |
| `CLAUDE_EVENT_ARGS` | `-p --output-format json` | event 모드에서 사용할 인자 |
| `CLAUDE_SESSION_ARGS` | (EVENT_ARGS와 동일) | session 모드에서 사용할 인자 |
| `CODEX_BIN` | `codex` | Codex CLI 실행 파일 경로 |
| `CODEX_EVENT_ARGS` | `exec --json --skip-git-repo-check` | event 모드에서 사용할 인자 |
| `CODEX_SESSION_ARGS` | (EVENT_ARGS와 동일) | session 모드에서 사용할 인자 |
| `OPENCODE_BIN` | `opencode` | OpenCode CLI 실행 파일 경로 |
| `OPENCODE_EVENT_ARGS` | `run --format json` | event 모드에서 사용할 인자 |
| `OPENCODE_SESSION_ARGS` | (EVENT_ARGS와 동일) | session 모드에서 사용할 인자 |

**인자 지정 방식** — 두 가지 형식 모두 지원됩니다:

```env
# 공백 구분 (간단한 경우)
CODEX_EVENT_ARGS=exec --json --skip-git-repo-check

# JSON 배열 (공백이 포함된 인자가 있는 경우)
CODEX_EVENT_ARGS=["exec","--json","--skip-git-repo-check"]
```

---

## CLI 호출 상세

게이트웨이가 AI CLI를 호출할 때의 실제 명령어 형태입니다. 커스텀 args를 설정하지 않으면 아래 기본값을 사용합니다.

### Event 모드 (단발)

```bash
# Claude
claude -p --output-format json -- "사용자의 프롬프트"

# Codex
codex exec --json --skip-git-repo-check -- "사용자의 프롬프트"

# OpenCode
opencode run --format json -- "사용자의 프롬프트"
```

### Session 모드 (대화 유지)

```bash
# Claude (세션 이어가기)
claude -p --output-format json -r <session_id> -- "사용자의 프롬프트"

# Codex (세션 이어가기)
codex exec --json --skip-git-repo-check resume <session_id> -- "사용자의 프롬프트"

# OpenCode (세션 이어가기)
opencode run --format json --session <session_id> -- "사용자의 프롬프트"
```

> `--` 구분자는 프롬프트 텍스트가 CLI 플래그로 해석되는 것을 방지합니다.

> CLI에 전달되는 환경변수: `ORKA_PROVIDER`, `ORKA_MODE`, `ORKA_CHANNEL`, `ORKA_CHAT_ID`, `ORKA_USER_ID`, `ORKA_SCOPE_KEY`

---

## 운영

### Health 체크 & Metrics

게이트웨이는 HTTP 엔드포인트를 제공합니다:

```bash
# Liveness — 프로세스가 살아있는지
curl http://127.0.0.1:8787/healthz
# 응답: "ok"

# Readiness — 어댑터 기동 완료 여부 (로드밸런서용)
curl http://127.0.0.1:8787/readyz
# 응답: "ready" (200) 또는 "not_ready" (503)

# Metrics — Prometheus 형식
curl http://127.0.0.1:8787/metrics
```

**Metrics 항목:**

```
orka_inbound_total 42                                              # 수신 이벤트 누적
orka_outbound_total 38                                             # 발신 메시지 누적
orka_error_total 3                                                 # 에러 누적
orka_provider_requests_total{provider="claude",mode="session",status="success"} 30
orka_provider_requests_total{provider="codex",mode="event",status="error"} 2
```

### 동시성 제어

- **글로벌 제한**: `MAX_CONCURRENT_TASKS`개를 초과하는 요청은 대기합니다
- **Scope별 제한**: 같은 사용자 scope에서 동시에 2건을 보내면, 먼저 온 것이 처리되고 나중 것은 "busy" 응답을 받습니다
- 공유 채널에서는 사용자마다 scope가 분리되므로, 한 사용자의 session/event 상태가 다른 사용자의 대화에 섞이지 않습니다
- 사용하지 않는 scope의 잠금은 100건 이벤트마다 자동 정리됩니다

### Rate Limiting

scope(채널+채팅방+사용자) 단위의 슬라이딩 윈도우 방식입니다:

```env
RATE_LIMIT_WINDOW_SECS=60      # 60초 윈도우
RATE_LIMIT_MAX_REQUESTS=10     # 사용자 scope당 10건 (0이면 제한 없음)
```

제한에 걸리면 사용자에게 "Rate limited. Please wait before sending more requests." 메시지가 전송됩니다.

### Graceful Shutdown

`Ctrl+C` 또는 `SIGTERM` 신호를 받으면:

1. **readyz를 false로 전환** — 로드밸런서가 새 트래픽을 보내지 않도록
2. **어댑터 중지** — 더 이상 새 메시지를 수신하지 않음
3. **진행 중 태스크 완료 대기** — 타임아웃의 절반 (기본 5초)
4. **대기열 잔여 처리** — 큐에 남은 이벤트를 순차 처리 (나머지 5초)
5. **종료**

처리 중이던 AI 응답을 가능한 한 완료한 후에 종료하므로, 사용자의 요청이 유실되지 않습니다.

---

## 트러블슈팅

### "no adapters enabled" 경고

```
WARN  no adapters enabled; set DISCORD_BOT_TOKEN and/or TELEGRAM_BOT_TOKEN
```

`.env`에 봇 토큰이 설정되지 않았습니다. `DISCORD_BOT_TOKEN` 또는 `TELEGRAM_BOT_TOKEN`을 설정하세요.

### Codex CLI가 "Operation not permitted" 에러

macOS에서 `codex` shim(npm 심볼릭 링크)이 샌드박스 제약으로 실패할 수 있습니다. 네이티브 바이너리 경로를 직접 지정하세요:

```env
CODEX_BIN=/Users/<username>/.nvm/versions/node/<version>/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/codex/codex
```

### AI 응답이 잘림

`MAX_OUTPUT_BYTES`를 늘려보세요 (기본 256KB):

```env
MAX_OUTPUT_BYTES=524288    # 512KB
```

### Session 모드에서 대화가 이상해짐

세션이 깨진 경우:
```
/new
```

운영자라면:
```
/session reset
```

또는 자동 복구를 활성화:
```env
SESSION_FAIL_FALLBACK_EVENT=true
```

### Provider CLI 타임아웃

긴 작업이 90초 기본 타임아웃을 초과하는 경우:

```env
PROVIDER_TIMEOUT_MS=180000    # 3분
```

---

## 프로젝트 구조

```
cli-rs-gateway-DT/
├── crates/
│   ├── core/                  # 도메인 모델, 파이프라인, 포트 정의
│   │   └── src/
│   │       ├── model.rs       # 데이터 타입 (InboundEvent, Command 등)
│   │       ├── pipeline.rs    # 이벤트 처리 파이프라인
│   │       ├── policy.rs      # 접근 정책 (RBAC)
│   │       ├── ports.rs       # Trait 정의 (EventStore, OutboundSender 등)
│   │       ├── rate_limit.rs  # 슬라이딩 윈도우 Rate Limiter
│   │       ├── session.rs     # 세션 키 생성
│   │       └── text.rs        # 텍스트 정규화, 청크 분할
│   ├── app/                   # 애플리케이션 진입점
│   │   └── src/
│   │       ├── main.rs        # main 함수, 이벤트 루프, 동시성 제어
│   │       ├── config.rs      # 환경변수 파싱
│   │       ├── runtime.rs     # CLI 프로세스 실행 로직
│   │       └── health.rs      # Health/Metrics HTTP 서버
│   ├── adapters-discord/      # Discord Gateway + REST API
│   ├── adapters-telegram/     # Telegram Long Polling + REST API
│   └── storage-sqlite/        # SQLite 기반 EventStore 구현
├── migrations/                # SQLite 마이그레이션 SQL (런타임에 ./migrations 에서 로드)
├── scripts/
│   └── windows/               # Windows 배포 스크립트
│       ├── setup.ps1          # 전제조건 자동 설치 (VC++ Runtime, pwsh7, Node.js, Codex)
│       ├── start-orka.ps1     # .env 로드 + 실행
│       ├── register-startup.ps1 # 시작프로그램 등록/해제
│       └── install-service.ps1 # NSSM 서비스 등록
├── docs/                      # 설계 문서
├── .env.example               # 환경변수 템플릿
└── Cargo.toml                 # Workspace 설정
```

### 아키텍처 원칙

- **Hexagonal Architecture**: `core`는 어댑터에 의존하지 않습니다. 모든 외부 의존성은 trait(port)로 추상화합니다.
- **어댑터 독립**: Discord 어댑터를 제거해도 나머지 코드에 영향이 없습니다.
- **Opaque Token Pattern**: `reply_token`, `claims` 등 채널 고유 데이터는 `Option<String>` / `Vec<String>`으로 전달하여 core가 Discord/Telegram 타입에 의존하지 않습니다.

---

## Windows 배포

Windows PC(미니 PC 등)에서 상시 실행할 수 있습니다. macOS에서 크로스 컴파일 후 바이너리만 전송하는 방식을 권장합니다.

```bash
# macOS에서 크로스 컴파일
brew install mingw-w64 && rustup target add x86_64-pc-windows-gnu
cargo build --release -p orka-app --target x86_64-pc-windows-gnu

# Windows PC로 전송
scp target/x86_64-pc-windows-gnu/release/orka-app.exe user@windows-pc:"C:/Users/you/orka/"
```

Windows PC에 필요한 것:
- **Visual C++ Redistributable** (Codex CLI에 필요)
- **Node.js** + `npm install -g @openai/codex` (AI CLI)

상세 가이드: [`docs/WINDOWS_DEPLOYMENT.md`](docs/WINDOWS_DEPLOYMENT.md)

---

## 추가 문서

상세한 기술 문서는 `docs/` 디렉토리에 있습니다:

| 문서 | 내용 |
|---|---|
| `ARCHITECTURE.md` | 전체 아키텍처 설계 |
| `TECHNICAL_PLAN.md` | 기술 구현 계획 |
| `RUNBOOK.md` | 운영 매뉴얼 |
| `DEPLOYMENT_CHECKLIST.md` | 배포 체크리스트 |
| `WINDOWS_DEPLOYMENT.md` | Windows 배포 가이드 |
| `DISCORD_MULTI_CLI_SPEC.md` | Discord 멀티 CLI 스펙 |
| `FAILURE_EDGECASE_PLAN.md` | 장애/엣지케이스 대응 |
| `CODE_REVIEW_REPORT.md` | 코드 리뷰 리포트 |

---

## 라이선스

MIT
