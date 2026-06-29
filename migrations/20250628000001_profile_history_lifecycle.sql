-- Irreversibly purge profile history when collection consent is withdrawn.
-- The trigger runs inside the profile update transaction.

CREATE OR REPLACE FUNCTION canopy_purge_history_when_disabled()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.history_enabled AND NOT NEW.history_enabled THEN
        DELETE FROM playback_history WHERE profile_id = NEW.id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_profiles_purge_history_when_disabled ON profiles;
CREATE TRIGGER trg_profiles_purge_history_when_disabled
    AFTER UPDATE OF history_enabled ON profiles
    FOR EACH ROW
    WHEN (OLD.history_enabled AND NOT NEW.history_enabled)
    EXECUTE FUNCTION canopy_purge_history_when_disabled();
