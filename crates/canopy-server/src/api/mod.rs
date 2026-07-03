//! API adapters — the "driving" side of the system.
//!
//! These adapters translate the public gRPC protocol into
//! calls on the domain services, and translate domain results and
//! [`canopy_core::CanopyError`] back into protocol responses. The domain layer
//! has no knowledge of the transport.

pub mod grpc;

use canopy_core::CanopyError;
use tonic::Status;

/// Maps a transport-agnostic [`CanopyError`] onto a gRPC [`Status`].
///
/// The orphan rule prevents a `From` impl (both types are foreign), so this
/// free function is the single conversion point used by the gRPC adapter.
pub(crate) fn to_status(err: CanopyError) -> Status {
    match err {
        CanopyError::NotFound { entity, id } => {
            Status::not_found(format!("{entity} not found: {id}"))
        }
        CanopyError::InvalidArgument(message) => Status::invalid_argument(message),
        CanopyError::Unauthenticated(message) => Status::unauthenticated(message),
        CanopyError::FailedPrecondition(message) => Status::failed_precondition(message),
        CanopyError::Aborted(message) => Status::aborted(message),
        CanopyError::Storage(message) => Status::unavailable(message),
        CanopyError::Internal(message) => Status::internal(message),
    }
}

#[cfg(test)]
mod tests {
    use tonic::Code;

    use super::*;

    #[test]
    fn maps_domain_errors_to_canonical_grpc_statuses() {
        let cases = [
            (
                CanopyError::InvalidArgument("bad".into()),
                Code::InvalidArgument,
                "bad",
            ),
            (
                CanopyError::Unauthenticated("auth".into()),
                Code::Unauthenticated,
                "auth",
            ),
            (
                CanopyError::FailedPrecondition("state".into()),
                Code::FailedPrecondition,
                "state",
            ),
            (CanopyError::Aborted("stale".into()), Code::Aborted, "stale"),
            (
                CanopyError::Storage("database".into()),
                Code::Unavailable,
                "database",
            ),
            (CanopyError::Internal("bug".into()), Code::Internal, "bug"),
        ];

        for (error, code, message) in cases {
            let status = to_status(error);
            assert_eq!(status.code(), code);
            assert_eq!(status.message(), message);
        }
    }

    #[test]
    fn maps_not_found_without_transport_specific_domain_state() {
        let status = to_status(CanopyError::not_found("track", "trk_1"));

        assert_eq!(status.code(), Code::NotFound);
        assert_eq!(status.message(), "track not found: trk_1");
    }
}
