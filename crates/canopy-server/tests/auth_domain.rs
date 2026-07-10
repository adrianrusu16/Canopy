use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use canopy_core::{
    AccountRecord, AccountStatus, AuthSession, CanopyError, CanopyResult, ChangePasswordRecord,
    CompletePasswordResetRecord, ConsumeChallenge, CreateEmailVerificationChallenge,
    CreatePasswordResetChallenge, CreateSessionRecord, ExternalIdentityRecord, IdentityRepository,
    PasswordCredentialRecord, PasswordLoginRecord, RateLimitBucket, RateLimitState,
    RegisterPasswordRecord, RotateRefreshTokenRecord, StoredAuthenticatedSession,
};
use canopy_proto::auth_service_server::AuthService;
use canopy_proto::{
    DeleteAccountRequest, GetAccountRequest, ListSessionsRequest, PageRequest,
    RegisterPasswordRequest, ResendVerificationRequest, RevokeSessionRequest, VerifyEmailRequest,
};
use canopy_server::api::grpc::AuthGrpc;
use canopy_server::identity::{
    AccessTokenConfig, Argon2PasswordHasher, AuthenticatedPrincipal, ChangePasswordCommand,
    CompletePasswordResetCommand, Ed25519AccessTokenIssuer, EmailOutboxPayload, FixedClock,
    IdentityService, LoginPasswordCommand, PasswordHasher, RefreshSessionCommand,
    RegisterPasswordCommand, RequestPasswordResetCommand, ResendVerificationCommand, TokenDigest,
    VerifyEmailCommand,
};

const NOW_MS: u64 = 1_780_000_000_000;
const PASSPHRASE: &str = "correct horse battery staple";

#[derive(Default)]
struct FakeIdentityRepository {
    registered: Mutex<Option<CapturedRegistration>>,
    verification_challenge: Mutex<Option<CapturedVerificationChallenge>>,
    password_reset_challenge: Mutex<Option<CapturedPasswordResetChallenge>>,
    password_credential: Mutex<Option<PasswordCredentialRecord>>,
    completed_reset: Mutex<Option<CompletePasswordResetRecord>>,
    changed_password: Mutex<Option<ChangePasswordRecord>>,
    account_lookup: Mutex<Option<AccountRecord>>,
    deleted_accounts: Mutex<Vec<String>>,
    rate_limit_state: Mutex<RateLimitState>,
    rate_limit_checks: Mutex<Vec<RateLimitBucket>>,
    rate_limit_hits: Mutex<Vec<RateLimitBucket>>,
    activation: Mutex<Option<StoredAuthenticatedSession>>,
    consumed_challenge: Mutex<Option<ConsumeChallenge>>,
    created_session: Mutex<Option<CreateSessionRecord>>,
    session_result: Mutex<Option<StoredAuthenticatedSession>>,
    password_login: Mutex<Option<PasswordLoginRecord>>,
    updated_password_hash: Mutex<Option<String>>,
    rotation: Mutex<Option<RotateRefreshTokenRecord>>,
    rotation_result: Mutex<Option<StoredAuthenticatedSession>>,
    validated_sessions: Mutex<Vec<(String, String, u64)>>,
    revoked_sessions: Mutex<Vec<(String, String)>>,
    listed_sessions: Mutex<Vec<AuthSession>>,
}

#[derive(Debug)]
struct CapturedVerificationChallenge {
    normalized_email: String,
    token_hash: [u8; 32],
    expires_at_epoch_ms: u64,
    encrypted_outbox_payload: Vec<u8>,
    outbox_key_id: String,
}

#[derive(Debug)]
struct CapturedPasswordResetChallenge {
    normalized_email: String,
    token_hash: [u8; 32],
    expires_at_epoch_ms: u64,
    encrypted_outbox_payload: Vec<u8>,
    outbox_key_id: String,
}
struct CapturedRegistration {
    normalized_email: String,
    password_hash_phc: String,
    verification_token_hash: [u8; 32],
    verification_expires_at_epoch_ms: u64,
    encrypted_outbox_payload: Vec<u8>,
    outbox_key_id: String,
}

