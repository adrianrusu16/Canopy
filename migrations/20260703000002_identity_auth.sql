-- Identity, credential, challenge, and per-device session persistence.
-- Profiles remain application state and may predate an authenticated account.

CREATE TYPE account_status AS ENUM (
    'pending_email_verification',
    'active',
    'disabled',
    'deletion_pending',
    'deleted'
);

CREATE TYPE auth_provider AS ENUM ('google');

CREATE TYPE auth_challenge_type AS ENUM (
    'email_verification',
    'password_reset',
    'email_change',
    'google_login_nonce',
    'google_link'
);

CREATE TABLE accounts (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    status        account_status NOT NULL DEFAULT 'pending_email_verification',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    activated_at  TIMESTAMPTZ,
    disabled_at   TIMESTAMPTZ,
    deleted_at    TIMESTAMPTZ,
    CONSTRAINT accounts_lifecycle_ck CHECK (
        (status <> 'active' OR activated_at IS NOT NULL)
        AND (status <> 'disabled' OR disabled_at IS NOT NULL)
        AND (status NOT IN ('deletion_pending', 'deleted') OR deleted_at IS NOT NULL)
    )
);

CREATE TABLE account_emails (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id        UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    normalized_email  VARCHAR(320) NOT NULL,
    is_primary        BOOLEAN NOT NULL DEFAULT FALSE,
    verified_at       TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at        TIMESTAMPTZ,
    CONSTRAINT account_emails_normalized_ck CHECK (
        normalized_email = LOWER(BTRIM(normalized_email))
        AND normalized_email <> ''
    )
);

CREATE UNIQUE INDEX account_emails_active_email_uq
    ON account_emails(normalized_email)
    WHERE deleted_at IS NULL;

CREATE UNIQUE INDEX account_emails_one_primary_uq
    ON account_emails(account_id)
    WHERE is_primary AND deleted_at IS NULL;

CREATE INDEX account_emails_account_idx ON account_emails(account_id);

CREATE TABLE password_credentials (
    account_id           UUID PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    password_hash_phc    TEXT NOT NULL,
    policy_version       INTEGER NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    password_changed_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT password_credentials_hash_ck CHECK (password_hash_phc <> ''),
    CONSTRAINT password_credentials_policy_ck CHECK (policy_version > 0)
);

CREATE TABLE external_identities (
    id                           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id                   UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    provider                     auth_provider NOT NULL,
    provider_subject             TEXT NOT NULL,
    provider_email_at_link_time  VARCHAR(320),
    created_at                   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT external_identities_subject_ck CHECK (provider_subject <> '')
);

CREATE UNIQUE INDEX external_identities_provider_subject_uq
    ON external_identities(provider, provider_subject);

CREATE INDEX external_identities_account_idx ON external_identities(account_id);

CREATE TABLE auth_sessions (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id         UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    token_family_id    UUID NOT NULL DEFAULT gen_random_uuid(),
    device_label       VARCHAR(256) NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at         TIMESTAMPTZ NOT NULL,
    revoked_at         TIMESTAMPTZ,
    revocation_reason  VARCHAR(64),
    CONSTRAINT auth_sessions_expiry_ck CHECK (expires_at > created_at),
    CONSTRAINT auth_sessions_revocation_ck CHECK (
        (revoked_at IS NULL AND revocation_reason IS NULL)
        OR (revoked_at IS NOT NULL AND revocation_reason IS NOT NULL)
    ),
    CONSTRAINT auth_sessions_family_uq UNIQUE (token_family_id)
);

CREATE INDEX auth_sessions_account_idx
    ON auth_sessions(account_id, created_at DESC);

CREATE INDEX auth_sessions_active_idx
    ON auth_sessions(account_id, expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE auth_session_tokens (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id   UUID NOT NULL REFERENCES auth_sessions(id) ON DELETE CASCADE,
    token_hash   BYTEA NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at   TIMESTAMPTZ NOT NULL,
    consumed_at  TIMESTAMPTZ,
    CONSTRAINT auth_session_tokens_hash_ck CHECK (octet_length(token_hash) = 32),
    CONSTRAINT auth_session_tokens_expiry_ck CHECK (expires_at > created_at),
    CONSTRAINT auth_session_tokens_hash_uq UNIQUE (token_hash)
);

CREATE INDEX auth_session_tokens_session_idx
    ON auth_session_tokens(session_id, created_at DESC);

CREATE TABLE auth_challenges (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id         UUID REFERENCES accounts(id) ON DELETE CASCADE,
    email_id           UUID REFERENCES account_emails(id) ON DELETE CASCADE,
    challenge_type     auth_challenge_type NOT NULL,
    token_hash         BYTEA NOT NULL,
    encrypted_payload  BYTEA,
    attempts           INTEGER NOT NULL DEFAULT 0,
    max_attempts       INTEGER NOT NULL DEFAULT 5,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at         TIMESTAMPTZ NOT NULL,
    consumed_at        TIMESTAMPTZ,
    CONSTRAINT auth_challenges_hash_ck CHECK (octet_length(token_hash) = 32),
    CONSTRAINT auth_challenges_attempts_ck CHECK (
        attempts >= 0 AND max_attempts > 0 AND attempts <= max_attempts
    ),
    CONSTRAINT auth_challenges_expiry_ck CHECK (expires_at > created_at),
    CONSTRAINT auth_challenges_hash_uq UNIQUE (token_hash)
);

CREATE INDEX auth_challenges_account_type_idx
    ON auth_challenges(account_id, challenge_type, expires_at DESC);

CREATE INDEX auth_challenges_live_idx
    ON auth_challenges(expires_at)
    WHERE consumed_at IS NULL;

CREATE TABLE auth_outbox (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    kind               VARCHAR(64) NOT NULL,
    encrypted_payload  BYTEA NOT NULL,
    key_id             VARCHAR(128) NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    available_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    attempts           INTEGER NOT NULL DEFAULT 0,
    delivered_at       TIMESTAMPTZ,
    CONSTRAINT auth_outbox_kind_ck CHECK (kind <> ''),
    CONSTRAINT auth_outbox_payload_ck CHECK (octet_length(encrypted_payload) >= 16),
    CONSTRAINT auth_outbox_attempts_ck CHECK (attempts >= 0)
);

CREATE INDEX auth_outbox_pending_idx
    ON auth_outbox(available_at, created_at)
    WHERE delivered_at IS NULL;

CREATE TABLE auth_rate_limits (
    operation     VARCHAR(64) NOT NULL,
    subject_hash  BYTEA NOT NULL,
    window_start  TIMESTAMPTZ NOT NULL,
    window_end    TIMESTAMPTZ NOT NULL,
    request_count INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (operation, subject_hash, window_start),
    CONSTRAINT auth_rate_limits_subject_ck CHECK (octet_length(subject_hash) = 32),
    CONSTRAINT auth_rate_limits_window_ck CHECK (window_end > window_start),
    CONSTRAINT auth_rate_limits_count_ck CHECK (request_count > 0)
);

CREATE INDEX auth_rate_limits_expiry_idx ON auth_rate_limits(window_end);

ALTER TABLE profiles
    ADD COLUMN account_id UUID;

ALTER TABLE profiles
    ADD CONSTRAINT profiles_account_id_uq UNIQUE (account_id),
    ADD CONSTRAINT profiles_account_id_fk
        FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE;
