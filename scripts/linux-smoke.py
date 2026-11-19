#!/usr/bin/env python3
"""Launch the actual Linux packages in an isolated X11 session without paid agent CLIs."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile
import sys

from release import digest, version


def launch(executable, picker=False):
    args = [sys.executable, str(Path(__file__).with_name("linux-ui-smoke.py")), str(executable)]
    if picker: args.append("--picker")
    subprocess.run(args, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="sonata-packages-") as temp:
        root = Path(temp)
        images = list(args.bundle.glob("appimage/*.AppImage"))
        debs = list(args.bundle.glob("deb/*.deb"))
        if len(images) != 1 or len(debs) != 1:
            raise RuntimeError("Expected one AppImage and one DEB")
        subprocess.run([str(images[0].resolve()), "--appimage-extract"], cwd=root, check=True, stdout=subprocess.DEVNULL)
        appdir = root / "squashfs-root"
        for binary in (appdir / "usr/bin").iterdir():
            if binary.name in ("herdr", "sbot"):
                raise RuntimeError("Runtime helpers must be private resources")
        helpers = list(appdir.glob("usr/lib/*/runtime/herdr"))
        if len(helpers) != 1 or not helpers[0].with_name("sbot").is_file():
            raise RuntimeError("Bundled helpers are missing")
        subprocess.run([str(helpers[0]), "--version"], check=True)
        # Install the real AppImage through the public installer, including a path with spaces.
        feed = root / "releases/download" / f"v{version()}"
        feed.mkdir(parents=True)
        name = f"Sonata-linux-{os.uname().machine}.AppImage"
        (feed / name).symlink_to(images[0].resolve())
        (feed / "SHA256SUMS").write_text(f"{digest(images[0])}  {name}\n")
        prefix = root / "installed app"
        env = {**os.environ, "SONATA_RELEASE_BASE_URL": (root / "releases").as_uri(),
               "SONATA_INSTALL_PREFIX": str(prefix), "XDG_DATA_HOME": str(root / "share")}
        installer = Path(__file__).resolve().parents[1] / "site/install.sh"
        subprocess.run(["bash", str(installer), "--version", version()], env=env, check=True)
        launch(prefix / "bin/sonata", picker=True)
        debdir = root / "deb"
        subprocess.run(["dpkg-deb", "-x", str(debs[0]), str(debdir)], check=True)
        launch(debdir / "usr/bin/sonata")


if __name__ == "__main__":
    main()
