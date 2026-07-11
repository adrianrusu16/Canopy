//! Authentication and account-management application services.

mod google;
mod outbox;
mod password;
mod service;
mod tokens;

pub use google::{GoogleIdentity, GoogleTokenInfoOidcVerifier, NoopOidcVerifier, OidcVerifier};
pub use outbox::{EmailOutboxPayload, SealedOutboxPayload};
pub use password::{Argon2PasswordHasher, PasswordHasher, PasswordPolicy, PasswordVerification};
pub use service::{
    AuthenticatedPrincipal, BeginGoogleLoginCommand, ChangePasswordCommand, Clock,
    CompleteGoogleLoginCommand, CompletePasswordResetCommand, FixedClock, GoogleLoginChallenge,
    GoogleLoginOutcome, IdentityService, LinkGoogleCommand, LoginPasswordCommand,
    RefreshSessionCommand, RegisterPasswordCommand, RequestPasswordResetCommand,
    ResendVerificationCommand, SessionEnvelope, SystemClock, UnlinkGoogleCommand,
    VerifyEmailCommand,
};
pub use tokens::{
    AccessTokenClaims, AccessTokenConfig, Ed25519AccessTokenIssuer, OpaqueToken, TokenDigest,
};
