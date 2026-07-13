//! Leased authentication email delivery worker and readiness state.

use std::{
    fmt,
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use canopy_core::{
    AuthOutboxFailureKind, AuthOutboxRepository, CanopyResult, ClaimAuthOutboxBatch,
    ClaimedAuthOutbox, MarkAuthOutboxFailed,
};
use sha2::{Digest, Sha256};
use tokio::{sync::watch, time::sleep};
use uuid::Uuid;

use super::{EmailDeliveryError, EmailOutboxPayload, EmailRenderer, EmailSender};
use crate::config::AuthEmailWorkerConfig;

/// Readiness state of authentication email delivery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmailDeliveryState {
    /// SMTP was probed successfully.
    Healthy,
    /// Delivery was explicitly disabled for development.
    Degraded,
    /// SMTP has not been verified or its latest probe failed.
    Unhealthy,
}

/// Safe snapshot exposed to health reporting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmailDeliverySnapshot {
    /// Current dependency state.
    pub state: EmailDeliveryState,
    /// Fixed diagnostic message without provider details.
    pub message: &'static str,
}

/// Shared authentication email readiness state.
#[derive(Clone)]
pub struct EmailDeliveryReadiness {
    inner: Arc<RwLock<EmailDeliverySnapshot>>,
}

impl EmailDeliveryReadiness {
    /// Creates readiness for configured SMTP awaiting its first successful probe.
    pub fn configured() -> Self {
        Self::new(
            EmailDeliveryState::Unhealthy,
            "SMTP connectivity has not been verified",
        )
    }

    /// Creates degraded readiness for the development-only disabled mode.
    pub fn disabled() -> Self {
        Self::new(
            EmailDeliveryState::Degraded,
            "authentication email delivery is disabled",
        )
    }

    /// Returns the latest safe readiness snapshot.
    pub fn snapshot(&self) -> EmailDeliverySnapshot {
        *self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records a successful SMTP probe.
    pub fn mark_healthy(&self) {
        self.replace(
            EmailDeliveryState::Healthy,
            "authentication email delivery is healthy",
        );
    }

    /// Records a failed SMTP probe using a fixed safe message.
    pub fn mark_unhealthy(&self, message: &'static str) {
        self.replace(EmailDeliveryState::Unhealthy, message);
    }

    fn new(state: EmailDeliveryState, message: &'static str) -> Self {
        Self {
            inner: Arc::new(RwLock::new(EmailDeliverySnapshot { state, message })),
        }
    }

    fn replace(&self, state: EmailDeliveryState, message: &'static str) {
        *self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            EmailDeliverySnapshot { state, message };
    }
}

/// Bounded leased worker for authentication email outbox entries.
pub struct AuthEmailWorker {
    repository: Arc<dyn AuthOutboxRepository>,
    sender: Arc<dyn EmailSender>,
    renderer: EmailRenderer,
    config: AuthEmailWorkerConfig,
    readiness: EmailDeliveryReadiness,
}

impl AuthEmailWorker {
    /// Creates a worker from its persistence, transport, rendering, and policy dependencies.
    pub fn new(
        repository: Arc<dyn AuthOutboxRepository>,
        sender: Arc<dyn EmailSender>,
        renderer: EmailRenderer,
        config: AuthEmailWorkerConfig,
        readiness: EmailDeliveryReadiness,
    ) -> Self {
        Self {
            repository,
            sender,
            renderer,
            config,
            readiness,
        }
    }

    /// Claims and processes at most one configured batch.
    pub async fn run_once(&self, now_epoch_ms: u64) -> CanopyResult<usize> {
        let lease_token = Uuid::new_v4().to_string();
        let rows = self
            .repository
            .claim_auth_outbox_batch(ClaimAuthOutboxBatch {
                now_epoch_ms,
                lease_expires_at_epoch_ms: now_epoch_ms
                    .saturating_add(duration_millis(self.config.lease_duration)),
                batch_size: self.config.batch_size,
                lease_token,
            })
            .await?;
        let processed = rows.len();

        for row in rows {
            match self.deliver(&row, now_epoch_ms).await {
                Ok(()) => {
                    self.repository
                        .mark_auth_outbox_delivered(&row.id, &row.lease_token, now_epoch_ms)
                        .await?;
                }
                Err(error) => self.mark_failed(&row, error, now_epoch_ms).await?,
            }
        }

        Ok(processed)
    }

