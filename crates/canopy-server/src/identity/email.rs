//! Rendering and SMTP delivery for authentication email outbox entries.

use std::fmt;

use async_trait::async_trait;
use canopy_core::{AuthOutboxFailureKind, CanopyError, CanopyResult};
use lettre::{
    Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart},
    transport::smtp::{
        Error as SmtpError,
        authentication::Credentials,
        client::{Certificate, Tls, TlsParametersBuilder},
    },
};
use reqwest::Url;
use uuid::Uuid;

use super::EmailOutboxPayload;
use crate::config::{SmtpConfig, SmtpTlsMode};

/// Fully rendered authentication email, with sensitive fields redacted from Debug.
pub struct RenderedEmail {
    /// Deterministic RFC 5322 message identifier.
    pub message_id: String,
    /// Recipient address.
    pub to: String,
    /// Message subject.
    pub subject: String,
    /// Plain-text alternative.
    pub text_body: String,
    /// HTML alternative.
    pub html_body: String,
}

impl fmt::Debug for RenderedEmail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenderedEmail")
            .field("message_id", &self.message_id)
            .field("to", &"[REDACTED]")
            .field("subject", &self.subject)
            .field("text_body", &"[REDACTED]")
            .field("html_body", &"[REDACTED]")
            .finish()
    }
}

/// Sanitized delivery failure returned by email transports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmailDeliveryError {
    /// Safe failure category for persistence and telemetry.
    pub kind: AuthOutboxFailureKind,
    /// Whether the worker may retry the operation.
    pub retryable: bool,
}

impl EmailDeliveryError {
    /// Creates a retryable sanitized failure.
    pub const fn retryable(kind: AuthOutboxFailureKind) -> Self {
        Self {
            kind,
            retryable: true,
        }
    }

    /// Creates a permanent sanitized failure.
    pub const fn permanent(kind: AuthOutboxFailureKind) -> Self {
        Self {
            kind,
            retryable: false,
        }
    }
}

/// Outbound email transport used by the auth outbox worker.
#[async_trait]
pub trait EmailSender: Send + Sync {
    /// Sends one fully rendered message.
    async fn send(&self, email: &RenderedEmail) -> Result<(), EmailDeliveryError>;

    /// Checks whether the configured transport is available.
    async fn probe(&self) -> Result<(), EmailDeliveryError>;
}

/// Pure renderer for authentication messages and action links.
#[derive(Clone)]
pub struct EmailRenderer {
    public_base_url: Url,
    from_domain: String,
}

impl EmailRenderer {
    /// Creates a renderer from a public action URL and sender address.
    pub fn new(public_base_url: Url, from_address: &str) -> CanopyResult<Self> {
        let from_address = from_address.parse::<Address>().map_err(|_| {
            CanopyError::InvalidArgument("invalid authentication email sender address".into())
        })?;

        Ok(Self {
            public_base_url,
            from_domain: from_address.domain().to_owned(),
        })
    }

    /// Renders one validated outbox payload without performing I/O.
    pub fn render(
        &self,
        outbox_id: &str,
        payload: &EmailOutboxPayload,
    ) -> Result<RenderedEmail, EmailDeliveryError> {
        let outbox_id = Uuid::parse_str(outbox_id)
            .map_err(|_| EmailDeliveryError::permanent(AuthOutboxFailureKind::Payload))?;
        payload
            .email
            .parse::<Address>()
            .map_err(|_| EmailDeliveryError::permanent(AuthOutboxFailureKind::Payload))?;

        let (route, token_key, subject, heading, instruction) = match payload.purpose.as_str() {
            "email_verification" => (
                "verify-email",
                "verification_token",
                "Verify your Canopy email",
                "Verify your email",
                "Use this link to verify your Canopy email address:",
            ),
            "password_reset" => (
                "reset-password",
                "reset_token",
                "Reset your Canopy password",
                "Reset your password",
                "Use this link to reset your Canopy password:",
            ),
            _ => {
                return Err(EmailDeliveryError::permanent(
                    AuthOutboxFailureKind::Payload,
                ));
            }
        };
        let token = payload
            .template_variables
            .get(token_key)
            .ok_or_else(|| EmailDeliveryError::permanent(AuthOutboxFailureKind::Payload))?;
        if token.is_empty() {
            return Err(EmailDeliveryError::permanent(
                AuthOutboxFailureKind::Payload,
            ));
        }

        let mut action_url = self
            .public_base_url
            .join(route)
            .map_err(|_| EmailDeliveryError::permanent(AuthOutboxFailureKind::Payload))?;
        action_url
            .query_pairs_mut()
            .append_pair("token", token)
            .append_pair("expires_at", &payload.expires_at_epoch_ms.to_string());

        let action_url = action_url.as_str();
        let escaped_action_url = html_escape::encode_safe(action_url);
        Ok(RenderedEmail {
            message_id: format!("<canopy-auth-{outbox_id}@{}>", self.from_domain),
            to: payload.email.clone(),
            subject: subject.into(),
            text_body: format!("{heading}\n\n{instruction}\n\n{action_url}\n"),
            html_body: format!(
                "<!doctype html><html><body><h1>{heading}</h1><p>{instruction}</p><p><a href=\"{escaped_action_url}\">Continue in Canopy</a></p></body></html>"
            ),
        })
    }
}

