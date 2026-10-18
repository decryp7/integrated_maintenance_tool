#!/usr/bin/env python3
"""Pins the llama.cpp release IMT downloads its inference runtimes from (ADR 0009).

Writes crates/imt-ai/src/runtimes/llama-runtimes.json: the release tag and, per platform and build,
the official prebuilt assets with their download URL, size and SHA-256. IMT embeds this file and
downloads nothing that is not listed in it, and verifies every byte against these checksums.

    python3 scripts/update-llama-runtimes.py                 # the release GitHub marks as latest
    python3 scripts/update-llama-runtimes.py --tag b11669    # a specific build tag
    python3 scripts/update-llama-runtimes.py --verify        # also download every asset and check the hashes

The checksums come from the GitHub release API (`digest`, computed by GitHub when the asset was
uploaded). `--verify` downloads each asset and recomputes them; run it before committing a new pin.
Review the diff: a new tag is a supply-chain change.
"""

import argparse
import hashlib
import json
import os
import re
import sys
import urllib.request

REPO = "ggml-org/llama.cpp"
API = f"https://api.github.com/repos/{REPO}/releases"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "crates", "imt-ai", "src", "runtimes", "llama-runtimes.json")

# platform -> build id -> asset name patterns ({tag} is the build tag). The first pattern is the
# llama-server archive; the others are runtimes unpacked into the same folder (CUDA's cudart).
BUILDS = {
    "windows-x64": {
        "cpu": [r"llama-{tag}-bin-win-cpu-x64\.zip"],
        "vulkan": [r"llama-{tag}-bin-win-vulkan-x64\.zip"],
        "cuda-12": [r"llama-{tag}-bin-win-cuda-12\.[0-9]+-x64\.zip", r"cudart-llama-bin-win-cuda-12\.[0-9]+-x64\.zip"],
        "cuda-13": [r"llama-{tag}-bin-win-cuda-13\.[0-9]+-x64\.zip", r"cudart-llama-bin-win-cuda-13\.[0-9]+-x64\.zip"],
        "rocm": [r"llama-{tag}-bin-win-rocm-[0-9.]+-x64\.zip"],
        "sycl": [r"llama-{tag}-bin-win-sycl-x64\.zip"],
        "openvino": [r"llama-{tag}-bin-win-openvino-[0-9.]+-x64\.zip"],
    },
    "windows-arm64": {
        "cpu": [r"llama-{tag}-bin-win-cpu-arm64\.zip"],
        "vulkan": [r"llama-{tag}-bin-win-vulkan-arm64\.zip"],
        "cuda-13": [r"llama-{tag}-bin-win-cuda-13\.[0-9]+-arm64\.zip", r"cudart-llama-bin-win-cuda-13\.[0-9]+-arm64\.zip"],
    },
    "linux-x64": {
        "cpu": [r"llama-{tag}-bin-ubuntu-x64\.tar\.gz"],
        "vulkan": [r"llama-{tag}-bin-ubuntu-vulkan-x64\.tar\.gz"],
        "cuda-12": [r"llama-{tag}-bin-ubuntu-cuda-12\.[0-9]+-x64\.tar\.gz", r"cudart-llama-{tag}-bin-ubuntu-cuda-12\.[0-9]+-x64\.tar\.gz"],
        "cuda-13": [r"llama-{tag}-bin-ubuntu-cuda-13\.[0-9]+-x64\.tar\.gz", r"cudart-llama-{tag}-bin-ubuntu-cuda-13\.[0-9]+-x64\.tar\.gz"],
        "rocm": [r"llama-{tag}-bin-ubuntu-rocm-[0-9.]+-x64\.tar\.gz"],
        "sycl": [r"llama-{tag}-bin-ubuntu-sycl-fp16-x64\.tar\.gz"],
        "sycl-fp32": [r"llama-{tag}-bin-ubuntu-sycl-fp32-x64\.tar\.gz"],
        "openvino": [r"llama-{tag}-bin-ubuntu-openvino-[0-9.]+-x64\.tar\.gz"],
    },
    "linux-arm64": {
        "cpu": [r"llama-{tag}-bin-ubuntu-arm64\.tar\.gz"],
        "vulkan": [r"llama-{tag}-bin-ubuntu-vulkan-arm64\.tar\.gz"],
        "cuda-13": [r"llama-{tag}-bin-ubuntu-cuda-13\.[0-9]+-arm64\.tar\.gz", r"cudart-llama-{tag}-bin-ubuntu-cuda-13\.[0-9]+-arm64\.tar\.gz"],
    },
    "macos-arm64": {"metal": [r"llama-{tag}-bin-macos-arm64\.tar\.gz"]},
    "macos-x64": {"cpu": [r"llama-{tag}-bin-macos-x64\.tar\.gz"]},
}


def get(url):
    req = urllib.request.Request(url, headers={"Accept": "application/vnd.github+json", "User-Agent": "imt-update-llama-runtimes"})
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def sha256_of(url):
    h = hashlib.sha256()
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "imt"}), timeout=600) as r:
        while True:
            b = r.read(1 << 20)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tag", help="a build tag such as b11669 (default: the latest release)")
    ap.add_argument("--verify", action="store_true", help="download every asset and check its SHA-256")
    args = ap.parse_args()

    release_name = None
    tag = args.tag
    if not tag:
        latest = get(f"{API}/latest")
        release_name = latest["tag_name"]
        # Stable releases (v0.x.y) point at a build tag in nightly-tag.txt; their assets live there.
        names = {a["name"]: a for a in latest.get("assets", [])}
        if "nightly-tag.txt" in names:
            with urllib.request.urlopen(names["nightly-tag.txt"]["browser_download_url"], timeout=60) as r:
                tag = r.read().decode().strip()
        else:
            tag = latest["tag_name"]
    rel = get(f"{API}/tags/{tag}")
    assets = {a["name"]: a for a in rel["assets"]}

    platforms = {}
    for platform, builds in BUILDS.items():
        out = []
        for build, patterns in builds.items():
            files = []
            for pat in patterns:
                rx = re.compile("^" + pat.replace("{tag}", re.escape(tag)) + "$")
                hits = sorted(n for n in assets if rx.match(n))
                if not hits:
                    files = None
                    break
                a = assets[hits[-1]]
                digest = (a.get("digest") or "").removeprefix("sha256:")
                if len(digest) != 64:
                    sys.exit(f"{a['name']}: the release API gave no SHA-256")
                if args.verify:
                    print(f"verifying {a['name']} ...", file=sys.stderr)
                    if sha256_of(a["browser_download_url"]) != digest:
                        sys.exit(f"{a['name']}: the SHA-256 does not match")
                files.append({"name": a["name"], "url": a["browser_download_url"], "size": a["size"], "sha256": digest})
            if files is None:
                print(f"note: {platform}/{build} is not published for {tag}", file=sys.stderr)
                continue
            out.append({"id": build, "assets": files})
        platforms[platform] = out

    manifest = {
        "version": 1,
        "release": release_name or tag,
        "tag": tag,
        "commit": rel.get("target_commitish", ""),
        "published": (rel.get("published_at") or "")[:10],
        "source": rel.get("html_url", f"https://github.com/{REPO}/releases/tag/{tag}"),
        "platforms": platforms,
    }
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")
    print(f"wrote {os.path.normpath(OUT)} ({tag})")


if __name__ == "__main__":
    main()
