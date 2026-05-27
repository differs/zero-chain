#!/usr/bin/env bash
# Strict local mainnet topology smoke with default mainnet mining, auth, and RocksDb.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT_DIR="${ROOT_DIR}/artifacts/mainnet-strict-smoke"
REPORT_FILE="${REPORT_DIR}/report.md"
LOG_DIR="${REPORT_DIR}/logs"

RPC_AUTH_TOKEN="${RPC_AUTH_TOKEN:-mainnet-strict-smoke-token}"
COINBASE="${COINBASE:-ZER0x526Dc404e751C7d52F6fFF75d563d8D0857C94E9}"

BOOTNODE_RPC_URL="${BOOTNODE_RPC_URL:-http://127.0.0.1:8545}"
FOLLOWER_RPC_URL="${FOLLOWER_RPC_URL:-http://127.0.0.1:29645}"
OBSERVER_RPC_URL="${OBSERVER_RPC_URL:-http://127.0.0.1:39745}"

RPC_TIMEOUT_SECS="${RPC_TIMEOUT_SECS:-8}"
EXPECTED_NET_VERSION="${EXPECTED_NET_VERSION:-10086}"

usage() {
  cat <<'EOF'
Usage: bash scripts/mainnet_strict_smoke.sh

This smoke intentionally keeps strict mainnet-like defaults:
  - network profile: mainnet
  - compute backend: RocksDb (profile default)
  - built-in miner enabled on bootnode
  - RPC auth token enabled
  - default RPC rate limit retained
EOF
}

while (($# > 0)); do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

cleanup() {
  cd "${ROOT_DIR}"
  ./scripts/mainnet.sh stop observer >/dev/null 2>&1 || true
  ./scripts/mainnet.sh stop follower >/dev/null 2>&1 || true
  ./scripts/mainnet.sh stop bootnode >/dev/null 2>&1 || true
}
trap cleanup EXIT

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing command: $1" >&2
    exit 1
  }
}

rpc_call() {
  local url="$1"
  local method="$2"
  curl -fsS --max-time "${RPC_TIMEOUT_SECS}" \
    -H 'Content-Type: application/json' \
    -H "authorization: Bearer ${RPC_AUTH_TOKEN}" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":[],\"id\":1}" \
    "${url}"
}

extract_result_hex() {
  sed -n 's/.*"result":"\([^"]*\)".*/\1/p'
}

extract_block_hex() {
  sed -n 's/.*"number":"\([^"]*\)".*/\1/p'
}

extract_sync_field_u64() {
  local field="$1"
  sed -n "s/.*\"${field}\":[ ]*\\([0-9][0-9]*\\).*/\\1/p"
}

hex_to_dec() {
  local value="$1"
  local hex="${value#0x}"
  if [[ -z "${hex}" ]]; then
    printf '0\n'
    return 0
  fi
  printf '%d\n' "$((16#${hex}))"
}

mkdir -p "${REPORT_DIR}" "${LOG_DIR}"
require_cmd curl

cd "${ROOT_DIR}"
./scripts/mainnet.sh stop observer >/dev/null 2>&1 || true
./scripts/mainnet.sh stop follower >/dev/null 2>&1 || true
./scripts/mainnet.sh stop bootnode >/dev/null 2>&1 || true

echo "==> start strict mainnet bootnode"
./scripts/mainnet.sh start bootnode \
  --mine \
  --coinbase "${COINBASE}" \
  --rpc-auth-token "${RPC_AUTH_TOKEN}" \
  --p2p-listen-addr 127.0.0.1 | tee "${LOG_DIR}/bootnode-start.log"

BOOTNODE_ENODE="$(grep -m1 'bootnode enode hint:' "${HOME}/.zerochain/mainnet/bootnode/bootnode.log" 2>/dev/null | sed 's/.*hint: //')"
if [[ -z "${BOOTNODE_ENODE}" ]]; then
  echo "failed to discover bootnode enode hint" >&2
  exit 1
fi

echo "==> start strict mainnet follower"
./scripts/mainnet.sh start follower \
  --bootnode "${BOOTNODE_ENODE}" \
  --p2p-listen-addr 127.0.0.1 | tee "${LOG_DIR}/follower-start.log"

echo "==> start strict mainnet observer"
./scripts/mainnet.sh start observer \
  --bootnode "${BOOTNODE_ENODE}" \
  --p2p-listen-addr 127.0.0.1 | tee "${LOG_DIR}/observer-start.log"

