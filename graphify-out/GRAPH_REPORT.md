# Graph Report - .  (2026-06-17)

## Corpus Check
- 113 files · ~83,843 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 378 nodes · 658 edges · 28 communities
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_crates canopy-server src api grpc.rs|crates canopy-server src api grpc.rs]]
- [[_COMMUNITY_crates canopy-server src playback.rs|crates canopy-server src playback.rs]]
- [[_COMMUNITY_crates canopy-server src jade_store pg.rs|crates canopy-server src jade_store pg.rs]]
- [[_COMMUNITY_crates canopy-server src jade_store memory.rs|crates canopy-server src jade_store memory.rs]]
- [[_COMMUNITY_crates canopy-server src auth.rs|crates canopy-server src auth.rs]]
- [[_COMMUNITY_crates canopy-server src discovery.rs|crates canopy-server src discovery.rs]]
- [[_COMMUNITY_crates canopy-server src signing.rs|crates canopy-server src signing.rs]]
- [[_COMMUNITY_crates canopy-server src catalog.rs|crates canopy-server src catalog.rs]]
- [[_COMMUNITY_crates canopy-core src model.rs|crates canopy-core src model.rs]]
- [[_COMMUNITY_crates canopy-server src search.rs|crates canopy-server src search.rs]]
- [[_COMMUNITY_crates canopy-server src providers fixture.rs|crates canopy-server src providers fixture.rs]]
- [[_COMMUNITY_crates canopy-server tests domain.rs|crates canopy-server tests domain.rs]]
- [[_COMMUNITY_crates canopy-server src providers service.rs|crates canopy-server src providers service.rs]]
- [[_COMMUNITY_crates canopy-core src repository.rs|crates canopy-core src repository.rs]]
- [[_COMMUNITY_crates canopy-server src health.rs|crates canopy-server src health.rs]]
- [[_COMMUNITY_crates canopy-server src lib.rs|crates canopy-server src lib.rs]]
- [[_COMMUNITY_crates canopy-server src config.rs|crates canopy-server src config.rs]]
- [[_COMMUNITY_crates canopy-core src error.rs|crates canopy-core src error.rs]]
- [[_COMMUNITY_crates canopy-proto build.rs|crates canopy-proto build.rs]]
- [[_COMMUNITY_crates canopy-server src main.rs|crates canopy-server src main.rs]]
- [[_COMMUNITY_crates canopy-server src observability.rs|crates canopy-server src observability.rs]]
- [[_COMMUNITY_crates canopy-server src api mod.rs|crates canopy-server src api mod.rs]]
- [[_COMMUNITY_crates canopy-core src signing.rs|crates canopy-core src signing.rs]]
- [[_COMMUNITY_crates canopy-server src providers adapter.rs|crates canopy-server src providers adapter.rs]]

## God Nodes (most connected - your core abstractions)
1. `GrpcApi` - 23 edges
2. `Request` - 14 edges
3. `Result` - 14 edges
4. `Response` - 14 edges
5. `Status` - 14 edges
6. `PgCatalogRepository` - 11 edges
7. `ResolverService` - 11 edges
8. `InMemoryCatalog` - 10 edges
9. `InMemorySessionStore` - 10 edges
10. `CanopyResult` - 10 edges

## Surprising Connections (you probably didn't know these)
- None detected - all connections are within the same source files.

## Import Cycles
- 1-file cycle: `crates/canopy-server/src/api/grpc.rs -> crates/canopy-server/src/api/grpc.rs`
- 1-file cycle: `crates/canopy-server/src/lib.rs -> crates/canopy-server/src/lib.rs`
- 1-file cycle: `crates/canopy-server/src/api/mod.rs -> crates/canopy-server/src/api/mod.rs`
- 1-file cycle: `crates/canopy-server/src/catalog.rs -> crates/canopy-server/src/catalog.rs`
- 1-file cycle: `crates/canopy-server/src/config.rs -> crates/canopy-server/src/config.rs`
- 1-file cycle: `crates/canopy-server/src/discovery.rs -> crates/canopy-server/src/discovery.rs`
- 1-file cycle: `crates/canopy-server/src/health.rs -> crates/canopy-server/src/health.rs`
- 1-file cycle: `crates/canopy-server/src/jade_store/memory.rs -> crates/canopy-server/src/jade_store/memory.rs`
- 1-file cycle: `crates/canopy-server/src/jade_store/pg.rs -> crates/canopy-server/src/jade_store/pg.rs`
- 1-file cycle: `crates/canopy-server/src/playback.rs -> crates/canopy-server/src/playback.rs`
- 1-file cycle: `crates/canopy-server/src/providers/fixture.rs -> crates/canopy-server/src/providers/fixture.rs`
- 1-file cycle: `crates/canopy-server/src/providers/service.rs -> crates/canopy-server/src/providers/service.rs`
- 1-file cycle: `crates/canopy-server/src/search.rs -> crates/canopy-server/src/search.rs`
- 1-file cycle: `crates/canopy-server/src/signing.rs -> crates/canopy-server/src/signing.rs`

## Communities (28 total, 0 thin omitted)

### Community 0 - "crates canopy-server src api grpc.rs"
Cohesion: 0.07
Nodes (47): GrpcApi, to_proto_item(), to_proto_items(), BrowseRequest, BrowseResponse, Canopy, CatalogService, MediaPage (+39 more)

### Community 1 - "crates canopy-server src playback.rs"
Cohesion: 0.12
Nodes (24): Arc, AudioAsset, AudioAssetRepository, CanopyResult, Duration, Option, Self, Session (+16 more)

