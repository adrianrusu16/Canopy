# Documentation Information Architecture Design

## Problem

Canopy's root `README.md` is approximately 44 KB and serves as a project
introduction, architecture specification, development guide, deployment guide,
configuration reference, media-administration manual, API summary,
authentication design, and client-integration guide. This makes the repository
landing page difficult to scan and creates duplicate or contradictory claims
across the root README, `RECOMMENDATIONS.md`, maintained guides, and completed
design or implementation records.

The current `docs/` directory also mixes documentation with different lifecycle
expectations. Maintained integration guidance sits beside completed designs and
implementation plans, while some historical pages still describe implemented
features as future work.

## Goals

- Make the root README a concise entry point for evaluating and starting Canopy.
- Give each maintained documentation page one clear reader task.
- Distinguish current behavior from historical design and implementation records.
- Preserve the technical substance that is currently embedded in the README.
- Remove contradictions where repository evidence establishes the current state.
- Keep the canonical protobuf contract in `canopy-api` instead of duplicating it.
- Preserve the runtime-tested `docs/openapi.json` path.
- Leave unrelated working-tree changes intact.

## Non-Goals

- Changing Rust behavior, protobuf definitions, database migrations, deployment
  manifests, or public endpoints.
- Turning historical plans into maintained product documentation.
- Adding a documentation framework, static-site generator, or new dependency.
- Restating every RPC and protobuf message already owned by `canopy-api`.
- Claiming unimplemented roadmap items as current capability.

## Audience Model

The documentation is organized around five reader intents:

1. **Evaluate:** understand what Canopy is, where it fits, and its main
   capabilities from the root README.
2. **Develop:** start the dependencies and server, use the local integration
   environment, and run verification.
3. **Operate:** configure and deploy Canopy, assess readiness, and administer
   local media.
4. **Integrate:** consume the supported API contract and connect a client to a
   deployed environment.
5. **Understand:** inspect the architecture, authentication model, playback
   policy, project status, and historical decisions.

## Target Structure

```text
README.md
docs/
|-- README.md
|-- architecture.md
|-- development.md
|-- configuration.md
|-- deployment.md
|-- media-administration.md
|-- authentication.md
|-- playback.md
|-- api.md
|-- client-integration.md
|-- roadmap.md
|-- openapi.json
`-- archive/
    |-- README.md
    |-- designs/
    `-- plans/
```

`docs/README.md` is the documentation landing page. It groups maintained pages
by reader intent and explicitly labels `archive/` as historical context rather
than current operational guidance.

## Root README Design

The root README targets approximately 5–8 KB and contains only:

1. A one-paragraph product overview.
2. A compact list of current capabilities.
3. One ecosystem architecture diagram.
4. A minimal quick start with links to detailed prerequisites and workflows.
5. A concise repository structure.
6. Links to the documentation landing page and the most common guides.

Detailed status tables, environment-variable tables, RPC inventories, database
schemas, administrative procedures, and multi-step deployment instructions do
not remain in the root README.

## Maintained Document Responsibilities

### `docs/architecture.md`

Owns the ecosystem and runtime diagrams, naming hierarchy, workspace boundaries,
service responsibilities, persistence model, database overview, and control-plane
versus media-plane distinction. It describes stable architecture rather than
step-by-step setup.

### `docs/development.md`

Owns prerequisites, ordinary local startup, server modes, the complete local
integration environment, and verification commands. The root quick start links
here for details.

### `docs/configuration.md`

Owns the environment-variable reference and cross-field validation rules. Values
are checked against `.env.example` and `crates/canopy-server/src/config.rs`; the
page groups settings by concern instead of presenting one undifferentiated table.

### `docs/deployment.md`

Owns production transport expectations, public and private surfaces, Nginx
authorization boundaries, health and readiness, migration expectations, and
operational cautions. It links to client handoff instead of duplicating client
bootstrap instructions.

### `docs/media-administration.md`

