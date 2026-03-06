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

run_check "R-01: missing binary handling" \
  cargo test -q -p orka-app cli_runtime_returns_error_when_binary_missing

run_check "R-02: provider timeout handling" \
  cargo test -q -p orka-app cli_runtime_times_out_provider_process

run_check "R-05: max output bytes handling" \
  cargo test -q -p orka-app cli_runtime_fails_on_output_size_limit

run_check "R-03: session failure fallback to event" \
  cargo test -q -p orka-core session_failure_can_fallback_to_event_mode

run_check "runtime safe error message on final failure" \
  cargo test -q -p orka-core runtime_failure_sends_safe_error_message

echo
echo "---- Drill Summary ----"
echo "PASS: ${pass_count}"
echo "FAIL: ${fail_count}"

if [ "${fail_count}" -gt 0 ]; then
  exit 1
fi

exit 0
