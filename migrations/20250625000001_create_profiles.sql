-- Durable profiles for real logged-in users.
-- Anonymous session IDs are not users and should not own durable state.

CREATE TABLE IF NOT EXISTS profiles (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    external_user_id  TEXT NOT NULL UNIQUE,
    display_name      TEXT,
    history_enabled   BOOLEAN NOT NULL DEFAULT FALSE,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_profiles_external_user_id ON profiles(external_user_id);