impl FakeIdentityRepository {
    fn with_activation(session: StoredAuthenticatedSession) -> Self {
        Self {
            activation: Mutex::new(Some(session)),
            ..Self::default()
        }
    }

    fn with_password_login(
        login: PasswordLoginRecord,
        session: StoredAuthenticatedSession,
    ) -> Self {
        Self {
            password_login: Mutex::new(Some(login)),
            session_result: Mutex::new(Some(session)),
            ..Self::default()
        }
    }

    fn with_password_credential(credential: PasswordCredentialRecord) -> Self {
        Self {
            password_credential: Mutex::new(Some(credential)),
            ..Self::default()
        }
    }
    fn with_refresh(session: StoredAuthenticatedSession) -> Self {
        Self {
            rotation_result: Mutex::new(Some(session)),
            ..Self::default()
        }
    }

    fn with_account_lookup(account: AccountRecord, session: StoredAuthenticatedSession) -> Self {
        Self {
            account_lookup: Mutex::new(Some(account)),
            activation: Mutex::new(Some(session)),
            ..Self::default()
        }
    }

    fn with_rate_limit_state(state: RateLimitState) -> Self {
        Self {
            rate_limit_state: Mutex::new(state),
            ..Self::default()
        }
    }
    fn with_activation_and_sessions(
        session: StoredAuthenticatedSession,
        listed_sessions: Vec<AuthSession>,
    ) -> Self {
        Self {
            activation: Mutex::new(Some(session)),
            listed_sessions: Mutex::new(listed_sessions),
            ..Self::default()
        }
    }

    fn registered(&self) -> CapturedRegistration {
        self.registered.lock().unwrap().take().unwrap()
    }

    fn verification_challenge(&self) -> CapturedVerificationChallenge {
        self.verification_challenge.lock().unwrap().take().unwrap()
    }

    fn password_reset_challenge(&self) -> CapturedPasswordResetChallenge {
        self.password_reset_challenge
            .lock()
            .unwrap()
            .take()
            .unwrap()
    }

    fn completed_reset(&self) -> CompletePasswordResetRecord {
        self.completed_reset.lock().unwrap().take().unwrap()
    }

    fn changed_password(&self) -> ChangePasswordRecord {
        self.changed_password.lock().unwrap().take().unwrap()
    }
    fn consumed_challenge(&self) -> ConsumeChallenge {
        self.consumed_challenge.lock().unwrap().take().unwrap()
    }

    fn created_session(&self) -> CreateSessionRecord {
        self.created_session.lock().unwrap().take().unwrap()
    }

    fn rotation(&self) -> RotateRefreshTokenRecord {
        self.rotation.lock().unwrap().take().unwrap()
    }

    fn validated_sessions(&self) -> Vec<(String, String, u64)> {
        self.validated_sessions.lock().unwrap().clone()
    }

    fn deleted_accounts(&self) -> Vec<String> {
        self.deleted_accounts.lock().unwrap().clone()
    }

    fn rate_limit_checks(&self) -> Vec<RateLimitBucket> {
        self.rate_limit_checks.lock().unwrap().clone()
    }

    fn recorded_rate_limits(&self) -> Vec<RateLimitBucket> {
        self.rate_limit_hits.lock().unwrap().clone()
    }

    fn revoked_sessions(&self) -> Vec<(String, String)> {
        self.revoked_sessions.lock().unwrap().clone()
    }
}

#[async_trait]
impl IdentityRepository for FakeIdentityRepository {
    async fn rate_limit_state(&self, bucket: RateLimitBucket) -> CanopyResult<RateLimitState> {
        self.rate_limit_checks.lock().unwrap().push(bucket);
        Ok(self.rate_limit_state.lock().unwrap().clone())
    }

