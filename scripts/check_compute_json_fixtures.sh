#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WORKSPACE_DIR="${WORKSPACE_DIR:-${ROOT_DIR}/..}"
WALLET_CHROME_DIR="${WALLET_CHROME_DIR:-${WORKSPACE_DIR}/zero-wallet-chrome}"
WALLET_MOBILE_DIR="${WALLET_MOBILE_DIR:-${WORKSPACE_DIR}/zero-wallet-mobile}"

echo "==> zeroapi compute json fixtures"
bash -lc "cd '${ROOT_DIR}' && cargo test -p zeroapi compute_json_fixture_ -- --nocapture"

echo "==> zero-wallet-chrome compute json fixtures"
bash -lc "cd '${WALLET_CHROME_DIR}' && bun test src/core/wallet/ComputeTx.fixture.test.ts"

echo "==> zero-wallet-mobile compute json fixtures"
bash -lc "cd '${WALLET_MOBILE_DIR}' && flutter test test/core/utils/compute_tx_fixture_test.dart"

echo "✅ compute json fixtures passed"
