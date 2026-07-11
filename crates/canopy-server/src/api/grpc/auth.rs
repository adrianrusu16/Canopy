use std::sync::Arc;

use canopy_core::{AccountRecord, AccountStatus, AuthSession};
use canopy_proto::auth_service_server::AuthService;
use canopy_proto::{
    AccountLinkRequired, AccountSummary, BeginGoogleLoginRequest, BeginGoogleLoginResponse,
    ChangePasswordRequest, CompleteGoogleLoginRequest, CompletePasswordResetRequest,
    DeleteAccountRequest, GenericAuthResponse, GetAccountRequest, GetAccountResponse,
    GoogleLoginResponse, LinkGoogleRequest, ListSessionsRequest, ListSessionsResponse,
    LoginPasswordRequest, LogoutAllRequest, LogoutRequest, PageInfo, RefreshSessionRequest,
    RegisterPasswordRequest, RequestPasswordResetRequest, ResendVerificationRequest,
    RevokeSessionRequest, SessionEnvelope, SessionSummary, UnlinkGoogleRequest, VerifyEmailRequest,
    google_login_response,
};
use tonic::{Request, Response, Status, metadata::MetadataMap};

use crate::api::to_status;
use crate::identity::{
    AuthenticatedPrincipal, BeginGoogleLoginCommand, ChangePasswordCommand,
    CompleteGoogleLoginCommand, CompletePasswordResetCommand, GoogleLoginOutcome, IdentityService,
    LinkGoogleCommand, LoginPasswordCommand, RefreshSessionCommand, RegisterPasswordCommand,
    RequestPasswordResetCommand, ResendVerificationCommand,
    SessionEnvelope as DomainSessionEnvelope, UnlinkGoogleCommand, VerifyEmailCommand,
};

pub struct AuthGrpc(pub Arc<IdentityService>);

impl AuthGrpc {
    async fn authenticate<T>(
        &self,
        request: &Request<T>,
    ) -> Result<AuthenticatedPrincipal, Status> {
        let token = access_token_from_metadata(request.metadata()).map_err(to_status)?;
        self.0
            .authenticate_access_token(token)
            .await
            .map_err(to_status)
    }
}

