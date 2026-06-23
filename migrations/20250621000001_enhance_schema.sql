-- Migration: enhance Canopy schema with production-ready tables, indexes, constraints,
-- and materialized views.
--
-- This migration adds:
--   - Genre support (genres + track_genres junction)
--   - Playlist track membership (playlist_tracks)
--   - User favorites (user_favorites)
--   - Provider tracking (provider_tracks)
--   - Enhanced user/artist/album/track metadata fields
--   - CHECK constraints for data integrity
--   - Updated-at triggers for auditability
--   - A pre-shuffled discovery materialized view
--   - Strategic indexes for common query patterns

-- ---------------------------------------------------------------------------
-- Enforce pg_trgm extension (already created in previous migration, but
-- idempotent to be safe).
-- ---------------------------------------------------------------------------
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- ---------------------------------------------------------------------------
-- Helper: updated_at trigger function
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION canopy_update_modified_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ---------------------------------------------------------------------------
-- Users -- enhance with identity and personalization fields
-- ---------------------------------------------------------------------------
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS email        VARCHAR(320) UNIQUE,
    ADD COLUMN IF NOT EXISTS display_name  VARCHAR(256),
    ADD COLUMN IF NOT EXISTS avatar_key    TEXT,
    ADD COLUMN IF NOT EXISTS updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW();

DROP TRIGGER IF EXISTS trg_users_updated_at ON users;
CREATE TRIGGER trg_users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

CREATE INDEX IF NOT EXISTS idx_users_email ON users(email) WHERE email IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Genres -- normalized genre taxonomy
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS genres (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name       VARCHAR(128) NOT NULL UNIQUE,
    parent_id  UUID REFERENCES genres(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_genres_parent_id ON genres(parent_id);

-- ---------------------------------------------------------------------------
-- Track genres -- many-to-many junction
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS track_genres (
    track_id  UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    genre_id  UUID NOT NULL REFERENCES genres(id) ON DELETE CASCADE,
    PRIMARY KEY (track_id, genre_id)
);

CREATE INDEX IF NOT EXISTS idx_track_genres_genre_id ON track_genres(genre_id);

-- ---------------------------------------------------------------------------
-- Artists -- enhance with metadata fields
-- ---------------------------------------------------------------------------
ALTER TABLE artists
    ADD COLUMN IF NOT EXISTS bio         TEXT,
    ADD COLUMN IF NOT EXISTS country      VARCHAR(128),
    ADD COLUMN IF NOT EXISTS artwork_key  TEXT,
    ADD COLUMN IF NOT EXISTS updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW();

DROP TRIGGER IF EXISTS trg_artists_updated_at ON artists;
CREATE TRIGGER trg_artists_updated_at
    BEFORE UPDATE ON artists
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

-- ---------------------------------------------------------------------------
-- Albums -- enhance with metadata fields
-- ---------------------------------------------------------------------------
ALTER TABLE albums
    ADD COLUMN IF NOT EXISTS description   TEXT,
    ADD COLUMN IF NOT EXISTS total_tracks   INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW();

DROP TRIGGER IF EXISTS trg_albums_updated_at ON albums;
CREATE TRIGGER trg_albums_updated_at
    BEFORE UPDATE ON albums
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

-- Add CHECK constraint for total_tracks (must be >= 0)
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_albums_total_tracks_positive'
        AND conrelid = 'albums'::regclass
    ) THEN
        ALTER TABLE albums
            ADD CONSTRAINT chk_albums_total_tracks_positive
            CHECK (total_tracks >= 0);
    END IF;
END $$;

-- ---------------------------------------------------------------------------
-- Tracks -- enhance with track metadata
-- ---------------------------------------------------------------------------
ALTER TABLE tracks
    ADD COLUMN IF NOT EXISTS track_number   INTEGER,
    ADD COLUMN IF NOT EXISTS disc_number    INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS genre          VARCHAR(128),
    ADD COLUMN IF NOT EXISTS lyrics         TEXT,
    ADD COLUMN IF NOT EXISTS composer       VARCHAR(512),
    ADD COLUMN IF NOT EXISTS updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW();

DROP TRIGGER IF EXISTS trg_tracks_updated_at ON tracks;
CREATE TRIGGER trg_tracks_updated_at
    BEFORE UPDATE ON tracks
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

-- Add CHECK constraints for track_number and disc_number
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_tracks_track_number_positive'
        AND conrelid = 'tracks'::regclass
    ) THEN
        ALTER TABLE tracks
            ADD CONSTRAINT chk_tracks_track_number_positive
            CHECK (track_number IS NULL OR track_number > 0);
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_tracks_disc_number_positive'
        AND conrelid = 'tracks'::regclass
    ) THEN
        ALTER TABLE tracks
            ADD CONSTRAINT chk_tracks_disc_number_positive
            CHECK (disc_number > 0);
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_tracks_duration_positive'
        AND conrelid = 'tracks'::regclass
    ) THEN
        ALTER TABLE tracks
            ADD CONSTRAINT chk_tracks_duration_positive
            CHECK (duration_ms >= 0);
    END IF;
