#![cfg(feature = "pg")]

#[test]
fn identity_migration_defines_lifecycle_and_secret_storage_constraints() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/20260703000002_identity_auth.sql"
    );
    let sql = std::fs::read_to_string(path).expect("identity migration should exist");

    for required in [
        "CREATE TYPE account_status",
        "pending_email_verification",
        "CREATE TABLE accounts",
        "CREATE TABLE account_emails",
        "CREATE TABLE password_credentials",
        "CREATE TABLE external_identities",
        "CREATE TABLE auth_sessions",
        "CREATE TABLE auth_session_tokens",
        "CREATE TABLE auth_challenges",
        "CREATE TABLE auth_outbox",
        "CREATE TABLE auth_rate_limits",
        "account_emails_active_email_uq",
        "account_emails_one_primary_uq",
        "external_identities_provider_subject_uq",
        "octet_length(token_hash) = 32",
        "encrypted_payload",
        "ADD COLUMN account_id",
    ] {
        assert!(sql.contains(required), "migration is missing {required}");
    }
}
#[test]
fn auth_outbox_delivery_migration_supports_leased_delivery() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/20260711000001_auth_outbox_delivery.sql"
    );
    let sql = std::fs::read_to_string(path).expect("auth outbox delivery migration should exist");

    for required in [
        "ALTER TABLE auth_outbox",
        "ALTER COLUMN encrypted_payload DROP NOT NULL",
        "lease_token UUID",
        "lease_expires_at TIMESTAMPTZ",
        "failed_at TIMESTAMPTZ",
        "last_error_kind VARCHAR(64)",
        "auth_outbox_payload_lifecycle_ck",
        "delivered_at IS NOT NULL",
        "encrypted_payload IS NULL",
        "auth_outbox_lease_ck",
        "auth_outbox_terminal_ck",
        "auth_outbox_error_kind_ck",
        "DROP INDEX auth_outbox_pending_idx",
        "WHERE delivered_at IS NULL AND failed_at IS NULL",
    ] {
        assert!(
            sql.contains(required),
            "auth outbox delivery migration is missing {required}"
        );
    }
}