    async fn record_rate_limit_hit(&self, bucket: RateLimitBucket) -> CanopyResult<RateLimitState> {
        self.rate_limit_hits.lock().unwrap().push(bucket);
        Ok(self.rate_limit_state.lock().unwrap().clone())
    }

    async fn register_password(&self, record: RegisterPasswordRecord) -> CanopyResult<()> {
        *self.registered.lock().unwrap() = Some(CapturedRegistration {
            normalized_email: record.normalized_email,
            password_hash_phc: record.password_hash_phc,
            verification_token_hash: record.verification_token_hash,
            verification_expires_at_epoch_ms: record.verification_expires_at_epoch_ms,
            encrypted_outbox_payload: record.encrypted_outbox_payload,
            outbox_key_id: record.outbox_key_id,
        });
        Ok(())
    }

    async fn create_password_reset_challenge(
        &self,
        record: CreatePasswordResetChallenge,
    ) -> CanopyResult<()> {
        *self.password_reset_challenge.lock().unwrap() = Some(CapturedPasswordResetChallenge {
            normalized_email: record.normalized_email,
            token_hash: record.token_hash,
            expires_at_epoch_ms: record.expires_at_epoch_ms,
            encrypted_outbox_payload: record.encrypted_outbox_payload,
            outbox_key_id: record.outbox_key_id,
        });
        Ok(())
    }

    async fn password_credential_for_account(
        &self,
        _account_id: &str,
    ) -> CanopyResult<Option<PasswordCredentialRecord>> {
        Ok(self.password_credential.lock().unwrap().clone())
    }

    async fn complete_password_reset(
        &self,
        record: CompletePasswordResetRecord,
    ) -> CanopyResult<()> {
        *self.completed_reset.lock().unwrap() = Some(record);
        Ok(())
    }

    async fn change_password(&self, record: ChangePasswordRecord) -> CanopyResult<()> {
        *self.changed_password.lock().unwrap() = Some(record);
        Ok(())
    }
    async fn create_email_verification_challenge(
        &self,
        record: CreateEmailVerificationChallenge,
    ) -> CanopyResult<()> {
        *self.verification_challenge.lock().unwrap() = Some(CapturedVerificationChallenge {
            normalized_email: record.normalized_email,
            token_hash: record.token_hash,
            expires_at_epoch_ms: record.expires_at_epoch_ms,
            encrypted_outbox_payload: record.encrypted_outbox_payload,
            outbox_key_id: record.outbox_key_id,
        });
        Ok(())
    }
    async fn activate_email_and_create_session(
        &self,
        challenge: ConsumeChallenge,
        session: CreateSessionRecord,
    ) -> CanopyResult<StoredAuthenticatedSession> {
        *self.consumed_challenge.lock().unwrap() = Some(challenge);
        *self.created_session.lock().unwrap() = Some(session);
        Ok(self.activation.lock().unwrap().clone().unwrap())
    }

    async fn password_login_record(
        &self,
        _normalized_email: &str,
    ) -> CanopyResult<Option<PasswordLoginRecord>> {
        Ok(self.password_login.lock().unwrap().clone())
    }

    async fn update_password_hash(
        &self,
        _account_id: &str,
        _password_hash_phc: &str,
        _policy_version: u32,
    ) -> CanopyResult<()> {
        *self.updated_password_hash.lock().unwrap() = Some(_password_hash_phc.to_string());
        Ok(())
    }

    async fn create_session(
        &self,
        record: CreateSessionRecord,
    ) -> CanopyResult<StoredAuthenticatedSession> {
        *self.created_session.lock().unwrap() = Some(record);
        Ok(self.session_result.lock().unwrap().clone().unwrap())
    }

    async fn rotate_refresh_token(
        &self,
        record: RotateRefreshTokenRecord,
    ) -> CanopyResult<StoredAuthenticatedSession> {
        *self.rotation.lock().unwrap() = Some(record);
        Ok(self.rotation_result.lock().unwrap().clone().unwrap())
    }

