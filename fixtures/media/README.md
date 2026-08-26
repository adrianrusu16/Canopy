# Media Import Test Fixtures

These files are synthetic test data generated locally with FFmpeg 6.1.1. They
contain a one-second 440 Hz sine wave and solid-color 16x16 artwork. They contain
no private or copyrighted music.

## Regeneration

```bash
ffmpeg -f lavfi -i sine=frequency=440:duration=1 \
  -c:a libmp3lame -q:a 9 \
  -metadata title="Test Tone" \
  -metadata artist="Canopy Tests" \
  -metadata album="Importer Fixtures" \
  fixtures/media/test-tone.mp3

ffmpeg -f lavfi -i color=c=green:s=16x16:d=1 \
  -frames:v 1 -update 1 fixtures/media/cover.jpg

ffmpeg -f lavfi -i color=c=blue:s=16x16:d=1 \
  -frames:v 1 -update 1 fixtures/media/cover.png

ffmpeg -i fixtures/media/test-tone.mp3 -i fixtures/media/cover.jpg \
  -map 0:a -map 1:v -c copy -id3v2_version 3 \
  -metadata:s:v title=Album_cover \
  -metadata:s:v comment=Cover_front \
  fixtures/media/test-tone-embedded.mp3
```

## SHA-256

```text
563fd9cdd9c3e1f724102fa61a945848bed078acc1d0369e5d5f9463679e4802  test-tone.mp3
10a204949bf253274a07e35b25952a3c65ac27c7b2e15cae010a229d0e3a141c  test-tone-embedded.mp3
2c92506da715009ab80b900244aba42b935bf5028a424adbb5f1c4a559cac1b3  cover.jpg
785090597e739d0ec824dc66223d33f26ae4be8da5fc5711f202d5a41dafffb5  cover.png
```

## Personal artwork (local only)

Personal cover art lives under `fixtures/media/personal/artwork/` (gitignored with `personal/`).
Regenerate with `scripts/download-personal-artwork.sh` from Cover Art Archive for local personal use only.
