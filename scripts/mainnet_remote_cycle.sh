#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "${ROOT_DIR}"
bash scripts/mainnet_remote_preflight.sh
bash scripts/public_node_reset_and_verify.sh
bash scripts/mainnet_checklist.sh

echo "mainnet remote cycle passed"
