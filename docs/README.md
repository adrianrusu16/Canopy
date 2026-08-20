# Canopy Documentation

The maintained documentation is organized by what you need to do. Historical
designs and implementation plans live separately and may describe older
behavior.

## Start Here

- [Development](development.md) — prerequisites, local server modes, the
  complete integration environment, and verification commands.
- [Configuration](configuration.md) — every server, identity, SMTP, streaming,
  media, and reserved Compose setting with defaults and validation rules.
- [Deployment](deployment.md) — production components, transport, Nginx
  boundaries, readiness, and operator responsibilities.

## Operate

- [Local Media Administration](media-administration.md) — assign the instance
  owner and import MP3 files and artwork into managed storage.
- [Project Status and Roadmap](roadmap.md) — implemented capabilities, current
  limitations, priorities, and architectural guardrails.

## Integrate

- [API Consumption](api.md) — exact Buf release and generated Rust SDK pins,
  registry setup, upgrade procedure, and verification gates.
- [Client Integration Handoff](client-integration.md) — public endpoints, TLS,
  authentication bootstrap, playback handling, and client-ready checks.

## Understand the System

- [Architecture](architecture.md) — ecosystem, runtime, workspace boundaries,
  service responsibilities, persistence, and health.
- [Authentication](authentication.md) — anonymous access, accounts, sessions,
  email delivery, Google identity, and durable profile state.
- [Playback and Streaming](playback.md) — owner-aware resolution, opaque
  capabilities, privacy semantics, Nginx authorization, and byte delivery.

## Reference

- [HTTP OpenAPI document](openapi.json) — the real Canopy HTTP routes; the
  protobuf contract remains authoritative for gRPC.
- [Environment template](../.env.example) — development placeholders for
  Compose interpolation or explicit shell export.
- [Client connection reference](../deploy/client-connection.example.json) —
  versioned, secret-free local handoff values.

## History

The [documentation archive](archive/README.md) contains completed or
superseded designs and implementation plans. Use it to understand decisions,
not as current setup or operational guidance.
