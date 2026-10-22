#!/usr/bin/env python3
"""Pins the FFmpeg builds Debrief downloads for its video support (ADR 0033).

Writes crates/debrief-video/src/runtime/ffmpeg-runtimes.json: one month-end autobuild of BtbN/FFmpeg-Builds (those are kept
for about two years; daily builds are deleted within weeks) on the release branch given, and per platform its
`lgpl-shared` archive with URL, size and SHA-256. Debrief embeds this file and downloads nothing that is not listed in it;
only LGPL builds are ever pinned (no GPL components such as x264/x265), and Debrief never links or redistributes FFmpeg.
There is no macOS entry: no publisher offers LGPL macOS builds with stable URLs and checksums, so on macOS the user
provides an ffmpeg (for example Homebrew's).

    python3 scripts/update-ffmpeg-runtimes.py                  # the newest month-end autobuild of the pinned branch
    python3 scripts/update-ffmpeg-runtimes.py --tag autobuild-2026-09-30-13-08
    python3 scripts/update-ffmpeg-runtimes.py --branch 9.0     # another release branch
    python3 scripts/update-ffmpeg-runtimes.py --verify         # also download every asset and check the hashes

This is a maintenance script run by a developer; it is not part of Debrief. The checksums come from the GitHub release API
(`digest`); `--verify` downloads each asset and recomputes them. Review the diff: a new pin is a supply-chain change,
and it must pass the runtime self-test on every platform's CI before it is merged.
"""

import argparse
import calendar
import hashlib
import json
import os
import re
import sys
import urllib.request

REPO = "BtbN/FFmpeg-Builds"
API = f"https://api.github.com/repos/{REPO}/releases"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "crates", "debrief-video", "src", "runtime", "ffmpeg-runtimes.json")

# platform -> archive name pattern ({version} is the FFmpeg version, {branch} the release branch such as 9.0).
PLATFORMS = {
    "windows-x64": r"ffmpeg-{version}-win64-lgpl-shared-{branch}\.zip",
    "windows-arm64": r"ffmpeg-{version}-winarm64-lgpl-shared-{branch}\.zip",
    "linux-x64": r"ffmpeg-{version}-linux64-lgpl-shared-{branch}\.tar\.xz",
    "linux-arm64": r"ffmpeg-{version}-linuxarm64-lgpl-shared-{branch}\.tar\.xz",
}

TAG = re.compile(r"^autobuild-(\d{4})-(\d{2})-(\d{2})-\d{2}-\d{2}$")


def get(url):
    req = urllib.request.Request(url, headers={"Accept": "application/vnd.github+json", "User-Agent": "debrief-update-ffmpeg-runtimes"})
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def sha256_of(url):
    h = hashlib.sha256()
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "debrief"}), timeout=1200) as r:
        while True:
            b = r.read(1 << 20)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def month_end(tag):
    """Whether an autobuild tag was made on the last day of its month (those are the ones kept long)."""
    m = TAG.match(tag)
    if not m:
        return False
    y, mo, d = map(int, m.groups())
    return d == calendar.monthrange(y, mo)[1]


def newest_month_end():
    page = 1
    best = None
    while page <= 10:
        rels = get(f"{API}?per_page=100&page={page}")
        if not rels:
            break
        for r in rels:
            if month_end(r["tag_name"]) and (best is None or r["published_at"] > best["published_at"]):
                best = r
        if best:
            break
        page += 1
    if not best:
        sys.exit("no month-end autobuild found")
    return best["tag_name"]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tag", help="an autobuild tag (default: the newest month-end autobuild)")
    ap.add_argument("--branch", default="9.0", help="the FFmpeg release branch the builds follow (default 9.0)")
    ap.add_argument("--verify", action="store_true", help="download every asset and check its SHA-256")
    args = ap.parse_args()

    tag = args.tag or newest_month_end()
    if not TAG.match(tag):
        sys.exit(f"{tag} is not an autobuild tag")
    rel = get(f"{API}/tags/{tag}")
    assets = {a["name"]: a for a in rel["assets"]}

    # The FFmpeg version of the release branch's builds: n9.0.2-17-g2a571b6068 in ffmpeg-n9.0.2-17-g2a571b6068-win64-...
    version = None
    probe = re.compile(rf"^ffmpeg-(n{re.escape(args.branch)}[0-9.]*-[0-9]+-g[0-9a-f]+)-win64-lgpl-shared-{re.escape(args.branch)}\.zip$")
    for name in assets:
        m = probe.match(name)
        if m:
            version = m.group(1)
    if not version:
        sys.exit(f"{tag} has no LGPL shared build of the {args.branch} branch")

    platforms = {}
    for platform, pattern in PLATFORMS.items():
        rx = re.compile("^" + pattern.format(version=re.escape(version), branch=re.escape(args.branch)) + "$")
        hits = sorted(n for n in assets if rx.match(n))
        if not hits:
            print(f"note: {platform} is not published for {tag}", file=sys.stderr)
            continue
        a = assets[hits[-1]]
        digest = (a.get("digest") or "").removeprefix("sha256:")
        if len(digest) != 64:
            sys.exit(f"{a['name']}: the release API gave no SHA-256")
        if args.verify:
            print(f"verifying {a['name']} ...", file=sys.stderr)
            if sha256_of(a["browser_download_url"]) != digest:
                sys.exit(f"{a['name']}: the SHA-256 does not match")
        platforms[platform] = {"name": a["name"], "url": a["browser_download_url"], "size": a["size"], "sha256": digest}

    manifest = {
        "version": 1,
        "source": REPO,
        "tag": tag,
        "release": version,
        "license": "LGPL-2.1-or-later",
        "platforms": platforms,
    }
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")
    print(f"wrote {os.path.normpath(OUT)} ({tag}, {version})")


if __name__ == "__main__":
    main()
