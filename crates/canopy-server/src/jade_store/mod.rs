//! JadeStore — the persistence layer.
//!
//! This module is the home of all storage-backend implementations of the
//! [`canopy_core`] repository ports. Today it ships in-memory
//! implementations used by the prototype; the future PostgreSQL (`sqlx`) and
//! RustFS backends will live alongside them and be selected at wiring time.

mod memory;

#[cfg(feature = "pg")]
mod pg;

pub use memory::{InMemoryAudioAssetStore, InMemoryCatalog, InMemorySessionStore};

#[cfg(feature = "pg")]
pub use pg::{PgAudioAssetRepository, PgCatalogRepository, PgSessionRepository};
