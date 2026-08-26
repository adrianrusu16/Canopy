#!/usr/bin/env python3
from __future__ import annotations
import json, shutil, subprocess, sys, time, urllib.parse, urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ALBUMS = ROOT / "fixtures/media/personal/artwork/albums"
TRACKS = ROOT / "fixtures/media/personal/artwork/tracks"
UA = "CanopyPersonalLibrary/1.0 ( local personal use )"
MB_SLEEP = 1.1
SUCCESS = 0
FAIL = 0
NOTES = []
FAILURES = []
SUCCESSES = []

def http_get(url, dest=None):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            data = resp.read()
            code = getattr(resp, "status", 200) or 200
            if dest is not None and code == 200 and data:
                dest.write_bytes(data)
            return code, data
    except Exception as e:
        print(f"  HTTP error for {url}: {e}")
        return 0, b""

def mb_get(url):
    time.sleep(MB_SLEEP)
    code, data = http_get(url)
    if code != 200 or not data:
        return {}
    try:
        return json.loads(data.decode("utf-8"))
    except json.JSONDecodeError:
        return {}

def pick_rg_mbid(payload, prefer):
    rgs = payload.get("release-groups") or []
    if not rgs:
        return None
    wanted = "Album" if prefer == "album" else "Single"
    for rg in rgs:
        if rg.get("primary-type") == wanted:
            return rg.get("id")
    return rgs[0].get("id")

def is_image(path):
    try:
        out = subprocess.check_output(["file", "-b", "--mime-type", str(path)], text=True).strip()
        return out.startswith("image/")
    except Exception:
        return path.stat().st_size > 1000

def caa_download(mbid, outfile, *, entity="release-group"):
    tmp = outfile.with_suffix(".tmp")
    code, _ = http_get(f"https://coverartarchive.org/{entity}/{mbid}/front-500", tmp)
    if code == 200 and tmp.exists() and tmp.stat().st_size > 0 and is_image(tmp):
        tmp.replace(outfile)
        return True
    if tmp.exists():
        tmp.unlink()
    return False

def search_and_download(slug, dest_dir, query, prefer, label, alt_query=None):
    global SUCCESS, FAIL
    outfile = dest_dir / f"{slug}.jpg"
    url = "https://musicbrainz.org/ws/2/release-group/?query=" + urllib.parse.quote(query) + "&fmt=json"
    payload = mb_get(url)
    mbid = pick_rg_mbid(payload, prefer)
    if not mbid and alt_query:
        print(f"  retry alt query: {alt_query}")
        url = "https://musicbrainz.org/ws/2/release-group/?query=" + urllib.parse.quote(alt_query) + "&fmt=json"
        payload = mb_get(url)
        mbid = pick_rg_mbid(payload, prefer)
    if not mbid:
        FAIL += 1
        FAILURES.append(f"{label}: no release-group found for query='{query}'")
        print(f"FAIL {label} - no RG for: {query}")
        return False
    if caa_download(mbid, outfile):
        SUCCESS += 1
        SUCCESSES.append(f"{outfile} (RG {mbid})")
        print(f"OK  {label} -> {outfile} (RG {mbid})")
        return True
    if alt_query:
        print(f"  CAA miss; trying alt query: {alt_query}")
        url = "https://musicbrainz.org/ws/2/release-group/?query=" + urllib.parse.quote(alt_query) + "&fmt=json"
        payload = mb_get(url)
        mbid2 = pick_rg_mbid(payload, prefer)
        if mbid2 and caa_download(mbid2, outfile):
            SUCCESS += 1
            SUCCESSES.append(f"{outfile} (alt RG {mbid2})")
            print(f"OK  {label} -> {outfile} (alt RG {mbid2})")
            return True
    FAIL += 1
    FAILURES.append(f"{label}: CAA front-500 failed for RG {mbid}")
    print(f"FAIL {label} - CAA failed for RG {mbid}")
    return False

def fetch_album(slug, artist, title, known_rg=None, alt_title=None):
    global SUCCESS
    label = f"album:{slug}"
    outfile = ALBUMS / f"{slug}.jpg"
    if known_rg and caa_download(known_rg, outfile):
        SUCCESS += 1
        SUCCESSES.append(f"{outfile} (known RG {known_rg})")
        print(f"OK  {label} -> {outfile} (RG {known_rg})")
        return
    if known_rg:
        print(f"WARN CAA failed for known RG {known_rg} ({label}); will try search")
    q = f'artist:"{artist}" AND releasegroup:"{title}" AND primarytype:Album'
    alt = f'artist:"{artist}" AND releasegroup:"{alt_title}" AND primarytype:Album' if alt_title else f"artist:{artist} AND releasegroup:{title}"
    search_and_download(slug, ALBUMS, q, "album", label, alt)

