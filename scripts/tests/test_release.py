import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("release", Path(__file__).resolve().parents[1] / "release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.dist = Path(self.temp.name)

    def fixture(self):
        for platform, names in release.ASSETS.items():
            entries = []
            for name in names:
                (self.dist / name).write_bytes(name.encode())
                entries.append(f"{release.digest(self.dist / name)}  {name}\n")
            (self.dist / f"SHA256SUMS-{platform}").write_text("".join(entries))

    def test_complete_matrix_produces_version_and_combined_checksums(self):
        self.fixture()
        release.finalize(self.dist)
        self.assertEqual(len((self.dist / "SHA256SUMS").read_text().splitlines()), 5)
        self.assertEqual((self.dist / "version.txt").read_text().strip(), release.version())

    def test_partial_release_and_corrupt_artifact_cannot_be_finalized(self):
        self.fixture()
        missing = self.dist / "Sonata-linux-aarch64.deb"
        missing.unlink()
        with self.assertRaises(FileNotFoundError): release.finalize(self.dist)
        self.assertFalse((self.dist / "version.txt").exists())
        missing.write_bytes(b"wrong artifact")
        with self.assertRaisesRegex(ValueError, "Checksum mismatch"): release.finalize(self.dist)

    def test_explicit_mac_release_requires_only_verified_mac_artifact(self):
        name = release.ASSETS["macos-aarch64"][0]
        (self.dist / name).write_bytes(b"signed and notarized fixture")
        checksum = release.digest(self.dist / name)
        (self.dist / "SHA256SUMS-macos-aarch64").write_text(f"{checksum}  {name}\n")
        release.finalize(self.dist, ["macos-aarch64"])
        self.assertEqual((self.dist / "SHA256SUMS").read_text(), f"{checksum}  {name}\n")
        self.assertEqual((self.dist / "version.txt").read_text().strip(), release.version())

    def metadata(self, names):
        prefix = f"https://github.com/{release.REPO}/releases/download/v1.0.0/"
        return {"tag_name": "v1.0.0", "draft": False, "prerelease": False,
                "assets": [{"name": name, "size": 42, "browser_download_url": prefix + name} for name in names]}

    def write_site(self, metadata):
        (self.dist / "index.html").write_text((release.ROOT / "site/index.html").read_text())
        with patch.object(release.urllib.request, "urlopen", return_value=io.BytesIO(json.dumps(metadata).encode())):
            release.site_metadata(self.dist)

    def test_mac_only_release_has_direct_download_and_no_linux_downloads(self):
        names = ["Sonata-macos-aarch64.dmg", "SHA256SUMS", "version.txt"]
        self.write_site(self.metadata(names))
        metadata = json.loads((self.dist / "release.json").read_text())
        self.assertEqual(set(metadata["downloads"]), set(names))
        rendered = (self.dist / "index.html").read_text()
        self.assertIn('href="https://github.com/jorgegorka/sonata/releases/download/v1.0.0/Sonata-macos-aarch64.dmg">Download DMG', rendered)
        self.assertIn("AppImage coming soon", rendered)
        self.assertIn("DEB coming soon", rendered)
        self.assertNotIn("Download AppImage", rendered)
        self.assertNotIn("Download DEB", rendered)
        self.assertIn("macOS (Apple Silicon)", rendered)
        self.assertIn("data-binary-install hidden", rendered)

    def test_metadata_rejects_incomplete_platform_or_missing_release_files(self):
        for names in (
            ["Sonata-macos-aarch64.dmg", "Sonata-linux-x86_64.AppImage", "SHA256SUMS", "version.txt"],
            ["Sonata-macos-aarch64.dmg", "version.txt"],
            ["Sonata-macos-aarch64.dmg", "SHA256SUMS"],
            ["SHA256SUMS", "version.txt"],
        ):
            with self.subTest(names=names), self.assertRaises(ValueError):
                self.write_site(self.metadata(names))

    def test_metadata_rejects_unexpected_asset_url(self):
        metadata = self.metadata(["Sonata-macos-aarch64.dmg", "SHA256SUMS", "version.txt"])
        metadata["assets"][0]["browser_download_url"] = "https://example.com/untrusted.dmg"
        with self.assertRaisesRegex(ValueError, "Unexpected release asset URL"):
            self.write_site(metadata)

    def repository_fixture(self):
        directory = self.dist / "downloads"
        directory.mkdir()
        dmg = directory / "Sonata_1.0.0.dmg"
        dmg.write_bytes(b"verified DMG fixture")
        (directory / "version.txt").write_text("1.0.0\n")
        (directory / "SHA256SUMS").write_text(f"{release.digest(dmg)}  {dmg.name}\n")
        (self.dist / "index.html").write_text((release.ROOT / "site/index.html").read_text())
        return dmg

    def test_repository_download_is_advertised_when_no_github_release_exists(self):
        self.repository_fixture()
        missing = release.urllib.error.HTTPError("https://api.github.com", 404, "Not Found", {}, None)
        self.addCleanup(missing.close)
        with patch.object(release.urllib.request, "urlopen", side_effect=missing):
            release.site_metadata(self.dist)
        rendered = (self.dist / "index.html").read_text()
        self.assertIn('href="./downloads/Sonata_1.0.0.dmg">Download DMG', rendered)
        self.assertIn('href="./downloads/SHA256SUMS"', rendered)
        self.assertIn("data-binary-install hidden", rendered)
        self.assertTrue(json.loads((self.dist / "release.json").read_text())["available"])

    def test_repository_download_with_changed_bytes_cannot_be_advertised(self):
        dmg = self.repository_fixture()
        dmg.write_bytes(b"corrupted DMG")
        with self.assertRaisesRegex(ValueError, "Repository download checksum mismatch"):
            release.repository_release(self.dist)

    def test_api_failure_does_not_replace_site_with_repository_fallback(self):
        self.repository_fixture()
        error = release.urllib.error.HTTPError("https://api.github.com", 503, "Unavailable", {}, None)
        self.addCleanup(error.close)
        with patch.object(release.urllib.request, "urlopen", side_effect=error), self.assertRaises(release.urllib.error.HTTPError):
            release.site_metadata(self.dist)
        self.assertFalse((self.dist / "release.json").exists())

    def test_static_download_links_follow_release_availability(self):
        source = (release.ROOT / "site/index.html").read_text()
        (self.dist / "index.html").write_text(source)
        names = [name for group in release.ASSETS.values() for name in group] + ["SHA256SUMS"]
        prefix = f"https://github.com/{release.REPO}/releases/download/v0.1.0/"
        release.render_site(self.dist, {"available": True, "version": "0.1.0", "notes": "https://github.com/jorgegorka/sonata/releases/tag/v0.1.0", "downloads": {name: prefix + name for name in names}})
        rendered = (self.dist / "index.html").read_text()
        for name in names: self.assertIn(f'href="{prefix}{name}"', rendered)
        self.assertNotIn("data-binary-install hidden", rendered)
        release.render_site(self.dist, {"available": False})
        unavailable = (self.dist / "index.html").read_text()
        self.assertIn("data-binary-install hidden", unavailable)
        self.assertNotIn("releases/download/", unavailable)


if __name__ == "__main__":
    unittest.main()
