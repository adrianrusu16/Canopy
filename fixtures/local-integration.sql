BEGIN;

DO $$
BEGIN
    IF (
        SELECT COUNT(*)
        FROM provider_tracks
        WHERE provider = 'fixture'
          AND provider_track_id = 'fixture-001'
    ) <> 1 THEN
        RAISE EXCEPTION 'designated local integration track is missing';
    END IF;
END
$$;

UPDATE licenses AS license
SET review_status = 'approved',
    reviewed_at = COALESCE(license.reviewed_at, NOW())
FROM tracks AS track
JOIN provider_tracks AS provider_track ON provider_track.track_id = track.id
WHERE license.id = track.license_id
  AND provider_track.provider = 'fixture'
  AND provider_track.provider_track_id = 'fixture-001';

UPDATE tracks AS track
SET composition_license_id = track.license_id,
    recording_license_id = track.license_id,
    ingest_status = 'ready',
    visibility = 'release_safe'
FROM provider_tracks AS provider_track
WHERE provider_track.track_id = track.id
  AND provider_track.provider = 'fixture'
  AND provider_track.provider_track_id = 'fixture-001';

COMMIT;
