# Documentation Reorganization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Replace Canopy's 44 KB multipurpose root README with a concise repository entry point and a task-oriented maintained documentation set separated from historical records.

**Architecture:** The root README becomes a short evaluation and quick-start page, while docs/README.md routes readers to focused maintained guides. Completed or superseded designs and implementation plans move under docs/archive/; maintained pages are reconciled against current source, tests, manifests, and the canonical canopy-api contract.

**Tech Stack:** GitHub-flavored Markdown, Mermaid, Rust/Cargo verification, PowerShell link validation, Git.

**Spec:** docs/archive/designs/2026-08-19-documentation-information-architecture.md

## Global Constraints

- Do not change Rust behavior, protobuf definitions, database migrations, deployment manifests, or public endpoints.
- Keep docs/openapi.json at its current path and unchanged unless a separately verified defect requires it.
- Preserve the content of modified and untracked historical documents while moving them.
- Do not stage or commit pre-existing changes to crates/canopy-server/tests/local_integration_contract.rs or scripts/local-integration.sh.
- Treat source, tests, manifests, and runtime configuration as more authoritative than README prose or historical plans.
- Keep product-neutral RPC and message semantics in the canonical canopy-api documentation.
- Describe inactive RustFS and unwired Redis as reserved, not active Canopy dependencies.
- Do not document removed backend player-session controls as current behavior.
- Document owner-aware personal playback as implemented.
- Keep the root README between 5 KB and 8 KB.

---

### Task 1: Separate Historical Records

**Files:**

- Create: docs/archive/README.md
- Move to docs/archive/designs/: docs/canopy-api-bsr-design.md
- Move to docs/archive/designs/: docs/owner-aware-playback-resolution.md
- Move to docs/archive/designs/: all eight Markdown files currently in docs/superpowers/specs/
- Move to docs/archive/plans/: docs/canopy-v1-contract-implementation-plan.md
- Move to docs/archive/plans/: docs/owner-aware-playback-implementation-plan.md
- Move to docs/archive/plans/: all nine Markdown files currently in docs/superpowers/plans/
- Preserve: this plan and the documentation information-architecture design already in docs/archive/

**Interfaces:**

- Consumes: the mixed current/historical documentation inventory.
- Produces: designs/ and plans/ as the only historical-record locations.

- [ ] **Step 1: Record the pre-move inventory**

Run:

~~~powershell
Get-ChildItem docs\superpowers\specs,docs\superpowers\plans -File -Filter *.md |
  Sort-Object FullName |
  Select-Object FullName,Length
Get-Item docs\canopy-api-bsr-design.md,docs\owner-aware-playback-resolution.md,docs\canopy-v1-contract-implementation-plan.md,docs\owner-aware-playback-implementation-plan.md |
  Select-Object FullName,Length
~~~

Expected: 8 specs, 9 plans, and four named top-level historical files. Retain the output for size comparison.

- [ ] **Step 2: Move the design records without rewriting them**

Move these exact files, retaining their basenames:

~~~text
docs/canopy-api-bsr-design.md
docs/owner-aware-playback-resolution.md
docs/superpowers/specs/2026-07-03-authentication-design.md
docs/superpowers/specs/2026-07-11-auth-email-delivery-design.md
docs/superpowers/specs/2026-07-13-api-documentation-ownership-design.md
docs/superpowers/specs/2026-07-14-client-integration-handoff-design.md
docs/superpowers/specs/2026-07-14-local-integration-environment-design.md
docs/superpowers/specs/2026-08-01-complete-auth-session-envelope-design.md
docs/superpowers/specs/2026-08-14-for-you-recommendations-design.md
docs/superpowers/specs/2026-08-14-swagger-ui-design.md
~~~

Do not alter the existing modification in the Swagger UI design.

- [ ] **Step 3: Move the implementation records without rewriting them**

Move these exact files, retaining their basenames:

