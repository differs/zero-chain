#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${ROOT_DIR}/target/release/zerochain"

DATA_BASE_DIR="${HOME}/.zerochain/mainnet"

DEFAULT_BOOTNODE_HTTP_PORT=8545
DEFAULT_BOOTNODE_WS_PORT=8546
DEFAULT_BOOTNODE_P2P_PORT=30303

DEFAULT_FOLLOWER_HTTP_PORT=29645
DEFAULT_FOLLOWER_WS_PORT=29646
DEFAULT_FOLLOWER_P2P_PORT=31303

DEFAULT_OBSERVER_HTTP_PORT=39745
DEFAULT_OBSERVER_WS_PORT=39746
DEFAULT_OBSERVER_P2P_PORT=32303

usage() {
    cat <<'EOF'
Usage:
  scripts/mainnet.sh start [role] [options]
  scripts/mainnet.sh stop [role]
  scripts/mainnet.sh status [role]
  scripts/mainnet.sh logs [role]

Roles:
  bootnode   Main coordinator / first node (default)
  follower   Public follower node
  observer   Read-only observer node

Options:
  --mine
  --coinbase ZER0x...
  --clean-data
  --bootnode enode://...        repeatable
  --p2p-listen-addr ADDR
  --disable-local-miner
  --rpc-rate-limit-per-minute N
  --rpc-auth-token TOKEN
  --http-port PORT
  --ws-port PORT
  --p2p-port PORT

Examples:
  scripts/mainnet.sh start bootnode --mine --coinbase ZER0x0000000000000000000000000000000000000001
  scripts/mainnet.sh start bootnode --p2p-listen-addr 127.0.0.1
  scripts/mainnet.sh start follower --bootnode enode://bootnode-1@1.2.3.4:30303
  scripts/mainnet.sh start observer --bootnode enode://bootnode-1@1.2.3.4:30303
  scripts/mainnet.sh status bootnode
  scripts/mainnet.sh logs follower
  scripts/mainnet.sh stop observer
EOF
}

ensure_binary() {
    if [[ -x "${BIN}" ]]; then
        return 0
    fi

    echo "Binary not found: ${BIN}"
    if command -v cargo >/dev/null 2>&1; then
        echo "Building zerocli release binary..."
        (cd "${ROOT_DIR}" && cargo build -p zerocli --release)
    else
        echo "cargo not available, please build ${BIN} manually."
        exit 1
    fi
}

normalize_role() {
    local role="${1:-bootnode}"
    case "${role}" in
        bootnode|follower|observer)
            printf '%s\n' "${role}"
            ;;
        *)
            echo "unknown role: ${role}" >&2
            usage >&2
            exit 1
            ;;
    esac
}

role_defaults() {
    local role="$1"
    case "${role}" in
        bootnode)
            DEFAULT_HTTP_PORT="${DEFAULT_BOOTNODE_HTTP_PORT}"
            DEFAULT_WS_PORT="${DEFAULT_BOOTNODE_WS_PORT}"
            DEFAULT_P2P_PORT="${DEFAULT_BOOTNODE_P2P_PORT}"
            DEFAULT_MINE="true"
            DEFAULT_DISABLE_LOCAL_MINER="false"
            ;;
        follower)
            DEFAULT_HTTP_PORT="${DEFAULT_FOLLOWER_HTTP_PORT}"
            DEFAULT_WS_PORT="${DEFAULT_FOLLOWER_WS_PORT}"
            DEFAULT_P2P_PORT="${DEFAULT_FOLLOWER_P2P_PORT}"
            DEFAULT_MINE="false"
            DEFAULT_DISABLE_LOCAL_MINER="false"
            ;;
        observer)
            DEFAULT_HTTP_PORT="${DEFAULT_OBSERVER_HTTP_PORT}"
            DEFAULT_WS_PORT="${DEFAULT_OBSERVER_WS_PORT}"
            DEFAULT_P2P_PORT="${DEFAULT_OBSERVER_P2P_PORT}"
            DEFAULT_MINE="false"
            DEFAULT_DISABLE_LOCAL_MINER="false"
            ;;
    esac
}

role_paths() {
    local role="$1"
    DATA_DIR="${DATA_BASE_DIR}/${role}"
    LOG_FILE="${DATA_DIR}/${role}.log"
    PID_FILE="${DATA_DIR}/${role}.pid"
}

