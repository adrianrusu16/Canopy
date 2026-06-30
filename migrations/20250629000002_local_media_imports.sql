-- Durable provenance and checksum guarantees for local administrative imports.

ALTER TABLE tracks
    ADD COLUMN ingest_source TEXT NOT NULL DEFAULT 'legacy_provider',
    ADD CONSTRAINT chk_tracks_ingest_source
        CHECK (ingest_source IN ('legacy_provider', 'local_admin'));

ALTER TABLE audio_assets
    ADD CONSTRAINT chk_audio_assets_checksum_hex
        CHECK (checksum_sha256 ~ '^[0-9a-fA-F]{64}$');

DO $$
BEGIN
    IF EXISTS (
        SELECT LOWER(checksum_sha256)
        FROM audio_assets
        GROUP BY LOWER(checksum_sha256)
        HAVING COUNT(*) > 1
    ) THEN
        RAISE EXCEPTION
            'duplicate audio checksums prevent local import uniqueness';
    END IF;
END;
$$;

CREATE UNIQUE INDEX uq_audio_assets_checksum_sha256
    ON audio_assets(LOWER(checksum_sha256));

CREATE INDEX idx_tracks_local_import_pending
    ON tracks(created_at, id)
    WHERE ingest_source = 'local_admin' AND ingest_status = 'pending';

COMMENT ON COLUMN tracks.ingest_source IS
    'Import provenance: legacy_provider compatibility or local_admin managed import.';
