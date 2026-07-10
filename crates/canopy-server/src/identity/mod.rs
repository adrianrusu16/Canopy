//! Authentication and account-management application services.

mod outbox;
mod password;
mod service;
mod tokens;

pub use outbox::{EmailOutboxPayload, SealedOutboxPayload};
pub use password::{Argon2PasswordHasher, PasswordHasher, PasswordPolicy, PasswordVerification};
pub use service::{
    AuthenticatedPrincipal, ChangePasswordCommand, Clock, CompletePasswordResetCommand, FixedClock,
    IdentityService, LoginPasswordCommand, RefreshSessionCommand, RegisterPasswordCommand,
    RequestPasswordResetCommand, ResendVerificationCommand, SessionEnvelope, SystemClock,
    VerifyEmailCommand,
};
pub use tokens::{
    AccessTokenClaims, AccessTokenConfig, Ed25519AccessTokenIssuer, OpaqueToken, TokenDigest,
};
