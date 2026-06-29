-- Local-media ownership, visibility, ingest lifecycle, and storage terminology.

DROP MATERIALIZED VIEW IF EXISTS mv_catalog_search;
DROP MATERIALIZED VIEW IF EXISTS mv_discovery_pool;

ALTER TABLE audio_assets RENAME COLUMN object_key TO storage_key;
ALTER TABLE albums RENAME COLUMN artwork_key TO artwork_storage_key;
ALTER TABLE tracks RENAME COLUMN artwork_key TO artwork_storage_key;

ALTER TABLE licenses
    ADD COLUMN review_status TEXT NOT NULL DEFAULT 'pending',
    ADD COLUMN reviewed_at TIMESTAMPTZ,
    ADD CONSTRAINT chk_licenses_review_status
        CHECK (review_status IN ('pending', 'approved', 'rejected')),
    ADD CONSTRAINT chk_licenses_approved_reviewed
        CHECK (review_status <> 'approved' OR reviewed_at IS NOT NULL);

ALTER TABLE tracks
    ADD COLUMN visibility TEXT NOT NULL DEFAULT 'quarantined',
    ADD COLUMN ingest_status TEXT NOT NULL DEFAULT 'quarantined',
    ADD COLUMN owner_profile_id UUID REFERENCES profiles(id) ON DELETE RESTRICT,
    ADD COLUMN composition_license_id UUID REFERENCES licenses(id) ON DELETE RESTRICT,
    ADD COLUMN recording_license_id UUID REFERENCES licenses(id) ON DELETE RESTRICT,
    ADD CONSTRAINT chk_tracks_visibility
        CHECK (visibility IN ('personal', 'release_safe', 'quarantined')),
    ADD CONSTRAINT chk_tracks_ingest_status
        CHECK (ingest_status IN ('pending', 'ready', 'quarantined')),
    ADD CONSTRAINT chk_tracks_personal_owner
        CHECK (visibility <> 'personal' OR owner_profile_id IS NOT NULL),
    ADD CONSTRAINT chk_tracks_release_shape
        CHECK (
            visibility <> 'release_safe'
            OR (
                owner_profile_id IS NULL
                AND ingest_status = 'ready'
                AND composition_license_id IS NOT NULL
                AND recording_license_id IS NOT NULL
            )
        ),
    ADD CONSTRAINT chk_tracks_quarantine_state
        CHECK (visibility <> 'quarantined' OR ingest_status = 'quarantined');

CREATE TABLE instance_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    owner_profile_id UUID UNIQUE REFERENCES profiles(id) ON DELETE RESTRICT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO instance_settings (singleton) VALUES (TRUE);

CREATE TRIGGER trg_instance_settings_updated_at
    BEFORE UPDATE ON instance_settings
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

CREATE OR REPLACE FUNCTION canopy_enforce_release_safe_track()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    composition_approved BOOLEAN;
    recording_approved BOOLEAN;
BEGIN
    IF NEW.visibility = 'release_safe' THEN
        SELECT review_status = 'approved' AND reviewed_at IS NOT NULL
        INTO composition_approved
        FROM licenses
        WHERE id = NEW.composition_license_id;

        SELECT review_status = 'approved' AND reviewed_at IS NOT NULL
        INTO recording_approved
        FROM licenses
        WHERE id = NEW.recording_license_id;

        IF NOT COALESCE(composition_approved, FALSE)
            OR NOT COALESCE(recording_approved, FALSE)
        THEN
            RAISE EXCEPTION
                'release-safe tracks require approved composition and recording licenses'
                USING ERRCODE = '23514',
                      CONSTRAINT = 'chk_tracks_release_licenses_approved';
        END IF;
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_tracks_enforce_release_safe
    BEFORE INSERT OR UPDATE OF visibility, composition_license_id, recording_license_id
    ON tracks
    FOR EACH ROW
    EXECUTE FUNCTION canopy_enforce_release_safe_track();

CREATE OR REPLACE FUNCTION canopy_quarantine_tracks_on_license_revocation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.review_status = 'approved'
        AND OLD.reviewed_at IS NOT NULL
        AND (NEW.review_status <> 'approved' OR NEW.reviewed_at IS NULL)
    THEN
        UPDATE tracks
        SET visibility = 'quarantined',
            ingest_status = 'quarantined'
        WHERE visibility = 'release_safe'
          AND (
              composition_license_id = NEW.id
              OR recording_license_id = NEW.id
          );
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_licenses_quarantine_release_safe_tracks
    AFTER UPDATE OF review_status, reviewed_at
    ON licenses
    FOR EACH ROW
    EXECUTE FUNCTION canopy_quarantine_tracks_on_license_revocation();

CREATE INDEX idx_tracks_visibility_ingest_created_at
    ON tracks(visibility, ingest_status, created_at);

CREATE INDEX idx_tracks_owner_profile_id
    ON tracks(owner_profile_id)
    WHERE owner_profile_id IS NOT NULL;

CREATE INDEX idx_audio_assets_storage_key
    ON audio_assets(storage_key);

CREATE MATERIALIZED VIEW mv_discovery_pool AS
SELECT
    t.id             AS track_id,
    t.title          AS track_title,
    a.name           AS artist_name,
    al.title         AS album_title,
    t.duration_ms    AS track_duration_ms,
    t.is_explicit    AS track_explicit,
    COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
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
  AND t.visibility = 'release_safe'
  AND t.ingest_status = 'ready'
ORDER BY shuffle_rank;

CREATE UNIQUE INDEX idx_mv_discovery_pool_track_id
    ON mv_discovery_pool(track_id);

CREATE INDEX idx_mv_discovery_pool_shuffle
    ON mv_discovery_pool(shuffle_rank, artist_group);

CREATE INDEX idx_mv_discovery_pool_explicit
    ON mv_discovery_pool(track_explicit, shuffle_rank);

CREATE MATERIALIZED VIEW mv_catalog_search AS
SELECT
    t.id,
    t.title,
    a.name AS artist_name,
    al.title AS album_title,
    setweight(to_tsvector('english', COALESCE(t.title, '')), 'A') ||
    setweight(to_tsvector('english', COALESCE(a.name, '')), 'B') ||
    setweight(to_tsvector('english', COALESCE(al.title, '')), 'C') AS search_vector
FROM tracks t
JOIN artists a ON t.artist_id = a.id
JOIN albums al ON t.album_id = al.id
WHERE t.visibility = 'release_safe'
  AND t.ingest_status = 'ready';

CREATE UNIQUE INDEX idx_mv_catalog_search_id
    ON mv_catalog_search(id);

CREATE INDEX idx_mv_catalog_search_vector
    ON mv_catalog_search USING GIN(search_vector);

COMMENT ON TABLE instance_settings IS
    'Singleton settings for this Canopy installation, including its explicit owner profile.';
COMMENT ON TABLE audio_assets IS
    'Encoded audio files per track, addressed by validated keys in Canopy managed storage.';
COMMENT ON COLUMN audio_assets.storage_key IS
    'Relative key within Canopy managed media storage; never an absolute filesystem path.';
COMMENT ON COLUMN tracks.visibility IS
    'Catalog boundary: personal, release_safe, or quarantined.';
COMMENT ON COLUMN tracks.ingest_status IS
    'Recoverable media ingest lifecycle: pending, ready, or quarantined.';
COMMENT ON MATERIALIZED VIEW mv_discovery_pool IS
    'Release-safe ready tracks, pre-shuffled with one representative asset per track.';
COMMENT ON MATERIALIZED VIEW mv_catalog_search IS
    'Release-safe ready tracks materialized for full-text search.';
