#!/usr/bin/env python3
"""Collect verified builds, finalize a complete release, or prepare the static download page."""
import argparse
import hashlib
import html
import json
from pathlib import Path
import re
import shutil
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
REPO = "jorgegorka/sonata"
ASSETS = {
    "macos-aarch64": ["Sonata-macos-aarch64.dmg"],
    "linux-x86_64": ["Sonata-linux-x86_64.AppImage", "Sonata-linux-x86_64.deb"],
    "linux-aarch64": ["Sonata-linux-aarch64.AppImage", "Sonata-linux-aarch64.deb"],
}


def version():
    cargo = re.search(r'(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"$', (ROOT / "Cargo.toml").read_text())
    if not cargo:
        raise ValueError("Cargo.toml must declare a stable workspace version")
    value = cargo.group(1)
    for file in ["app/package.json", "app/package-lock.json", "app/src-tauri/tauri.conf.json"]:
        if json.loads((ROOT / file).read_text())["version"] != value:
            raise ValueError(f"Version mismatch in {file}; expected {value}")
    if json.loads((ROOT / "app/package-lock.json").read_text())["packages"][""]["version"] != value:
        raise ValueError(f"Version mismatch in package-lock.json root package; expected {value}")
    return value


def digest(file):
    with file.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest() if hasattr(hashlib, "file_digest") else hashlib.sha256(stream.read()).hexdigest()


def collect(platform, bundle, output):
    value = version()
    output.mkdir(parents=True, exist_ok=True)
    entries = []
    for name in ASSETS[platform]:
        extension = name.rsplit(".", 1)[1]
        candidates = list(bundle.glob(f"{extension.lower()}/*.{extension}"))
        candidates = [p for p in candidates if f"_{value}" in p.name]
        if len(candidates) != 1:
            raise ValueError(f"Expected one {platform} {extension} for {value}, found {candidates}")
        shutil.copy2(candidates[0], output / name)
        entries.append(f"{digest(output / name)}  {name}\n")
    (output / f"SHA256SUMS-{platform}").write_text("".join(entries))


def finalize(output, platforms=None):
    platforms = platforms or list(ASSETS)
    expected = {name for platform in platforms for name in ASSETS[platform]}
    entries = {}
    for platform in platforms:
        names = ASSETS[platform]
        lines = (output / f"SHA256SUMS-{platform}").read_text().splitlines()
        parsed = {}
        for line in lines:
            sha, name = line.split("  ", 1)
            if not re.fullmatch(r"[0-9a-f]{64}", sha) or name not in names or name in parsed:
                raise ValueError(f"Invalid manifest entry: {line}")
            if digest(output / name) != sha:
                raise ValueError(f"Checksum mismatch: {name}")
            parsed[name] = sha
        if set(parsed) != set(names):
            raise ValueError(f"Incomplete {platform} release")
        entries.update(parsed)
    if set(entries) != expected:
        raise ValueError("Incomplete release")
    (output / "SHA256SUMS").write_text("".join(f"{entries[name]}  {name}\n" for name in sorted(entries)))
    (output / "version.txt").write_text(version() + "\n")
    for platform in platforms:
        (output / f"SHA256SUMS-{platform}").unlink()


def repository_release(directory):
    """Verify a notarized DMG hosted with Pages when no GitHub Release exists."""
    downloads = directory / "downloads"
    if not downloads.exists():
        return {"available": False}
    value = (downloads / "version.txt").read_text().strip()
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", value):
        raise ValueError("Invalid repository download version")
    name = f"Sonata_{value}.dmg"
    manifest = (downloads / "SHA256SUMS").read_text().splitlines()
    if len(manifest) != 1 or manifest[0] != f"{digest(downloads / name)}  {name}":
        raise ValueError("Repository download checksum mismatch")
    return {
        "available": True, "version": value,
        "notes": f"https://github.com/{REPO}/blob/master/CHANGELOG.md",
        "downloads": {"Sonata-macos-aarch64.dmg": f"./downloads/{name}",
                      "SHA256SUMS": "./downloads/SHA256SUMS", "version.txt": "./downloads/version.txt"},
    }


def desktop_tag(release):
    tag = release.get("tag_name", "")
    return bool(re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag)) and not release.get("draft") and not release.get("prerelease")


