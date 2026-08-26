#!/usr/bin/env bash
# Copy personal artwork into an existing local-integration media tree and
# patch artwork_storage_key on matching tracks/albums. Does not wipe users
# or audio. Safe to re-run.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

state_root="$repo_root/target/local-integration"
runtime_env="$state_root/runtime.env"
compose_file="$repo_root/docker-compose.local-integration.yml"
compose_project="canopy-local-integration"
src_tracks="$repo_root/fixtures/media/personal/artwork/tracks"
src_albums="$repo_root/fixtures/media/personal/artwork/albums"
dst_tracks="$state_root/media/library/artwork/tracks/personal"
dst_albums="$state_root/media/library/artwork/albums/personal"

die() {
  echo "apply-personal-artwork: $1" >&2
  exit 1
}

[[ -d "$src_tracks" && -d "$src_albums" ]] \
  || die "missing fixtures/media/personal/artwork — run ./scripts/download-personal-artwork.sh first"
[[ -d "$state_root/media/library" ]] \
  || die "local-integration media root missing; start the environment once with ./scripts/local-integration.sh up"
[[ -f "$runtime_env" ]] \
  || die "missing $runtime_env"

mkdir -p "$dst_tracks" "$dst_albums"
cp "$src_tracks"/*.jpg "$dst_tracks/"
cp "$src_albums"/*.jpg "$dst_albums/"
chmod -R a+rX "$state_root/media/library/artwork"

echo "tracks_copied=$(find "$dst_tracks" -maxdepth 1 -type f -name '*.jpg' | wc -l)"
echo "albums_copied=$(find "$dst_albums" -maxdepth 1 -type f -name '*.jpg' | wc -l)"
echo "audio_still=$(find "$state_root/media/library/audio/tracks/personal" -maxdepth 1 -type f -name '*.mp3' 2>/dev/null | wc -l)"

sql_file="$(mktemp)"
trap 'rm -f "$sql_file"' EXIT

python3 - "$repo_root/fixtures/catalog.json" "$sql_file" <<'PY'
import json, sys
from pathlib import Path

catalog = json.loads(Path(sys.argv[1]).read_text())
out = Path(sys.argv[2])
lines = ["BEGIN;"]
for t in catalog["tracks"]:
    if not str(t.get("provider_id", "")).startswith("personal-"):
        continue
    title = t["title"].replace("'", "''")
    artist = t["artist"].replace("'", "''")
    album = t["album"].replace("'", "''")
    track_key = t["artwork_storage_key"].replace("'", "''")
    album_key = t["album_artwork_storage_key"].replace("'", "''")
    lines.append(
        "UPDATE tracks tr SET artwork_storage_key = "
        f"'{track_key}' FROM artists ar "
        f"WHERE tr.artist_id = ar.id AND tr.title = '{title}' AND ar.name = '{artist}';"
    )
    lines.append(
        "UPDATE albums al SET artwork_storage_key = "
        f"'{album_key}' FROM artists ar "
        f"WHERE al.artist_id = ar.id AND al.title = '{album}' AND ar.name = '{artist}';"
    )
lines += [
    "SELECT count(*) AS tracks_with_personal_art "
    "FROM tracks WHERE artwork_storage_key LIKE 'artwork/tracks/personal/%';",
    "SELECT count(*) AS albums_with_personal_art "
    "FROM albums WHERE artwork_storage_key LIKE 'artwork/albums/personal/%';",
    "SELECT count(*) AS accounts FROM accounts;",
    "COMMIT;",
]
out.write_text("\n".join(lines) + "\n")
PY

set -a
# shellcheck disable=SC1090
source "$runtime_env"
set +a

docker compose -p "$compose_project" -f "$compose_file" exec -T postgres \
  psql --username canopy_local --dbname canopy_local --no-psqlrc --set ON_ERROR_STOP=1 \
  <"$sql_file"

echo "apply-personal-artwork: done (users and audio untouched)"