    /// Polls until shutdown, probing SMTP before every claim pass.
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) -> CanopyResult<()> {
        loop {
            if *shutdown.borrow() {
                return Ok(());
            }

            match self.sender.probe().await {
                Ok(()) => {
                    self.readiness.mark_healthy();
                    self.run_once(now_epoch_ms()?).await?;
                }
                Err(_) => self.readiness.mark_unhealthy("SMTP connectivity failed"),
            }

            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
                () = sleep(self.config.poll_interval) => {}
            }
        }
    }

    async fn deliver(
        &self,
        row: &ClaimedAuthOutbox,
        now_epoch_ms: u64,
    ) -> Result<(), EmailDeliveryError> {
        let payload = EmailOutboxPayload::open_with_key_id(&row.key_id, &row.encrypted_payload)
            .map_err(|_| EmailDeliveryError::permanent(AuthOutboxFailureKind::Payload))?;
        if payload.purpose != row.kind || payload.expires_at_epoch_ms <= now_epoch_ms {
            return Err(EmailDeliveryError::permanent(
                AuthOutboxFailureKind::Payload,
            ));
        }
        let rendered = self.renderer.render(&row.id, &payload)?;
        self.sender.send(&rendered).await
    }

    async fn mark_failed(
        &self,
        row: &ClaimedAuthOutbox,
        error: EmailDeliveryError,
        now_epoch_ms: u64,
    ) -> CanopyResult<()> {
        let terminal = !error.retryable || row.attempts >= self.config.max_attempts;
        let available_at_epoch_ms = if terminal {
            now_epoch_ms
        } else {
            now_epoch_ms.saturating_add(duration_millis(retry_delay(
                &self.config,
                &row.id,
                row.attempts,
            )))
        };
        self.repository
            .mark_auth_outbox_failed(MarkAuthOutboxFailed {
                id: row.id.clone(),
                lease_token: row.lease_token.clone(),
                error_kind: error.kind,
                available_at_epoch_ms,
                failed_at_epoch_ms: terminal.then_some(now_epoch_ms),
            })
            .await?;
        Ok(())
    }
}

impl fmt::Debug for AuthEmailWorker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthEmailWorker")
            .field("config", &self.config)
            .field("readiness", &self.readiness.snapshot())
            .finish_non_exhaustive()
    }
}

