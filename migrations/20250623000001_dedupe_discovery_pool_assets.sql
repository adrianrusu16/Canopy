-- Migration: make catalog materializations choose one representative asset per track.
--
-- Tracks may have multiple audio assets (one per codec). The discovery pool is
-- keyed by track_id, so the materialized view must not join all assets directly
-- or a multi-codec track will violate the unique index required for refreshes.

DROP MATERIALIZED VIEW IF EXISTS mv_discovery_pool;

CREATE MATERIALIZED VIEW mv_discovery_pool AS
SELECT
    t.id             AS track_id,
    t.title          AS track_title,
    a.name           AS artist_name,
    al.title         AS album_title,
    t.duration_ms    AS track_duration_ms,
    t.is_explicit    AS track_explicit,
    COALESCE(t.artwork_key, al.artwork_key) AS artwork_key,
    aa.codec         AS asset_codec,
    aa.content_type  AS asset_content_type,
    aa.size_bytes    AS asset_size_bytes,
    random()         AS shuffle_rank,
    DENSE_RANK() OVER (ORDER BY a.name) AS artist_group
FROM tracks t
JOIN artists a ON t.artist_id = a.id
JOIN albums al ON t.album_id = al.id
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
ORDER BY shuffle_rank;

CREATE UNIQUE INDEX idx_mv_discovery_pool_track_id
    ON mv_discovery_pool(track_id);

CREATE INDEX idx_mv_discovery_pool_shuffle
    ON mv_discovery_pool(shuffle_rank, artist_group);

CREATE INDEX idx_mv_discovery_pool_explicit
    ON mv_discovery_pool(track_explicit, shuffle_rank);

COMMENT ON MATERIALIZED VIEW mv_discovery_pool IS
    'Pre-shuffled, diversified discovery pool with one representative asset per track. Refresh periodically.';