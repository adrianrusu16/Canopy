//! Shared gRPC wire contract for the Canopy backend.
//!
//! This crate contains nothing but the types generated from
//! `proto/canopy.proto`. It is the single source of truth for the wire
//! contract between PandaEngine (client) and Canopy (server); both sides
//! depend on it rather than maintaining their own copy of the message shapes.

tonic::include_proto!("canopy");