    async fn validate_active_session(
        &self,
        account_id: &str,
        session_id: &str,
        now_epoch_ms: u64,
    ) -> CanopyResult<()> {
        self.validated_sessions.lock().unwrap().push((
            account_id.to_string(),
            session_id.to_string(),
            now_epoch_ms,
        ));
        Ok(())
    }

    async fn account_by_id(&self, _account_id: &str) -> CanopyResult<Option<AccountRecord>> {
        Ok(self.account_lookup.lock().unwrap().clone())
    }
    async fn revoke_session(&self, account_id: &str, session_id: &str) -> CanopyResult<()> {
        self.revoked_sessions
            .lock()
            .unwrap()
            .push((account_id.to_string(), session_id.to_string()));
        Ok(())
    }

    async fn revoke_all_sessions(&self, account_id: &str, reason: &str) -> CanopyResult<()> {
        self.revoked_sessions
            .lock()
            .unwrap()
            .push((account_id.to_string(), reason.to_string()));
        Ok(())
    }

    async fn list_sessions(&self, _account_id: &str) -> CanopyResult<Vec<AuthSession>> {
        Ok(self.listed_sessions.lock().unwrap().clone())
    }

    async fn account_by_external_identity(
        &self,
        _identity: &ExternalIdentityRecord,
    ) -> CanopyResult<Option<AccountRecord>> {
        unimplemented!()
    }

    async fn delete_account(&self, account_id: &str) -> CanopyResult<()> {
        self.deleted_accounts
            .lock()
            .unwrap()
            .push(account_id.to_string());
        Ok(())
    }
}

fn service(repo: Arc<dyn IdentityRepository>) -> IdentityService {
    IdentityService::new(
        repo,
        Arc::new(Argon2PasswordHasher::default()),
        Ed25519AccessTokenIssuer::generate(AccessTokenConfig {
            issuer: "canopy.test".into(),
            audience: "pandawave".into(),
            key_id: "test-key".into(),
            ttl_seconds: 900,
        }),
        Arc::new(FixedClock::new(NOW_MS)),
    )
}

#[tokio::test]
async fn native_registration_is_pending_and_stores_only_verification_digest() {
    let repo = Arc::new(FakeIdentityRepository::default());
    let service = service(repo.clone());

    service
        .register_password(RegisterPasswordCommand {
            email: "  ADA@Example.TEST  ".into(),
            password: PASSPHRASE.into(),
        })
        .await
        .unwrap();

    let registered = repo.registered();
    assert_eq!(registered.normalized_email, "ada@example.test");
    assert!(registered.password_hash_phc.starts_with("$argon2id$"));
    assert_ne!(registered.verification_token_hash, [0; 32]);
    assert!(registered.verification_expires_at_epoch_ms > NOW_MS);
    assert!(!registered.outbox_key_id.is_empty());
    let payload: EmailOutboxPayload =
        serde_json::from_slice(&registered.encrypted_outbox_payload).unwrap();
    assert_eq!(payload.email, "ada@example.test");
    assert_eq!(payload.purpose, "email_verification");
    assert_eq!(
        payload.expires_at_epoch_ms,
        registered.verification_expires_at_epoch_ms
    );
    assert!(
        payload
            .template_variables
            .contains_key("verification_token")
    );
}