~~~text
docs/canopy-v1-contract-implementation-plan.md
docs/owner-aware-playback-implementation-plan.md
docs/superpowers/plans/2026-07-03-authentication-implementation.md
docs/superpowers/plans/2026-07-09-authentication-completion.md
docs/superpowers/plans/2026-07-11-auth-email-delivery-implementation.md
docs/superpowers/plans/2026-07-13-api-documentation-ownership-implementation.md
docs/superpowers/plans/2026-07-14-client-integration-handoff-implementation.md
docs/superpowers/plans/2026-07-14-local-integration-environment-implementation.md
docs/superpowers/plans/2026-08-01-complete-auth-session-envelope.md
docs/superpowers/plans/2026-08-14-for-you-recommendations-implementation.md
docs/superpowers/plans/2026-08-14-swagger-ui-implementation.md
~~~

The untracked files become organized historical records; preserve their content during the move.

- [ ] **Step 4: Create the archive index**

Write docs/archive/README.md with a historical-content warning, a Designs link, an Implementation Plans link, and a link back to ../README.md. State that archived pages may contain obsolete paths, versions, statuses, or planned behavior and are not maintained operational instructions.

- [ ] **Step 5: Verify preservation**

Run:

~~~powershell
Get-ChildItem docs\archive\designs -File -Filter *.md | Measure-Object
Get-ChildItem docs\archive\plans -File -Filter *.md | Measure-Object
Get-ChildItem docs\superpowers -Recurse -File -ErrorAction SilentlyContinue
~~~

Expected: 11 design records, 12 plan records, no remaining files under docs/superpowers/, and moved sizes matching Step 1.

- [ ] **Step 6: Commit the archive separation**

Stage only docs/archive and the exact retired historical source paths. Confirm with git diff --cached --name-only that no source or script file is staged, then commit:

~~~bash
git commit -m "docs: separate historical design records"
~~~

### Task 2: Create Maintained Architecture and Policy Guides

**Files:**

- Create: docs/architecture.md
- Create: docs/authentication.md
- Create: docs/playback.md
- Create: docs/roadmap.md
- Read: README.md, RECOMMENDATIONS.md, and the relevant current source/tests

**Interfaces:**

- Consumes: conceptual and status material from the old README and recommendations ledger.
- Produces: current system guides that other maintained pages can link to.

- [ ] **Step 1: Write docs/architecture.md**

Use this outline:

~~~text
# Architecture
## Ecosystem
## Runtime Architecture
## Workspace Boundaries
## Service Responsibilities
## Persistence and Storage
## Database Model
## Observability and Health Boundaries
~~~

Carry over the useful Mermaid diagrams, naming hierarchy, Cargo dependency direction, repository-port model, managed storage layout, and database overview. State that gRPC is the control plane, Nginx is the authorized byte-delivery plane, and PostgreSQL owns metadata and policy. Exclude setup commands, configuration tables, and the full RPC inventory.

- [ ] **Step 2: Reconcile architecture claims**

Check crates/canopy-server/src/api/grpc/, jade_store/, stream/, Cargo.toml, and docker-compose.yml. Remove unsupported claims. Do not restore Play, Pause, Seek, SetPlaybackSpeed, Stop, or a Canopy player-session repository.

- [ ] **Step 3: Write docs/authentication.md**

Use this outline:

~~~text
# Authentication
## Access Model
## Password Registration and Verification
## Access and Refresh Sessions
## Email Delivery
## Google Identity
## Durable Profile State
## Readiness and Failure Behavior
## Client Integration
~~~

Preserve current behavior: anonymous browse/search/playback, authenticated durable state, hashed credentials/challenges, Ed25519 access tokens, transactional refresh rotation, session-family revocation on reuse, TLS-only SMTP, encrypted outbox payloads, readiness degradation, explicit Google configuration, and history consent. Link client sequencing to client-integration.md.

- [ ] **Step 4: Write docs/playback.md**

Use this outline:

~~~text
# Playback and Streaming
## Resolution Policy
## Privacy and Error Semantics
## Capability Issuance
## Stream Authorization
## Playback Flow
## Storage Layout
## Client Contract
## Verification Boundaries
~~~

Document the implemented owner-aware policy, invalid-auth and NotFound semantics, opaque capabilities, current-policy revalidation, Nginx range delivery, storage-key secrecy, and diagrams. Do not claim Canopy owns player state.

