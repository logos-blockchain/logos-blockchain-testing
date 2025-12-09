#!/bin/sh

set -e

check_binary_arch() {
  if ! command -v file >/dev/null 2>&1; then
    echo "Warning: 'file' command not available; skipping nomos-node arch check" >&2
    return
  fi
  bin_info="$(file -b /usr/bin/nomos-node 2>/dev/null || true)"
  host_arch="$(uname -m)"
  case "$bin_info" in
    *"Mach-O"*) echo "nomos-node binary is Mach-O (host bundle) but container requires Linux ELF for ${host_arch}" >&2; exit 126 ;;
    *"ELF"*) : ;;
    *) echo "nomos-node binary missing or unreadable; info='${bin_info}'" >&2; exit 126 ;;
  esac
  case "$host_arch" in
    x86_64) expected="x86-64|x86_64" ;;
    aarch64|arm64) expected="arm64|aarch64" ;;
    *) expected="" ;;
  esac
  if [ -n "$expected" ] && ! echo "$bin_info" | grep -Eqi "$expected"; then
    echo "nomos-node binary architecture mismatch: host=${host_arch}, file='${bin_info}'" >&2
    exit 126
  fi
}

check_binary_arch

export CFG_FILE_PATH="/config.yaml" \
       CFG_SERVER_ADDR="${CFG_SERVER_ADDR:-http://cfgsync:4400}" \
       CFG_HOST_IP=$(hostname -i) \
       CFG_HOST_KIND="${CFG_HOST_KIND:-validator}" \
       CFG_HOST_IDENTIFIER="${CFG_HOST_IDENTIFIER:-validator-$(hostname -i)}" \
       NOMOS_KZGRS_PARAMS_PATH="${NOMOS_KZGRS_PARAMS_PATH:-/kzgrs_test_params/kzgrs_test_params}" \
       NOMOS_TIME_BACKEND="${NOMOS_TIME_BACKEND:-monotonic}" \
       LOG_LEVEL="INFO" \
       POL_PROOF_DEV_MODE="${POL_PROOF_DEV_MODE:-true}"

# Ensure recovery directory exists to avoid early crashes in services that
# persist state.
mkdir -p /recovery

# cfgsync-server can start a little after the node container; retry until it is
# reachable instead of exiting immediately and crash-looping.
attempt=0
max_attempts=30
sleep_seconds=3
until /usr/bin/cfgsync-client; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge "$max_attempts" ]; then
    echo "cfgsync-client failed after ${max_attempts} attempts, giving up"
    exit 1
  fi
  echo "cfgsync not ready yet (attempt ${attempt}/${max_attempts}), retrying in ${sleep_seconds}s..."
  sleep "$sleep_seconds"
done

# Align bootstrap timing with executors to keep configs consistent.
sed -i "s/prolonged_bootstrap_period: .*/prolonged_bootstrap_period: '3.000000000'/" /config.yaml

exec /usr/bin/nomos-node /config.yaml