#[tokio::test]
async fn resend_verification_rotates_challenge_for_pending_account_only() {
    let repo = Arc::new(FakeIdentityRepository::default());
    let service = service(repo.clone());

    service
        .resend_verification(ResendVerificationCommand {
            email: "  ADA@Example.TEST  ".into(),
        })
        .await
        .unwrap();

    let challenge = repo.verification_challenge();
    assert_eq!(challenge.normalized_email, "ada@example.test");
    assert_ne!(challenge.token_hash, [0; 32]);
    assert!(challenge.expires_at_epoch_ms > NOW_MS);
    assert!(!challenge.outbox_key_id.is_empty());
    let payload: EmailOutboxPayload =
        serde_json::from_slice(&challenge.encrypted_outbox_payload).unwrap();
    assert_eq!(payload.email, "ada@example.test");
    assert_eq!(payload.purpose, "email_verification");
    assert!(
        payload
            .template_variables
            .contains_key("verification_token")
    );
}
#[tokio::test]
async fn email_verification_consumes_challenge_and_issues_device_session() {
    let account = AccountRecord {
        id: "account-1".into(),
        status: AccountStatus::Active,
        primary_email: Some("ada@example.test".into()),
        created_at_epoch_ms: NOW_MS,
    };
    let session = AuthSession {
        id: "session-1".into(),
        account_id: account.id.clone(),
        device_label: "PandaWave Android".into(),
        expires_at_epoch_ms: NOW_MS + 86_400_000,
        revoked_at_epoch_ms: None,
    };
    let repo = Arc::new(FakeIdentityRepository::with_activation(
        StoredAuthenticatedSession { account, session },
    ));
    let service = service(repo.clone());

    let envelope = service
        .verify_email(VerifyEmailCommand {
            verification_token: "presented-email-token".into(),
            device_label: "PandaWave Android".into(),
        })
        .await
        .unwrap();

    assert_eq!(envelope.account.id, "account-1");
    assert_eq!(envelope.session.id, "session-1");
    assert!(!envelope.access_token.is_empty());
    assert!(!envelope.refresh_token.is_empty());

    let challenge = repo.consumed_challenge();
    assert_eq!(challenge.challenge_type, "email_verification");
    assert_eq!(challenge.now_epoch_ms, NOW_MS);
    assert_ne!(challenge.token_hash, [0; 32]);

    let created = repo.created_session();
    assert_eq!(created.device_label, "PandaWave Android");
    assert_ne!(created.refresh_token_hash, [0; 32]);
    assert!(created.expires_at_epoch_ms > NOW_MS);
}

fn active_account() -> AccountRecord {
    AccountRecord {
        id: "account-1".into(),
        status: AccountStatus::Active,
        primary_email: Some("ada@example.test".into()),
        created_at_epoch_ms: NOW_MS,
    }
}

fn active_session(id: &str) -> AuthSession {
    AuthSession {
        id: id.into(),
        account_id: "account-1".into(),
        device_label: "PandaWave Android".into(),
        expires_at_epoch_ms: NOW_MS + 86_400_000,
        revoked_at_epoch_ms: None,
    }
}

#[tokio::test]
async fn request_password_reset_queues_generic_reset_payload() {
    let repo = Arc::new(FakeIdentityRepository::default());
    let service = service(repo.clone());

    service
        .request_password_reset(RequestPasswordResetCommand {
            email: "ADA@example.test".into(),
        })
        .await
        .unwrap();

    let challenge = repo.password_reset_challenge();
    assert_eq!(challenge.normalized_email, "ada@example.test");
    assert_ne!(challenge.token_hash, [0; 32]);
    assert!(challenge.expires_at_epoch_ms > NOW_MS);
    assert!(!challenge.outbox_key_id.is_empty());
    let payload: EmailOutboxPayload =
        serde_json::from_slice(&challenge.encrypted_outbox_payload).unwrap();
    assert_eq!(payload.purpose, "password_reset");
    assert!(payload.template_variables.contains_key("reset_token"));
}

#[tokio::test]
async fn complete_password_reset_consumes_token_and_writes_new_hash() {
    let repo = Arc::new(FakeIdentityRepository::default());
    let service = service(repo.clone());

    service
        .complete_password_reset(CompletePasswordResetCommand {
            reset_token: "reset-token".into(),
            new_password: "new correct horse battery staple".into(),
        })
        .await
        .unwrap();

    let reset = repo.completed_reset();
    assert_eq!(
        reset.token_hash,
        *TokenDigest::from_secret("reset-token").as_bytes()
    );
    assert!(reset.password_hash_phc.starts_with("$argon2id$"));
    assert_eq!(reset.policy_version, 1);
    assert_eq!(reset.now_epoch_ms, NOW_MS);
}

