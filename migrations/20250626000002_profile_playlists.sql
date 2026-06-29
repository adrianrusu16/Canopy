-- Profile-owned durable playlists.
-- Anonymous sessions are operational only and never own these rows.

CREATE TABLE IF NOT EXISTS profile_playlists (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id  UUID NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    name        VARCHAR(256) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_profile_playlists_name_not_empty CHECK (LENGTH(BTRIM(name)) > 0)
);

CREATE INDEX IF NOT EXISTS idx_profile_playlists_profile_updated_at
    ON profile_playlists(profile_id, updated_at DESC);

DROP TRIGGER IF EXISTS trg_profile_playlists_updated_at ON profile_playlists;
CREATE TRIGGER trg_profile_playlists_updated_at
    BEFORE UPDATE ON profile_playlists
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

CREATE TABLE IF NOT EXISTS profile_playlist_tracks (
    playlist_id UUID NOT NULL REFERENCES profile_playlists(id) ON DELETE CASCADE,
    track_id    UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    added_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (playlist_id, track_id),
    CONSTRAINT chk_profile_playlist_tracks_position_non_negative CHECK (position >= 0)
);

CREATE INDEX IF NOT EXISTS idx_profile_playlist_tracks_playlist_position
    ON profile_playlist_tracks(playlist_id, position);

CREATE INDEX IF NOT EXISTS idx_profile_playlist_tracks_track_id
    ON profile_playlist_tracks(track_id);

DROP TRIGGER IF EXISTS trg_profile_playlist_tracks_updated_at ON profile_playlist_tracks;
CREATE TRIGGER trg_profile_playlist_tracks_updated_at
    BEFORE UPDATE ON profile_playlist_tracks
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();
