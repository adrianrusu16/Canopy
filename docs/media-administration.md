# Local Media Administration

canopy-admin assigns the single instance owner and imports local MP3 files into
Canopy-managed storage. It is deliberately stricter than the server: it
requires PostgreSQL and never falls back to in-memory state.

## Prerequisites

Apply the full migration chain and create the user's account/profile before
assigning it as owner. Export:

```bash
export CANOPY_DATABASE_URL='postgres://canopy:canopy@localhost:5432/canopy'
export CANOPY_MEDIA_ROOT='/srv/canopy/media'
```

The binary is gated behind the pg feature:

```bash
cargo run -p canopy-server --features pg --bin canopy-admin -- --help
```

Run it with credentials that can read profiles and update media, ownership, and
instance settings. Do not expose this command as a public service.

## Assign the Instance Owner

```bash
cargo run -p canopy-server --features pg --bin canopy-admin -- \
  owner set <external-user-id>
```

The external user ID must already resolve to a profile. Canopy stores that
profile in the singleton instance settings row. The configured owner is the
only identity eligible for owner-scoped personal playback.

Changing the owner immediately affects stream authorization: personal
capabilities issued for the previous owner fail their next policy recheck.

## Import Media

Import one MP3 or the top-level MP3 files in one directory:

```bash
cargo run -p canopy-server --features pg --bin canopy-admin -- \
  media import /path/to/track-or-directory
```

Directory traversal is deliberately non-recursive, case-insensitive for the
.mp3 extension, sorted, and deterministic. A mixed directory reports per-file
successes and failures without rolling back successful independent imports.

Optional limits:

| Variable | Default |
| --- | --- |
| CANOPY_MAX_AUDIO_BYTES | 2147483648 bytes (2 GiB) |
| CANOPY_MAX_ARTWORK_BYTES | 20971520 bytes (20 MiB) |

Both must be positive integers.

## File and Artwork Selection

Track metadata comes from MP3 tags with conservative fallbacks when tags are
missing. Artwork selection order is:

1. embedded cover art;
2. cover.jpg beside the MP3;
3. cover.png beside the MP3; or
4. no artwork.

Missing artwork is valid. Oversized or malformed MP3/artwork data is rejected.
Source files are never modified or deleted, and their absolute paths are not
persisted or returned in JSON output. Symlink sources are rejected.

## Import Lifecycle

For a new track, the importer:

1. validates the configured owner and source;
2. inspects metadata, duration, artwork, size, and audio checksum;
3. stages content beneath staging/{track-id};
4. inserts PostgreSQL rows as personal, pending, and local_admin;
5. finalizes audio to library/audio/{sha-prefix}/{sha}.mp3;
6. finalizes optional artwork to
   library/artwork/{sha-prefix}/{sha}.{ext}; and
7. marks the database row ready.

Pending imports are invisible to catalog and playback policy. Only the matching
configured owner can resolve a personal-ready asset.

## Idempotency and Failure Recovery

Audio checksum uniqueness makes retrying the same content idempotent. An
existing checksum returns the winning track ID rather than creating a second
track. A concurrent duplicate discards its staging data and reports the
existing result.

Failure behavior is fail closed:

- failure before persistence discards staging;
- insert failure discards staging;
- finalization failure keeps the pending row and recoverable staging state;
- failure to mark ready may leave finalized files with a pending row; and
- conflicting content at a final destination is rejected.

Canopy does not yet ship a reconciliation command. Inspect pending rows and
managed files carefully; do not manually delete staging or final content
without a checksum-aware recovery procedure.

## Managed Directory Layout

```text
/srv/canopy/media/
|-- staging/
|-- library/
|   |-- audio/
|   `-- artwork/
|-- originals/
`-- quarantine/
```

Storage keys are validated relative paths within the expected media class.
Nginx receives a read-only mount of library/ and serves files only after Canopy
authorizes the current capability.

## Current Limitations

- No administrative reconciliation or repair command.
- No recursive directory import.
- MP3 is the only accepted source format.
- Artwork delivery and universal fallback remain incomplete client-facing work.
- Local import is an owner-only administrative path, not a general upload API.
- RustFS is not an import destination or active playback store.

For playback policy, see [Playback and Streaming](playback.md). For operational
requirements, see [Deployment](deployment.md).