- [ ] **Step 5: Write docs/roadmap.md**

Use Current Capabilities, Current Limitations, Priorities, and Architectural Guardrails. Merge the useful status/recommendation material, remove owner-aware issuance from unfinished work, retain reconciliation, artwork fallback, observability, operational hardening, evidence-driven search, and evidence-gated JadeCache, and set Last updated: 2026-08-19.

- [ ] **Step 6: Search for stale claims**

Run Select-String across these four pages for:

~~~text
implementation is the next phase
Owner-Only Personal Playback Issuance
Playback Session Controls
session repository
~~~

Expected: no matches. Supabase may appear only as removed.

- [ ] **Step 7: Commit the four maintained guides**

Stage only the four new pages and commit:

~~~bash
git commit -m "docs: add maintained architecture and policy guides"
~~~

### Task 3: Create Development and Operations Guides

**Files:**

- Create: docs/development.md
- Create: docs/configuration.md
- Create: docs/deployment.md
- Create: docs/media-administration.md
- Read: .env.example, config.rs, docker-compose.yml, scripts/local-integration.sh, and deploy/nginx/

**Interfaces:**

- Consumes: setup, configuration, deployment, health, streaming, and administration material from the old README.
- Produces: developer and operator guides.

- [ ] **Step 1: Write docs/development.md**

Use Prerequisites, Quick Start, Run Canopy, Local Integration Environment, Database Integration Tests, Streaming Integration Tests, CI Verification, and SQLx Query Checking. Preserve current commands and distinguish active dependencies from optional/reserved Compose services.

- [ ] **Step 2: Inventory configuration from source**

Extract CANOPY_ lookups from crates/canopy-server/src/config.rs and compare with .env.example. Group them into server/database, identity tokens, authentication email, streaming/media, provider fixtures, and reserved Compose variables.

- [ ] **Step 3: Write docs/configuration.md**

For every setting, record default or required, purpose, and security/validation notes. Explicitly distinguish container-only variables from Canopy runtime settings. Link the environment template rather than duplicating copy-ready secrets.

- [ ] **Step 4: Write docs/deployment.md**

Use Required Components, Public and Private Surfaces, Database Migrations, Production Transport, Nginx Streaming Boundary, Health and Readiness, Authentication Delivery, Client Handoff, and Operational Gaps. Do not present the development Compose file as production topology.

- [ ] **Step 5: Write docs/media-administration.md**

Preserve exact owner/import commands, database/media requirements, 2 GiB and 20 MiB defaults, deterministic non-recursive import, metadata fallback, artwork priority, non-modification of sources, pending/finalization behavior, checksum deduplication, directory layout, and the missing reconciliation tool.

- [ ] **Step 6: Verify commands and setting names**

Run:

~~~bash
cargo run -p canopy-server --features pg --bin canopy-admin -- --help
cargo run -p canopy-server --features pg --bin canopy-admin -- media import --help
~~~

Then compare documented variables against Select-String results from config.rs and .env.example. Expected: command shapes match Clap output and every documented setting has a source.

- [ ] **Step 7: Commit the four operations guides**

Stage only the four pages and commit:

~~~bash
git commit -m "docs: add development and operations guides"
~~~

### Task 4: Rebuild Navigation and Integration Documentation

**Files:**

- Create: docs/README.md
- Move: docs/canopy-api-consumption.md to docs/api.md
- Modify: docs/api.md
- Modify: docs/client-integration.md
- Replace: README.md
- Remove after migration: RECOMMENDATIONS.md

**Interfaces:**

- Consumes: all maintained guides.
- Produces: the repository landing page, documentation hub, API guide, and focused client handoff.

- [ ] **Step 1: Rename and tighten the API guide**

Preserve the exact BSR module/release/commit and generated SDK pins, registry setup, facade rule, upgrade procedure, and verification. Link development.md and the canonical consumer guide. Do not add a local RPC list.

- [ ] **Step 2: Refocus client-integration.md**

Preserve supported pins, public surfaces, connectivity, production transport, password bootstrap, playback handling, error recovery, handoff checklist, and troubleshooting. Replace root README references with configuration.md and deployment.md; link authentication.md and playback.md for deeper policy.