Owns instance-owner assignment, import commands, size limits, metadata and
artwork selection, managed directory layout, staging/finalization behavior,
idempotency, and recovery limitations.

### `docs/authentication.md`

Owns the maintained identity model: anonymous versus authenticated behavior,
password registration and verification, session rotation and revocation, email
delivery, Google linking, durable profile-state authorization, and readiness
effects. Client-specific call sequences remain in `client-integration.md`.

### `docs/playback.md`

Owns owner-aware playback resolution, privacy-preserving error semantics,
capability issuance and revalidation, Nginx byte delivery, stream and storage
boundaries, and playback flow diagrams. It describes owner-aware playback as
implemented and does not restore removed backend player-session controls.

### `docs/api.md`

Replaces `canopy-api-consumption.md`. It owns the exact BSR release and generated
SDK pins supported by this repository, registry setup, the facade rule, the
upgrade procedure, and Canopy verification gates. Product-neutral RPC semantics
remain links to the canonical `canopy-api` consumer guide.

### `docs/client-integration.md`

Remains the focused deployment-to-client handoff. It owns public connection
values, TLS expectations, authentication bootstrap, playback URL handling,
error recovery, and the handoff checklist. Links and small overlaps are updated
to point to the new maintained pages.

### `docs/roadmap.md`

Replaces root `RECOMMENDATIONS.md` and the oversized README status table. It
separates current capabilities from incomplete priorities and removes work that
repository evidence shows is complete, including owner-only personal playback
issuance.

## Historical Records

Completed or superseded design and implementation material moves under
`docs/archive/` without being rewritten as current guidance:

- design records move to `docs/archive/designs/`;
- implementation plans move to `docs/archive/plans/`;
- `docs/archive/README.md` explains that these files may contain obsolete paths,
  versions, statuses, and planned behavior.

The archive preserves decision history but is not included in the primary
getting-started or operations navigation. Existing local modifications within
historical files are preserved when files move.

## Content Reconciliation Rules

When sources disagree, maintained documentation uses this precedence:

1. Current source code, tests, manifests, and checked-in runtime configuration.
2. The canonical `canopy-api` contract and exact dependency pins.
3. Current focused guides.
4. The root README and roadmap.
5. Historical designs and plans.

The restructure specifically resolves these known conflicts:

- backend player/session controls are not documented as a current Canopy
  responsibility because the audited contract moved player control to
  PandaEngine;
- owner-aware personal playback is documented as implemented;
- owner-aware playback is removed from unfinished recommendations;
- inactive RustFS and unwired Redis remain clearly marked as reserved rather
  than active runtime dependencies;
- gRPC API details are linked to `canopy-api` rather than reproduced as an
  error-prone RPC list.

## Link and Compatibility Policy

- `docs/openapi.json` remains at its current path because server routes and tests
  depend on it.
- All relative Markdown links are updated after file moves.
- Code, scripts, tests, manifests, and documentation are searched for references
  to retired paths.
- Links into the external `canopy-api` repository continue to target the
  canonical consumer guide.
- Historical files may link to paths that existed at the time, but archive
  navigation must not present those links as maintained instructions.

## Verification

The completed restructure is accepted when:

- the root README is within the 5–8 KB target and follows the approved section
  order;
- every maintained topic has exactly one clear owning page;
- `docs/README.md` reaches every maintained page and the archive index;
- all relative links in maintained Markdown resolve to existing files or valid
  anchors;
- no maintained page links to the retired `RECOMMENDATIONS.md`,
  `canopy-api-consumption.md`, or owner-aware design paths;
- searches find no current claim that backend playback-session controls remain
  a Canopy responsibility;
- searches find no current claim that owner-aware playback is unimplemented;
- `docs/openapi.json` is unchanged unless a separate factual defect is found;
- documentation-sensitive tests for OpenAPI and client-handoff artifacts still
  pass;
- `git diff --check` reports no whitespace errors; and
- the final diff contains no accidental changes to unrelated source or script
  work already present in the working tree.

