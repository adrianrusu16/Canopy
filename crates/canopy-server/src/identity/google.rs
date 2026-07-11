use std::collections::HashSet;

use async_trait::async_trait;
use canopy_core::{CanopyError, CanopyResult};
use serde::Deserialize;

/// Google identity facts proven by an OIDC ID token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoogleIdentity {
    /// Immutable Google subject (`sub`).
    pub subject: String,
    /// Email claim observed in the ID token, when present.
    pub email: Option<String>,
    /// Whether Google asserted the email as verified.
    pub email_verified: bool,
}

/// Boundary for Google OIDC ID-token verification.
#[async_trait]
pub trait OidcVerifier: Send + Sync {
    async fn verify_google_id_token(
        &self,
        id_token: &str,
        nonce: &str,
    ) -> CanopyResult<GoogleIdentity>;
}

/// Verifies Google ID tokens through Google's tokeninfo endpoint.
pub struct GoogleTokenInfoOidcVerifier {
    client: reqwest::Client,
    tokeninfo_url: reqwest::Url,
    client_ids: HashSet<String>,
}

impl GoogleTokenInfoOidcVerifier {
    pub fn new(
        client_ids: impl IntoIterator<Item = String>,
        tokeninfo_url: reqwest::Url,
    ) -> CanopyResult<Self> {
        let client_ids: HashSet<_> = client_ids
            .into_iter()
            .map(|client_id| client_id.trim().to_owned())
            .filter(|client_id| !client_id.is_empty())
            .collect();
        if client_ids.is_empty() {
            return Err(CanopyError::InvalidArgument(
                "at least one Google OIDC client id is required".into(),
            ));
        }

        Ok(Self {
            client: reqwest::Client::new(),
            tokeninfo_url,
            client_ids,
        })
    }

    fn validate_response(
        client_ids: &HashSet<String>,
        response: GoogleTokenInfoResponse,
        expected_nonce: &str,
    ) -> CanopyResult<GoogleIdentity> {
        if !client_ids.contains(response.aud.trim()) {
            return Err(invalid_google_token());
        }
        if response.sub.trim().is_empty() {
            return Err(invalid_google_token());
        }
        if response.nonce.as_deref() != Some(expected_nonce) {
            return Err(invalid_google_token());
        }

        Ok(GoogleIdentity {
            subject: response.sub,
            email: response
                .email
                .map(|email| email.trim().to_owned())
                .filter(|email| !email.is_empty()),
            email_verified: response.email_verified.is_some_and(|value| value.as_bool()),
        })
    }
}

#[async_trait]
impl OidcVerifier for GoogleTokenInfoOidcVerifier {
    async fn verify_google_id_token(
        &self,
        id_token: &str,
        nonce: &str,
    ) -> CanopyResult<GoogleIdentity> {
        if id_token.trim().is_empty() || nonce.trim().is_empty() {
            return Err(invalid_google_token());
        }

        let response = self
            .client
            .get(self.tokeninfo_url.clone())
            .query(&[("id_token", id_token)])
            .send()
            .await
            .map_err(|_| CanopyError::Internal("google oidc verification failed".into()))?;
        if !response.status().is_success() {
            return Err(invalid_google_token());
        }

        let tokeninfo = response
            .json::<GoogleTokenInfoResponse>()
            .await
            .map_err(|_| CanopyError::Internal("google oidc verification failed".into()))?;
        Self::validate_response(&self.client_ids, tokeninfo, nonce)
    }
}

fn invalid_google_token() -> CanopyError {
    CanopyError::unauthenticated("invalid google id token")
}

#[derive(Deserialize)]
struct GoogleTokenInfoResponse {
    aud: String,
    sub: String,
    email: Option<String>,
    email_verified: Option<GoogleBool>,
    nonce: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum GoogleBool {
    Bool(bool),
    String(String),
}

impl GoogleBool {
    fn as_bool(&self) -> bool {
        match self {
            Self::Bool(value) => *value,
            Self::String(value) => value.eq_ignore_ascii_case("true"),
        }
    }
}

/// Fail-closed verifier used until production Google OIDC config is supplied.
#[derive(Debug, Default)]
pub struct NoopOidcVerifier;

#[async_trait]
impl OidcVerifier for NoopOidcVerifier {
    async fn verify_google_id_token(
        &self,
        _id_token: &str,
        _nonce: &str,
    ) -> CanopyResult<GoogleIdentity> {
        Err(CanopyError::FailedPrecondition(
            "google oidc verifier is not configured".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{GoogleBool, GoogleTokenInfoOidcVerifier, GoogleTokenInfoResponse};

    fn client_ids() -> HashSet<String> {
        HashSet::from(["client-1.apps.googleusercontent.com".to_owned()])
    }

    #[test]
    fn tokeninfo_validation_uses_audience_subject_and_nonce() {
        let identity = GoogleTokenInfoOidcVerifier::validate_response(
            &client_ids(),
            GoogleTokenInfoResponse {
                aud: "client-1.apps.googleusercontent.com".into(),
                sub: "google-subject".into(),
                email: Some("person@example.test".into()),
                email_verified: Some(GoogleBool::String("true".into())),
                nonce: Some("expected-nonce".into()),
            },
            "expected-nonce",
        )
        .unwrap();

        assert_eq!(identity.subject, "google-subject");
        assert_eq!(identity.email.as_deref(), Some("person@example.test"));
        assert!(identity.email_verified);
    }

    #[test]
    fn tokeninfo_validation_rejects_wrong_audience_or_nonce() {
        let wrong_audience = GoogleTokenInfoOidcVerifier::validate_response(
            &client_ids(),
            GoogleTokenInfoResponse {
                aud: "other-client".into(),
                sub: "google-subject".into(),
                email: None,
                email_verified: Some(GoogleBool::Bool(true)),
                nonce: Some("expected-nonce".into()),
            },
            "expected-nonce",
        );
        assert!(wrong_audience.is_err());

        let wrong_nonce = GoogleTokenInfoOidcVerifier::validate_response(
            &client_ids(),
            GoogleTokenInfoResponse {
                aud: "client-1.apps.googleusercontent.com".into(),
                sub: "google-subject".into(),
                email: None,
                email_verified: Some(GoogleBool::Bool(true)),
                nonce: Some("different-nonce".into()),
            },
            "expected-nonce",
        );
        assert!(wrong_nonce.is_err());
    }
}