is_running_pid() {
    local pid="$1"
    kill -0 "${pid}" 2>/dev/null
}

wait_rpc_ready() {
    local port="$1"
    local timeout_secs="${2:-20}"
    local elapsed=0
    while (( elapsed < timeout_secs )); do
        if curl -fsS --max-time 2 \
            -H 'Content-Type: application/json' \
            -d '{"jsonrpc":"2.0","method":"net_version","params":[],"id":1}' \
            "http://127.0.0.1:${port}" >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done
    return 1
}

start_node() {
    local role="$1"
    local mine="$2"
    local coinbase="$3"
    local clean_data="$4"
    local disable_local_miner="$5"
    local rpc_rate_limit_per_minute="$6"
    local rpc_auth_token="$7"
    local p2p_listen_addr="$8"
    local http_port="$9"
    local ws_port="${10}"
    local p2p_port="${11}"
    shift 11
    local bootnodes=("$@")

    ensure_binary
    role_paths "${role}"
    mkdir -p "${DATA_DIR}"

    if [[ -f "${PID_FILE}" ]]; then
        local old_pid
        old_pid="$(cat "${PID_FILE}" 2>/dev/null || true)"
        if [[ -n "${old_pid}" ]] && is_running_pid "${old_pid}"; then
            echo "${role} node already running (pid=${old_pid})"
            exit 1
        fi
        rm -f "${PID_FILE}"
    fi

    if [[ "${clean_data}" == "true" ]]; then
        rm -rf "${DATA_DIR}"
        mkdir -p "${DATA_DIR}"
        echo "cleaned ${role} data directory"
    fi

    local args=(
        -d "${DATA_DIR}"
        --network mainnet
        run
        --http-port "${http_port}"
        --ws-port "${ws_port}"
        --p2p-listen-addr "${p2p_listen_addr}"
        --p2p-listen-port "${p2p_port}"
    )

    if [[ "${mine}" == "true" ]]; then
        args+=(--mine)
    fi
    if [[ "${disable_local_miner}" == "true" ]]; then
        args+=(--disable-local-miner)
    fi
    if [[ -n "${coinbase}" ]]; then
        args+=(--coinbase "${coinbase}" --rpc-coinbase "${coinbase}")
    fi
    if [[ -n "${rpc_rate_limit_per_minute}" ]]; then
        args+=(--rpc-rate-limit-per-minute "${rpc_rate_limit_per_minute}")
    fi
    if [[ -n "${rpc_auth_token}" ]]; then
        args+=(--rpc-auth-token "${rpc_auth_token}")
    fi
    for bootnode in "${bootnodes[@]}"; do
        args+=(--bootnode "${bootnode}")
    done

    if command -v setsid >/dev/null 2>&1; then
        setsid "${BIN}" "${args[@]}" >"${LOG_FILE}" 2>&1 < /dev/null &
    else
        nohup "${BIN}" "${args[@]}" >"${LOG_FILE}" 2>&1 < /dev/null &
    fi
    local pid=$!
    echo "${pid}" > "${PID_FILE}"

    if is_running_pid "${pid}" && wait_rpc_ready "${http_port}" 20; then
        echo "started ${role} node pid=${pid}"
        echo "rpc=http://127.0.0.1:${http_port} ws=ws://127.0.0.1:${ws_port} p2p=${p2p_listen_addr}:${p2p_port}"
        echo "mine=${mine} disable_local_miner=${disable_local_miner} rpc_rate_limit_per_minute=${rpc_rate_limit_per_minute:-default}"
        if [[ "${role}" == "bootnode" ]]; then
            echo "bootnode_enode_hint=enode://BOOTNODE_PEER_ID@${p2p_listen_addr}:${p2p_port}"
        fi
        if [[ ${#bootnodes[@]} -gt 0 ]]; then
            echo "bootnodes=${bootnodes[*]}"
        fi
        echo "log=${LOG_FILE}"
    else
        echo "failed to start ${role} node"
        tail -n 80 "${LOG_FILE}" || true
        if is_running_pid "${pid}"; then
            kill "${pid}" 2>/dev/null || true
            sleep 1
            if is_running_pid "${pid}"; then
                kill -9 "${pid}" 2>/dev/null || true
            fi
        fi
        rm -f "${PID_FILE}"
        exit 1
    fi
}

stop_node() {
    local role="$1"
    role_paths "${role}"
    if [[ ! -f "${PID_FILE}" ]]; then
        echo "${role} node not running"
        return 0
    fi

    local pid
    pid="$(cat "${PID_FILE}" 2>/dev/null || true)"
    if [[ -n "${pid}" ]] && is_running_pid "${pid}"; then
        kill "${pid}" 2>/dev/null || true
        for _ in {1..40}; do
            if ! is_running_pid "${pid}"; then
                break
            fi
            sleep 0.1
        done
        if is_running_pid "${pid}"; then
            kill -9 "${pid}" 2>/dev/null || true
        fi
        echo "stopped ${role} node pid=${pid}"
    else
        echo "stale ${role} pid removed"
    fi
    rm -f "${PID_FILE}"
}

status_node() {
    local role="$1"
    role_paths "${role}"
    if [[ -f "${PID_FILE}" ]]; then
        local pid
        pid="$(cat "${PID_FILE}" 2>/dev/null || true)"
        if [[ -n "${pid}" ]] && is_running_pid "${pid}"; then
            echo "${role} node running pid=${pid}"
            return 0
        fi
    fi
    echo "${role} node stopped"
}

show_logs() {
    local role="$1"
    role_paths "${role}"
    if [[ ! -f "${LOG_FILE}" ]]; then
        echo "log file missing: ${LOG_FILE}"
        return 0
    fi
    tail -n 120 "${LOG_FILE}"
}

cmd="${1:-start}"
shift || true

role="bootnode"
if [[ $# -gt 0 && "${1}" != --* ]]; then
    role="$(normalize_role "$1")"
    shift
fi

role_defaults "${role}"

case "${cmd}" in
    start)
        mine="${DEFAULT_MINE}"
        coinbase=""
        clean_data="false"
        disable_local_miner="${DEFAULT_DISABLE_LOCAL_MINER}"
        rpc_rate_limit_per_minute=""
        rpc_auth_token=""
        p2p_listen_addr="0.0.0.0"
        http_port="${DEFAULT_HTTP_PORT}"
        ws_port="${DEFAULT_WS_PORT}"
        p2p_port="${DEFAULT_P2P_PORT}"
        bootnodes=()
        while [[ $# -gt 0 ]]; do
            case "$1" in
                --mine)
                    mine="true"
                    shift
                    ;;
                --coinbase)
                    coinbase="${2:-}"
                    shift 2
                    ;;
                --clean-data)
                    clean_data="true"
                    shift
                    ;;
                --bootnode)
                    bootnodes+=("${2:-}")
                    shift 2
                    ;;
                --p2p-listen-addr)
                    p2p_listen_addr="${2:-}"
                    shift 2
                    ;;
                --disable-local-miner)
                    disable_local_miner="true"
                    shift
                    ;;
                --rpc-rate-limit-per-minute)
                    rpc_rate_limit_per_minute="${2:-}"
                    shift 2
                    ;;
                --rpc-auth-token)
                    rpc_auth_token="${2:-}"
                    shift 2
                    ;;
                --http-port)
                    http_port="${2:-}"
                    shift 2
                    ;;
                --ws-port)
                    ws_port="${2:-}"
                    shift 2
                    ;;
                --p2p-port)
                    p2p_port="${2:-}"
                    shift 2
                    ;;
                *)
                    echo "unknown option: $1" >&2
                    usage >&2
                    exit 1
                    ;;
            esac
        done
        start_node "${role}" "${mine}" "${coinbase}" "${clean_data}" "${disable_local_miner}" "${rpc_rate_limit_per_minute}" "${rpc_auth_token}" "${p2p_listen_addr}" "${http_port}" "${ws_port}" "${p2p_port}" "${bootnodes[@]}"
        ;;
    stop)
        stop_node "${role}"
        ;;
    status)
        status_node "${role}"
        ;;
    logs)
        show_logs "${role}"
        ;;
    help|-h|--help)
        usage
        ;;
    *)
        echo "unknown command: ${cmd}" >&2
        usage >&2
        exit 1
        ;;
esac
