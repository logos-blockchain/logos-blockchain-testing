#!/usr/bin/env bash
set -euo pipefail

VERSION="${VERSION:?VERSION build arg missing}"
CIRCUITS_PLATFORM="${CIRCUITS_PLATFORM:?CIRCUITS_PLATFORM build arg missing}"
CIRCUITS_OVERRIDE="${CIRCUITS_OVERRIDE:-}"

mkdir -p /opt/circuits

select_circuits_source() {
  if [ -n "${CIRCUITS_OVERRIDE}" ] && [ -e "/workspace/${CIRCUITS_OVERRIDE}" ]; then
    echo "/workspace/${CIRCUITS_OVERRIDE}"
    return 0
  fi
  if [ -e "/workspace/tests/kzgrs/kzgrs_test_params" ]; then
    echo "/workspace/tests/kzgrs/kzgrs_test_params"
    return 0
  fi
  return 1
}

if CIRCUITS_PATH="$(select_circuits_source)"; then
  echo "Using prebuilt circuits bundle from ${CIRCUITS_PATH#/workspace/}"
  if [ -d "${CIRCUITS_PATH}" ]; then
    cp -R "${CIRCUITS_PATH}/." /opt/circuits
  else
    cp "${CIRCUITS_PATH}" /opt/circuits/
  fi
fi

TARGET_ARCH="$(uname -m)"

expect_arch_pattern() {
  case "$1" in
    x86_64) echo "x86-64|x86_64" ;;
    aarch64|arm64) echo "arm64|aarch64" ;;
    *) echo "$1" ;;
  esac
}

require_linux_execs=0

check_linux_exec() {
  local path="$1"
  if [ ! -f "${path}" ]; then
    return 0
  fi
  local info
  info="$(file -b "${path}" 2>/dev/null || true)"
  case "${info}" in
    *ELF*) : ;;
    *)
      echo "Circuits executable is not ELF: ${path} (${info}); forcing circuits download"
      require_linux_execs=1
      return 0
      ;;
  esac

  local pattern
  pattern="$(expect_arch_pattern "${TARGET_ARCH}")"
  if [ -n "${pattern}" ] && ! echo "${info}" | grep -Eqi "${pattern}"; then
    echo "Circuits executable arch mismatch: ${path} (${info}); forcing circuits download"
    require_linux_execs=1
  fi
}

check_linux_exec /opt/circuits/zksign/witness_generator
check_linux_exec /opt/circuits/pol/witness_generator

if [ -f "/opt/circuits/prover" ]; then
  PROVER_INFO="$(file -b /opt/circuits/prover || true)"
  case "${TARGET_ARCH}" in
    x86_64) EXPECT_ARCH="x86-64" ;;
    aarch64|arm64) EXPECT_ARCH="aarch64" ;;
    *) EXPECT_ARCH="${TARGET_ARCH}" ;;
  esac
  if [ -n "${PROVER_INFO}" ] && ! echo "${PROVER_INFO}" | grep -qi "${EXPECT_ARCH}"; then
    echo "Circuits prover architecture (${PROVER_INFO}) does not match target ${TARGET_ARCH}; rebuilding rapidsnark binaries"
    RAPIDSNARK_FORCE_REBUILD=1 \
      scripts/build-rapidsnark.sh /opt/circuits
  fi
fi

if [ "${require_linux_execs}" -eq 1 ] || [ ! -f "/opt/circuits/pol/verification_key.json" ]; then
  echo "Downloading ${VERSION} circuits bundle for ${CIRCUITS_PLATFORM}"
  NOMOS_CIRCUITS_PLATFORM="${CIRCUITS_PLATFORM}" \
  NOMOS_CIRCUITS_REBUILD_RAPIDSNARK=1 \
  RAPIDSNARK_BUILD_GMP=1 \
    scripts/setup-nomos-circuits.sh "${VERSION}" "/opt/circuits"
fi

