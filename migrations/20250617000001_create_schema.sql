-- Migration: create initial Canopy schema
-- Based on the ER diagram and domain model from README.md

-- Users (end-user identities for playback history and personalization)
CREATE TABLE IF NOT EXISTS users (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Licenses (every track ingested through a provider carries a license record)
CREATE TABLE IF NOT EXISTS licenses (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    license_type      VARCHAR(64) NOT NULL,
    source_url        TEXT,
    attribution_text  TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Artists
CREATE TABLE IF NOT EXISTS artists (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name       VARCHAR(512) NOT NULL,
    sort_name  VARCHAR(512) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_artists_sort_name ON artists(sort_name);

-- Albums
CREATE TABLE IF NOT EXISTS albums (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title         VARCHAR(512) NOT NULL,
    artist_id     UUID NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
    release_year  INTEGER,
    artwork_key   TEXT,                    -- RustFS object key for album artwork
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_albums_artist_id ON albums(artist_id);

-- Tracks
CREATE TABLE IF NOT EXISTS tracks (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title         VARCHAR(512) NOT NULL,
    artist_id     UUID NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
    album_id      UUID REFERENCES albums(id) ON DELETE SET NULL,
    duration_ms   INTEGER NOT NULL DEFAULT 0,
    license_id    UUID REFERENCES licenses(id) ON DELETE SET NULL,
    is_explicit   BOOLEAN NOT NULL DEFAULT FALSE,
    artwork_key   TEXT,                    -- override album artwork if set
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_tracks_artist_id ON tracks(artist_id);
CREATE INDEX idx_tracks_album_id  ON tracks(album_id);

-- Audio assets (one per codec per track)
CREATE TABLE IF NOT EXISTS audio_assets (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    track_id        UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    codec           VARCHAR(16) NOT NULL,
    content_type    VARCHAR(128) NOT NULL,
    object_key      TEXT NOT NULL,          -- RustFS path within the media bucket
    size_bytes      BIGINT NOT NULL DEFAULT 0,
    checksum_sha256 VARCHAR(64) NOT NULL,
    duration_ms     BIGINT NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(track_id, codec)
);

CREATE INDEX idx_audio_assets_track_id ON audio_assets(track_id);

-- Playlists (basic table; track membership is a future expansion)
CREATE TABLE IF NOT EXISTS playlists (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title      VARCHAR(512) NOT NULL,
    user_id    UUID REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Playback history (needed for discovery exclusion and personalization)
CREATE TABLE IF NOT EXISTS playback_history (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    track_id        UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    played_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    duration_ms     INTEGER NOT NULL DEFAULT 0,
    completion_pct  REAL NOT NULL DEFAULT 0.0,
    UNIQUE(user_id, track_id, played_at)
);

CREATE INDEX idx_playback_history_user_id ON playback_history(user_id);
CREATE INDEX idx_playback_history_user_track ON playback_history(user_id, track_id);

-- Sessions (lightweight playback state for the PgSessionRepository stub)
CREATE TABLE IF NOT EXISTS sessions (
    id                VARCHAR(36) PRIMARY KEY,
    current_media_id  VARCHAR(36),
    position_ms       INTEGER NOT NULL DEFAULT 0,
    playback_speed    DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    is_playing        BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Trigram index support for future search (pg_trgm extension)
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- GIN trigram indexes on searchable text columns
CREATE INDEX idx_tracks_title_trgm ON tracks USING GIN (title gin_trgm_ops);
CREATE INDEX idx_artists_name_trgm ON artists USING GIN (name gin_trgm_ops);
CREATE INDEX idx_albums_title_trgm ON albums USING GIN (title gin_trgm_ops);