- [ ] **Step 3: Create docs/README.md**

Use Start Here, Operate, Integrate, Understand the System, Reference, and History. Link every maintained page and the archive index with one sentence describing its audience.

- [ ] **Step 4: Replace the root README**

Use this exact order:

~~~text
# Canopy
## Overview
## Capabilities
## Architecture
## Quick Start
## Repository Structure
## Documentation
## Project Status
~~~

Identify Canopy as PandaWave's Rust backend, keep one compact ecosystem diagram, make Quick Start minimal, describe the three workspace crates and infrastructure directories, and link all detail outward.

- [ ] **Step 5: Enforce README size**

Run:

~~~powershell
(Get-Item README.md).Length
~~~

Expected: 5120–8192 bytes. Adjust only the approved sections; do not move reference detail back into README.

- [ ] **Step 6: Retire RECOMMENDATIONS.md**

Confirm all unfinished items and guardrails exist in docs/roadmap.md, then delete the root ledger and update maintained inbound links.

- [ ] **Step 7: Commit navigation and integration pages**

Stage only the listed files and their rename/removal, inspect git diff --cached --name-only, then commit:

~~~bash
git commit -m "docs: rebuild repository documentation navigation"
~~~

### Task 5: Verify the Complete Documentation Set

**Files:**

- Verify: README.md and every top-level Markdown file in docs/
- Verify unchanged: docs/openapi.json
- Verify archive classification: docs/archive/README.md

**Interfaces:**

- Consumes: the reorganized documentation tree.
- Produces: link, consistency, runtime-artifact, and Git-scope evidence.

- [ ] **Step 1: Validate relative file links**

From the repository root, run:

~~~powershell
$ErrorActionPreference = 'Stop'
$files = @(Get-Item README.md) + @(Get-ChildItem docs -File -Filter *.md)
$broken = @()
foreach ($file in $files) {
    $text = Get-Content -Raw -LiteralPath $file.FullName
    foreach ($match in [regex]::Matches($text, '\[[^\]]+\]\((?!https?://|mailto:|#)([^)#]+)(?:#[^)]+)?\)')) {
        $relative = [Uri]::UnescapeDataString($match.Groups[1].Value)
        $target = Join-Path $file.DirectoryName $relative
        if (-not (Test-Path -LiteralPath $target)) {
            $broken += "$($file.FullName): $relative"
        }
    }
}
if ($broken.Count -gt 0) {
    $broken | ForEach-Object { Write-Error $_ }
    exit 1
}
~~~

Expected: exit 0 with no broken relative file targets.

- [ ] **Step 2: Search for retired paths and stale claims**

Search README.md and top-level docs/*.md for:

~~~text
RECOMMENDATIONS.md
canopy-api-consumption.md
owner-aware-playback-resolution.md
implementation is the next phase
Playback Session Controls
Owner-Only Personal Playback Issuance
~~~

Expected: no matches. Archive files are intentionally excluded.

- [ ] **Step 3: Confirm OpenAPI stability**

Run:

~~~bash
test -f docs/openapi.json
git diff --exit-code -- docs/openapi.json
~~~

Expected: both commands succeed.

- [ ] **Step 4: Run documentation-sensitive tests**

Run:

~~~bash
cargo test -p canopy-server --test http_openapi --test client_handoff --locked
~~~

Expected: both integration-test binaries pass.

- [ ] **Step 5: Check formatting and Git scope**

Run:

~~~bash
git diff --check
git status --short
git diff --stat HEAD~4..HEAD
~~~

Expected: no whitespace errors; documentation commits contain only the approved reorganization; the pre-existing source/script changes remain outside those commits.

- [ ] **Step 6: Review rendered navigation**

Preview README.md and docs/README.md using GitHub-compatible rendering. Confirm the Mermaid diagram, headings, code fences, and one-click routing for each reader intent.

- [ ] **Step 7: Commit verification corrections only if needed**

If verification required documentation corrections, stage only those files and commit:

~~~bash
git commit -m "docs: fix documentation navigation checks"
~~~

Do not create an empty commit.