/// TLS-only SMTP transport backed by lettre.
pub struct SmtpEmailSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

fn custom_tls(config: &SmtpConfig) -> CanopyResult<Option<Tls>> {
    let Some(pem) = config.ca_certificate_pem.as_deref() else {
        return Ok(None);
    };
    let certificate = Certificate::from_pem(pem)
        .map_err(|_| CanopyError::InvalidArgument("invalid custom SMTP CA certificate".into()))?;
    let parameters = TlsParametersBuilder::new(config.host.clone())
        .add_root_certificate(certificate)
        .build()
        .map_err(|_| CanopyError::InvalidArgument("invalid SMTP TLS configuration".into()))?;

    Ok(Some(match config.tls_mode {
        SmtpTlsMode::Implicit => Tls::Wrapper(parameters),
        SmtpTlsMode::StartTls => Tls::Required(parameters),
    }))
}

impl SmtpEmailSender {
    /// Builds a sender from validated SMTP configuration.
    pub fn new(config: &SmtpConfig) -> CanopyResult<Self> {
        let mut builder = match config.tls_mode {
            SmtpTlsMode::Implicit => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host),
            SmtpTlsMode::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
            }
        }
        .map_err(|_| CanopyError::InvalidArgument("invalid SMTP relay configuration".into()))?;
        if let Some(tls) = custom_tls(config)? {
            builder = builder.tls(tls);
        }
        let from_address = config.from_address.parse::<Address>().map_err(|_| {
            CanopyError::InvalidArgument("invalid authentication email sender address".into())
        })?;

        Ok(Self {
            transport: builder
                .port(config.port)
                .credentials(Credentials::new(
                    config.username.clone(),
                    config.password.clone(),
                ))
                .timeout(Some(config.timeout))
                .build(),
            from: Mailbox::new(Some(config.from_name.clone()), from_address),
        })
    }

    fn message(&self, email: &RenderedEmail) -> Result<Message, EmailDeliveryError> {
        let recipient = email
            .to
            .parse::<Address>()
            .map(|address| Mailbox::new(None, address))
            .map_err(|_| EmailDeliveryError::permanent(AuthOutboxFailureKind::Payload))?;

        Message::builder()
            .from(self.from.clone())
            .to(recipient)
            .subject(email.subject.clone())
            .message_id(Some(email.message_id.clone()))
            .multipart(MultiPart::alternative_plain_html(
                email.text_body.clone(),
                email.html_body.clone(),
            ))
            .map_err(|_| EmailDeliveryError::permanent(AuthOutboxFailureKind::Payload))
    }
}

#[async_trait]
impl EmailSender for SmtpEmailSender {
    async fn send(&self, email: &RenderedEmail) -> Result<(), EmailDeliveryError> {
        let message = self.message(email)?;
        self.transport
            .send(message)
            .await
            .map(|_| ())
            .map_err(classify_smtp_error)
    }

    async fn probe(&self) -> Result<(), EmailDeliveryError> {
        match self.transport.test_connection().await {
            Ok(true) => Ok(()),
            Ok(false) => Err(EmailDeliveryError::retryable(
                AuthOutboxFailureKind::Connection,
            )),
            Err(error) => Err(classify_smtp_error(error)),
        }
    }
}

