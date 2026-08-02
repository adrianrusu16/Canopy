use canopy_core::{AccountStatus, AuthSession, CanopyError};

#[test]
fn only_pending_accounts_can_activate() {
    assert_eq!(
        AccountStatus::PendingEmailVerification.activate().unwrap(),
        AccountStatus::Active
    );
    assert!(matches!(
        AccountStatus::Disabled.activate(),
        Err(CanopyError::FailedPrecondition(_))
    ));
}

#[test]
fn revoked_sessions_are_not_active() {
    let session = AuthSession {
        id: "session-1".into(),
        account_id: "account-1".into(),
        device_label: "PandaWave".into(),
        created_at_epoch_ms: 1_000,
        last_used_at_epoch_ms: 2_000,
        expires_at_epoch_ms: 10_000,
        revoked_at_epoch_ms: Some(5_000),
    };

    assert!(!session.is_active_at(6_000));
}
