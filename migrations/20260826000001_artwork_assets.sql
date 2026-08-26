-- Promote artwork to a first-class asset resource with content-addressed
-- versioning. Tracks and albums reference artwork_assets by id; storage keys
-- remain on artwork_assets only (never in client-facing contracts).

DROP MATERIALIZED VIEW IF EXISTS mv_discovery_pool;

CREATE TABLE artwork_assets (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    storage_key     TEXT NOT NULL,
    content_type    TEXT NOT NULL,
    checksum_sha256 CHAR(64) NOT NULL,
    width_px        INTEGER,
    height_px       INTEGER,
    size_bytes      BIGINT NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_artwork_assets_checksum_hex
        CHECK (checksum_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT chk_artwork_assets_size_non_negative
        CHECK (size_bytes >= 0),
    CONSTRAINT chk_artwork_assets_dimensions
        CHECK (
            (width_px IS NULL AND height_px IS NULL)
            OR (width_px IS NOT NULL AND height_px IS NOT NULL
                AND width_px > 0 AND height_px > 0)
        )
);

CREATE UNIQUE INDEX uq_artwork_assets_storage_key
    ON artwork_assets(storage_key);

CREATE UNIQUE INDEX uq_artwork_assets_checksum_sha256
    ON artwork_assets(checksum_sha256);

CREATE TRIGGER trg_artwork_assets_updated_at
    BEFORE UPDATE ON artwork_assets
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

COMMENT ON TABLE artwork_assets IS
    'First-class artwork blobs. Client contracts expose id + checksum only.';
COMMENT ON COLUMN artwork_assets.storage_key IS
    'Internal library-relative object key; never returned on canopy.v1.';
COMMENT ON COLUMN artwork_assets.checksum_sha256 IS
    'Lowercase SHA-256 hex of artwork bytes; ArtworkRef.content_hash.';

ALTER TABLE tracks
    ADD COLUMN artwork_id UUID REFERENCES artwork_assets(id) ON DELETE SET NULL;

ALTER TABLE albums
    ADD COLUMN artwork_id UUID REFERENCES artwork_assets(id) ON DELETE SET NULL;

CREATE INDEX idx_tracks_artwork_id
    ON tracks(artwork_id)
    WHERE artwork_id IS NOT NULL;

CREATE INDEX idx_albums_artwork_id
    ON albums(artwork_id)
    WHERE artwork_id IS NOT NULL;

-- Backfill assets from legacy storage keys shaped like
-- artwork/{aa}/{bb}/{sha256}.{jpg|jpeg|png}.
WITH legacy_keys AS (
    SELECT DISTINCT artwork_storage_key AS storage_key
    FROM (
        SELECT artwork_storage_key FROM tracks
        UNION ALL
        SELECT artwork_storage_key FROM albums
    ) keys
    WHERE artwork_storage_key IS NOT NULL
      AND artwork_storage_key ~ '^artwork/[0-9a-fA-F]{2}/[0-9a-fA-F]{2}/[0-9a-fA-F]{64}\.(jpg|jpeg|png)$'
)
INSERT INTO artwork_assets (storage_key, content_type, checksum_sha256, size_bytes)
SELECT
    storage_key,
    CASE
        WHEN lower(storage_key) LIKE '%.png' THEN 'image/png'
        ELSE 'image/jpeg'
    END,
    lower(regexp_replace(substring(storage_key FROM '[^/]+$'), '\.[^.]+$', '')),
    0
FROM legacy_keys
ON CONFLICT (storage_key) DO NOTHING;

UPDATE tracks AS t
SET artwork_id = a.id
FROM artwork_assets AS a
WHERE t.artwork_storage_key IS NOT NULL
  AND t.artwork_storage_key = a.storage_key
  AND t.artwork_id IS NULL;

UPDATE albums AS al
SET artwork_id = a.id
FROM artwork_assets AS a
WHERE al.artwork_storage_key IS NOT NULL
  AND al.artwork_storage_key = a.storage_key
  AND al.artwork_id IS NULL;

-- Discovery pool now projects opaque artwork identity + content hash.
CREATE MATERIALIZED VIEW mv_discovery_pool AS
SELECT
    t.id             AS track_id,
    t.title          AS track_title,
    a.name           AS artist_name,
    al.title         AS album_title,
    t.duration_ms    AS track_duration_ms,
    t.is_explicit    AS track_explicit,
    COALESCE(t_art.id, al_art.id) AS artwork_id,
    COALESCE(t_art.checksum_sha256, al_art.checksum_sha256) AS artwork_content_hash,
    aa.codec         AS asset_codec,
    aa.content_type  AS asset_content_type,
    aa.size_bytes    AS asset_size_bytes,
    random()         AS shuffle_rank,
    DENSE_RANK() OVER (ORDER BY a.name) AS artist_group
FROM tracks t
JOIN artists a ON t.artist_id = a.id
JOIN albums al ON t.album_id = al.id
LEFT JOIN artwork_assets t_art ON t.artwork_id = t_art.id
LEFT JOIN artwork_assets al_art ON al.artwork_id = al_art.id
LEFT JOIN LATERAL (
    SELECT codec, content_type, size_bytes
    FROM audio_assets
    WHERE track_id = t.id
    ORDER BY
        CASE codec
            WHEN 'opus' THEN 0
            WHEN 'mp4' THEN 1
            WHEN 'mp3' THEN 2
            WHEN 'flac' THEN 3
            ELSE 4
        END,
        codec
    LIMIT 1
) aa ON TRUE
WHERE t.is_explicit = FALSE
  AND t.visibility = 'release_safe'
  AND t.ingest_status = 'ready'
ORDER BY shuffle_rank;

CREATE UNIQUE INDEX idx_mv_discovery_pool_track_id
    ON mv_discovery_pool(track_id);

CREATE INDEX idx_mv_discovery_pool_shuffle
    ON mv_discovery_pool(shuffle_rank, artist_group);

CREATE INDEX idx_mv_discovery_pool_explicit
    ON mv_discovery_pool(track_explicit, shuffle_rank);
