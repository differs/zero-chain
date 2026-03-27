#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cleanup() {
  bash "${ROOT_DIR}/scripts/mainnet_local_stop.sh" >/dev/null 2>&1 || true
}
trap cleanup EXIT

cd "${ROOT_DIR}"
bash scripts/mainnet_local_bringup.sh &
BRINGUP_PID=$!

sleep 15
bash scripts/mainnet_local_check.sh

kill "${BRINGUP_PID}" >/dev/null 2>&1 || true
wait "${BRINGUP_PID}" >/dev/null 2>&1 || true
bash scripts/mainnet_local_stop.sh

echo "mainnet local cycle passed"
