-- Profile-owned durable library, likes, and preferences.
-- Anonymous sessions are operational only and never own these rows.

CREATE TABLE IF NOT EXISTS profile_library_items (
    profile_id  UUID NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    track_id    UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    added_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    source      VARCHAR(64) NOT NULL DEFAULT 'manual',
    PRIMARY KEY (profile_id, track_id)
);

CREATE INDEX IF NOT EXISTS idx_profile_library_items_profile_added_at
    ON profile_library_items(profile_id, added_at DESC);

CREATE INDEX IF NOT EXISTS idx_profile_library_items_track_id
    ON profile_library_items(track_id);

DROP TRIGGER IF EXISTS trg_profile_library_items_updated_at ON profile_library_items;
CREATE TRIGGER trg_profile_library_items_updated_at
    BEFORE UPDATE ON profile_library_items
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

CREATE TABLE IF NOT EXISTS profile_track_likes (
    profile_id  UUID NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    track_id    UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    liked_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (profile_id, track_id)
);

CREATE INDEX IF NOT EXISTS idx_profile_track_likes_profile_liked_at
    ON profile_track_likes(profile_id, liked_at DESC);

CREATE INDEX IF NOT EXISTS idx_profile_track_likes_track_id
    ON profile_track_likes(track_id);

DROP TRIGGER IF EXISTS trg_profile_track_likes_updated_at ON profile_track_likes;
CREATE TRIGGER trg_profile_track_likes_updated_at
    BEFORE UPDATE ON profile_track_likes
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

CREATE TABLE IF NOT EXISTS profile_preferences (
    profile_id   UUID PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE,
    preferences  JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

DROP TRIGGER IF EXISTS trg_profile_preferences_updated_at ON profile_preferences;
CREATE TRIGGER trg_profile_preferences_updated_at
    BEFORE UPDATE ON profile_preferences
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();
