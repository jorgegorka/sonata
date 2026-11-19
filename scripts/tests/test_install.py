import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

INSTALLER = Path(__file__).resolve().parents[2] / "site/install.sh"


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sonata-installer-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.prefix = self.root / "prefix with spaces"
        self.dest = self.prefix / "opt/sonata"
        self.feed = self.root / "releases"
        self.mock = self.root / "commands"
        self.mock.mkdir()
        self.bundle = self.root / "bundle"
        for path in ["AppRun", "usr/bin/sonata", "usr/lib/sonata/runtime/herdr", "usr/lib/sonata/runtime/sbot"]:
            self.executable(self.bundle / path, "#!/bin/sh\nexit 0\n")
        icon = self.bundle / "usr/share/icons/hicolor/256x256/apps/sonata.png"
        icon.parent.mkdir(parents=True)
        icon.write_bytes(b"test icon")
        for name, text in {
            "uname": 'if [ "$1" = -s ]; then echo Linux; else echo aarch64; fi',
            "ps": 'if [ "${SONATA_TEST_RUNNING:-}" = 1 ]; then printf "%s sonata\\n" "$(id -u)"; fi',
            "xdg-open": "exit 0", "dpkg-query": 'if [ "${SONATA_TEST_DEB:-}" = 1 ]; then echo "install ok installed"; else exit 1; fi',
            "update-desktop-database": "exit 0",
        }.items():
            self.executable(self.mock / name, "#!/bin/sh\n" + text + "\n")
        self.env = {**os.environ, "PATH": str(self.mock) + ":" + os.environ["PATH"],
                    "SONATA_RELEASE_BASE_URL": self.feed.as_uri(), "SONATA_INSTALL_PREFIX": str(self.prefix),
                    "XDG_DATA_HOME": str(self.prefix / "share"), "SONATA_TEST_BUNDLE": str(self.bundle)}
        self.release("0.1.0")

    def executable(self, path, body):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)
        path.chmod(0o755)

    def release(self, version):
        directory = self.feed / "download" / f"v{version}"
        directory.mkdir(parents=True)
        asset = directory / "Sonata-linux-aarch64.AppImage"
        self.executable(asset, '#!/bin/sh\nset -e\n[ "$1" = --appimage-extract ]\ncp -R "$SONATA_TEST_BUNDLE" squashfs-root\n')
        (directory / "SHA256SUMS").write_text(f"{hashlib.sha256(asset.read_bytes()).hexdigest()}  {asset.name}\n")
        latest = self.feed / "latest/download"
        latest.mkdir(parents=True, exist_ok=True)
        (latest / "version.txt").write_text(version + "\n")
        return asset

    def run_installer(self, *args, ok=True):
        result = subprocess.run(["bash", str(INSTALLER), *args], env=self.env, text=True, capture_output=True)
        if ok:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout + result.stderr

    def test_fresh_install_and_repeat_with_spaces(self):
        self.run_installer()
        self.assertEqual((self.dest / ".sonata-version").read_text().strip(), "0.1.0")
        self.assertTrue((self.prefix / "bin/sonata").is_file())
        self.assertTrue((self.prefix / "share/applications/io.sonata.desktop.desktop").is_file())
        self.assertIn("already installed", self.run_installer())
        self.assertEqual(subprocess.run([str(self.prefix / "bin/sonata")]).returncode, 0)

    def test_upgrade_preserves_user_data_and_refuses_downgrade(self):
        self.run_installer()
        data = self.prefix / "share/sonata/sonata.db"
        data.parent.mkdir()
        data.write_bytes(b"saved user data")
        self.release("0.2.0")
        self.run_installer()
        self.assertEqual(data.read_bytes(), b"saved user data")
        self.assertEqual((self.dest / ".sonata-version").read_text().strip(), "0.2.0")
        self.assertIn("Downgrades", self.run_installer("--version", "0.1.0", ok=False))

    def test_checksum_failure_keeps_previous_installation(self):
        self.run_installer()
        asset = self.release("0.2.0")
        asset.write_bytes(asset.read_bytes() + b"corrupted")
        self.assertIn("checksum mismatch", self.run_installer(ok=False))
        self.assertEqual((self.dest / ".sonata-version").read_text().strip(), "0.1.0")

    def test_running_app_and_deb_installations_are_not_replaced(self):
        self.env["SONATA_TEST_RUNNING"] = "1"
        self.assertIn("Sonata is running", self.run_installer(ok=False))
        self.env.pop("SONATA_TEST_RUNNING")
        self.env["SONATA_TEST_DEB"] = "1"
        self.assertIn("DEB-managed", self.run_installer(ok=False))
        self.assertFalse(self.dest.exists())

    def test_failed_swap_restores_previous_installation(self):
        self.run_installer()
        self.release("0.2.0")
        real_mv = shutil.which("mv")
        self.executable(self.mock / "mv", f'#!/bin/sh\ncase "$1" in */next) exit 1 ;; esac\nexec "{real_mv}" "$@"\n')
        self.run_installer(ok=False)
        self.assertEqual((self.dest / ".sonata-version").read_text().strip(), "0.1.0")

    def test_failed_desktop_registration_rolls_back_bundle_and_launcher(self):
        self.run_installer()
        before = (self.prefix / "bin/sonata").read_bytes()
        self.release("0.2.0")
        real_mv = shutil.which("mv")
        self.executable(self.mock / "mv", f'#!/bin/sh\ncase "$2:$3" in */.sonata.*:*/applications/*) exit 1 ;; esac\nexec "{real_mv}" "$@"\n')
        self.run_installer(ok=False)
        self.assertEqual((self.dest / ".sonata-version").read_text().strip(), "0.1.0")
        self.assertEqual((self.prefix / "bin/sonata").read_bytes(), before)

    def test_custom_destination_and_unmanaged_directory(self):
        custom = self.root / "custom app"
        self.run_installer("--dest", str(custom), "--version", "v0.1.0")
        self.assertTrue((custom / ".sonata-version").exists())
        unknown = self.root / "unmanaged"
        unknown.mkdir()
        (unknown / "important.txt").write_text("keep")
        self.assertIn("not managed", self.run_installer("--dest", str(unknown), ok=False))
        self.assertEqual((unknown / "important.txt").read_text(), "keep")

    def test_missing_download_and_invalid_arguments(self):
        self.assertIn("Invalid stable", self.run_installer("--version", "../other", ok=False))
        self.assertIn("needs a value", self.run_installer("--dest", ok=False))
        self.assertIn("absolute path", self.run_installer("--dest", "relative", ok=False))
        self.assertIn("Usage:", self.run_installer("--help"))
        self.run_installer("--version", "9.9.9", ok=False)
        self.assertFalse(self.dest.exists())

    def test_unavailable_platform_preserves_existing_installation(self):
        self.run_installer()
        self.release("0.2.0")
        (self.feed / "download/v0.2.0/SHA256SUMS").write_text("0" * 64 + "  Sonata-macos-aarch64.dmg\n")
        self.assertIn("no package for linux/aarch64 yet", self.run_installer(ok=False))
        self.assertEqual((self.dest / ".sonata-version").read_text().strip(), "0.1.0")


if __name__ == "__main__":
    unittest.main()
