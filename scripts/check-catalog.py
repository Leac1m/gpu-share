#!/usr/bin/env python3
"""Checks every entry in the model catalog against Hugging Face before it is published
(roadmap P6-T1): the file exists at its pinned URL, and its size and SHA-256 are the ones
Hugging Face reports (the X-Linked-Size / X-Linked-Etag headers on the download redirect).

    scripts/check-catalog.py [crates/host/catalog.json]
"""
import json
import sys
import urllib.error
import urllib.request

path = sys.argv[1] if len(sys.argv) > 1 else "crates/host/catalog.json"
entries = json.load(open(path))


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


opener = urllib.request.build_opener(NoRedirect)
ids = [e["id"] for e in entries]
problems = [f"duplicate id {i}" for i in set(ids) if ids.count(i) > 1]
for e in entries:
    try:
        r = opener.open(urllib.request.Request(e["file_url"], method="HEAD"), timeout=30)
        headers = r.headers
    except urllib.error.HTTPError as err:  # the redirect itself carries the headers
        headers = err.headers
        if err.code not in (301, 302, 303, 307, 308):
            problems.append(f"{e['id']}: HTTP {err.code} for {e['file_url']}")
            continue
    size = headers.get("X-Linked-Size")
    sha = (headers.get("X-Linked-Etag") or "").strip('"')
    if size is None or not sha:
        problems.append(f"{e['id']}: Hugging Face gave no size or checksum (not an LFS file?)")
        continue
    if int(size) != e["size_bytes"]:
        problems.append(f"{e['id']}: size {e['size_bytes']} in the catalog, {size} on Hugging Face")
    if sha.lower() != e["sha256"].lower():
        problems.append(f"{e['id']}: SHA-256 in the catalog doesn't match Hugging Face's {sha}")
    print(f"ok  {e['id']}" if not any(p.startswith(e["id"] + ":") for p in problems) else f"BAD {e['id']}")

for p in problems:
    print("problem:", p, file=sys.stderr)
sys.exit(1 if problems else 0)
