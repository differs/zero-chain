#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${ROOT_DIR}/target/release/zerochain"
DATA_DIR="${HOME}/.zerochain/mainnet"
LOG_FILE="${DATA_DIR}/mainnet.log"
PID_FILE="${DATA_DIR}/mainnet.pid"

HTTP_PORT=8545
WS_PORT=8546

usage() {
    cat <<'EOF'
Usage:
  scripts/mainnet.sh start [--mine] [--coinbase ZER0x...] [--clean-data] [--bootnode enode://...] [--disable-local-miner] [--rpc-rate-limit-per-minute N] [--rpc-auth-token TOKEN]
  scripts/mainnet.sh stop
  scripts/mainnet.sh status
  scripts/mainnet.sh logs

Examples:
  scripts/mainnet.sh start
  scripts/mainnet.sh start --mine --coinbase ZER0x0000000000000000000000000000000000000001
  scripts/mainnet.sh start --bootnode enode://bootnode-1@1.2.3.4:30303
  scripts/mainnet.sh start --mine --disable-local-miner --rpc-rate-limit-per-minute 0
  scripts/mainnet.sh status
  scripts/mainnet.sh logs
  scripts/mainnet.sh stop
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

is_running_pid() {
    local pid="$1"
    kill -0 "${pid}" 2>/dev/null
}

start_node() {
    local mine="$1"
    local coinbase="$2"
    local clean_data="$3"
    local disable_local_miner="$4"
    local rpc_rate_limit_per_minute="$5"
    local rpc_auth_token="$6"
    shift 6
    local bootnodes=("$@")

    ensure_binary
    mkdir -p "${DATA_DIR}"

    if [[ -f "${PID_FILE}" ]]; then
        local old_pid
        old_pid="$(cat "${PID_FILE}" 2>/dev/null || true)"
        if [[ -n "${old_pid}" ]] && is_running_pid "${old_pid}"; then
            echo "mainnet node already running (pid=${old_pid})"
            exit 1
        fi
        rm -f "${PID_FILE}"
    fi

    if [[ "${clean_data}" == "true" ]]; then
        rm -rf "${DATA_DIR}"
        mkdir -p "${DATA_DIR}"
        echo "cleaned mainnet data directory"
    fi

    local args=(
        -d "${DATA_DIR}"
        --network mainnet
        run
        --http-port "${HTTP_PORT}"
        --ws-port "${WS_PORT}"
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

    nohup "${BIN}" "${args[@]}" >"${LOG_FILE}" 2>&1 &
    local pid=$!
    echo "${pid}" > "${PID_FILE}"

    sleep 1
    if is_running_pid "${pid}"; then
        echo "started mainnet node pid=${pid}"
        echo "rpc=http://127.0.0.1:${HTTP_PORT} ws=ws://127.0.0.1:${WS_PORT}"
        echo "mine=${mine} disable_local_miner=${disable_local_miner} rpc_rate_limit_per_minute=${rpc_rate_limit_per_minute:-default}"
        if [[ ${#bootnodes[@]} -gt 0 ]]; then
            echo "bootnodes=${bootnodes[*]}"
        fi
        echo "log=${LOG_FILE}"
    else
        echo "failed to start mainnet node"
        tail -n 80 "${LOG_FILE}" || true
        rm -f "${PID_FILE}"
        exit 1
    fi
}

stop_node() {
    if [[ ! -f "${PID_FILE}" ]]; then
        echo "mainnet node not running"
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
        echo "stopped mainnet node pid=${pid}"
    else
        echo "stale pid removed"
    fi
    rm -f "${PID_FILE}"
}

status_node() {
    if [[ -f "${PID_FILE}" ]]; then
        local pid
        pid="$(cat "${PID_FILE}" 2>/dev/null || true)"
        if [[ -n "${pid}" ]] && is_running_pid "${pid}"; then
            echo "mainnet node running pid=${pid}"
            return 0
        fi
    fi
    echo "mainnet node stopped"
}

show_logs() {
    if [[ ! -f "${LOG_FILE}" ]]; then
        echo "log file missing: ${LOG_FILE}"
        return 0
    fi
    tail -n 120 "${LOG_FILE}"
}

cmd="${1:-start}"
shift || true

case "${cmd}" in
    start)
        mine="false"
        coinbase=""
        clean_data="false"
        disable_local_miner="false"
        rpc_rate_limit_per_minute=""
        rpc_auth_token=""
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
                *)
                    echo "unknown option: $1"
                    usage
                    exit 1
                    ;;
            esac
        done
        start_node "${mine}" "${coinbase}" "${clean_data}" "${disable_local_miner}" "${rpc_rate_limit_per_minute}" "${rpc_auth_token}" "${bootnodes[@]}"
        ;;
    stop)
        stop_node
        ;;
    status)
        status_node
        ;;
    logs)
        show_logs
        ;;
    help|-h|--help)
        usage
        ;;
    *)
        echo "unknown command: ${cmd}"
        usage
        exit 1
        ;;
esac
