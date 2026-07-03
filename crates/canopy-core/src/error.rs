//! Transport-agnostic error type for the Canopy domain.
//!
//! Domain and repository code returns [`CanopyError`] rather than a
//! transport-specific error (such as `tonic::Status`). The mapping to a wire
//! status code lives in the API adapter layer, which keeps the domain free of
//! any knowledge of how it is exposed over gRPC or private HTTP infrastructure.

use thiserror::Error;

/// Errors that can occur while serving a Canopy domain operation.
#[derive(Debug, Error)]
pub enum CanopyError {
    /// The requested entity does not exist.
    #[error("{entity} not found: {id}")]
    NotFound {
        /// Kind of entity that was looked up (e.g. `"media"`, `"session"`).
        entity: &'static str,
        /// Identifier that was requested.
        id: String,
    },

    /// The request was malformed or violated a domain invariant.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// The caller could not be authenticated (missing, malformed, expired, or
    /// improperly signed credential).
    #[error("unauthenticated: {0}")]
    Unauthenticated(String),

    /// The operation cannot proceed while the resource is in its current state.
    #[error("failed precondition: {0}")]
    FailedPrecondition(String),

    /// The operation lost an optimistic concurrency race and may be retried.
    #[error("aborted: {0}")]
    Aborted(String),

    /// A persistence or managed-media dependency failed.
    #[error("storage error: {0}")]
    Storage(String),

    /// An otherwise unclassified internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl CanopyError {
    /// Convenience constructor for [`CanopyError::NotFound`].
    pub fn not_found(entity: &'static str, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity,
            id: id.into(),
        }
    }

    /// Convenience constructor for [`CanopyError::Unauthenticated`].
    pub fn unauthenticated(reason: impl Into<String>) -> Self {
        Self::Unauthenticated(reason.into())
    }
}

/// Convenience result alias for fallible domain operations.
pub type CanopyResult<T> = Result<T, CanopyError>;
