#!/usr/bin/env bash
# Copy personal artwork into an existing local-integration media tree and
# register artwork_assets (id + SHA-256) so canopy.v1 ArtworkRef can be emitted.
# Does not wipe users or audio. Safe to re-run.
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
media_root="$state_root/media/library"

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

python3 - "$repo_root/fixtures/catalog.json" "$media_root" "$sql_file" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

catalog = json.loads(Path(sys.argv[1]).read_text())
media_root = Path(sys.argv[2])
out = Path(sys.argv[3])

def sql_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"

def file_meta(storage_key: str) -> tuple[str, int, str] | None:
    path = media_root / storage_key
    if not path.is_file():
        return None
    data = path.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    content_type = "image/png" if path.suffix.lower() == ".png" else "image/jpeg"
    return digest, len(data), content_type

lines = ["BEGIN;"]
seen_keys: set[str] = set()
missing: list[str] = []

for track in catalog["tracks"]:
    if not str(track.get("provider_id", "")).startswith("personal-"):
        continue
    title = track["title"]
    artist = track["artist"]
    album = track["album"]
    track_key = track["artwork_storage_key"]
    album_key = track["album_artwork_storage_key"]

    for storage_key in (track_key, album_key):
        if storage_key in seen_keys:
            continue
        seen_keys.add(storage_key)
        meta = file_meta(storage_key)
        if meta is None:
            missing.append(storage_key)
            continue
        checksum, size_bytes, content_type = meta
        # Insert by storage_key when new; if checksum already exists under another
        # key, reuse that row (identical bytes = same ArtworkRef identity).
        lines.append(
            "WITH existing AS ("
            "  SELECT id FROM artwork_assets"
            f"  WHERE storage_key = {sql_quote(storage_key)}"
            f"     OR checksum_sha256 = {sql_quote(checksum)}"
            "  ORDER BY CASE WHEN storage_key = "
            f"    {sql_quote(storage_key)} THEN 0 ELSE 1 END"
            "  LIMIT 1"
            "), inserted AS ("
            "  INSERT INTO artwork_assets (storage_key, content_type, checksum_sha256, size_bytes)"
            f"  SELECT {sql_quote(storage_key)}, {sql_quote(content_type)},"
            f"         {sql_quote(checksum)}, {size_bytes}"
            "  WHERE NOT EXISTS (SELECT 1 FROM existing)"
            "  RETURNING id"
            ") "
            "SELECT id FROM inserted "
            "UNION ALL "
            "SELECT id FROM existing;"
        )

    track_meta = file_meta(track_key)
    album_meta = file_meta(album_key)
    lines.append(
        "UPDATE tracks tr SET "
        f"artwork_storage_key = {sql_quote(track_key)}, "
        "artwork_id = ("
        "  SELECT id FROM artwork_assets"
        f"  WHERE storage_key = {sql_quote(track_key)}"
        + (
            f" OR checksum_sha256 = {sql_quote(track_meta[0])}"
            if track_meta is not None
            else ""
        )
        + "  ORDER BY CASE WHEN storage_key = "
        f"{sql_quote(track_key)} THEN 0 ELSE 1 END LIMIT 1"
        ") "
        "FROM artists ar "
        f"WHERE tr.artist_id = ar.id AND tr.title = {sql_quote(title)} "
        f"AND ar.name = {sql_quote(artist)};"
    )
    lines.append(
        "UPDATE albums al SET "
        f"artwork_storage_key = {sql_quote(album_key)}, "
        "artwork_id = ("
        "  SELECT id FROM artwork_assets"
        f"  WHERE storage_key = {sql_quote(album_key)}"
        + (
            f" OR checksum_sha256 = {sql_quote(album_meta[0])}"
            if album_meta is not None
            else ""
        )
        + "  ORDER BY CASE WHEN storage_key = "
        f"{sql_quote(album_key)} THEN 0 ELSE 1 END LIMIT 1"
        ") "
        "FROM artists ar "
        f"WHERE al.artist_id = ar.id AND al.title = {sql_quote(album)} "
        f"AND ar.name = {sql_quote(artist)};"
    )

if missing:
    print(
        "apply-personal-artwork: missing files (skipped): " + ", ".join(missing),
        file=sys.stderr,
    )

lines += [
    "REFRESH MATERIALIZED VIEW mv_discovery_pool;",
    "SELECT count(*) AS artwork_assets FROM artwork_assets;",
    "SELECT count(*) AS tracks_with_artwork_id FROM tracks WHERE artwork_id IS NOT NULL;",
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