END $$;

-- ---------------------------------------------------------------------------
-- Audio assets -- add CHECK constraint for size_bytes
-- ---------------------------------------------------------------------------
ALTER TABLE audio_assets
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

DROP TRIGGER IF EXISTS trg_audio_assets_updated_at ON audio_assets;
CREATE TRIGGER trg_audio_assets_updated_at
    BEFORE UPDATE ON audio_assets
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_audio_assets_size_positive'
        AND conrelid = 'audio_assets'::regclass
    ) THEN
        ALTER TABLE audio_assets
            ADD CONSTRAINT chk_audio_assets_size_positive
            CHECK (size_bytes >= 0);
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_audio_assets_checksum_length'
        AND conrelid = 'audio_assets'::regclass
    ) THEN
        ALTER TABLE audio_assets
            ADD CONSTRAINT chk_audio_assets_checksum_length
            CHECK (LENGTH(checksum_sha256) = 64);
    END IF;
END $$;

-- ---------------------------------------------------------------------------
-- Playlist tracks -- many-to-many junction with ordering
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS playlist_tracks (
    playlist_id  UUID NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    track_id     UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL DEFAULT 0,
    added_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (playlist_id, track_id)
);

CREATE INDEX IF NOT EXISTS idx_playlist_tracks_track_id ON playlist_tracks(track_id);
CREATE INDEX IF NOT EXISTS idx_playlist_tracks_position  ON playlist_tracks(playlist_id, position);

-- ---------------------------------------------------------------------------
-- User favorites -- many-to-many user<->track
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS user_favorites (
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    track_id   UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    favorited_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, track_id)
);

CREATE INDEX IF NOT EXISTS idx_user_favorites_track_id ON user_favorites(track_id);

-- ---------------------------------------------------------------------------
-- Provider tracks -- tracks the source of every ingested track
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS provider_tracks (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    track_id          UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    provider          VARCHAR(64) NOT NULL,
    provider_track_id VARCHAR(256) NOT NULL,
    raw_metadata      JSONB,
    fetched_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(provider, provider_track_id)
);

CREATE INDEX IF NOT EXISTS idx_provider_tracks_track_id ON provider_tracks(track_id);
CREATE INDEX IF NOT EXISTS idx_provider_tracks_provider  ON provider_tracks(provider, provider_track_id);

DROP TRIGGER IF EXISTS trg_provider_tracks_updated_at ON provider_tracks;
CREATE TRIGGER trg_provider_tracks_updated_at
    BEFORE UPDATE ON provider_tracks
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

-- ---------------------------------------------------------------------------
-- Playback history -- enhanced with partition-ready design
-- (For now we use a single table; in production, monthly partitioning
--  via pg_partman keeps the hot table small while retaining historical data.)
-- ---------------------------------------------------------------------------
ALTER TABLE playback_history
    ADD COLUMN IF NOT EXISTS source VARCHAR(64) DEFAULT 'discovery',
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

DROP TRIGGER IF EXISTS trg_playback_history_updated_at ON playback_history;
CREATE TRIGGER trg_playback_history_updated_at
    BEFORE UPDATE ON playback_history
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_playback_completion_pct_range'
        AND conrelid = 'playback_history'::regclass
    ) THEN
        ALTER TABLE playback_history
            ADD CONSTRAINT chk_playback_completion_pct_range
            CHECK (completion_pct >= 0.0 AND completion_pct <= 1.0);
    END IF;
END $$;

-- BRIN index for time-series playback_history (very efficient for append-only data)
CREATE INDEX IF NOT EXISTS idx_playback_history_played_at_brin
    ON playback_history USING BRIN (played_at);

-- Partial index for recent playback (hot query: "recently played")
-- NOTE: PostgreSQL requires IMMUTABLE functions in index predicates.
-- NOW() is STABLE, so we use a regular index instead and let the query
-- planner filter by the WHERE clause at query time.
CREATE INDEX IF NOT EXISTS idx_playback_history_recent
    ON playback_history(user_id, played_at DESC);

-- ---------------------------------------------------------------------------
-- Strategic indexes for hot query patterns
-- ---------------------------------------------------------------------------

-- Composite index for the common browse query (tracks + artist + album + assets)
CREATE INDEX IF NOT EXISTS idx_tracks_created_at_covering
    ON tracks(created_at) INCLUDE (id, title, artist_id, album_id, duration_ms, is_explicit, artwork_key);

-- Partial index for explicit tracks (common filter)
CREATE INDEX IF NOT EXISTS idx_tracks_not_explicit
    ON tracks(created_at) INCLUDE (id, title, artist_id, album_id, duration_ms, artwork_key)
    WHERE is_explicit = FALSE;

