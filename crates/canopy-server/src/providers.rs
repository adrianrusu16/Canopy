//! Provider adapters — ingestion pipeline entry point.
//!
//! The deterministic fixture adapter is the supported catalog source.

pub mod adapter;
pub mod fixture;
pub mod service;

pub use adapter::ProviderAdapter;
pub use fixture::TestFixtureProvider;
pub use service::IngestionService;
