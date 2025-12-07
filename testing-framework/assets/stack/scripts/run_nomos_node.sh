#!/bin/sh

set -e

export CFG_FILE_PATH="/config.yaml" \
       CFG_SERVER_ADDR="${CFG_SERVER_ADDR:-http://cfgsync:4400}" \
       CFG_HOST_IP=$(hostname -i) \
       CFG_HOST_KIND="${CFG_HOST_KIND:-validator}" \
       CFG_HOST_IDENTIFIER="${CFG_HOST_IDENTIFIER:-validator-$(hostname -i)}" \
       NOMOS_KZGRS_PARAMS_PATH="${NOMOS_KZGRS_PARAMS_PATH:-/opt/circuits/kzgrs_test_params/pol/proving_key.zkey}" \
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
