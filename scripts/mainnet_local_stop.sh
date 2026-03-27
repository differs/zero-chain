#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_STATE_DIR="${ROOT_DIR}/artifacts/mainnet-local-bringup-state"

stop_pid_file() {
  local name="$1"
  local pid_file="${RUN_STATE_DIR}/${name}.pid"
  if [[ ! -f "${pid_file}" ]]; then
    return 0
  fi

  local pid
  pid="$(cat "${pid_file}" 2>/dev/null || true)"
  rm -f "${pid_file}"
  if [[ -z "${pid}" ]]; then
    return 0
  fi

  kill "${pid}" >/dev/null 2>&1 || true
  for _ in {1..40}; do
    if ! kill -0 "${pid}" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  kill -9 "${pid}" >/dev/null 2>&1 || true
}

cd "${ROOT_DIR}"
./scripts/mainnet.sh stop observer >/dev/null 2>&1 || true
./scripts/mainnet.sh stop follower >/dev/null 2>&1 || true
./scripts/mainnet.sh stop bootnode >/dev/null 2>&1 || true

stop_pid_file "miner"
stop_pid_file "pool"
stop_pid_file "explorer-backend"

echo "mainnet local bring-up stopped"