-- Index for track search by title (already has GIN trigram, but a B-tree
-- for prefix matching is faster for exact-match lookups)
CREATE INDEX IF NOT EXISTS idx_tracks_title_btree ON tracks(title);
CREATE INDEX IF NOT EXISTS idx_artists_name_btree ON artists(name);
CREATE INDEX IF NOT EXISTS idx_albums_title_btree ON albums(title);

-- ---------------------------------------------------------------------------
-- Discovery materialized view: pre-shuffled, diversified pool
--
-- This is the production path for the DiscoveryService. Instead of running
-- ORDER BY random() on every request, we maintain a pre-shuffled pool that
-- is refreshed periodically. The view includes:
--   - a random ordering column
--   - artist grouping to help diversity
--   - the full denormalized MediaItem data
--
-- Refresh strategy: REFRESH MATERIALIZED VIEW CONCURRENTLY on a cron
-- schedule (e.g., every 5 minutes) so the refresh does not block reads.
--
-- To use CONCURRENTLY, we need a unique index on the view.
-- ---------------------------------------------------------------------------

CREATE MATERIALIZED VIEW IF NOT EXISTS mv_discovery_pool AS
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
    -- Randomize per-row (stable for the lifetime of the refresh)
    random()         AS shuffle_rank,
    -- Artist grouping for diversity-aware selection
    DENSE_RANK() OVER (ORDER BY a.name) AS artist_group
FROM tracks t
JOIN artists a     ON t.artist_id = a.id
JOIN albums al     ON t.album_id = al.id
LEFT JOIN audio_assets aa ON aa.track_id = t.id
WHERE t.is_explicit = FALSE  -- default: exclude explicit from discovery
ORDER BY shuffle_rank;

-- Unique index required for CONCURRENTLY refresh
CREATE UNIQUE INDEX IF NOT EXISTS idx_mv_discovery_pool_track_id
    ON mv_discovery_pool(track_id);

-- Index for fast artist-diversity queries
CREATE INDEX IF NOT EXISTS idx_mv_discovery_pool_shuffle
    ON mv_discovery_pool(shuffle_rank, artist_group);

-- Index for explicit-filtered queries
CREATE INDEX IF NOT EXISTS idx_mv_discovery_pool_explicit
    ON mv_discovery_pool(track_explicit, shuffle_rank);

-- ---------------------------------------------------------------------------
-- Full-text search materialized view (optional, for advanced search)
-- ---------------------------------------------------------------------------
CREATE MATERIALIZED VIEW IF NOT EXISTS mv_catalog_search AS
SELECT
    t.id,
    t.title,
    a.name AS artist_name,
    al.title AS album_title,
    -- Weighted tsvector: title (A), artist (B), album (C)
    setweight(to_tsvector('english', coalesce(t.title, '')), 'A') ||
    setweight(to_tsvector('english', coalesce(a.name, '')), 'B') ||
    setweight(to_tsvector('english', coalesce(al.title, '')), 'C') AS search_vector
FROM tracks t
JOIN artists a ON t.artist_id = a.id
JOIN albums al ON t.album_id = al.id;

CREATE UNIQUE INDEX IF NOT EXISTS idx_mv_catalog_search_id
    ON mv_catalog_search(id);

CREATE INDEX IF NOT EXISTS idx_mv_catalog_search_vector
    ON mv_catalog_search USING GIN(search_vector);

-- ---------------------------------------------------------------------------
-- Comment: document the schema for discoverability
-- ---------------------------------------------------------------------------
COMMENT ON TABLE users              IS 'End-user identities for personalization and playback history.';
COMMENT ON TABLE artists            IS 'Performing artists in the catalog.';
COMMENT ON TABLE albums             IS 'Albums containing one or more tracks.';
COMMENT ON TABLE tracks             IS 'Individual audio tracks (songs, pieces, etc.).';
COMMENT ON TABLE audio_assets       IS 'Encoded audio files per track (one per codec). Stored in RustFS; metadata only here.';
COMMENT ON TABLE licenses           IS 'License records for provider-ingested tracks.';
COMMENT ON TABLE playlists          IS 'User-created playlists.';
COMMENT ON TABLE playlist_tracks    IS 'Track membership within playlists, with ordering.';
COMMENT ON TABLE user_favorites     IS 'User<->track favorite relationships.';
COMMENT ON TABLE provider_tracks    IS 'Tracks the source/provider of every ingested track for deduplication and sync.';
COMMENT ON TABLE playback_history   IS 'Historical playback events per user. Partition-ready for time-series scaling.';
COMMENT ON TABLE genres             IS 'Normalized genre taxonomy (supports hierarchical genres via parent_id).';
COMMENT ON TABLE track_genres       IS 'Many-to-many junction: tracks <-> genres.';
COMMENT ON MATERIALIZED VIEW mv_discovery_pool IS 'Pre-shuffled, diversified discovery pool. Refresh periodically.';
COMMENT ON MATERIALIZED VIEW mv_catalog_search IS 'Full-text search materialization. Refresh when catalog changes.';
