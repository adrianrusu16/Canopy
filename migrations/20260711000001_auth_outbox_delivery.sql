ALTER TABLE auth_outbox
    DROP CONSTRAINT auth_outbox_payload_ck,
    ALTER COLUMN encrypted_payload DROP NOT NULL,
    ADD COLUMN lease_token UUID,
    ADD COLUMN lease_expires_at TIMESTAMPTZ,
    ADD COLUMN failed_at TIMESTAMPTZ,
    ADD COLUMN last_error_kind VARCHAR(64);

ALTER TABLE auth_outbox
    ADD CONSTRAINT auth_outbox_payload_lifecycle_ck CHECK (
        (
            delivered_at IS NULL
            AND encrypted_payload IS NOT NULL
            AND octet_length(encrypted_payload) >= 16
        )
        OR (
            delivered_at IS NOT NULL
            AND encrypted_payload IS NULL
        )
    ),
    ADD CONSTRAINT auth_outbox_lease_ck CHECK (
        (lease_token IS NULL) = (lease_expires_at IS NULL)
    ),
    ADD CONSTRAINT auth_outbox_terminal_ck CHECK (
        NOT (delivered_at IS NOT NULL AND failed_at IS NOT NULL)
    ),
    ADD CONSTRAINT auth_outbox_error_kind_ck CHECK (
        last_error_kind IS NULL OR last_error_kind IN (
            'configuration',
            'connection',
            'timeout',
            'authentication',
            'rejected',
            'payload',
            'internal'
        )
    );

DROP INDEX auth_outbox_pending_idx;

CREATE INDEX auth_outbox_pending_idx
    ON auth_outbox(available_at, lease_expires_at, created_at)
    WHERE delivered_at IS NULL AND failed_at IS NULL;