echo "==> wait for height progression"
bootnode_before_json="$(rpc_call "${BOOTNODE_RPC_URL}" zero_getLatestBlock)"
bootnode_before_hex="$(printf '%s' "${bootnode_before_json}" | extract_block_hex)"
bootnode_before_dec="$(hex_to_dec "${bootnode_before_hex}")"
bootnode_after_hex="${bootnode_before_hex}"
bootnode_after_dec="${bootnode_before_dec}"
for _ in {1..12}; do
  sleep 5
  bootnode_after_json="$(rpc_call "${BOOTNODE_RPC_URL}" zero_getLatestBlock)"
  bootnode_after_hex="$(printf '%s' "${bootnode_after_json}" | extract_block_hex)"
  bootnode_after_dec="$(hex_to_dec "${bootnode_after_hex}")"
  if (( bootnode_after_dec > bootnode_before_dec )); then
    break
  fi
done
if (( bootnode_after_dec <= bootnode_before_dec )); then
  echo "bootnode height did not increase within strict timeout: ${bootnode_before_hex} -> ${bootnode_after_hex}" >&2
  exit 1
fi

bootnode_net="$(rpc_call "${BOOTNODE_RPC_URL}" net_version | extract_result_hex)"
follower_net="$(rpc_call "${FOLLOWER_RPC_URL}" net_version | extract_result_hex)"
observer_net="$(rpc_call "${OBSERVER_RPC_URL}" net_version | extract_result_hex)"

if [[ "${bootnode_net}" != "${EXPECTED_NET_VERSION}" || "${follower_net}" != "${EXPECTED_NET_VERSION}" || "${observer_net}" != "${EXPECTED_NET_VERSION}" ]]; then
  echo "unexpected net_version values: bootnode=${bootnode_net} follower=${follower_net} observer=${observer_net}" >&2
  exit 1
fi

follower_peer_hex="$(rpc_call "${FOLLOWER_RPC_URL}" net_peerCount | extract_result_hex)"
observer_peer_hex="$(rpc_call "${OBSERVER_RPC_URL}" net_peerCount | extract_result_hex)"
follower_peer_dec="$(hex_to_dec "${follower_peer_hex:-0x0}")"
observer_peer_dec="$(hex_to_dec "${observer_peer_hex:-0x0}")"
if (( follower_peer_dec < 1 || observer_peer_dec < 1 )); then
  echo "peer counts below expected threshold: follower=${follower_peer_dec} observer=${observer_peer_dec}" >&2
  exit 1
fi

follower_block_hex="$(rpc_call "${FOLLOWER_RPC_URL}" zero_getLatestBlock | extract_block_hex)"
observer_block_hex="$(rpc_call "${OBSERVER_RPC_URL}" zero_getLatestBlock | extract_block_hex)"
follower_block_dec="$(hex_to_dec "${follower_block_hex}")"
observer_block_dec="$(hex_to_dec "${observer_block_hex}")"
follower_gap=$(( bootnode_after_dec - follower_block_dec ))
observer_gap=$(( bootnode_after_dec - observer_block_dec ))
if (( follower_gap < 0 )); then follower_gap=$(( -follower_gap )); fi
if (( observer_gap < 0 )); then observer_gap=$(( -observer_gap )); fi
if (( follower_gap > 2 || observer_gap > 2 )); then
  echo "sync gap too large: follower_gap=${follower_gap} observer_gap=${observer_gap}" >&2
  exit 1
fi

bootnode_sync_json="$(rpc_call "${BOOTNODE_RPC_URL}" zero_syncStatus)"
follower_sync_json="$(rpc_call "${FOLLOWER_RPC_URL}" zero_syncStatus)"
observer_sync_json="$(rpc_call "${OBSERVER_RPC_URL}" zero_syncStatus)"

DATE_UTC="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
COMMIT="$(git -C "${ROOT_DIR}" rev-parse --short HEAD)"
cat > "${REPORT_FILE}" <<EOF
# Mainnet Strict Smoke Report

- Generated at: ${DATE_UTC}
- Commit: ${COMMIT}
- RPC auth token enabled: yes
- Mainnet profile: yes
- Bootnode RPC: ${BOOTNODE_RPC_URL}
- Follower RPC: ${FOLLOWER_RPC_URL}
- Observer RPC: ${OBSERVER_RPC_URL}
- Bootnode enode: ${BOOTNODE_ENODE}

## Checks

- [x] bootnode started with mainnet profile, default built-in miner, and RPC auth token
- [x] follower and observer joined via real bootnode enode
- [x] net_version consistent across all nodes (${EXPECTED_NET_VERSION})
- [x] bootnode height increased (${bootnode_before_hex} -> ${bootnode_after_hex})
- [x] follower peerCount >= 1 (actual ${follower_peer_dec})
- [x] observer peerCount >= 1 (actual ${observer_peer_dec})
- [x] follower/observer sync gap <= 2 (actual ${follower_gap}/${observer_gap})

## Sync Status

- bootnode:
\`\`\`json
${bootnode_sync_json}
\`\`\`

- follower:
\`\`\`json
${follower_sync_json}
\`\`\`

- observer:
\`\`\`json
${observer_sync_json}
\`\`\`
EOF

echo "✅ mainnet strict smoke passed"
echo "📄 Report: ${REPORT_FILE}"
