#!/usr/bin/env bash
# Unified workspace acceptance for zero-chain + zero-explore + zero-mining-stack + wallets.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WORKSPACE_DIR="${WORKSPACE_DIR:-${ROOT_DIR}/..}"
MINING_STACK_DIR="${MINING_STACK_DIR:-${WORKSPACE_DIR}/zero-mining-stack}"
EXPLORER_DIR="${EXPLORER_DIR:-${WORKSPACE_DIR}/zero-explore}"
EXPLORER_BACKEND_DIR="${EXPLORER_BACKEND_DIR:-${EXPLORER_DIR}/backend}"
WALLET_CHROME_DIR="${WALLET_CHROME_DIR:-${WORKSPACE_DIR}/zero-wallet-chrome}"
WALLET_MOBILE_DIR="${WALLET_MOBILE_DIR:-${WORKSPACE_DIR}/zero-wallet-mobile}"

REPORT_DIR="${ROOT_DIR}/artifacts/workspace-acceptance"
REPORT_FILE="${REPORT_DIR}/workspace-acceptance-report.md"
mkdir -p "${REPORT_DIR}"

DATE_UTC="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

run_step() {
  local name="$1"
  shift
  echo "==> ${name}"
  "$@"
}

assert_dir() {
  local dir="$1"
  if [[ ! -d "${dir}" ]]; then
    echo "Missing directory: ${dir}" >&2
    exit 1
  fi
}

assert_dir "${MINING_STACK_DIR}"
assert_dir "${EXPLORER_BACKEND_DIR}"
assert_dir "${WALLET_CHROME_DIR}"
assert_dir "${WALLET_MOBILE_DIR}"

run_step "zero-chain full-chain e2e" bash -lc "cd '${ROOT_DIR}' && bash scripts/full_chain_e2e.sh"
run_step "zero-mining-stack nightly local qa" bash -lc "cd '${MINING_STACK_DIR}' && bash scripts/nightly_local_qa.sh"

run_step "zero-wallet-chrome build" bash -lc "cd '${WALLET_CHROME_DIR}' && bun run build"
run_step "zero-wallet-chrome tests" bash -lc "cd '${WALLET_CHROME_DIR}' && bun run test"
run_step "zero-wallet-chrome extension smoke" bash -lc "cd '${WALLET_CHROME_DIR}' && bun run qa:extension"

run_step "zero-wallet-mobile analyze" bash -lc "cd '${WALLET_MOBILE_DIR}' && flutter analyze"
run_step "zero-wallet-mobile tests" bash -lc "cd '${WALLET_MOBILE_DIR}' && flutter test"
run_step "zero-wallet-mobile devices" bash -lc "cd '${WALLET_MOBILE_DIR}' && flutter devices"

cat > "${REPORT_FILE}" <<EOF
# Workspace Acceptance Report

- Generated at: ${DATE_UTC}
- Workspace: ${WORKSPACE_DIR}

## Automated Checks

- [x] zero-chain/scripts/full_chain_e2e.sh
- [x] zero-mining-stack/scripts/nightly_local_qa.sh
- [x] zero-wallet-chrome bun run build
- [x] zero-wallet-chrome bun run test
- [x] zero-wallet-chrome bun run qa:extension
- [x] zero-wallet-mobile flutter analyze
- [x] zero-wallet-mobile flutter test
- [x] zero-wallet-mobile flutter devices

## Manual Follow-up

- [ ] If a GUI session is available, run zero-wallet-mobile with \`flutter run -d linux\` or a real Android device
- [ ] If browser sandbox policy allows, run zero-wallet-mobile with \`flutter run -d chrome\`
- [ ] If multi-node mirroring is required, rerun zero-mining-stack pool with explicit \`--mirror-peer\` values
EOF

echo "✅ workspace acceptance passed"
echo "📄 Report: ${REPORT_FILE}"
