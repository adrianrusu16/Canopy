#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
compose_file="$repo_root/docker-compose.streaming-test.yml"
compose_project="${CANOPY_STREAM_TEST_COMPOSE_PROJECT:-canopy-stream-test}"
media_root="$(mktemp -d -t canopy-stream-test.XXXXXX)"
managed_stack=0

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
      compose logs --no-color postgres nginx
    fi
    # PostgreSQL uses tmpfs here; no Docker volumes need to be removed.
    compose down --remove-orphans
  fi
  rm -rf -- "$media_root"
  exit "$status"
}

trap cleanup EXIT
cd "$repo_root"

command -v docker >/dev/null 2>&1 || {
  echo "docker is required for the streaming integration test" >&2
  exit 1
}
docker compose version >/dev/null

mkdir -p "$media_root/library/audio/aa/bb"
cp "$repo_root/fixtures/media/test-tone.mp3" \
  "$media_root/library/audio/aa/bb/stream-test.mp3"

export CANOPY_STREAM_TEST_MEDIA_ROOT="$media_root"
export CANOPY_STREAM_TEST_DATABASE_URL=\
"postgres://canopy_stream_test:canopy_stream_test@127.0.0.1:55433/canopy_stream_test"

compose config --quiet
managed_stack=1
compose up -d --wait postgres nginx

openapi_document="$(curl --fail --silent --show-error http://127.0.0.1:18080/openapi.json)"
grep -Fq '"openapi": "3.1.0"' <<<"$openapi_document"
grep -Fq '"/stream/{capability}"' <<<"$openapi_document"
if grep -Fq '"/canopy.' <<<"$openapi_document"; then
  echo "OpenAPI must not expose gRPC methods as HTTP paths" >&2
  exit 1
fi

cargo test -p canopy-server --features pg --test streaming_integration \
  nginx_serves_ranges_and_rechecks_revoked_policy -- \
  --ignored --exact --test-threads=1
