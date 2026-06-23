-- Seed enhanced demo data with genres, playlist, and favorites

-- Insert genres
INSERT INTO genres (id, name) VALUES
    ('55555555-5555-5555-5555-555555555555', 'Classical'),
    ('66666666-6666-6666-6666-666666666666', 'Ambient'),
    ('77777777-7777-7777-7777-777777777777', 'Electronic')
ON CONFLICT DO NOTHING;

-- Link demo track to genre
INSERT INTO track_genres (track_id, genre_id) VALUES
    ('369d3897-8977-430c-b7dd-4a288505a22b', '55555555-5555-5555-5555-555555555555')
ON CONFLICT DO NOTHING;

-- Insert a demo playlist
INSERT INTO playlists (id, title, user_id) VALUES
    ('88888888-8888-8888-8888-888888888888', 'Demo Playlist', '11111111-1111-1111-1111-111111111111')
ON CONFLICT DO NOTHING;

-- Add demo track to playlist
INSERT INTO playlist_tracks (playlist_id, track_id, position) VALUES
    ('88888888-8888-8888-8888-888888888888', '369d3897-8977-430c-b7dd-4a288505a22b', 0)
ON CONFLICT DO NOTHING;

-- Add a favorite
INSERT INTO user_favorites (user_id, track_id) VALUES
    ('11111111-1111-1111-1111-111111111111', '369d3897-8977-430c-b7dd-4a288505a22b')
ON CONFLICT DO NOTHING;

-- Insert provider track record
INSERT INTO provider_tracks (id, track_id, provider, provider_track_id, raw_metadata) VALUES
    ('99999999-9999-9999-9999-999999999999', '369d3897-8977-430c-b7dd-4a288505a22b', 'fixture', 'demo-1', '{"source": "demo"}'::jsonb)
ON CONFLICT DO NOTHING;

-- Insert some playback history
INSERT INTO playback_history (id, user_id, track_id, duration_ms, completion_pct, source) VALUES
    ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa', '11111111-1111-1111-1111-111111111111', '369d3897-8977-430c-b7dd-4a288505a22b', 240000, 0.95, 'discovery')
ON CONFLICT DO NOTHING;
