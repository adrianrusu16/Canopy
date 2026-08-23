#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
compose_file="$repo_root/docker-compose.test.yml"
compose_project="${CANOPY_TEST_COMPOSE_PROJECT:-canopy-pg-test}"
postgres_port="${CANOPY_TEST_POSTGRES_PORT:-55432}"
managed_stack=0


if [[ "$compose_project" == "canopy" ]]; then
  echo "CANOPY_TEST_COMPOSE_PROJECT must not target the persistent Canopy stack" >&2
  exit 2
fi
compose() {
  docker compose -p "$compose_project" -f "$compose_file" "$@"
}

cleanup() {
  status=$?
  trap - EXIT
  set +e

  if [[ "$managed_stack" == "1" ]]; then
    if [[ "$status" != "0" ]]; then
      compose ps
      compose logs --no-color postgres
    fi
    # PostgreSQL uses tmpfs here; no Docker volumes need to be removed.
    compose down --remove-orphans
  fi

  exit "$status"
}

trap cleanup EXIT
cd "$repo_root"

if [[ -z "${CANOPY_TEST_DATABASE_URL:-}" ]]; then
  command -v docker >/dev/null 2>&1 || {
    echo "docker is required when CANOPY_TEST_DATABASE_URL is not set" >&2
    exit 1
  }
  docker compose version >/dev/null
  compose config --quiet

  managed_stack=1
  compose up -d --wait postgres
  export CANOPY_TEST_DATABASE_URL="postgres://canopy_test:canopy_test@127.0.0.1:${postgres_port}/canopy_test"
fi

cargo test --workspace --features canopy-server/pg -- --test-threads=1
