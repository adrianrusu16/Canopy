-- Align durable playback history with real logged-in profiles.
-- Anonymous sessions are operational only and never own backend history.

ALTER TABLE playback_history DROP CONSTRAINT IF EXISTS playback_history_user_id_fkey;
ALTER TABLE playback_history DROP CONSTRAINT IF EXISTS playback_history_user_id_track_id_played_at_key;
DROP INDEX IF EXISTS idx_playback_history_user_id;
DROP INDEX IF EXISTS idx_playback_history_user_track;

ALTER TABLE playback_history
    DROP COLUMN IF EXISTS user_id,
    ADD COLUMN IF NOT EXISTS profile_id UUID REFERENCES profiles(id) ON DELETE CASCADE;

DELETE FROM playback_history WHERE profile_id IS NULL;

ALTER TABLE playback_history
    ALTER COLUMN profile_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_playback_history_profile_id
    ON playback_history(profile_id);

CREATE INDEX IF NOT EXISTS idx_playback_history_profile_played_at
    ON playback_history(profile_id, played_at DESC);