#[tokio::test]
async fn change_password_verifies_current_password_and_writes_new_hash() {
    let hasher = Argon2PasswordHasher::default();
    let repo = Arc::new(FakeIdentityRepository::with_password_credential(
        PasswordCredentialRecord {
            account: active_account(),
            password_hash_phc: hasher.hash(PASSPHRASE).unwrap(),
            policy_version: 1,
        },
    ));
    let service = service(repo.clone());

    service
        .change_password(ChangePasswordCommand {
            principal: AuthenticatedPrincipal {
                account_id: "account-1".into(),
                session_id: "session-1".into(),
            },
            current_password: PASSPHRASE.into(),
            new_password: "new correct horse battery staple".into(),
        })
        .await
        .unwrap();

    let changed = repo.changed_password();
    assert_eq!(changed.account_id, "account-1");
    assert_eq!(changed.current_session_id, "session-1");
    assert!(changed.password_hash_phc.starts_with("$argon2id$"));
}
#[tokio::test]
async fn password_login_verifies_password_and_issues_device_session() {
    let hasher = Argon2PasswordHasher::default();
    let account = active_account();
    let session = active_session("login-session-1");
    let repo = Arc::new(FakeIdentityRepository::with_password_login(
        PasswordLoginRecord {
            account: account.clone(),
            password_hash_phc: hasher.hash(PASSPHRASE).unwrap(),
            policy_version: 1,
        },
        StoredAuthenticatedSession { account, session },
    ));
    let service = service(repo.clone());

    let envelope = service
        .login_password(LoginPasswordCommand {
            email: "ADA@example.test".into(),
            password: PASSPHRASE.into(),
            device_label: "PandaWave Android".into(),
        })
        .await
        .unwrap();

    assert_eq!(envelope.session.id, "login-session-1");
    assert!(!envelope.access_token.is_empty());
    assert!(!envelope.refresh_token.is_empty());

    let created = repo.created_session();
    assert_eq!(created.account_id, "account-1");
    assert_eq!(created.device_label, "PandaWave Android");
    assert_ne!(created.refresh_token_hash, [0; 32]);
}

#[tokio::test]
async fn password_login_records_failed_attempts_and_blocks_limited_bucket() {
    let hasher = Argon2PasswordHasher::default();
    let account = active_account();
    let session = active_session("login-session-1");
    let repo = Arc::new(FakeIdentityRepository::with_password_login(
        PasswordLoginRecord {
            account: account.clone(),
            password_hash_phc: hasher.hash(PASSPHRASE).unwrap(),
            policy_version: 1,
        },
        StoredAuthenticatedSession { account, session },
    ));
    let service = service(repo.clone());

    let error = service
        .login_password(LoginPasswordCommand {
            email: "ADA@example.test".into(),
            password: "wrong horse battery staple".into(),
            device_label: "PandaWave Android".into(),
        })
        .await
        .unwrap_err();

    assert!(matches!(error, CanopyError::Unauthenticated(_)));
    assert_eq!(repo.rate_limit_checks().len(), 1);
    let hits = repo.recorded_rate_limits();
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit.operation, "login_password_failure");
    assert_ne!(hit.subject_hash, [0; 32]);
    assert_eq!(hit.window_start_epoch_ms, NOW_MS - (NOW_MS % 900_000));
    assert_eq!(hit.window_end_epoch_ms, hit.window_start_epoch_ms + 900_000);
    assert_eq!(hit.max_attempts, 5);

    let limited_repo = Arc::new(FakeIdentityRepository::with_rate_limit_state(
        RateLimitState {
            request_count: 5,
            limited: true,
        },
    ));
    let service = service(limited_repo.clone());

    let error = service
        .login_password(LoginPasswordCommand {
            email: "ADA@example.test".into(),
            password: PASSPHRASE.into(),
            device_label: "PandaWave Android".into(),
        })
        .await
        .unwrap_err();

    assert!(matches!(error, CanopyError::RateLimited(_)));
    assert_eq!(limited_repo.rate_limit_checks().len(), 1);
    assert!(limited_repo.recorded_rate_limits().is_empty());
}

