# API Documentation Ownership Design

**Date:** 2026-07-13

**Status:** Approved

## Purpose

Define a durable documentation boundary between the product-neutral
`canopy-api` contract repository, the Canopy backend implementation, and API
consumers. The boundary must let a new consumer use a released contract without
reading server internals while preventing implementation and deployment details
from becoming stale in the contract repository.

## Ownership Principle

Documentation ownership follows the source of truth for each claim.

- `canopy-api` owns released protocol promises.
- Canopy owns claims about its implementation, configuration, deployment, and
  supported contract version.
- Each consumer owns its product-specific integration, storage, UI, release,
  and CI/CD behavior.
- Automation may exchange generated artifacts and immutable version metadata,
  but must not copy mutable prose between repositories.

Centralized discovery is useful. Centralized authorship across independently
owned systems is not.

## `canopy-api` Responsibilities

`canopy-api` is a product-neutral contract and consumer-documentation
repository. Its documentation must not require knowledge of Canopy,
PandaEngine, or any other specific implementation.

It owns:

- canonical protobuf schemas and complete versioned comments;
- service, RPC, message, field, presence, and oneof semantics;
- generic transport requirements and generated SDK consumption;
- authentication metadata and token-lifecycle promises visible on the wire;
- canonical gRPC status behavior, pagination, idempotency, concurrency, and
  retry semantics;
- compatibility, deprecation, and release policy;
- BSR module coordinates, immutable release identifiers, and dependency
  pinning guidance;
- product-neutral executable examples and conformance fixtures;
- generated reference documentation; and
- a changelog describing released contract changes.

The primary handoff document is `docs/consumer-guide.md`. "Consumer" includes
applications, services, CLIs, test harnesses, gateways, and alternative
implementations, so it is more durable than a product-specific or
client-specific filename.

The repository describes the latest stable released contract by default while
keeping older release documentation addressable. It does not claim which
contract version a particular deployment implements.

## Canopy Responsibilities

Canopy owns every claim whose truth depends on the backend implementation or a
running deployment, including:

- the exact `canopy-api` release and generated SDK versions it implements;
- implemented and enabled services or compatibility surfaces;
- listener addresses, public endpoints, TLS, and deployment topology;
- runtime configuration and secret names;
- authentication implementation details such as signing algorithms, password
  hashing, PostgreSQL session rechecks, SMTP delivery, and OIDC provider
  configuration;
- health, readiness, observability, operational failure modes, and
  troubleshooting;
- database, storage, streaming, and internal HTTP behavior;
- Canopy-specific migration and release procedures;
- Swagger hosting for real Canopy HTTP endpoints; and
- Canopy CI/CD.

Canopy may propose ordinary pull requests to `canopy-api` when public protocol
semantics change. Those changes remain contract-owned and are reviewed and
released through the `canopy-api` process. Canopy automation must not write
implementation prose into `canopy-api`.

## Consumer Responsibilities

Each consumer owns:

- its pinned `canopy-api` SDK version;
- channel construction and product-specific dependency injection;
- secure credential storage and local session orchestration;
- UI, bridge, deep-link, and platform integration;
- product-specific retry and offline behavior within contract guarantees;
- compatibility testing against the supported backend version; and
- its own build, release, and CI/CD configuration.

Consumer documentation links to `canopy-api/docs/consumer-guide.md` for
protocol semantics rather than duplicating them.

## OpenAPI And Swagger Boundary

Protobuf and BSR-generated documentation are authoritative for gRPC. Swagger
must not imply that a gRPC RPC is a REST endpoint unless real HTTP transcoding
exists.

`canopy-api` may publish an explicitly noncanonical OpenAPI projection only
when it is generated from the canonical contract and contains product-neutral
protocol information. Generated-projection drift must fail validation.

Canopy owns and may serve Swagger/OpenAPI for actual HTTP endpoints exposed by
the backend. Private stream authorization, health, operational, or other
Canopy-specific HTTP endpoints do not belong in the product-neutral contract
projection. A combined document must not mix invented gRPC HTTP paths with
real implementation endpoints.

## CI/CD And Automation Boundary

`canopy-api` CI/CD is in scope only for its own contract lifecycle:

- format, lint, build, and breaking-change checks;
- protobuf-comment and documentation validation;
- descriptor, SDK, reference, and contract-projection generation;
- generated-artifact drift detection;
- BSR publication and immutable release labels;
- release-note and link validation; and
- contract-level conformance fixtures.

Maintainer-only release instructions belong in `CONTRIBUTING.md` or a clearly
separated maintainer document. Generic registry access requirements may appear
in the consumer guide, but downstream workflow files, provider-specific secret
names, and repository-specific CI examples do not.

Downstream automation may open dependency-update pull requests for new
`canopy-api` releases. It may not synchronize documentation prose. Canopy CI
must verify its pinned contract and conformance tests; consumer CI must verify
its own pinned SDK and integration tests.

## Governance

The boundary is enforced with:

- `CODEOWNERS` and required review for protobuf and contract documentation;
- a concise ownership matrix in `canopy-api` contributor documentation;
- immutable BSR and generated SDK pins in downstream repositories;
- a machine-verifiable declaration in Canopy of the contract version it
  implements, using its exact dependency pins as the primary source;
- breaking-change, generated-drift, and cross-repository link checks; and
- a release process that distinguishes contract publication from backend and
  consumer rollout.

When a statement spans repositories, split it into independently verifiable
claims and link them. For example, `canopy-api` documents the authentication
metadata contract, while Canopy documents how its server validates that
metadata.

## Documentation Layout

The intended `canopy-api` layout is:

```text
README.md
CONTRIBUTING.md
CHANGELOG.md
docs/
  consumer-guide.md
  compatibility.md
proto/
  canopy/v1/canopy.proto
openapi/
  openapi.json              # only if generated and product-neutral
```

Additional focused documents such as `authentication.md`, `errors.md`, or
`pagination.md` are added only when `consumer-guide.md` becomes difficult to
navigate. Protobuf comments remain the field- and method-level source.

## Migration

1. Classify existing documentation statements as contract, implementation,
   consumer, or maintainer owned.
2. Create `docs/consumer-guide.md` and the ownership matrix in `canopy-api`.
3. Expand protobuf comments so generated BSR documentation is independently
   useful.
4. Move Canopy/PandaEngine migration, deployment, feature-status, and CI/CD
   prose to the repository that owns it.
5. Split product-neutral contract projections from Canopy-specific HTTP
   documentation.
6. Correct stale release and implementation-status statements.
7. Add generation, drift, compatibility, and link checks.

Existing links receive redirects or clear replacement links where the hosting
system supports them. Duplicate mutable prose is removed after its canonical
destination is established.

## Acceptance Criteria

- A new consumer can discover, pin, and call the latest stable released API
  without reading Canopy source or product-specific documentation.
- `canopy-api` contains no Canopy or PandaEngine workflow, deployment, runtime,
  or CI/CD claims.
- Canopy identifies and verifies the exact contract release it implements.
- gRPC documentation is authoritative in protobuf/BSR and does not masquerade
  as REST.
- Real Canopy HTTP endpoints are documented by Canopy.
- Contract-derived artifacts are generated or drift-checked.
- Every mutable claim has one authoritative repository owner.