### Community 2 - "crates canopy-server src jade_store pg.rs"
Cohesion: 0.12
Nodes (19): Arc, AudioAsset, AudioAssetRepository, CanopyResult, CatalogRepository, DiscoveryRepository, MediaItem, MediaPage (+11 more)

### Community 3 - "crates canopy-server src jade_store memory.rs"
Cohesion: 0.13
Nodes (19): AudioAsset, AudioAssetRepository, CanopyResult, CatalogRepository, DiscoveryRepository, MediaItem, MediaPage, Option (+11 more)

### Community 4 - "crates canopy-server src auth.rs"
Cohesion: 0.20
Nodes (15): CanopyResult, Duration, Into, Self, String, Vec, AuthService, constant_time_eq() (+7 more)

### Community 5 - "crates canopy-server src discovery.rs"
Cohesion: 0.18
Nodes (14): Arc, CanopyResult, DiscoveryRepository, MediaItem, MediaPage, Self, String, Vec (+6 more)

### Community 6 - "crates canopy-server src signing.rs"
Cohesion: 0.24
Nodes (11): Into, Self, String, UrlSigner, Vec, constant_time_eq(), HmacUrlSigner, sign_is_deterministic_and_input_dependent() (+3 more)

### Community 7 - "crates canopy-server src catalog.rs"
Cohesion: 0.21
Nodes (10): Arc, CanopyResult, CatalogRepository, MediaItem, MediaPage, Option, Page, Self (+2 more)

### Community 8 - "crates canopy-core src model.rs"
Cohesion: 0.27
Nodes (13): Option, String, Vec, AudioAsset, MediaItem, MediaPage, Page, PlaybackSource (+5 more)

### Community 9 - "crates canopy-server src search.rs"
Cohesion: 0.21
Nodes (9): Arc, CanopyResult, CatalogRepository, MediaPage, Page, Self, String, normalize_query() (+1 more)

### Community 10 - "crates canopy-server src providers fixture.rs"
Cohesion: 0.22
Nodes (9): AsRef, CanopyResult, ProviderAdapter, ProviderTrack, Self, String, Vec, Path (+1 more)

### Community 11 - "crates canopy-server tests domain.rs"
Cohesion: 0.28
Nodes (10): MediaItem, Vec, browse_paginates(), discovery_excludes_recently_played(), get_media_returns_none_for_unknown(), page(), sample_items(), search_matches_title_and_artist() (+2 more)

### Community 12 - "crates canopy-server src providers service.rs"
Cohesion: 0.24
Nodes (9): CatalogIngest, Arc, CanopyResult, ProviderAdapter, ProviderTrack, Self, IngestBatchResult, IngestionService (+1 more)

### Community 13 - "crates canopy-core src repository.rs"
Cohesion: 0.33
Nodes (10): Send, String, Sync, Vec, AudioAssetRepository, CatalogIngest, CatalogRepository, DiscoveryRepository (+2 more)

### Community 14 - "crates canopy-server src health.rs"
Cohesion: 0.27
Nodes (7): Arc, Option, PgPool, Self, String, HealthService, HealthStatus

### Community 15 - "crates canopy-server src lib.rs"
Cohesion: 0.22
Nodes (9): Config, Box, Error, Result, InMemoryAudioAssetStore, InMemoryCatalog, demo_assets(), demo_catalog() (+1 more)

### Community 16 - "crates canopy-server src config.rs"
Cohesion: 0.25
Nodes (7): Box, Error, Result, Self, String, SocketAddr, Config

### Community 17 - "crates canopy-core src error.rs"
Cohesion: 0.43
Nodes (4): Into, Self, String, CanopyError

### Community 18 - "crates canopy-proto build.rs"
Cohesion: 0.40
Nodes (4): main(), Box, Error, Result

### Community 19 - "crates canopy-server src main.rs"
Cohesion: 0.40
Nodes (4): Box, Error, Result, main()

### Community 20 - "crates canopy-server src observability.rs"
Cohesion: 0.40
Nodes (4): Box, Error, Result, init()

### Community 21 - "crates canopy-server src api mod.rs"
Cohesion: 0.83
Nodes (3): to_status(), CanopyError, Status

### Community 22 - "crates canopy-core src signing.rs"
Cohesion: 0.50
Nodes (3): Send, Sync, UrlSigner

### Community 23 - "crates canopy-server src providers adapter.rs"
Cohesion: 0.50
Nodes (3): Send, Sync, ProviderAdapter

## Knowledge Gaps
- **96 isolated node(s):** `Page`, `Vec`, `String`, `Send`, `Sync` (+91 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `demo_assets()` connect `crates canopy-server src lib.rs` to `crates canopy-server src api grpc.rs`?**
  _High betweenness centrality (0.043) - this node is a cross-community bridge._
- **Why does `InMemoryAudioAssetStore` connect `crates canopy-server src lib.rs` to `crates canopy-server src playback.rs`?**
  _High betweenness centrality (0.042) - this node is a cross-community bridge._
- **What connects `Page`, `Vec`, `String` to the rest of the system?**
  _96 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `crates canopy-server src api grpc.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.07291666666666667 - nodes in this community are weakly interconnected._
- **Should `crates canopy-server src playback.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.1226890756302521 - nodes in this community are weakly interconnected._
- **Should `crates canopy-server src jade_store pg.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.12310606060606061 - nodes in this community are weakly interconnected._
- **Should `crates canopy-server src jade_store memory.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.12688172043010754 - nodes in this community are weakly interconnected._