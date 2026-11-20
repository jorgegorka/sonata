#!/usr/bin/env python3
"""Record a Pages artifact's revision and verify its publicly served contents."""
import argparse
import hashlib
import http.client
import json
from pathlib import Path
import re
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = "deployment.json"


def digest(stream):
    checksum = hashlib.sha256()
    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
        checksum.update(chunk)
    return checksum.hexdigest()


def record(directory, revision):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("Expected the full checked-out commit SHA")
    files = {}
    for path in sorted(directory.rglob("*")):
        relative = path.relative_to(directory)
        # upload-pages-artifact excludes hidden files and directories.
        if not path.is_file() or any(part.startswith(".") for part in relative.parts):
            continue
        if relative.as_posix() == MANIFEST:
            continue
        with path.open("rb") as stream:
            files[relative.as_posix()] = digest(stream)
    if "index.html" not in files:
        raise ValueError("The Pages artifact must contain index.html")
    manifest = {"revision": revision, "files": files}
    (directory / MANIFEST).write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def fetch(base_url, path):
    # Check the ordinary public URLs, including the project subpath.
    url = base_url.rstrip("/") + "/" + urllib.parse.quote(path)
    request = urllib.request.Request(url, headers={
        "Cache-Control": "no-cache", "User-Agent": "Sonata-Pages-verification",
    })
    return urllib.request.urlopen(request, timeout=30)


def verify_once(base_url, expected):
    with fetch(base_url, MANIFEST) as response:
        actual = json.load(response)
    if actual != expected:
        raise ValueError("Public deployment manifest does not match the uploaded artifact")
    # Browsers normally request /sonata/, which can be cached separately from index.html.
    files = {"": expected["files"]["index.html"], **expected["files"]}
    for name, checksum in files.items():
        with fetch(base_url, name) as response:
            actual_checksum = digest(response)
        if actual_checksum != checksum:
            raise ValueError(f"Public checksum mismatch: {name or '/ (homepage)'}")


def verify(base_url, expected, timeout=720, interval=15):
    deadline = time.monotonic() + timeout
    while True:
        try:
            verify_once(base_url, expected)
        except (OSError, ValueError, http.client.HTTPException) as error:
            if isinstance(error, urllib.error.HTTPError):
                error.close()
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise RuntimeError(f"Public site verification failed: {error}") from error
            print(f"Waiting for Pages propagation: {error}", flush=True)
            time.sleep(min(interval, remaining))
        else:
            print(f"Verified {expected['revision']}: homepage and {len(expected['files'])} files match.")
            return


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare = commands.add_parser("record")
    prepare.add_argument("--directory", type=Path, default=ROOT / "site")
    check = commands.add_parser("verify")
    check.add_argument("--directory", type=Path, default=ROOT / "site")
    check.add_argument("--url", required=True)
    args = parser.parse_args()
    if args.command == "record":
        revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        record(args.directory, revision)
    else:
        expected = json.loads((args.directory / MANIFEST).read_text())
        try:
            verify(args.url, expected)
        except RuntimeError as error:
            raise SystemExit(str(error)) from error


if __name__ == "__main__":
    main()