def desktop_downloads(release):
    """Homepage metadata for a desktop release.

    Returns None when the release ships no Mac or Linux desktop package, so the
    caller can keep the repository DMG. A platform that is only partly uploaded
    still fails the deploy.
    """
    tag = release["tag_name"]
    assets = {a["name"]: a for a in release["assets"]}
    expected = {"SHA256SUMS", "version.txt"}
    platforms = []
    for platform, names in ASSETS.items():
        present = set(names) & assets.keys()
        if present and present != set(names):
            raise ValueError(f"The latest release has incomplete {platform} packages")
        if present:
            platforms.append(platform)
            expected.update(names)
    if not platforms:
        return None
    if not expected.issubset(assets) or any(assets[n]["size"] <= 0 for n in expected):
        raise ValueError("The latest release is incomplete; refusing to advertise it")
    prefix = f"https://github.com/{REPO}/releases/download/{tag}/"
    for name in expected:
        if assets[name]["browser_download_url"] != prefix + name:
            raise ValueError("Unexpected release asset URL")
    return {
        "available": True, "version": tag[1:],
        "notes": f"https://github.com/{REPO}/releases/tag/{tag}",
        "downloads": {name: prefix + name for name in sorted(expected)},
    }


def site_metadata(directory, api="https://api.github.com"):
    request = urllib.request.Request(f"{api}/repos/{REPO}/releases/latest", headers={"Accept": "application/vnd.github+json", "User-Agent": "Sonata-release"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            release = json.load(response)
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        error.close()
        release = None
    # An Omarchy package release has no Mac or Linux desktop assets. The homepage
    # keeps the repository DMG until a release actually ships those packages.
    if release and desktop_tag(release):
        desktop = desktop_downloads(release)
        result = desktop if desktop is not None else repository_release(directory)
    else:
        result = repository_release(directory)
    (directory / "release.json").write_text(json.dumps(result, indent=2) + "\n")
    render_site(directory, result)


def render_site(directory, release):
    """Bake download links into HTML so they also work with JavaScript disabled."""
    path = directory / "index.html"
    source = path.read_text()
    ready = release.get("available", False)
    downloads = release.get("downloads", {}) if ready else {}
    def link(match):
        attrs, name, label = match.group("attrs", "name", "label")
        url = downloads.get(name, f"https://github.com/{REPO}/releases")
        attrs = re.sub(r'href="[^"]*"', f'href="{html.escape(url, quote=True)}"', attrs)
        kind = "AppImage" if name.endswith(".AppImage") else "DEB" if name.endswith(".deb") else "DMG" if name.endswith(".dmg") else None
        if kind:
            label = f"Download {kind}" if name in downloads else f"{kind} coming soon"
        return f"<a{attrs}>{label}</a>"
    source = re.sub(r'<a(?P<attrs>\s[^>]*data-asset="(?P<name>[^"]+)"[^>]*)>(?P<label>.*?)</a>', link, source)
    complete = all(name in downloads for names in ASSETS.values() for name in names)
    source = re.sub(r'data-binary-install(?: hidden)?', 'data-binary-install' + ('' if complete else ' hidden'), source)
    notes = release["notes"] if ready else f"https://github.com/{REPO}/releases"
    source = re.sub(r'data-release-notes href="[^"]*"', f'data-release-notes href="{html.escape(notes, quote=True)}"', source)
    path.write_text(source)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    check = commands.add_parser("check-version")
    check.add_argument("--tag")
    package = commands.add_parser("collect")
    package.add_argument("--platform", choices=ASSETS, required=True)
    package.add_argument("--bundle", type=Path, default=ROOT / "target/release/bundle")
    package.add_argument("--output", type=Path, default=ROOT / "release-dist")
    complete = commands.add_parser("finalize")
    complete.add_argument("--output", type=Path, default=ROOT / "release-dist")
    complete.add_argument("--platform", choices=ASSETS, action="append", help="Publish selected complete platforms; default: all")
    site = commands.add_parser("site")
    site.add_argument("--directory", type=Path, default=ROOT / "site")
    args = parser.parse_args()
    if args.command == "check-version":
        value = version()
        if args.tag and args.tag != f"v{value}":
            raise ValueError(f"Tag {args.tag} does not match v{value}")
        print(value)
    elif args.command == "collect":
        collect(args.platform, args.bundle, args.output)
    elif args.command == "finalize":
        finalize(args.output, args.platform)
    else:
        site_metadata(args.directory)


if __name__ == "__main__":
    main()
