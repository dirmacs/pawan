#!/usr/bin/env bash
set -euo pipefail

MIN_FREE_GB="${PAWAN_RELEASE_MIN_FREE_GB:-8}"
CHECK_PATH="${PAWAN_RELEASE_CHECK_PATH:-.}"

echo "Release storage preflight"
echo "check_path=${CHECK_PATH}"
echo "min_free_gb=${MIN_FREE_GB}"

if command -v duf >/dev/null 2>&1; then
  echo "--- duf ${CHECK_PATH} ---"
  duf --only local "${CHECK_PATH}"
else
  echo "--- df ${CHECK_PATH} ---"
  df -h "${CHECK_PATH}"
fi

if command -v dust >/dev/null 2>&1; then
  echo "--- dust top-level ---"
  dust -d 1 "${CHECK_PATH}"
else
  echo "--- du top-level ---"
  du -h -d 1 "${CHECK_PATH}"
fi

available_kb="$(df -Pk "${CHECK_PATH}" | awk 'NR == 2 {print $4}')"
required_kb="$((MIN_FREE_GB * 1024 * 1024))"

if [ "${available_kb}" -lt "${required_kb}" ]; then
  echo "::error::release storage preflight failed: ${available_kb} KiB free, need ${required_kb} KiB. Inspect with duf/dust; do not delete systemd-managed release binaries."
  exit 1
fi

echo "storage_preflight=ok available_kib=${available_kb} required_kib=${required_kb}"

echo "Safety: this script is read-only. It never runs cargo clean, rm, or package-manager cleanup."