def fetch_single(slug, artist, title, fallback_album=None, alt_title=None, known_rg=None, known_release=None):
    global SUCCESS, FAIL
    label = f"track:{slug}"
    outfile = TRACKS / f"{slug}.jpg"
    if known_release and caa_download(known_release, outfile, entity="release"):
        SUCCESS += 1
        SUCCESSES.append(f"{outfile} (known release {known_release})")
        print(f"OK  {label} -> {outfile} (release {known_release})")
        return
    if known_release:
        print(f"WARN CAA failed for known release {known_release} ({label}); will try RG/search")
    if known_rg and caa_download(known_rg, outfile):
        SUCCESS += 1
        SUCCESSES.append(f"{outfile} (known RG {known_rg})")
        print(f"OK  {label} -> {outfile} (RG {known_rg})")
        return
    if known_rg:
        print(f"WARN CAA failed for known RG {known_rg} ({label}); will try search")
    q = f'artist:"{artist}" AND releasegroup:"{title}" AND primarytype:Single'
    alt = f'artist:"{artist}" AND releasegroup:"{alt_title}" AND primarytype:Single' if alt_title else f'artist:"{artist}" AND releasegroup:"{title}"'
    if search_and_download(slug, TRACKS, q, "single", label, alt):
        return
    if fallback_album:
        src = ALBUMS / f"{fallback_album}.jpg"
        if src.exists():
            dst = TRACKS / f"{slug}.jpg"
            shutil.copyfile(src, dst)
            SUCCESS += 1
            FAIL = max(0, FAIL - 1)
            FAILURES[:] = [f for f in FAILURES if not f.startswith(label)]
            SUCCESSES.append(f"{dst} (fallback album {fallback_album})")
            NOTES.append(f"{slug}: used album fallback {fallback_album}.jpg")
            print(f"OK  {label} -> fallback from album {fallback_album}.jpg")

def copy_track(track_slug, album_slug):
    global SUCCESS, FAIL
    src = ALBUMS / f"{album_slug}.jpg"
    dst = TRACKS / f"{track_slug}.jpg"
    if src.exists():
        shutil.copyfile(src, dst)
        SUCCESS += 1
        SUCCESSES.append(f"{dst} (copy of album {album_slug})")
        print(f"OK  track:{track_slug} <- album:{album_slug}")
    else:
        FAIL += 1
        FAILURES.append(f"track:{track_slug}: missing album art {album_slug}.jpg")
        print(f"FAIL track:{track_slug} - missing {album_slug}.jpg")

