#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

LOCAL_ZEROCHAIN_BIN="${LOCAL_ZEROCHAIN_BIN:-${ROOT_DIR}/target/debug/zerochain}"
REMOTE_ZEROCHAIN_BIN="${REMOTE_ZEROCHAIN_BIN:-/root/works/zero-chain-public-soak/bin/zerochain_local}"

REMOTE_HOST="${REMOTE_HOST:-139.180.207.66}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_RPC_PORT="${REMOTE_RPC_PORT:-28545}"
REMOTE_WS_PORT="${REMOTE_WS_PORT:-28546}"
REMOTE_P2P_PORT="${REMOTE_P2P_PORT:-30303}"
SSH_KEY="${SSH_KEY:-/root/.ssh/agent_139_180_207_66}"
SSH_TIMEOUT_SECS="${SSH_TIMEOUT_SECS:-10}"

LOCAL_RPC_PORT="${LOCAL_RPC_PORT:-29645}"
LOCAL_WS_PORT="${LOCAL_WS_PORT:-29646}"
LOCAL_P2P_PORT="${LOCAL_P2P_PORT:-31303}"

NETWORK_ID="${NETWORK_ID:-31337}"
CHAIN_ID="${CHAIN_ID:-31337}"
COINBASE="${COINBASE:-ZER0x526Dc404e751C7d52F6fFF75d563d8D0857C94E9}"

VERIFY_TIMEOUT_SECS="${VERIFY_TIMEOUT_SECS:-180}"
VERIFY_INTERVAL_SECS="${VERIFY_INTERVAL_SECS:-10}"
VERIFY_MIN_HEIGHT="${VERIFY_MIN_HEIGHT:-1}"
VERIFY_MAX_GAP="${VERIFY_MAX_GAP:-8}"

MONITOR_DURATION_SECS="${MONITOR_DURATION_SECS:-259200}"
MONITOR_INTERVAL_SECS="${MONITOR_INTERVAL_SECS:-60}"

