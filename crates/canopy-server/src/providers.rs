//! Provider adapters — ingestion pipeline entry point.
//!
//! Delegates to the submodules defined in `providers/`.

pub mod adapter;
pub mod fixture;
pub mod service;

pub use adapter::ProviderAdapter;
pub use fixture::TestFixtureProvider;
pub use service::IngestionService;