fn retry_delay(config: &AuthEmailWorkerConfig, outbox_id: &str, attempt: u32) -> Duration {
    let mut base = config.initial_retry_delay;
    for _ in 1..attempt {
        base = base.saturating_mul(2).min(config.max_retry_delay);
    }
    if base >= config.max_retry_delay {
        return config.max_retry_delay;
    }

    let digest = Sha256::digest(format!("{outbox_id}:{attempt}").as_bytes());
    let random = u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 prefix has 8 bytes"));
    let jitter_window_ms = duration_millis(base) / 2;
    let jitter_ms = random % jitter_window_ms.saturating_add(1);
    base.saturating_add(Duration::from_millis(jitter_ms))
        .min(config.max_retry_delay)
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn now_epoch_ms() -> CanopyResult<u64> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        canopy_core::CanopyError::Internal("system clock is before Unix epoch".into())
    })?;
    Ok(duration_millis(elapsed))
}
#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use async_trait::async_trait;
    use canopy_core::{
        AuthOutboxFailureKind, AuthOutboxRepository, CanopyResult, ClaimAuthOutboxBatch,
        ClaimedAuthOutbox, MarkAuthOutboxFailed,
    };
    use tokio::sync::watch;

    use super::*;
    use crate::{
        config::AuthEmailWorkerConfig,
        identity::{
            EmailDeliveryError, EmailOutboxPayload, EmailRenderer, EmailSender, RenderedEmail,
        },
    };

    const OUTBOX_ID: &str = "00000000-0000-0000-0000-000000000042";

    #[derive(Clone, Debug)]
    struct FailureRecord {
        kind: AuthOutboxFailureKind,
        available_at_epoch_ms: u64,
        failed_at_epoch_ms: Option<u64>,
    }

    struct RepositoryState {
        rows: Vec<ClaimedAuthOutbox>,
        delivered: Vec<String>,
        failures: Vec<FailureRecord>,
        delivered_result: bool,
    }

    struct FakeRepository {
        state: Mutex<RepositoryState>,
    }

    impl FakeRepository {
        fn empty() -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(RepositoryState {
                    rows: Vec::new(),
                    delivered: Vec::new(),
                    failures: Vec::new(),
                    delivered_result: true,
                }),
            })
        }

        fn with_row(row: ClaimedAuthOutbox) -> Arc<Self> {
            let repository = Self::empty();
            repository.state.lock().unwrap().rows.push(row);
            repository
        }
    }

    #[async_trait]
    impl AuthOutboxRepository for FakeRepository {
        async fn claim_auth_outbox_batch(
            &self,
            command: ClaimAuthOutboxBatch,
        ) -> CanopyResult<Vec<ClaimedAuthOutbox>> {
            let mut state = self.state.lock().unwrap();
            let mut rows = std::mem::take(&mut state.rows);
            rows.truncate(command.batch_size as usize);
            for row in &mut rows {
                row.lease_token.clone_from(&command.lease_token);
            }
            Ok(rows)
        }

        async fn mark_auth_outbox_delivered(
            &self,
            id: &str,
            _lease_token: &str,
            _delivered_at_epoch_ms: u64,
        ) -> CanopyResult<bool> {
            let mut state = self.state.lock().unwrap();
            if state.delivered_result {
                state.delivered.push(id.to_owned());
            }
            Ok(state.delivered_result)
        }

        async fn mark_auth_outbox_failed(
            &self,
            command: MarkAuthOutboxFailed,
        ) -> CanopyResult<bool> {
            self.state.lock().unwrap().failures.push(FailureRecord {
                kind: command.error_kind,
                available_at_epoch_ms: command.available_at_epoch_ms,
                failed_at_epoch_ms: command.failed_at_epoch_ms,
            });
            Ok(true)
        }
    }

    struct FakeSender {
        results: Mutex<VecDeque<Result<(), EmailDeliveryError>>>,
    }

    impl FakeSender {
        fn succeeding() -> Arc<Self> {
            Arc::new(Self {
                results: Mutex::new(VecDeque::from([Ok(())])),
            })
        }

        fn failing(error: EmailDeliveryError) -> Arc<Self> {
            Arc::new(Self {
                results: Mutex::new(VecDeque::from([Err(error)])),
            })
        }
    }

    #[async_trait]
    impl EmailSender for FakeSender {
        async fn send(&self, _email: &RenderedEmail) -> Result<(), EmailDeliveryError> {
            self.results.lock().unwrap().pop_front().unwrap_or(Ok(()))
        }

        async fn probe(&self) -> Result<(), EmailDeliveryError> {
            Ok(())
        }
    }

    fn config() -> AuthEmailWorkerConfig {
        AuthEmailWorkerConfig {
            poll_interval: Duration::from_secs(3600),
            lease_duration: Duration::from_secs(60),
            batch_size: 20,
            max_attempts: 3,
            initial_retry_delay: Duration::from_secs(5),
            max_retry_delay: Duration::from_secs(60),
        }
    }

    fn renderer() -> EmailRenderer {
        EmailRenderer::new(
            "https://app.example.test/auth/".parse().unwrap(),
            "auth@example.test",
        )
        .unwrap()
    }

    fn claimed_row(attempts: u32, token: &str) -> ClaimedAuthOutbox {
        let sealed =
            EmailOutboxPayload::email_verification("ada@example.test".into(), token, 100_000)
                .seal()
                .unwrap();
        let key_id = sealed.key_id().to_owned();

        ClaimedAuthOutbox {
            id: OUTBOX_ID.into(),
            kind: "email_verification".into(),
            encrypted_payload: sealed.into_bytes(),
            key_id,
            attempts,
            lease_token: String::new(),
        }
    }

    fn worker(
        repository: Arc<dyn AuthOutboxRepository>,
        sender: Arc<dyn EmailSender>,
    ) -> AuthEmailWorker {
        AuthEmailWorker::new(
            repository,
            sender,
            renderer(),
            config(),
            EmailDeliveryReadiness::configured(),
        )
    }

    #[tokio::test]
    async fn run_once_delivers_and_completes_claimed_row() {
        let repository = FakeRepository::with_row(claimed_row(1, "secret-token"));
        let worker = worker(repository.clone(), FakeSender::succeeding());

        let processed = worker.run_once(1_000).await.unwrap();

        assert_eq!(processed, 1);
        let state = repository.state.lock().unwrap();
        assert_eq!(state.delivered, [OUTBOX_ID]);
        assert!(state.failures.is_empty());
    }

    #[tokio::test]
    async fn retryable_failure_reschedules_with_bounded_backoff() {
        let repository = FakeRepository::with_row(claimed_row(1, "secret-token"));
        let sender = FakeSender::failing(EmailDeliveryError::retryable(
            AuthOutboxFailureKind::Connection,
        ));
        let worker = worker(repository.clone(), sender);

        worker.run_once(1_000).await.unwrap();

        let state = repository.state.lock().unwrap();
        let failure = state.failures.first().unwrap();
        assert_eq!(failure.kind, AuthOutboxFailureKind::Connection);
        assert_eq!(failure.failed_at_epoch_ms, None);
        assert!(failure.available_at_epoch_ms > 1_000);
        assert!(failure.available_at_epoch_ms <= 61_000);
    }

    #[tokio::test]
    async fn permanent_failure_exhausts_immediately() {
        let repository = FakeRepository::with_row(claimed_row(1, "secret-token"));
        let sender = FakeSender::failing(EmailDeliveryError::permanent(
            AuthOutboxFailureKind::Rejected,
        ));
        let worker = worker(repository.clone(), sender);

        worker.run_once(2_000).await.unwrap();

        let state = repository.state.lock().unwrap();
        let failure = state.failures.first().unwrap();
        assert_eq!(failure.kind, AuthOutboxFailureKind::Rejected);
        assert_eq!(failure.failed_at_epoch_ms, Some(2_000));
    }

    #[tokio::test]
    async fn max_attempts_exhausts_retryable_failure() {
        let repository = FakeRepository::with_row(claimed_row(config().max_attempts, "token"));
        let sender = FakeSender::failing(EmailDeliveryError::retryable(
            AuthOutboxFailureKind::Timeout,
        ));
        let worker = worker(repository.clone(), sender);

        worker.run_once(3_000).await.unwrap();

        let state = repository.state.lock().unwrap();
        assert_eq!(state.failures[0].failed_at_epoch_ms, Some(3_000));
    }

    #[tokio::test]
    async fn stale_completion_is_treated_as_lease_loss() {
        let repository = FakeRepository::with_row(claimed_row(1, "secret-token"));
        repository.state.lock().unwrap().delivered_result = false;
        let worker = worker(repository.clone(), FakeSender::succeeding());

        assert_eq!(worker.run_once(4_000).await.unwrap(), 1);

        let state = repository.state.lock().unwrap();
        assert!(state.delivered.is_empty());
        assert!(state.failures.is_empty());
    }

    #[tokio::test]
    async fn run_stops_when_shutdown_watch_changes() {
        let worker = worker(FakeRepository::empty(), FakeSender::succeeding());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(worker.run(shutdown_rx));

        tokio::task::yield_now().await;
        shutdown_tx.send(true).unwrap();

        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("worker did not stop")
            .unwrap()
            .unwrap();
    }

    #[test]
    fn retry_delay_is_deterministic_jittered_and_bounded() {
        let config = config();

        let first = retry_delay(&config, OUTBOX_ID, 2);
        let repeated = retry_delay(&config, OUTBOX_ID, 2);
        let later = retry_delay(&config, OUTBOX_ID, 20);

        assert_eq!(first, repeated);
        assert!(first >= config.initial_retry_delay);
        assert!(first <= config.max_retry_delay);
        assert_eq!(later, config.max_retry_delay);
    }

    #[test]
    fn worker_debug_does_not_contain_payload_data() {
        let repository = FakeRepository::with_row(claimed_row(1, "secret-token"));
        let worker = worker(repository, FakeSender::succeeding());

        let debug = format!("{worker:?}");

        assert!(!debug.contains("secret-token"));
        assert!(!debug.contains("ada@example.test"));
        assert!(debug.contains("AuthEmailWorker"));
    }
}
