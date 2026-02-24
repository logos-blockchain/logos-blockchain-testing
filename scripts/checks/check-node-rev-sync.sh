#!/usr/bin/env sh
set -eu

repo_root="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
versions_env="${repo_root}/versions.env"
cargo_toml="${repo_root}/Cargo.toml"
logos_git='https://github.com/logos-blockchain/logos-blockchain.git'

env_rev="$(grep -E '^LOGOS_BLOCKCHAIN_NODE_REV=[0-9a-f]{40}$' "$versions_env" | head -n1 | cut -d= -f2 || true)"
if [ -z "$env_rev" ]; then
  echo "node-rev-sync check failed: LOGOS_BLOCKCHAIN_NODE_REV is missing or invalid in versions.env" >&2
  exit 1
fi

cargo_revs="$(
  grep -oE "git *= *\"${logos_git}\" *, *rev *= *\"[0-9a-f]{40}\"" "$cargo_toml" \
    | sed -E 's/.*rev *= *"([0-9a-f]{40})"/\1/' \
    | sort -u
)"

if [ -z "$cargo_revs" ]; then
  echo "node-rev-sync check failed: no logos-blockchain git rev found in Cargo.toml" >&2
  exit 1
fi

rev_count="$(printf '%s\n' "$cargo_revs" | wc -l | tr -d ' ')"
if [ "$rev_count" -ne 1 ]; then
  echo "node-rev-sync check failed: multiple logos-blockchain revs found in Cargo.toml:" >&2
  printf '  %s\n' "$cargo_revs" >&2
  exit 1
fi

cargo_rev="$(printf '%s\n' "$cargo_revs")"
if [ "$env_rev" != "$cargo_rev" ]; then
  echo "node-rev-sync check failed:" >&2
  echo "  versions.env LOGOS_BLOCKCHAIN_NODE_REV: $env_rev" >&2
  echo "  Cargo.toml logos-blockchain rev:        $cargo_rev" >&2
  echo "  Fix: set LOGOS_BLOCKCHAIN_NODE_REV to match Cargo.toml." >&2
  exit 1
fi