#[tokio::test]
async fn refresh_session_rotates_presented_token_and_returns_replacement() {
    let account = active_account();
    let session = active_session("refresh-session-1");
    let repo = Arc::new(FakeIdentityRepository::with_refresh(
        StoredAuthenticatedSession { account, session },
    ));
    let service = service(repo.clone());

    let envelope = service
        .refresh_session(RefreshSessionCommand {
            refresh_token: "presented-refresh-token".into(),
        })
        .await
        .unwrap();

    assert_eq!(envelope.session.id, "refresh-session-1");
    assert!(!envelope.access_token.is_empty());
    assert!(!envelope.refresh_token.is_empty());

    let rotation = repo.rotation();
    assert_eq!(
        rotation.presented_token_hash,
        *TokenDigest::from_secret("presented-refresh-token").as_bytes()
    );
    assert_ne!(rotation.replacement_token_hash, [0; 32]);
    assert!(rotation.replacement_expires_at_epoch_ms > NOW_MS);
    assert_eq!(rotation.now_epoch_ms, NOW_MS);
}

#[tokio::test]
async fn auth_grpc_maps_verify_email_to_session_envelope() {
    let account = active_account();
    let session = active_session("grpc-session-1");
    let repo = Arc::new(FakeIdentityRepository::with_activation(
        StoredAuthenticatedSession { account, session },
    ));
    let service = Arc::new(service(repo));
    let grpc = AuthGrpc(service);

    let response = grpc
        .verify_email(tonic::Request::new(VerifyEmailRequest {
            verification_token: "presented-email-token".into(),
            device_label: "PandaWave Android".into(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(response.account.unwrap().id, "account-1");
    assert_eq!(response.session.unwrap().id, "grpc-session-1");
    assert!(!response.access_token.is_empty());
    assert!(!response.refresh_token.is_empty());
}

#[tokio::test]
async fn auth_grpc_maps_register_password_to_generic_acceptance() {
    let repo = Arc::new(FakeIdentityRepository::default());
    let service = Arc::new(service(repo));
    let grpc = AuthGrpc(service);

    let response = grpc
        .register_password(tonic::Request::new(RegisterPasswordRequest {
            email: "ada@example.test".into(),
            password: PASSPHRASE.into(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(response.accepted);
}

#[tokio::test]
async fn auth_grpc_maps_resend_verification_to_generic_acceptance() {
    let repo = Arc::new(FakeIdentityRepository::default());
    let service = Arc::new(service(repo.clone()));
    let grpc = AuthGrpc(service);

    let response = grpc
        .resend_verification(tonic::Request::new(ResendVerificationRequest {
            email: "ada@example.test".into(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(response.accepted);
    assert_eq!(
        repo.verification_challenge().normalized_email,
        "ada@example.test"
    );
}

#[tokio::test]
async fn auth_grpc_get_account_returns_authenticated_account() {
    let account = active_account();
    let session = active_session("account-session-1");
    let repo = Arc::new(FakeIdentityRepository::with_account_lookup(
        account.clone(),
        StoredAuthenticatedSession { account, session },
    ));
    let service = Arc::new(service(repo));
    let grpc = AuthGrpc(service);

    let envelope = grpc
        .verify_email(tonic::Request::new(VerifyEmailRequest {
            verification_token: "presented-email-token".into(),
            device_label: "PandaWave Android".into(),
        }))
        .await
        .unwrap()
        .into_inner();

    let mut request = tonic::Request::new(GetAccountRequest {});
    request.metadata_mut().insert(
        "authorization",
        tonic::metadata::MetadataValue::try_from(format!("Bearer {}", envelope.access_token))
            .unwrap(),
    );

    let response = grpc.get_account(request).await.unwrap().into_inner();
    let account = response.account.unwrap();
    assert_eq!(account.id, "account-1");
    assert_eq!(account.primary_email, "ada@example.test");
}

#[tokio::test]
async fn auth_grpc_delete_account_is_accepted_for_authenticated_account() {
    let account = active_account();
    let session = active_session("delete-session-1");
    let repo = Arc::new(FakeIdentityRepository::with_account_lookup(
        account.clone(),
        StoredAuthenticatedSession { account, session },
    ));
    let service = Arc::new(service(repo.clone()));
    let grpc = AuthGrpc(service);

    let envelope = grpc
        .verify_email(tonic::Request::new(VerifyEmailRequest {
            verification_token: "presented-email-token".into(),
            device_label: "PandaWave Android".into(),
        }))
        .await
        .unwrap()
        .into_inner();

    let mut request = tonic::Request::new(DeleteAccountRequest {});
    request.metadata_mut().insert(
        "authorization",
        tonic::metadata::MetadataValue::try_from(format!("Bearer {}", envelope.access_token))
            .unwrap(),
    );

    let response = grpc.delete_account(request).await.unwrap().into_inner();
    assert!(response.accepted);
    assert_eq!(repo.deleted_accounts(), vec!["account-1".to_string()]);
}
#[tokio::test]
async fn auth_grpc_session_management_requires_valid_access_token() {
    let account = active_account();
    let current = active_session("grpc-current-session");
    let other = AuthSession {
        id: "grpc-other-session".into(),
        account_id: account.id.clone(),
        device_label: "PandaWave Tablet".into(),
        expires_at_epoch_ms: NOW_MS + 86_400_000,
        revoked_at_epoch_ms: None,
    };
    let repo = Arc::new(FakeIdentityRepository::with_activation_and_sessions(
        StoredAuthenticatedSession {
            account,
            session: current.clone(),
        },
        vec![current.clone(), other.clone()],
    ));
    let service = Arc::new(service(repo.clone()));
    let grpc = AuthGrpc(service);

    let envelope = grpc
        .verify_email(tonic::Request::new(VerifyEmailRequest {
            verification_token: "presented-email-token".into(),
            device_label: "PandaWave Android".into(),
        }))
        .await
        .unwrap()
        .into_inner();

    let mut list_request = tonic::Request::new(ListSessionsRequest {
        page: Some(PageRequest::default()),
    });
    list_request.metadata_mut().insert(
        "authorization",
        tonic::metadata::MetadataValue::try_from(format!("Bearer {}", envelope.access_token))
            .unwrap(),
    );

    let listed = grpc.list_sessions(list_request).await.unwrap().into_inner();
    assert_eq!(listed.sessions.len(), 2);
    assert_eq!(
        listed
            .sessions
            .iter()
            .filter(|session| session.current)
            .count(),
        1
    );
    assert!(
        listed
            .sessions
            .iter()
            .any(|session| session.id == "grpc-current-session" && session.current)
    );

    let mut revoke_request = tonic::Request::new(RevokeSessionRequest {
        session_id: "grpc-other-session".into(),
    });
    revoke_request.metadata_mut().insert(
        "authorization",
        tonic::metadata::MetadataValue::try_from(format!("Bearer {}", envelope.access_token))
            .unwrap(),
    );

    let revoked = grpc
        .revoke_session(revoke_request)
        .await
        .unwrap()
        .into_inner();
    assert!(revoked.accepted);
    assert_eq!(
        repo.revoked_sessions(),
        vec![("account-1".into(), "grpc-other-session".into())]
    );
    assert_eq!(
        repo.validated_sessions(),
        vec![
            ("account-1".into(), "grpc-current-session".into(), NOW_MS),
            ("account-1".into(), "grpc-current-session".into(), NOW_MS),
        ]
    );
}
