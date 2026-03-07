#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${ROOT_DIR}/target/release/zerochain"
BASE_DIR="${HOME}/.zerochain/testnet"
LOG_DIR="${BASE_DIR}/logs"
PID_DIR="${BASE_DIR}/pids"

DEFAULT_NODE_COUNT=3
NETWORK_ID=10087
BASE_HTTP_PORT=8545
BASE_WS_PORT=8645

usage() {
    cat <<'EOF'
Usage:
  scripts/testnet.sh start [--nodes N] [--clean-data]
  scripts/testnet.sh stop
  scripts/testnet.sh status
  scripts/testnet.sh logs [NODE_INDEX]

Examples:
  scripts/testnet.sh start
  scripts/testnet.sh start --nodes 4
  scripts/testnet.sh start --nodes 5 --clean-data
  scripts/testnet.sh status
  scripts/testnet.sh logs 2
  scripts/testnet.sh stop
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
        echo "cargo is not available. Please install Rust toolchain or build ${BIN} manually."
        exit 1
    fi
}

ensure_dirs() {
    mkdir -p "${BASE_DIR}" "${LOG_DIR}" "${PID_DIR}"
}

is_running_pid() {
    local pid="$1"
    kill -0 "${pid}" 2>/dev/null
}

stop_nodes() {
    ensure_dirs
    local found=0

    for pid_file in "${PID_DIR}"/node*.pid; do
        [[ -f "${pid_file}" ]] || continue
        found=1

        local node_name pid
        node_name="$(basename "${pid_file}" .pid)"
        pid="$(cat "${pid_file}" 2>/dev/null || true)"

        if [[ -n "${pid}" ]] && is_running_pid "${pid}"; then
            kill "${pid}" 2>/dev/null || true
            for _ in {1..30}; do
                if ! is_running_pid "${pid}"; then
                    break
                fi
                sleep 0.1
            done
            if is_running_pid "${pid}"; then
                kill -9 "${pid}" 2>/dev/null || true
            fi
            echo "Stopped ${node_name} (pid=${pid})"
        else
            echo "Removed stale pid for ${node_name}"
        fi
        rm -f "${pid_file}"
    done

    if [[ "${found}" -eq 0 ]]; then
        echo "No managed testnet nodes are running."
    fi
}

clean_logs() {
    ensure_dirs
    rm -f "${LOG_DIR}"/node*.log
    echo "Cleaned logs under ${LOG_DIR}"
}

start_nodes() {
    local node_count="$1"
    local clean_data="$2"

    if ! [[ "${node_count}" =~ ^[0-9]+$ ]] || [[ "${node_count}" -lt 1 ]]; then
        echo "Invalid node count: ${node_count}"
        exit 1
    fi

    ensure_binary
    ensure_dirs
    stop_nodes
    clean_logs

    if [[ "${clean_data}" == "true" ]]; then
        for i in $(seq 1 "${node_count}"); do
            rm -rf "${BASE_DIR}/node${i}"
        done
        echo "Cleaned node data directories for node1..node${node_count}"
    fi

    echo "Starting ${node_count} testnet nodes (network_id=${NETWORK_ID})"
    for i in $(seq 1 "${node_count}"); do
        local node_dir http_port ws_port log_file pid_file pid
        node_dir="${BASE_DIR}/node${i}"
        http_port=$((BASE_HTTP_PORT + i - 1))
        ws_port=$((BASE_WS_PORT + i - 1))
        log_file="${LOG_DIR}/node${i}.log"
        pid_file="${PID_DIR}/node${i}.pid"

        mkdir -p "${node_dir}"

        nohup "${BIN}" \
            -d "${node_dir}" \
            --network testnet \
            run \
            --http-port "${http_port}" \
            --ws-port "${ws_port}" \
            --rpc-network-id "${NETWORK_ID}" \
            --chain-id "${NETWORK_ID}" \
            >"${log_file}" 2>&1 &

        pid=$!
        echo "${pid}" > "${pid_file}"
        sleep 0.5

        if is_running_pid "${pid}"; then
            echo "Started node${i}: pid=${pid}, http=${http_port}, ws=${ws_port}, log=${log_file}"
        else
            echo "Failed to start node${i}. Last log output:"
            tail -n 40 "${log_file}" || true
            exit 1
        fi
    done

    for _round in 1 2 3; do
        sleep 1
        local quick_exit=0
        for i in $(seq 1 "${node_count}"); do
            local pid_file pid log_file
            pid_file="${PID_DIR}/node${i}.pid"
            log_file="${LOG_DIR}/node${i}.log"
            pid="$(cat "${pid_file}" 2>/dev/null || true)"

            if [[ -z "${pid}" ]] || ! is_running_pid "${pid}"; then
                quick_exit=1
                echo "node${i} exited during startup. Last log output:"
                tail -n 40 "${log_file}" || true
            fi
        done
        if [[ "${quick_exit}" -ne 0 ]]; then
            echo "At least one node exited unexpectedly during startup."
            exit 1
        fi
    done

    echo "All ${node_count} testnet nodes are running."
}

status_nodes() {
    ensure_dirs
    local found=0

    printf "%-8s %-8s %-10s %-8s %-8s %s\n" "NODE" "PID" "STATUS" "HTTP" "WS" "LOG"
    for pid_file in "${PID_DIR}"/node*.pid; do
        [[ -f "${pid_file}" ]] || continue
        found=1

        local name index pid status http_port ws_port
        name="$(basename "${pid_file}" .pid)"
        index="${name#node}"
        pid="$(cat "${pid_file}" 2>/dev/null || true)"
        http_port=$((BASE_HTTP_PORT + index - 1))
        ws_port=$((BASE_WS_PORT + index - 1))

        if [[ -n "${pid}" ]] && is_running_pid "${pid}"; then
            status="running"
        else
            status="stopped"
        fi

        printf "%-8s %-8s %-10s %-8s %-8s %s\n" \
            "${name}" "${pid:-n/a}" "${status}" "${http_port}" "${ws_port}" "${LOG_DIR}/${name}.log"
    done

    if [[ "${found}" -eq 0 ]]; then
        echo "No managed testnet nodes found."
    fi
}

show_logs() {
    ensure_dirs
    local index="${1:-1}"
    local log_file="${LOG_DIR}/node${index}.log"
    if [[ ! -f "${log_file}" ]]; then
        echo "Log file not found: ${log_file}"
        exit 1
    fi
    tail -n 80 "${log_file}"
}

command="${1:-start}"
shift || true

case "${command}" in
    start)
        node_count="${DEFAULT_NODE_COUNT}"
        clean_data="false"
        while [[ $# -gt 0 ]]; do
            case "$1" in
                --nodes|-n)
                    node_count="${2:-}"
                    shift 2
                    ;;
                ''|*[!0-9]*)
                    if [[ "$1" == "--clean-data" ]]; then
                        clean_data="true"
                        shift
                    else
                        echo "Unknown option for start: $1"
                        usage
                        exit 1
                    fi
                    ;;
                *)
                    node_count="$1"
                    shift
                    ;;
            esac
        done
        start_nodes "${node_count}" "${clean_data}"
        ;;
    stop)
        stop_nodes
        ;;
    status)
        status_nodes
        ;;
    logs)
        show_logs "${1:-1}"
        ;;
    help|-h|--help)
        usage
        ;;
    *)
        echo "Unknown command: ${command}"
        usage
        exit 1
        ;;
esac
