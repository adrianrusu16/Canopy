//! Capability-based authorization for Canopy-managed media streams.

pub mod artwork;
pub mod authorizer;
pub mod http;
pub mod token;

pub use artwork::{ArtworkAuthorizer, ArtworkGrant};
pub use authorizer::{StreamAuthorizer, StreamGrant};
pub use http::{serve_stream_auth, stream_auth_router};
pub use token::{StreamClaims, StreamTokenCodec};
