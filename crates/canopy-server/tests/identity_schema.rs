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