#[tonic::async_trait]
impl AuthService for AuthGrpc {
    async fn register_password(
        &self,
        request: Request<RegisterPasswordRequest>,
    ) -> Result<Response<GenericAuthResponse>, Status> {
        let request = request.into_inner();
        self.0
            .register_password(RegisterPasswordCommand {
                email: request.email,
                password: request.password,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(GenericAuthResponse { accepted: true }))
    }

    async fn resend_verification(
        &self,
        request: Request<ResendVerificationRequest>,
    ) -> Result<Response<GenericAuthResponse>, Status> {
        let request = request.into_inner();
        self.0
            .resend_verification(ResendVerificationCommand {
                email: request.email,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(GenericAuthResponse { accepted: true }))
    }

    async fn verify_email(
        &self,
        request: Request<VerifyEmailRequest>,
    ) -> Result<Response<SessionEnvelope>, Status> {
        let request = request.into_inner();
        let envelope = self
            .0
            .verify_email(VerifyEmailCommand {
                verification_token: request.verification_token,
                device_label: request.device_label,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(to_proto_session_envelope(envelope)))
    }

    async fn login_password(
        &self,
        request: Request<LoginPasswordRequest>,
    ) -> Result<Response<SessionEnvelope>, Status> {
        let request = request.into_inner();
        let envelope = self
            .0
            .login_password(LoginPasswordCommand {
                email: request.email,
                password: request.password,
                device_label: request.device_label,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(to_proto_session_envelope(envelope)))
    }

    async fn request_password_reset(
        &self,
        request: Request<RequestPasswordResetRequest>,
    ) -> Result<Response<GenericAuthResponse>, Status> {
        let request = request.into_inner();
        self.0
            .request_password_reset(RequestPasswordResetCommand {
                email: request.email,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(GenericAuthResponse { accepted: true }))
    }

    async fn complete_password_reset(
        &self,
        request: Request<CompletePasswordResetRequest>,
    ) -> Result<Response<GenericAuthResponse>, Status> {
        let request = request.into_inner();
        self.0
            .complete_password_reset(CompletePasswordResetCommand {
                reset_token: request.reset_token,
                new_password: request.new_password,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(GenericAuthResponse { accepted: true }))
    }

    async fn change_password(
        &self,
        request: Request<ChangePasswordRequest>,
    ) -> Result<Response<GenericAuthResponse>, Status> {
        let principal = self.authenticate(&request).await?;
        let request = request.into_inner();
        self.0
            .change_password(ChangePasswordCommand {
                principal,
                current_password: request.current_password,
                new_password: request.new_password,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(GenericAuthResponse { accepted: true }))
    }

    async fn begin_google_login(
        &self,
        _request: Request<BeginGoogleLoginRequest>,
    ) -> Result<Response<BeginGoogleLoginResponse>, Status> {
        let challenge = self
            .0
            .begin_google_login(BeginGoogleLoginCommand)
            .await
            .map_err(to_status)?;
        Ok(Response::new(BeginGoogleLoginResponse {
            challenge_id: challenge.challenge_id,
            nonce: challenge.nonce,
            expires_at: Some(timestamp_from_epoch_ms(challenge.expires_at_epoch_ms)),
        }))
    }

    async fn complete_google_login(
        &self,
        request: Request<CompleteGoogleLoginRequest>,
    ) -> Result<Response<GoogleLoginResponse>, Status> {
        let request = request.into_inner();
        let outcome = self
            .0
            .complete_google_login(CompleteGoogleLoginCommand {
                challenge_id: request.challenge_id,
                id_token: request.id_token,
                device_label: request.device_label,
            })
            .await
            .map_err(to_status)?;
        let result = match outcome {
            GoogleLoginOutcome::Session(envelope) => {
                google_login_response::Result::Session(to_proto_session_envelope(envelope))
            }
            GoogleLoginOutcome::AccountLinkRequired { link_challenge_id } => {
                google_login_response::Result::AccountLinkRequired(AccountLinkRequired {
                    link_challenge_id,
                })
            }
        };
        Ok(Response::new(GoogleLoginResponse {
            result: Some(result),
        }))
    }

    async fn link_google(
        &self,
        request: Request<LinkGoogleRequest>,
    ) -> Result<Response<GenericAuthResponse>, Status> {
        let principal = self.authenticate(&request).await?;
        let request = request.into_inner();
        self.0
            .link_google(LinkGoogleCommand {
                principal,
                link_challenge_id: request.link_challenge_id,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(GenericAuthResponse { accepted: true }))
    }

    async fn unlink_google(
        &self,
        request: Request<UnlinkGoogleRequest>,
    ) -> Result<Response<GenericAuthResponse>, Status> {
        let principal = self.authenticate(&request).await?;
        self.0
            .unlink_google(UnlinkGoogleCommand { principal })
            .await
            .map_err(to_status)?;
        Ok(Response::new(GenericAuthResponse { accepted: true }))
    }

    async fn refresh_session(
        &self,
        request: Request<RefreshSessionRequest>,
    ) -> Result<Response<SessionEnvelope>, Status> {
        let request = request.into_inner();
        let envelope = self
            .0
            .refresh_session(RefreshSessionCommand {
                refresh_token: request.refresh_token,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(to_proto_session_envelope(envelope)))
    }

    async fn logout(
        &self,
        request: Request<LogoutRequest>,
    ) -> Result<Response<GenericAuthResponse>, Status> {
        let principal = self.authenticate(&request).await?;
        self.0.logout(&principal).await.map_err(to_status)?;
        Ok(Response::new(GenericAuthResponse { accepted: true }))
    }

    async fn logout_all(
        &self,
        request: Request<LogoutAllRequest>,
    ) -> Result<Response<GenericAuthResponse>, Status> {
        let principal = self.authenticate(&request).await?;
        self.0.logout_all(&principal).await.map_err(to_status)?;
        Ok(Response::new(GenericAuthResponse { accepted: true }))
    }

    async fn list_sessions(
        &self,
        request: Request<ListSessionsRequest>,
    ) -> Result<Response<ListSessionsResponse>, Status> {
        let principal = self.authenticate(&request).await?;
        let sessions = self.0.list_sessions(&principal).await.map_err(to_status)?;
        Ok(Response::new(ListSessionsResponse {
            sessions: sessions
                .into_iter()
                .map(|session| {
                    let current = session.id == principal.session_id;
                    to_proto_session(session, current)
                })
                .collect(),
            page_info: Some(PageInfo {
                next_page_token: String::new(),
            }),
        }))
    }

    async fn revoke_session(
        &self,
        request: Request<RevokeSessionRequest>,
    ) -> Result<Response<GenericAuthResponse>, Status> {
        let principal = self.authenticate(&request).await?;
        let request = request.into_inner();
        self.0
            .revoke_session(&principal, &request.session_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(GenericAuthResponse { accepted: true }))
    }

    async fn get_account(
        &self,
        request: Request<GetAccountRequest>,
    ) -> Result<Response<GetAccountResponse>, Status> {
        let principal = self.authenticate(&request).await?;
        let account = self.0.get_account(&principal).await.map_err(to_status)?;
        Ok(Response::new(GetAccountResponse {
            account: Some(to_proto_account(account)),
        }))
    }

    async fn delete_account(
        &self,
        request: Request<DeleteAccountRequest>,
    ) -> Result<Response<GenericAuthResponse>, Status> {
        let principal = self.authenticate(&request).await?;
        self.0.delete_account(&principal).await.map_err(to_status)?;
        Ok(Response::new(GenericAuthResponse { accepted: true }))
    }
}

fn timestamp_from_epoch_ms(epoch_ms: u64) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: (epoch_ms / 1000) as i64,
        nanos: ((epoch_ms % 1000) * 1_000_000) as i32,
    }
}

fn to_proto_session_envelope(envelope: DomainSessionEnvelope) -> SessionEnvelope {
    SessionEnvelope {
        access_token: envelope.access_token,
        refresh_token: envelope.refresh_token,
        access_expires_at_epoch_ms: 0,
        refresh_expires_at_epoch_ms: i64::try_from(envelope.session.expires_at_epoch_ms)
            .unwrap_or(i64::MAX),
        account: Some(to_proto_account(envelope.account)),
        session: Some(to_proto_session(envelope.session, true)),
    }
}

fn to_proto_account(account: AccountRecord) -> AccountSummary {
    AccountSummary {
        id: account.id,
        primary_email: account.primary_email.unwrap_or_default(),
        status: account_status_name(account.status).into(),
        created_at: None,
    }
}

fn to_proto_session(session: AuthSession, current: bool) -> SessionSummary {
    SessionSummary {
        id: session.id,
        device_label: session.device_label,
        created_at: None,
        last_used_at: None,
        expires_at: None,
        current,
    }
}

fn account_status_name(status: AccountStatus) -> &'static str {
    match status {
        AccountStatus::PendingEmailVerification => "pending_email_verification",
        AccountStatus::Active => "active",
        AccountStatus::Disabled => "disabled",
        AccountStatus::DeletionPending => "deletion_pending",
        AccountStatus::Deleted => "deleted",
    }
}

fn access_token_from_metadata(metadata: &MetadataMap) -> canopy_core::CanopyResult<&str> {
    let Some(raw) = metadata.get("authorization") else {
        return Err(canopy_core::CanopyError::unauthenticated(
            "missing access token",
        ));
    };
    let value = raw
        .to_str()
        .map_err(|_| canopy_core::CanopyError::unauthenticated("invalid authorization metadata"))?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            canopy_core::CanopyError::unauthenticated("authorization must use Bearer token")
        })
}
