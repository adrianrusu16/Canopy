#!/usr/bin/env bash
# Safe lifecycle for the persistent local Canopy dependency stack.
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
compose_file="$repo_root/docker-compose.yml"
compose_project="canopy"
readonly persistent_volumes=(
  canopy-postgres-data
  canopy_redis_data
  canopy_rustfs_data
)

usage() {
  cat >&2 <<'EOF'
usage: ./scripts/canopy.sh up [compose arguments...]
       ./scripts/canopy.sh down [compose arguments...]
       ./scripts/canopy.sh status
       ./scripts/canopy.sh destroy-data --confirm-delete-canopy-data

up/down never remove data. destroy-data is the only supported deletion path
and requires the flag above plus an interactive typed confirmation.
EOF
}

die() {
  echo "canopy lifecycle: $1" >&2
  exit "${2:-1}"
}

require_docker() {
  command -v docker >/dev/null 2>&1 || die "docker is required"
  docker compose version >/dev/null
}

compose() {
  docker compose -p "$compose_project" -f "$compose_file" "$@"
}

ensure_persistent_volumes() {
  local volume
  for volume in "${persistent_volumes[@]}"; do
    docker volume create "$volume" >/dev/null
  done
}

destroy_data() {
  [[ "${1:-}" == "--confirm-delete-canopy-data" && "$#" == "1" ]] || die \
    "refusing to delete data; use destroy-data --confirm-delete-canopy-data"
  [[ -t 0 && -t 1 ]] || die "refusing non-interactive data deletion"

  echo "This permanently removes all Canopy PostgreSQL, Redis, and RustFS data."
  read -r -p "Type DELETE CANOPY DATA to continue: " confirmation
  [[ "$confirmation" == "DELETE CANOPY DATA" ]] || die "confirmation did not match; nothing was deleted"

  compose down --remove-orphans
  local volume
  for volume in "${persistent_volumes[@]}"; do
    docker volume rm "$volume"
  done
}

main() {
  [[ "$#" -ge 1 ]] || {
    usage
    exit 2
  }
  local command="$1"
  shift
  require_docker
  cd "$repo_root"

  case "$command" in
    up)
      ensure_persistent_volumes
      compose up -d "$@"
      ;;
    down)
      for argument in "$@"; do
        [[ "${argument%%=*}" != "--volumes" && "$argument" != "-v" ]] || die \
          "down never deletes data; use destroy-data for the guarded deletion path"
      done
      compose down --remove-orphans "$@"
      ;;
    status)
      [[ "$#" == "0" ]] || die "status does not accept arguments"
      compose ps
      ;;
    destroy-data)
      destroy_data "$@"
      ;;
    *)
      usage
      exit 2
      ;;
  esac
}

main "$@"
