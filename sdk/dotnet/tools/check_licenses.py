#!/usr/bin/env python3
"""Checks the licences of every NuGet package the .NET SDK, examples and tests resolve (packages.lock.json).

Reads each package's .nuspec from the NuGet cache (run after `dotnet restore`). Allowed: the permissive licences
of the repository's deny.toml. Exits 1 and lists the offenders otherwise. Usage: python3 tools/check_licenses.py
"""

import json
import os
import re
import sys
from pathlib import Path

ALLOWED = {"MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "Unicode-3.0", "Unicode-DFS-2016", "0BSD", "CC0-1.0"}
# Packages that predate licence expressions and only carry a licence URL, with the licence it points to.
KNOWN_URLS = {
    "https://licenses.nuget.org/MIT": "MIT",
    "https://github.com/dotnet/corefx/blob/master/LICENSE.TXT": "MIT",
    "https://github.com/dotnet/runtime/blob/main/LICENSE.TXT": "MIT",
    # K4os LZ4 and xxHash (via Apache.Arrow.Compression): MIT License, Copyright (c) 2017 Milosz Krajewski (checked 2026-10-17).
    "https://github.com/MiloszKrajewski/K4os.Compression.LZ4/blob/master/LICENSE?raw=true": "MIT",
    "https://raw.githubusercontent.com/MiloszKrajewski/K4os.Hash.xxHash/master/LICENSE": "MIT",
}

root = Path(__file__).resolve().parent.parent
cache = Path(os.environ.get("NUGET_PACKAGES", Path.home() / ".nuget" / "packages"))
packages = {}
for lock in root.rglob("packages.lock.json"):
    if "artifacts" in lock.parts or "obj" in lock.parts:
        continue
    for deps in json.loads(lock.read_text())["dependencies"].values():
        for name, info in deps.items():
            if info.get("type") != "Project" and "resolved" in info:
                packages[(name, info["resolved"])] = lock.relative_to(root)

bad = []
for (name, version), where in sorted(packages.items()):
    nuspec = cache / name.lower() / version.lower() / f"{name.lower()}.nuspec"
    if not nuspec.exists():
        bad.append(f"{name} {version}: not in the NuGet cache (restore first)")
        continue
    text = nuspec.read_text(encoding="utf-8", errors="replace")
    m = re.search(r'<license type="expression">([^<]+)</license>', text)
    if m:
        expr = m.group(1).strip()
    else:
        url = re.search(r"<licenseUrl>([^<]+)</licenseUrl>", text)
        expr = KNOWN_URLS.get(url.group(1).strip(), f"unknown ({url.group(1).strip()})") if url else "none"
    terms = [t for t in re.split(r"\s+(?:OR|AND|WITH)\s+|[()]", expr) if t.strip()]
    ok = any(t.strip() in ALLOWED for t in terms) if " OR " in expr else all(t.strip() in ALLOWED for t in terms)
    print(f"{'ok  ' if ok else 'FAIL'} {name} {version}: {expr}")
    if not ok:
        bad.append(f"{name} {version}: {expr} (used by {where})")

if bad:
    print("\nNot allowed:\n  " + "\n  ".join(bad))
    sys.exit(1)
print(f"\n{len(packages)} packages, all permissively licensed")
