#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT_DIR}"

ALLOW_TAG="REDLINE_ALLOW"
FAILED=0

check_pattern() {
  local title="$1"
  local pattern="$2"
  local path="$3"
  local matches filtered

  matches="$(rg -n -P --no-heading --color never "${pattern}" "${path}" \
    --glob '!**/tests/**' --glob '!**/target/**' || true)"

  if [[ -z "${matches}" ]]; then
    return 0
  fi

  filtered="$(printf '%s\n' "${matches}" | grep -v "${ALLOW_TAG}" || true)"
  if [[ -z "${filtered}" ]]; then
    return 0
  fi

  FAILED=1
  echo "❌ ${title}"
  printf '%s\n' "${filtered}"
  echo
}

echo "🔒 Engineering redline guard (no silent fallback)"

check_pattern \
  "禁止配置错误后回退默认配置继续运行" \
  'Self::try_new\((ApiConfig|RpcConfig)::default\(\)\)' \
  crates/zeroapi/src

check_pattern \
  "禁止以 fallback 语义吞掉关键故障（default/mem）" \
  'fallback to (default|mem)' \
  crates

check_pattern \
  "禁止 enode 端口解析失败后默认 30303" \
  'parse\(\)\.unwrap_or\(\s*30303\s*\)' \
  crates/zeronet/src/discovery.rs

if [[ "${FAILED}" -ne 0 ]]; then
  echo "🚫 Redline guard failed. Resolve or annotate with ${ALLOW_TAG} (must include rationale)."
  exit 1
fi

echo "✅ Redline guard passed"
