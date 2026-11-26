import base64
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("updates", Path(__file__).resolve().parents[1] / "updates.py")
updates = importlib.util.module_from_spec(spec)
spec.loader.exec_module(updates)


class FeedTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.metadata = {}
        self.fetch = patch.object(updates, "fetch_json", side_effect=lambda url: self.metadata[url])
        self.fetch.start()
        self.addCleanup(self.fetch.stop)

    def release(self, version, targets):
        prefix = f"https://github.com/{updates.REPO}/releases/download/v{version}/"
        names = {"SHA256SUMS", "version.txt"}
        for target in targets:
            _, asset = updates.TARGETS[target]
            names.update([asset, asset + ".sig", f"update-{target}.json"])
            if target.startswith("darwin"):
                names.add("Sonata-macos-aarch64.dmg")
            self.metadata[prefix + f"update-{target}.json"] = {
                "version": version, "url": prefix + asset, "signature": "signed",
                "notes_url": f"https://github.com/{updates.REPO}/releases/tag/v{version}",
                "sha256": "a" * 64,
            }
        return {"tag_name": "v" + version, "assets": [
            {"name": name, "size": 100, "browser_download_url": prefix + name} for name in names
        ]}

    def arch_release(self, version="2.0.0"):
        tag = "omarchy-" + version
        name = "sonata-x86_64.pkg.tar.zst"
        return {"tag_name": tag, "assets": [{"name": name, "size": 100,
            "browser_download_url": f"https://github.com/{updates.REPO}/releases/download/{tag}/{name}"}]}

    def test_platforms_select_newest_versions_independently(self):
        linux = ["linux-x86_64-appimage", "linux-x86_64-deb"]
        releases = [self.arch_release(), self.release("1.9.0", ["darwin-aarch64"]),
                    self.release("1.10.0", ["darwin-aarch64"]), self.release("1.8.0", linux)]
        feeds = updates.candidates(releases)
        self.assertEqual(feeds["darwin-aarch64"]["version"], "1.10.0")
        self.assertEqual(feeds["linux-x86_64-deb"]["version"], "1.8.0")
        self.assertEqual(feeds["linux-x86_64-arch"]["version"], "2.0.0")
        self.assertNotIn("linux-aarch64-deb", feeds)

    def test_drafts_prereleases_and_legacy_downloads_are_not_native_updates(self):
        draft = self.release("1.2.0", ["darwin-aarch64"])
        draft["draft"] = True
        preview = self.release("1.3.0", ["darwin-aarch64"])
        preview["prerelease"] = True
        legacy = {"tag_name": "v1.4.0", "assets": [{"name": "Sonata-macos-aarch64.dmg"}]}
        self.assertEqual(updates.candidates([draft, preview, legacy]), {})

    def test_incomplete_linux_and_missing_signatures_fail(self):
        with self.assertRaisesRegex(ValueError, "Missing"):
            updates.candidates([self.release("1.2.0", ["linux-x86_64-appimage"])])
        release = self.release("1.2.0", ["darwin-aarch64"])
        release["assets"] = [a for a in release["assets"] if not a["name"].endswith(".sig")]
        with self.assertRaisesRegex(ValueError, "Missing"):
            updates.candidates([release])

    def test_manifest_cannot_change_release_version_or_download_destination(self):
        for key, value in [("version", "9.0.0"), ("url", "https://example.com/app"), ("signature", "")]:
            release = self.release("1.2.0", ["darwin-aarch64"])
            next(iter(self.metadata.values()))[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                updates.candidates([release])

    def test_failed_verification_preserves_existing_feeds(self):
        output = self.directory / "updates"
        output.mkdir()
        previous = output / "darwin-aarch64.json"
        previous.write_text("previous")
        with patch.object(updates, "verify_remote", side_effect=ValueError("bad signature")):
            with self.assertRaisesRegex(ValueError, "bad signature"):
                updates.site(self.directory, [self.release("1.2.0", ["darwin-aarch64"])])
        self.assertEqual(previous.read_text(), "previous")

    def test_verified_feeds_are_written_only_for_available_targets(self):
        releases = [self.release("1.2.0", ["darwin-aarch64"]), self.arch_release()]
        with patch.object(updates, "verify_remote") as verify:
            updates.site(self.directory, releases)
        self.assertEqual(verify.call_count, 1)
        self.assertEqual({p.name for p in (self.directory / "updates").iterdir()},
                         {"darwin-aarch64.json", "linux-x86_64-arch.json"})

    def test_public_signing_requires_key_and_matching_finalized_version(self):
        with patch.dict(os.environ, {"TAURI_SIGNING_PRIVATE_KEY": ""}):
            with self.assertRaisesRegex(ValueError, "required"):
                updates.sign(self.directory, "1.2.0", ["macos-aarch64"])
        (self.directory / "version.txt").write_text("1.0.0")
        with patch.dict(os.environ, {"TAURI_SIGNING_PRIVATE_KEY": "test"}):
            with self.assertRaisesRegex(ValueError, "Finalize"):
                updates.sign(self.directory, "1.2.0", ["macos-aarch64"])


@unittest.skipUnless(shutil.which("minisign") and (updates.ROOT / "app/node_modules/.bin/tauri").exists(),
                     "Install minisign and npm dependencies to exercise actual signatures")
class SignatureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="sonata-signature-tests-")
        cls.root = Path(cls.temp.name)
        key = cls.root / "test.key"
        env = {k: v for k, v in os.environ.items() if not k.startswith("TAURI_SIGNING_")}
        subprocess.run(["npm", "exec", "--", "tauri", "signer", "generate", "--ci",
                        "--password", "", "--write-keys", str(key)],
                       cwd=updates.ROOT / "app", env=env, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        cls.public = base64.b64decode(key.with_suffix(".key.pub").read_text()).decode().splitlines()[1]
        cls.env = {**env, "TAURI_SIGNING_PRIVATE_KEY": str(key), "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": ""}

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=self.root)
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.artifact = self.directory / updates.TARGETS["darwin-aarch64"][1]
        self.artifact.write_bytes(b"disposable signed update fixture")
        (self.directory / "version.txt").write_text("1.2.0")
        (self.directory / "SHA256SUMS").write_text("")
        self.public_patch = patch.object(updates, "public_key", return_value=self.public)
        self.public_patch.start()
        self.addCleanup(self.public_patch.stop)

    def sign(self, repository=False):
        with patch.dict(os.environ, self.env, clear=True):
            updates.sign(self.directory, "1.2.0", ["macos-aarch64"], repository)
        return json.loads((self.directory / "update-darwin-aarch64.json").read_text())

    def test_real_signature_and_signed_version_reject_tampering(self):
        feed = self.sign()
        updates.verify(self.artifact, feed["signature"], "1.2.0")
        with self.assertRaisesRegex(ValueError, "bound"):
            updates.verify(self.artifact, feed["signature"], "1.3.0")
        self.artifact.write_bytes(b"tampered")
        with self.assertRaises(subprocess.CalledProcessError):
            updates.verify(self.artifact, feed["signature"], "1.2.0")

    def test_wrong_public_key_is_rejected(self):
        feed = self.sign()
        self.public_patch.stop()
        with self.assertRaises(subprocess.CalledProcessError):
            updates.verify(self.artifact, feed["signature"], "1.2.0")

    def test_repository_signing_keeps_dmg_manifest_and_immutable_asset_names(self):
        feed = self.sign(repository=True)
        self.assertEqual((self.directory / "SHA256SUMS").read_text(), "")
        self.assertTrue((self.directory / "UPDATES-SHA256SUMS").is_file())
        self.assertEqual(feed["url"], updates.PAGE + "/downloads/Sonata_1.2.0.app.tar.gz")
        with self.assertRaisesRegex(ValueError, "immutable"):
            self.sign(repository=True)
        site = self.root / "site"
        downloads = site / "downloads"
        if downloads.exists():
            shutil.rmtree(downloads)
        shutil.copytree(self.directory, downloads)
        updates.site(site, [])
        self.assertEqual(json.loads((site / "updates/darwin-aarch64.json").read_text()), feed)
