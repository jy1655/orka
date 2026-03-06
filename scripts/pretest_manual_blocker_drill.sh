#!/usr/bin/env bash
set -u

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

pass_count=0
fail_count=0

run_check() {
  local name="$1"
  shift

  echo
  echo "==> ${name}"
  if "$@"; then
    echo "[PASS] ${name}"
    pass_count=$((pass_count + 1))
  else
    echo "[FAIL] ${name}"
    fail_count=$((fail_count + 1))
  fi
}

check_c01_invalid_discord_token() {
  local log_file
  log_file="$(mktemp)"
  local pid=""

  (
    DISCORD_BOT_TOKEN="invalid-token" \
    TELEGRAM_BOT_TOKEN="" \
    GATEWAY_AUTH_TOKEN="" \
    RUNTIME_ENGINE="echo" \
    HEALTH_BIND="127.0.0.1:18787" \
    CONTROL_BIND="127.0.0.1:18788" \
    cargo run -q -p orka-app >"${log_file}" 2>&1
  ) &
  pid=$!

  # Boot + discord connect 시도까지 잠시 대기
  sleep 8

  # 앱이 살아있는 상태에서 어댑터 에러 로그를 확인하는 것이 핵심
  local ok=0
  if rg -q "discord adapter stopped with error|invalid token|Authentication failed|Shard manager" "${log_file}"; then
    ok=1
  fi

  if kill -0 "${pid}" 2>/dev/null; then
    kill -INT "${pid}" 2>/dev/null || true
    wait "${pid}" >/dev/null 2>&1 || true
  fi

  rm -f "${log_file}"

  if [ "${ok}" -eq 1 ]; then
    return 0
  fi
  return 1
}

check_s01_sqlite_permission_denied() {
  local ro_dir
  ro_dir="$(mktemp -d)"
  chmod 500 "${ro_dir}"

  local log_file
  log_file="$(mktemp)"

  local ok=0
  if DISCORD_BOT_TOKEN="" \
    TELEGRAM_BOT_TOKEN="" \
    GATEWAY_AUTH_TOKEN="" \
    RUNTIME_ENGINE="echo" \
    DATABASE_URL="sqlite://${ro_dir}/orka.db" \
    cargo run -q -p orka-app >"${log_file}" 2>&1; then
    ok=0
  else
    if rg -q "Permission denied|unable to open database file|connect|database" "${log_file}"; then
      ok=1
    fi
  fi

  chmod 700 "${ro_dir}" 2>/dev/null || true
  rm -rf "${ro_dir}"
  rm -f "${log_file}"

  if [ "${ok}" -eq 1 ]; then
    return 0
  fi
  return 1
}

run_check "C-01: invalid discord token handling" check_c01_invalid_discord_token
run_check "S-01: sqlite permission denied handling" check_s01_sqlite_permission_denied

echo
echo "---- Manual Drill Summary ----"
echo "PASS: ${pass_count}"
echo "FAIL: ${fail_count}"

if [ "${fail_count}" -gt 0 ]; then
  exit 1
fi

exit 0