log() {
  printf '[%s] %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

ssh_cmd() {
  ssh -i "${SSH_KEY}" \
    -o StrictHostKeyChecking=no \
    -o BatchMode=yes \
    -o ConnectTimeout="${SSH_TIMEOUT_SECS}" \
    "${REMOTE_USER}@${REMOTE_HOST}" "$@"
}

rpc_local() {
  local method="$1"
  curl -fsS --max-time 8 -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":[],\"id\":1}" \
    "http://127.0.0.1:${LOCAL_RPC_PORT}"
}

rpc_remote() {
  local method="$1"
  ssh_cmd "curl -fsS --max-time 8 -H 'Content-Type: application/json' -d '{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":[],\"id\":1}' http://127.0.0.1:${REMOTE_RPC_PORT}"
}

extract_result_hex() {
  sed -n 's/.*"result":"\([^"]*\)".*/\1/p'
}

extract_block_hex() {
  sed -n 's/.*"number":"\([^"]*\)".*/\1/p'
}

hex_to_dec() {
  local value="$1"
  local hex="${value#0x}"
  if [[ -z "${hex}" ]]; then
    printf '0\n'
  else
    printf '%d\n' "$((16#${hex}))"
  fi
}

wait_rpc_up() {
  local kind="$1"
  local timeout_secs="$2"
  local interval=2
  local elapsed=0
  while (( elapsed < timeout_secs )); do
    if [[ "${kind}" == "local" ]]; then
      if rpc_local net_version >/dev/null 2>&1; then
        return 0
      fi
    else
      if rpc_remote net_version >/dev/null 2>&1; then
        return 0
      fi
    fi
    sleep "${interval}"
    elapsed=$((elapsed + interval))
  done
  return 1
}

require_cmd curl
require_cmd ssh
require_cmd sed

if [[ ! -x "${LOCAL_ZEROCHAIN_BIN}" ]]; then
  echo "local zerochain binary not found: ${LOCAL_ZEROCHAIN_BIN}" >&2
  exit 1
fi
if [[ ! -f "${SSH_KEY}" ]]; then
  echo "ssh key not found: ${SSH_KEY}" >&2
  exit 1
fi

cd "${ROOT_DIR}"

log "停止公网监控"
./scripts/public_node_soak_monitor.sh stop || true

log "停止本地公网节点"
if [[ -f artifacts/public-node-soak/current.env ]]; then
  # shellcheck disable=SC1091
  source artifacts/public-node-soak/current.env || true
  if [[ -n "${NODE_PID:-}" ]] && kill -0 "${NODE_PID}" 2>/dev/null; then
    kill "${NODE_PID}" || true
    sleep 1
  fi
fi

log "停止远端公网节点"
ssh_cmd '
set -euo pipefail
if [ -f /root/works/zero-chain-public-soak/current.env ]; then
  . /root/works/zero-chain-public-soak/current.env || true
  if [ -n "${NODE_PID:-}" ] && kill -0 "${NODE_PID}" 2>/dev/null; then
    kill "${NODE_PID}" || true
    sleep 1
  fi
fi
'

log "清空本地公网数据"
rm -rf artifacts/public-node-soak artifacts/public-node-soak-monitor
mkdir -p artifacts/public-node-soak artifacts/public-node-soak-monitor

log "清空远端公网数据"
ssh_cmd '
set -euo pipefail
rm -rf /root/works/zero-chain-public-soak/[0-9]* \
       /root/works/zero-chain-public-soak/current.env \
       /root/works/zero-chain-public-soak/remote-node.pid
mkdir -p /root/works/zero-chain-public-soak
'

log "启动远端公网节点"
ssh_cmd "
set -euo pipefail
TS=\$(date -u +%Y%m%dT%H%M%SZ)
RUN_DIR=/root/works/zero-chain-public-soak/\${TS}
DATA_DIR=\${RUN_DIR}/remote-node-data
LOG_FILE=\${RUN_DIR}/remote-node.log
mkdir -p \"\${RUN_DIR}\"
if command -v setsid >/dev/null 2>&1; then
  setsid ${REMOTE_ZEROCHAIN_BIN} \\
    --data-dir \"\${DATA_DIR}\" \\
    run \\
    --http-port ${REMOTE_RPC_PORT} \\
    --ws-port ${REMOTE_WS_PORT} \\
    --p2p-listen-addr 0.0.0.0 \\
    --p2p-listen-port ${REMOTE_P2P_PORT} \\
    --chain-id ${CHAIN_ID} \\
    --rpc-network-id ${NETWORK_ID} \\
    --rpc-coinbase ${COINBASE} \\
    >\"\${LOG_FILE}\" 2>&1 < /dev/null &
else
  nohup ${REMOTE_ZEROCHAIN_BIN} \\
    --data-dir \"\${DATA_DIR}\" \\
    run \\
    --http-port ${REMOTE_RPC_PORT} \\
    --ws-port ${REMOTE_WS_PORT} \\
    --p2p-listen-addr 0.0.0.0 \\
    --p2p-listen-port ${REMOTE_P2P_PORT} \\
    --chain-id ${CHAIN_ID} \\
    --rpc-network-id ${NETWORK_ID} \\
    --rpc-coinbase ${COINBASE} \\
    >\"\${LOG_FILE}\" 2>&1 < /dev/null &
fi
NEW_PID=\$!
cat > /root/works/zero-chain-public-soak/current.env <<ENV
RUN_DIR=\${RUN_DIR}
DATA_DIR=\${DATA_DIR}
LOG_FILE=\${LOG_FILE}
NODE_PID=\${NEW_PID}
ENV
echo \"\${NEW_PID}\" > /root/works/zero-chain-public-soak/remote-node.pid
echo \"remote_pid=\${NEW_PID}\"
"

log "启动本地公网节点（开启挖矿）"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
LOCAL_RUN_DIR="${ROOT_DIR}/artifacts/public-node-soak/${TS}"
LOCAL_DATA_DIR="${LOCAL_RUN_DIR}/local-node-data"
LOCAL_LOG_FILE="${LOCAL_RUN_DIR}/local-node.log"
mkdir -p "${LOCAL_RUN_DIR}"
if command -v setsid >/dev/null 2>&1; then
  setsid "${LOCAL_ZEROCHAIN_BIN}" \
    --data-dir "${LOCAL_DATA_DIR}" \
    run \
    --mine \
    --coinbase "${COINBASE}" \
    --rpc-coinbase "${COINBASE}" \
    --http-port "${LOCAL_RPC_PORT}" \
    --ws-port "${LOCAL_WS_PORT}" \
    --p2p-listen-addr 0.0.0.0 \
    --p2p-listen-port "${LOCAL_P2P_PORT}" \
    --bootnode "enode://remote-public@${REMOTE_HOST}:${REMOTE_P2P_PORT}" \
    --chain-id "${CHAIN_ID}" \
    --rpc-network-id "${NETWORK_ID}" \
    >"${LOCAL_LOG_FILE}" 2>&1 < /dev/null &
else
  nohup "${LOCAL_ZEROCHAIN_BIN}" \
    --data-dir "${LOCAL_DATA_DIR}" \
    run \
    --mine \
    --coinbase "${COINBASE}" \
    --rpc-coinbase "${COINBASE}" \
    --http-port "${LOCAL_RPC_PORT}" \
    --ws-port "${LOCAL_WS_PORT}" \
    --p2p-listen-addr 0.0.0.0 \
    --p2p-listen-port "${LOCAL_P2P_PORT}" \
    --bootnode "enode://remote-public@${REMOTE_HOST}:${REMOTE_P2P_PORT}" \
    --chain-id "${CHAIN_ID}" \
    --rpc-network-id "${NETWORK_ID}" \
    >"${LOCAL_LOG_FILE}" 2>&1 < /dev/null &
fi
LOCAL_PID=$!
cat > artifacts/public-node-soak/current.env <<ENV
RUN_DIR=${LOCAL_RUN_DIR}
DATA_DIR=${LOCAL_DATA_DIR}
LOG_FILE=${LOCAL_LOG_FILE}
NODE_PID=${LOCAL_PID}
ENV
echo "${LOCAL_PID}" > artifacts/public-node-soak/local-node.pid

if ! wait_rpc_up remote 25; then
  echo "remote rpc did not become ready" >&2
  exit 1
fi
if ! wait_rpc_up local 25; then
  echo "local rpc did not become ready" >&2
  exit 1
fi

# discover remote pid from remote env
REMOTE_PID="$(ssh_cmd 'set -euo pipefail; . /root/works/zero-chain-public-soak/current.env; printf "%s" "${NODE_PID}"')"

log "启动公网监控"
./scripts/public_node_soak_monitor.sh start \
  --duration-secs "${MONITOR_DURATION_SECS}" \
  --interval-secs "${MONITOR_INTERVAL_SECS}" \
  --local-rpc-url "http://127.0.0.1:${LOCAL_RPC_PORT}" \
  --remote-host "${REMOTE_HOST}" \
  --remote-user "${REMOTE_USER}" \
  --remote-rpc-port "${REMOTE_RPC_PORT}" \
  --ssh-key "${SSH_KEY}" \
  --local-node-pid "${LOCAL_PID}" \
  --remote-node-pid "${REMOTE_PID}" \
  --rpc-timeout-secs 8 \
  --ssh-timeout-secs "${SSH_TIMEOUT_SECS}"

log "验证公网节点高度增长与同步"
start_ts="$(date +%s)"
pass=0
while true; do
  now_ts="$(date +%s)"
  if (( now_ts - start_ts > VERIFY_TIMEOUT_SECS )); then
    break
  fi

  local_peers_hex="$(rpc_local net_peerCount | extract_result_hex || true)"
  remote_peers_hex="$(rpc_remote net_peerCount | extract_result_hex || true)"
  local_block_hex="$(rpc_local zero_getLatestBlock | extract_block_hex || true)"
  remote_block_hex="$(rpc_remote zero_getLatestBlock | extract_block_hex || true)"

  if [[ -n "${local_peers_hex}" && -n "${remote_peers_hex}" && -n "${local_block_hex}" && -n "${remote_block_hex}" ]]; then
    local_peers_dec="$(hex_to_dec "${local_peers_hex}")"
    remote_peers_dec="$(hex_to_dec "${remote_peers_hex}")"
    local_block_dec="$(hex_to_dec "${local_block_hex}")"
    remote_block_dec="$(hex_to_dec "${remote_block_hex}")"

    gap=$(( local_block_dec - remote_block_dec ))
    if (( gap < 0 )); then gap=$(( -gap )); fi

    log "sample peers(local=${local_peers_dec},remote=${remote_peers_dec}) blocks(local=${local_block_dec},remote=${remote_block_dec}) gap=${gap}"

    if (( local_peers_dec >= 1 && remote_peers_dec >= 1 && local_block_dec >= VERIFY_MIN_HEIGHT && remote_block_dec >= VERIFY_MIN_HEIGHT && gap <= VERIFY_MAX_GAP )); then
      pass=1
      break
    fi
  fi

  sleep "${VERIFY_INTERVAL_SECS}"
done

if (( pass != 1 )); then
  echo "verification failed within timeout=${VERIFY_TIMEOUT_SECS}s" >&2
  exit 1
fi

log "重置与验证完成"
log "local_pid=${LOCAL_PID} remote_pid=${REMOTE_PID}"
log "local_run_dir=${LOCAL_RUN_DIR}"
log "use scripts/node_sync_check.sh for periodic checks"
