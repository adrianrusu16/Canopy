-- Seed demo data matching the in-memory prototype catalog.
-- This inserts a single demo track so the PgCatalogRepository can serve
-- the same data as InMemoryCatalog::demo_catalog() once wired up.

INSERT INTO users (id) VALUES
    ('11111111-1111-1111-1111-111111111111');

INSERT INTO licenses (id, license_type, source_url, attribution_text) VALUES
    ('22222222-2222-2222-2222-222222222222', 'CC0', 'https://musopen.org', 'Public Domain');

INSERT INTO artists (id, name, sort_name) VALUES
    ('33333333-3333-3333-3333-333333333333', 'Demo Artist', 'Demo Artist');

INSERT INTO albums (id, title, artist_id, release_year, artwork_key) VALUES
    ('44444444-4444-4444-4444-444444444444', 'Demo Album', '33333333-3333-3333-3333-333333333333', 2024, 'artwork/albums/demo-album.png');

INSERT INTO tracks (id, title, artist_id, album_id, duration_ms, license_id, is_explicit, artwork_key) VALUES
    ('demo-1', 'Demo Track', '33333333-3333-3333-3333-333333333333', '44444444-4444-4444-4444-444444444444', 240000, '22222222-2222-2222-2222-222222222222', false, 'artwork/tracks/demo-1.png');

INSERT INTO audio_assets (track_id, codec, content_type, object_key, size_bytes, checksum_sha256, duration_ms) VALUES
    ('demo-1', 'mp4', 'audio/mp4', 'audio/tracks/demo-1.m4a', 9600000, '0000000000000000000000000000000000000000000000000000000000000000', 240000);