def main():
    global SUCCESS, FAIL
    ALBUMS.mkdir(parents=True, exist_ok=True)
    TRACKS.mkdir(parents=True, exist_ok=True)
    print("=== Album release-group covers ===")
    fetch_album("hybrid-theory", "Linkin Park", "Hybrid Theory", "b5b4bb4b-8ba5-3acf-88cb-4cae2699d8da")
    fetch_album("fallen", "Evanescence", "Fallen")
    fetch_album("korn", "Korn", "Korn")
    fetch_album("take-a-look-in-the-mirror", "Korn", "Take a Look in the Mirror")
    fetch_album("follow-the-leader", "Korn", "Follow the Leader")
    fetch_album("see-you-on-the-other-side", "Korn", "See You on the Other Side")
    fetch_album("mayhem", "Lady Gaga", "MAYHEM", alt_title="Mayhem")
    fetch_album("meteora-20", "Linkin Park", "Meteora (20th anniversary edition)", alt_title="Meteora 20")
    fetch_album("rosie", "ROSÉ", "rosie", alt_title="Rosie")
    fetch_album("the-gray-chapter", "Slipknot", ".5: The Gray Chapter", alt_title="The Gray Chapter")
    fetch_album("vol-3-the-subliminal-verses", "Slipknot", "Vol. 3: (The Subliminal Verses)", alt_title="Vol. 3: The Subliminal Verses")
    fetch_album("slipknot", "Slipknot", "Slipknot")
    fetch_album("iowa", "Slipknot", "Iowa", "082c68eb-d993-36cf-9b32-6663cba2d052")
    fetch_album("all-hope-is-gone", "Slipknot", "All Hope Is Gone")
    fetch_album("we-are-not-your-kind", "Slipknot", "We Are Not Your Kind")
    fetch_album("cheese", "Stromae", "Cheese")
    fetch_album("toxicity", "System of a Down", "Toxicity", "f50fbcb4-bfcd-3784-b4c9-44f4793e66b2")
    fetch_album("mezmerize", "System of a Down", "Mezmerize")
    fetch_album("hypnotize", "System of a Down", "Hypnotize")
    fetch_album("from-zero", "Linkin Park", "From Zero")
    print("")
    print("=== Special single / splash track art ===")
    fetch_single("linkin-park-lost", "Linkin Park", "Lost", known_rg="d81eeeb3-3f08-43ca-aede-e59601417836", known_release="9adfd6e0-54e4-4bc4-932d-6c84ebccc680")
    fetch_single("linkin-park-the-emptiness-machine", "Linkin Park", "The Emptiness Machine")
    fetch_single("rose-bruno-mars-apt", "ROSÉ", "APT.", alt_title="APT")
    fetch_single("lady-gaga-bruno-mars-die-with-a-smile", "Lady Gaga", "Die with a Smile", alt_title="Die With A Smile")
    fetch_single("slipknot-the-negative-one", "Slipknot", "The Negative One")
    fetch_single("slipknot-unsainted", "Slipknot", "Unsainted")
    fetch_single("slipknot-solway-firth", "Slipknot", "Solway Firth")
    fetch_single("slipknot-killpop", "Slipknot", "Killpop", fallback_album="the-gray-chapter")
    fetch_single("korn-did-my-time", "Korn", "Did My Time", fallback_album="take-a-look-in-the-mirror")
    print("Searching for eric-cartman-bring-me-to-life...")
    ok = search_and_download(
        "eric-cartman-bring-me-to-life",
        TRACKS,
        'artist:"Eric Cartman" AND releasegroup:"Bring Me to Life"',
        "single",
        "track:eric-cartman-bring-me-to-life",
        'artist:"Cartman" AND bring me to life',
    )
    if not ok:
        fallen = ALBUMS / "fallen.jpg"
        if fallen.exists():
            dst = TRACKS / "eric-cartman-bring-me-to-life.jpg"
            shutil.copyfile(fallen, dst)
            FAIL = max(0, FAIL - 1)
            FAILURES[:] = [f for f in FAILURES if not f.startswith("track:eric-cartman-bring-me-to-life")]
            SUCCESS += 1
            SUCCESSES.append(f"{dst} (fallback fallen.jpg)")
            NOTES.append("eric-cartman-bring-me-to-life: no good MB hit; copied fallen.jpg as fallback")
            print("OK  track:eric-cartman-bring-me-to-life -> fallback fallen.jpg")
    print("")
    print("=== Copy album art onto tracks without special singles ===")
    pairs = [
        ("linkin-park-a-place-for-my-head", "hybrid-theory"),
        ("linkin-park-points-of-authority", "hybrid-theory"),
        ("evanescence-going-under", "fallen"),
        ("korn-blind", "korn"),
        ("korn-freak-on-a-leash", "follow-the-leader"),
        ("korn-liar", "see-you-on-the-other-side"),
        ("slipknot-custer", "the-gray-chapter"),
        ("slipknot-duality", "vol-3-the-subliminal-verses"),
        ("slipknot-the-blister-exists", "vol-3-the-subliminal-verses"),
        ("slipknot-vermilion-pt-2", "vol-3-the-subliminal-verses"),
        ("slipknot-eyeless", "slipknot"),
        ("slipknot-people-equals-shit", "iowa"),
        ("slipknot-psychosocial", "all-hope-is-gone"),
        ("slipknot-snuff", "all-hope-is-gone"),
        ("stromae-alors-on-danse", "cheese"),
        ("system-of-a-down-aerials", "toxicity"),
        ("system-of-a-down-forest", "toxicity"),
        ("system-of-a-down-toxicity", "toxicity"),
        ("system-of-a-down-cigaro", "mezmerize"),
        ("system-of-a-down-lonely-day", "hypnotize"),
        ("system-of-a-down-vicinity-of-obscenity", "hypnotize"),
    ]
    for track, album in pairs:
        copy_track(track, album)
    print("")
    print("=== SUMMARY ===")
    print(f"Success: {SUCCESS}")
    print(f"Fail:    {FAIL}")
    print("")
    print("--- albums ---")
    subprocess.run(["ls", "-la", str(ALBUMS)], check=False)
    print("")
    print("--- tracks ---")
    subprocess.run(["ls", "-la", str(TRACKS)], check=False)
    if SUCCESSES:
        print("")
        print("Successful files:")
        for s in SUCCESSES:
            print(f"  {s}")
    if FAILURES:
        print("")
        print("Failures:")
        for f in FAILURES:
            print(f"  {f}")
    if NOTES:
        print("")
        print("Notes:")
        for n in NOTES:
            print(f"  {n}")
    print("")
    print(f"Album JPGs: {len(list(ALBUMS.glob('*.jpg')))}")
    print(f"Track JPGs: {len(list(TRACKS.glob('*.jpg')))}")
    report = ROOT / "fixtures/media/personal/artwork/_download_report.txt"
    lines = [f"Success: {SUCCESS}", f"Fail: {FAIL}", "SUCCESS:", *SUCCESSES, "FAILURES:", *FAILURES, "NOTES:", *NOTES]
    report.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return 0 if FAIL == 0 else 1

if __name__ == "__main__":
    sys.exit(main())