fn classify_smtp_error(error: SmtpError) -> EmailDeliveryError {
    if error.is_timeout() {
        return EmailDeliveryError::retryable(AuthOutboxFailureKind::Timeout);
    }
    if error.is_transient() {
        return EmailDeliveryError::retryable(AuthOutboxFailureKind::Rejected);
    }
    if error.is_permanent() {
        let kind = match error.status().map(u16::from) {
            Some(530..=539) => AuthOutboxFailureKind::Authentication,
            _ => AuthOutboxFailureKind::Rejected,
        };
        return EmailDeliveryError::permanent(kind);
    }
    if error.is_client() {
        return EmailDeliveryError::permanent(AuthOutboxFailureKind::Internal);
    }
    EmailDeliveryError::retryable(AuthOutboxFailureKind::Connection)
}
#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use canopy_core::AuthOutboxFailureKind;
    use reqwest::Url;

    use super::*;
    use crate::identity::EmailOutboxPayload;

    const OUTBOX_ID: &str = "00000000-0000-0000-0000-000000000042";

    fn renderer() -> EmailRenderer {
        EmailRenderer::new(
            Url::parse("https://app.example.test/auth/").unwrap(),
            "auth@example.test",
        )
        .unwrap()
    }

    fn smtp_config(tls_mode: SmtpTlsMode) -> SmtpConfig {
        SmtpConfig {
            host: "smtp.example.test".into(),
            port: 465,
            tls_mode,
            username: "canopy".into(),
            password: "secret".into(),
            from_address: "auth@example.test".into(),
            from_name: "Canopy".into(),
            public_base_url: Url::parse("https://app.example.test/auth/").unwrap(),
            timeout: std::time::Duration::from_secs(10),
            ca_certificate_pem: Some(
                include_bytes!("../../../../fixtures/certs/local-integration-test-ca.pem").to_vec(),
            ),
        }
    }

    #[test]
    fn custom_ca_keeps_starttls_required() {
        let tls = custom_tls(&smtp_config(SmtpTlsMode::StartTls))
            .unwrap()
            .unwrap();

        assert!(matches!(
            tls,
            lettre::transport::smtp::client::Tls::Required(_)
        ));
    }

    #[test]
    fn custom_ca_keeps_implicit_tls_wrapped() {
        let tls = custom_tls(&smtp_config(SmtpTlsMode::Implicit))
            .unwrap()
            .unwrap();

        assert!(matches!(
            tls,
            lettre::transport::smtp::client::Tls::Wrapper(_)
        ));
    }

    #[test]
    fn renders_encoded_verification_link_and_deterministic_message_id() {
        let payload = EmailOutboxPayload::email_verification(
            "ada@example.test".into(),
            "token with spaces&symbols",
            42,
        );

        let rendered = renderer().render(OUTBOX_ID, &payload).unwrap();

        assert_eq!(
            rendered.message_id,
            "<canopy-auth-00000000-0000-0000-0000-000000000042@example.test>"
        );
        assert!(rendered.text_body.contains(
            "https://app.example.test/auth/verify-email?token=token+with+spaces%26symbols&expires_at=42"
        ));
        assert!(!rendered.text_body.contains("token with spaces&symbols"));
    }

    #[test]
    fn renders_encoded_password_reset_link() {
        let payload = EmailOutboxPayload::password_reset(
            "ada@example.test".into(),
            "reset/token?secret=yes",
            84,
        );

        let rendered = renderer().render(OUTBOX_ID, &payload).unwrap();

        assert!(rendered.text_body.contains(
            "https://app.example.test/auth/reset-password?token=reset%2Ftoken%3Fsecret%3Dyes&expires_at=84"
        ));
        assert_eq!(rendered.subject, "Reset your Canopy password");
    }

    #[test]
    fn rejects_unknown_purpose() {
        let payload = EmailOutboxPayload {
            email: "ada@example.test".into(),
            purpose: "unknown".into(),
            expires_at_epoch_ms: 42,
            template_variables: BTreeMap::new(),
        };

        let error = renderer().render(OUTBOX_ID, &payload).unwrap_err();

        assert_eq!(
            error,
            EmailDeliveryError::permanent(AuthOutboxFailureKind::Payload)
        );
    }

    #[test]
    fn rejects_missing_template_token() {
        let payload = EmailOutboxPayload {
            email: "ada@example.test".into(),
            purpose: "email_verification".into(),
            expires_at_epoch_ms: 42,
            template_variables: BTreeMap::new(),
        };

        let error = renderer().render(OUTBOX_ID, &payload).unwrap_err();

        assert_eq!(
            error,
            EmailDeliveryError::permanent(AuthOutboxFailureKind::Payload)
        );
    }

    #[test]
    fn escapes_link_for_html_body() {
        let payload =
            EmailOutboxPayload::email_verification("ada@example.test".into(), "token&unsafe", 42);

        let rendered = renderer().render(OUTBOX_ID, &payload).unwrap();

        assert!(rendered.html_body.contains("%26unsafe&amp;expires_at=42"));
        assert!(!rendered.html_body.contains("%26unsafe&expires_at=42"));
    }

    #[test]
    fn rendered_email_debug_redacts_recipient_and_bodies() {
        let payload =
            EmailOutboxPayload::email_verification("ada@example.test".into(), "secret-token", 42);
        let rendered = renderer().render(OUTBOX_ID, &payload).unwrap();

        let debug = format!("{rendered:?}");

        assert!(!debug.contains("ada@example.test"));
        assert!(!debug.contains("secret-token"));
        assert!(!debug.contains("verify-email"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn delivery_error_debug_contains_only_safe_classification() {
        let error = EmailDeliveryError::retryable(AuthOutboxFailureKind::Connection);

        assert_eq!(
            format!("{error:?}"),
            "EmailDeliveryError { kind: Connection, retryable: true }"
        );
    }
}
