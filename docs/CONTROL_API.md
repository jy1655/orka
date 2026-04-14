# Control API (Archived)

이 문서는 과거 설계용 보관 문서입니다.

## 현재 상태

- `orka-gateway` 현재 런타임에서는 HTTP Control API를 노출하지 않습니다.
- `main.rs`에서 control server 배선이 제거되어 `/control/v1/*` 엔드포인트는 활성화되지 않습니다.
- 현재 운영 제어는 채널 명령(`/help`, `/status`, `/new`, `/provider ...`, `/mode ...`, `/session ...`, `/pause`, `/resume`)으로 수행합니다.

## 왜 비활성화했는가

- 초기 목표가 "게이트웨이 최소화"로 정리됨
- 별도 API 토큰/포트 운영 복잡도 감소
- 채널 내 operator 명령으로 필요한 제어를 충족

## 참고

소스 저장소에는 `crates/app/src/control.rs`가 남아 있으나,
현재 실행 경로에 연결되지 않은 상태입니다.

향후 HTTP 제어가 다시 필요하면 아래 항목부터 재검토합니다.

1. 인증 토큰 관리 정책
2. bind 주소/네트워크 접근 제어
3. 채널 명령과 API 제어 간 우선순위/충돌 규칙
