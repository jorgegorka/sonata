#!/usr/bin/env python3
"""Create, sign, verify, and publish per-installation Sonata update feeds."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
REPO = "jorgegorka/sonata"
PAGE = "https://jorgegorka.github.io/sonata"
TARGETS = {
    "darwin-aarch64": ("macos-aarch64", "Sonata-macos-aarch64.app.tar.gz"),
    "linux-x86_64-appimage": ("linux-x86_64", "Sonata-linux-x86_64.AppImage"),
    "linux-x86_64-deb": ("linux-x86_64", "Sonata-linux-x86_64.deb"),
    "linux-aarch64-appimage": ("linux-aarch64", "Sonata-linux-aarch64.AppImage"),
    "linux-aarch64-deb": ("linux-aarch64", "Sonata-linux-aarch64.deb"),
}
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+")
MAX_METADATA = 64 * 1024


def stable(value):
    if not isinstance(value, str) or not VERSION.fullmatch(value):
        raise ValueError("Invalid stable update version")
    return tuple(map(int, value.split(".")))


def sha256(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def public_key():
    config = json.loads((ROOT / "app/src-tauri/tauri.conf.json").read_text())
    return base64.b64decode(config["plugins"]["updater"]["pubkey"], validate=True).decode().splitlines()[1]


def verify(path, signature, version):
    """Minisign verifies both the artifact and the trusted version comment."""
    stable(version)
    decoded = base64.b64decode(signature, validate=True).decode()
    with tempfile.TemporaryDirectory(prefix="sonata-update-verify-") as temp:
        sig = Path(temp) / "artifact.sig"
        sig.write_text(decoded)
        subprocess.run(["minisign", "-Vm", str(path), "-x", str(sig), "-P", public_key(), "-q"],
                       check=True, stdout=subprocess.DEVNULL)
    comments = decoded.splitlines()
    if len(comments) != 4 or not comments[2].startswith("trusted comment: "):
        raise ValueError("Missing trusted signature comment")
    if f"version:{version}" not in comments[2][len("trusted comment: "):].split("\t"):
        raise ValueError("The signature is not bound to the advertised version")


def archive_mac(bundle, output, version):
    stable(version)
    actual = subprocess.check_output(["/usr/libexec/PlistBuddy", "-c", "Print CFBundleShortVersionString",
                                      str(bundle / "Contents/Info.plist")], text=True).strip()
    if actual != version:
        raise ValueError("Mac application version does not match the release")
    for command in (["codesign", "--verify", "--deep", "--strict", str(bundle)],
                    ["xcrun", "stapler", "validate", str(bundle)],
                    ["spctl", "--assess", "--type", "execute", str(bundle)]):
        subprocess.run(command, check=True)
    output.mkdir(parents=True, exist_ok=True)
    with tarfile.open(output / TARGETS["darwin-aarch64"][1], "w:gz") as archive:
        archive.add(bundle, arcname="Sonata.app")


def sign(directory, version, platforms, repository=False):
    stable(version)
    if not os.environ.get("TAURI_SIGNING_PRIVATE_KEY"):
        raise ValueError("TAURI_SIGNING_PRIVATE_KEY is required for public updater artifacts")
    if repository and platforms != ["macos-aarch64"]:
        raise ValueError("Repository-hosted updates support the Mac release only")
    if (directory / "version.txt").read_text().strip() != version:
        raise ValueError("Finalize this release before signing its update artifacts")
    manifest = {}
    for line in (directory / "SHA256SUMS").read_text().splitlines():
        digest, name = line.split("  ", 1)
        if Path(name).name != name or sha256(directory / name) != digest:
            raise ValueError("Release checksum mismatch")
        manifest[name] = digest
    sign_env = os.environ.copy()
    key = sign_env["TAURI_SIGNING_PRIVATE_KEY"]
    sign_env.pop("TAURI_SIGNING_PRIVATE_KEY_PATH", None)
    if key.startswith(("/", "./", "../", "~/")) or (len(key) < 240 and "\n" not in key and Path(key).is_file()):
        if not Path(key).expanduser().is_file():
            raise ValueError("The updater private key file is missing")
        sign_env.pop("TAURI_SIGNING_PRIVATE_KEY")
        sign_env["TAURI_SIGNING_PRIVATE_KEY_PATH"] = str(Path(key).expanduser())
    entries = []
    for target, (platform, asset) in TARGETS.items():
        if platform not in platforms:
            continue
        path = directory / asset
        if repository:
            destination = directory / f"Sonata_{version}.app.tar.gz"
            if destination.exists():
                raise ValueError("Repository update artifacts are immutable; publish a new version")
            shutil.move(path, destination)
            path = destination
        subprocess.run(["npm", "exec", "--", "tauri", "signer", "sign", "--app-version", version, str(path.resolve())],
                       cwd=ROOT / "app", env=sign_env, check=True, stdout=subprocess.DEVNULL)
        signature = path.with_name(path.name + ".sig").read_text().strip()
        verify(path, signature, version)
        base = f"{PAGE}/downloads" if repository else f"https://github.com/{REPO}/releases/download/v{version}"
        feed = {
            "version": version, "url": f"{base}/{path.name}", "signature": signature,
            "notes_url": f"https://github.com/{REPO}/blob/master/CHANGELOG.md" if repository else f"https://github.com/{REPO}/releases/tag/v{version}",
            "sha256": sha256(path),
        }
        name = f"update-{target}.json"
        (directory / name).write_text(json.dumps(feed, indent=2) + "\n")
        entries.extend([path.name, path.name + ".sig", name])
    for name in entries:
        manifest[name] = sha256(directory / name)
    if repository:
        manifest = {name: manifest[name] for name in entries}
    (directory / ("UPDATES-SHA256SUMS" if repository else "SHA256SUMS")).write_text(
        "".join(f"{manifest[name]}  {name}\n" for name in sorted(manifest)))


def fetch_json(url, authenticated=False):
    headers = {"User-Agent": "Sonata-update-feeds", "Accept": "application/vnd.github+json"}
    if authenticated and os.environ.get("GH_TOKEN"):
        headers["Authorization"] = "Bearer " + os.environ["GH_TOKEN"]
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=30) as response:
        data = response.read(MAX_METADATA + 1 if not authenticated else 4 * 1024 * 1024)
    if not authenticated and len(data) > MAX_METADATA:
        raise ValueError("Update metadata is too large")
    return json.loads(data)


def release_list():
    releases = []
    for page in range(1, 101):
        batch = fetch_json(f"https://api.github.com/repos/{REPO}/releases?per_page=100&page={page}", True)
        releases.extend(batch)
        if len(batch) < 100:
            return releases
    raise ValueError("Too many releases to resolve update feeds completely")


def verify_remote(feed):
    with tempfile.TemporaryDirectory(prefix="sonata-feed-") as temp:
        path = Path(temp) / "artifact"
        with urllib.request.urlopen(feed["url"], timeout=300) as response, path.open("wb") as output:
            if not response.url.startswith("https://"):
                raise ValueError("Insecure artifact redirect")
            shutil.copyfileobj(response, output)
        if sha256(path) != feed["sha256"]:
            raise ValueError("Update artifact checksum mismatch")
        verify(path, feed["signature"], feed["version"])


def candidates(releases):
    """Only published, complete releases qualify; select each target independently."""
    result = {}
    def add(target, feed):
        if target not in result or stable(feed["version"]) > stable(result[target]["version"]):
            result[target] = feed
    for release in releases:
        tag = release.get("tag_name", "")
        match = re.fullmatch(r"(v|omarchy-)([0-9]+\.[0-9]+\.[0-9]+)", tag)
        if not match or release.get("draft") or release.get("prerelease"):
            continue
        version = match[2]
        prefix = f"https://github.com/{REPO}/releases/download/{tag}/"
        assets = {asset["name"]: asset for asset in release.get("assets", [])}
        def asset_url(name):
            entry = assets.get(name)
            if not entry or entry.get("size", 0) <= 0 or entry.get("browser_download_url") != prefix + name:
                raise ValueError(f"Missing or unexpected update asset: {name}")
            return prefix + name
        if match[1] == "omarchy-":
            for arch in ("x86_64", "aarch64"):
                name = f"sonata-{arch}.pkg.tar.zst"
                if name in assets:
                    add(f"linux-{arch}-arch", {"version": version, "url": asset_url(name),
                        "notes_url": f"https://github.com/{REPO}/releases/tag/{tag}"})
            continue
        present = [target for target in TARGETS if f"update-{target}.json" in assets]
        for target in present:
            platform, name = TARGETS[target]
            if platform.startswith("linux"):
                arch = platform.removeprefix("linux-")
                for kind in ("deb", "appimage"):
                    asset_url(f"update-linux-{arch}-{kind}.json")
                asset_url(f"Sonata-linux-{arch}.deb")
                asset_url(f"Sonata-linux-{arch}.AppImage")
            else:
                asset_url("Sonata-macos-aarch64.dmg")
            asset_url("SHA256SUMS")
            asset_url("version.txt")
            feed = fetch_json(asset_url(f"update-{target}.json"))
            if (feed.get("version") != version or feed.get("url") != asset_url(name)
                    or feed.get("notes_url") != f"https://github.com/{REPO}/releases/tag/{tag}"
                    or not feed.get("signature") or not re.fullmatch(r"[0-9a-f]{64}", feed.get("sha256", ""))):
                raise ValueError("Invalid release update metadata")
            asset_url(name + ".sig")
            add(target, feed)
    return result


def site(directory, releases=None):
    feeds = candidates(release_list() if releases is None else releases)
    local = directory / "downloads/update-darwin-aarch64.json"
    local_feed = None
    if local.exists():
        local_feed = json.loads(local.read_text())
        value = local_feed["version"]
        stable(value)
        name = f"Sonata_{value}.app.tar.gz"
        if (local_feed["url"] != f"{PAGE}/downloads/{name}"
                or local_feed["notes_url"] != f"https://github.com/{REPO}/blob/master/CHANGELOG.md"):
            raise ValueError("Invalid repository update URL")
        artifact = local.parent / name
        if sha256(artifact) != local_feed["sha256"]:
            raise ValueError("Repository update checksum mismatch")
        verify(artifact, local_feed["signature"], value)
        if "darwin-aarch64" not in feeds or stable(value) > stable(feeds["darwin-aarch64"]["version"]):
            feeds["darwin-aarch64"] = local_feed
    for target, feed in feeds.items():
        if not target.endswith("-arch") and feed is not local_feed:
            verify_remote(feed)
    output = directory / "updates"
    output.mkdir(parents=True, exist_ok=True)
    for stale in output.glob("*.json"):
        stale.unlink()
    for target, feed in feeds.items():
        (output / f"{target}.json").write_text(json.dumps(feed, indent=2) + "\n")


def main():
    import sys
    sys.path.insert(0, str(ROOT / "scripts"))
    from release import version
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    archive = commands.add_parser("archive-mac")
    archive.add_argument("--bundle", type=Path, default=ROOT / "target/release/bundle/macos/Sonata.app")
    archive.add_argument("--output", type=Path, default=ROOT / "release-dist")
    signing = commands.add_parser("sign")
    signing.add_argument("--directory", type=Path, default=ROOT / "release-dist")
    signing.add_argument("--platform", choices=["macos-aarch64", "linux-x86_64", "linux-aarch64"], action="append")
    signing.add_argument("--repository", action="store_true")
    feeds = commands.add_parser("site")
    feeds.add_argument("--directory", type=Path, default=ROOT / "site")
    args = parser.parse_args()
    if args.command == "archive-mac":
        archive_mac(args.bundle, args.output, version())
    elif args.command == "sign":
        sign(args.directory, version(), args.platform or ["macos-aarch64", "linux-x86_64", "linux-aarch64"], args.repository)
    else:
        site(args.directory)


if __name__ == "__main__":
    main()
