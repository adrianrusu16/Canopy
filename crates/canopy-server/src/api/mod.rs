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
        CanopyError::NotFound { .. } => Status::not_found(err.to_string()),
        CanopyError::InvalidArgument(_) => Status::invalid_argument(err.to_string()),
        CanopyError::Unauthenticated(_) => Status::unauthenticated(err.to_string()),
        CanopyError::Storage(_) | CanopyError::Internal(_) => Status::internal(err.to_string()),
    }
}
